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

/// Every fixture has to be in the repository, not merely on this machine.
///
/// A `.gitignore` rule matching a fixture is silent: the tests here walk
/// the working tree, so they pass locally while the factory sees a
/// directory that is short of files, or missing altogether. It happened
/// twice at once - an unanchored `data/` at the root swallowed
/// `samples/purescript/src/Data/`, and an unanchored `worker` inside
/// `samples/go/.gitignore`, meant for the built binary, swallowed
/// `samples/go/cmd/worker/`. Neither said anything.
#[test]
fn every_sample_file_is_in_the_repository() {
    let output = std::process::Command::new("git")
        .args([
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "samples",
        ])
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .output()
        .expect("git is how the factory checks this repository out");
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let ignored: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();

    assert!(
        ignored.is_empty(),
        "{} fixture(s) under samples/ are ignored by git, so they exist here \
         and nowhere else:\n{}\n\nAnchor the rule that matches them - a bare \
         `name/` matches at every depth, `/name/` only at the root.",
        ignored.len(),
        ignored.join("\n")
    );
}

/// Files that really are nothing but text, so falling through to the
/// general reader is the right answer rather than a plugin missing its
/// own format. One line each, with the reason.
///
/// The line between the two is whether the file is the *language* of
/// the directory it sits in. A `Directory.Build.props` is `MSBuild` and a
/// `plugins.sbt` is Scala, so their plugins were taught to see them. A
/// `dune` file is a different language that merely lives beside OCaml,
/// and claiming it would need a plugin of its own.
const PLAINLY_TEXT: &[&str] = &[
    // The folder plugin's fixture. Its subject is the directory, and
    // these are the ordinary files a directory is expected to hold.
    "directory/notes.txt",
    "directory/shopping-list.txt",
    // Two bytes. A marker whose presence is the whole message (PEP 561);
    // there is nothing in it to read.
    "python/src/taskqueue/py.typed",
    // A bare list of file paths, with no Perl in it.
    "perl/MANIFEST",
    // A bare list of `prefix=path` remappings, with no Solidity in it.
    "solidity/remappings.txt",
    // A bare list of names, with no `!`, `/` or `*` in it. `ignorefile`
    // asks for a glob-shaped pattern on purpose: without that it would
    // claim `perl/MANIFEST` above, which is also a list of names and is
    // not an ignore file.
    "dockerfile/.gitignore",
    // S-expressions, and a language of its own rather than OCaml. It
    // would need its own plugin, not a looser OCaml sniff.
    "ocaml/bin/dune",
    // A package manifest in R's own small vocabulary - `export()`,
    // `importFrom()` - and not R code. The R plugin reads R code
    // structure, of which this has none, so claiming it would relabel
    // the file without saying anything more about it.
    "r/NAMESPACE",
];

/// Rule 3. A file in `samples/<plugin>/` that goes to the general text
/// reader is one that plugin cannot read, and rule 2 cannot see it: one
/// file per directory is enough to satisfy that, so the rest can rot.
///
/// It found five real ones the first time it ran - a `.hs` and an `.exs`
/// whose plugins own those extensions and could not recognise a
/// one-line instance of them, two `Directory.Build.props`, and a
/// `plugins.sbt` - all the same shape: a sniff too strict for a small
/// but entirely ordinary file of its own format.
#[test]
fn no_file_falls_through_to_the_general_text_reader() {
    let mut failures = Vec::new();
    for plugin_dir in sorted_dir_entries(&samples_dir())
        .into_iter()
        .filter(|path| path.is_dir())
    {
        let owner = plugin_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        if owner == "text" {
            continue;
        }
        for file in files_under(&plugin_dir) {
            let relative = file
                .strip_prefix(samples_dir())
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            if relative.ends_with("README.md") || PLAINLY_TEXT.contains(&relative.as_str()) {
                continue;
            }
            if recognised_by(&file) == vec!["text".to_owned()] {
                failures.push(relative);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} file(s) fell through to the general text reader:\n{}\n\nEither teach \
         the plugin that owns the directory to recognise its own format, or - if \
         the file really is nothing but text - say so in PLAINLY_TEXT with the \
         reason.",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_plainly_text_entry_names_a_file_that_exists() {
    // An exception left behind after a fixture was renamed would quietly
    // excuse a file that is no longer there, and go on excusing it.
    let missing: Vec<&str> = PLAINLY_TEXT
        .iter()
        .copied()
        .filter(|relative| !samples_dir().join(relative).exists())
        .collect();

    assert!(
        missing.is_empty(),
        "PLAINLY_TEXT names files that are not in the set: {missing:?}"
    );
}
