// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The WordNet 3.0 engine behind Onym and Onymdroid.
//!
//! This crate owns the model, the morphology, the lemma index, and the lookup rules. It reads
//! the WordNet database files from a directory the caller supplies and holds no global state.
//! Its behaviour is specified by `spec/engine.md` and proven by the conformance kit in
//! `conformance/`.

#![forbid(unsafe_code)]

mod data;
mod dump;
mod etymology;
mod lemma_index;
mod lookup;
mod model;
mod morphology;
mod textforms;
mod translations;
mod verb_examples;

pub use model::{
    Antonym, Definition, Entry, LanguageWords, Section, SectionItems, SenseTranslations, TreeNode,
};
pub use textforms::{edit_distance, to_display_form, to_query_form};

use data::DictSource;
use lemma_index::LemmaIndex;
use std::fmt;
use std::path::{Path, PathBuf};

/// A failed engine open: a missing directory or an unreadable required file. This is distinct
/// from a word that is simply not in WordNet, which a lookup reports by returning nothing.
#[derive(Debug)]
pub struct OpenError {
    /// The file (or directory) that could not be read.
    pub file: PathBuf,
    /// The underlying I/O error.
    pub source: std::io::Error,
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot read {}: {}", self.file.display(), self.source)
    }
}

impl std::error::Error for OpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// The engine's public face. It resolves a word to the [`Entry`] model, completes a typed prefix,
/// and suggests near-misses for a missed word.
///
/// An engine is immutable after [`Engine::open`] and safe for concurrent lookups, completions,
/// and suggestions from any number of threads. It keeps no global state of any kind: two engines
/// over two directories coexist in one process.
pub struct Engine {
    source: DictSource,
    index: LemmaIndex,
}

// The suggestion cap of the dump's Did-you-mean line, per spec/dump-format.md.
const DUMP_SUGGEST_CAP: usize = 5;

impl Engine {
    /// Open an engine over the WordNet 3.0 database in `data_dir`. The directory is read in
    /// place, read-only; no environment variables are consulted. The `index.*` and `data.*`
    /// files are required; the exception files, `cntlist.rev`, and the verb example tables are
    /// optional, per `spec/engine.md` section 10.
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Engine, OpenError> {
        let dir = data_dir.as_ref();
        Ok(Engine {
            source: DictSource::open(dir)?,
            index: LemmaIndex::build(dir)?,
        })
    }

    /// Look `word` up. Returns nothing when the word is simply not in WordNet (distinct from a
    /// missing database, which [`Engine::open`] reports).
    pub fn lookup(&self, word: &str) -> Option<Entry> {
        lookup::Lookup {
            source: &self.source,
        }
        .lookup(word)
    }

    /// Headwords beginning with `prefix`, in lowercased display form and alphabetical order,
    /// capped at `max` (0 means no cap).
    pub fn complete(&self, prefix: &str, max: usize) -> Vec<String> {
        self.index.complete(prefix, max)
    }

    /// Spelling suggestions for a missed `word`, ordered by edit distance then alphabetically,
    /// capped at `max` (0 means no cap).
    pub fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        self.index.suggest(word, max)
    }

    /// How many headwords the lemma index holds.
    pub fn lemma_count(&self) -> usize {
        self.index.lemma_count()
    }

    /// The headword at `index` in sorted order, in lowercased display form. A caller-chosen
    /// random index makes this the "surprise me" action; the engine takes no randomness itself.
    pub fn lemma_at(&self, index: usize) -> Option<&str> {
        self.index.lemma_at(index)
    }

    /// Render `word`'s entry in the stable dump format of `spec/dump-format.md`, including the
    /// not-found form with its capped Did-you-mean line. This is the conformance surface; the
    /// applications render the model directly instead.
    pub fn dump(&self, word: &str) -> String {
        match self.lookup(word) {
            Some(entry) => dump::render(&entry),
            None => {
                let mut out = format!("No entry for \"{word}\".\n");
                let suggestions = self.suggest(word, DUMP_SUGGEST_CAP);
                if !suggestions.is_empty() {
                    out.push_str("Did you mean: ");
                    out.push_str(&suggestions.join(", "));
                    out.push('\n');
                }
                out
            }
        }
    }
}
