//! YAML file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;
use yaml_rust2::{Yaml, YamlLoader};

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`YamlCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// The first document's top-level mapping keys, if it parses as YAML
    /// and its top level is a mapping.
    pub top_level_keys: Vec<String>,
    /// The number of `---`-separated documents the content parses into, or
    /// `0` if the content does not parse as YAML at all.
    pub document_count: usize,
}

/// Whether `line`, trimmed of surrounding whitespace, is a YAML document
/// start (`---`) or document end (`...`) marker.
fn is_document_marker(line: &str) -> bool {
    matches!(line.trim(), "---" | "...")
}

/// Whether `line` ends with a block scalar indicator (`|` or `>`, optionally
/// followed by a chomping (`+`/`-`) or explicit indentation digit modifier)
/// immediately after a mapping colon, e.g. `description: |` or `notes: >-`.
fn has_block_scalar_indicator(line: &str) -> bool {
    let trimmed = line.trim_end();
    let Some(colon) = trimmed.rfind(':') else {
        return false;
    };
    let rest = trimmed[colon + 1..].trim_start();
    let mut chars = rest.chars();
    match chars.next() {
        Some('|' | '>') => {}
        _ => return false,
    }
    chars.all(|c| c == '+' || c == '-' || c.is_ascii_digit())
}

/// Whether `line` contains a mapping-value anchor (`: &name`) or alias
/// (`: *name`), the markers YAML uses to reuse a node elsewhere in the
/// document.
fn has_anchor_or_alias(line: &str) -> bool {
    ["&", "*"].iter().any(|sigil| {
        line.split(": ").skip(1).any(|value| {
            value
                .strip_prefix(sigil)
                .is_some_and(|rest| rest.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
        })
    })
}

/// Whether `text` parses as YAML whose first document's top level is a
/// mapping with at least two keys or a sequence with at least two items —
/// strong evidence of real YAML data rather than a prose line that
/// incidentally contains a colon.
fn parses_as_multi_entry_document(text: &str) -> bool {
    let Ok(docs) = YamlLoader::load_from_str(text) else {
        return false;
    };
    docs.first().is_some_and(|doc| match doc {
        Yaml::Hash(map) => map.len() >= 2,
        Yaml::Array(items) => items.len() >= 2,
        _ => false,
    })
}

/// Whether `text` looks like YAML: a document marker, a `%YAML` directive, a
/// block scalar indicator, an anchor/alias, or a parsed multi-entry
/// document — markers not used by any sibling plugin.
fn has_yaml_syntax(text: &str) -> bool {
    text.lines().any(is_document_marker)
        || text
            .lines()
            .any(|line| line.trim_start().starts_with("%YAML"))
        || text.lines().any(has_block_scalar_indicator)
        || text.lines().any(has_anchor_or_alias)
        || parses_as_multi_entry_document(text)
}

/// Renders a [`Yaml`] scalar as a string, for use as a mapping key.
fn yaml_scalar_to_string(yaml: &Yaml) -> String {
    match yaml {
        Yaml::Real(value) | Yaml::String(value) => value.clone(),
        Yaml::Integer(value) => value.to_string(),
        Yaml::Boolean(value) => value.to_string(),
        Yaml::Array(_) | Yaml::Hash(_) | Yaml::Alias(_) | Yaml::Null | Yaml::BadValue => {
            String::new()
        }
    }
}

/// Extracts the first document's top-level mapping keys from `content`, or
/// an empty list if it doesn't parse as YAML or its top level isn't a
/// mapping.
fn parse_top_level_keys(content: &str) -> Vec<String> {
    let Ok(docs) = YamlLoader::load_from_str(content) else {
        return Vec::new();
    };
    let Some(Yaml::Hash(map)) = docs.first() else {
        return Vec::new();
    };
    map.keys().map(yaml_scalar_to_string).collect()
}

/// Counts the `---`-separated documents in `content`, or `0` if it does not
/// parse as YAML at all.
fn count_documents(content: &str) -> usize {
    YamlLoader::load_from_str(content).map_or(0, |docs| docs.len())
}

/// The YAML plugin's core half.
#[derive(Debug, Default)]
pub struct YamlCore;

impl PluginCore for YamlCore {
    fn name(&self) -> &'static str {
        "yaml"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_yaml_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let top_level_keys = parse_top_level_keys(&content);
        let document_count = count_documents(&content);
        let view = YamlView {
            content,
            truncated,
            top_level_keys,
            document_count,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The YAML plugin's presentation half.
#[derive(Debug, Default)]
pub struct YamlPresentation;

impl PluginPresentation for YamlPresentation {
    fn name(&self) -> &'static str {
        "yaml"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: YamlView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.top_level_keys.is_empty() {
            lines.push(format!("keys: {}", view.top_level_keys.join(", ")));
        }
        if view.document_count > 1 {
            lines.push(format!("documents: {}", view.document_count));
        }
        lines.extend(view.content.lines().map(str::to_owned));
        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_VIEW_BYTES, YamlCore, YamlPresentation, YamlView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-yaml-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_yaml_markers_as_yaml() {
        assert!(YamlCore.sniff(b"---\nname: widget\n"));
        assert!(YamlCore.sniff(b"%YAML 1.2\n---\nname: widget\n"));
        assert!(YamlCore.sniff(b"description: |\n  line one\n  line two\n"));
        assert!(YamlCore.sniff(b"base: &defaults\n  retries: 3\nother: *defaults\n"));
        assert!(YamlCore.sniff(b"name: widget\nversion: 1\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_yaml() {
        assert!(!YamlCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!YamlCore.sniff(b"<!DOCTYPE html>\n<html></html>\n"));
        assert!(!YamlCore.sniff(b"<?xml version=\"1.0\"?>\n<root></root>\n"));
        assert!(!YamlCore.sniff(b"just a regular line of text\n"));
        assert!(!YamlCore.sniff(b"Note: see below for details\n"));
        assert!(!YamlCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_yaml_file_and_extracts_keys_and_document_count() {
        let path = unique_temp_file("config.yaml");
        std::fs::write(
            &path,
            "---\nname: widget\nversion: 1\ntags:\n  - a\n  - b\n---\nname: gadget\n",
        )
        .unwrap();

        let data = YamlCore.view(&path).unwrap();
        let view: YamlView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.top_level_keys, vec!["name", "version", "tags"]);
        assert_eq!(view.document_count, 2);
        assert!(view.content.contains("widget"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.yaml");
        let mut content = "items:\n".to_owned();
        content.push_str(&"  - a paragraph of body text\n".repeat(MAX_VIEW_BYTES));
        std::fs::write(&path, content).unwrap();

        let data = YamlCore.view(&path).unwrap();
        let view: YamlView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_keys_document_count_and_content() {
        let data = serde_json::to_value(YamlView {
            content: "name: widget\nversion: 1\n".to_owned(),
            truncated: false,
            top_level_keys: vec!["name".to_owned(), "version".to_owned()],
            document_count: 2,
        })
        .unwrap();

        let lines = YamlPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "keys: name, version",
                "documents: 2",
                "name: widget",
                "version: 1",
            ]
        );
    }
}
