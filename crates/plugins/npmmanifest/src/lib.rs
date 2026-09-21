//! Node package manifest (`package.json`) file type plugin: core and
//! presentation halves.
//!
//! Registered before `json`, which would otherwise claim it on the
//! extension. Node is the runtime and ecosystem this format was written
//! for, and "Node" is used throughout as its name.

use plugin_api::{Icon, PluginCore, PluginPresentation, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;
use syntax::{Language, Quote};

/// The lowercase extensions this type claims, without their dot. `none;
/// recognised by file name` per the work order - there is no extension a
/// manifest carries, only the bare name `package.json`, and this plugin's
/// sniff reads the object shape a manifest has instead.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// Keys that mean the object belongs to a different manifest format that
/// also names and versions itself - so a content sniff on `name` and
/// `version` alone would misclaim it. Each is a key this project's own
/// sample fixtures for other ecosystems genuinely carry: an npm lock file
/// (`lockfileVersion`), `elm.json` (`elm-version`), `dub.json`
/// (`targetType`), `spago.json` (`packages_db_version`) and
/// `composer.json` (`require`, its own dependency key).
const OTHER_MANIFEST_MARKERS: &[&str] = &[
    "lockfileVersion",
    "elm-version",
    "targetType",
    "packages_db_version",
    "require",
];

/// Keys that mark a JSON object as distinctly an npm/Node manifest,
/// beyond a bare `name` or `version` that plenty of unrelated JSON
/// documents also carry - this project's own samples do: a `GeoJSON`
/// `FeatureCollection` names itself, and a JSON configuration fixture
/// both names and versions itself. At least one of these has to be
/// present too before the object counts as a manifest.
const MANIFEST_MARKERS: &[&str] = &[
    "scripts",
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "bin",
    "exports",
    "engines",
    "packageManager",
    "workspaces",
    "private",
];

/// One `name: value` pair, used for scripts, dependencies, engines and
/// `bin` entries alike.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The key: a script name, a dependency name, an engine name, or a
    /// `bin` command name.
    pub name: String,
    /// The associated value: a command, a version range, an engine
    /// requirement, a path, or an export target.
    pub value: String,
}

/// View data produced by [`NpmmanifestCore::view`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpmmanifestView {
    /// Whether the manifest parsed as a JSON object at all. When `false`
    /// every other field is empty, and the presentation half says so
    /// instead of showing a manifest with nothing in it.
    pub valid: bool,
    /// The package name.
    pub name: Option<String>,
    /// The package version.
    pub version: Option<String>,
    /// Whether the package is marked private, refusing an accidental
    /// publish.
    pub private: bool,
    /// The module system: `"commonjs"` or `"module"`.
    pub module_type: Option<String>,
    /// The executables it publishes, name to path.
    pub bin: Vec<Entry>,
    /// The subpaths it publishes, from the `exports` map.
    pub exports: Vec<Entry>,
    /// The `scripts` block, name to command.
    pub scripts: Vec<Entry>,
    /// Dependencies needed to run the package.
    pub dependencies: Vec<Entry>,
    /// Dependencies needed only to develop it.
    pub dev_dependencies: Vec<Entry>,
    /// Dependencies the package expects its host to provide.
    pub peer_dependencies: Vec<Entry>,
    /// The runtimes and tools it demands, such as `node` or `npm`.
    pub engines: Vec<Entry>,
    /// The pinned package manager, as `name@version`.
    pub package_manager: Option<String>,
    /// The workspace member globs, when this manifest is a workspace
    /// root.
    pub workspaces: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// `value` as a string: itself, when it already is one, or its compact
/// JSON rendering otherwise - which is how a conditional `exports` target
/// or a single-string `bin` entry's sibling values are shown.
fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// `object[key]`'s entries as `name: value` pairs, or empty when the key
/// is absent or not an object.
fn entries_of(object: &serde_json::Map<String, Value>, key: &str) -> Vec<Entry> {
    object
        .get(key)
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(name, value)| Entry {
                    name: name.clone(),
                    value: value_to_string(value),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The `bin` field's entries: an object maps command names to paths, and
/// a bare string is one command, named after the package itself.
fn bin_entries(object: &serde_json::Map<String, Value>, name: Option<&str>) -> Vec<Entry> {
    match object.get("bin") {
        Some(Value::Object(_)) => entries_of(object, "bin"),
        Some(Value::String(path)) => vec![Entry {
            name: name.unwrap_or_default().to_owned(),
            value: path.clone(),
        }],
        _ => Vec::new(),
    }
}

/// The `workspaces` field's member globs: a bare array, or an object's
/// `packages` array.
fn workspaces_of(object: &serde_json::Map<String, Value>) -> Vec<String> {
    let strings = |value: &Value| {
        value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    match object.get("workspaces") {
        Some(array @ Value::Array(_)) => strings(array),
        Some(Value::Object(workspaces)) => {
            workspaces.get("packages").map(strings).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// Everything [`NpmmanifestView`] holds, read from `text`.
fn parse(text: &str) -> NpmmanifestView {
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return NpmmanifestView::default();
    };
    let Some(object) = root.as_object() else {
        return NpmmanifestView::default();
    };
    let string = |key: &str| object.get(key).and_then(Value::as_str).map(str::to_owned);

    NpmmanifestView {
        valid: true,
        name: string("name"),
        version: string("version"),
        private: object
            .get("private")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        module_type: string("type"),
        bin: bin_entries(object, string("name").as_deref()),
        exports: entries_of(object, "exports"),
        scripts: entries_of(object, "scripts"),
        dependencies: entries_of(object, "dependencies"),
        dev_dependencies: entries_of(object, "devDependencies"),
        peer_dependencies: entries_of(object, "peerDependencies"),
        engines: entries_of(object, "engines"),
        package_manager: string("packageManager"),
        workspaces: workspaces_of(object),
        truncated: false,
    }
}

/// Whether `text` looks like a Node package manifest: a JSON object
/// naming or versioning itself, carrying at least one of
/// [`MANIFEST_MARKERS`], and none of [`OTHER_MANIFEST_MARKERS`].
fn looks_like_it(text: &str) -> bool {
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    let Some(object) = root.as_object() else {
        return false;
    };
    if OTHER_MANIFEST_MARKERS
        .iter()
        .any(|marker| object.contains_key(*marker))
    {
        return false;
    }
    let names_itself = object.contains_key("name") || object.contains_key("version");
    names_itself
        && MANIFEST_MARKERS
            .iter()
            .any(|marker| object.contains_key(*marker))
}

/// The Node package manifest plugin's core half.
#[derive(Debug, Default)]
pub struct NpmmanifestCore;

/// How this language is coloured, for the shared tokeniser. GUIDANCE.md
/// §3.6: the plugin describes its own format, the pane paints what it is
/// told.
const NPMMANIFEST: Language = Language {
    line_comment: &[],
    block_comment: &[],
    quotes: &[Quote::simple('"'), Quote::simple('\'')],
    keywords: &["false", "null", "true"],
    types: &[],
    calls: false,
    ignore_case: false,
};

impl PluginCore for NpmmanifestCore {
    fn name(&self) -> &'static str {
        "npmmanifest"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A manifest is JSON, and `json` owns the extension. Without this
        // the extension hint hands it over regardless of order.
        &["json"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Node package manifest plugin's presentation half.
#[derive(Debug, Default)]
pub struct NpmmanifestPresentation;

/// `entries` as indented `name value` lines, under `heading` - or nothing
/// when there are none.
fn section(lines: &mut Vec<String>, heading: &str, entries: &[Entry]) {
    if entries.is_empty() {
        return;
    }
    lines.push(format!("{heading}:"));
    for entry in entries {
        lines.push(format!("  {} {}", entry.name, entry.value));
    }
}

impl PluginPresentation for NpmmanifestPresentation {
    fn classify(&self, text: &str) -> Vec<Span> {
        syntax::classify(text, &NPMMANIFEST)
    }

    fn name(&self) -> &'static str {
        "npmmanifest"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "NPM",
            tint: 0x00cb_3837,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: NpmmanifestView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        if !view.valid {
            return vec!["not a valid package manifest: could not parse it as JSON".to_owned()];
        }
        let mut lines = Vec::new();
        let identity = match (&view.name, &view.version) {
            (Some(name), Some(version)) => format!("{name}@{version}"),
            (Some(name), None) => name.clone(),
            (None, Some(version)) => format!("(unnamed) {version}"),
            (None, None) => "(unnamed package)".to_owned(),
        };
        lines.push(identity);
        if view.private {
            lines.push("private: not published".to_owned());
        }
        if let Some(module_type) = &view.module_type {
            lines.push(format!("type: {module_type}"));
        }
        if let Some(package_manager) = &view.package_manager {
            lines.push(format!("packageManager: {package_manager}"));
        }
        if !view.workspaces.is_empty() {
            lines.push(format!("workspaces: {}", view.workspaces.join(", ")));
        }
        section(&mut lines, "scripts", &view.scripts);
        section(&mut lines, "bin", &view.bin);
        section(&mut lines, "exports", &view.exports);
        section(&mut lines, "dependencies", &view.dependencies);
        section(&mut lines, "devDependencies", &view.dev_dependencies);
        section(&mut lines, "peerDependencies", &view.peer_dependencies);
        section(&mut lines, "engines", &view.engines);

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{NpmmanifestCore, NpmmanifestPresentation, NpmmanifestView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const MANIFEST: &str = r#"{
      "name": "@example/toolkit",
      "version": "3.1.0",
      "private": true,
      "type": "module",
      "bin": { "toolkit": "./bin/toolkit.js" },
      "exports": { ".": "./index.js", "./feature": "./feature.js" },
      "scripts": { "build": "tsc", "test": "vitest run", "lint": "eslint ." },
      "dependencies": { "chalk": "^5.3.0" },
      "devDependencies": { "vitest": "^2.1.0" },
      "peerDependencies": { "react": "^18.0.0" },
      "engines": { "node": ">=20" },
      "packageManager": "pnpm@9.6.0",
      "workspaces": ["packages/*"]
    }"#;

    #[test]
    fn sniffs_a_manifest_by_its_object_shape() {
        assert!(NpmmanifestCore.sniff(MANIFEST.as_bytes()));
        assert!(NpmmanifestCore.sniff(br#"{"name": "a", "scripts": {"build": "x"}}"#));
        assert!(NpmmanifestCore.sniff(br#"{"version": "1.0.0", "dependencies": {"x": "1"}}"#));
    }

    #[test]
    fn does_not_claim_json_with_only_a_name_or_only_a_marker_key() {
        // A bare `name` and nothing npm-specific is what an unrelated
        // document - a GeoJSON `FeatureCollection`, a JSON configuration
        // file - also carries; requiring one of `MANIFEST_MARKERS` too is
        // what tells them apart.
        assert!(!NpmmanifestCore.sniff(br#"{"name": "a"}"#));
        assert!(!NpmmanifestCore.sniff(br#"{"scripts": {"build": "x"}}"#));
        assert!(!NpmmanifestCore.sniff(br#"{"description": "no marker keys here"}"#));
        assert!(!NpmmanifestCore.sniff(b""));
    }

    #[test]
    fn does_not_claim_an_unrelated_document_that_merely_names_itself() {
        // samples/geojson/london.geojson has a top-level `name`; the JSON
        // plugin's own fixture has `name` and `version` both - neither
        // carries anything npm-specific.
        assert!(!NpmmanifestCore.sniff(
            br#"{"type": "FeatureCollection", "name": "london-transport-sample", "features": []}"#
        ));
        assert!(
            !NpmmanifestCore.sniff(
                br#"{"name": "repo-sphere-explorer", "version": "0.6.0", "environments": {}}"#
            )
        );
    }

    #[test]
    fn does_not_claim_an_npm_lock_file() {
        assert!(
            !NpmmanifestCore.sniff(
                br#"{"name": "a", "version": "1.0.0", "lockfileVersion": 3, "packages": {}}"#
            )
        );
    }

    #[test]
    fn does_not_claim_other_ecosystems_manifests() {
        // elm.json
        assert!(!NpmmanifestCore.sniff(
            br#"{"type": "application", "elm-version": "0.19.1", "dependencies": {"direct": {}}}"#
        ));
        // dub.json
        assert!(
            !NpmmanifestCore
                .sniff(br#"{"name": "a", "targetType": "executable", "dependencies": {}}"#)
        );
        // spago.json
        assert!(
            !NpmmanifestCore.sniff(
                br#"{"name": "a", "dependencies": ["prelude"], "packages_db_version": "1"}"#
            )
        );
        // composer.json
        assert!(!NpmmanifestCore.sniff(
            br#"{"name": "a/b", "require": {"php": ">=8.3"}, "scripts": {"test": "phpunit"}}"#
        ));
    }

    #[test]
    fn reads_identity_and_flags() {
        let view = parse(MANIFEST);

        assert_eq!(view.name.as_deref(), Some("@example/toolkit"));
        assert_eq!(view.version.as_deref(), Some("3.1.0"));
        assert!(view.private);
        assert_eq!(view.module_type.as_deref(), Some("module"));
        assert_eq!(view.package_manager.as_deref(), Some("pnpm@9.6.0"));
        assert_eq!(view.workspaces, vec!["packages/*".to_owned()]);
    }

    #[test]
    fn reads_bin_as_an_object() {
        let view = parse(MANIFEST);

        assert_eq!(view.bin.len(), 1);
        assert_eq!(view.bin[0].name, "toolkit");
        assert_eq!(view.bin[0].value, "./bin/toolkit.js");
    }

    #[test]
    fn reads_a_single_string_bin_named_after_the_package() {
        let view = parse(r#"{"name": "solo", "bin": "./cli.js"}"#);

        assert_eq!(
            view.bin,
            vec![super::Entry {
                name: "solo".to_owned(),
                value: "./cli.js".to_owned(),
            }]
        );
    }

    #[test]
    fn reads_exports() {
        let view = parse(MANIFEST);

        assert_eq!(view.exports.len(), 2);
        assert!(
            view.exports
                .iter()
                .any(|entry| entry.name == "." && entry.value == "./index.js")
        );
    }

    #[test]
    fn reads_conditional_exports_as_their_json_rendering() {
        let view = parse(r#"{"name": "a", "exports": {".": {"import": "./index.mjs"}}}"#);

        assert_eq!(view.exports.len(), 1);
        assert!(view.exports[0].value.contains("index.mjs"));
    }

    #[test]
    fn splits_dependencies_into_three_kinds() {
        let view = parse(MANIFEST);

        assert_eq!(view.dependencies.len(), 1);
        assert_eq!(view.dev_dependencies.len(), 1);
        assert_eq!(view.peer_dependencies.len(), 1);
        assert_eq!(view.dependencies[0].name, "chalk");
    }

    #[test]
    fn reads_scripts_and_engines() {
        let view = parse(MANIFEST);

        assert_eq!(view.scripts.len(), 3);
        assert!(
            view.scripts
                .iter()
                .any(|entry| entry.name == "build" && entry.value == "tsc")
        );
        assert_eq!(view.engines.len(), 1);
        assert_eq!(view.engines[0].name, "node");
    }

    #[test]
    fn reads_workspaces_from_a_packages_object() {
        let view = parse(r#"{"name": "root", "workspaces": {"packages": ["apps/*", "libs/*"]}}"#);

        assert_eq!(
            view.workspaces,
            vec!["apps/*".to_owned(), "libs/*".to_owned()]
        );
    }

    #[test]
    fn a_malformed_manifest_is_refused_without_panicking() {
        let view = parse("{ this is not valid json");

        assert!(!view.valid);

        let data = serde_json::to_value(&view).unwrap();
        let lines = NpmmanifestPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["not a valid package manifest: could not parse it as JSON"]
        );
    }

    #[test]
    fn a_truncated_manifest_that_no_longer_parses_is_refused_too() {
        let view = super::NpmmanifestView {
            valid: false,
            truncated: true,
            ..NpmmanifestView::default()
        };
        let data = serde_json::to_value(&view).unwrap();

        let lines = NpmmanifestPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["not a valid package manifest: could not parse it as JSON"]
        );
    }

    #[test]
    fn presents_the_identity_line_first() {
        let data = serde_json::to_value(parse(MANIFEST)).unwrap();

        let lines = NpmmanifestPresentation.present(&data);

        assert_eq!(lines[0], "@example/toolkit@3.1.0");
        assert!(lines.iter().any(|line| line == "private: not published"));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/npmmanifest/package.json");

        let data = NpmmanifestCore.view(&path).unwrap();
        let view: NpmmanifestView = serde_json::from_value(data).unwrap();

        assert!(view.valid);
        assert!(view.name.is_some());
        assert!(view.version.is_some());
        assert!(view.private);
        assert!(view.module_type.is_some());
        assert!(!view.bin.is_empty());
        assert!(!view.exports.is_empty());
        assert!(!view.scripts.is_empty());
        assert!(!view.dependencies.is_empty());
        assert!(!view.dev_dependencies.is_empty());
        assert!(!view.peer_dependencies.is_empty());
        assert!(!view.engines.is_empty());
        assert!(view.package_manager.is_some());
        assert_eq!(view.workspaces.len(), 2);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::NpmmanifestCore),
            plugin_api::PluginPresentation::extensions(&crate::NpmmanifestPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
