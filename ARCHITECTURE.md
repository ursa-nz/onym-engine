<!--
SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Architecture

onym-engine is one engine for two applications: Onym on GNOME and Onymdroid on Android. The
repository holds the specification, the conformance kit, and a Rust workspace that renders the
specification as code.

## The layers

- The core crate, `crates/onym-engine`, holds the model, the morphology, the lemma index, and the
  lookup rules. It is std-only with zero dependencies.
- An ffi crate will expose the C ABI that libonym consumes. It arrives with the GNOME swap.
- A jni crate will expose a one-call serialised codec for Android, so a lookup crosses the JNI
  boundary once. It arrives with the Android swap.
- The conformance kit in `conformance/` gates everything. No layer changes behaviour without the
  kit agreeing.

## The data contract

The engine reads the WordNet 3.0 database files from a directory the caller names explicitly. It
opens them read-only and holds no global state, so two engines over two directories coexist in one
process.

## Threading

An engine is immutable once opened, and concurrent lookups from any number of threads are safe.
This is the deliberate break from the WordNet C library, whose global state forced every call onto
one thread.

## The spec is the contract

`spec/engine.md` defines what the engine does. The crate is one rendering of that contract, and
the conformance kit measures the rendering against it. When the code and the spec disagree, either
the code is wrong or the spec is fixed first.
