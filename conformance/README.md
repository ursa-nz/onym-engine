<!--
SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Conformance kit

This directory pins the observable behaviour of the Onym engine. The
fixtures under `fixtures/` are owned golden masters: byte-exact outputs
of `onym-dump`, the conformance dumper built from the engine here, for
every word in `corpus.txt`, plus a fixed set of completion prefixes and
suggestion queries. Any engine implementation passes conformance when it
reproduces every fixture exactly.

The corpus has two parts: a hand-curated block of edge cases (indirect
antonyms, deep hyponym trees, multi-word lemmas, morphology exceptions,
misses with suggestions, and the documented quirk-fix words) and a
stratified sample of every 200th lemma from each WordNet index file.

## Running the check

```sh
conformance/run-conformance [DUMPER [ARG...]]
```

The dumper defaults to `onym-dump`, built from the Rust core in this
repository, and must accept `--dump WORD`, `--complete PREFIX` and
`--suggest WORD`. The script prints one PASS or FAIL line per fixture
and a summary, and exits nonzero on any difference, so CI can call it
directly.

The fixtures carry the two deliberate fixes in `spec/engine.md`. Two
dumpers over the one core pass everything and so cross-check the
bindings: `onym-dump`, the default, reaching the core directly, and
Onym's `onym-cli`, which reaches the same core through libonym and the
C FFI.

## Regenerating fixtures

```sh
conformance/gen-fixtures [DUMPER [ARG...]]
```

This rewrites `fixtures/` from scratch. Output is deterministic and
carries no timestamps.

## The etymology overlay (`etym/`)

The optional etymology overlay (`spec/engine.md` section 6.10) is proven apart from the WordNet kit
above, because its prose is UTF-8 where the database is ISO-8859-1, so `onym-dump` (which emits
ISO-8859-1) cannot render it. `etym/` holds a small, hand-authored test overlay (`etym/etym.onym`,
deliberately not from any Wiktionary dump so it never churns), a `etym/corpus.txt`, and UTF-8
`etym/fixtures/`. The check lives in `crates/onym-engine/tests/etymology.rs`: it opens an engine over
WordNet plus the test overlay, dumps each corpus word, and compares as UTF-8. Regenerate the fixtures
after a deliberate spec change with `ONYM_BLESS=1 cargo test -p onym-engine --test etymology`. The
same rule below applies: the spec changes first.

## Rule: fixtures change only via a spec change

The fixtures are the contract, not a cache. `spec/dump-format.md`
governs the output format and `spec/engine.md` governs the behaviour.
Do not regenerate and commit fixtures to make a failing implementation
pass. Change the spec first, then regenerate from the implementation
that follows the amended spec, and land both in the same change.
