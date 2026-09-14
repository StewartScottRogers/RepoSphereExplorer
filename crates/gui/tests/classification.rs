//! Holds every plugin's classifier to the contract, across every fixture
//! in the repository.
//!
//! The contract on `plugin_api::Span` is that a classifier's spans come
//! back in order and cover the text exactly once, with no gap and no
//! overlap. The pane draws the text by walking them and nothing else, so
//! a classifier that breaks it does not render badly - it slices a string
//! at a byte that is not a character boundary and takes the pane with it.
//!
//! Checked here rather than per plugin because the rule is the same for
//! all hundred and eighty, and because the fixture set is the only place
//! that holds a real file of every format.

use plugin_api::{Class, PluginPresentation, Span};
use protocol::Response;
use std::path::{Path, PathBuf};

/// The twelve described by the work order that added the tokeniser. Each
/// has to colour its own fixture, or the description is not doing
/// anything and nobody would notice.
const DESCRIBED: &[&str] = &[
    "rust",
    "python",
    "javascript",
    "typescript",
    "c",
    "cpp",
    "java",
    "csharp",
    "go",
    "shell",
    "json",
    "yaml",
];

fn samples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found
}

fn presentation(name: &str) -> Option<&'static dyn PluginPresentation> {
    gui::PRESENTATION_PLUGINS
        .iter()
        .copied()
        .find(|candidate| candidate.name() == name)
}

/// The plugin that opens `path`, and the text it carries.
fn opened(path: &Path) -> Option<(&'static dyn PluginPresentation, String)> {
    let Ok(Response::FileView { plugin, data, .. }) = service::view_file(path) else {
        return None;
    };
    let text = data
        .get("content")
        .and_then(serde_json::Value::as_str)?
        .to_owned();
    presentation(&plugin).map(|found| (found, text))
}

#[test]
fn every_classifier_covers_its_text_exactly_once() {
    let mut broken = Vec::new();
    for file in files_under(&samples_dir()) {
        let Some((plugin, text)) = opened(&file) else {
            continue;
        };
        let spans = plugin.classify(&text);
        if spans.is_empty() {
            continue;
        }
        if !syntax::spans_cover(&text, &spans) {
            broken.push(format!(
                "{} ({}): {} span(s) do not cover {} bytes",
                file.strip_prefix(samples_dir())
                    .unwrap_or(&file)
                    .to_string_lossy()
                    .replace('\\', "/"),
                plugin.name(),
                spans.len(),
                text.len()
            ));
        }
    }

    assert!(
        broken.is_empty(),
        "{} classifier(s) break the span contract:\n{}\n\nSpans come back \
         in order and cover the text exactly once. The pane slices the text \
         with them without checking.",
        broken.len(),
        broken.join("\n")
    );
}

#[test]
fn no_span_ever_splits_a_character() {
    // The failure this prevents is not a wrong colour, it is a panic:
    // slicing a string at a byte inside a multi-byte character.
    for file in files_under(&samples_dir()) {
        let Some((plugin, text)) = opened(&file) else {
            continue;
        };
        for Span { start, len, .. } in plugin.classify(&text) {
            assert!(
                text.is_char_boundary(start) && text.is_char_boundary(start + len),
                "{} ({}) returned a span at {start}..{} that splits a character",
                file.display(),
                plugin.name(),
                start + len
            );
        }
    }
}

#[test]
fn each_described_language_colours_its_own_fixture() {
    let mut silent = Vec::new();
    for name in DESCRIBED {
        let directory = samples_dir().join(name);
        let coloured = files_under(&directory).into_iter().any(|file| {
            opened(&file).is_some_and(|(plugin, text)| {
                plugin.name() == *name
                    && plugin
                        .classify(&text)
                        .iter()
                        .any(|span| span.class != Class::Plain)
            })
        });
        if !coloured {
            silent.push((*name).to_owned());
        }
    }

    assert!(
        silent.is_empty(),
        "{} described language(s) colour nothing in their own samples/ \
         directory, so the description is not reaching the file: {}",
        silent.len(),
        silent.join(", ")
    );
}

#[test]
fn a_plugin_with_no_description_says_nothing_rather_than_something_wrong() {
    // The default is empty, and empty has to mean "draw it as before"
    // rather than "one plain span", so a pane can tell the two apart.
    let undescribed = gui::PRESENTATION_PLUGINS
        .iter()
        .find(|plugin| plugin.classify("let x = 1;").is_empty())
        .expect("not every plugin has been described yet");
    assert!(
        undescribed.classify("anything at all").is_empty(),
        "a plugin that describes no language returns no spans"
    );
}

/// Formats whose plugin describes no language, with the reason.
///
/// The line is whether the file has syntax at all. A `.csv` is values
/// with commas between them; a comma is a separator, not punctuation a
/// reader needs picked out. Prose is prose. Describing one of these
/// would not colour it - it would put a colour on a comma and call that
/// a language.
const NO_SYNTAX: &[(&str, &str)] = &[
    (
        "text",
        "the general reader: a plain text file has no syntax, which is what makes it plain",
    ),
    (
        "csv",
        "values with separators between them; the separator is not syntax",
    ),
    (
        "markdown",
        "prose. Its emphasis markers are the same characters as an apostrophe and a multiplication sign, and a tokeniser reading them as delimiters would colour the rest of every contraction",
    ),
    (
        "diff",
        "the plugin draws its own added and removed lines, which is the only colouring a diff wants",
    ),
    (
        "tar",
        "an archive. The view lists what is inside it, and that listing is the plugin's own words rather than the file's",
    ),
];

/// Every plugin that opens a text file either describes its language or
/// is in [`NO_SYNTAX`] with the reason it does not.
///
/// Without this, the hundred and eighteen descriptions are a batch of
/// work that happened once. A plugin added next month would colour
/// nothing, and the only way to notice would be to open one of its files
/// and look.
#[test]
fn every_text_format_either_describes_its_language_or_says_why_not() {
    let mut silent = Vec::new();
    let mut directories: Vec<std::path::PathBuf> = std::fs::read_dir(samples_dir())
        .expect("samples/ is there")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    directories.sort();

    for directory in directories {
        let name = directory.file_name().unwrap().to_string_lossy().to_string();
        if NO_SYNTAX.iter().any(|(listed, _)| *listed == name) {
            continue;
        }
        // Its own files, opened by it, that carry text. A plugin whose
        // subject is not text has nothing to describe.
        let mut describes = false;
        let mut has_text = false;
        for file in files_under(&directory) {
            let Some((plugin, text)) = opened(&file) else {
                continue;
            };
            if plugin.name() != name {
                continue;
            }
            has_text = true;
            if plugin
                .classify(&text)
                .iter()
                .any(|span| span.class != Class::Plain)
            {
                describes = true;
                break;
            }
        }
        if has_text && !describes {
            silent.push(name);
        }
    }

    assert!(
        silent.is_empty(),
        "{} plugin(s) open a text file and colour nothing in it:\n{}\n\nGive \
         the format a `Language` description, or - if it has no syntax to \
         colour - add it to NO_SYNTAX with the reason.",
        silent.len(),
        silent.join("\n")
    );
}

/// An entry left behind after a plugin was renamed would quietly excuse
/// a format that is no longer there, and go on excusing it.
#[test]
fn every_no_syntax_entry_names_a_plugin_that_exists() {
    let missing: Vec<&str> = NO_SYNTAX
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| !samples_dir().join(name).is_dir())
        .collect();

    assert!(
        missing.is_empty(),
        "NO_SYNTAX names formats that are not in the set: {missing:?}"
    );
}

/// And each has to say why, because the list is the argument.
#[test]
fn every_no_syntax_entry_gives_a_reason() {
    let mute: Vec<&str> = NO_SYNTAX
        .iter()
        .filter(|(_, reason)| reason.len() < 20)
        .map(|(name, _)| *name)
        .collect();

    assert!(
        mute.is_empty(),
        "an exception without a reason is just a plugin nobody got to: {mute:?}"
    );
}
