// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The conformance dumper: a command-line face over the engine that emits the stable text format
//! of `spec/dump-format.md`, byte for byte. The conformance kit drives it one word at a time, and
//! `--batch` streams a word list through one engine for the total cross-diff against the Kotlin
//! reference.
//!
//! Arguments and output are treated as ISO-8859-1 byte sequences, matching how the C oracle saw
//! them and how the fixtures are compared.

use onym_engine::Engine;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

const DEFAULT_DATA_DIR: &str = "/usr/share/wordnet";
const COMPLETE_CAP: usize = 20;
const SUGGEST_CAP: usize = 10;

fn main() -> ExitCode {
    let mut args: Vec<OsString> = std::env::args_os().skip(1).collect();

    let data_dir = if args.first().is_some_and(|a| a == "--data") {
        if args.len() < 2 {
            return usage();
        }
        let dir = PathBuf::from(args[1].clone());
        args.drain(..2);
        dir
    } else {
        PathBuf::from(DEFAULT_DATA_DIR)
    };

    enum Mode {
        Dump(String),
        Complete(String),
        Suggest(String),
        Batch,
    }

    let mode = match args.as_slice() {
        [flag, word] if flag == "--dump" => Mode::Dump(os_to_latin1(word)),
        [flag, prefix] if flag == "--complete" => Mode::Complete(os_to_latin1(prefix)),
        [flag, word] if flag == "--suggest" => Mode::Suggest(os_to_latin1(word)),
        [flag] if flag == "--batch" => Mode::Batch,
        // A bare word dumps, exactly as onym-cli treats it; anything flag-shaped is an error.
        [word] if !os_to_latin1(word).starts_with("--") => Mode::Dump(os_to_latin1(word)),
        _ => return usage(),
    };

    let engine = match Engine::open(&data_dir) {
        Ok(engine) => engine,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(1);
        }
    };

    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let result = match mode {
        Mode::Dump(word) => out.write_all(&latin1_bytes(&engine.dump(&word))),
        Mode::Complete(prefix) => write_lines(&mut out, &engine.complete(&prefix, COMPLETE_CAP)),
        Mode::Suggest(word) => write_lines(&mut out, &engine.suggest(&word, SUGGEST_CAP)),
        Mode::Batch => batch(&engine, &mut out),
    };
    if result.and_then(|()| out.flush()).is_err() {
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

/// Dump every word read from stdin (one per line, ISO-8859-1), each preceded by a `==> word <==`
/// marker line so a cross-diff can name the word a difference belongs to. No dump line ever
/// starts with `=`, so the marker is unambiguous.
fn batch(engine: &Engine, out: &mut impl Write) -> std::io::Result<()> {
    let mut input = Vec::new();
    std::io::stdin().lock().read_to_end(&mut input)?;
    let text: String = input.iter().map(|&b| b as char).collect();
    for word in text.lines() {
        if word.is_empty() {
            continue;
        }
        out.write_all(&latin1_bytes(&format!("==> {word} <==\n")))?;
        out.write_all(&latin1_bytes(&engine.dump(word)))?;
    }
    Ok(())
}

fn write_lines(out: &mut impl Write, lines: &[String]) -> std::io::Result<()> {
    for line in lines {
        out.write_all(&latin1_bytes(line))?;
        out.write_all(b"\n")?;
    }
    Ok(())
}

fn usage() -> ExitCode {
    eprintln!("usage: onym-dump [--data DIR] [--dump|--complete|--suggest|--batch] WORD");
    ExitCode::from(2)
}

/// Read an argument as ISO-8859-1 bytes, as the C oracle did, so a high-bit byte is one
/// character and round-trips into the dump unchanged.
fn os_to_latin1(os: &OsString) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        os.as_bytes().iter().map(|&b| b as char).collect()
    }
    #[cfg(not(unix))]
    {
        os.to_string_lossy().into_owned()
    }
}

/// Encode engine output back to ISO-8859-1 bytes. Engine strings only carry code points the
/// database supplied, so every character fits in one byte.
fn latin1_bytes(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| if (c as u32) <= 0xff { c as u8 } else { b'?' })
        .collect()
}
