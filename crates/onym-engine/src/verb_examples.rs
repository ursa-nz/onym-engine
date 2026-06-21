// SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
// SPDX-License-Identifier: GPL-3.0-or-later

//! WordNet's generic verb example sentences. A verb sense maps (in `sentidx.vrb`, keyed by sense
//! key) to one or more frame numbers, and each frame number names a template (in `sents.vrb`)
//! with a single `%s` placeholder for the verb. This reproduces Onym's `find_example` /
//! `get_example`: the templates are filled with the verb and returned in the same order Onym
//! emits them (it prepends as it reads, so the file order is reversed). Transcribed from the
//! Kotlin reference (`VerbExampleIndex.kt`), per `spec/engine.md` section 9.

use crate::textforms::decode;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub(crate) struct VerbExampleIndex {
    templates: HashMap<String, String>,
    frames: HashMap<String, Vec<String>>,
}

impl VerbExampleIndex {
    /// Load the verb example tables from `data_dir`; both tables are empty when the files are
    /// absent, which means no generic examples, never an error.
    pub(crate) fn load(data_dir: &Path) -> std::io::Result<VerbExampleIndex> {
        let mut templates = HashMap::new();
        for_each_line(&data_dir.join("sents.vrb"), |line| {
            if let Some(space) = line.find(' ')
                && space > 0
            {
                templates.insert(
                    line[..space].to_string(),
                    line[space + 1..].trim().to_string(),
                );
            }
        })?;
        let mut frames = HashMap::new();
        for_each_line(&data_dir.join("sentidx.vrb"), |line| {
            if let Some(space) = line.find(' ')
                && space > 0
            {
                let numbers: Vec<String> = line[space + 1..]
                    .split([' ', ','])
                    .filter(|n| !n.trim().is_empty())
                    .map(|n| n.to_string())
                    .collect();
                if !numbers.is_empty() {
                    frames.insert(line[..space].to_string(), numbers);
                }
            }
        })?;
        Ok(VerbExampleIndex { templates, frames })
    }

    /// The example sentences for `sense_key` (e.g. `cow%2:37:00::`), with `display_word`
    /// substituted in.
    pub(crate) fn sentences(&self, sense_key: &str, display_word: &str) -> Vec<String> {
        let Some(numbers) = self.frames.get(sense_key) else {
            return Vec::new();
        };
        let mut sentences: Vec<String> = numbers
            .iter()
            .filter_map(|number| self.templates.get(number))
            .map(|template| template.replacen("%s", display_word, 1))
            .collect();
        sentences.reverse();
        sentences
    }
}

// WordNet data files are UTF-8; decode them as such.
fn for_each_line(path: &Path, mut action: impl FnMut(&str)) -> std::io::Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    let text = decode(&fs::read(path)?);
    for line in text.lines() {
        if !line.trim().is_empty() {
            action(line);
        }
    }
    Ok(())
}
