<!--
SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# onym-engine

onym-engine is the WordNet-family lookup engine behind the Onym GTK and Android apps, written in
Rust. It is std-only with zero dependencies. One crate carries the model, the morphology, the lemma
index, and the lookup rules, so both applications share one engine with one behaviour. It reads Open
English WordNet (the base is pinned by the `onym-data` submodule and bumpable), and replaces the
abandoned WordNet C library (libwordnet), the engine Onym vendored from Artha, and the extJWNL
library the Android app used.

## Status

Pre-release. `spec/engine.md` is the contract; the core crate in `crates/onym-engine` implements
it, and the conformance kit in `conformance/` proves the implementation against it. The C ABI
crate in `crates/onym-engine-ffi` backs the GTK app, and the JNI crate in `crates/onym-engine-jni`
backs the Android app.

## Data

The engine reads the WordNet data files from a directory the caller supplies. The reference and test
data come from the `onym-data` submodule, which vendors the pinned base graph and the overlays;
`onym-data/prepare.sh` materialises them into a directory. This repository ships no database.

## Building and testing

The crate builds with stable Rust and Cargo:

```
cargo build
cargo test
```

The conformance kit proves the engine against the spec, given a WordNet database directory:

```
conformance/run-conformance
```

## Licence

onym-engine is free software under the GPL, version 3 or later. Full licence texts live in
[LICENSES/](LICENSES/), and the repository is [REUSE](https://reuse.software)-compliant.

## Acknowledgements

- Word data from [Open English WordNet](https://github.com/globalwordnet/english-wordnet), the
  actively maintained WordNet derived from Princeton University's database, under CC-BY-4.0.
- Etymology prose, for the optional overlay, from [Wiktionary](https://en.wiktionary.org) via the
  [wiktextract](https://kaikki.org) dataset, under CC-BY-SA-3.0.
- Engine behaviour derived from [Artha](https://artha.sourceforge.net), an earlier WordNet
  thesaurus by Sundaram Ramaswamy.
- Crafted in Narrm, in Australia, with respect to the Wurundjeri Woi-wurrung people of the Kulin
  Nation, their language, and their continuing connection to this Country.
