<!--
SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Provenance

This file records where engine code that did not originate in this repository came from, so the
licence trail stays auditable alongside the REUSE metadata.

## Morphology (`crates/onym-engine/src/morphology.rs`)

The morphology module is a port of the WordNet 3.0 C library's `lib/morph.c`: the functions
`morphstr`, `morphword`, `wordbase`, `exc_lookup`, `hasprep`, `morphprep`, and `cntwords`, along
with the `sufx[]`, `addr[]`, `offsets[]`, `cnts[]`, and `prepositions[]` tables. The port was made
by way of the Kotlin reference implementation in the sibling Onymdroid repository
(`core/src/main/kotlin/nz/ursa/onymdroid/core/Morphology.kt`), which was itself transcribed from
`morph.c` and proven against the C oracle byte for byte.

WordNet is copyright 2006 Princeton University and is distributed under its own permissive
licence, included here as [LICENSES/LicenseRef-WordNet.txt](LICENSES/LicenseRef-WordNet.txt). The
file therefore carries both the Princeton copyright notice and the repository's GPL-3.0-or-later
licence; the combined expression is recorded in its SPDX header.

## Engine behaviour

The lookup rules in `spec/engine.md` distil the behaviour of Artha (GPL-2, by Sundaram Ramaswamy)
and the WordNet C library, as reimplemented twice: once in Onym's vendored `wni.c` and once in
Onymdroid's Kotlin core. The Rust code in this repository is a fresh transcription of the Kotlin
reference against that specification; no Artha or libwordnet source text was copied into it. The
morphology module above is the single exception, being a port of `morph.c` itself.
