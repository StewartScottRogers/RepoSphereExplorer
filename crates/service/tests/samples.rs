//! Regression tests for the repo-root `samples/` fixture set, run end to
//! end through [`service::view_file`]. Unlike each plugin's own unit tests,
//! which exercise that plugin's `sniff` in isolation, this walks the full
//! priority-ordered `CORE_PLUGINS` list a real file goes through, catching
//! a plugin registered in the wrong order relative to a sibling with an
//! overlapping marker.
//!
//! Two rules, which together are what the single old rule was for.
//!
//! The old rule was that every file in `samples/<plugin>/` is recognised by
//! `<plugin>`. It caught real defects - a C file with a top-level `struct`
//! opening as Rust, every Java file opening as Perl (#272) - and it also
//! made a language directory unable to hold anything but that language. A
//! Rust *project* has a `Cargo.toml` in it, and a `Cargo.toml` is not Rust.
//!
//! So:
//!
//! 1. **No orphans.** Every file anywhere under `samples/` is recognised by
//!    *some* plugin. Stronger than the old rule, which only ever asked
//!    within one directory and never descended into a subdirectory.
//! 2. **Every directory proves its own plugin.** At least one file in
//!    `samples/<plugin>/` is recognised by `<plugin>`. This is what keeps
//!    the misattribution check alive: if `ringbuffer.c` started opening as
//!    Rust, `samples/c/` would hold no C file at all and would fail.

use protocol::Response;
use service::view_file;
use std::path::{Path, PathBuf};

/// Sample directories belonging to a plugin whose subject is the folder
/// rather than a file in it. The folder itself is what proves the plugin,
/// so rule 2 asks about the directory rather than looking for a file.
const FOLDER_PLUGIN_SAMPLES: &[&str] = &["directory", "project-cargo"];

/// Which plugin recognises `path`, or `None` when nothing does.
///
/// A folder answers as the directory plugin and carries every folder
/// plugin that also recognises it, because a folder is several things at
/// once. All of those names count.
fn recognised_by(path: &Path) -> Vec<String> {
    match view_file(path) {
        Ok(Response::FileView { plugin, also, .. }) => std::iter::once(plugin)
            .chain(also.into_iter().map(|view| view.plugin))
            .collect(),
        _ => Vec::new(),
    }
}

/// Every file under `dir`, at any depth, sorted.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in sorted_dir_entries(dir) {
        if entry.is_dir() {
            found.extend(files_under(&entry));
        } else {
            found.push(entry);
        }
    }
    found
}

fn samples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

fn sorted_dir_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("could not read {}: {err}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    entries
}

#[test]
fn every_sample_file_is_recognised_by_some_plugin() {
    // Rule 1. An unrecognised file in the set is a hole: nothing renders
    // it, and nobody notices until somebody opens it.
    let mut failures = Vec::new();

    for plugin_dir in sorted_dir_entries(&samples_dir())
        .into_iter()
        .filter(|path| path.is_dir())
    {
        for file in files_under(&plugin_dir) {
            if recognised_by(&file).is_empty() {
                failures.push(format!("{} is recognised by no plugin", file.display()));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} sample file(s) nothing can open:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_sample_directory_proves_its_own_plugin() {
    // Rule 2, and the one that catches misattribution. A directory whose
    // own plugin claims nothing in it has stopped testing that plugin,
    // whatever else it holds.
    let plugin_dirs: Vec<PathBuf> = sorted_dir_entries(&samples_dir())
        .into_iter()
        .filter(|path| path.is_dir())
        .collect();
    assert!(
        !plugin_dirs.is_empty(),
        "samples/ should contain at least one plugin's fixtures"
    );

    let mut failures = Vec::new();
    for plugin_dir in plugin_dirs {
        let plugin = plugin_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap()
            .to_owned();

        // A folder plugin has no file of its own: the directory itself is
        // what it recognises.
        if FOLDER_PLUGIN_SAMPLES.contains(&plugin.as_str()) {
            if !recognised_by(&plugin_dir).contains(&plugin) {
                failures.push(format!(
                    "samples/{plugin} is not recognised by the {plugin:?} folder plugin"
                ));
            }
            continue;
        }

        let files = files_under(&plugin_dir);
        if files.is_empty() {
            failures.push(format!("samples/{plugin} has no sample file"));
            continue;
        }
        if !files
            .iter()
            .any(|file| recognised_by(file).contains(&plugin))
        {
            failures.push(format!(
                "nothing in samples/{plugin} is recognised by the {plugin:?} plugin, so the                  directory no longer tests it"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} sample directory/directories prove nothing:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn samples_has_one_subdirectory_per_plugin_crate() {
    let plugins_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins");
    let mut plugin_names: Vec<String> = sorted_dir_entries(&plugins_dir)
        .into_iter()
        .filter(|path| path.is_dir())
        .map(|path| path.file_name().unwrap().to_str().unwrap().to_owned())
        .collect();
    plugin_names.sort();

    let mut sample_names: Vec<String> = sorted_dir_entries(&samples_dir())
        .into_iter()
        .filter(|path| path.is_dir())
        .map(|path| path.file_name().unwrap().to_str().unwrap().to_owned())
        .collect();
    sample_names.sort();

    assert_eq!(
        sample_names, plugin_names,
        "samples/ must have exactly one subdirectory per crates/plugins/* entry"
    );
}
