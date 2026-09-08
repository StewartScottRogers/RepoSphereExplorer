//! Sweeps every fixture in `samples/` through the whole preview path, the
//! way the File pane does it: the core half reads the file, the presentation
//! half turns that into the lines the pane shows.
//!
//! `service`'s own `samples.rs` checks that each fixture is *recognised* by
//! the right plugin. It stops there, so a plugin whose presentation half
//! cannot read its own core half's data - or renders nothing at all - has
//! been invisible to the test suite while looking broken in the pane.

use std::path::{Path, PathBuf};

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

/// The picture the File pane would draw for `path`, if its type is one.
fn graphic(path: &Path) -> Option<plugin_api::Graphic> {
    match service::view_file(path) {
        Ok(protocol::Response::FileView { plugin, data }) => {
            gui::app::present_graphic(&plugin, &data)
        }
        _ => None,
    }
}

/// The lines the File pane would show for `path`, or the reason it cannot.
fn preview(path: &Path) -> Result<Vec<String>, String> {
    match service::view_file(path) {
        Ok(protocol::Response::FileView { plugin, data }) => Ok(gui::app::present(&plugin, &data)),
        Ok(protocol::Response::Error { message }) => Err(format!("not recognised: {message}")),
        Ok(other) => Err(format!("unexpected response: {other:?}")),
        Err(err) => Err(format!("could not view: {err}")),
    }
}

#[test]
fn every_sample_renders_lines_in_the_file_pane() {
    let mut failures = Vec::new();
    for (plugin, file) in sample_files() {
        let name = file.file_name().unwrap_or_default().to_string_lossy();
        match preview(&file) {
            Ok(lines) if lines.is_empty() => {
                failures.push(format!("{plugin}/{name}: rendered no lines at all"));
            }
            Ok(lines) => {
                // Both halves report their own failures as a rendered line,
                // so a pane full of text can still be a broken preview.
                for line in &lines {
                    if line.starts_with("could not read view data")
                        || line.starts_with("no presentation for plugin")
                    {
                        failures.push(format!("{plugin}/{name}: {line}"));
                    }
                }
            }
            Err(reason) => failures.push(format!("{plugin}/{name}: {reason}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} sample(s) do not preview:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn a_directory_previews_as_its_own_plugin_would() {
    let dir = samples_dir().join("directory");
    let lines = preview(&dir).expect("a directory should preview");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("entries") || line.contains("entry")),
        "a directory preview counts what is in it: {lines:?}"
    );
}

#[test]
fn a_picture_previews_as_a_picture() {
    let png = samples_dir().join("image").join("logo.png");
    match graphic(&png).expect("an image offers a picture") {
        plugin_api::Graphic::Rgba {
            width,
            height,
            pixels,
        } => {
            assert!(width > 0 && height > 0, "{width}x{height}");
            assert_eq!(
                pixels.len(),
                width as usize * height as usize * 4,
                "RGBA8 is four bytes a pixel, and a miscount would panic the front end"
            );
        }
        plugin_api::Graphic::Svg(source) => panic!("expected decoded pixels, got svg {source:.40}"),
    }
}

#[test]
fn an_svg_previews_as_its_own_source() {
    let file = samples_dir().join("svg");
    let file = std::fs::read_dir(file)
        .expect("samples/svg should be readable")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.is_file())
        .expect("samples/svg has a fixture");

    match graphic(&file).expect("an svg offers a picture") {
        plugin_api::Graphic::Svg(source) => {
            assert!(source.contains("<svg"), "the markup itself: {source:.60}");
        }
        plugin_api::Graphic::Rgba { width, height, .. } => {
            panic!("expected svg source, got {width}x{height} pixels")
        }
    }
}

#[test]
fn every_graphic_a_plugin_offers_is_well_formed() {
    let mut failures = Vec::new();
    for (plugin, file) in sample_files() {
        let Some(picture) = graphic(&file) else {
            continue;
        };
        match picture {
            plugin_api::Graphic::Rgba {
                width,
                height,
                pixels,
            } => {
                let expected = width as usize * height as usize * 4;
                if expected == 0 || pixels.len() != expected {
                    failures.push(format!(
                        "{plugin}: {width}x{height} needs {expected} bytes, got {}",
                        pixels.len()
                    ));
                }
            }
            plugin_api::Graphic::Svg(source) => {
                if source.trim().is_empty() {
                    failures.push(format!("{plugin}: offered empty svg source"));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{}",
        failures.join(
            "
"
        )
    );
}

#[test]
fn a_type_that_is_not_a_picture_offers_none() {
    for plugin in ["rust", "json", "csv"] {
        let dir = samples_dir().join(plugin);
        let file = std::fs::read_dir(&dir)
            .expect("a sample directory should be readable")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.is_file())
            .expect("the directory has a fixture");
        assert!(
            graphic(&file).is_none(),
            "{plugin} is text, and text says more as text"
        );
    }
}
