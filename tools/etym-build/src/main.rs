// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Builds `etym.onym`, the engine's optional etymology overlay, from a wiktextract English dump.
//!
//! The overlay is to etymology what the WordNet database is to senses: a frozen, preprocessed,
//! read-only artifact the engine reads in place. This tool is the producer. It never ships inside
//! the engine; it runs offline, joins Wiktionary's etymology prose against the WordNet lemma set,
//! and writes the compact file the loader in `crates/onym-engine/src/etymology.rs` consumes.
//!
//! Input is wiktextract JSONL on stdin (one JSON object per line, as kaikki.org publishes for
//! English). Only three fields matter: `lang_code`, `word`, and `etymology_text`. An entry counts
//! when its language is English, its word normalises to a WordNet lemma, and it carries etymology
//! prose. Prose is whitespace-collapsed, length-capped, and deduplicated; a word with several
//! distinct etymologies (Wiktionary's "Etymology 1", "Etymology 2") keeps each as its own
//! paragraph, in first-seen order.
//!
//! Usage:
//!   zcat english.jsonl.gz | etym-build --wordnet <dir> --out <etym.onym> [--source <label>]
//!
//! The file is keyed by WordNet query form (ASCII, lowercase, underscored, exactly an index key),
//! sorted, and headed by a provenance block of leading-space lines the loader skips. Coverage and
//! size statistics are written to stderr.

use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{self, BufRead, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The cap on one paragraph, in characters. Etymology prose past this is truncated at a sentence
/// or word boundary; it keeps the overlay small enough to ship without losing the substance, which
/// almost always sits in the first sentence or two.
const PARAGRAPH_CAP: usize = 600;

/// The cap on paragraphs per word, so a page with many homographs cannot bloat one entry.
const PARAGRAPHS_PER_WORD: usize = 3;

/// Openers of a real etymology sentence. They rescue the prose from wiktextract's rendered
/// "Etymology tree" blocks, whose tree-node lines (a language and a form) never begin this way.
const ETYMOLOGY_CUES: &[&str] = &[
    "From",
    "Borrowed",
    "Inherited",
    "Derived",
    "Calque",
    "Calqued",
    "Coined",
    "Compound",
    "Clipping",
    "Clipped",
    "Blend",
    "Back-formation",
    "Abbreviation",
    "Acronym",
    "Initialism",
    "Univerbation",
    "Ultimately",
    "Via",
    "After",
    "Named",
    "Shortening",
    "Shortened",
    "Contraction",
    "Variant",
    "Alteration",
    "Eponym",
    "Onomatopoeia",
    "Imitative",
    "Probably",
    "Possibly",
    "Perhaps",
    "Apparently",
    "Of ",
    "A ",
    "An ",
    "The ",
];

/// The wiktextract fields the overlay needs; serde ignores the rest of each line.
#[derive(Deserialize)]
struct WiktEntry {
    word: Option<String>,
    lang_code: Option<String>,
    etymology_text: Option<String>,
}

struct Args {
    wordnet: PathBuf,
    out: PathBuf,
    source: String,
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("etym-build: {message}");
            eprintln!(
                "usage: etym-build --wordnet <dir> --out <etym.onym> [--source <label>]\n\
                 reads wiktextract JSONL on stdin"
            );
            return ExitCode::from(2);
        }
    };

    let lemmas = match load_wordnet_lemmas(&args.wordnet) {
        Ok(lemmas) => lemmas,
        Err(error) => {
            eprintln!(
                "etym-build: cannot read WordNet index in {}: {error}",
                args.wordnet.display()
            );
            return ExitCode::from(1);
        }
    };
    eprintln!("WordNet lemmas: {}", lemmas.len());

    let mut entries: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut stats = Stats::default();
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("etym-build: read error: {error}");
                return ExitCode::from(1);
            }
        };
        if line.is_empty() {
            continue;
        }
        stats.lines += 1;
        let Ok(entry) = serde_json::from_str::<WiktEntry>(&line) else {
            stats.unparsed += 1;
            continue;
        };
        if entry.lang_code.as_deref() != Some("en") {
            continue;
        }
        stats.english += 1;
        let (Some(word), Some(text)) = (entry.word.as_deref(), entry.etymology_text.as_deref())
        else {
            continue;
        };
        stats.with_etymology += 1;
        let key = normalise_key(word);
        if key.is_empty() || !lemmas.contains(&key) {
            continue;
        }
        let Some(paragraph) = clean(text) else {
            continue;
        };
        let bucket = entries.entry(key).or_default();
        if bucket.len() < PARAGRAPHS_PER_WORD && !bucket.contains(&paragraph) {
            bucket.push(paragraph);
        }
    }

    match write_overlay(&args, &entries, &lemmas, &stats) {
        Ok(bytes) => {
            stats.report(&lemmas, &entries, bytes);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("etym-build: cannot write {}: {error}", args.out.display());
            ExitCode::from(1)
        }
    }
}

fn parse_args() -> Result<Args, String> {
    let mut wordnet = None;
    let mut out = None;
    let mut source = String::from("kaikki.org English wiktextract");
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--wordnet" => {
                wordnet = Some(PathBuf::from(args.next().ok_or("--wordnet needs a path")?))
            }
            "--out" => out = Some(PathBuf::from(args.next().ok_or("--out needs a path")?)),
            "--source" => source = args.next().ok_or("--source needs a label")?,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(Args {
        wordnet: wordnet.ok_or("--wordnet is required")?,
        out: out.ok_or("--out is required")?,
        source,
    })
}

/// The WordNet lemma set: the first field of every non-header line of the four index files. These
/// are already ASCII, lowercase, and underscored, so they are exactly the keys the overlay uses.
fn load_wordnet_lemmas(dir: &Path) -> io::Result<HashSet<String>> {
    let mut lemmas = HashSet::new();
    for name in ["index.noun", "index.verb", "index.adj", "index.adv"] {
        let text = fs::read_to_string(dir.join(name))?;
        for line in text.lines() {
            if line.is_empty() || line.starts_with(' ') {
                continue;
            }
            if let Some(lemma) = line.split(' ').next() {
                lemmas.insert(lemma.to_string());
            }
        }
    }
    Ok(lemmas)
}

/// A wiktextract word turned into a WordNet query-form key: trimmed, spaces to underscores, and
/// WordNet's ASCII-only lower casing (matching `strtolower`, so accented letters are untouched and
/// therefore never spuriously match an ASCII lemma).
fn normalise_key(word: &str) -> String {
    word.trim()
        .replace(' ', "_")
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Collapse a wiktextract `etymology_text` to one shippable line. Typographic characters are
/// folded to ASCII, a rendered "Etymology tree" block is reduced to the prose that follows it,
/// bullet markers are dropped, whitespace runs (including embedded newlines) become single spaces,
/// and the result is length-capped at a sentence or word boundary, then backslash-escaped for the
/// overlay format. Returns nothing when there is no readable prose, as for a tree-only entry.
fn clean(text: &str) -> Option<String> {
    let text = normalise_chars(text);
    // wiktextract sometimes renders an "Etymology tree" block before the prose. Its tree-node
    // lines are not sentences, so keep from the first line that opens like an etymology; a tree
    // with no such line carries nothing readable.
    let kept: Vec<&str> = if text.lines().any(|line| line.trim() == "Etymology tree") {
        match text.lines().position(|line| starts_with_cue(line.trim())) {
            Some(start) => text.lines().skip(start).collect(),
            None => return None,
        }
    } else {
        text.lines().collect()
    };
    let flattened = kept
        .iter()
        .map(|line| line.trim_start().trim_start_matches('*').trim_start())
        .collect::<Vec<_>>()
        .join(" ");
    let collapsed: String = flattened.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    Some(truncate(&collapsed, PARAGRAPH_CAP).replace('\\', "\\\\"))
}

/// True when a line opens like a real etymology sentence (section [`ETYMOLOGY_CUES`]).
fn starts_with_cue(line: &str) -> bool {
    ETYMOLOGY_CUES.iter().any(|cue| line.starts_with(cue))
}

/// Fold the typographic characters wiktextract carries to ASCII: curly quotes, the dash family,
/// the ellipsis, and the fixed-width spaces. The prose then reads cleanly and survives every
/// downstream encoding, including the conformance dumper's.
fn normalise_chars(text: &str) -> String {
    let mut out: String = text
        .chars()
        .map(|c| match c {
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' | '\u{2033}' => '"',
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{2032}' => '\'',
            '\u{2013}' | '\u{2014}' | '\u{2015}' => '-',
            '\u{00A0}' | '\u{2002}' | '\u{2003}' | '\u{2009}' | '\u{200A}' => ' ',
            other => other,
        })
        .collect();
    if out.contains('\u{2026}') {
        out = out.replace('\u{2026}', "...");
    }
    out
}

/// Truncate to at most `cap` characters, preferring to end at the last sentence boundary, then the
/// last word boundary, before the cap. A short string is returned whole.
fn truncate(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        return text.to_string();
    }
    let head: String = text.chars().take(cap).collect();
    if let Some(stop) = head.rfind(". ") {
        return head[..=stop].trim_end().to_string();
    }
    if let Some(space) = head.rfind(' ') {
        return head[..space].trim_end().to_string();
    }
    head
}

fn write_overlay(
    args: &Args,
    entries: &BTreeMap<String, Vec<String>>,
    lemmas: &HashSet<String>,
    stats: &Stats,
) -> io::Result<u64> {
    let file = fs::File::create(&args.out)?;
    let mut out = BufWriter::new(file);
    // The provenance header: leading-space lines the loader skips, so the artifact is auditable on
    // its own, the way the WordNet files carry their licence header.
    writeln!(out, " etym.onym v1")?;
    writeln!(out, " source: {}", args.source)?;
    writeln!(out, " wordnet-lemmas: {}", lemmas.len())?;
    writeln!(out, " matched-lemmas: {}", entries.len())?;
    writeln!(out, " english-entries: {}", stats.english)?;
    for (key, paragraphs) in entries {
        write!(out, "{key}")?;
        for paragraph in paragraphs {
            write!(out, "\t{paragraph}")?;
        }
        writeln!(out)?;
    }
    out.flush()?;
    Ok(fs::metadata(&args.out)?.len())
}

#[derive(Default)]
struct Stats {
    lines: u64,
    unparsed: u64,
    english: u64,
    with_etymology: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_wordnet_query_form() {
        assert_eq!(normalise_key("ice cream"), "ice_cream");
        assert_eq!(normalise_key("Wordsworth"), "wordsworth");
        assert_eq!(normalise_key("  dog "), "dog");
        // ASCII-only lowering, so an accented letter never folds onto an ASCII lemma.
        assert_eq!(normalise_key("café"), "café");
    }

    #[test]
    fn smart_typography_folds_to_ascii() {
        let cleaned =
            clean("From aarde (\u{201C}earth\u{201D}) \u{2014} the animal\u{2019}s habit\u{2026}")
                .unwrap();
        assert_eq!(cleaned, "From aarde (\"earth\") - the animal's habit...");
    }

    #[test]
    fn etymology_tree_block_reduces_to_its_prose() {
        let raw = "Etymology tree\nProto-Polynesian *kaha\nHawaiian a\nBorrowed from Hawaiian aa.";
        assert_eq!(clean(raw).unwrap(), "Borrowed from Hawaiian aa.");
    }

    #[test]
    fn tree_without_prose_is_dropped() {
        let raw = "Etymology tree\nProto-Polynesian *kaha\nHawaiian aa";
        assert_eq!(clean(raw), None);
    }

    #[test]
    fn bullet_markers_are_flattened() {
        let raw = "From Old English.\n* (sense one): one origin.\n* (sense two): another.";
        assert_eq!(
            clean(raw).unwrap(),
            "From Old English. (sense one): one origin. (sense two): another."
        );
    }

    #[test]
    fn long_prose_truncates_at_a_sentence_boundary() {
        let first = "From Latin longus, a first sentence that runs on for a while.";
        let rest = " ".to_string() + &"word ".repeat(200);
        let cleaned = clean(&(first.to_string() + &rest)).unwrap();
        assert!(cleaned.chars().count() <= PARAGRAPH_CAP);
        assert!(cleaned.ends_with("while."));
    }

    #[test]
    fn empty_prose_yields_nothing() {
        assert_eq!(clean("   \n  "), None);
    }
}

impl Stats {
    fn report(
        &self,
        lemmas: &HashSet<String>,
        entries: &BTreeMap<String, Vec<String>>,
        bytes: u64,
    ) {
        let matched = entries.len();
        let paragraphs: usize = entries.values().map(Vec::len).sum();
        let coverage = if lemmas.is_empty() {
            0.0
        } else {
            100.0 * matched as f64 / lemmas.len() as f64
        };
        eprintln!("JSONL lines:        {}", self.lines);
        eprintln!("unparsed lines:     {}", self.unparsed);
        eprintln!("English entries:    {}", self.english);
        eprintln!("with etymology:     {}", self.with_etymology);
        eprintln!("matched lemmas:     {matched} ({coverage:.1}% of WordNet)");
        eprintln!("paragraphs written: {paragraphs}");
        eprintln!(
            "overlay size:       {bytes} bytes ({:.1} MiB)",
            bytes as f64 / 1_048_576.0
        );
    }
}
