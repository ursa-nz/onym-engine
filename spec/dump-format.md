<!--
SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Dump format

The stable text rendering of a lookup, used for conformance testing. It reproduces the output of
`../onym/tools/onym-cli.c` exactly (also documented at `../onymdroid/PLAN.md:374-411`), and was the
format the Kotlin port was proven byte-for-byte against. The conformance dumper in this repository
must emit it byte for byte.

**This file versions the format.** Fixtures regenerate only when this file or `engine.md` changes. The *content*
of a dump (which sections appear, their items, their order) is governed by `engine.md`, including
its two deliberate fixes; this file fixes only the rendering. A fixture mismatch therefore means
either an engine bug or an intentional `engine.md` change, never formatting drift.

## General rules

- Every line ends with a single LF. There are no blank lines and no trailing whitespace beyond
  what the rules below produce.
- Strings pass through from the engine unchanged. The database is ISO-8859-1, so dump bytes outside
  ASCII are ISO-8859-1; fixtures compare bytes, not characters.
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
`../onym/libonym/onym-result.c:304`). A top-level node has depth 0, so it prints with one unit of
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
