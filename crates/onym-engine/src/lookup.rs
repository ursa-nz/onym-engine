// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The bridge from the dictionary reader to the public model. It walks the senses the reader
//! returns and gathers them into the ordered, titled sections the model carries, reproducing the
//! rules of Onym's `wni.c` engine and `onym-lookup.c` bridge as `spec/engine.md` sections 4 to 7
//! specify them, including the spec's two deliberate fixes. Transcribed from the Kotlin reference
//! (`WordNetLookup.kt`); the quirks reproduced here are normative, so transcription beats
//! improvement everywhere.
//!
//!   overview definitions  -> a Definitions section, grouped by part of speech and sense
//!   overview synonyms     -> a Synonyms section
//!   antonyms              -> an Antonyms section, direct and indirect (the adjective-cluster
//!                            case)
//!   derivations, similar, attributes, causes, entails -> flat Words sections
//!   pertainyms, hypernyms, hyponyms -> Tree sections, grown to full depth
//!   holonyms, meronyms    -> Tree sections, grown to full depth only when Onym's is_defined
//!                            depth bit is set (see build_holonyms / build_meronyms), otherwise
//!                            kept flat
//!   domains               -> a Domains words section
//!
//! Lexical knowledge lives here, not in any consumer: this file decides section order, titles,
//! the direct/indirect antonym distinction, and which sections are dropped for being empty.

use crate::data::{DictSource, WnPointer, WnPos, WnRelation, WnSynset};
use crate::model::{Antonym, Definition, Entry, Section, SectionItems, TreeNode};
use crate::textforms::{ascii_lower, display_lower, index_variants, to_display_form};
use std::collections::{HashMap, HashSet};

const POS_ORDER: [WnPos; 4] = [WnPos::Noun, WnPos::Verb, WnPos::Adjective, WnPos::Adverb];
const MAX_TREE_DEPTH: usize = 20;

const HYPERNYM_GROUP: &[WnRelation] = &[WnRelation::Hypernym, WnRelation::InstanceHypernym];
const HYPONYM_GROUP: &[WnRelation] = &[WnRelation::Hyponym, WnRelation::InstanceHyponym];
const HOLONYM_GROUPS: [&[WnRelation]; 3] = [
    &[WnRelation::MemberHolonym],
    &[WnRelation::SubstanceHolonym],
    &[WnRelation::PartHolonym],
];
const MERONYM_GROUPS: [&[WnRelation]; 3] = [
    &[WnRelation::MemberMeronym],
    &[WnRelation::SubstanceMeronym],
    &[WnRelation::PartMeronym],
];
const MERONYM_RELATIONS: &[WnRelation] = &[
    WnRelation::MemberMeronym,
    WnRelation::SubstanceMeronym,
    WnRelation::PartMeronym,
];
const HOLONYM_RELATIONS: &[WnRelation] = &[
    WnRelation::MemberHolonym,
    WnRelation::SubstanceHolonym,
    WnRelation::PartHolonym,
];
const DOMAIN_RELATIONS: &[WnRelation] = &[
    WnRelation::Category,
    WnRelation::Usage,
    WnRelation::Region,
    WnRelation::CategoryMember,
    WnRelation::UsageMember,
    WnRelation::RegionMember,
];

/// A processed sense: the synset, the lemma that found it, and that lemma's index in the synset.
struct Sense {
    lemma: String,
    pos: WnPos,
    synset: WnSynset,
    which_word: usize,
}

/// The gathered senses and the set of lemmas that found them, for synonym exclusion.
struct Gathered {
    senses: Vec<Sense>,
    lemmas: HashSet<String>,
}

/// One Definitions item under construction, carrying the keys the headword ordering needs.
struct DefinitionItem {
    display_lemma: String,
    definitions: Vec<Definition>,
    tag_count: u32,
    polysemy: usize,
}

/// Onym's is_defined depth bits for a noun lemma.
struct NounDepth {
    meronym: bool,
    holonym: bool,
}

/// One tree growth pass: the relation group it follows, the searched lemma it filters out of
/// every node, and how deep it may recurse.
struct TreeWalk<'a> {
    group: &'a [WnRelation],
    self_lemma_lower: &'a str,
    max_depth: usize,
}

pub(crate) struct Lookup<'a> {
    pub(crate) source: &'a DictSource,
}

impl Lookup<'_> {
    /// Look `query` up and build its entry, or nothing when the word is simply not in WordNet.
    pub(crate) fn lookup(&self, query: &str) -> Option<Entry> {
        let normalized = normalize(query)?;
        let gathered = self.gather_senses(&normalized);
        let senses = &gathered.senses;
        if senses.is_empty() {
            return None;
        }

        let items = self.build_definition_items(senses, &normalized);
        if items.is_empty() {
            return None;
        }
        let headword = items[0].display_lemma.clone();
        let noun_depth = self.compute_noun_depth(senses);

        let mut sections = Vec::new();
        let definitions: Vec<Definition> = items
            .into_iter()
            .flat_map(|item| item.definitions)
            .collect();
        if !definitions.is_empty() {
            sections.push(Section {
                title: "Definitions",
                items: SectionItems::Definitions(definitions),
            });
        }
        add_words(
            &mut sections,
            "Synonyms",
            self.build_synonyms(senses, &gathered.lemmas),
        );
        self.add_antonyms(&mut sections, senses);
        add_words(
            &mut sections,
            "Derived forms",
            self.build_flat(senses.iter(), &[WnRelation::Derivation], false, false),
        );
        add_words(
            &mut sections,
            "Similar to",
            self.build_flat(senses.iter(), &[WnRelation::SimilarTo], true, false),
        );
        add_words(
            &mut sections,
            "Attributes",
            self.build_flat(senses.iter(), &[WnRelation::Attribute], false, false),
        );
        add_words(
            &mut sections,
            "Causes",
            self.build_flat(senses.iter(), &[WnRelation::Cause], false, false),
        );
        add_words(
            &mut sections,
            "Entails",
            self.build_flat(senses.iter(), &[WnRelation::Entailment], false, false),
        );
        add_tree(&mut sections, "Pertains to", self.build_pertainyms(senses));
        add_tree(
            &mut sections,
            "Is a kind of",
            self.build_tree(senses, &[HYPERNYM_GROUP]),
        );
        add_tree(
            &mut sections,
            "Kinds",
            self.build_tree(senses, &[HYPONYM_GROUP]),
        );
        add_tree(
            &mut sections,
            "Part of",
            self.build_holonyms(senses, &noun_depth),
        );
        add_tree(
            &mut sections,
            "Parts",
            self.build_meronyms(senses, &noun_depth),
        );
        add_words(&mut sections, "Domains", self.build_domains(senses));

        Some(Entry {
            term: headword,
            sections,
        })
    }

    /// Gather every sense across the four parts of speech, reproducing Onym's wni_request_nyms:
    /// for each part of speech the surface form is searched, then morphology, and each searched
    /// form is expanded to its index variants the way WordNet's getindex does
    /// (hyphen/underscore/joined/period forms). Every variant that resolves contributes a lemma
    /// (used to suppress those forms as their own synonyms), and its senses unless their synset
    /// has already been seen for that searched form.
    ///
    /// The morphology dispatch mirrors wni.c exactly, including its Ubuntu work-around:
    /// morphology is tried first with the part of speech shifted down by one, and only if that
    /// yields nothing (across all parts of speech so far, because the flag is sticky) is it
    /// retried at the correct part of speech.
    fn gather_senses(&self, normalized: &str) -> Gathered {
        let mut senses = Vec::new();
        let mut lemmas = HashSet::new();
        let mut morphword_in_file = true;
        for (index, &pos) in POS_ORDER.iter().enumerate() {
            // The surface form, then each base form WordNet's morphstr yields.
            self.gather(normalized, pos, &mut senses, &mut lemmas);

            let shifted_bases = if morphword_in_file {
                self.source.base_forms(normalized, index)
            } else {
                Vec::new()
            };
            if !shifted_bases.is_empty() {
                for base in &shifted_bases {
                    self.gather(base, pos, &mut senses, &mut lemmas);
                }
            } else {
                let bases = self.source.base_forms(normalized, index + 1);
                if !bases.is_empty() {
                    morphword_in_file = false;
                    for base in &bases {
                        self.gather(base, pos, &mut senses, &mut lemmas);
                    }
                }
            }
        }
        Gathered { senses, lemmas }
    }

    /// Search `form` in `pos` through its WordNet getindex variants, adding each resolved
    /// variant's lemma to `lemmas` and its senses to `senses` (offset-deduplicated across
    /// variants, as Onym's populate does).
    ///
    /// Every resolving variant is always visited. This is fix 1 of the spec's deliberate fixes:
    /// the WordNet C library's populate, while building a noun's part-of / parts tree, calls
    /// is_defined, which shares getindex's static iteration state and cuts short populate's own
    /// walk over the remaining variants, so a noun carrying meronyms or holonyms only ever saw
    /// its first variant. That is why "shore bird" used to keep "shorebird" as a synonym while
    /// "ash bin" suppressed ash-bin and ashbin. Variant suppression now behaves uniformly for
    /// every word.
    fn gather(
        &self,
        form: &str,
        pos: WnPos,
        senses: &mut Vec<Sense>,
        lemmas: &mut HashSet<String>,
    ) {
        let mut seen_offsets = HashSet::new();
        for variant in index_variants(form) {
            let variant_senses = self.source.senses_of(&variant, pos);
            if variant_senses.is_empty() {
                continue;
            }
            lemmas.insert(display_lower(&variant));
            for synset in variant_senses {
                if seen_offsets.insert(synset.offset) {
                    let which_word = which_word(&synset, &variant);
                    senses.push(Sense {
                        lemma: variant.clone(),
                        pos,
                        synset,
                        which_word,
                    });
                }
            }
        }
    }

    // --- Definitions and the headword ---------------------------------------------------------

    /// Group senses into one item per (lemma, part of speech), then order the items as Onym does:
    /// an exact match to the query wins, then the higher first-sense tag count, then the higher
    /// polysemy. The headword is the top item's lemma.
    fn build_definition_items(&self, senses: &[Sense], normalized: &str) -> Vec<DefinitionItem> {
        // One item per (lemma, part of speech), as Onym does: morphology can yield several lemmas
        // (better -> good, well, better) and each is ordered independently.
        let mut grouped: Vec<((&str, WnPos), Vec<&Sense>)> = Vec::new();
        for sense in senses {
            let key = (sense.lemma.as_str(), sense.pos);
            match grouped.iter_mut().find(|(k, _)| *k == key) {
                Some((_, group)) => group.push(sense),
                None => grouped.push((key, vec![sense])),
            }
        }

        // Items are built in WordNet's part-of-speech order (noun, verb, adjective, adverb),
        // which is the order POS_ORDER gathered them, so case-claiming below follows Onym's
        // processing order.
        let mut claimed = HashSet::new();
        let mut items = Vec::new();
        for ((lemma, pos), group) in &grouped {
            let display_lemma = resolve_display_lemma(lemma, &group[0].synset, &claimed);
            claimed.insert(display_lemma.clone());
            let definitions: Vec<Definition> = group
                .iter()
                .map(|sense| {
                    let (gloss, examples) = parse_definition(&sense.synset.gloss);
                    // A verb whose gloss carries no example falls back to WordNet's generic
                    // sentence frames, exactly as Onym's find_example does.
                    let examples = if examples.is_empty() && *pos == WnPos::Verb {
                        self.source.example_sentences(
                            sense.pos,
                            sense.synset.offset,
                            sense.which_word,
                        )
                    } else {
                        examples
                    };
                    Definition {
                        pos: Some(pos_name(*pos)),
                        gloss,
                        examples,
                    }
                })
                .collect();
            // The first-sense tag count, keyed on the FIRST synset word matching the lemma:
            // Onym's GetTagcnt builds its sense key from WNSnsToStr, which takes the first match,
            // not the last one which_word resolves to (a synset may list both "Moon" and "moon",
            // with only the capitalised first one carrying the tag count).
            let first = group[0];
            let target = display_lower(&first.lemma);
            let tag_count = first
                .synset
                .words
                .iter()
                .position(|word| display_lower(&word.lemma) == target)
                .map_or(0, |i| self.source.use_count(&first.synset, i));
            items.push(DefinitionItem {
                display_lemma,
                definitions,
                tag_count,
                polysemy: group.len(),
            });
        }

        // Onym's pos_list_compare: when the lemmas differ, the one that matches the search string
        // exactly (case sensitively) wins; otherwise the higher first-sense tag count, then
        // polysemy. The sort is stable, so ties keep gather order.
        items.sort_by(|a, b| {
            use std::cmp::Ordering;
            if a.display_lemma != b.display_lemma {
                if normalized == a.display_lemma {
                    return Ordering::Less;
                }
                if normalized == b.display_lemma {
                    return Ordering::Greater;
                }
            }
            if a.tag_count != b.tag_count {
                return b.tag_count.cmp(&a.tag_count);
            }
            b.polysemy.cmp(&a.polysemy)
        });
        items
    }

    // --- Synonyms ------------------------------------------------------------------------------

    fn build_synonyms(&self, senses: &[Sense], lemmas: &HashSet<String>) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for sense in senses {
            for word in &sense.synset.words {
                // A synset word is its own lemma's synonym only when it is not itself a searched
                // form: Onym suppresses every getindex variant and morphology base form (ash bin,
                // ash-bin, ashbin) as a synonym of one another, so only genuinely distinct words
                // remain.
                let lower = display_lower(&word.lemma);
                if lemmas.contains(&lower) {
                    continue;
                }
                // Synonyms are deduplicated case-insensitively, first spelling kept, as Onym's
                // check_term_in_list does, so "wye" lists "Y" once, not both "Y" and "y".
                if seen.insert(lower) {
                    result.push(to_display_form(&word.lemma));
                }
            }
        }
        result
    }

    // --- Antonyms ------------------------------------------------------------------------------

    fn add_antonyms(&self, sections: &mut Vec<Section>, senses: &[Sense]) {
        // Antonyms merge across senses by term: the first occurrence's direct flag is kept and
        // implication lists merge in order without duplicates.
        let mut order: Vec<String> = Vec::new();
        let mut merged: HashMap<String, Antonym> = HashMap::new();
        for sense in senses {
            let gathered = if sense.pos == WnPos::Adjective {
                self.adjective_antonyms(sense)
            } else {
                self.plain_antonyms(sense)
            };
            for antonym in gathered {
                match merged.get_mut(&antonym.term) {
                    Some(existing) => {
                        merge_words(&mut existing.implications, &antonym.implications);
                    }
                    None => {
                        order.push(antonym.term.clone());
                        merged.insert(antonym.term.clone(), antonym);
                    }
                }
            }
        }
        if !order.is_empty() {
            let items = order
                .iter()
                .filter_map(|term| merged.remove(term))
                .collect();
            sections.push(Section {
                title: "Antonyms",
                items: SectionItems::Antonyms(items),
            });
        }
    }

    /// Noun, verb, and adverb antonyms: every one is direct; implications are the antonym's
    /// synset-mates.
    fn plain_antonyms(&self, sense: &Sense) -> Vec<Antonym> {
        let mut result = Vec::new();
        for pointer in &sense.synset.pointers {
            if pointer.relation != WnRelation::Antonym {
                continue;
            }
            if !source_applies(pointer, sense.which_word) || pointer.target_word_index == 0 {
                continue;
            }
            let Some(target) = self
                .source
                .synset_at(pointer.target_pos, pointer.target_offset)
            else {
                continue;
            };
            let Some(word) = target.words.get(pointer.target_word_index - 1) else {
                continue;
            };
            let term = to_display_form(&word.lemma);
            let implications = target
                .words
                .iter()
                .map(|w| to_display_form(&w.lemma))
                .filter(|t| *t != term)
                .collect();
            result.push(Antonym {
                term,
                direct: true,
                implications,
            });
        }
        result
    }

    /// Adjective antonyms. A cluster head (or standalone) reports its direct antonyms; a
    /// satellite has none of its own, so it follows similar-to to its head and reports the head's
    /// antonym indirectly.
    fn adjective_antonyms(&self, sense: &Sense) -> Vec<Antonym> {
        if sense.synset.adjective_satellite {
            self.indirect_adjective_antonyms(sense)
        } else {
            self.direct_adjective_antonyms(sense)
        }
    }

    fn direct_adjective_antonyms(&self, sense: &Sense) -> Vec<Antonym> {
        let mut result = Vec::new();
        for pointer in &sense.synset.pointers {
            if pointer.relation != WnRelation::Antonym
                || pointer.source_word_index != sense.which_word
            {
                continue;
            }
            let Some(antonym_synset) = self
                .source
                .synset_at(pointer.target_pos, pointer.target_offset)
            else {
                continue;
            };
            let Some(first) = antonym_synset.words.first() else {
                continue;
            };
            let term = to_display_form(&first.lemma);
            // The implications keep first-insertion order without duplicates: the antonym's
            // synset-mates, then the words of every cluster its similar-to pointers reach.
            let mut implications: Vec<String> = Vec::new();
            for word in &antonym_synset.words[1..] {
                push_unique(&mut implications, to_display_form(&word.lemma));
            }
            for similar in &antonym_synset.pointers {
                if similar.relation != WnRelation::SimilarTo {
                    continue;
                }
                let Some(cluster) = self
                    .source
                    .synset_at(similar.target_pos, similar.target_offset)
                else {
                    continue;
                };
                for word in &cluster.words {
                    push_unique(&mut implications, to_display_form(&word.lemma));
                }
            }
            implications.retain(|t| *t != term);
            result.push(Antonym {
                term,
                direct: true,
                implications,
            });
        }
        result
    }

    fn indirect_adjective_antonyms(&self, sense: &Sense) -> Vec<Antonym> {
        let Some(head) = sense
            .synset
            .pointers
            .iter()
            .find(|p| p.relation == WnRelation::SimilarTo)
            .and_then(|p| self.source.synset_at(p.target_pos, p.target_offset))
        else {
            return Vec::new();
        };

        let mut result = Vec::new();
        for pointer in &head.pointers {
            if pointer.relation != WnRelation::Antonym {
                continue;
            }
            let Some(antonym_head) = self
                .source
                .synset_at(pointer.target_pos, pointer.target_offset)
            else {
                continue;
            };
            let Some(term) = self.indirect_via(&antonym_head) else {
                continue;
            };
            let implications = antonym_head
                .words
                .iter()
                .map(|w| to_display_form(&w.lemma))
                .filter(|t| *t != term)
                .collect();
            result.push(Antonym {
                term,
                direct: false,
                implications,
            });
        }
        result
    }

    /// Resolve the specific opposing word for an indirect antonym: from the antonym's head,
    /// follow its antonym pointers back to the synset that points at ours, and take the word at
    /// that pointer's end.
    fn indirect_via(&self, antonym_head: &WnSynset) -> Option<String> {
        for pointer in &antonym_head.pointers {
            if pointer.relation != WnRelation::Antonym || pointer.source_word_index != 1 {
                continue;
            }
            let Some(back) = self
                .source
                .synset_at(pointer.target_pos, pointer.target_offset)
            else {
                continue;
            };
            for inner in &back.pointers {
                if inner.relation == WnRelation::Antonym
                    && inner.target_word_index == 1
                    && inner.target_offset == antonym_head.offset
                {
                    let word_index = if inner.source_word_index > 0 {
                        inner.source_word_index - 1
                    } else {
                        0
                    };
                    return back
                        .words
                        .get(word_index)
                        .map(|w| to_display_form(&w.lemma));
                }
            }
        }
        None
    }

    // --- Flat relations and domains ------------------------------------------------------------

    fn build_flat<'s>(
        &self,
        senses: impl Iterator<Item = &'s Sense>,
        relations: &[WnRelation],
        adjectives_only: bool,
        ignore_source_word: bool,
    ) -> Vec<String> {
        // Terms are deduplicated case-insensitively with the first spelling kept, as Onym's
        // check_term_in_list does, so a derived form listed as "Catholicity" is not repeated as
        // "catholicity".
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for sense in senses {
            if adjectives_only && sense.pos != WnPos::Adjective {
                continue;
            }
            let self_lower = display_lower(&sense.lemma);
            for pointer in &sense.synset.pointers {
                if !relations.contains(&pointer.relation) {
                    continue;
                }
                if !ignore_source_word && !source_applies(pointer, sense.which_word) {
                    continue;
                }
                let Some(target) = self
                    .source
                    .synset_at(pointer.target_pos, pointer.target_offset)
                else {
                    continue;
                };
                for word in &target.words {
                    let lower = display_lower(&word.lemma);
                    if lower == self_lower {
                        continue;
                    }
                    if seen.insert(lower) {
                        result.push(to_display_form(&word.lemma));
                    }
                }
            }
        }
        result
    }

    /// Domains: the in-domain pointers (category / usage / region) and the domain-member
    /// pointers.
    ///
    /// These behave unusually, mirroring WordNet's index files and Onym's populate. The section
    /// appears for a part of speech only if the searched word is itself the source of a domain
    /// pointer there (the `;` / `-` symbols its index line would carry, is_defined's
    /// CLASSIFICATION / CLASS bits). But once it appears, populate follows every domain pointer
    /// of those senses regardless of which word it springs from. So "chequing account" (whose own
    /// word has a region link) lists all of its synset's UK, Canadian and US domains, whereas
    /// "nadolol" (whose only domain link springs from its synonym "Corgard") shows no domains at
    /// all.
    fn build_domains(&self, senses: &[Sense]) -> Vec<String> {
        let gated_positions: HashSet<WnPos> = senses
            .iter()
            .filter(|sense| {
                sense.synset.pointers.iter().any(|pointer| {
                    DOMAIN_RELATIONS.contains(&pointer.relation)
                        && source_applies(pointer, sense.which_word)
                })
            })
            .map(|sense| sense.pos)
            .collect();
        let gated = senses
            .iter()
            .filter(|sense| gated_positions.contains(&sense.pos));
        self.build_flat(gated, DOMAIN_RELATIONS, false, true)
    }

    // --- Trees ---------------------------------------------------------------------------------

    /// Build a tree section: per sense, grow each relation group to full depth, then deduplicate
    /// the top-level nodes by label across senses (first occurrence wins), as the bridge does.
    /// Deeper nodes are never deduplicated.
    fn build_tree(&self, senses: &[Sense], groups: &[&[WnRelation]]) -> Vec<TreeNode> {
        let mut seen = HashSet::new();
        let mut nodes = Vec::new();
        for sense in senses {
            for group in groups {
                let walk = TreeWalk {
                    group,
                    self_lemma_lower: &display_lower(&sense.lemma),
                    max_depth: MAX_TREE_DEPTH,
                };
                for node in self.grow_nodes(&sense.synset, sense.which_word, &walk, 0) {
                    if seen.insert(node.label()) {
                        nodes.push(node);
                    }
                }
            }
        }
        nodes
    }

    fn grow_nodes(
        &self,
        synset: &WnSynset,
        which_word: usize,
        walk: &TreeWalk,
        depth: usize,
    ) -> Vec<TreeNode> {
        let mut nodes = Vec::new();
        self.grow_into(&mut nodes, synset, which_word, walk, depth);
        nodes
    }

    /// Grow the walk's relation nodes under `parent`, following WordNet/Onym's grow_tree with one
    /// deliberate departure, fix 2 of the spec: grow_tree never reset its current node between
    /// pointers, so when a pointer's target carried only the searched word itself (no new term,
    /// no node created), the target's children were appended to the previous sibling instead.
    /// That is why door's "casing, case" used to gain a phantom "lock" child and sing's "choir,
    /// chorus" the bare-"sing" synset's hyponyms. A target that contributes no node now
    /// contributes no children either; siblings only ever carry their own.
    fn grow_into(
        &self,
        parent: &mut Vec<TreeNode>,
        synset: &WnSynset,
        which_word: usize,
        walk: &TreeWalk,
        depth: usize,
    ) {
        for pointer in &synset.pointers {
            if !walk.group.contains(&pointer.relation) {
                continue;
            }
            if !source_applies(pointer, which_word) {
                continue;
            }
            let Some(target) = self
                .source
                .synset_at(pointer.target_pos, pointer.target_offset)
            else {
                continue;
            };
            let terms: Vec<String> = target
                .words
                .iter()
                .map(|w| to_display_form(&w.lemma))
                .filter(|t| display_lower(t) != walk.self_lemma_lower)
                .collect();
            if terms.is_empty() {
                continue;
            }
            let mut node = TreeNode {
                terms,
                children: Vec::new(),
            };
            if depth + 1 < walk.max_depth {
                // Only semantic pointers are followed beneath the top level, so the word index
                // drops to 0 here.
                self.grow_into(&mut node.children, &target, 0, walk, depth + 1);
            }
            parent.push(node);
        }
    }

    /// Pertainyms, with one level of hypernyms shown beneath the first only. grow_tree zeroes its
    /// depth the first time it descends into a pertainym's hypernyms, so every later pertainym of
    /// the same sense is left bare, which is why "hasidic" shows "Orthodox Judaism" under
    /// "Hasidism" but nothing under "Hasidim". The oracle does this, so the engine must.
    fn build_pertainyms(&self, senses: &[Sense]) -> Vec<TreeNode> {
        let mut seen = HashSet::new();
        let mut nodes = Vec::new();
        for sense in senses {
            let mut grown = false;
            for pointer in &sense.synset.pointers {
                if pointer.relation != WnRelation::Pertainym {
                    continue;
                }
                if !source_applies(pointer, sense.which_word) {
                    continue;
                }
                let Some(target) = self
                    .source
                    .synset_at(pointer.target_pos, pointer.target_offset)
                else {
                    continue;
                };
                let self_lower = display_lower(&sense.lemma);
                let terms: Vec<String> = target
                    .words
                    .iter()
                    .map(|w| to_display_form(&w.lemma))
                    .filter(|t| display_lower(t) != self_lower)
                    .collect();
                if terms.is_empty() {
                    continue;
                }
                let children = if grown {
                    Vec::new()
                } else {
                    let walk = TreeWalk {
                        group: HYPERNYM_GROUP,
                        self_lemma_lower: &self_lower,
                        max_depth: 1,
                    };
                    self.grow_nodes(&target, 0, &walk, 0)
                };
                grown = true;
                let node = TreeNode { terms, children };
                if seen.insert(node.label()) {
                    nodes.push(node);
                }
            }
        }
        nodes
    }

    /// Compute Onym's is_defined depth bits per noun lemma. is_defined(lemma, NOUN) sets
    /// bit(HMERONYM) / bit(HHOLONYM) when any of the lemma's noun senses has an immediate
    /// hypernym that itself carries a meronym / holonym pointer (WordNet's HasHoloMero).
    ///
    /// Two faithful details matter. is_defined resolves the lemma through getindex's variants and
    /// ORs the bits across all of them, so a word's depth can be raised by a same-spelt
    /// homograph: the plant "pica-pica" grows its part-of tree deep only because the magpie
    /// "pica_pica" (a getindex variant) inherits a holonym. And is_defined is given the
    /// space-separated lemma, whose getindex variants never recover an underscored multiword
    /// index key, so a multiword term never resolves and stays flat.
    fn compute_noun_depth(&self, senses: &[Sense]) -> HashMap<String, NounDepth> {
        let mut result = HashMap::new();
        for sense in senses {
            if sense.pos != WnPos::Noun || result.contains_key(&sense.lemma) {
                continue;
            }
            let mut meronym = false;
            let mut holonym = false;
            // is_defined sees the space form, whose variants are looked up by exact key; WordNet
            // index keys never contain spaces, so a still-spaced multiword variant cannot match,
            // which is what keeps multiword nouns' trees flat.
            for variant in index_variants(&to_display_form(&sense.lemma))
                .iter()
                .filter(|v| !v.contains(' '))
            {
                for synset in self.source.senses_of(variant, WnPos::Noun) {
                    for pointer in &synset.pointers {
                        if pointer.relation != WnRelation::Hypernym {
                            continue;
                        }
                        let Some(hypernym) = self
                            .source
                            .synset_at(pointer.target_pos, pointer.target_offset)
                        else {
                            continue;
                        };
                        if !meronym
                            && hypernym
                                .pointers
                                .iter()
                                .any(|p| MERONYM_RELATIONS.contains(&p.relation))
                        {
                            meronym = true;
                        }
                        if !holonym
                            && hypernym
                                .pointers
                                .iter()
                                .any(|p| HOLONYM_RELATIONS.contains(&p.relation))
                        {
                            holonym = true;
                        }
                    }
                }
            }
            result.insert(sense.lemma.clone(), NounDepth { meronym, holonym });
        }
        result
    }

    /// Part of (holonyms): the three subtypes combined in order. Grown to full depth only when
    /// the lemma's HHOLONYM bit is set (a noun sense's immediate hypernym is itself
    /// part/member/substance of something); otherwise just the word's own holonyms, one level
    /// deep.
    fn build_holonyms(
        &self,
        senses: &[Sense],
        noun_depth: &HashMap<String, NounDepth>,
    ) -> Vec<TreeNode> {
        let mut seen = HashSet::new();
        let mut nodes = Vec::new();
        for sense in senses {
            let deep = noun_depth.get(&sense.lemma).is_some_and(|d| d.holonym);
            let max_depth = if deep { MAX_TREE_DEPTH } else { 1 };
            let self_lower = display_lower(&sense.lemma);
            for group in HOLONYM_GROUPS {
                let walk = TreeWalk {
                    group,
                    self_lemma_lower: &self_lower,
                    max_depth,
                };
                for node in self.grow_nodes(&sense.synset, sense.which_word, &walk, 0) {
                    if seen.insert(node.label()) {
                        nodes.push(node);
                    }
                }
            }
        }
        nodes
    }

    /// Parts (meronyms): the three subtypes combined in order. Grown to full depth, with each
    /// ancestor's inherited meronyms traced, only when the lemma's HMERONYM bit is set; otherwise
    /// just the word's own meronyms, one level deep.
    fn build_meronyms(
        &self,
        senses: &[Sense],
        noun_depth: &HashMap<String, NounDepth>,
    ) -> Vec<TreeNode> {
        let mut seen = HashSet::new();
        let mut nodes = Vec::new();
        for sense in senses {
            let deep = noun_depth.get(&sense.lemma).is_some_and(|d| d.meronym);
            let max_depth = if deep { MAX_TREE_DEPTH } else { 1 };
            let self_lower = display_lower(&sense.lemma);
            let mut top_level = Vec::new();
            for group in MERONYM_GROUPS {
                let walk = TreeWalk {
                    group,
                    self_lemma_lower: &self_lower,
                    max_depth,
                };
                top_level.extend(self.grow_nodes(&sense.synset, sense.which_word, &walk, 0));
            }
            if deep {
                top_level.extend(self.trace_inherit(
                    &sense.synset,
                    sense.which_word,
                    &self_lower,
                    1,
                ));
            }
            for node in top_level {
                if seen.insert(node.label()) {
                    nodes.push(node);
                }
            }
        }
        nodes
    }

    /// Trace inherited meronyms up the is-a chain, keeping only ancestors that contribute parts.
    fn trace_inherit(
        &self,
        synset: &WnSynset,
        which_word: usize,
        self_lemma_lower: &str,
        depth: usize,
    ) -> Vec<TreeNode> {
        let mut nodes = Vec::new();
        for pointer in &synset.pointers {
            if pointer.relation != WnRelation::Hypernym {
                continue;
            }
            if !source_applies(pointer, which_word) {
                continue;
            }
            let Some(ancestor) = self
                .source
                .synset_at(pointer.target_pos, pointer.target_offset)
            else {
                continue;
            };
            let ancestor_terms: Vec<String> = ancestor
                .words
                .iter()
                .map(|w| to_display_form(&w.lemma))
                .filter(|t| display_lower(t) != self_lemma_lower)
                .collect();
            if ancestor_terms.is_empty() {
                continue;
            }
            let mut children = Vec::new();
            for group in MERONYM_GROUPS {
                let walk = TreeWalk {
                    group,
                    self_lemma_lower,
                    max_depth: MAX_TREE_DEPTH,
                };
                children.extend(self.grow_nodes(&ancestor, 0, &walk, 0));
            }
            if depth + 1 < MAX_TREE_DEPTH {
                children.extend(self.trace_inherit(&ancestor, 0, self_lemma_lower, depth + 1));
            }
            if !children.is_empty() {
                nodes.push(TreeNode {
                    terms: ancestor_terms,
                    children,
                });
            }
        }
        nodes
    }
}

// --- Shared helpers ----------------------------------------------------------------------------

fn normalize(query: &str) -> Option<String> {
    let mut lemma = query.trim().replace(' ', "_");
    if let Some(paren) = lemma.find('(') {
        lemma.truncate(paren);
    }
    let lemma = ascii_lower(&lemma);
    if lemma.is_empty() || lemma == "." || lemma == "-" || lemma == "_" {
        return None;
    }
    Some(lemma)
}

/// WordNet's read_synset keeps the LAST synset word whose lower case matches, not the first, so a
/// synset carrying both "utopian" and "Utopian" resolves to the second; lexical pointers
/// restricted to that word index then apply, which is how "utopian" reaches its "Utopia"
/// derivation. The result is 1-based, or 0 when no word matches.
fn which_word(synset: &WnSynset, lemma: &str) -> usize {
    let target = display_lower(lemma);
    synset
        .words
        .iter()
        .rposition(|word| display_lower(&word.lemma) == target)
        .map_or(0, |i| i + 1)
}

/// Resolve the lemma's case the way Onym's populate_synonyms does. While listing the prime
/// sense's words it re-points the lemma at each synset word that matches it case-insensitively,
/// so the LAST such word wins: "wordsworth" becomes "Wordsworth", but a synset listing both
/// "Moon" and "moon" settles on "moon", which is why the lower-cased query then sorts as an exact
/// match. A spelling already claimed as another item's lemma is skipped (is_synm_a_lemma), so a
/// demonym adjective stays lower-case once its proper-noun twin has taken the capital.
fn resolve_display_lemma(
    lemma: &str,
    sense1_synset: &WnSynset,
    claimed: &HashSet<String>,
) -> String {
    let mut current = to_display_form(lemma);
    let target = display_lower(lemma);
    for word in &sense1_synset.words {
        let word_display = to_display_form(&word.lemma);
        if claimed.contains(&word_display) {
            continue;
        }
        if display_lower(&word_display) == target {
            current = word_display;
        }
    }
    current
}

fn source_applies(pointer: &WnPointer, which_word: usize) -> bool {
    pointer.source_word_index == 0 || pointer.source_word_index == which_word
}

fn push_unique(list: &mut Vec<String>, term: String) {
    if !list.contains(&term) {
        list.push(term);
    }
}

/// Merge `extra` implication terms into `existing`, keeping first-occurrence order without
/// duplicates.
fn merge_words(existing: &mut Vec<String>, extra: &[String]) {
    for term in extra {
        push_unique(existing, term.clone());
    }
}

fn add_words(sections: &mut Vec<Section>, title: &'static str, words: Vec<String>) {
    if !words.is_empty() {
        sections.push(Section {
            title,
            items: SectionItems::Words(words),
        });
    }
}

fn add_tree(sections: &mut Vec<Section>, title: &'static str, items: Vec<TreeNode>) {
    if !items.is_empty() {
        sections.push(Section {
            title,
            items: SectionItems::Tree(items),
        });
    }
}

fn pos_name(pos: WnPos) -> &'static str {
    match pos {
        WnPos::Noun => "noun",
        WnPos::Verb => "verb",
        WnPos::Adjective => "adjective",
        WnPos::Adverb => "adverb",
    }
}

/// Split a gloss into its definition and example sentences, reproducing Onym's parse_definition
/// index for index. WordNet glosses run the definition and quoted examples together, separated by
/// semicolons, and an example may carry an attribution after its closing quote. The gloss is
/// wrapped in parentheses to match the form the original parser expects (the WordNet C library
/// adds them); the trailing space some data lines carry is trimmed first, as the C library does.
/// The accumulated text is split on the `|` separators the scan plants: the first part is the
/// gloss, the remaining non-empty parts are the examples.
fn parse_definition(gloss: &str) -> (String, Vec<String>) {
    let wrapped: Vec<char> = std::iter::once('(')
        .chain(gloss.trim_end().chars())
        .chain(std::iter::once(')'))
        .collect();
    let len = wrapped.len() - 1; // skip the closing parenthesis
    let mut out: Vec<char> = Vec::new();
    let mut brace_met = 0usize;
    let mut double_quotes = 0u32;
    let mut just_ended = false;
    let mut i = 1; // skip the opening parenthesis
    while i < len {
        let mut ch = Some(wrapped[i]);
        if wrapped[i] == '"' {
            // An opening quote (even count) starts an example: turn the preceding separator into
            // a delimiter, collapsing a preceding comma as the original does for "compound".
            if double_quotes.is_multiple_of(2) && wrapped[i - 1] != '(' {
                if out.len() >= 2 && out[out.len() - 2] == ',' {
                    out.pop();
                }
                if let Some(last) = out.last_mut()
                    && *last != '|'
                {
                    *last = '|';
                }
            }
            double_quotes += 1;
            just_ended = false;
            ch = None;
        } else if wrapped[i] == ' ' && just_ended {
            ch = None;
        } else if wrapped[i] == '(' && just_ended {
            just_ended = false;
            brace_met = 1;
        } else if wrapped[i] == ')' && brace_met != 0 {
            // Close the hoisted parenthesised note at the front of the accumulated text.
            out.insert(brace_met - 1, ')');
            out.insert(brace_met, ' ');
            brace_met = 0;
            ch = None;
        } else if wrapped[i] == ';'
            && (i + 1 == len
                || (double_quotes.is_multiple_of(2) && wrapped.get(i + 2) == Some(&'"'))
                || wrapped.get(i + 2) == Some(&'('))
        {
            ch = Some('|');
            just_ended = true;
        }
        if let Some(c) = ch {
            if brace_met == 0 {
                out.push(c);
            } else {
                // A parenthesised note immediately after a separator is hoisted, character by
                // character, to the front of the accumulated text.
                out.insert(brace_met - 1, c);
                brace_met += 1;
            }
        }
        i += 1;
    }
    let joined: String = out.into_iter().collect();
    let mut parts = joined.split('|');
    let gloss = parts.next().unwrap_or("").to_string();
    let examples = parts
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    (gloss, examples)
}
