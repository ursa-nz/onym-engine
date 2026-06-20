// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared test fixture: the WordNet base directory, materialised from the `onym-data` submodule.
//! The engine reads its data over an explicit path, so the tests pin that path to the data set the
//! whole project ships, not to whatever WordNet a host happens to have installed. The base is laid
//! down overlay-free, the way the conformance fixtures are generated.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The prepared WordNet base, built once per test binary from the submodule. Returns `None` when the
/// submodule is not checked out, so the tests skip instead of failing on a bare clone.
pub fn wordnet_base() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(prepare).as_deref()
}

fn prepare() -> Option<PathBuf> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../onym-data/prepare.sh");
    if !script.is_file() {
        eprintln!("skipping: onym-data submodule not checked out (git submodule update --init)");
        return None;
    }
    // CARGO_TARGET_TMPDIR is per test target and inside target/, so the base is laid down once and
    // reused across runs. prepare.sh only writes, never fetches.
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("wordnet-base");
    if !target.join("index.noun").is_file() {
        let status = std::process::Command::new(&script)
            .arg("--base-only")
            .arg(&target)
            .status();
        match status {
            Ok(code) if code.success() => {}
            _ => {
                eprintln!("skipping: onym-data prepare.sh did not produce a base at {target:?}");
                return None;
            }
        }
    }
    Some(target)
}
