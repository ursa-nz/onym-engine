// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Builds `omw.onym`, the engine's optional translations overlay, from the Open Multilingual
//! Wordnet components.
//!
//! The overlay is to translations what the WordNet database is to senses: a frozen, preprocessed,
//! read-only artifact the engine reads in place. This tool is the producer. It never ships inside
//! the engine; it runs offline and joins each OEWN synset to the words other languages use for the
//! same concept, through the Collaborative Interlingual Index that both sides already speak.
//!
//! The central concern is keying. Translations are per concept, so the overlay keys on the WNDB
//! synset offset the engine reads, which the OEWNTK grinder assigns and which is not the OEWN synset
//! number. The bridge runs through the shipped base's own `index.sense`, so the key is tied to the
//! exact bytes the engine opens:
//!
//!   index.sense  : sense key      -> WNDB offset (and the part of speech)
//!   OEWN LMF      : sense          -> synset -> ILI
//!   OMW component : ILI            -> words in that language
//!
//! A WNDB sense key maps to its LMF sense id by a mechanical transform (`dog%1:05:00::` to
//! `oewn-dog__1.05.00..`), so no fuzzy matching is involved. Because the producer reads the freshly
//! ground base's `index.sense`, the overlay stays in lockstep with the base, the way the etymology
//! overlay stays in lockstep with the lemma set; a base re-grind that moves offsets needs an overlay
//! rebuild, sequenced after the grind.
//!
//! Usage:
//!   omw-build --wordnet <dir> --lmf <oewn-lmf.xml> --out <omw.onym> [--source <label>] <component.xml>...
//!
//! `--wordnet` is the freshly ground base directory holding `index.sense`. `--lmf` is the OEWN 2025
//! LMF, decompressed. Each positional argument is one OMW component in WN-LMF; its language code is
//! read from its `<Lexicon>` and shown under the display name in [`language_name`]. The file is
//! keyed `<pos><offset>` and sorted; coverage and size statistics are written to stderr.

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

struct Args {
    wordnet: PathBuf,
    lmf: PathBuf,
    out: PathBuf,
    source: String,
    components: Vec<PathBuf>,
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("omw-build: {message}");
            eprintln!(
                "usage: omw-build --wordnet <dir> --lmf <oewn-lmf.xml> --out <omw.onym> \
                 [--source <label>] <component.xml>..."
            );
            return ExitCode::from(2);
        }
    };

    // The OEWN bridge: synset to ILI, and sense to synset. Senses precede synsets in the LMF, so
    // both maps are read in full and composed afterwards rather than inline.
    let (synset_ili, sense_synset) = match read_lmf(&args.lmf) {
        Ok(maps) => maps,
        Err(error) => {
            eprintln!("omw-build: cannot read LMF {}: {error}", args.lmf.display());
            return ExitCode::from(1);
        }
    };
    eprintln!(
        "OEWN synsets with ILI: {}, senses: {}",
        synset_ili.len(),
        sense_synset.len()
    );

    // The shipped base's index.sense ties each WNDB (pos, offset) key to an ILI.
    let key_ili = match read_index_sense(&args.wordnet.join("index.sense"), &synset_ili, &sense_synset)
    {
        Ok(map) => map,
        Err(error) => {
            eprintln!("omw-build: cannot read index.sense: {error}");
            return ExitCode::from(1);
        }
    };
    eprintln!("WNDB synsets keyed to an ILI: {}", key_ili.len());

    // The OMW side: ILI to that language's words, per component.
    let mut ili_words: HashMap<String, BTreeMap<String, Vec<String>>> = HashMap::new();
    let mut codes: BTreeMap<String, usize> = BTreeMap::new();
    for path in &args.components {
        let (code, pairs) = match read_component(path) {
            Ok(component) => component,
            Err(error) => {
                eprintln!("omw-build: cannot read component {}: {error}", path.display());
                return ExitCode::from(1);
            }
        };
        let mut added = 0;
        for (ili, word) in pairs {
            let words = ili_words.entry(ili).or_default().entry(code.clone()).or_default();
            if !words.contains(&word) {
                words.push(word);
                added += 1;
            }
        }
        eprintln!("  {} ({code}): {added} words", path.display());
        *codes.entry(code).or_default() += added;
    }

    match write_overlay(&args, &key_ili, &ili_words, &codes) {
        Ok((synsets, per_language, bytes)) => {
            eprintln!("keyed synsets written: {synsets} of {} with an ILI", key_ili.len());
            for (code, count) in &per_language {
                eprintln!("  {code}: {count} synsets covered");
            }
            eprintln!("overlay size: {bytes} bytes ({:.2} MiB)", bytes as f64 / 1_048_576.0);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("omw-build: cannot write {}: {error}", args.out.display());
            ExitCode::from(1)
        }
    }
}

fn parse_args() -> Result<Args, String> {
    let mut wordnet = None;
    let mut lmf = None;
    let mut out = None;
    let mut source = String::from("Open Multilingual Wordnet via the Collaborative Interlingual Index");
    let mut components = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--wordnet" => wordnet = Some(PathBuf::from(args.next().ok_or("--wordnet needs a path")?)),
            "--lmf" => lmf = Some(PathBuf::from(args.next().ok_or("--lmf needs a path")?)),
            "--out" => out = Some(PathBuf::from(args.next().ok_or("--out needs a path")?)),
            "--source" => source = args.next().ok_or("--source needs a label")?,
            other if other.starts_with("--") => return Err(format!("unknown argument {other}")),
            other => components.push(PathBuf::from(other)),
        }
    }
    if components.is_empty() {
        return Err("at least one OMW component is required".into());
    }
    Ok(Args {
        wordnet: wordnet.ok_or("--wordnet is required")?,
        lmf: lmf.ok_or("--lmf is required")?,
        out: out.ok_or("--out is required")?,
        source,
        components,
    })
}

/// Read the OEWN LMF into synset-to-ILI (skipping the `ili="in"` concepts that have no
/// interlingual id) and sense-to-synset maps.
fn read_lmf(path: &Path) -> io::Result<(HashMap<String, String>, HashMap<String, String>)> {
    let mut synset_ili = HashMap::new();
    let mut sense_synset = HashMap::new();
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        let trimmed = line.trim_start();
        if trimmed.starts_with("<Synset ") {
            if let (Some(id), Some(ili)) = (attr(&line, "id"), attr(&line, "ili"))
                && ili != "in"
            {
                synset_ili.insert(id.to_string(), ili.to_string());
            }
        } else if trimmed.starts_with("<Sense ") {
            if let (Some(id), Some(synset)) = (attr(&line, "id"), attr(&line, "synset")) {
                sense_synset.insert(id.to_string(), synset.to_string());
            }
        }
    }
    Ok((synset_ili, sense_synset))
}

/// Read the base's `index.sense` into a sorted map from the WNDB key (`<pos><offset>`) to the ILI of
/// that synset, via the OEWN bridge. The first sense of a synset settles its ILI; the others repeat
/// it.
fn read_index_sense(
    path: &Path,
    synset_ili: &HashMap<String, String>,
    sense_synset: &HashMap<String, String>,
) -> io::Result<BTreeMap<String, String>> {
    let mut key_ili = BTreeMap::new();
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        let mut fields = line.split_whitespace();
        let (Some(sense_key), Some(offset)) = (fields.next(), fields.next()) else {
            continue;
        };
        let Some(pos) = pos_letter(sense_key) else {
            continue;
        };
        let sense_id = sense_key_to_id(sense_key);
        let Some(synset) = sense_synset.get(&sense_id) else {
            continue;
        };
        let Some(ili) = synset_ili.get(synset) else {
            continue;
        };
        key_ili
            .entry(format!("{pos}{offset}"))
            .or_insert_with(|| ili.clone());
    }
    Ok(key_ili)
}

/// Read one OMW component: its language code from the `<Lexicon>`, and the words each synset's ILI
/// carries. Synsets follow the lexical entries in the file, so the synset-to-ILI map is built first
/// over the whole file and the `(synset, word)` pairs resolved against it afterwards.
fn read_component(path: &Path) -> io::Result<(String, Vec<(String, String)>)> {
    let mut code = String::new();
    let mut synset_ili = HashMap::new();
    let mut current_word: Option<String> = None;
    let mut synset_words: Vec<(String, String)> = Vec::new();
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if code.is_empty()
            && let Some(language) = attr(&line, "language")
        {
            code = language.to_string();
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("<Synset ") {
            if let (Some(id), Some(ili)) = (attr(&line, "id"), attr(&line, "ili")) {
                synset_ili.insert(id.to_string(), ili.to_string());
            }
        } else if trimmed.starts_with("<Lemma ") {
            current_word = attr(&line, "writtenForm").map(xml_unescape);
        } else if trimmed.starts_with("<Sense ") {
            if let (Some(word), Some(synset)) = (current_word.as_ref(), attr(&line, "synset")) {
                synset_words.push((synset.to_string(), word.clone()));
            }
        }
    }
    if code.is_empty() {
        return Err(io::Error::other(format!(
            "{}: no language on its Lexicon",
            path.display()
        )));
    }
    let pairs = synset_words
        .into_iter()
        .filter_map(|(synset, word)| synset_ili.get(&synset).map(|ili| (ili.clone(), word)))
        .collect();
    Ok((code, pairs))
}

fn write_overlay(
    args: &Args,
    key_ili: &BTreeMap<String, String>,
    ili_words: &HashMap<String, BTreeMap<String, Vec<String>>>,
    codes: &BTreeMap<String, usize>,
) -> io::Result<(usize, BTreeMap<String, usize>, u64)> {
    // The body is built first so the header can carry the coverage counts the gate reads, the way
    // the etymology overlay's header carries its lemma counts.
    let mut body = Vec::new();
    let mut synsets = 0;
    let mut per_language: BTreeMap<String, usize> = codes.keys().map(|c| (c.clone(), 0)).collect();
    for (key, ili) in key_ili {
        let Some(languages) = ili_words.get(ili) else {
            continue;
        };
        write!(body, "{key}")?;
        for (code, words) in languages {
            let escaped: Vec<String> = words.iter().map(|word| escape(word)).collect();
            write!(body, "\t{code}:{}", escaped.join(","))?;
            *per_language.entry(code.clone()).or_default() += 1;
        }
        writeln!(body)?;
        synsets += 1;
    }

    let mut out = BufWriter::new(File::create(&args.out)?);
    // The provenance header, language legend, and coverage counts: leading-space lines the loader
    // skips for keying but reads the legend from, so the artifact is auditable on its own and the
    // language names and coverage travel with the data, the way the WordNet files carry their
    // licence header.
    writeln!(out, " omw.onym v1")?;
    writeln!(out, " source: {} (offline join, keyed by WNDB synset offset)", args.source)?;
    writeln!(out, " licence: per component, see PROVENANCE.md and LICENSES/")?;
    writeln!(out, " base-synsets-with-ili: {}", key_ili.len())?;
    writeln!(out, " keyed-synsets: {synsets}")?;
    for code in codes.keys() {
        writeln!(out, " lang {code} {}", language_name(code).unwrap_or(code))?;
    }
    for (code, count) in &per_language {
        writeln!(out, " coverage {code}: {count}")?;
    }
    out.write_all(&body)?;
    out.flush()?;
    Ok((synsets, per_language, fs::metadata(&args.out)?.len()))
}

/// The WNDB part-of-speech letter for a sense key, from the `%` digit (1 noun, 2 verb, 3 and 5
/// adjective, 4 adverb), matching the engine's pos letters.
fn pos_letter(sense_key: &str) -> Option<char> {
    let digit = sense_key.split('%').nth(1)?.chars().next()?;
    Some(match digit {
        '1' => 'n',
        '2' => 'v',
        '3' | '5' => 'a',
        '4' => 'r',
        _ => return None,
    })
}

/// A WNDB sense key turned into its OEWN LMF sense id: the lemma kept verbatim, the `%` turned to
/// `__`, and the tail colons turned to dots. `dog%1:05:00::` becomes `oewn-dog__1.05.00..`.
fn sense_key_to_id(sense_key: &str) -> String {
    let (lemma, tail) = sense_key.rsplit_once('%').unwrap_or((sense_key, ""));
    format!("oewn-{lemma}__{}", tail.replace(':', "."))
}

/// Escape a written form for the overlay: backslash first, then the comma that separates words in a
/// group. The loader reverses exactly these two.
fn escape(word: &str) -> String {
    word.replace('\\', "\\\\").replace(',', "\\,")
}

/// Find the value of `name="..."` on `line`. The leading space anchors the match to a real
/// attribute, so a name that is the tail of another attribute never matches.
fn attr<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let pattern = format!(" {name}=\"");
    let start = line.find(&pattern)? + pattern.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Decode the five predefined XML entities and numeric character references a written form may
/// carry, so the overlay holds display text. An unknown entity is left verbatim.
fn xml_unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix('#')
                .and_then(|number| match number.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => number.parse().ok(),
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The display name for an OMW language code, covering the components PLAN section 8 considers
/// shippable. A code with no entry shows verbatim, so a new component never silently mislabels.
fn language_name(code: &str) -> Option<&'static str> {
    Some(match code {
        "sq" => "Albanian",
        "arb" | "ar" => "Arabic",
        "eu" => "Basque",
        "bg" => "Bulgarian",
        "ca" => "Catalan",
        "cmn" | "zh" => "Chinese",
        "hr" => "Croatian",
        "da" => "Danish",
        "nl" => "Dutch",
        "fi" => "Finnish",
        "fr" => "French",
        "gl" => "Galician",
        "el" => "Greek",
        "he" => "Hebrew",
        "is" => "Icelandic",
        "id" => "Indonesian",
        "it" => "Italian",
        "ja" => "Japanese",
        "lt" => "Lithuanian",
        "zsm" | "ms" => "Malay",
        "nb" => "Norwegian Bokmål",
        "nn" => "Norwegian Nynorsk",
        "fa" => "Persian",
        "pl" => "Polish",
        "pt" => "Portuguese",
        "ro" => "Romanian",
        "sk" => "Slovak",
        "sl" => "Slovenian",
        "es" => "Spanish",
        "sv" => "Swedish",
        "th" => "Thai",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sense_key_maps_to_the_lmf_sense_id() {
        assert_eq!(sense_key_to_id("dog%1:05:00::"), "oewn-dog__1.05.00..");
        assert_eq!(
            sense_key_to_id("-apos-hood%1:14:01::"),
            "oewn--apos-hood__1.14.01.."
        );
    }

    #[test]
    fn pos_letter_reads_the_sense_key_digit() {
        assert_eq!(pos_letter("dog%1:05:00::"), Some('n'));
        assert_eq!(pos_letter("run%2:38:00::"), Some('v'));
        assert_eq!(pos_letter("good%3:00:01::"), Some('a'));
        assert_eq!(pos_letter("able%5:00:00:competent:00"), Some('a'));
        assert_eq!(pos_letter("nope"), None);
    }

    #[test]
    fn attr_anchors_on_a_real_attribute() {
        let line = r#"      <Sense id="oewn-run__2.38.00.." subcat="vii" synset="oewn-01930264-v">"#;
        assert_eq!(attr(line, "id"), Some("oewn-run__2.38.00.."));
        assert_eq!(attr(line, "synset"), Some("oewn-01930264-v"));
        assert_eq!(attr(line, "missing"), None);
    }

    #[test]
    fn escape_protects_comma_and_backslash() {
        assert_eq!(escape("Buena Vista, Virginia"), "Buena Vista\\, Virginia");
        assert_eq!(escape("a\\b"), "a\\\\b");
        assert_eq!(escape("cane"), "cane");
    }

    #[test]
    fn xml_unescape_decodes_entities() {
        assert_eq!(xml_unescape("AT&amp;T"), "AT&T");
        assert_eq!(xml_unescape("caf&#233;"), "café");
        assert_eq!(xml_unescape("plain"), "plain");
    }
}
