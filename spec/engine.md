<!--
SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Onym engine specification

This document is the single source of truth for the Onym lookup engine. The Rust core in this
repository implements it; the conformance fixtures answer to it. It distils two parity-proven
implementations: the C library Onym carried before it adopted this core (libonym over the vendored
Artha `wni.c` and libwordnet, now only in Onym's git history) and the Kotlin core in
`../android`, which was proven byte-for-byte equal to `onym-cli`.

Citations use `file:line`. Kotlin files live at
`../android/core/src/main/kotlin/nz/ursa/onymdroid/core/`; `PLAN.md` is the umbrella `../../PLAN.md`;
`onym-engine.c` is in `../gtk/libonym/`; `onym-cli.c` is in `../gtk/tools/`. Citations into
`onym-lookup.c` refer to the pre-swap libonym, which survives in Onym's git history at the commit
before the engine swap. Line numbers are as of the trees at the time of writing.

This specification is primary: it governs the engine's behaviour, and the conformance fixtures are
generated from it, not from any external tool. Where it deliberately departs from the WordNet C
library that Onym descends from, the departure is confined to the two changes in
[Deliberate fixes](#7-deliberate-fixes). Everything else is bug-for-bug faithful to that library.
Every word a deliberate fix or validation note names must appear in the conformance corpus.

## 1. Scope

- The engine reads a WordNet-family database in WNDB format and nothing else. The edition is pinned,
  not frozen: it is whatever the `onym-data` submodule supplies, and it moves only when that pin is
  bumped behind the data-validation and coverage suite. The base is currently Open English WordNet
  2025, the Plus edition with proper nouns, replacing the Princeton 3.0 database the engine was first
  written against.
- The database files come from the `onym-data` submodule, which vendors one set of bytes that backs
  the GTK app, the Flatpak, and the Android port alike, so every implementation reads identical
  bytes. There is no system-WordNet fallback; the engine opens the prepared submodule data over an
  explicit path.
- The engine owns everything above the data files: file parsing, morphology, the lookup pipeline,
  completion, suggestion, and verb example sentences. The data files are an external, read-only
  input.
- Two optional inputs sit outside the base graph: the etymology overlay of [section 6.10](#610-etymology-the-optional-overlay)
  and the translations overlay of [section 6.11](#611-translations-the-optional-overlay). Each is a
  preprocessed file the engine reads in place, exactly as it reads the database, and together they are
  the only deliberate exception to "the base graph and nothing else". The exception is confined to
  additive material: an overlay never alters a WordNet-derived section, and when both are absent the
  engine's output is byte-for-byte what the base graph alone produces. They only add the Etymology and
  Translations sections.

## 2. The result model

The model mirrors `OnymResult.kt:14-80` and PLAN.md:213-248. Every string a consumer sees is in
display form (section 3). The model carries no WordNet types; consumers read it and never build it.

- **Result**: the resolved headword `term` plus `sections`, an ordered list (OnymResult.kt:14-17).
- **Section**: a `title` and items of exactly one kind. The four base kinds are `Definitions`,
  `Words`, `Antonyms`, and `Tree` (OnymResult.kt:20-46); the two optional overlays add the
  `Etymology` kind (section 6.10) and the `Translations` kind (section 6.11). The kind is fixed per
  title; the exact title strings and their order are in section 6.1.
- **Sense translations** (the `Translations` kind, section 6.11): one block per looked-up sense,
  carrying the sense's part of speech and gloss and its words in other languages grouped by language.
- **Definition**: an optional `pos` (one of `noun`, `verb`, `adjective`, `adverb`, or absent), a
  `gloss`, and a possibly empty list of `examples` (OnymResult.kt:54-58).
- **Word**: a single term (OnymResult.kt:49-51).
- **Antonym**: a `term`, a `direct` flag (true for a direct antonym, false for an indirect one
  reached through a similar sense), and a possibly empty list of implication words
  (OnymResult.kt:64-68).
- **Tree node**: a list of `terms` (the words of one synset, each independently meaningful) and a
  list of child nodes. A node's `label` is its terms joined with `", "` exactly; the label is used
  for display and for top-level deduplication (OnymResult.kt:75-80, PLAN.md:246, the join rule
  originates at `../gtk/libonym/onym-result.c:304`).

A lookup returns either a result or nothing. Nothing means the word is simply not in WordNet; a
missing database is a distinct open-time error, never a quiet miss (PLAN.md:270-271).

## 3. Query and display forms

- **Query form**: trim leading and trailing whitespace, then replace every space with an
  underscore. `"  ice cream "` becomes `"ice_cream"` (TextForms.kt:13, PLAN.md:259).
- **Display form**: replace every underscore with a space (TextForms.kt:16, PLAN.md:260). Every
  string in the result model is display form.
- **Query normalisation** before lookup (WordNetLookup.kt:66-73): take the query form, truncate at
  the first `(` if present, then ASCII-lower it. If the normalised string is empty or is exactly
  `"."`, `"-"`, or `"_"`, the lookup returns nothing.
- **ASCII lowering**: only `A` to `Z` are lowered, locale-independent, matching WordNet's
  `strtolower` (WordNetLookup.kt:859-862). No Unicode case mapping is ever applied. Index lookups
  are exact-key: index keys are lowercase and underscored, and every searched form already is.
  (The Kotlin reference's extJWNL adapter applied a further locale lowercase before each search;
  that is an adapter wart, not contract.)
- **Encoding**: the database is UTF-8. Non-ASCII is confined to glosses and a few accented lemmas
  and proper nouns; the structure (offsets, pointers, counts) is ASCII. ASCII-only lowering means
  an accented letter in a lemma is never case-folded, which is correct because the index keys carry
  those letters in the same case, so an exact-key match still finds them. The engine decodes every
  file as UTF-8 and every result string is UTF-8.
- **Unicode edges, pinned**: query trimming strips the Unicode white space property, and lengths
  and edit distances count Unicode scalar values (`textforms.rs`), so an accented gloss character is
  one unit. They are fixed here so every port agrees. The JVM reference differed harmlessly at five
  trim code points and on astral-plane distances, which no fixture can reach.
- **Lowercased display form** (`displayLower`): underscores to spaces, then ASCII lowering
  (WordNetLookup.kt:865). Used for all case-insensitive comparisons and for the lemma index.

### 3.1 getindex search variants

Every form searched in an index is expanded to WordNet's `getindex` variants, deduplicated in this
order (WordNetLookup.kt:848-856):

1. the form itself;
2. underscores turned to hyphens;
3. hyphens turned to underscores;
4. all underscores and hyphens removed (the joined spelling, `cut-in` finds `cutin`);
5. all periods removed.

This is how a hyphenated query finds its joined spelling, how variant spellings are recognised as
the same word, and how morphology's existence test accepts a base form spelled differently
(`horse_race` matches via `horse-race`).

## 4. Headword resolution

Senses are gathered per part of speech in the fixed order noun, verb, adjective, adverb
(WordNetLookup.kt:96, 808). For each part of speech the surface form is searched, then each base
form morphology yields (section 5.3). Each searched form is expanded to its getindex variants; each
resolving variant contributes its lemma (lowercased display form, used later for synonym
suppression) and its senses, deduplicated by synset offset across the variants of one searched form
(WordNetLookup.kt:132-156). Under this specification every resolving variant is always visited; see
[Fix 1](#fix-1-getindex-variant-truncation).

Definition items are then built, one per (lemma, part of speech) pair, in gather order
(WordNetLookup.kt:207-244). Each item carries:

- its **display lemma**, with the case resolved by the last-match rule below;
- its **definitions**, one per sense, in sense order;
- its **tag count**: the use count of the first sense's first synset word whose lowercased display
  form matches the lemma. First match, not last: a synset may list both `Moon` and `moon` and only
  the capitalised first one carries the count (WordNetLookup.kt:237-243);
- its **polysemy**: the number of gathered senses in the group (WordNetLookup.kt:244).

Items are sorted with a stable sort (WordNetLookup.kt:249-258, mirroring Artha's
`pos_list_compare`): when two items' display lemmas differ, the one equal to the normalised query
(case-sensitively; the normalised query is underscored and lowered, so a capitalised or multiword
display lemma never matches) wins; otherwise the higher tag count wins; otherwise the higher
polysemy. The **headword is the first item's display lemma** (WordNetLookup.kt:35). The pre-swap
C bridge fell back to the raw query when there was no overview (onym-lookup.c:308), but an empty
overview means no result, so the fallback was unreachable in practice (WordNetLookup.kt:33-34).

### 4.1 The last-match case rule

Two places resolve a lemma against a synset's word list, and both keep the **last** case-insensitive
match, not the first. This is WordNet's own `read_synset` behaviour and is kept as intended
behaviour, not fixed:

- **Word index resolution** (WordNetLookup.kt:178-189): the sense's word index (`whichWord`, 1-based)
  is the position of the last synset word whose lowercased display form matches the searched lemma.
  A synset carrying both `utopian` and `Utopian` resolves to the second, and the lexical pointers
  restricted to that word index then apply, which is how `utopian` reaches its `Utopia` derivation.
- **Display lemma case** (WordNetLookup.kt:270-283, Artha's `populate_synonyms`): walking the first
  sense's words, the lemma is re-pointed at each case-insensitive match, so the last wins:
  `wordsworth` becomes `Wordsworth`, but a synset listing `Moon` then `moon` settles on `moon`,
  which is why the lowercased query then sorts as an exact match. A spelling already claimed by an
  earlier item is skipped (`is_synm_a_lemma`), so a demonym adjective stays lowercase once its
  proper-noun twin has taken the capital. Claiming follows item build order (WordNetLookup.kt:219-223).

### 4.2 The morphology dispatch and its sticky flag

The dispatch mirrors `wni.c` exactly, including its Ubuntu work-around (WordNetLookup.kt:92-112).
For each part of speech, with `index` running 0 to 3:

1. Search the surface form.
2. While a sticky flag (initially set) holds, call morphology with the part-of-speech code shifted
   down by one (`index`, so 0 for nouns, which yields nothing by construction). If that yields base
   forms, search them.
3. Otherwise call morphology with the correct code (`index + 1`). If that yields base forms, clear
   the sticky flag for all remaining parts of speech and search them.

Once cleared, the flag stays cleared, so later parts of speech skip the shifted call and go
straight to the correct code.

## 5. Morphology

The engine implements WordNet's `morphstr` itself, ported to the letter from `lib/morph.c`
(`morphstr`, `morphword`, `wordbase`, `exc_lookup`, `hasprep`, `morphprep`), because reader
libraries over-generate relative to it (Morphology.kt:8-26). Part-of-speech codes follow WordNet's
numbering: 1 noun, 2 verb, 3 adjective, 4 adverb. Code 0 (the shifted-noun case from section 4.2)
has no exception file and no suffix rules, so it always yields nothing (Morphology.kt:22-25,
220-221).

### 5.1 Exception files

`noun.exc`, `verb.exc`, `adj.exc`, `adv.exc`, keyed by part-of-speech code (Morphology.kt:202).
UTF-8, whitespace-separated: the inflected headword, then its base forms. Lines starting with
a space are skipped. An absent file contributes nothing (Morphology.kt:228-248).

### 5.2 morphstr

`morphstr(origstr, posCode)` returns base forms in order, or an empty list (Morphology.kt:35-89).
The input is ASCII-lowered with spaces turned to underscores first.

1. **Exception list**: if the word's line exists and its first base form differs from the input,
   return every base form on the line (Morphology.kt:43-47).
2. **Whole-string morph** (every part of speech except verbs): `morphword` on the whole string; if
   it yields a different word, return just it (Morphology.kt:50-57).
3. **Verb plus preposition**: for a verb input of more than one word (counted by `cntwords` on
   `_`) containing a known preposition, return `morphprep`'s single result or nothing
   (Morphology.kt:58-62).
4. **Component-wise**: split on underscores and hyphens preserving the original separators
   (component count from `cntwords` on `-`), `morphword` each component (a failed component keeps
   its surface form), and recombine. The candidate counts only when it differs from the input and
   `isDefined(posCode, candidate)` holds (Morphology.kt:67-88).

`morphword(word, posCode)` (Morphology.kt:92-120):

1. An empty word yields nothing.
2. The exception list's first base form wins. Adverbs have only the exception list.
3. Nouns: a word ending `ful` is morphed on its stem with `ful` re-appended; a word ending `ss` or
   of length two or less yields nothing.
4. Otherwise scan the part of speech's slice of the suffix tables; the first candidate that differs
   from the stem and is defined wins.

The suffix tables, exactly (Morphology.kt:205-221, `morph.c`'s `sufx[]`/`addr[]`):

| pos | offset | count | strip -> add |
|---|---|---|---|
| noun | 0 | 8 | `s`->``, `ses`->`s`, `xes`->`x`, `zes`->`z`, `ches`->`ch`, `shes`->`sh`, `men`->`man`, `ies`->`y` |
| verb | 8 | 8 | `s`->``, `ies`->`y`, `es`->`e`, `es`->``, `ed`->`e`, `ed`->``, `ing`->`e`, `ing`->`` |
| adjective | 16 | 4 | `er`->``, `est`->``, `er`->`e`, `est`->`e` |
| adverb | - | 0 | exception list only |

`morphprep(phrase)` (Morphology.kt:138-170): take the first word as the verb and keep the rest
verbatim. For a phrase of three or more words, also prepare an alternate tail with the last word
noun-morphed. A first word containing a non-alphanumeric character yields nothing. Then: the verb's
exception base with the rest re-attached, if defined; the same with the alternate tail; each
suffix-rule base of the verb with the rest, then the alternate tail, if defined; finally the
unmorphed verb with the rest (if that differs from the phrase), then with the alternate tail.

`hasprep` (Morphology.kt:173-187): true when any word after the first is exactly one of `to`,
`at`, `of`, `on`, `off`, `in`, `out`, `up`, `down`, `from`, `with`, `into`, `for`, `about`,
`between` (Morphology.kt:224-225).

`cntwords(s, separator)` counts words split on runs of the separator, spaces, and underscores
(Morphology.kt:253-269).

### 5.3 The isDefined callback contract

Candidate existence is index reading, not morphology, so it sits outside the algorithm:
`isDefined(posCode, candidate)` must be true exactly when any getindex variant (section 3.1) of the
candidate is a headword in that part of speech's index (`ExtjwnlWordNetSource.kt:33-38`). This is
WordNet's `is_defined`/`in_wn`, and it is why a base form spelled differently still counts
(`horse_race` resolves through `horse-race`). Code 0 never matches.

## 6. The lookup pipeline

### 6.1 The fourteen sections, in exact emission order

The pipeline emits at most fourteen WordNet sections, in this order, with these exact titles
(onym-lookup.c:311-333, WordNetLookup.kt:38-54, PLAN.md:284-299, PLAN.md:501-503). A section that
gathers zero items is dropped entirely (`add_section_if_filled`, onym-lookup.c:106-114). When the
etymology overlay is present and carries the headword, one further section, `Etymology`, is emitted
immediately after `Definitions` (so before `Synonyms`); it is additive and gated, and is specified
on its own in [section 6.10](#610-etymology-the-optional-overlay), apart from the fourteen below. When
the translations overlay is present and carries any resolved sense, one further section,
`Translations`, is emitted immediately after `Synonyms` (so before `Antonyms`); it too is additive and
gated, specified on its own in [section 6.11](#611-translations-the-optional-overlay).

| # | Title | Kind | Source relations |
|---|---|---|---|
| 1 | `Definitions` | Definitions | overview glosses |
| 2 | `Synonyms` | Words | overview synset words |
| 3 | `Antonyms` | Antonyms | ANTONYM (section 6.4) |
| 4 | `Derived forms` | Words | DERIVATION |
| 5 | `Similar to` | Words | SIMILAR_TO, adjective senses only |
| 6 | `Attributes` | Words | ATTRIBUTE |
| 7 | `Causes` | Words | CAUSE |
| 8 | `Entails` | Words | ENTAILMENT |
| 9 | `Pertains to` | Tree | PERTAINYM (section 6.6) |
| 10 | `Is a kind of` | Tree | HYPERNYM + INSTANCE_HYPERNYM, one group |
| 11 | `Kinds` | Tree | HYPONYM + INSTANCE_HYPONYM, one group |
| 12 | `Part of` | Tree | MEMBER, SUBSTANCE, PART holonyms, three groups in that order |
| 13 | `Parts` | Tree | MEMBER, SUBSTANCE, PART meronyms, three groups in that order |
| 14 | `Domains` | Words | CATEGORY, USAGE, REGION and their MEMBER inverses (section 6.8) |

Relation groups are defined at WordNetLookup.kt:811-837. A pointer is followed for a sense only
when it applies to it: its source word index is 0 (a whole-synset, semantic pointer) or equals the
sense's word index from section 4.1 (WordNetLookup.kt:715-718).

### 6.2 Part-of-speech codes

WordNet codes map to display names as: 1 `noun`, 2 `verb`, 3 `adjective`, 4 `adverb`, 5 `adjective`
(a satellite adjective is shown as an adjective), anything else absent (onym-lookup.c:33-51,
PLAN.md:273-277).

### 6.3 Definitions

The Definitions section is the sorted items' definitions flattened in item order
(WordNetLookup.kt:285-288). Each definition's gloss and examples come from splitting the synset
gloss with Artha's `parse_definition`, reproduced exactly (WordNetLookup.kt:736-797):

- The gloss (trailing whitespace trimmed) is scanned wrapped in parentheses. Quoted runs are
  examples; quotes themselves are never emitted, so an attribution after a closing quote
  (`"..." - Wordsworth`) stays inside its example text.
- A `;` followed (after one character) by a `"` outside a quoted run, or by a `(`, or ending the
  gloss, becomes a part separator; spaces immediately after a separator are skipped.
- An opening quote not at the very start turns the preceding character into a separator; a comma
  directly before it is collapsed into the separator first (the `compound` case).
- A parenthesised note immediately after a separator is hoisted, character by character, to the
  front of the accumulated text and closed with `") "`.
- The accumulated text splits on the separators: the first part is the gloss, the remaining
  non-empty parts are the examples.

A verb sense whose gloss yields no examples falls back to WordNet's generic sentence frames
(section 9; WordNetLookup.kt:228-235, Artha's `find_example`).

### 6.4 Synonyms

Walk every gathered sense's synset words in order. A word is suppressed when its lowercased display
form is one of the searched lemmas (every resolved getindex variant and morphology base form:
`ash bin` suppresses `ash-bin` and `ashbin`). Survivors are deduplicated case-insensitively with the
first spelling kept, as Artha's `check_term_in_list` does, so `wye` lists `Y` once, not both `Y`
and `y` (WordNetLookup.kt:292-311).

### 6.5 Antonyms

Antonyms gather per sense and merge across senses by term: the first occurrence's direct flag is
kept and implication lists merge in order without duplicates (WordNetLookup.kt:315-331).

- **Nouns, verbs, adverbs** (WordNetLookup.kt:334-349): every ANTONYM pointer that applies to the
  sense and has a positive target word index is direct. The term is the target synset's word at
  that index; the implications are the target synset's other words in order.
- **Adjectives, cluster head or standalone** (not satellite; WordNetLookup.kt:362-378): every
  ANTONYM pointer whose source word index equals the sense's word index exactly. The term is the
  antonym synset's first word. The implications are the antonym synset's remaining words, then the
  words of every synset its SIMILAR_TO pointers reach, in order, deduplicated, with the term itself
  removed. Direct.
- **Satellite adjectives** (the synset's satellite flag; WordNetLookup.kt:355-360, 380-400): a
  satellite has no antonyms of its own. Follow its first SIMILAR_TO pointer to the cluster head; for
  each ANTONYM pointer of the head, the target is the opposing cluster's head. The reported term is
  resolved by walking back (WordNetLookup.kt:406-420): among the opposing head's ANTONYM pointers
  with source word index 1, follow to the synset that itself carries an ANTONYM pointer with target
  word index 1 back to the opposing head's offset; the term is that back-pointer's source word (word
  1 when the source index is 0). No resolution, no antonym. Implications are the opposing head's
  other words. Indirect (`direct` false). Validate against `beautiful`, `fast`, `good`
  (PLAN.md:315-317).

### 6.6 Trees

Tree sections grow per sense, per relation group, then deduplicate the **top-level nodes only** by
label across everything in the section, first occurrence kept; deeper nodes are never deduplicated
(WordNetLookup.kt:494-509, onym-lookup.c:230-264, PLAN.md:322-324).

Growth (`grow_tree`; WordNetLookup.kt:540-563, as amended by
[Fix 2](#fix-2-grow_tree-sibling-attachment)): for each pointer of the synset, in pointer order,
whose relation is in the group and which applies to the word index (the sense's index at the top
level, 0 below it, so only semantic pointers are followed beneath the top): resolve the target
synset; its node terms are the target's words in display form minus any word matching the searched
lemma case-insensitively. Non-empty terms make a new node; a target whose terms all filter away
contributes no node and no children. Recursion continues into a new node while `depth + 1` is below
the maximum depth of 20 (WordNetLookup.kt:809).

`Is a kind of` and `Kinds` grow to full depth always. `Part of` and `Parts` grow under the depth
gate of section 6.7. The three holonym (and meronym) subtype groups run in member, substance, part
order, their nodes concatenating before deduplication (WordNetLookup.kt:639-655, 662-684).

`Parts` additionally traces inherited meronyms when its sense is deep (WordNetLookup.kt:687-711):
for each applicable HYPERNYM pointer, the ancestor synset becomes a node (terms minus the searched
lemma; an ancestor with no terms is skipped) whose children are the ancestor's own meronyms grown to
full depth, group by group, plus the ancestor's further inherited meronyms while the trace depth
stays below 20. An ancestor contributing no children is dropped. Traced nodes append after the
sense's own meronym nodes and join the same top-level deduplication.

### 6.7 The holonym and meronym depth gate

Whether `Part of` and `Parts` grow deep or stay flat reproduces `is_defined`'s HHOLONYM and
HMERONYM bits, computed per noun lemma (WordNetLookup.kt:593-632):

- For each **space-free** getindex variant of the lemma's display form, for each of its noun
  senses, for each HYPERNYM pointer: if the hypernym synset carries a meronym pointer the meronym
  bit sets; if it carries a holonym pointer the holonym bit sets (WordNet's `HasHoloMero`).
- The bits OR across all variants, so a same-spelt homograph can raise a word's depth: the plant
  `pica-pica` grows its part-of tree deep only because the magpie `pica_pica`, a getindex variant,
  inherits a holonym. Kept as intended behaviour.
- `is_defined` is handed the space-separated lemma, and index keys never contain spaces, so a
  multiword variant that still contains a space can never match; a **multiword noun therefore never
  resolves and its trees stay flat**. Kept as intended behaviour; `pica-pica` versus a true
  multiword like `ice cream` demonstrates both sides.

A sense whose lemma's holonym bit is clear grows `Part of` one level deep (its own holonyms only);
set, it grows to depth 20. Likewise the meronym bit for `Parts`, which also enables the inherited
trace of section 6.6 (WordNetLookup.kt:639-684).

### 6.8 Domains

The Domains section is gated per part of speech (WordNetLookup.kt:453-472, mirroring the `;` and
`-` symbols an index line carries, `is_defined`'s CLASSIFICATION and CLASS bits): a part of speech
participates only when at least one of its senses has a domain pointer (CATEGORY, USAGE, REGION, or
their MEMBER inverses) that applies to the sense's own word. Once a part of speech is in, **every**
domain pointer of all of its senses is followed regardless of source word. So `chequing account`,
whose own word carries a region link, lists all of its synset's UK, Canadian, and US domains, while
`nadolol`, whose only domain link springs from its synonym `Corgard`, shows no domains at all. Kept
as intended behaviour. Gathered terms exclude the sense's own lemma and deduplicate
case-insensitively, first spelling kept, like every flat section (WordNetLookup.kt:424-450).

### 6.9 The pertainym depth rule

`Pertains to` shows each applicable PERTAINYM target as a node (terms minus the searched lemma; an
empty node is skipped), deduplicated by label across senses. Exactly one node per sense gets
children: the **first** pertainym grown gets one level of hypernyms (the HYPERNYM and
INSTANCE_HYPERNYM group, depth 1); every later pertainym of the same sense is left bare, because
`grow_tree` zeroes its depth the first time it descends. `hasidic` shows `Orthodox Judaism` under
`Hasidism` but nothing under `Hasidim`. Kept as intended behaviour (WordNetLookup.kt:565-591).

### 6.10 Etymology (the optional overlay)

The engine reads one optional file that is not part of WordNet: `etym.onym`, the etymology overlay,
from the data directory (section 10). It is a preprocessed artifact keyed by WordNet lemma,
built offline from Wiktionary's etymology prose by `tools/etym-build`; the engine reads it in place,
read-only, like every other file, and parses none of WordNet's own files differently because of it.

When the overlay is present, the pipeline looks the resolved headword (section 4) up in it, by the
headword's query form: the index-key form of ASCII-lowercased, underscored display text (section 3),
which is exactly what a lemma index key is. A hit emits a single `Etymology` section, of a new kind
whose items are prose paragraphs in source order; a word with several distinct etymologies (a
Wiktionary page's "Etymology 1", "Etymology 2") contributes one paragraph each. A miss, or an absent
overlay, emits nothing, so a lookup over a plain WordNet directory is byte-for-byte unchanged.

The keying is by **headword only**: the section reflects the word the entry resolved to, not the raw
query and not the other gathered lemmas, so a morphological lookup (`dogs`) shows its base word's
etymology (`dog`) and the result is deterministic regardless of index iteration order. The paragraphs
are display text. The engine does not parse, reflow, or navigate them; it passes them through exactly
as the overlay holds them. Like every model string they are UTF-8 (the source languages carry
accented spellings), so a consumer treats the section's strings, and all others, as UTF-8.

The overlay's format, the producer's join against the WordNet lemma set, and the prose cleaning are
specified by `tools/etym-build`, not here: the engine's contract is only that it reads `etym.onym` if
present, keys by headword query form, and emits the paragraphs verbatim. The overlay's own provenance
travels with it (its leading-space header lines), the way the WordNet files carry their licence
header.

### 6.11 Translations (the optional overlay)

The engine reads a second optional file that is not part of WordNet: `omw.onym`, the translations
overlay, from the data directory (section 10). It is a preprocessed artifact built offline by
`tools/omw-build`, which joins each OEWN synset through the Collaborative Interlingual Index to the
Open Multilingual Wordnet components and records, per synset, the words other languages use for that
concept. The engine reads it in place, read-only, like every other file.

Where the etymology overlay is keyed by headword lemma, this one is keyed by synset, by the part of
speech and the WNDB offset the engine already holds for every gathered sense (section 4). So the
lookup needs no extra index; it asks the overlay only for the senses it has already resolved.

When the overlay is present, the pipeline looks each gathered sense up by its synset's part of speech
and offset. A sense the overlay carries contributes one block to a single `Translations` section: the
sense's part of speech and gloss, in the same form the Definitions section shows them, which names
the meaning the block belongs to, and the translated words grouped by language. Blocks follow the
order the senses were gathered, deduplicated by synset so one concept appears once; within a block the
languages are ordered by their display name, and a language's words keep the overlay's order. A sense
the overlay does not carry contributes no block, and an overlay that carries none of the resolved
senses, or that is absent, emits no section, so a lookup over a plain WordNet directory is
byte-for-byte unchanged.

The section is additive and gated exactly like etymology. It never alters a WordNet-derived section,
the base graph reads identically whether it is present or not, and the conformance fixtures are
generated overlay-free. It is emitted immediately after `Synonyms`, so the English synonyms and the
other-language words for the same senses sit together, and before `Antonyms`.

The translated words are display text in their own scripts and accents, UTF-8 like every model
string. The engine does not navigate or reflow them; a consumer treats them as plain display strings,
not headwords to look up. The overlay's format, its language set, the synset-to-CILI-to-component
join, and the per-component licences are specified by `tools/omw-build` and recorded with the data,
not here. The engine's contract is only that it reads `omw.onym` if present, keys each sense by its
part of speech and offset, and emits the per-sense, language-grouped words verbatim. The overlay's
provenance and its language legend travel with it in its leading-space header, the way the WordNet
files and the etymology overlay carry theirs.

## 7. Deliberate fixes

This specification departs from Artha and the WordNet C library (libwordnet) in **exactly two**
behaviours. Both repair iteration-state bugs in that library which produce arbitrary, input-dependent
output. Everything else in this document, including every quirk marked "kept as intended behaviour",
is normative as written. Fixtures for the affected words are written from this specification, not
captured from any reference build, and the affected words below must all be in the conformance
corpus.

### Fix 1: getindex variant truncation

**Old behaviour.** The C library's `getindex` keeps static iteration state. While building a noun's
`Part of` and `Parts` trees, `populate` calls `is_defined`, which shares that state and cuts short
`populate`'s own walk over the remaining getindex variants. A noun whose senses give it a meronym or
holonym tree (directly, or through an immediate hypernym) therefore only ever sees its **first**
resolved variant. The visible consequence: `shore bird` keeps `shorebird` as a synonym, because the
`shorebird` variant is never reached to claim it as a searched lemma, while `ash bin`, having no
such tree, suppresses `ash-bin` and `ashbin` (WordNetLookup.kt:120-131, where the Kotlin port
reproduces the truncation at WordNetLookup.kt:148-156).

**New behaviour.** Every resolving getindex variant is always visited, for every word. Every
resolved variant's lemma joins the searched-lemma set (so variant suppression in Synonyms behaves
uniformly) and its senses contribute, still deduplicated by synset offset within the searched form.
The truncation test (`nounHasMeronymOrHolonym`, WordNetLookup.kt:164-176) ceases to exist.

**Affected examples.** `shore bird` (now suppresses `shorebird`, behaving like `ash bin`); `ash bin`
(unchanged, the uniform case); `hot dog` (now suppresses `hotdog`); and `pica-pica`, where the
truncation was hiding a whole homograph: the magpie sense (the `pica_pica` variant) now contributes
its definition, its `European magpie` synonym, and its is-a tree alongside the plant. All four must
be in the conformance corpus.

### Fix 2: grow_tree sibling attachment

**Old behaviour.** `grow_tree` does not reset its current-node variable between pointers. When a
pointer's target contributes no new node (its only term is the searched word itself, so every term
filters away), the target's own children are grown anyway and attach to the **previous sibling**.
`door`'s `casing, case` node gains a phantom `lock` child that belongs to a skipped synset, and
`sing`'s `choir, chorus` node gains the bare-`sing` synset's hyponyms (WordNetLookup.kt:532-563,
where the Kotlin port reproduces the misattachment by recursing into the stale current node at
WordNetLookup.kt:559-561).

**New behaviour.** A pointer target that contributes no node contributes no children either: when a
target's terms all filter away, the walk emits nothing for that target and does not recurse. Sibling
nodes only ever carry their own children.

**Affected examples.** `door` (the `casing, case` node loses the phantom `lock` child), `sing`
(the `choir, chorus` node loses the bare-`sing` synset's hyponyms), and `sang`, which resolves to
`sing` through morphology and mirrors it. All three must be in the conformance corpus.

## 8. Completion and suggestion

The lemma index depends only on the `index.*` files (LemmaIndex.kt:9-13, PLAN.md:331-360).

**Building the index** (LemmaIndex.kt:88-115): read `index.noun`, `index.verb`, `index.adj`,
`index.adv`. Skip empty lines and lines starting with a space (the licence header). Take each
line's first space-delimited field, convert it to lowercased display form, collect across all four
files, sort by plain byte order (`strcmp`), and drop adjacent duplicates.

**Completion** (LemmaIndex.kt:21-38, PLAN.md:344-348): the needle is the lowercased display form of
the typed prefix; an empty prefix or needle returns nothing. Binary-search the lower bound, then
walk forward collecting lemmas that start with the needle until the prefix stops matching or the
cap is hit. Results are therefore alphabetical, in lowercased display form. Caps: **8** in the app,
**20** in the CLI (PLAN.md:349, onym-cli.c:206); a cap of 0 means no cap.

**Suggestion** (LemmaIndex.kt:45-64, TextForms.kt:22-44, PLAN.md:351-359): the needle is the
lowercased display form of the missed word. Scan every lemma; skip when the length difference
exceeds **2**; keep lemmas whose Levenshtein distance (unit insert, delete, substitute; a two-row
dynamic program) is **1 or 2**, so exact matches are excluded. Lengths and distances count
Unicode scalar values (section 3). Sort by distance ascending then term
ascending; return up to the cap. Caps: **5** on the app's not-found page and in the CLI dump's
Did-you-mean line, **10** for the CLI `--suggest` mode (PLAN.md:359, onym-cli.c:160, 213); 0 means
no cap.

## 9. Verb example sentences

WordNet's generic verb frames live in two files beside the database (VerbExampleIndex.kt:8-14,
Artha's `find_example`/`get_example`):

- `sents.vrb`: each line is a frame number, a space, and a template containing one `%s` placeholder
  (VerbExampleIndex.kt:34-37).
- `sentidx.vrb`: each line is a sense key, a space, and frame numbers separated by spaces or commas
  (VerbExampleIndex.kt:38-44).

The sense key is built as `lemma%2:ll:nn::` with the lemma underscored, the lexicographer file
number and the lexical id each two-digit zero-padded, and the verb part-of-speech digit 2
(ExtjwnlWordNetSource.kt:71-72). The lemma is the synset word at the sense's word index (section
4.1). The matching templates are filled with the word in display form and returned **in reverse
file order**, because Artha prepends as it reads (VerbExampleIndex.kt:19-28). The sentences are used
only as the fallback of section 6.3, for a verb sense whose gloss has no examples. Absent files mean
no generic examples, never an error (VerbExampleIndex.kt:31-46).

## 10. The data contract

An engine opens over an **explicit data directory path** passed by the caller. No environment
variables are consulted (the C engine's `WNSEARCHDIR`/`WNHOME` resolution at
`../gtk/libonym/onym-engine.c:74-96` does not carry over). The directory is never written to:
files are read in place, read-only (the Android port's copy-to-writable-storage step exists only
because extJWNL demands it; this engine must not).

The file set, all UTF-8:

| Files | Required | Absent means |
|---|---|---|
| `index.noun`, `index.verb`, `index.adj`, `index.adv` | yes | open fails |
| `data.noun`, `data.verb`, `data.adj`, `data.adv` | yes | open fails |
| `noun.exc`, `verb.exc`, `adj.exc`, `adv.exc` | no | no morphology exceptions for that pos (Morphology.kt:233-236) |
| `cntlist.rev` | no | every tag count is 0 |
| `sentidx.vrb`, `sents.vrb` | no | no generic verb examples (VerbExampleIndex.kt:31-46) |
| `etym.onym` | no | no Etymology section for any word (section 6.10) |
| `omw.onym` | no | no Translations section for any word (section 6.11) |

`cntlist.rev` is the reverse sense-count list (sense key, sense number, tag count), the file the
WordNet C library's `GetTagcnt` reads; OEWN's WNDB build ships it under exactly that name. The
forward `cntlist` file is not read. A failed open (missing directory, missing required file) is an
error, reported distinctly from a word that is simply not in WordNet (PLAN.md:270-271).

## 11. Threading

An engine handle is **immutable after open** and safe for concurrent lookups, completions, and
suggestions from any number of threads. This is a deliberate improvement over libwordnet, which
keeps global state and is not reentrant (PLAN.md:109-111). Implementations must not use global
mutable state of any kind: no process-wide caches, no static iteration cursors (the root cause of
[Fix 1](#fix-1-getindex-variant-truncation)), no environment mutation. Any internal laziness or
caching must be confined to the handle, internally synchronised, and behaviourally invisible: the
same query returns the same result regardless of what ran before or concurrently.
