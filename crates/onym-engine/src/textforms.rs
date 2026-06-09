// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pure string helpers shared by the engine and the lemma index: the query and display form
//! conversions, WordNet's locale-independent case folding, the getindex search variants, and the
//! bounded edit distance used for spelling suggestions. They depend on nothing, so the unit tests
//! exercise them without any WordNet data. Transcribed from the Kotlin reference (`TextForms.kt`
//! and the helpers in `WordNetLookup.kt`), per `spec/engine.md` section 3.

/// The query form: trimmed, with spaces turned to underscores. `"  ice cream "` becomes
/// `"ice_cream"`.
pub fn to_query_form(input: &str) -> String {
    input.trim().replace(' ', "_")
}

/// The display form: underscores turned to spaces. `"ice_cream"` becomes `"ice cream"`.
pub fn to_display_form(raw: &str) -> String {
    raw.replace('_', " ")
}

/// The Levenshtein edit distance between `a` and `b`, with unit insert, delete, and substitute
/// costs. A two-row dynamic program, matching Onym's `onym_edit_distance`. Distances count
/// characters, not bytes, so a Latin-1 gloss character is one edit.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }

    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur: Vec<usize> = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// ASCII-only lower-casing, matching WordNet's `strtolower` (locale-independent). No Unicode case
/// mapping is ever applied.
pub(crate) fn ascii_lower(value: &str) -> String {
    value.chars().map(|c| c.to_ascii_lowercase()).collect()
}

/// The lowercased display form: underscores to spaces and ASCII upper case to lower case. Used
/// for all case-insensitive comparisons and for the lemma index.
pub(crate) fn display_lower(value: &str) -> String {
    ascii_lower(&to_display_form(value))
}

/// WordNet's getindex search forms for `form`: the form itself, with underscores turned to
/// hyphens, with hyphens turned to underscores, with underscores and hyphens removed, and with
/// periods removed, deduplicated, in that order. This is how a hyphenated query also finds its
/// joined spelling (`cut-in` finds `cutin`), how variant spellings of a headword are recognised
/// as the same word, and how morphology's existence test accepts a base form spelled differently
/// (`horse_race` resolves through `horse-race`).
pub(crate) fn index_variants(form: &str) -> Vec<String> {
    let candidates = [
        form.to_string(),
        form.replace('_', "-"),
        form.replace('-', "_"),
        form.chars().filter(|&c| c != '_' && c != '-').collect(),
        form.chars().filter(|&c| c != '.').collect(),
    ];
    let mut variants: Vec<String> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if !variants.contains(&candidate) {
            variants.push(candidate);
        }
    }
    variants
}

/// Decode ISO-8859-1 bytes, the dictionary encoding, into a string. Every byte is the code point
/// of the same value, so the conversion is total and never alters a gloss byte.
pub(crate) fn latin1_to_string(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_form_trims_and_underscores_spaces() {
        assert_eq!(to_query_form("  ice cream "), "ice_cream");
        assert_eq!(to_query_form("run"), "run");
        assert_eq!(to_query_form("hot dog"), "hot_dog");
    }

    #[test]
    fn display_form_turns_underscores_to_spaces() {
        assert_eq!(to_display_form("ice_cream"), "ice cream");
        assert_eq!(to_display_form("hot_dog"), "hot dog");
        assert_eq!(to_display_form("run"), "run");
    }

    #[test]
    fn edit_distance_matches_known_values() {
        assert_eq!(edit_distance("abc", "abc"), 0);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("abc", ""), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("beutiful", "beautiful"), 1);
        assert_eq!(edit_distance("wrod", "word"), 2);
    }

    #[test]
    fn ascii_lower_leaves_non_ascii_alone() {
        assert_eq!(ascii_lower("Ice CREAM"), "ice cream");
        // U+00C9 is outside A to Z, so WordNet's strtolower leaves it untouched.
        assert_eq!(ascii_lower("\u{c9}clair"), "\u{c9}clair");
    }

    #[test]
    fn index_variants_dedupe_in_order() {
        assert_eq!(index_variants("cut-in"), vec!["cut-in", "cut_in", "cutin"]);
        assert_eq!(
            index_variants("ice_cream"),
            vec!["ice_cream", "ice-cream", "icecream"]
        );
        assert_eq!(index_variants("run"), vec!["run"]);
    }
}
