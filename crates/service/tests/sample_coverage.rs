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

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Fields that a fixture legitimately leaves empty or absent, because the
/// format itself has no way to carry them. Each entry is
/// `("<plugin>/<file>", "<field>")`, and every one of them is a statement
/// about the format, not about the fixture.
const ALLOWED_GAPS: &[(&str, &str)] = &[
    // Wavefront OBJ carries geometry only: no generator, and no notion of a
    // mesh or a scene. `samples/model3d/scene.gltf` covers those fields.
    ("model3d/sphere.obj", "generator"),
    ("model3d/sphere.obj", "mesh_count"),
    ("model3d/sphere.obj", "scene_count"),
    // glTF is the other way round: `Model3dView` documents vertex and face
    // counts as OBJ and STL fields, which `samples/model3d/sphere.obj`
    // supplies.
    ("model3d/scene.gltf", "vertex_count"),
    ("model3d/scene.gltf", "face_count"),
    // The AVI has a video track and no audio track, which is what a silent
    // screen recording looks like.
    ("video/orbit.avi", "audio_codec"),
    // A poster is an *attachment*, which AVI has no box for. Pulling a
    // frame out of the stream would need a decoder.
    ("video/orbit.avi", "poster"),
    // A shared object has no entry point; the loader calls into it.
    ("executable/example.so", "entry"),
];

/// The repo's `samples/` directory.
fn samples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples")
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
        // The directory plugin's fixture is the directory itself; the files
        // inside it are ordinary files belonging to other plugins.
        if plugin == "directory" {
            continue;
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("a sample directory should be readable")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .collect();
        files.sort();
        for file in files {
            found.push((plugin.clone(), file));
        }
    }
    found
}

/// `<plugin>/<file name>`, as `ALLOWED_GAPS` spells it.
fn key(plugin: &str, file: &Path) -> String {
    format!(
        "{plugin}/{}",
        file.file_name().unwrap_or_default().to_string_lossy()
    )
}

#[test]
fn every_sample_fills_every_field_its_plugin_extracts() {
    let mut failures = Vec::new();

    for (plugin, file) in sample_files() {
        let name = key(&plugin, &file);
        let data = match service::view_file(&file) {
            Ok(protocol::Response::FileView { data, .. }) => data,
            other => {
                failures.push(format!("{name}: did not view as a file ({other:?})"));
                continue;
            }
        };
        let Some(object) = data.as_object() else {
            failures.push(format!("{name}: view data is not an object"));
            continue;
        };

        for (field, value) in object {
            let allowed = ALLOWED_GAPS
                .iter()
                .any(|(sample, gap)| *sample == name && gap == field);
            if allowed {
                continue;
            }
            match value {
                serde_json::Value::Array(items) if items.is_empty() => failures.push(format!(
                    "{name}: `{field}` is empty, so nothing proves the plugin can fill it"
                )),
                serde_json::Value::Null => failures.push(format!(
                    "{name}: `{field}` is absent, so nothing proves the plugin can find it"
                )),
                _ => {}
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} fixture field(s) prove nothing:\n{}\n\nEither enrich the fixture, or - if the format \
         cannot carry the field at all - say so in ALLOWED_GAPS.",
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
fn every_allowed_gap_names_a_fixture_that_exists() {
    // An exception left behind after its fixture was renamed would quietly
    // excuse a field on a file that no longer exists.
    let samples: BTreeSet<String> = sample_files()
        .into_iter()
        .map(|(plugin, file)| key(&plugin, &file))
        .collect();

    let orphans: Vec<&str> = ALLOWED_GAPS
        .iter()
        .map(|(sample, _field)| *sample)
        .filter(|sample| !samples.contains(*sample))
        .collect();

    assert!(
        orphans.is_empty(),
        "ALLOWED_GAPS names fixtures that are not there: {orphans:?}"
    );
}
