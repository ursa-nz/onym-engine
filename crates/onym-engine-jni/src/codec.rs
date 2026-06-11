// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The encode side of the wire format described in the crate documentation. Everything here is
//! safe code over the engine's model; the JNI glue in `lib.rs` only moves the finished bytes.

use onym_engine::{Entry, SectionItems, TreeNode};

/// The section-kind bytes, one per [`SectionItems`] variant. The Kotlin decoder mirrors them.
const KIND_DEFINITIONS: u8 = 0;
const KIND_WORDS: u8 = 1;
const KIND_ANTONYMS: u8 = 2;
const KIND_TREE: u8 = 3;

/// A successful open: tag 1 and the engine handle.
pub fn open_ok(handle: u64) -> Vec<u8> {
    let mut out = vec![1];
    out.extend_from_slice(&handle.to_le_bytes());
    out
}

/// A failed open: tag 0 and the error message.
pub fn open_error(message: &str) -> Vec<u8> {
    let mut out = vec![0];
    put_string(&mut out, message);
    out
}

/// A list of strings, as completion and suggestion answers cross.
pub fn string_list(items: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    put_string_list(&mut out, items);
    out
}

/// A whole entry: term, then each section as title, kind byte, and the kind's payload.
pub fn entry(entry: &Entry) -> Vec<u8> {
    let mut out = Vec::new();
    put_string(&mut out, &entry.term);
    put_u32(&mut out, entry.sections.len());
    for section in &entry.sections {
        put_string(&mut out, section.title);
        match &section.items {
            SectionItems::Definitions(items) => {
                out.push(KIND_DEFINITIONS);
                put_u32(&mut out, items.len());
                for item in items {
                    match item.pos {
                        Some(pos) => {
                            out.push(1);
                            put_string(&mut out, pos);
                        }
                        None => out.push(0),
                    }
                    put_string(&mut out, &item.gloss);
                    put_string_list(&mut out, &item.examples);
                }
            }
            SectionItems::Words(words) => {
                out.push(KIND_WORDS);
                put_string_list(&mut out, words);
            }
            SectionItems::Antonyms(items) => {
                out.push(KIND_ANTONYMS);
                put_u32(&mut out, items.len());
                for item in items {
                    put_string(&mut out, &item.term);
                    out.push(u8::from(item.direct));
                    put_string_list(&mut out, &item.implications);
                }
            }
            SectionItems::Tree(nodes) => {
                out.push(KIND_TREE);
                put_tree(&mut out, nodes);
            }
        }
    }
    out
}

/// Encode sibling nodes depth-first: each node is its terms, then its children, recursively.
fn put_tree(out: &mut Vec<u8>, nodes: &[TreeNode]) {
    put_u32(out, nodes.len());
    for node in nodes {
        put_string_list(out, &node.terms);
        put_tree(out, &node.children);
    }
}

fn put_string_list(out: &mut Vec<u8>, items: &[String]) {
    put_u32(out, items.len());
    for item in items {
        put_string(out, item);
    }
}

fn put_string(out: &mut Vec<u8>, value: &str) {
    put_u32(out, value.len());
    out.extend_from_slice(value.as_bytes());
}

/// Lengths and counts cross as u32. The expect cannot fire: every length here comes from the
/// 16-megabyte WordNet database, orders of magnitude below the limit.
fn put_u32(out: &mut Vec<u8>, value: usize) {
    let value = u32::try_from(value).expect("length fits in u32");
    out.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use onym_engine::{Antonym, Definition, Section};

    #[test]
    fn open_results_carry_tag_then_payload() {
        assert_eq!(open_ok(0x0102), [1, 0x02, 0x01, 0, 0, 0, 0, 0, 0]);
        assert_eq!(open_error("no"), [0, 2, 0, 0, 0, b'n', b'o']);
    }

    #[test]
    fn string_lists_are_count_prefixed() {
        assert_eq!(string_list(&[]), [0, 0, 0, 0]);
        assert_eq!(
            string_list(&["ab".to_string()]),
            [1, 0, 0, 0, 2, 0, 0, 0, b'a', b'b']
        );
    }

    #[test]
    fn entries_encode_every_section_kind() {
        let sample = Entry {
            term: "x".to_string(),
            sections: vec![
                Section {
                    title: "Definitions",
                    items: SectionItems::Definitions(vec![Definition {
                        pos: None,
                        gloss: "g".to_string(),
                        examples: vec!["e".to_string()],
                    }]),
                },
                Section {
                    title: "Synonyms",
                    items: SectionItems::Words(vec!["y".to_string()]),
                },
                Section {
                    title: "Antonyms",
                    items: SectionItems::Antonyms(vec![Antonym {
                        term: "z".to_string(),
                        direct: true,
                        implications: Vec::new(),
                    }]),
                },
                Section {
                    title: "Kinds",
                    items: SectionItems::Tree(vec![TreeNode {
                        terms: vec!["t".to_string()],
                        children: vec![TreeNode {
                            terms: vec!["c".to_string()],
                            children: Vec::new(),
                        }],
                    }]),
                },
            ],
        };
        let bytes = entry(&sample);
        let mut expected: Vec<u8> = Vec::new();
        let s = |out: &mut Vec<u8>, v: &str| {
            out.extend_from_slice(&(v.len() as u32).to_le_bytes());
            out.extend_from_slice(v.as_bytes());
        };
        let n = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
        s(&mut expected, "x");
        n(&mut expected, 4);
        s(&mut expected, "Definitions");
        expected.push(0);
        n(&mut expected, 1);
        expected.push(0);
        s(&mut expected, "g");
        n(&mut expected, 1);
        s(&mut expected, "e");
        s(&mut expected, "Synonyms");
        expected.push(1);
        n(&mut expected, 1);
        s(&mut expected, "y");
        s(&mut expected, "Antonyms");
        expected.push(2);
        n(&mut expected, 1);
        s(&mut expected, "z");
        expected.push(1);
        n(&mut expected, 0);
        s(&mut expected, "Kinds");
        expected.push(3);
        n(&mut expected, 1);
        n(&mut expected, 1);
        s(&mut expected, "t");
        n(&mut expected, 1);
        n(&mut expected, 1);
        s(&mut expected, "c");
        n(&mut expected, 0);
        assert_eq!(bytes, expected);
    }
}
