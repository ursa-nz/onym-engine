// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The JNI library Onymdroid's `:core` loads, exporting the engine as
//! `Java_nz_ursa_onymdroid_core_NativeEngine_*` symbols. This file is the contract for the
//! boundary, the way `onym-core.h` is the contract for the C ABI: the Kotlin decoder in
//! Onymdroid's `:core` is the only other party, the two change together, and the format is
//! therefore versionless.
//!
//! # Shape of the boundary
//!
//! One call per operation, one buffer per answer. A lookup crosses the boundary exactly once:
//! the whole entry is encoded into a byte array on this side and decoded into the Kotlin model
//! classes on the other. There are no per-field callbacks and no JNI object construction.
//!
//! The exported operations, with their Kotlin signatures:
//!
//! - `open(path: String): ByteArray` opens an engine over a WordNet database directory, read in
//!   place, read-only. The answer is tag byte 1 and a little-endian u64 handle on success, or
//!   tag byte 0 and an error string on failure.
//! - `close(handle: Long)` frees the engine. A zero handle is ignored.
//! - `lookup(handle: Long, word: String): ByteArray?` answers null when the word is simply not
//!   in WordNet, otherwise an encoded entry.
//! - `complete(handle: Long, prefix: String, max: Int): ByteArray` and
//!   `suggest(handle: Long, word: String, max: Int): ByteArray` answer encoded string lists;
//!   a max of 0 means no cap.
//! - `dump(handle: Long, word: String): ByteArray` answers the `--dump` text of
//!   `spec/dump-format.md` as raw UTF-8, no prefix.
//! - `lemmaCount(handle: Long): Long` and `lemmaAt(handle: Long, index: Long): ByteArray?`
//!   expose the sorted lemma index for the surprise-me action; `lemmaAt` is raw UTF-8 and null
//!   out of range.
//!
//! # Wire format
//!
//! All integers are little-endian. A *u32* is the only length and count type. A *string* is a
//! u32 byte length followed by that many UTF-8 bytes. A *string list* is a u32 count followed
//! by that many strings.
//!
//! An *entry* is: string term; u32 section count; then per section a string title, one kind
//! byte, and the kind's payload:
//!
//! - kind 0, definitions: u32 count; each item is a presence byte (0 or 1) and the part of
//!   speech string when present, then a gloss string and an examples string list.
//! - kind 1, words: a string list.
//! - kind 2, antonyms: u32 count; each item is a term string, a direct byte (0 or 1), and an
//!   implications string list.
//! - kind 3, tree: a *node list*, where a node list is a u32 count of nodes and each node is a
//!   terms string list followed by its children as a node list, depth-first.
//! - kind 4, etymology: a string list of prose paragraphs. It crosses only when the optional
//!   etymology overlay is present, so a plain WordNet build never emits it.
//! - kind 5, translations: u32 block count; each block is a presence byte (0 or 1) and the part of
//!   speech string when present, a gloss string, then a u32 language count, each a language name
//!   string and a words string list. It crosses only when the optional translations overlay is
//!   present.
//!
//! # Strings crossing inward
//!
//! Queries arrive as JNI strings and are read with `GetStringChars`, which is true UTF-16,
//! never `GetStringUTFChars`, which is modified UTF-8 (CESU-8 surrogate pairs, NUL as
//! 0xC0 0x80). An unpaired surrogate decodes to U+FFFD. The engine's query rules
//! (`spec/engine.md` section 3) then apply unchanged; nothing here trims, lowercases, or
//! otherwise preprocesses.
//!
//! # Threading and lifetime
//!
//! An engine handle is immutable after open and safe for concurrent calls from any thread; the
//! only unsafe transitions are `open` and `close`, which the Kotlin side serialises. The JNI
//! environment pointer is used only within the call it was passed to, as the JNI specification
//! requires. The function table is reached by index; the indices are transcribed from `jni.h`
//! and fixed by the JNI 1.6 specification, which every later JVM preserves.

mod codec;

use onym_engine::Engine;
use std::ffi::c_void;
use std::path::Path;

/// A JNI environment: a pointer to a pointer to the function table.
type JniEnv = *mut *const c_void;
/// An opaque JVM object reference; this library never looks inside one.
type JObject = *mut c_void;
type JString = *mut c_void;
type JByteArray = *mut c_void;
/// JNI's size type for string and array lengths.
type JSize = i32;

// The function table indices this library uses, transcribed from jni.h. The layout is fixed by
// the JNI specification (the functions below are unchanged since JNI 1.1 and the table layout
// since 1.6), so the indices are constants rather than a generated binding.
const GET_STRING_LENGTH: usize = 164;
const GET_STRING_CHARS: usize = 165;
const RELEASE_STRING_CHARS: usize = 166;
const NEW_BYTE_ARRAY: usize = 176;
const SET_BYTE_ARRAY_REGION: usize = 208;

/// Fetch a function table entry.
///
/// # Safety
/// `env` must be the JNI environment pointer the JVM passed to the current native call, and
/// `index` one of the constants above.
unsafe fn jni_entry(env: JniEnv, index: usize) -> *const c_void {
    let table = unsafe { *env } as *const *const c_void;
    unsafe { *table.add(index) }
}

/// Read a JVM string as true UTF-16 and convert it ourselves; an unpaired surrogate becomes
/// U+FFFD. Null reads as empty, so a null query simply finds nothing.
///
/// # Safety
/// `env` must be the environment of the current call; `s` must be null or a valid `jstring`
/// reference for the duration of the call.
unsafe fn jstring_to_string(env: JniEnv, s: JString) -> String {
    if s.is_null() {
        return String::new();
    }
    unsafe {
        let length: extern "system" fn(JniEnv, JString) -> JSize =
            std::mem::transmute(jni_entry(env, GET_STRING_LENGTH));
        let chars: extern "system" fn(JniEnv, JString, *mut u8) -> *const u16 =
            std::mem::transmute(jni_entry(env, GET_STRING_CHARS));
        let release: extern "system" fn(JniEnv, JString, *const u16) =
            std::mem::transmute(jni_entry(env, RELEASE_STRING_CHARS));
        let len = length(env, s);
        let units = chars(env, s, std::ptr::null_mut());
        if units.is_null() {
            return String::new();
        }
        let slice = std::slice::from_raw_parts(units, len as usize);
        let out = char::decode_utf16(slice.iter().copied())
            .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
        release(env, s, units);
        out
    }
}

/// Copy `bytes` into a fresh JVM byte array. A null answer means the JVM could not allocate,
/// in which case it has a pending OutOfMemoryError that throws when this call returns.
///
/// # Safety
/// `env` must be the environment of the current call.
unsafe fn byte_array(env: JniEnv, bytes: &[u8]) -> JByteArray {
    unsafe {
        let new: extern "system" fn(JniEnv, JSize) -> JByteArray =
            std::mem::transmute(jni_entry(env, NEW_BYTE_ARRAY));
        let fill: extern "system" fn(JniEnv, JByteArray, JSize, JSize, *const i8) =
            std::mem::transmute(jni_entry(env, SET_BYTE_ARRAY_REGION));
        let array = new(env, bytes.len() as JSize);
        if !array.is_null() {
            fill(
                env,
                array,
                0,
                bytes.len() as JSize,
                bytes.as_ptr() as *const i8,
            );
        }
        array
    }
}

/// Borrow the engine behind a handle; a zero or stale-free handle answers None and the call
/// degrades to the empty answer, mirroring the ffi crate's null tolerance.
///
/// # Safety
/// `handle` must be zero or a value from a successful `open` not yet passed to `close`.
unsafe fn engine_ref<'a>(handle: i64) -> Option<&'a Engine> {
    unsafe { (handle as *const Engine).as_ref() }
}

/// Open an engine; the answer buffer carries a handle or an error message.
///
/// # Safety
/// Called only by the JVM with its own `env` and a valid `path` string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_nz_ursa_onymdroid_core_NativeEngine_open(
    env: JniEnv,
    _this: JObject,
    path: JString,
) -> JByteArray {
    let path = unsafe { jstring_to_string(env, path) };
    let buffer = match Engine::open(Path::new(&path)) {
        Ok(engine) => codec::open_ok(Box::into_raw(Box::new(engine)) as usize as u64),
        Err(error) => codec::open_error(&error.to_string()),
    };
    unsafe { byte_array(env, &buffer) }
}

/// Free the engine. Zero is allowed.
///
/// # Safety
/// `handle` must be zero or a handle from `open` not yet closed, and no other call may be using
/// it; the Kotlin side serialises open and close.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_nz_ursa_onymdroid_core_NativeEngine_close(
    _env: JniEnv,
    _this: JObject,
    handle: i64,
) {
    if handle != 0 {
        drop(unsafe { Box::from_raw(handle as *mut Engine) });
    }
}

/// Look a word up; null when the word is simply not in WordNet.
///
/// # Safety
/// Called only by the JVM; `handle` as for `close`, but live.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_nz_ursa_onymdroid_core_NativeEngine_lookup(
    env: JniEnv,
    _this: JObject,
    handle: i64,
    word: JString,
) -> JByteArray {
    let Some(engine) = (unsafe { engine_ref(handle) }) else {
        return std::ptr::null_mut();
    };
    let word = unsafe { jstring_to_string(env, word) };
    match engine.lookup(&word) {
        Some(entry) => unsafe { byte_array(env, &codec::entry(&entry)) },
        None => std::ptr::null_mut(),
    }
}

/// Headwords beginning with a prefix, capped (0 means no cap), as a string list buffer.
///
/// # Safety
/// Called only by the JVM; `handle` as for `lookup`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_nz_ursa_onymdroid_core_NativeEngine_complete(
    env: JniEnv,
    _this: JObject,
    handle: i64,
    prefix: JString,
    max: i32,
) -> JByteArray {
    let answer = match unsafe { engine_ref(handle) } {
        Some(engine) => {
            let prefix = unsafe { jstring_to_string(env, prefix) };
            engine.complete(&prefix, max.max(0) as usize)
        }
        None => Vec::new(),
    };
    unsafe { byte_array(env, &codec::string_list(&answer)) }
}

/// Spelling suggestions for a missed word, capped (0 means no cap), as a string list buffer.
///
/// # Safety
/// Called only by the JVM; `handle` as for `lookup`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_nz_ursa_onymdroid_core_NativeEngine_suggest(
    env: JniEnv,
    _this: JObject,
    handle: i64,
    word: JString,
    max: i32,
) -> JByteArray {
    let answer = match unsafe { engine_ref(handle) } {
        Some(engine) => {
            let word = unsafe { jstring_to_string(env, word) };
            engine.suggest(&word, max.max(0) as usize)
        }
        None => Vec::new(),
    };
    unsafe { byte_array(env, &codec::string_list(&answer)) }
}

/// The `--dump` rendering of a word's entry, as raw UTF-8. The parity suite diffs this against
/// the conformance fixtures, so the whole JNI path answers to the kit.
///
/// # Safety
/// Called only by the JVM; `handle` as for `lookup`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_nz_ursa_onymdroid_core_NativeEngine_dump(
    env: JniEnv,
    _this: JObject,
    handle: i64,
    word: JString,
) -> JByteArray {
    let text = match unsafe { engine_ref(handle) } {
        Some(engine) => {
            let word = unsafe { jstring_to_string(env, word) };
            engine.dump(&word)
        }
        None => String::new(),
    };
    unsafe { byte_array(env, text.as_bytes()) }
}

/// How many headwords the lemma index holds.
///
/// # Safety
/// Called only by the JVM; `handle` as for `lookup`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_nz_ursa_onymdroid_core_NativeEngine_lemmaCount(
    _env: JniEnv,
    _this: JObject,
    handle: i64,
) -> i64 {
    match unsafe { engine_ref(handle) } {
        Some(engine) => engine.lemma_count() as i64,
        None => 0,
    }
}

/// The headword at an index in sorted order, as raw UTF-8; null out of range.
///
/// # Safety
/// Called only by the JVM; `handle` as for `lookup`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_nz_ursa_onymdroid_core_NativeEngine_lemmaAt(
    env: JniEnv,
    _this: JObject,
    handle: i64,
    index: i64,
) -> JByteArray {
    let Some(engine) = (unsafe { engine_ref(handle) }) else {
        return std::ptr::null_mut();
    };
    if index < 0 {
        return std::ptr::null_mut();
    }
    match engine.lemma_at(index as usize) {
        Some(lemma) => unsafe { byte_array(env, lemma.as_bytes()) },
        None => std::ptr::null_mut(),
    }
}
