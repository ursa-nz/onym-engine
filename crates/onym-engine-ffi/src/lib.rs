// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The C ABI over the engine, the archive libonym links. This crate is the project's only unsafe
//! code: it owns the boundary where Rust values become plain C structs and back. The contract is
//! the hand-written header in `include/onym-core.h`, kept in step with this file; every returned
//! allocation is reclaimed by exactly one matching `onym_core_*` free function, and fields the
//! header documents as static are never freed.
//!
//! Strings cross the boundary as UTF-8, the encoding of the OEWN database and of every string the
//! engine produces, so a result carries accented lemmas and glosses unchanged; queries arrive as
//! whatever the application passes (GTK hands the engine UTF-8) and the engine normalises them
//! itself.

use onym_engine::{
    Antonym, Definition, Engine, Entry, LanguageWords, SectionItems, SenseTranslations, TreeNode,
};
use std::ffi::{CStr, CString, c_char, c_int};
use std::path::PathBuf;

/// The engine handle behind the opaque `OnymCoreEngine` the header declares.
pub struct OnymCoreEngine(Engine);

/// One node of a lexical hierarchy, as the header lays it out.
#[repr(C)]
pub struct OnymCoreTreeNode {
    terms: *mut *mut c_char,
    children: *mut *mut OnymCoreTreeNode,
}

/// One sense of a word, as the header lays it out.
#[repr(C)]
pub struct OnymCoreDefinition {
    pos: *const c_char,
    gloss: *mut c_char,
    examples: *mut *mut c_char,
}

/// An opposite of the looked-up word, as the header lays it out.
#[repr(C)]
pub struct OnymCoreAntonym {
    term: *mut c_char,
    direct: c_int,
    implications: *mut *mut c_char,
}

/// One language's words for a sense, as the header lays it out.
#[repr(C)]
pub struct OnymCoreLanguageWords {
    language: *mut c_char,
    words: *mut *mut c_char,
}

/// One looked-up sense's translations, as the header lays it out.
#[repr(C)]
pub struct OnymCoreSenseTranslations {
    pos: *const c_char,
    gloss: *mut c_char,
    languages: *mut OnymCoreLanguageWords,
    n_languages: usize,
}

const ONYM_CORE_SECTION_DEFINITIONS: c_int = 0;
const ONYM_CORE_SECTION_WORDS: c_int = 1;
const ONYM_CORE_SECTION_ANTONYMS: c_int = 2;
const ONYM_CORE_SECTION_TREE: c_int = 3;
const ONYM_CORE_SECTION_ETYMOLOGY: c_int = 4;
const ONYM_CORE_SECTION_TRANSLATIONS: c_int = 5;

/// A titled group of items of one kind, as the header lays it out. Exactly the array named by
/// `kind` is non-null.
#[repr(C)]
pub struct OnymCoreSection {
    kind: c_int,
    title: *const c_char,
    n_items: usize,
    definitions: *mut OnymCoreDefinition,
    words: *mut *mut c_char,
    antonyms: *mut OnymCoreAntonym,
    tree: *mut *mut OnymCoreTreeNode,
    translations: *mut OnymCoreSenseTranslations,
}

/// The whole entry for a looked-up word, as the header lays it out.
#[repr(C)]
pub struct OnymCoreEntry {
    term: *mut c_char,
    n_sections: usize,
    sections: *mut OnymCoreSection,
}

/// Allocate a C copy of a string. Engine strings never contain NUL (the database has none and
/// queries are normalised), so the fallback strip is belt-and-braces, never behaviour.
fn c_string(s: &str) -> *mut c_char {
    let owned = match CString::new(s) {
        Ok(owned) => owned,
        Err(_) => CString::new(s.replace('\0', "")).unwrap_or_default(),
    };
    owned.into_raw()
}

/// Free a string made by [`c_string`].
unsafe fn c_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

/// Allocate a NULL-terminated array of C strings.
fn c_strv(items: &[String]) -> *mut *mut c_char {
    let mut out: Vec<*mut c_char> = Vec::with_capacity(items.len() + 1);
    out.extend(items.iter().map(|item| c_string(item)));
    out.push(std::ptr::null_mut());
    Box::into_raw(out.into_boxed_slice()) as *mut *mut c_char
}

/// Free an array made by [`c_strv`], walking to the terminator to recover its length.
unsafe fn c_strv_free(strv: *mut *mut c_char) {
    if strv.is_null() {
        return;
    }
    let mut len = 0;
    unsafe {
        while !(*strv.add(len)).is_null() {
            c_string_free(*strv.add(len));
            len += 1;
        }
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            strv,
            len + 1,
        )));
    }
}

/// The static C spelling of a part of speech name the model carries.
fn pos_cstr(pos: Option<&'static str>) -> *const c_char {
    match pos {
        Some("noun") => c"noun".as_ptr(),
        Some("verb") => c"verb".as_ptr(),
        Some("adjective") => c"adjective".as_ptr(),
        Some("adverb") => c"adverb".as_ptr(),
        _ => std::ptr::null(),
    }
}

/// The static C spelling of a section title. The titles are the closed set of
/// `spec/engine.md` section 6.1.
fn title_cstr(title: &'static str) -> *const c_char {
    match title {
        "Definitions" => c"Definitions".as_ptr(),
        "Etymology" => c"Etymology".as_ptr(),
        "Translations" => c"Translations".as_ptr(),
        "Synonyms" => c"Synonyms".as_ptr(),
        "Antonyms" => c"Antonyms".as_ptr(),
        "Derived forms" => c"Derived forms".as_ptr(),
        "Similar to" => c"Similar to".as_ptr(),
        "Attributes" => c"Attributes".as_ptr(),
        "Causes" => c"Causes".as_ptr(),
        "Entails" => c"Entails".as_ptr(),
        "Pertains to" => c"Pertains to".as_ptr(),
        "Is a kind of" => c"Is a kind of".as_ptr(),
        "Kinds" => c"Kinds".as_ptr(),
        "Part of" => c"Part of".as_ptr(),
        "Parts" => c"Parts".as_ptr(),
        "Domains" => c"Domains".as_ptr(),
        _ => c"".as_ptr(),
    }
}

/// Allocate a C copy of a tree node, children included.
fn tree_node(node: &TreeNode) -> *mut OnymCoreTreeNode {
    let mut children: Vec<*mut OnymCoreTreeNode> = Vec::with_capacity(node.children.len() + 1);
    children.extend(node.children.iter().map(tree_node));
    children.push(std::ptr::null_mut());
    Box::into_raw(Box::new(OnymCoreTreeNode {
        terms: c_strv(&node.terms),
        children: Box::into_raw(children.into_boxed_slice()) as *mut *mut OnymCoreTreeNode,
    }))
}

/// Free a node made by [`tree_node`], recursing through its children before the node itself.
unsafe fn tree_node_free(node: *mut OnymCoreTreeNode) {
    if node.is_null() {
        return;
    }
    unsafe {
        let owned = Box::from_raw(node);
        c_strv_free(owned.terms);
        let mut len = 0;
        while !(*owned.children.add(len)).is_null() {
            tree_node_free(*owned.children.add(len));
            len += 1;
        }
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            owned.children,
            len + 1,
        )));
    }
}

fn tree_array(nodes: &[TreeNode]) -> *mut *mut OnymCoreTreeNode {
    let mut out: Vec<*mut OnymCoreTreeNode> = Vec::with_capacity(nodes.len() + 1);
    out.extend(nodes.iter().map(tree_node));
    out.push(std::ptr::null_mut());
    Box::into_raw(out.into_boxed_slice()) as *mut *mut OnymCoreTreeNode
}

fn definition(definition: &Definition) -> OnymCoreDefinition {
    OnymCoreDefinition {
        pos: pos_cstr(definition.pos),
        gloss: c_string(&definition.gloss),
        examples: c_strv(&definition.examples),
    }
}

fn antonym(antonym: &Antonym) -> OnymCoreAntonym {
    OnymCoreAntonym {
        term: c_string(&antonym.term),
        direct: c_int::from(antonym.direct),
        implications: c_strv(&antonym.implications),
    }
}

fn language_words(language: &LanguageWords) -> OnymCoreLanguageWords {
    OnymCoreLanguageWords {
        language: c_string(&language.language),
        words: c_strv(&language.words),
    }
}

fn sense_translations(block: &SenseTranslations) -> OnymCoreSenseTranslations {
    OnymCoreSenseTranslations {
        pos: pos_cstr(block.pos),
        gloss: c_string(&block.gloss),
        languages: Box::into_raw(
            block.languages.iter().map(language_words).collect::<Box<[_]>>(),
        ) as *mut OnymCoreLanguageWords,
        n_languages: block.languages.len(),
    }
}

fn entry_to_c(entry: &Entry) -> *mut OnymCoreEntry {
    let sections: Vec<OnymCoreSection> = entry
        .sections
        .iter()
        .map(|section| {
            let title = title_cstr(section.title);
            match &section.items {
                SectionItems::Definitions(items) => OnymCoreSection {
                    kind: ONYM_CORE_SECTION_DEFINITIONS,
                    title,
                    n_items: items.len(),
                    definitions: Box::into_raw(items.iter().map(definition).collect::<Box<[_]>>())
                        as *mut OnymCoreDefinition,
                    words: std::ptr::null_mut(),
                    antonyms: std::ptr::null_mut(),
                    tree: std::ptr::null_mut(),
                    translations: std::ptr::null_mut(),
                },
                SectionItems::Words(items) => OnymCoreSection {
                    kind: ONYM_CORE_SECTION_WORDS,
                    title,
                    n_items: items.len(),
                    definitions: std::ptr::null_mut(),
                    words: c_strv(items),
                    antonyms: std::ptr::null_mut(),
                    tree: std::ptr::null_mut(),
                    translations: std::ptr::null_mut(),
                },
                SectionItems::Antonyms(items) => OnymCoreSection {
                    kind: ONYM_CORE_SECTION_ANTONYMS,
                    title,
                    n_items: items.len(),
                    definitions: std::ptr::null_mut(),
                    words: std::ptr::null_mut(),
                    antonyms: Box::into_raw(items.iter().map(antonym).collect::<Box<[_]>>())
                        as *mut OnymCoreAntonym,
                    tree: std::ptr::null_mut(),
                    translations: std::ptr::null_mut(),
                },
                SectionItems::Tree(items) => OnymCoreSection {
                    kind: ONYM_CORE_SECTION_TREE,
                    title,
                    n_items: items.len(),
                    definitions: std::ptr::null_mut(),
                    words: std::ptr::null_mut(),
                    antonyms: std::ptr::null_mut(),
                    tree: tree_array(items),
                    translations: std::ptr::null_mut(),
                },
                // Etymology prose crosses as a plain string array, reusing the `words` slot, so the
                // section struct keeps its layout. The kind tells the consumer to render the strings
                // as paragraphs rather than navigable terms.
                SectionItems::Etymology(paragraphs) => OnymCoreSection {
                    kind: ONYM_CORE_SECTION_ETYMOLOGY,
                    title,
                    n_items: paragraphs.len(),
                    definitions: std::ptr::null_mut(),
                    words: c_strv(paragraphs),
                    antonyms: std::ptr::null_mut(),
                    tree: std::ptr::null_mut(),
                    translations: std::ptr::null_mut(),
                },
                // Sense translations cross in their own array: one block per sense, each its pos,
                // gloss, and per-language word lists. The kind tells the consumer to render it.
                SectionItems::Translations(items) => OnymCoreSection {
                    kind: ONYM_CORE_SECTION_TRANSLATIONS,
                    title,
                    n_items: items.len(),
                    definitions: std::ptr::null_mut(),
                    words: std::ptr::null_mut(),
                    antonyms: std::ptr::null_mut(),
                    tree: std::ptr::null_mut(),
                    translations: Box::into_raw(
                        items.iter().map(sense_translations).collect::<Box<[_]>>(),
                    ) as *mut OnymCoreSenseTranslations,
                },
            }
        })
        .collect();

    Box::into_raw(Box::new(OnymCoreEntry {
        term: c_string(&entry.term),
        n_sections: sections.len(),
        sections: Box::into_raw(sections.into_boxed_slice()) as *mut OnymCoreSection,
    }))
}

/// Read a C string argument; a null pointer reads as empty, which the engine treats as a miss.
unsafe fn arg_str<'a>(s: *const c_char) -> std::borrow::Cow<'a, str> {
    if s.is_null() {
        return std::borrow::Cow::Borrowed("");
    }
    unsafe { CStr::from_ptr(s) }.to_string_lossy()
}

/// Open an engine over the WordNet 3.0 database in `data_dir`, read-only and in place.
///
/// # Safety
/// `data_dir` must be a valid NUL-terminated string or null. `error`, when non-null, must point
/// at writable storage for one `char *`; on failure it receives a message to free with
/// [`onym_core_string_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onym_core_open(
    data_dir: *const c_char,
    error: *mut *mut c_char,
) -> *mut OnymCoreEngine {
    let set_error = |message: &str| {
        if !error.is_null() {
            unsafe { *error = c_string(message) };
        }
    };

    if data_dir.is_null() {
        set_error("no data directory given");
        return std::ptr::null_mut();
    }
    let dir = unsafe { CStr::from_ptr(data_dir) };
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(dir.to_bytes()))
    };
    #[cfg(not(unix))]
    let path = PathBuf::from(dir.to_string_lossy().into_owned());

    match Engine::open(&path) {
        Ok(engine) => Box::into_raw(Box::new(OnymCoreEngine(engine))),
        Err(open_error) => {
            set_error(&open_error.to_string());
            std::ptr::null_mut()
        }
    }
}

/// Release an engine. Null is allowed.
///
/// # Safety
/// `engine` must be null or a pointer from [`onym_core_open`] not yet freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onym_core_free(engine: *mut OnymCoreEngine) {
    if !engine.is_null() {
        drop(unsafe { Box::from_raw(engine) });
    }
}

/// Look `word` up; null when the word is simply not in WordNet.
///
/// # Safety
/// `engine` must be null or a live pointer from [`onym_core_open`]; `word` must be a valid
/// NUL-terminated string or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onym_core_lookup(
    engine: *const OnymCoreEngine,
    word: *const c_char,
) -> *mut OnymCoreEntry {
    if engine.is_null() {
        return std::ptr::null_mut();
    }
    let word = unsafe { arg_str(word) };
    match unsafe { &(*engine).0 }.lookup(&word) {
        Some(entry) => entry_to_c(&entry),
        None => std::ptr::null_mut(),
    }
}

/// Release an entry and everything it owns. Null is allowed.
///
/// # Safety
/// `entry` must be null or a pointer from [`onym_core_lookup`] not yet freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onym_core_entry_free(entry: *mut OnymCoreEntry) {
    if entry.is_null() {
        return;
    }
    unsafe {
        let owned = Box::from_raw(entry);
        c_string_free(owned.term);
        let sections = Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            owned.sections,
            owned.n_sections,
        ));
        for section in &sections {
            if !section.definitions.is_null() {
                let items = Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                    section.definitions,
                    section.n_items,
                ));
                for item in &items {
                    c_string_free(item.gloss);
                    c_strv_free(item.examples);
                }
            }
            c_strv_free(section.words);
            if !section.antonyms.is_null() {
                let items = Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                    section.antonyms,
                    section.n_items,
                ));
                for item in &items {
                    c_string_free(item.term);
                    c_strv_free(item.implications);
                }
            }
            if !section.tree.is_null() {
                let mut len = 0;
                while !(*section.tree.add(len)).is_null() {
                    tree_node_free(*section.tree.add(len));
                    len += 1;
                }
                drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                    section.tree,
                    len + 1,
                )));
            }
            if !section.translations.is_null() {
                let blocks = Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                    section.translations,
                    section.n_items,
                ));
                for block in &blocks {
                    c_string_free(block.gloss);
                    let languages = Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                        block.languages,
                        block.n_languages,
                    ));
                    for language in &languages {
                        c_string_free(language.language);
                        c_strv_free(language.words);
                    }
                }
            }
        }
    }
}

/// Headwords beginning with `prefix`, capped at `max` (0 means no cap). Never null.
///
/// # Safety
/// `engine` must be null or a live pointer from [`onym_core_open`]; `prefix` must be a valid
/// NUL-terminated string or null. Free the result with [`onym_core_strv_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onym_core_complete(
    engine: *const OnymCoreEngine,
    prefix: *const c_char,
    max: usize,
) -> *mut *mut c_char {
    if engine.is_null() {
        return c_strv(&[]);
    }
    let prefix = unsafe { arg_str(prefix) };
    c_strv(&unsafe { &(*engine).0 }.complete(&prefix, max))
}

/// Spelling suggestions for a missed `word`, capped at `max` (0 means no cap). Never null.
///
/// # Safety
/// `engine` must be null or a live pointer from [`onym_core_open`]; `word` must be a valid
/// NUL-terminated string or null. Free the result with [`onym_core_strv_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onym_core_suggest(
    engine: *const OnymCoreEngine,
    word: *const c_char,
    max: usize,
) -> *mut *mut c_char {
    if engine.is_null() {
        return c_strv(&[]);
    }
    let word = unsafe { arg_str(word) };
    c_strv(&unsafe { &(*engine).0 }.suggest(&word, max))
}

/// How many headwords the lemma index holds.
///
/// # Safety
/// `engine` must be null or a live pointer from [`onym_core_open`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onym_core_lemma_count(engine: *const OnymCoreEngine) -> usize {
    if engine.is_null() {
        return 0;
    }
    unsafe { &(*engine).0 }.lemma_count()
}

/// The headword at `index` in sorted order, or null when out of range.
///
/// # Safety
/// `engine` must be null or a live pointer from [`onym_core_open`]. Free the result with
/// [`onym_core_string_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onym_core_lemma_at(
    engine: *const OnymCoreEngine,
    index: usize,
) -> *mut c_char {
    if engine.is_null() {
        return std::ptr::null_mut();
    }
    match unsafe { &(*engine).0 }.lemma_at(index) {
        Some(lemma) => c_string(lemma),
        None => std::ptr::null_mut(),
    }
}

/// Free a NULL-terminated string array returned by this library. Null is allowed.
///
/// # Safety
/// `strv` must be null or an array returned by this library not yet freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onym_core_strv_free(strv: *mut *mut c_char) {
    unsafe { c_strv_free(strv) };
}

/// Free a single string returned by this library. Null is allowed.
///
/// # Safety
/// `s` must be null or a string returned by this library not yet freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onym_core_string_free(s: *mut c_char) {
    unsafe { c_string_free(s) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    // The WordNet base, materialised once from the onym-data submodule (base-only). The C boundary
    // takes a path, so the FFI tests pin it to the shipped data set, as the engine's own tests do.
    fn data_dir() -> Option<&'static CStr> {
        static DIR: OnceLock<Option<CString>> = OnceLock::new();
        DIR.get_or_init(|| {
            let base = prepare_base()?;
            CString::new(base.as_os_str().as_bytes()).ok()
        })
        .as_deref()
    }

    fn prepare_base() -> Option<PathBuf> {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../onym-data/prepare.sh");
        if !script.is_file() {
            eprintln!("skipping: onym-data submodule not checked out");
            return None;
        }
        let target =
            std::env::temp_dir().join(format!("onym-ffi-wordnet-base-{}", std::process::id()));
        if !target.join("index.noun").is_file() {
            let status = std::process::Command::new(&script)
                .arg("--base-only")
                .arg(&target)
                .status();
            match status {
                Ok(code) if code.success() => {}
                _ => {
                    eprintln!("skipping: onym-data prepare.sh did not produce a base");
                    return None;
                }
            }
        }
        Some(target)
    }

    fn engine() -> Option<*mut OnymCoreEngine> {
        let data = data_dir()?;
        let mut error: *mut c_char = std::ptr::null_mut();
        let engine = unsafe { onym_core_open(data.as_ptr(), &mut error) };
        assert!(!engine.is_null());
        Some(engine)
    }

    #[test]
    fn lookup_builds_and_frees_an_entry() {
        let Some(engine) = engine() else { return };
        unsafe {
            let entry = onym_core_lookup(engine, c"dog".as_ptr());
            assert!(!entry.is_null());
            assert_eq!(CStr::from_ptr((*entry).term).to_str(), Ok("dog"));
            assert!((*entry).n_sections > 0);
            let first = &*(*entry).sections;
            assert_eq!(first.kind, ONYM_CORE_SECTION_DEFINITIONS);
            assert_eq!(CStr::from_ptr(first.title).to_str(), Ok("Definitions"));
            assert!(!first.definitions.is_null());
            onym_core_entry_free(entry);

            assert!(onym_core_lookup(engine, c"zzzxyqq".as_ptr()).is_null());
            onym_core_free(engine);
        }
    }

    #[test]
    fn completion_suggestion_and_lemmas_cross_the_boundary() {
        let Some(engine) = engine() else { return };
        unsafe {
            let matches = onym_core_complete(engine, c"serend".as_ptr(), 10);
            assert!(!(*matches).is_null());
            assert_eq!(CStr::from_ptr(*matches).to_str(), Ok("serendipitous"));
            onym_core_strv_free(matches);

            let suggestions = onym_core_suggest(engine, c"beutiful".as_ptr(), 5);
            assert_eq!(CStr::from_ptr(*suggestions).to_str(), Ok("beautiful"));
            onym_core_strv_free(suggestions);

            assert!(onym_core_lemma_count(engine) > 100_000);
            let lemma = onym_core_lemma_at(engine, 0);
            assert!(!lemma.is_null());
            onym_core_string_free(lemma);
            assert!(onym_core_lemma_at(engine, onym_core_lemma_count(engine)).is_null());

            onym_core_free(engine);
        }
    }
}
