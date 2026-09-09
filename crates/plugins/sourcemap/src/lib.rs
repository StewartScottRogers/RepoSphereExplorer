//! Source map file type plugin: core and presentation halves.
//!
//! A specialisation of JSON: `version: 3` alongside `mappings` and
//! `sources` is the source map shape and nothing else's.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["map"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`SourcemapCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcemapView {
    /// The format version, which is 3 everywhere in practice.
    pub version: u64,
    /// The generated file this maps back from.
    pub file: Option<String>,
    /// The prefix joined to every source path.
    pub source_root: Option<String>,
    /// The original sources, in order.
    pub sources: Vec<String>,
    /// How many of them carry their content inline, so the map stands
    /// alone without the originals beside it.
    pub sources_with_content: usize,
    /// The names table, which is what makes a stack trace readable.
    pub names: usize,
    /// How many segments the mappings hold, comma- and semicolon-separated.
    pub segments: usize,
    /// How many lines of generated output the mappings cover.
    pub generated_lines: usize,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Everything [`SourcemapView`] holds, read from `text`.
fn parse(text: &str) -> SourcemapView {
    let mut view = SourcemapView {
        version: 0,
        file: None,
        source_root: None,
        sources: Vec::new(),
        sources_with_content: 0,
        names: 0,
        segments: 0,
        generated_lines: 0,
        truncated: false,
    };
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return view;
    };

    view.version = root.get("version").and_then(Value::as_u64).unwrap_or(0);
    view.file = root.get("file").and_then(Value::as_str).map(str::to_owned);
    view.source_root = root
        .get("sourceRoot")
        .and_then(Value::as_str)
        .map(str::to_owned);

    if let Some(sources) = root.get("sources").and_then(Value::as_array) {
        view.sources = sources
            .iter()
            .map(|source| source.as_str().unwrap_or("(unnamed)").to_owned())
            .collect();
    }
    if let Some(contents) = root.get("sourcesContent").and_then(Value::as_array) {
        view.sources_with_content = contents.iter().filter(|entry| !entry.is_null()).count();
    }
    if let Some(names) = root.get("names").and_then(Value::as_array) {
        view.names = names.len();
    }
    if let Some(mappings) = root.get("mappings").and_then(Value::as_str) {
        let lines: Vec<&str> = mappings.split(';').collect();
        view.generated_lines = lines.len();
        view.segments = lines
            .iter()
            .map(|line| line.split(',').filter(|s| !s.is_empty()).count())
            .sum();
    }
    view
}

/// Whether `text` is a source map.
fn looks_like_it(text: &str) -> bool {
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    root.get("version").and_then(Value::as_u64) == Some(3)
        && root.get("mappings").is_some()
        && root.get("sources").is_some()
}

/// The Source map plugin's core half.
#[derive(Debug, Default)]
pub struct SourcemapCore;

impl PluginCore for SourcemapCore {
    fn name(&self) -> &'static str {
        "sourcemap"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A specialisation of JSON, which owns the extension. Without this
        // the extension hint hands the file over whatever the order (D13).
        &["json"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        // A source map's `mappings` string is one unreadable line hundreds of
        // kilobytes long. Carrying it would fill the pane with base64 and
        // tell a reader nothing, so the view holds the summary instead.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Source map plugin's presentation half.
#[derive(Debug, Default)]
pub struct SourcemapPresentation;

impl PluginPresentation for SourcemapPresentation {
    fn name(&self) -> &'static str {
        "sourcemap"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "MAP",
            tint: 0x0064_95ed,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: SourcemapView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("Source map version {}", view.version));
        if let Some(file) = &view.file {
            lines.push(format!("Maps back from: {file}"));
        }
        if let Some(root) = &view.source_root {
            lines.push(format!("Source root: {root}"));
        }
        lines.push(format!(
            "{} source(s), {} carrying their own content",
            view.sources.len(),
            view.sources_with_content
        ));
        for source in &view.sources {
            lines.push(format!("  {source}"));
        }
        if view.sources_with_content < view.sources.len() {
            lines.push(
                "Without the originals beside it this map cannot show the code it names."
                    .to_owned(),
            );
        }
        lines.push(format!("Names: {}", view.names));
        lines.push(format!(
            "{} mapping segment(s) over {} generated line(s)",
            view.segments, view.generated_lines
        ));

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{SourcemapCore, SourcemapPresentation, SourcemapView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const MAP: &str = r#"{
      "version": 3,
      "file": "bundle.js",
      "sourceRoot": "/src",
      "sources": ["a.ts", "b.ts"],
      "sourcesContent": ["export const a = 1;", null],
      "names": ["a", "b", "run"],
      "mappings": "AAAA,SAASA;AACT,SAASC,IAAI"
    }"#;

    #[test]
    fn sniffs_version_three_with_mappings_and_sources() {
        assert!(SourcemapCore.sniff(MAP.as_bytes()));
    }

    #[test]
    fn does_not_claim_json_that_merely_has_a_version() {
        assert!(!SourcemapCore.sniff(br#"{"version": 3, "name": "a"}"#));
        assert!(!SourcemapCore.sniff(br#"{"mappings": "A", "sources": []}"#));
        assert!(!SourcemapCore.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_json() {
        assert_eq!(SourcemapCore.specialises(), &["json"]);
    }

    #[test]
    fn reads_the_generated_file_and_its_sources() {
        let view = parse(MAP);

        assert_eq!(view.version, 3);
        assert_eq!(view.file.as_deref(), Some("bundle.js"));
        assert_eq!(view.source_root.as_deref(), Some("/src"));
        assert_eq!(view.sources, vec!["a.ts".to_owned(), "b.ts".to_owned()]);
    }

    #[test]
    fn a_null_in_sources_content_is_not_content() {
        let view = parse(MAP);

        assert_eq!(view.sources_with_content, 1, "one of the two is null");
    }

    #[test]
    fn counts_segments_and_the_lines_they_cover() {
        let view = parse(MAP);

        assert_eq!(view.generated_lines, 2);
        // Two on the first generated line, three on the second.
        assert_eq!(view.segments, 5);
        assert_eq!(view.names, 3);
    }

    #[test]
    fn presents_the_warning_when_the_originals_are_missing() {
        let data = serde_json::to_value(parse(MAP)).unwrap();

        let lines = SourcemapPresentation.present(&data);

        assert_eq!(lines[0], "Source map version 3");
        assert!(
            lines
                .iter()
                .any(|line| line.contains("cannot show the code"))
        );
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/sourcemap/bundle.js.map");

        let data = SourcemapCore.view(&path).unwrap();
        let view: SourcemapView = serde_json::from_value(data).unwrap();

        assert_eq!(view.version, 3);
        assert!(view.file.is_some());
        assert!(view.source_root.is_some());
        assert!(view.sources.len() >= 3);
        assert!(view.sources_with_content >= 1);
        assert!(view.names >= 3);
        assert!(view.segments > 5);
        assert!(view.generated_lines > 1);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::SourcemapCore),
            plugin_api::PluginPresentation::extensions(&crate::SourcemapPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
