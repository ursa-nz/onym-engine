// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The translations overlay's conformance, kept apart from the WordNet kit because the main
//! fixtures are generated overlay-free, so the Translations section never appears in them
//! (`engine.md` section 6.11). It opens an engine over a temporary directory of WordNet symlinks
//! plus the committed test overlay (`conformance/omw/omw.onym`, a trimmed slice keyed to the base's
//! synset offsets), dumps every word of `conformance/omw/corpus.txt`, and compares each dump, as
//! UTF-8 characters, against its fixture. The base comes from the `onym-data` submodule. The fixtures
//! regenerate with `ONYM_BLESS=1`; the test skips itself when the submodule is absent, so it is safe
//! to run on a bare clone. It proves the engine reads the overlay, keys it by each gathered sense's
//! synset, and renders the Translations section in place, while a word whose senses the overlay does
//! not carry shows none.

use onym_engine::{Engine, to_query_form};
use std::fs;
use std::path::{Path, PathBuf};

mod common;

fn omw_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/omw")
}

/// A temporary data directory: every WordNet file symlinked in, plus the committed test overlay as
/// `omw.onym`. Removed on drop. The overlay must be a real file, not a symlink into the source tree,
/// because the loader reads it as the data directory's own.
struct Fixtureset {
    dir: PathBuf,
}

static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

impl Fixtureset {
    fn build(base: &Path) -> Fixtureset {
        let unique = format!(
            "onym-omw-conformance-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let dir = std::env::temp_dir().join(unique);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp data dir");
        for entry in fs::read_dir(base).expect("read WordNet dir") {
            let path = entry.expect("WordNet entry").path();
            let name = path.file_name().expect("file name");
            std::os::unix::fs::symlink(&path, dir.join(name)).expect("symlink WordNet file");
        }
        fs::copy(omw_dir().join("omw.onym"), dir.join("omw.onym")).expect("copy test overlay");
        Fixtureset { dir }
    }
}

impl Drop for Fixtureset {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn corpus_words() -> Vec<String> {
    let text = fs::read_to_string(omw_dir().join("corpus.txt")).expect("translations corpus reads");
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// The first differing line, named the way the WordNet kit names one, so a failure points at the
/// byte-level disagreement rather than dumping two whole entries.
fn first_diff(expected: &str, actual: &str) -> String {
    let expected: Vec<&str> = expected.lines().collect();
    let actual: Vec<&str> = actual.lines().collect();
    for i in 0..expected.len().max(actual.len()) {
        if expected.get(i) != actual.get(i) {
            return format!(
                "line {}: fixture {:?}, engine {:?}",
                i + 1,
                expected.get(i),
                actual.get(i)
            );
        }
    }
    "no line difference (trailing bytes differ)".to_string()
}

#[test]
fn every_translation_dump_matches_its_fixture() {
    let Some(base) = common::wordnet_base() else {
        return;
    };
    let fixtures = Fixtureset::build(base);
    let engine = Engine::open(&fixtures.dir).expect("engine opens over the test overlay");
    let bless = std::env::var_os("ONYM_BLESS").is_some();
    let fixtures_dir = omw_dir().join("fixtures");
    if bless {
        fs::create_dir_all(&fixtures_dir).expect("create fixtures dir");
    }

    let mut mismatches = Vec::new();
    for word in corpus_words() {
        let actual = engine.dump(&word);
        let path = fixtures_dir.join(format!("{}.txt", to_query_form(&word)));
        if bless {
            fs::write(&path, &actual).expect("write fixture");
            continue;
        }
        let expected = fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!("fixture {} exists (run with ONYM_BLESS=1)", path.display())
        });
        if expected != actual {
            mismatches.push(format!("\"{word}\": {}", first_diff(&expected, &actual)));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} translation dumps differ:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}

#[test]
fn the_overlay_is_additive_and_gated() {
    let Some(base) = common::wordnet_base() else {
        return;
    };
    let fixtures = Fixtureset::build(base);
    let with_overlay = Engine::open(&fixtures.dir).expect("engine opens over the test overlay");
    let plain = Engine::open(base).expect("engine opens over plain WordNet");

    // A word in the overlay gains a Translations section, immediately after Synonyms, and the rest of
    // its entry is otherwise the plain WordNet dump with that one section spliced in.
    let dump = with_overlay.dump("dog");
    let synonyms = dump.find("[Synonyms]").expect("dog has synonyms");
    let translations = dump
        .find("[Translations]")
        .expect("dog gains a Translations section");
    let next = dump.find("[Is a kind of]").expect("dog has an is-a tree");
    assert!(
        synonyms < translations && translations < next,
        "Translations must sit after Synonyms and before the relation trees"
    );
    // The section carries the language-grouped words, in their own scripts and accents.
    assert!(dump.contains("Italian: cane"));
    assert!(dump.contains("Portuguese: cão"));

    // A word whose senses the overlay does not carry is byte-for-byte the plain WordNet dump.
    assert!(!with_overlay.dump("serendipity").contains("[Translations]"));
    assert_eq!(with_overlay.dump("serendipity"), plain.dump("serendipity"));

    // "dogs" resolves to "dog" by morphology and still carries the senses' translations.
    assert!(with_overlay.dump("dogs").contains("[Translations]"));
}
