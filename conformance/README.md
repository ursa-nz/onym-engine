<!--
SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Conformance kit

This directory pins the observable behaviour of the Onym engine. The
fixtures under `fixtures/` are byte-exact outputs of the reference
dumper for every word in `corpus.txt`, plus a fixed set of completion
prefixes and suggestion queries. Any engine implementation passes
conformance when it reproduces every fixture exactly.

The corpus has two parts: a hand-curated block of edge cases (indirect
antonyms, deep hyponym trees, multi-word lemmas, morphology exceptions,
misses with suggestions, and the documented quirk-fix words) and a
stratified sample of every 200th lemma from each WordNet index file.

## Running the check

```sh
conformance/run-conformance [DUMPER [ARG...]]
```

The dumper defaults to the sibling `onym-cli` build and must accept
`--dump WORD`, `--complete PREFIX` and `--suggest WORD`. The script
prints one PASS or FAIL line per fixture and a summary, and exits
nonzero on any difference, so CI can call it directly.

The fixtures carry the two deliberate fixes in `spec/engine.md`, and
they were regenerated from the quirk-fixed Onymdroid engine, currently
the reference implementation. Until Onym adopts the shared core, its
`onym-cli` predates those fixes and fails exactly the affected
fixtures: the dumps for shore bird, hot dog, pica-pica, door, sing and
sang. A conformant implementation passes everything.

## Regenerating fixtures

```sh
conformance/gen-fixtures [DUMPER [ARG...]]
```

This rewrites `fixtures/` from scratch. Output is deterministic and
carries no timestamps.

## Rule: fixtures change only via a spec change

The fixtures are the contract, not a cache. `spec/dump-format.md`
governs the output format and `spec/engine.md` governs the behaviour.
Do not regenerate and commit fixtures to make a failing implementation
pass. Change the spec first, then regenerate from the implementation
that follows the amended spec, and land both in the same change.
