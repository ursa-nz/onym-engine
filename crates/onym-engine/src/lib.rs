// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The WordNet 3.0 engine behind Onym and Onymdroid.
//!
//! This crate owns the model, the morphology, the lemma index, and the lookup rules. It reads
//! the WordNet database files from a directory the caller supplies and holds no global state.
//! Its behaviour is specified by `spec/engine.md` and proven by the conformance kit in
//! `conformance/`.

#![forbid(unsafe_code)]
