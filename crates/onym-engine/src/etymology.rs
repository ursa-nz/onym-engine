// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The optional etymology overlay, `etym.onym`, per `spec/engine.md` section 6.10. It is the one
//! file the engine reads that does not come from WordNet: a frozen, preprocessed artifact keyed by
//! WordNet lemma, built offline from Wiktionary by the tool in `tools/`. When the file is absent
//! the engine is byte-for-byte what it was without it, so the overlay never alters WordNet
//! behaviour; it only adds the Etymology section when a looked-up headword has an entry.
//!
//! The file is UTF-8, unlike the ISO-8859-1 WordNet database, because etymology prose carries the
//! accented spellings of source languages. Each entry is one physical line, `lemma\tparagraph`
//! with further tab-separated paragraphs for a word with several etymologies (Wiktionary's
//! "Etymology 1", "Etymology 2"). The lemma is WordNet query form: ASCII, lowercase, underscored,
//! exactly an index key. A leading-space or `#` line is a header and is skipped, mirroring the
//! WordNet index files. Paragraphs are whitespace-collapsed by the build tool, so an entry never
//! spans lines; the only escapes are `\\` for a literal backslash and `\n` for a newline.
//!
//! Like the WordNet reader, the file is held once as bytes and the prose for a lemma is sliced and
//! unescaped on demand, so the overlay costs about its own size in memory and nothing is parsed
//! that a lookup never asks for.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub(crate) struct Etymology {
    data: Vec<u8>,
    // Lemma to the byte range of its paragraph payload (the part after the first tab).
    index: HashMap<String, (u32, u32)>,
}

impl Etymology {
    /// Load the overlay from `data_dir`. An absent file yields an empty overlay, which means no
    /// etymology for any word, never an error, exactly as the optional WordNet side tables behave.
    pub(crate) fn load(data_dir: &Path) -> std::io::Result<Etymology> {
        let path = data_dir.join("etym.onym");
        if !path.is_file() {
            return Ok(Etymology {
                data: Vec::new(),
                index: HashMap::new(),
            });
        }
        let data = fs::read(&path)?;
        let mut index = HashMap::new();
        let mut pos = 0;
        while pos < data.len() {
            let end = data[pos..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(data.len(), |p| pos + p);
            let line = &data[pos..end];
            // A header line (leading space or '#') and a line with no tab carry no entry.
            if !matches!(line.first(), None | Some(b' ') | Some(b'#'))
                && let Some(tab) = line.iter().position(|&b| b == b'\t')
                && let Ok(key) = std::str::from_utf8(&line[..tab])
            {
                let body_start = pos + tab + 1;
                index.insert(
                    key.to_string(),
                    (body_start as u32, (end - body_start) as u32),
                );
            }
            pos = end + 1;
        }
        Ok(Etymology { data, index })
    }

    /// The etymology paragraphs for the query-form `lemma`, in source order, or empty when the
    /// overlay is absent or the lemma has none.
    pub(crate) fn paragraphs(&self, lemma: &str) -> Vec<String> {
        let Some(&(start, len)) = self.index.get(lemma) else {
            return Vec::new();
        };
        let body = &self.data[start as usize..start as usize + len as usize];
        body.split(|&b| b == b'\t')
            .filter(|paragraph| !paragraph.is_empty())
            .map(unescape)
            .collect()
    }
}

/// Decode the `\\` and `\n` escapes the build tool writes, over UTF-8 prose. An unknown escape is
/// left verbatim, so the function never loses a byte.
fn unescape(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if !text.contains('\\') {
        return text.into_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn overlay(contents: &[u8]) -> (tempdir::Guard, Etymology) {
        let guard = tempdir::Guard::new();
        let mut file = fs::File::create(guard.path.join("etym.onym")).unwrap();
        file.write_all(contents).unwrap();
        let etym = Etymology::load(&guard.path).unwrap();
        (guard, etym)
    }

    #[test]
    fn absent_file_is_an_empty_overlay() {
        let guard = tempdir::Guard::new();
        let etym = Etymology::load(&guard.path).unwrap();
        assert!(etym.paragraphs("dog").is_empty());
    }

    #[test]
    fn one_paragraph_round_trips() {
        let (_g, etym) = overlay(b"dog\tFrom Middle English dogge.\n");
        assert_eq!(etym.paragraphs("dog"), vec!["From Middle English dogge."]);
        assert!(etym.paragraphs("cat").is_empty());
    }

    #[test]
    fn several_etymologies_split_on_tabs() {
        let (_g, etym) = overlay(
            b"bear\tThe animal, from Old English bera.\tTo carry, from Old English beran.\n",
        );
        assert_eq!(
            etym.paragraphs("bear"),
            vec![
                "The animal, from Old English bera.",
                "To carry, from Old English beran."
            ]
        );
    }

    #[test]
    fn header_and_blank_lines_are_skipped() {
        let (_g, etym) = overlay(
            b" generated by tools/etym-build\n#source: wiktextract\nant\tFrom Latin formica.\n",
        );
        assert_eq!(etym.paragraphs("ant"), vec!["From Latin formica."]);
    }

    #[test]
    fn escapes_decode_and_utf8_survives() {
        let (_g, etym) = overlay("ox\tFrom Proto-Germanic *uhsô \\\\ a\\nb.\n".as_bytes());
        assert_eq!(
            etym.paragraphs("ox"),
            vec!["From Proto-Germanic *uhsô \\ a\nb."]
        );
    }

    /// A throwaway temporary directory, removed on drop, so the loader is exercised over a real
    /// file without depending on any crate. Test-only.
    mod tempdir {
        use std::path::PathBuf;

        pub struct Guard {
            pub path: PathBuf,
        }

        impl Guard {
            pub fn new() -> Guard {
                let mut path = std::env::temp_dir();
                let unique = format!(
                    "onym-etym-test-{}-{}",
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
