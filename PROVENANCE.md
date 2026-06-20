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

## Etymology overlay (`tools/etym-build`, `etym.onym`)

The optional etymology overlay the engine reads (`spec/engine.md` section 6.10) is built by
`tools/etym-build` from the English **wiktextract** dump published at
[kaikki.org](https://kaikki.org), a machine parse of the English
[Wiktionary](https://en.wiktionary.org). That etymology prose is written by Wiktionary's editors
and is licensed **CC-BY-SA-3.0** (the Wiktionary corpus is dual-licensed CC-BY-SA and GFDL).
wiktextract is the work of Tatu Ylonen.

The overlay is therefore a derivative of Wiktionary, not of this repository: the tool filters the
dump to English, joins it to the WordNet lemma set, cleans the prose, and writes the result. This
repository ships no overlay. A built `etym.onym` carries its own provenance and licence in its
leading-space header, so the attribution travels with the data wherever it is bundled (the Onym
Flatpak and the Onymdroid APK), and both apps credit Wiktionary in their About screens.
