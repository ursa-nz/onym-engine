<!--
SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Dump format

The stable text rendering of a lookup, used for conformance testing. The conformance dumper
`onym-dump` in this repository emits it byte for byte, and the owned golden masters under
`conformance/fixtures/` are generated from it. The format originated as the output of Onym's
`onym-cli` and was the contract the Kotlin port was proven against; it is specified here in its own
right.

**This file versions the format.** Fixtures regenerate only when this file or `engine.md` changes. The *content*
of a dump (which sections appear, their items, their order) is governed by `engine.md`, including
its two deliberate fixes; this file fixes only the rendering. A fixture mismatch therefore means
either an engine bug or an intentional `engine.md` change, never formatting drift.

## General rules

- Every line ends with a single LF. There are no blank lines and no trailing whitespace beyond
  what the rules below produce.
- Strings pass through from the engine unchanged. The database is UTF-8, so the dump is UTF-8 and
  carries accented lemmas and glosses verbatim; the conformance dumper `onym-dump` reads and writes
  UTF-8 throughout. Fixtures are UTF-8 and compare as bytes, which for UTF-8 is the same as comparing
  characters. The Etymology section (`engine.md` section 6.10) and the Translations section
  (`engine.md` section 6.11) are no different in encoding, but the main fixtures are generated
  overlay-free, so neither appears in them; each is proven by a separate test over a committed test
  overlay (`crates/onym-engine/tests/etymology.rs` and `crates/onym-engine/tests/translations.rs`).
- All terms and labels are in display form (spaces, never underscores).

## The entry dump (`WORD` and `--dump WORD`)

A bare word argument and `--dump` are identical; `--dump` exists so snapshots are named
(onym-cli.c:200-203).

### The term line

The first line of a successful lookup (onym-cli.c:113):

```
term: <headword>
```

### Section headings

Each section, in the order the engine emitted it, prints its exact title in square brackets on its
own line (onym-cli.c:122):

```
[<Section title>]
```

### Definition lines

Each definition prints with a two-space indent and a dash; the part of speech, when present, is
parenthesised before the gloss, and the `(<pos>) ` prefix is omitted entirely when the pos is
absent (onym-cli.c:33-37):

```
  - (<pos>) <gloss>
  - <gloss>
```

Each example sentence follows its definition on its own line, six-space indent, wrapped in literal
double quotes (onym-cli.c:40-41):

```
      "<example>"
```

### Word lines

Each item of a Words section (Synonyms, Derived forms, Similar to, Attributes, Causes, Entails,
Domains) prints with a two-space indent (onym-cli.c:54):

```
  - <term>
```

### Etymology lines

The Etymology section (`engine.md` section 6.10), present only when the optional overlay carries the
headword, prints each prose paragraph on its own line with a two-space indent and a dash, in source
order, exactly like a word line:

```
  - <paragraph>
```

The paragraphs are single-line (the overlay whitespace-collapses them) and UTF-8.

### Translation lines

The Translations section (`engine.md` section 6.11), present only when the optional overlay carries a
resolved sense, prints one block per sense. Each block opens with the sense line: a two-space indent
and a dash, then the part of speech parenthesised before the gloss exactly as a definition line, with
the `(<pos>) ` prefix omitted entirely when the pos is absent:

```
  - (<pos>) <gloss>
  - <gloss>
```

Each language follows on its own line, six-space indent, the language's display name, then `: ` and
that language's words joined with `", "`, in the overlay's order:

```
      <Language>: <word>, <word>
```

The languages are ordered by display name; the blocks print in the order the senses were gathered,
one concept per block. The gloss is the definition text only, without its examples, the same string
the Definitions section shows for that sense. All words are UTF-8 and carry their own scripts and
accents.

### Antonym lines

Each antonym prints with a two-space indent, its term, then a space and `(direct)` or `(indirect)`
(onym-cli.c:66-67). Each implication follows on its own line: six spaces, `-> `, the term
(onym-cli.c:74):

```
  - <term> (direct)
  - <term> (indirect)
      -> <implication>
```

### Tree lines

A tree node prints `depth + 1` units of two spaces, then `- ` and the node's label, where the label
is the node's terms joined with `", "` (onym-cli.c:82-96, the label join per
`../gtk/libonym/onym-result.c:304`). A top-level node has depth 0, so it prints with one unit of
indent; each level adds one more unit. Children print immediately after their parent, depth first,
in order:

```
  - <root label>
    - <child label>
      - <grandchild label>
  - <next root label>
```

### The not-found output

A word not in WordNet prints (onym-cli.c:157-169):

```
No entry for "<word>".
```

with the word as given on the command line, in literal double quotes. When suggestions exist (cap
**5**, per `engine.md` section 8), one further line follows, the suggestions joined with `", "`:

```
Did you mean: <s1>, <s2>, ...
```

No suggestions, no second line. The exit status is 0 either way; a database error instead prints
`error: <message>` to stderr and exits 1 (onym-cli.c:148-153).

## `--complete PREFIX`

Headwords beginning with the prefix, one per line, alphabetical, lowercased display form, capped at
**20** (onym-cli.c:204-209, print_strv at onym-cli.c:19-24). Zero matches produce empty output.

## `--suggest WORD`

Spelling suggestions, one per line, ordered by edit distance then alphabetically, lowercased
display form, capped at **10** (onym-cli.c:211-216). Zero suggestions produce empty output.

## Usage errors

Any other argument shape prints `usage: onym-cli [--dump|--complete|--suggest] WORD` to stderr and
exits 2 (onym-cli.c:192-194, 218-221). Fixtures capture stdout only.
