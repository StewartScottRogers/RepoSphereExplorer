//! Holds every plugin's classifier to the contract, across every fixture in
//! the repository - the terminal front end's own copy of the graphical
//! front end's `crates/gui/tests/classification.rs`.
//!
//! The contract on `plugin_api::Span` is that a classifier's spans come
//! back in order and cover the text exactly once, with no gap and no
//! overlap. The File pane draws the text by walking them and nothing
//! else, so a classifier that breaks it does not render badly - it slices
//! a string at a byte that is not a character boundary and takes the pane
//! with it.
//!
//! Checked here rather than per plugin because the rule is the same for
//! all of them, and because the fixture set is the only place that holds
//! a real file of every format.

use plugin_api::{PluginPresentation, Span};
use protocol::Response;
use std::path::{Path, PathBuf};

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
    tui::PRESENTATION_PLUGINS
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
         in order and cover the text exactly once. The File pane slices \
         the text with them without checking.",
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
