//! Regression test for the repo-root `samples/` fixture set: every
//! `samples/<plugin>/` entry must be recognised, end to end through
//! [`service::view_file`], by that same plugin - and by no other. Unlike
//! each plugin's own unit tests (which only exercise that plugin's `sniff`
//! in isolation), this walks the full priority-ordered `CORE_PLUGINS` list a
//! real file goes through, catching a plugin registered in the wrong order
//! relative to a sibling with an overlapping marker.

use protocol::Response;
use service::view_file;
use std::path::{Path, PathBuf};

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
fn every_sample_is_recognised_by_its_own_plugin() {
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
        let expected_plugin = plugin_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap();

        // The `directory` plugin has no file of its own to sniff: the
        // sample directory itself is the "file" it recognises.
        if expected_plugin == "directory" {
            check_recognised_as(&plugin_dir, expected_plugin, &mut failures);
            continue;
        }

        let files: Vec<PathBuf> = sorted_dir_entries(&plugin_dir)
            .into_iter()
            .filter(|path| path.is_file())
            .collect();
        if files.is_empty() {
            failures.push(format!("samples/{expected_plugin} has no sample file"));
            continue;
        }
        for file in files {
            check_recognised_as(&file, expected_plugin, &mut failures);
        }
    }

    assert!(
        failures.is_empty(),
        "{} sample(s) misrecognised:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn check_recognised_as(path: &Path, expected_plugin: &str, failures: &mut Vec<String>) {
    match view_file(path) {
        Ok(Response::FileView { plugin, .. }) if plugin == expected_plugin => {}
        Ok(Response::FileView { plugin, .. }) => failures.push(format!(
            "{} was recognised by the {plugin:?} plugin, not {expected_plugin:?}",
            path.display()
        )),
        Ok(Response::Error { message }) => failures.push(format!(
            "{} was not recognised by any plugin: {message}",
            path.display()
        )),
        Ok(other) => failures.push(format!("{}: unexpected response {other:?}", path.display())),
        Err(err) => failures.push(format!("could not view {}: {err}", path.display())),
    }
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
