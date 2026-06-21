// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-FileCopyrightText: 2006 Princeton University
// SPDX-License-Identifier: GPL-3.0-or-later AND LicenseRef-WordNet

//! WordNet's morphology, ported faithfully from the WordNet 3.0 C library (`lib/morph.c`:
//! `morphstr`, `morphword`, `wordbase`, `exc_lookup`, `hasprep`, `morphprep`), via the Kotlin
//! reference (`Morphology.kt`). Reader libraries over-generate relative to `morphstr` (robed to
//! rob, puss to several senses), so the engine inflects words itself and reproduces `morphstr` to
//! the letter. See `PROVENANCE.md` at the repository root for the derivation record.
//!
//! The algorithm: the exception files are consulted first; a hit returns every base form listed
//! on the word's line. Failing that, the per-part-of-speech suffix tables are applied in order,
//! the first candidate that exists in the index winning; a `-ful` noun is morphed on its stem and
//! re-suffixed; and a collocation is morphed component by component and recombined on its
//! original separators, with a verb-plus-preposition phrase handled specially (`morphprep`).
//!
//! Candidate existence is index reading, not morphology, so it stays outside this module: every
//! entry point takes an `is_defined` callback, which the dictionary source wires to the WordNet
//! index (WordNet's `is_defined` / `in_wn`). Part-of-speech codes follow WordNet's numbering:
//! 1 noun, 2 verb, 3 adjective, 4 adverb. The engine also calls `morphstr` with the part of
//! speech shifted down by one (Onym's Ubuntu work-around in `wni.c`), which passes code 0 for a
//! noun; code 0 has no exception file and no suffix rules, so it always yields nothing, exactly
//! as the C library does.

use crate::textforms::{ascii_lower, decode};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub(crate) const NOUN: usize = 1;
pub(crate) const VERB: usize = 2;
pub(crate) const ADJ: usize = 3;
pub(crate) const ADV: usize = 4;

/// The in-WordNet test `morphstr` consults: true exactly when any getindex variant of the
/// candidate is a headword in the part of speech's index. Code 0 never matches.
pub(crate) type IsDefined<'a> = &'a dyn Fn(usize, &str) -> bool;

// The exception file names, indexed by WordNet part-of-speech number (morph.c's partnames[]).
const EXC_FILES: [(usize, &str); 4] = [
    (NOUN, "noun.exc"),
    (VERB, "verb.exc"),
    (ADJ, "adj.exc"),
    (ADV, "adv.exc"),
];

// morph.c's sufx[]/addr[] suffix-rule tables: noun rules at 0, verb at 8, adjective at 16.
const SUFX: [&str; 20] = [
    "s", "ses", "xes", "zes", "ches", "shes", "men", "ies", "s", "ies", "es", "es", "ed", "ed",
    "ing", "ing", "er", "est", "er", "est",
];
const ADDR: [&str; 20] = [
    "", "s", "x", "z", "ch", "sh", "man", "y", "", "y", "e", "", "e", "", "e", "", "", "", "e", "e",
];

// morph.c's offsets[]/cnts[] into the tables, indexed by part-of-speech number (0 is the
// degenerate shifted-noun case: no rules). Adverbs use only the exception list.
const OFFSETS: [usize; 5] = [0, 0, 8, 16, 0];
const COUNTS: [usize; 5] = [0, 8, 8, 4, 0];

// morph.c's preposition table, used to spot a verb-plus-preposition phrase.
const PREPOSITIONS: [&str; 15] = [
    "to", "at", "of", "on", "off", "in", "out", "up", "down", "from", "with", "into", "for",
    "about", "between",
];

pub(crate) struct Morphology {
    // The exception lists keyed by part-of-speech number; index 0 stays empty by construction.
    exceptions: [HashMap<String, Vec<String>>; 5],
}

impl Morphology {
    /// Load the exception files from `data_dir`; absent files contribute nothing.
    pub(crate) fn load(data_dir: &Path) -> std::io::Result<Morphology> {
        let mut exceptions: [HashMap<String, Vec<String>>; 5] = Default::default();
        for (pos_code, name) in EXC_FILES {
            let path = data_dir.join(name);
            if !path.is_file() {
                continue;
            }
            // The exception files are UTF-8, whitespace-separated: the inflected headword,
            // then its base forms. Lines starting with a space are skipped.
            let text = decode(&fs::read(path)?);
            for line in text.lines() {
                if line.is_empty() || line.starts_with(' ') {
                    continue;
                }
                let parts: Vec<&str> = line.trim().split(' ').collect();
                if parts.len() >= 2 {
                    exceptions[pos_code].insert(
                        parts[0].to_string(),
                        parts[1..].iter().map(|p| p.to_string()).collect(),
                    );
                }
            }
        }
        Ok(Morphology { exceptions })
    }

    /// The base forms of `origstr` for `pos_code`, in the order WordNet's `morphstr` returns them
    /// (the sequence of its first call and its subsequent strtok-style calls). Empty when there
    /// are none.
    pub(crate) fn morphstr(
        &self,
        origstr: &str,
        pos_code: usize,
        is_defined: IsDefined,
    ) -> Vec<String> {
        let str_ = ascii_lower(origstr).replace(' ', "_");
        let mut result = Vec::new();

        // First try the exception list: a hit returns every base form on the word's line.
        let exc_words = self.exc_lookup(&str_, pos_code);
        if !exc_words.is_empty() && exc_words[0] != str_ {
            result.extend(exc_words.iter().cloned());
            return result;
        }

        // Then try a straight morph of the whole string (verbs skip this and go via the loop
        // below).
        if pos_code != VERB
            && let Some(word) = self.morphword(&str_, pos_code, is_defined)
            && word != str_
        {
            result.push(word);
            return result;
        }

        if pos_code == VERB && cntwords(&str_, '_') > 1 && hasprep(&str_) {
            // A verb followed by a preposition: morph the verb and re-attach the rest.
            if let Some(phrase) = self.morphprep(&str_, is_defined) {
                result.push(phrase);
            }
            return result;
        }

        // Otherwise morph each component of the (possibly single-word) string and recombine on
        // its original separators. For a single word this just morphs the whole word; either way
        // the recombined form must differ from the input and be defined, exactly as morphstr
        // requires.
        let mut searchstr = String::new();
        let mut st_idx = 0;
        let mut remaining = cntwords(&str_, '-');
        while remaining > 1 {
            let underscore = str_[st_idx..].find('_').map(|p| p + st_idx);
            let hyphen = str_[st_idx..].find('-').map(|p| p + st_idx);
            let (end_idx, append) = match (underscore, hyphen) {
                (Some(u), Some(h)) if u < h => (u, '_'),
                (Some(u), None) => (u, '_'),
                (_, Some(h)) => (h, '-'),
                (None, None) => return result,
            };
            let component = &str_[st_idx..end_idx];
            match self.morphword(component, pos_code, is_defined) {
                Some(morphed) => searchstr.push_str(&morphed),
                None => searchstr.push_str(component),
            }
            searchstr.push(append);
            st_idx = end_idx + 1;
            remaining -= 1;
        }
        let last_component = &str_[st_idx..];
        match self.morphword(last_component, pos_code, is_defined) {
            Some(morphed) => searchstr.push_str(&morphed),
            None => searchstr.push_str(last_component),
        }
        if searchstr != str_ && is_defined(pos_code, &searchstr) {
            result.push(searchstr);
        }
        result
    }

    /// WordNet's `morphword`: the base form of a single `word` in `pos_code`, or nothing.
    fn morphword(&self, word: &str, pos_code: usize, is_defined: IsDefined) -> Option<String> {
        if word.is_empty() {
            return None;
        }

        // The exception list wins, and is the only source for adverbs.
        if let Some(first) = self.exc_lookup(word, pos_code).first() {
            return Some(first.clone());
        }
        if pos_code == ADV {
            return None;
        }

        let mut stem = word;
        let mut end = "";
        if pos_code == NOUN {
            if word.ends_with("ful") {
                // The suffix starts at the last 'f', which "ful" guarantees exists.
                stem = &word[..word.rfind('f').unwrap_or(0)];
                end = "ful";
            } else if word.ends_with("ss") || word.chars().count() <= 2 {
                return None;
            }
        }

        let offset = OFFSETS[pos_code];
        for i in 0..COUNTS[pos_code] {
            let candidate = wordbase(stem, i + offset);
            if candidate != stem && is_defined(pos_code, &candidate) {
                return Some(candidate + end);
            }
        }
        None
    }

    /// WordNet's `morphprep`: assume the first word of `phrase` is a verb, strip it, and try
    /// morphs of the verb (exception list then suffix rules) with the rest re-attached, returning
    /// the first phrase that is defined. A three-or-more-word phrase also tries morphing the
    /// trailing noun.
    fn morphprep(&self, phrase: &str, is_defined: IsDefined) -> Option<String> {
        let first_underscore = phrase.find('_')?;
        let last_underscore = phrase.rfind('_')?;
        let rest = &phrase[first_underscore..];
        let mut end: Option<String> = None;
        if first_underscore != last_underscore
            && let Some(last_word) =
                self.morphword(&phrase[last_underscore + 1..], NOUN, is_defined)
        {
            end = Some(format!(
                "{}{}",
                &phrase[first_underscore..=last_underscore],
                last_word
            ));
        }

        let word = &phrase[..first_underscore];
        if word.chars().any(|c| !c.is_alphanumeric()) {
            return None;
        }

        if let Some(exc_word) = self.exc_lookup(word, VERB).first()
            && exc_word != word
        {
            let candidate = format!("{exc_word}{rest}");
            if is_defined(VERB, &candidate) {
                return Some(candidate);
            }
            if let Some(end) = &end {
                let candidate = format!("{exc_word}{end}");
                if is_defined(VERB, &candidate) {
                    return Some(candidate);
                }
            }
        }

        for i in 0..COUNTS[VERB] {
            let base = wordbase(word, i + OFFSETS[VERB]);
            if base != word {
                let candidate = format!("{base}{rest}");
                if is_defined(VERB, &candidate) {
                    return Some(candidate);
                }
                if let Some(end) = &end {
                    let candidate = format!("{base}{end}");
                    if is_defined(VERB, &candidate) {
                        return Some(candidate);
                    }
                }
            }
        }

        let candidate = format!("{word}{rest}");
        if phrase != candidate {
            return Some(candidate);
        }
        if let Some(end) = &end {
            let candidate = format!("{word}{end}");
            if phrase != candidate {
                return Some(candidate);
            }
        }
        None
    }

    /// Every base form listed for `word` on its line of `pos_code`'s exception file; empty if
    /// absent.
    fn exc_lookup(&self, word: &str, pos_code: usize) -> &[String] {
        self.exceptions[pos_code]
            .get(word)
            .map_or(&[], Vec::as_slice)
    }
}

/// WordNet's `wordbase`: strip suffix `ender` from `word` and append its replacement, if it
/// matches.
fn wordbase(word: &str, ender: usize) -> String {
    match word.strip_suffix(SUFX[ender]) {
        Some(stripped) => format!("{stripped}{}", ADDR[ender]),
        None => word.to_string(),
    }
}

/// WordNet's `hasprep`: true when one of `phrase`'s words after the first is a known preposition.
fn hasprep(phrase: &str) -> bool {
    let mut from = 0;
    loop {
        let Some(found) = phrase[from..].find('_') else {
            return false;
        };
        let after = from + found + 1;
        for prep in PREPOSITIONS {
            if phrase[after..].starts_with(prep) {
                let boundary = after + prep.len();
                if boundary == phrase.len() || phrase[boundary..].starts_with('_') {
                    return true;
                }
            }
        }
        from = after;
    }
}

/// WordNet's `cntwords`: the number of words in `s` split on spaces, underscores, or `separator`.
pub(crate) fn cntwords(s: &str, separator: char) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut count = 0;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == separator || chars[i] == ' ' || chars[i] == '_' {
            count += 1;
            while i < chars.len() && (chars[i] == separator || chars[i] == ' ' || chars[i] == '_') {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    count + 1
}
