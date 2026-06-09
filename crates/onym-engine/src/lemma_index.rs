// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The lemma index: every WordNet headword once, lowercased and in display form, sorted by plain
//! byte order. It reads the `index.*` files directly and depends on nothing else, which makes
//! prefix completion a binary search and spelling suggestions a bounded edit-distance scan.
//! Transcribed from the Kotlin reference (`LemmaIndex.kt`), which mirrors Onym's `wn-index.c`,
//! per `spec/engine.md` section 8.

use crate::OpenError;
use crate::textforms::{display_lower, edit_distance, latin1_to_string};
use std::fs;
use std::path::Path;

const INDEX_FILES: [&str; 4] = ["index.noun", "index.verb", "index.adj", "index.adv"];

pub(crate) struct LemmaIndex {
    lemmas: Vec<String>,
}

impl LemmaIndex {
    /// Build the index from the WordNet `index.*` files in `data_dir`. Absent files contribute
    /// nothing, mirroring the reference; the engine's open has already required them.
    pub(crate) fn build(data_dir: &Path) -> Result<LemmaIndex, OpenError> {
        let mut raw = Vec::new();
        for name in INDEX_FILES {
            let path = data_dir.join(name);
            if !path.is_file() {
                continue;
            }
            let bytes = fs::read(&path).map_err(|source| OpenError { file: path, source })?;
            let text = latin1_to_string(&bytes);
            for line in text.lines() {
                // Lines beginning with a space are the licence header, not lemmas.
                if line.is_empty() || line.starts_with(' ') {
                    continue;
                }
                let token = line.split_once(' ').map_or(line, |(token, _)| token);
                raw.push(display_lower(token));
            }
        }
        // Byte order matches WordNet's strcmp, which is what the C oracle sorted by.
        raw.sort_unstable();
        raw.dedup();
        Ok(LemmaIndex { lemmas: raw })
    }

    /// Headwords beginning with `prefix`, in lowercased display form and alphabetical order,
    /// capped at `max` (0 means no cap).
    pub(crate) fn complete(&self, prefix: &str, max: usize) -> Vec<String> {
        if prefix.is_empty() {
            return Vec::new();
        }
        let needle = display_lower(prefix);
        if needle.is_empty() {
            return Vec::new();
        }

        let mut result = Vec::new();
        // partition_point is the lower bound: the first lemma not below the needle.
        let mut i = self
            .lemmas
            .partition_point(|lemma| lemma.as_str() < needle.as_str());
        while i < self.lemmas.len() && (max == 0 || result.len() < max) {
            if !self.lemmas[i].starts_with(&needle) {
                break;
            }
            result.push(self.lemmas[i].clone());
            i += 1;
        }
        result
    }

    /// Headwords close to `word` by edit distance, for a "did you mean" prompt. Candidates differ
    /// in length by at most two and in edit distance by one or two, ordered by distance and then
    /// alphabetically, capped at `max` (0 means no cap). Exact matches (distance zero) are
    /// excluded.
    pub(crate) fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        if word.is_empty() {
            return Vec::new();
        }
        let needle = display_lower(word);
        if needle.is_empty() {
            return Vec::new();
        }
        let needle_len = needle.chars().count();

        let mut candidates: Vec<(usize, &String)> = Vec::new();
        for lemma in &self.lemmas {
            // Lengths are in characters, as the edit distance counts, so a Latin-1 lemma is not
            // penalised for its byte width.
            if lemma.chars().count().abs_diff(needle_len) > 2 {
                continue;
            }
            let distance = edit_distance(&needle, lemma);
            if (1..=2).contains(&distance) {
                candidates.push((distance, lemma));
            }
        }
        candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
        let limit = if max == 0 {
            candidates.len()
        } else {
            max.min(candidates.len())
        };
        candidates.truncate(limit);
        candidates
            .into_iter()
            .map(|(_, term)| term.clone())
            .collect()
    }

    /// How many headwords the index holds.
    pub(crate) fn lemma_count(&self) -> usize {
        self.lemmas.len()
    }

    /// The headword at `index` in sorted order, in lowercased display form. With a caller-chosen
    /// random index this is the "surprise me" action; the engine takes no randomness itself.
    pub(crate) fn lemma_at(&self, index: usize) -> Option<&str> {
        self.lemmas.get(index).map(String::as_str)
    }
}
