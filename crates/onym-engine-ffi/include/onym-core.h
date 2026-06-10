/* onym-core.h
 *
 * SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

/* The C ABI of the Onym engine, hand-written and kept in step with the Rust source in
 * crates/onym-engine-ffi/src/lib.rs. The symbols use the onym_core_ prefix so they never collide
 * with libonym's own onym_ namespace.
 *
 * Ownership is simple: everything a call returns is freed by exactly one matching onym_core_*
 * free function, and fields documented as static are never freed. All strings are UTF-8; the
 * WordNet database is ASCII, so the bytes equal what the dictionary files carry. An engine is
 * immutable after open and safe to use from any number of threads. */

#pragma once

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The engine handle. Opaque; created by onym_core_open and released by onym_core_free. */
typedef struct OnymCoreEngine OnymCoreEngine;

/* One node of a lexical hierarchy: the words of one synset and the nodes one level deeper. */
typedef struct OnymCoreTreeNode OnymCoreTreeNode;
struct OnymCoreTreeNode
{
  char             **terms;    /* NULL-terminated, never empty */
  OnymCoreTreeNode **children; /* NULL-terminated, possibly empty */
};

/* One sense of a word. */
typedef struct
{
  const char *pos;      /* "noun", "verb", "adjective", "adverb", or NULL; static, do not free */
  char       *gloss;
  char      **examples; /* NULL-terminated, possibly empty */
} OnymCoreDefinition;

/* An opposite of the looked-up word, direct or reached through a similar sense. */
typedef struct
{
  char  *term;
  int    direct;        /* nonzero for a direct antonym */
  char **implications;  /* NULL-terminated, possibly empty */
} OnymCoreAntonym;

/* Which item array of a section is populated. */
typedef enum
{
  ONYM_CORE_SECTION_DEFINITIONS = 0,
  ONYM_CORE_SECTION_WORDS = 1,
  ONYM_CORE_SECTION_ANTONYMS = 2,
  ONYM_CORE_SECTION_TREE = 3,
} OnymCoreSectionKind;

/* A titled group of items of one kind. Exactly the array named by kind is non-NULL; n_items is
 * its length. A section never arrives empty. */
typedef struct
{
  int          kind;  /* an OnymCoreSectionKind */
  const char  *title; /* static, do not free */
  size_t       n_items;
  OnymCoreDefinition *definitions;
  char              **words; /* also NULL-terminated */
  OnymCoreAntonym    *antonyms;
  OnymCoreTreeNode  **tree;  /* also NULL-terminated */
} OnymCoreSection;

/* The whole entry for a looked-up word: the resolved headword and its sections in display order. */
typedef struct
{
  char            *term;
  size_t           n_sections;
  OnymCoreSection *sections;
} OnymCoreEntry;

/* Open an engine over the WordNet 3.0 database in data_dir, read-only and in place. Returns NULL
 * on failure; when error is non-NULL it receives a message to free with onym_core_string_free. */
OnymCoreEngine *onym_core_open (const char *data_dir, char **error);

/* Release an engine. NULL is allowed. */
void onym_core_free (OnymCoreEngine *engine);

/* Look word up. The query may be inflected or padded; the engine normalises it. Returns NULL when
 * the word is simply not in WordNet. Free the entry with onym_core_entry_free. */
OnymCoreEntry *onym_core_lookup (const OnymCoreEngine *engine, const char *word);

/* Release an entry and everything it owns. NULL is allowed. */
void onym_core_entry_free (OnymCoreEntry *entry);

/* Headwords beginning with prefix, lowercased display form, alphabetical, capped at max (0 means
 * no cap). Never NULL; a NULL-terminated array to free with onym_core_strv_free. */
char **onym_core_complete (const OnymCoreEngine *engine, const char *prefix, size_t max);

/* Spelling suggestions for a missed word, by edit distance then alphabetical, capped at max
 * (0 means no cap). Never NULL; free with onym_core_strv_free. */
char **onym_core_suggest (const OnymCoreEngine *engine, const char *word, size_t max);

/* How many headwords the lemma index holds. */
size_t onym_core_lemma_count (const OnymCoreEngine *engine);

/* The headword at index in sorted order, or NULL when out of range. A caller-chosen random index
 * makes this the "surprise me" action. Free with onym_core_string_free. */
char *onym_core_lemma_at (const OnymCoreEngine *engine, size_t index);

/* Free a NULL-terminated string array returned by this library. NULL is allowed. */
void onym_core_strv_free (char **strv);

/* Free a single string returned by this library. NULL is allowed. */
void onym_core_string_free (char *s);

#ifdef __cplusplus
}
#endif
