// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The WordNet dictionary reader: the engine's only source of database facts. It parses the
//! `index.*` and `data.*` files directly, replacing the extJWNL reader behind the Kotlin
//! reference's `WordNetSource` boundary, and follows the data contract of `spec/engine.md`
//! section 10: an explicit directory, every file UTF-8, read in place and read-only, no
//! environment variables, no global state. The neutral `Wn*` types mirror exactly what the
//! lookup needs: the senses of a word, the synset a pointer leads to, and morphology. Word and
//! pointer indices follow WordNet's convention, where 0 means the whole synset (a semantic
//! pointer) and a positive index identifies one word (a lexical pointer), the distinction the
//! antonym logic turns on.

use crate::OpenError;
use crate::etymology::Etymology;
use crate::morphology::{self, Morphology};
use crate::translations::Translations;
use crate::textforms::{ascii_lower, decode, index_variants, to_display_form};
use crate::verb_examples::VerbExampleIndex;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum WnPos {
    Noun,
    Verb,
    Adjective,
    Adverb,
}

impl WnPos {
    fn index(self) -> usize {
        match self {
            WnPos::Noun => 0,
            WnPos::Verb => 1,
            WnPos::Adjective => 2,
            WnPos::Adverb => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WnRelation {
    Antonym,
    Hypernym,
    InstanceHypernym,
    Hyponym,
    InstanceHyponym,
    Entailment,
    SimilarTo,
    MemberHolonym,
    SubstanceHolonym,
    PartHolonym,
    MemberMeronym,
    SubstanceMeronym,
    PartMeronym,
    Cause,
    Pertainym,
    Attribute,
    Derivation,
    VerbGroup,
    AlsoSee,
    Participle,
    Category,
    Usage,
    Region,
    CategoryMember,
    UsageMember,
    RegionMember,
}

/// One word of a synset: its lemma in WordNet form (underscores for spaces, case preserved, any
/// adjective syntax marker stripped) and its lexical id, kept for building sense keys.
#[derive(Clone, Debug)]
pub(crate) struct WnWord {
    pub(crate) lemma: String,
    lex_id: u8,
    // The adjective syntax marker as written in the data file ("(a)", "(p)", or "(ip)"), or
    // empty. Sense keys keep it on a satellite's head word (cntlist.rev carries keys like
    // above%5:00:00:preceding(a):00), while every other consumer sees the bare lemma.
    marker: String,
}

/// A pointer from one synset to another. The source and target word indices are 0 for a
/// whole-synset (semantic) pointer, or a positive word position for a lexical pointer.
#[derive(Clone, Debug)]
pub(crate) struct WnPointer {
    pub(crate) relation: WnRelation,
    pub(crate) source_word_index: usize,
    pub(crate) target_word_index: usize,
    pub(crate) target_pos: WnPos,
    pub(crate) target_offset: u32,
}

/// A synset: its part of speech, offset, words, gloss, adjective-satellite flag, and outgoing
/// pointers, plus the lexicographer file number that sense keys need.
#[derive(Clone, Debug)]
pub(crate) struct WnSynset {
    pub(crate) pos: WnPos,
    pub(crate) offset: u32,
    lex_filenum: u8,
    pub(crate) adjective_satellite: bool,
    pub(crate) words: Vec<WnWord>,
    pub(crate) gloss: String,
    pub(crate) pointers: Vec<WnPointer>,
}

const INDEX_FILES: [&str; 4] = ["index.noun", "index.verb", "index.adj", "index.adv"];
const DATA_FILES: [&str; 4] = ["data.noun", "data.verb", "data.adj", "data.adv"];

/// The WordNet reader the lookup depends on. It holds the index as a map from headword to synset
/// offsets, the data files as raw bytes parsed on demand, and the small side tables. Everything
/// is immutable after open, so an engine handle is safe for concurrent lookups.
pub(crate) struct DictSource {
    index: [HashMap<String, Vec<u32>>; 4],
    data: [Vec<u8>; 4],
    morphology: Morphology,
    // Tag counts from cntlist.rev, keyed by sense key; the forward cntlist file is not read.
    tag_counts: HashMap<String, u32>,
    verb_examples: VerbExampleIndex,
    // The optional etymology overlay; empty when etym.onym is absent.
    etymology: Etymology,
    // The optional translations overlay; empty when omw.onym is absent.
    translations: Translations,
}

impl DictSource {
    pub(crate) fn open(data_dir: &Path) -> Result<DictSource, OpenError> {
        let read_required = |name: &str| -> Result<Vec<u8>, OpenError> {
            let path = data_dir.join(name);
            fs::read(&path).map_err(|source| OpenError { file: path, source })
        };

        let mut index: [HashMap<String, Vec<u32>>; 4] = Default::default();
        for (i, name) in INDEX_FILES.iter().enumerate() {
            index[i] = parse_index(&read_required(name)?);
        }
        let mut data: [Vec<u8>; 4] = Default::default();
        for (i, name) in DATA_FILES.iter().enumerate() {
            data[i] = read_required(name)?;
        }

        let morphology = Morphology::load(data_dir).map_err(|source| OpenError {
            file: data_dir.to_path_buf(),
            source,
        })?;
        let verb_examples = VerbExampleIndex::load(data_dir).map_err(|source| OpenError {
            file: data_dir.to_path_buf(),
            source,
        })?;
        let etymology = Etymology::load(data_dir).map_err(|source| OpenError {
            file: data_dir.join("etym.onym"),
            source,
        })?;
        let translations = Translations::load(data_dir).map_err(|source| OpenError {
            file: data_dir.join("omw.onym"),
            source,
        })?;

        // The reverse sense-count list is optional: absent, every tag count is 0.
        let mut tag_counts = HashMap::new();
        let cntlist = data_dir.join("cntlist.rev");
        if cntlist.is_file() {
            let text = decode(&fs::read(&cntlist).map_err(|source| OpenError {
                file: cntlist,
                source,
            })?);
            for line in text.lines() {
                // Each line is a sense key, a sense number, and a tag count.
                let mut fields = line.split_whitespace();
                if let (Some(key), Some(_), Some(count)) =
                    (fields.next(), fields.next(), fields.next())
                    && let Ok(count) = count.parse()
                {
                    tag_counts.insert(key.to_string(), count);
                }
            }
        }

        Ok(DictSource {
            index,
            data,
            morphology,
            tag_counts,
            verb_examples,
            etymology,
            translations,
        })
    }

    /// The etymology paragraphs for the query-form `lemma`, from the optional overlay, in source
    /// order. Empty when the overlay is absent or the lemma has no entry, so a lookup over a plain
    /// WordNet directory never gains an Etymology section.
    pub(crate) fn etymology(&self, lemma: &str) -> Vec<String> {
        self.etymology.paragraphs(lemma)
    }

    /// The translation groups for the synset at `pos`/`offset`, from the optional overlay, each a
    /// language display name and its words, the groups ordered by display name. Empty when the
    /// overlay is absent or the synset has no entry, so a lookup over a plain WordNet directory
    /// never gains a Translations section.
    pub(crate) fn translations(&self, pos: WnPos, offset: u32) -> Vec<(String, Vec<String>)> {
        self.translations.groups(pos.index() as u8, offset)
    }

    /// The base forms WordNet's `morphstr` yields for `lemma` in `pos_code`, in its order, or
    /// empty. `pos_code` is WordNet's part-of-speech number (1 noun to 4 adverb), and 0 for the
    /// shifted-noun degenerate case the engine uses (Onym's `wni.c` work-around), which yields
    /// nothing.
    pub(crate) fn base_forms(&self, lemma: &str, pos_code: usize) -> Vec<String> {
        // morphstr's existence test is WordNet's is_defined, which matches a candidate through
        // its getindex variants, so a base form spelled differently still counts (horse_race
        // resolves through horse-race).
        self.morphology
            .morphstr(lemma, pos_code, &|code, candidate| {
                let Some(pos) = pos_from_code(code) else {
                    return false;
                };
                index_variants(candidate)
                    .iter()
                    .any(|variant| self.index_word_exists(variant, pos))
            })
    }

    /// Whether `lemma` is a headword in `pos`'s index, the engine's in-WordNet test for one
    /// variant. The match is exact: index keys are lowercase and underscored, and every caller
    /// passes that form.
    pub(crate) fn index_word_exists(&self, lemma: &str, pos: WnPos) -> bool {
        self.index[pos.index()].contains_key(lemma)
    }

    /// The senses of `lemma` in `pos`, in WordNet sense order; empty if `lemma` is not defined.
    pub(crate) fn senses_of(&self, lemma: &str, pos: WnPos) -> Vec<WnSynset> {
        let Some(offsets) = self.index[pos.index()].get(lemma) else {
            return Vec::new();
        };
        offsets
            .iter()
            .filter_map(|&offset| self.synset_at(pos, offset))
            .collect()
    }

    /// The synset at `pos` and `offset`, for following a pointer; absent if the offset does not
    /// start a well-formed data line.
    pub(crate) fn synset_at(&self, pos: WnPos, offset: u32) -> Option<WnSynset> {
        let data = &self.data[pos.index()];
        let start = offset as usize;
        if start >= data.len() {
            return None;
        }
        let end = data[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(data.len(), |p| start + p);
        parse_data_line(&data[start..end], offset)
    }

    /// WordNet's generic example sentences for the `word_index`-th word (1-based) of the synset
    /// at `pos`/`offset`, with that word substituted in. Only verbs carry these (from
    /// `sentidx.vrb` / `sents.vrb`); the list is empty for any other part of speech or when the
    /// sense has none.
    pub(crate) fn example_sentences(
        &self,
        pos: WnPos,
        offset: u32,
        word_index: usize,
    ) -> Vec<String> {
        if pos != WnPos::Verb || word_index < 1 {
            return Vec::new();
        }
        let Some(synset) = self.synset_at(WnPos::Verb, offset) else {
            return Vec::new();
        };
        let Some(word) = synset.words.get(word_index - 1) else {
            return Vec::new();
        };
        // The sense key Onym builds for sentidx.vrb: lemma%2:lexfilenum:lexid:: (verb pos is 2).
        let key = format!(
            "{}%2:{:02}:{:02}::",
            word.lemma, synset.lex_filenum, word.lex_id
        );
        self.verb_examples
            .sentences(&key, &to_display_form(&word.lemma))
    }

    /// The tagged-use count of the `word_index`-th word (0-based) of `synset`, from cntlist.rev.
    /// The Kotlin reference carried this on every synset word because extJWNL precomputed it; here
    /// it is resolved on demand, in the one place the lookup reads it, so tree growth never pays
    /// for sense keys it does not need. The value is identical either way.
    pub(crate) fn use_count(&self, synset: &WnSynset, word_index: usize) -> u32 {
        self.sense_key(synset, word_index)
            .and_then(|key| self.tag_counts.get(&key).copied())
            .unwrap_or(0)
    }

    /// The Princeton sense key of one synset word: `lemma%ss:lexfile:lexid:head:headid`, with the
    /// lemma lowercased and bare, the head fields filled only for a satellite adjective (whose
    /// head is the target of its similar-to pointer), and the numeric fields two-digit. The head
    /// word keeps its adjective syntax marker, because cntlist.rev keys do
    /// (above%5:00:00:preceding(a):00) while no key's lemma part ever carries one.
    fn sense_key(&self, synset: &WnSynset, word_index: usize) -> Option<String> {
        let word = synset.words.get(word_index)?;
        let lemma = ascii_lower(&word.lemma);
        let ss_type = match synset.pos {
            WnPos::Noun => 1,
            WnPos::Verb => 2,
            WnPos::Adjective if synset.adjective_satellite => 5,
            WnPos::Adjective => 3,
            WnPos::Adverb => 4,
        };
        if synset.adjective_satellite {
            let head_pointer = synset
                .pointers
                .iter()
                .find(|p| p.relation == WnRelation::SimilarTo)?;
            let head = self.synset_at(head_pointer.target_pos, head_pointer.target_offset)?;
            let head_word = head.words.first()?;
            Some(format!(
                "{lemma}%{ss_type}:{:02}:{:02}:{}{}:{:02}",
                synset.lex_filenum,
                word.lex_id,
                ascii_lower(&head_word.lemma),
                head_word.marker,
                head_word.lex_id
            ))
        } else {
            Some(format!(
                "{lemma}%{ss_type}:{:02}:{:02}::",
                synset.lex_filenum, word.lex_id
            ))
        }
    }
}

/// WordNet's part-of-speech numbering, as the morphology uses it; none for the degenerate code 0.
fn pos_from_code(code: usize) -> Option<WnPos> {
    match code {
        morphology::NOUN => Some(WnPos::Noun),
        morphology::VERB => Some(WnPos::Verb),
        morphology::ADJ => Some(WnPos::Adjective),
        morphology::ADV => Some(WnPos::Adverb),
        _ => None,
    }
}

/// Parse one `index.*` file into headword-to-offsets entries. Each line is the lemma, the part of
/// speech, the synset count, a pointer-symbol list, two more counts, and the synset offsets in
/// sense order; lines starting with a space are the licence header.
fn parse_index(bytes: &[u8]) -> HashMap<String, Vec<u32>> {
    let text = decode(bytes);
    let mut map = HashMap::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with(' ') {
            continue;
        }
        let tokens: Vec<&str> = line.split(' ').filter(|t| !t.is_empty()).collect();
        if tokens.len() < 3 {
            continue;
        }
        let Ok(synset_count) = tokens[2].parse::<usize>() else {
            continue;
        };
        if synset_count == 0 || tokens.len() < synset_count {
            continue;
        }
        let offsets: Vec<u32> = tokens[tokens.len() - synset_count..]
            .iter()
            .filter_map(|t| t.parse().ok())
            .collect();
        map.insert(tokens[0].to_string(), offsets);
    }
    map
}

/// Parse one `data.*` line: offset, lexicographer file number, synset type, a two-digit
/// hexadecimal word count, the words with their one-digit hexadecimal lexical ids, a three-digit
/// pointer count, the pointers, then (for verbs) frame data the engine does not read, and the
/// gloss after `" | "`.
fn parse_data_line(line: &[u8], offset: u32) -> Option<WnSynset> {
    let (head, gloss) = match line.windows(3).position(|w| w == b" | ") {
        Some(p) => {
            // A few data lines carry a second space after the pipe; extJWNL trimmed the gloss's
            // leading whitespace and the reference behaviour follows it. Trailing whitespace
            // stays, for parse_definition to trim the way the C library does.
            let mut gloss = &line[p + 3..];
            while let Some((&first, rest)) = gloss.split_first() {
                if first > b' ' {
                    break;
                }
                gloss = rest;
            }
            (&line[..p], decode(gloss))
        }
        None => (line, String::new()),
    };
    let head = decode(head);
    let mut tokens = head.split(' ').filter(|t| !t.is_empty());

    let _offset_field = tokens.next()?;
    let lex_filenum: u8 = tokens.next()?.parse().ok()?;
    let ss_type = tokens.next()?;
    let pos = match ss_type {
        "n" => WnPos::Noun,
        "v" => WnPos::Verb,
        "a" | "s" => WnPos::Adjective,
        "r" => WnPos::Adverb,
        _ => return None,
    };

    let word_count = usize::from_str_radix(tokens.next()?, 16).ok()?;
    let mut words = Vec::with_capacity(word_count);
    for _ in 0..word_count {
        let raw = tokens.next()?;
        // An adjective may carry a syntax marker such as "(p)"; the lemma is the part before it,
        // as WordNet's own parser keeps it, and the marker is kept aside for sense keys.
        let (lemma, marker) = match raw.find('(') {
            Some(p) => (&raw[..p], &raw[p..]),
            None => (raw, ""),
        };
        let lex_id = u8::from_str_radix(tokens.next()?, 16).ok()?;
        words.push(WnWord {
            lemma: lemma.to_string(),
            lex_id,
            marker: marker.to_string(),
        });
    }

    let pointer_count: usize = tokens.next()?.parse().ok()?;
    let mut pointers = Vec::with_capacity(pointer_count);
    for _ in 0..pointer_count {
        let symbol = tokens.next()?;
        let target_offset: u32 = tokens.next()?.parse().ok()?;
        let target_pos = match tokens.next()? {
            "n" => WnPos::Noun,
            "v" => WnPos::Verb,
            "a" | "s" => WnPos::Adjective,
            "r" => WnPos::Adverb,
            _ => return None,
        };
        // The source/target field is four hexadecimal digits: the source word number then the
        // target word number, 00 meaning the whole synset.
        let source_target = tokens.next()?;
        let source_word_index = usize::from_str_radix(source_target.get(..2)?, 16).ok()?;
        let target_word_index = usize::from_str_radix(source_target.get(2..)?, 16).ok()?;
        if let Some(relation) = relation_from_symbol(symbol) {
            pointers.push(WnPointer {
                relation,
                source_word_index,
                target_word_index,
                target_pos,
                target_offset,
            });
        }
    }

    Some(WnSynset {
        pos,
        offset,
        lex_filenum,
        adjective_satellite: ss_type == "s",
        words,
        gloss,
        pointers,
    })
}

/// The WordNet 3.0 pointer symbols. An unknown symbol contributes no pointer, mirroring the
/// Kotlin adapter's handling of relations the engine does not use.
fn relation_from_symbol(symbol: &str) -> Option<WnRelation> {
    Some(match symbol {
        "!" => WnRelation::Antonym,
        "@" => WnRelation::Hypernym,
        "@i" => WnRelation::InstanceHypernym,
        "~" => WnRelation::Hyponym,
        "~i" => WnRelation::InstanceHyponym,
        "*" => WnRelation::Entailment,
        "&" => WnRelation::SimilarTo,
        "#m" => WnRelation::MemberHolonym,
        "#s" => WnRelation::SubstanceHolonym,
        "#p" => WnRelation::PartHolonym,
        "%m" => WnRelation::MemberMeronym,
        "%s" => WnRelation::SubstanceMeronym,
        "%p" => WnRelation::PartMeronym,
        ">" => WnRelation::Cause,
        "\\" => WnRelation::Pertainym,
        "=" => WnRelation::Attribute,
        "+" => WnRelation::Derivation,
        "$" => WnRelation::VerbGroup,
        "^" => WnRelation::AlsoSee,
        "<" => WnRelation::Participle,
        ";c" => WnRelation::Category,
        ";u" => WnRelation::Usage,
        ";r" => WnRelation::Region,
        "-c" => WnRelation::CategoryMember,
        "-u" => WnRelation::UsageMember,
        "-r" => WnRelation::RegionMember,
        _ => return None,
    })
}
