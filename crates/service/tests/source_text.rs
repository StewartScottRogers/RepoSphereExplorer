//! Every view of a text file carries that file's text.
//!
//! A `content` string in a view is what makes the front end offer the raw
//! Text tab and the editor. It used to depend on whether the plugin that
//! read the file had kept the source, and fifty-two of them do not: they
//! parse a text format thoroughly and throw the text away. A `.css`, a
//! `.zig`, a `CMakeLists.txt` could be read *about* but never read or
//! edited, so being well supported was what made a file uneditable.
//!
//! The guarantee wanted is about the file, not its format, so it is the
//! service that makes it - and this is where it is checked, across the
//! whole fixture set rather than on one example.

use protocol::Response;
use std::path::{Path, PathBuf};

/// The ceiling in `service`. A file past it carries no text, because an
/// editor that saved back the first 64KB of a file would lose the rest.
const MAX_SOURCE_BYTES: u64 = 64 * 1024;

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

/// Whether the plugin that read `path` says it stopped part way.
fn truncated(path: &Path) -> bool {
    match service::view_file(path) {
        Ok(Response::FileView { data, .. }) => data
            .get("truncated")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        _ => false,
    }
}

/// The text a view carries, if it carries any.
fn text_in_view(path: &Path) -> Option<String> {
    match service::view_file(path) {
        Ok(Response::FileView { data, .. }) => data
            .get("content")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        _ => None,
    }
}

#[test]
fn every_text_fixture_carries_its_text() {
    let mut missing = Vec::new();
    for file in files_under(&samples_dir()) {
        let Ok(bytes) = std::fs::read(&file) else {
            continue;
        };
        if bytes.len() as u64 > MAX_SOURCE_BYTES || std::str::from_utf8(&bytes).is_err() {
            continue;
        }
        if text_in_view(&file).is_none() {
            missing.push(
                file.strip_prefix(samples_dir())
                    .unwrap_or(&file)
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }

    assert!(
        missing.is_empty(),
        "{} text file(s) reach the front end with no text, so they get no \
         Text tab and no editor:\n{}",
        missing.len(),
        missing.join("\n")
    );
}

/// Whatever a view carries as `content` is what Save writes over the
/// file, so it has to be that file's own text and nothing else.
///
/// The zstd plugin used to have a field called `content` holding the word
/// `frames` - a classification of the archive, not its text. The front
/// end reads a `content` string as "this file can be edited", so a `.zst`
/// offered a Text tab reading `frames` and an Edit button that, on Save,
/// would have written that word over the compressed file. Nothing failed;
/// the name simply meant two different things in two places.
#[test]
fn whatever_a_view_calls_content_is_the_file_it_came_from() {
    let mut wrong = Vec::new();
    for file in files_under(&samples_dir()) {
        // A view that says it was cut short is meant to differ from the
        // file, and is already refused an editor for exactly that reason.
        if truncated(&file) {
            continue;
        }
        let Some(in_view) = text_in_view(&file) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&file) else {
            continue;
        };
        let relative = file
            .strip_prefix(samples_dir())
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        match String::from_utf8(bytes) {
            Ok(on_disk) if on_disk == in_view => {}
            Ok(_) => wrong.push(format!("{relative}: content is not what the file says")),
            Err(_) => wrong.push(format!(
                "{relative}: not a text file, and yet a view of it offers text to edit"
            )),
        }
    }

    assert!(
        wrong.is_empty(),
        "{} view(s) carry a `content` that Save would write over the file \
         wrongly:\n{}\n\nA view's `content` is the file's own text. A plugin \
         with something else to say should call the field something else.",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn a_file_past_the_ceiling_carries_none() {
    // 99KB of log. Reading it is fine; editing it is not, because saving
    // would write back only what was read.
    let big = samples_dir().join("text/access.log");
    assert!(
        std::fs::metadata(&big).expect("the fixture is there").len() > MAX_SOURCE_BYTES,
        "this fixture is the one that tests the ceiling, so it has to exceed it"
    );
    let view = service::view_file(&big).expect("it still opens");
    let Response::FileView { data, .. } = view else {
        panic!("a log file opens as a file view");
    };
    assert_eq!(
        data.get("truncated").and_then(serde_json::Value::as_bool),
        Some(true),
        "a file read only in part says so, and that is what closes the editor"
    );
}
