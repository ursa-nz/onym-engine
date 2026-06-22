// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The conformance dumper: a command-line face over the engine that emits the stable text format
//! of `spec/dump-format.md`, byte for byte. The conformance kit drives it one word at a time, and
//! `--batch` streams a word list through one engine for the total cross-diff against the other
//! bindings.
//!
//! Arguments and output are UTF-8, the encoding of the OEWN database and of every string the engine
//! produces, and how the fixtures are compared.

use onym_engine::Engine;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

const COMPLETE_CAP: usize = 20;
const SUGGEST_CAP: usize = 10;

fn main() -> ExitCode {
    let mut args: Vec<OsString> = std::env::args_os().skip(1).collect();

    // The data directory is explicit: the engine reads no environment and assumes no system install.
    // The conformance kit prepares it from the onym-data submodule and passes it here.
    let data_dir = if args.first().is_some_and(|a| a == "--data") {
        if args.len() < 2 {
            return usage();
        }
        let dir = PathBuf::from(args[1].clone());
        args.drain(..2);
        dir
    } else {
        return usage();
    };

    enum Mode {
        Dump(String),
        Complete(String),
        Suggest(String),
        Batch,
    }

    let mode = match args.as_slice() {
        [flag, word] if flag == "--dump" => Mode::Dump(os_to_string(word)),
        [flag, prefix] if flag == "--complete" => Mode::Complete(os_to_string(prefix)),
        [flag, word] if flag == "--suggest" => Mode::Suggest(os_to_string(word)),
        [flag] if flag == "--batch" => Mode::Batch,
        // A bare word dumps, exactly as onym-cli treats it; anything flag-shaped is an error.
        [word] if !os_to_string(word).starts_with("--") => Mode::Dump(os_to_string(word)),
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
        Mode::Dump(word) => out.write_all(engine.dump(&word).as_bytes()),
        Mode::Complete(prefix) => write_lines(&mut out, &engine.complete(&prefix, COMPLETE_CAP)),
        Mode::Suggest(word) => write_lines(&mut out, &engine.suggest(&word, SUGGEST_CAP)),
        Mode::Batch => batch(&engine, &mut out),
    };
    if result.and_then(|()| out.flush()).is_err() {
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

/// Dump every word read from stdin (one per line, UTF-8), each preceded by a `==> word <==`
/// marker line so a cross-diff can name the word a difference belongs to. No dump line ever
/// starts with `=`, so the marker is unambiguous.
fn batch(engine: &Engine, out: &mut impl Write) -> std::io::Result<()> {
    let mut input = Vec::new();
    std::io::stdin().lock().read_to_end(&mut input)?;
    let text = String::from_utf8_lossy(&input);
    for word in text.lines() {
        if word.is_empty() {
            continue;
        }
        out.write_all(format!("==> {word} <==\n").as_bytes())?;
        out.write_all(engine.dump(word).as_bytes())?;
    }
    Ok(())
}

fn write_lines(out: &mut impl Write, lines: &[String]) -> std::io::Result<()> {
    for line in lines {
        out.write_all(line.as_bytes())?;
        out.write_all(b"\n")?;
    }
    Ok(())
}

fn usage() -> ExitCode {
    eprintln!("usage: onym-dump --data DIR [--dump WORD|--complete PREFIX|--suggest WORD|--batch]");
    ExitCode::from(2)
}

/// Read an argument as UTF-8, the encoding of the database and of every query the engine accepts.
/// A byte sequence that is not valid UTF-8 has its bad bytes replaced rather than rejected.
fn os_to_string(os: &OsString) -> String {
    os.to_string_lossy().into_owned()
}
