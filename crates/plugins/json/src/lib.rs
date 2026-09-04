//! JSON file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// UTF-8 byte order mark, stripped before sniffing.
const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// View data produced by [`JsonCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary), shown
    /// as a fallback when `parsed` is `None`.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// The file parsed as a JSON value, or `None` if it is not valid JSON.
    pub parsed: Option<Value>,
}

/// Strips a leading UTF-8 BOM and ASCII whitespace from `prefix`.
fn trim_prefix(prefix: &[u8]) -> &[u8] {
    let without_bom = prefix.strip_prefix(UTF8_BOM).unwrap_or(prefix);
    let start = without_bom
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(without_bom.len());
    &without_bom[start..]
}

/// Whether `prefix` looks like JSON: it starts with `{` or `[`, and the
/// first JSON value in it either parses cleanly or runs out of bytes
/// (`prefix` is a bounded prefix of a potentially larger file, so a clean
/// parse isn't required — only that nothing seen so far is invalid syntax).
/// This is stronger than this project's usual marker-based sniffing, since
/// JSON's own grammar gives a precise test; no sibling plugin's markers
/// overlap with it. A future YAML plugin, whose flow style is a JSON
/// superset, will need to be ordered after this one, or use its own
/// stronger markers, to claim genuine YAML documents that happen to open
/// with `{` or `[`.
fn looks_like_json(prefix: &[u8]) -> bool {
    let trimmed = trim_prefix(prefix);
    match trimmed.first() {
        Some(b'{' | b'[') => {}
        _ => return false,
    }
    let mut values =
        serde_json::Deserializer::from_slice(trimmed).into_iter::<serde::de::IgnoredAny>();
    match values.next() {
        Some(Ok(_)) => true,
        Some(Err(err)) => err.is_eof(),
        None => false,
    }
}

/// A tree-line label: the root has none, an object child is labelled by its
/// key, an array child by its index.
enum Label<'a> {
    Root,
    Key(&'a str),
    Index(usize),
}

/// The English plural suffix for `count`.
fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// Renders a JSON scalar (anything but an object or array) as it would
/// appear in JSON source.
fn scalar_text(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => format!("{value:?}"),
        Value::Array(_) | Value::Object(_) => unreachable!("objects and arrays handled by caller"),
    }
}

/// Appends `value` to `lines` as an indented tree: one line per node, with
/// objects and arrays expanded into their children at one deeper indent.
fn push_tree_lines(value: &Value, depth: usize, label: &Label<'_>, lines: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    let prefix = match label {
        Label::Root => String::new(),
        Label::Key(key) => format!("{key:?}: "),
        Label::Index(index) => format!("[{index}]: "),
    };
    match value {
        Value::Object(entries) => {
            lines.push(format!(
                "{indent}{prefix}{{}} ({} key{})",
                entries.len(),
                plural(entries.len())
            ));
            for (key, child) in entries {
                push_tree_lines(child, depth + 1, &Label::Key(key), lines);
            }
        }
        Value::Array(items) => {
            lines.push(format!(
                "{indent}{prefix}[] ({} item{})",
                items.len(),
                plural(items.len())
            ));
            for (index, child) in items.iter().enumerate() {
                push_tree_lines(child, depth + 1, &Label::Index(index), lines);
            }
        }
        scalar => lines.push(format!("{indent}{prefix}{}", scalar_text(scalar))),
    }
}

/// The JSON plugin's core half.
#[derive(Debug, Default)]
pub struct JsonCore;

impl PluginCore for JsonCore {
    fn name(&self) -> &'static str {
        "json"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_json(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let parsed = serde_json::from_slice::<Value>(&bytes).ok();
        let view = JsonView {
            content,
            truncated,
            parsed,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The JSON plugin's presentation half.
#[derive(Debug, Default)]
pub struct JsonPresentation;

impl PluginPresentation for JsonPresentation {
    fn name(&self) -> &'static str {
        "json"
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: JsonView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(value) = &view.parsed {
            push_tree_lines(value, 0, &Label::Root, &mut lines);
        } else {
            lines.push("could not parse as JSON; showing raw content".to_owned());
            lines.extend(view.content.lines().map(str::to_owned));
        }
        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{JsonCore, JsonPresentation, JsonView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};
    use serde_json::json;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-json-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_full_and_truncated_json_as_json() {
        assert!(JsonCore.sniff(b"{\"a\": 1}"));
        assert!(JsonCore.sniff(b"[1, 2, 3]"));
        assert!(JsonCore.sniff(b"  \n\t{\"a\": 1}"));
        assert!(JsonCore.sniff(b"{\"a\": [1, 2, {\"b\":"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_json() {
        assert!(!JsonCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!JsonCore.sniff(b"<?xml version=\"1.0\"?>\n<root></root>\n"));
        assert!(!JsonCore.sniff(b"just a regular line of text\n"));
        assert!(!JsonCore.sniff(b"{ invalid json"));
        assert!(!JsonCore.sniff(b""));
        assert!(!JsonCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_json_file_and_parses_it() {
        let path = unique_temp_file("doc.json");
        std::fs::write(&path, "{\"name\": \"Alice\", \"tags\": [\"a\", \"b\"]}\n").unwrap();

        let data = JsonCore.view(&path).unwrap();
        let view: JsonView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(
            view.parsed,
            Some(json!({"name": "Alice", "tags": ["a", "b"]}))
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_an_invalid_json_file_with_no_parsed_value() {
        let path = unique_temp_file("invalid.json");
        std::fs::write(&path, "{ not json").unwrap();

        let data = JsonCore.view(&path).unwrap();
        let view: JsonView = serde_json::from_value(data).unwrap();

        assert_eq!(view.parsed, None);
        assert_eq!(view.content, "{ not json");

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_content_of_a_file_larger_than_the_view_limit_but_still_parses_it() {
        let path = unique_temp_file("large.json");
        let items: Vec<String> = (0..MAX_VIEW_BYTES).map(|i| i.to_string()).collect();
        let content = format!("[{}]", items.join(","));
        std::fs::write(&path, &content).unwrap();

        let data = JsonCore.view(&path).unwrap();
        let view: JsonView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);
        assert!(view.parsed.is_some());

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_a_tree_of_nested_objects_and_arrays() {
        let data = serde_json::to_value(JsonView {
            content: String::new(),
            truncated: false,
            parsed: Some(json!({"name": "Alice", "tags": ["a", "b"]})),
        })
        .unwrap();

        let lines = JsonPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "{} (2 keys)",
                "  \"name\": \"Alice\"",
                "  \"tags\": [] (2 items)",
                "    [0]: \"a\"",
                "    [1]: \"b\"",
            ]
        );
    }

    #[test]
    fn presents_raw_content_when_not_parseable() {
        let data = serde_json::to_value(JsonView {
            content: "{ not json".to_owned(),
            truncated: true,
            parsed: None,
        })
        .unwrap();

        let lines = JsonPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "could not parse as JSON; showing raw content",
                "{ not json",
                "… (truncated)",
            ]
        );
    }
}
