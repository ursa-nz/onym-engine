// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The optional translations overlay, `omw.onym`, per `spec/engine.md` section 6.11. Like the
//! etymology overlay it is a frozen, preprocessed artifact the engine reads in place, built offline
//! by the tool in `tools/omw-build`, and absent by default: when the file is missing the engine is
//! byte-for-byte what it was without it. It only adds the Translations section.
//!
//! Where the etymology overlay is keyed by lemma, this one is keyed by synset, by the part of speech
//! and the WNDB offset the engine already holds for a gathered sense, because translations are per
//! concept. A body line is `key\tgroup\tgroup...`. The key is a part-of-speech letter (`n`, `v`,
//! `a`, `r`) and the eight-digit offset, exactly as `index.sense` and the data files write it. Each
//! group is a language code, a colon, and that language's words joined by commas; a comma or
//! backslash inside a word is backslash-escaped. The header carries the provenance and a language
//! legend, leading-space lines the body scan skips: a `lang <code> <display name>` line maps a code
//! to the name the section shows, so the language set stays open-ended and self-describing.
//!
//! Like the WordNet reader, the file is held once as bytes and a synset's groups are sliced and
//! decoded on demand, so the overlay costs about its own size in memory and nothing is parsed that a
//! lookup never asks for.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub(crate) struct Translations {
    data: Vec<u8>,
    // (pos index 0..=3, synset offset) to the byte range of the groups payload (after the key tab).
    index: HashMap<(u8, u32), (u32, u32)>,
    // Language code to display name, from the header legend; a code with no entry shows verbatim.
    legend: HashMap<String, String>,
}

impl Translations {
    /// Load the overlay from `data_dir`. An absent file yields an empty overlay, which means no
    /// translations for any synset, never an error, exactly as the optional WordNet side tables
    /// behave.
    pub(crate) fn load(data_dir: &Path) -> std::io::Result<Translations> {
        let path = data_dir.join("omw.onym");
        if !path.is_file() {
            return Ok(Translations {
                data: Vec::new(),
                index: HashMap::new(),
                legend: HashMap::new(),
            });
        }
        let data = fs::read(&path)?;
        let mut index = HashMap::new();
        let mut legend = HashMap::new();
        let mut pos = 0;
        while pos < data.len() {
            let end = data[pos..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(data.len(), |p| pos + p);
            let line = &data[pos..end];
            match line.first() {
                None => {}
                // A header line (leading space or '#') carries no entry, but may carry a legend line.
                Some(b' ') | Some(b'#') => {
                    if let Ok(text) = std::str::from_utf8(line) {
                        let mut tokens = text.split_whitespace();
                        if tokens.next() == Some("lang")
                            && let Some(code) = tokens.next()
                        {
                            let name = tokens.collect::<Vec<_>>().join(" ");
                            if !name.is_empty() {
                                legend.insert(code.to_string(), name);
                            }
                        }
                    }
                }
                Some(_) => {
                    if let Some(tab) = line.iter().position(|&b| b == b'\t')
                        && let Some(key) = parse_key(&line[..tab])
                    {
                        let body_start = pos + tab + 1;
                        index.insert(key, (body_start as u32, (end - body_start) as u32));
                    }
                }
            }
            pos = end + 1;
        }
        Ok(Translations {
            data,
            index,
            legend,
        })
    }

    /// The translation groups for the synset at `pos_index` (0 noun to 3 adverb) and `offset`, each
    /// a language display name and its words in overlay order, the groups sorted by display name.
    /// Empty when the overlay is absent or the synset has none.
    pub(crate) fn groups(&self, pos_index: u8, offset: u32) -> Vec<(String, Vec<String>)> {
        let Some(&(start, len)) = self.index.get(&(pos_index, offset)) else {
            return Vec::new();
        };
        let body = &self.data[start as usize..start as usize + len as usize];
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for field in body.split(|&b| b == b'\t') {
            let Some(colon) = field.iter().position(|&b| b == b':') else {
                continue;
            };
            let code = String::from_utf8_lossy(&field[..colon]);
            let language = self
                .legend
                .get(code.as_ref())
                .cloned()
                .unwrap_or_else(|| code.into_owned());
            groups.push((language, unescape_words(&field[colon + 1..])));
        }
        groups.sort_by(|a, b| a.0.cmp(&b.0));
        groups
    }
}

/// Parse a body key, a part-of-speech letter then the synset offset, into the engine's pos index
/// (matching `WnPos::index`) and the offset.
fn parse_key(key: &[u8]) -> Option<(u8, u32)> {
    let (&letter, digits) = key.split_first()?;
    let pos_index = match letter {
        b'n' => 0,
        b'v' => 1,
        b'a' => 2,
        b'r' => 3,
        _ => return None,
    };
    let offset = std::str::from_utf8(digits).ok()?.parse().ok()?;
    Some((pos_index, offset))
}

/// Split a comma-separated group into words, decoding the `\\` and `\,` escapes the build tool
/// writes over UTF-8 forms. An unknown escape is left verbatim, so no byte is lost, and empty words
/// (a stray or trailing comma) are dropped.
fn unescape_words(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let mut words = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(',') => current.push(','),
                Some('\\') => current.push('\\'),
                Some(other) => {
                    current.push('\\');
                    current.push(other);
                }
                None => current.push('\\'),
            },
            ',' => words.push(std::mem::take(&mut current)),
            other => current.push(other),
        }
    }
    words.push(current);
    words.retain(|word| !word.is_empty());
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn overlay(contents: &[u8]) -> (tempdir::Guard, Translations) {
        let guard = tempdir::Guard::new();
        let mut file = fs::File::create(guard.path.join("omw.onym")).unwrap();
        file.write_all(contents).unwrap();
        let omw = Translations::load(&guard.path).unwrap();
        (guard, omw)
    }

    #[test]
    fn absent_file_is_an_empty_overlay() {
        let guard = tempdir::Guard::new();
        let omw = Translations::load(&guard.path).unwrap();
        assert!(omw.groups(0, 2148998).is_empty());
    }

    #[test]
    fn groups_resolve_names_and_sort_by_display_name() {
        let (_g, omw) = overlay(
            b" omw.onym test\n lang it Italian\n lang pt Portuguese\n\
              n02148998\tpt:c\xc3\xa3o,cachorro\tit:cane\n",
        );
        let groups = omw.groups(0, 2148998);
        assert_eq!(
            groups,
            vec![
                ("Italian".to_string(), vec!["cane".to_string()]),
                (
                    "Portuguese".to_string(),
                    vec!["cão".to_string(), "cachorro".to_string()]
                ),
            ]
        );
        // Another part of speech at the same offset is a different synset and is not found.
        assert!(omw.groups(1, 2148998).is_empty());
    }

    #[test]
    fn a_code_without_a_legend_entry_shows_verbatim() {
        let (_g, omw) = overlay(b"v02001779\txx:correre\n");
        assert_eq!(
            omw.groups(1, 2001779),
            vec![("xx".to_string(), vec!["correre".to_string()])]
        );
    }

    #[test]
    fn escaped_comma_and_backslash_survive_in_a_word() {
        let (_g, omw) =
            overlay(b" lang en English\nn00000001\ten:Buena Vista\\, Virginia,a\\\\b\n");
        assert_eq!(
            omw.groups(0, 1),
            vec![(
                "English".to_string(),
                vec!["Buena Vista, Virginia".to_string(), "a\\b".to_string()]
            )]
        );
    }

    #[test]
    fn header_and_unkeyed_lines_carry_no_entry() {
        let (_g, omw) =
            overlay(b"#comment\n lang it Italian\nbroken-no-tab\nn00000005\tit:gatto\n");
        assert_eq!(
            omw.groups(0, 5),
            vec![("Italian".to_string(), vec!["gatto".to_string()])]
        );
    }

    /// A throwaway temporary directory, removed on drop, mirroring the etymology loader's test
    /// fixture so the loader is exercised over a real file without depending on any crate.
    mod tempdir {
        use std::path::PathBuf;

        pub struct Guard {
            pub path: PathBuf,
        }

        impl Guard {
            pub fn new() -> Guard {
                let mut path = std::env::temp_dir();
                let unique = format!(
                    "onym-omw-test-{}-{}",
                    std::process::id(),
                    COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                );
                path.push(unique);
                std::fs::create_dir_all(&path).unwrap();
                Guard { path }
            }
        }

        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }

        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    }
}
