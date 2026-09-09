//! Checks that every fixture under `samples/` still exercises the plugin
//! that owns it.
//!
//! `samples.rs` next door checks that each fixture is *recognised* by the
//! right plugin. That is a low bar: a hello-world clears it while leaving
//! most of what the plugin extracts untouched. Before this test the whole
//! set was hello-worlds, and the consequences were not theoretical - a C
//! file with a `struct` was opening as Rust, every Java file as Perl, and
//! a modern TypeScript module reported no classes, no interfaces and no
//! functions. None of it showed, because nothing in `samples/` was big
//! enough to ask.
//!
//! So: every list a plugin puts on the wire must have something in it, and
//! every optional field must be filled, unless the format genuinely cannot
//! supply one - and those exceptions are named here, one line each, rather
//! than being silently tolerated.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Sample directories belonging to a plugin whose subject is the folder
/// rather than a file in it. The files inside them are ordinary files that
/// other plugins own, so the per-file checks skip these directories and
/// ask about the folder instead.
const FOLDER_PLUGIN_SAMPLES: &[&str] = &["directory", "project-cargo"];

/// Fields no fixture anywhere fills, because the format itself has no way
/// to carry them. Each entry is `("<plugin>", "<field>")`, and every one of
/// them is a statement about the format, not about a fixture.
///
/// Named per plugin rather than per file because the question is whether
/// *anything* proves the plugin can fill the field. `model3d` used to need
/// five entries - OBJ has no generator, glTF no vertex count - and needs
/// none now that its two fixtures are asked together.
const ALLOWED_GAPS: &[(&str, &str)] = &[
    // The AVI has a video track and no audio track, which is what a silent
    // screen recording looks like.
    ("video", "audio_codec"),
    // A poster is an *attachment*, which AVI has no box for. Pulling a
    // frame out of the stream would need a decoder.
    ("video", "poster"),
    // A shared object has no entry point; the loader calls into it.
    ("executable", "entry"),
];

/// The repo's `samples/` directory.
fn samples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// Every file under `dir`, at any depth, sorted.
///
/// Recursive, because a language sample is a project: its source sits in
/// `src/`, `lib/` or `Sources/`, and a walk that stopped at the top level
/// would stop proving anything about the plugin that owns it the moment
/// the fixture moved where its ecosystem puts it.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("a sample directory should be readable")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    entries.sort();

    let mut found = Vec::new();
    for entry in entries {
        if entry.is_dir() {
            found.extend(files_under(&entry));
        } else {
            found.push(entry);
        }
    }
    found
}

/// Every file under `samples/`, with the plugin directory it sits in.
fn sample_files() -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(samples_dir())
        .expect("samples/ should be readable")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    for dir in dirs {
        let plugin = dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        // A folder plugin's fixture is the directory itself; the files
        // inside it are ordinary files belonging to other plugins.
        if FOLDER_PLUGIN_SAMPLES.contains(&plugin.as_str()) {
            continue;
        }
        for file in files_under(&dir) {
            found.push((plugin.clone(), file));
        }
    }
    found
}

/// `<plugin>/<path within the sample directory>`, as `ALLOWED_GAPS`
/// spells it.
///
/// A file at the top level keys as `<plugin>/<name>`, exactly as it did
/// before the walk went recursive, so every existing exception still names
/// the file it was written for.
fn key(plugin: &str, file: &Path) -> String {
    let root = samples_dir().join(plugin);
    let within = file.strip_prefix(&root).unwrap_or(file);
    format!("{plugin}/{}", within.to_string_lossy().replace('\\', "/"))
}

/// Whether `value` counts as filled: present, and not an empty list.
fn is_filled(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Array(items) => !items.is_empty(),
        _ => true,
    }
}

#[test]
fn every_sample_directory_fills_every_field_its_plugin_extracts() {
    // Asked per plugin, not per file, and grouped by the plugin that
    // actually recognised each file rather than by the directory it sits
    // in. A language sample is a project now, so `samples/go/` holds a
    // Makefile and a README as well as Go, and an ordinary source file in
    // any project does not declare a struct and an interface and a class
    // each - `main.go` parses arguments and calls the package. Requiring
    // every file to fill every field would mean hundreds of exceptions
    // that say nothing about any format, burying the three that say
    // something about one.
    //
    // What this still catches is what it was written for: a plugin with a
    // field that no fixture anywhere ever fills, so nobody would notice if
    // the plugin stopped filling it. Which fixture proves it is a question
    // for `samples.rs`, whose second rule keeps each directory testing its
    // own plugin.
    let mut seen: BTreeMap<String, BTreeMap<String, bool>> = BTreeMap::new();
    let mut failures = Vec::new();

    for (plugin, file) in sample_files() {
        let name = key(&plugin, &file);
        let (recognised_by, data) = match service::view_file(&file) {
            Ok(protocol::Response::FileView { plugin, data, .. }) => (plugin, data),
            other => {
                failures.push(format!("{name}: did not view as a file ({other:?})"));
                continue;
            }
        };
        let Some(object) = data.as_object() else {
            failures.push(format!("{name}: view data is not an object"));
            continue;
        };

        let fields = seen.entry(recognised_by).or_default();
        for (field, value) in object {
            let filled = fields.entry(field.clone()).or_default();
            *filled = *filled || is_filled(value);
        }
    }

    for (plugin, fields) in seen {
        for (field, filled) in fields {
            if filled {
                continue;
            }
            if !ALLOWED_GAPS
                .iter()
                .any(|(owner, gap)| *owner == plugin && *gap == field)
            {
                failures.push(format!(
                    "the {plugin:?} plugin: no fixture fills `{field}`, so nothing proves it can"
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} field(s) prove nothing:\n{}\n\nEither enrich a fixture in that directory, or - \
         if the format cannot carry the field at all - say so in ALLOWED_GAPS.",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn a_text_fixture_is_long_enough_to_truncate() {
    // Truncation decides whether a file can be edited and how the plain
    // text view renders, and until `samples/text/access.log` there was no
    // fixture in the set long enough to reach a plugin's read cap.
    let truncated: Vec<String> = sample_files()
        .into_iter()
        .filter_map(|(plugin, file)| match service::view_file(&file) {
            Ok(protocol::Response::FileView { data, .. })
                if data
                    .get("truncated")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false) =>
            {
                Some(key(&plugin, &file))
            }
            _ => None,
        })
        .collect();

    assert!(
        !truncated.is_empty(),
        "no fixture exceeds a plugin's read cap, so nothing exercises `truncated`"
    );
}

#[test]
fn every_allowed_gap_names_a_plugin_with_fixtures() {
    // An exception left behind after a plugin was renamed would quietly
    // excuse a field on something that no longer exists.
    let directories: BTreeSet<String> = sample_files()
        .into_iter()
        .filter_map(|(_dir, file)| match service::view_file(&file) {
            Ok(protocol::Response::FileView { plugin, .. }) => Some(plugin),
            _ => None,
        })
        .collect();

    let orphans: Vec<&str> = ALLOWED_GAPS
        .iter()
        .map(|(sample, _field)| *sample)
        .filter(|sample| !directories.contains(*sample))
        .collect();

    assert!(
        orphans.is_empty(),
        "ALLOWED_GAPS names plugins with no fixtures at all: {orphans:?}"
    );
}
