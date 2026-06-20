// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The engine's acceptance tests, mirroring the Kotlin reference's parity suite: the dump of
//! every conformance corpus word, and every committed completion and suggestion fixture, must
//! match byte for byte. The fixtures drive the cases, so the prefix and suggestion lists live
//! only in `conformance/gen-fixtures` and its mirrors, never here. The base comes from the
//! `onym-data` submodule, prepared on demand; the tests skip themselves when the submodule or the
//! conformance kit is absent, so they are safe to run on a bare clone.

use onym_engine::{Engine, to_display_form, to_query_form};
use std::fs;
use std::path::{Path, PathBuf};

mod common;

fn conformance_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

fn open_engine() -> Option<Engine> {
    let data_dir = common::wordnet_base()?;
    if !conformance_dir().join("corpus.txt").is_file() {
        eprintln!("skipping: conformance kit not found");
        return None;
    }
    Some(Engine::open(data_dir).expect("the WordNet database opens"))
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

fn read_fixture(kind: &str, name: &str) -> String {
    let path = conformance_dir().join(format!("fixtures/{kind}/{}.txt", to_query_form(name)));
    latin1(&fs::read(&path).unwrap_or_else(|_| panic!("fixture {} exists", path.display())))
}

/// Report the first differing line of one mismatching word, the way the Kotlin parity tests do,
/// so a failure names the byte-level disagreement instead of dumping two whole entries.
fn first_diff(expected: &str, actual: &str) -> String {
    let expected_lines: Vec<&str> = expected.lines().collect();
    let actual_lines: Vec<&str> = actual.lines().collect();
    for i in 0..expected_lines.len().max(actual_lines.len()) {
        if expected_lines.get(i) != actual_lines.get(i) {
            return format!(
                "line {}: fixture {:?}, engine {:?}",
                i + 1,
                expected_lines.get(i),
                actual_lines.get(i)
            );
        }
    }
    "no line difference (trailing bytes differ)".to_string()
}

#[test]
fn every_corpus_dump_matches_its_fixture() {
    let Some(engine) = open_engine() else { return };
    let corpus = latin1(&fs::read(conformance_dir().join("corpus.txt")).expect("corpus reads"));

    let mut mismatches = Vec::new();
    let mut checked = 0;
    for line in corpus.lines() {
        let word = line.trim();
        if word.is_empty() || word.starts_with('#') {
            continue;
        }
        checked += 1;
        let expected = read_fixture("dump", word);
        let actual = engine.dump(word);
        if expected != actual {
            mismatches.push(format!("\"{word}\": {}", first_diff(&expected, &actual)));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} of {checked} corpus dumps differ:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}

#[test]
fn every_completion_fixture_matches() {
    let Some(engine) = open_engine() else { return };
    let dir = conformance_dir().join("fixtures/complete");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).expect("completion fixtures exist") {
        let path = entry.expect("fixture entry reads").path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // The fixture name is the query form of the prefix; the engine takes the typed form.
        let prefix = to_display_form(stem);
        let expected = latin1(&fs::read(&path).expect("fixture reads"));
        let actual: String = engine
            .complete(&prefix, 20)
            .iter()
            .map(|term| format!("{term}\n"))
            .collect();
        assert_eq!(actual, expected, "completion of \"{prefix}\"");
        checked += 1;
    }
    assert!(
        checked > 0,
        "no completion fixtures found in {}",
        dir.display()
    );
}

#[test]
fn every_suggestion_fixture_matches() {
    let Some(engine) = open_engine() else { return };
    let dir = conformance_dir().join("fixtures/suggest");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).expect("suggestion fixtures exist") {
        let path = entry.expect("fixture entry reads").path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let word = to_display_form(stem);
        let expected = latin1(&fs::read(&path).expect("fixture reads"));
        let actual: String = engine
            .suggest(&word, 10)
            .iter()
            .map(|term| format!("{term}\n"))
            .collect();
        assert_eq!(actual, expected, "suggestion for \"{word}\"");
        checked += 1;
    }
    assert!(
        checked > 0,
        "no suggestion fixtures found in {}",
        dir.display()
    );
}

#[test]
fn completion_and_suggestion_behave_at_the_edges() {
    let Some(engine) = open_engine() else { return };

    // Mirrors the Kotlin LemmaIndexTest: caps hold, matches share the prefix, order is
    // alphabetical, and case and underscores are folded.
    let matches = engine.complete("fel", 8);
    assert!(matches.len() <= 8);
    assert!(matches.iter().all(|m| m.starts_with("fel")));
    let mut sorted = matches.clone();
    sorted.sort();
    assert_eq!(matches, sorted);
    assert!(
        engine
            .complete("fel", 0)
            .contains(&"felicitous".to_string())
    );
    assert!(
        engine
            .complete("ICE CR", 20)
            .contains(&"ice cream".to_string())
    );

    // The nearest suggestion comes first, and a headword is never suggested for itself.
    assert_eq!(
        engine.suggest("beutiful", 5).first().map(String::as_str),
        Some("beautiful")
    );
    assert!(!engine.suggest("dog", 10).contains(&"dog".to_string()));

    // The lemma index backs the "surprise me" action: every index names a headword.
    assert!(engine.lemma_count() > 100_000);
    assert!(engine.lemma_at(0).is_some());
    assert!(engine.lemma_at(engine.lemma_count()).is_none());
}
