// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The stable text rendering of a lookup, exactly as `spec/dump-format.md` fixes it. It exists so
//! the engine can be diffed, byte for byte, against the conformance fixtures; applications render
//! the model directly instead.

use crate::model::{Antonym, Definition, Entry, SectionItems, SenseTranslations, TreeNode};

pub(crate) fn render(entry: &Entry) -> String {
    let mut out = String::new();
    out.push_str("term: ");
    out.push_str(&entry.term);
    out.push('\n');
    for section in &entry.sections {
        out.push('[');
        out.push_str(section.title);
        out.push_str("]\n");
        match &section.items {
            SectionItems::Definitions(items) => {
                for definition in items {
                    render_definition(definition, &mut out);
                }
            }
            SectionItems::Words(items) => {
                for term in items {
                    out.push_str("  - ");
                    out.push_str(term);
                    out.push('\n');
                }
            }
            SectionItems::Antonyms(items) => {
                for antonym in items {
                    render_antonym(antonym, &mut out);
                }
            }
            SectionItems::Tree(items) => {
                for node in items {
                    render_tree_node(node, 0, &mut out);
                }
            }
            SectionItems::Etymology(paragraphs) => {
                for paragraph in paragraphs {
                    out.push_str("  - ");
                    out.push_str(paragraph);
                    out.push('\n');
                }
            }
            SectionItems::Translations(blocks) => {
                for block in blocks {
                    render_sense_translations(block, &mut out);
                }
            }
        }
    }
    out
}

fn render_definition(definition: &Definition, out: &mut String) {
    match definition.pos {
        Some(pos) => {
            out.push_str("  - (");
            out.push_str(pos);
            out.push_str(") ");
        }
        None => out.push_str("  - "),
    }
    out.push_str(&definition.gloss);
    out.push('\n');
    for example in &definition.examples {
        out.push_str("      \"");
        out.push_str(example);
        out.push_str("\"\n");
    }
}

fn render_sense_translations(block: &SenseTranslations, out: &mut String) {
    match block.pos {
        Some(pos) => {
            out.push_str("  - (");
            out.push_str(pos);
            out.push_str(") ");
        }
        None => out.push_str("  - "),
    }
    out.push_str(&block.gloss);
    out.push('\n');
    for language in &block.languages {
        out.push_str("      ");
        out.push_str(&language.language);
        out.push_str(": ");
        out.push_str(&language.words.join(", "));
        out.push('\n');
    }
}

fn render_antonym(antonym: &Antonym, out: &mut String) {
    out.push_str("  - ");
    out.push_str(&antonym.term);
    out.push_str(if antonym.direct {
        " (direct)\n"
    } else {
        " (indirect)\n"
    });
    for implication in &antonym.implications {
        out.push_str("      -> ");
        out.push_str(implication);
        out.push('\n');
    }
}

fn render_tree_node(node: &TreeNode, depth: usize, out: &mut String) {
    for _ in 0..=depth {
        out.push_str("  ");
    }
    out.push_str("- ");
    out.push_str(&node.label());
    out.push('\n');
    for child in &node.children {
        render_tree_node(child, depth + 1, out);
    }
}
