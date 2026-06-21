// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The public lexical model a lookup returns, mirroring `spec/engine.md` section 2. Every string
//! a consumer sees is in display form (spaces, never underscores). The model carries no WordNet
//! types: the engine builds it, and consumers only read it.

/// The whole entry for a looked-up word: the resolved headword and its ordered sections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The resolved headword, with its case settled by the engine's last-match rule.
    pub term: String,
    /// The sections in emission order; a section that gathered nothing is absent entirely.
    pub sections: Vec<Section>,
}

/// A titled group of items of exactly one kind. The titles and their order are fixed by
/// `spec/engine.md` section 6.1, so the field is a static string the engine supplies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub title: &'static str,
    pub items: SectionItems,
}

/// The items of a section; the kind is fixed per title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SectionItems {
    /// Numbered meanings, grouped sense by sense.
    Definitions(Vec<Definition>),
    /// A flat list of terms: synonyms, derived forms, domains, and the like.
    Words(Vec<String>),
    /// Opposites, each direct or indirect, possibly carrying implication terms.
    Antonyms(Vec<Antonym>),
    /// A lexical hierarchy, such as is-a, kinds, or part-of, as nested nodes.
    Tree(Vec<TreeNode>),
    /// Etymology prose: one or more whitespace-collapsed paragraphs in source order, from the
    /// optional overlay of `spec/engine.md` section 6.10. Plain display text, never navigable.
    Etymology(Vec<String>),
    /// Sense translations: one block per looked-up sense, from the optional overlay of
    /// `spec/engine.md` section 6.11. Plain display text, never navigable.
    Translations(Vec<SenseTranslations>),
}

/// One looked-up sense's translations: the sense identified by its part of speech and gloss, as the
/// Definitions section shows them, and its words in other languages grouped by language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SenseTranslations {
    pub pos: Option<&'static str>,
    pub gloss: String,
    pub languages: Vec<LanguageWords>,
}

/// One language's words for a sense: the language's display name and the words in overlay order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageWords {
    pub language: String,
    pub words: Vec<String>,
}

/// One sense of a word: its part of speech (which may be absent), gloss, and example sentences.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    /// One of `noun`, `verb`, `adjective`, or `adverb`; a satellite adjective is shown as an
    /// adjective (`spec/engine.md` section 6.2).
    pub pos: Option<&'static str>,
    pub gloss: String,
    pub examples: Vec<String>,
}

/// An opposite of the looked-up word. It is either a direct antonym or an indirect one reached
/// through a similar sense, and it may carry related implication terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Antonym {
    pub term: String,
    pub direct: bool,
    pub implications: Vec<String>,
}

/// One node of a lexical hierarchy. A node is one synset, so it carries several terms, each a
/// word that can be looked up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeNode {
    pub terms: Vec<String>,
    pub children: Vec<TreeNode>,
}

impl TreeNode {
    /// The node's terms joined with `", "` exactly, used for display and for the top-level
    /// deduplication the engine does across senses.
    pub fn label(&self) -> String {
        self.terms.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_node_label_joins_terms_with_comma_space() {
        let node = TreeNode {
            terms: vec!["frozen dessert".to_string()],
            children: Vec::new(),
        };
        assert_eq!(node.label(), "frozen dessert");
        let node = TreeNode {
            terms: vec![
                "dessert".to_string(),
                "sweet".to_string(),
                "afters".to_string(),
            ],
            children: Vec::new(),
        };
        assert_eq!(node.label(), "dessert, sweet, afters");
    }
}
