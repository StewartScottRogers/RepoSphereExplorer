//! TOML file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// UTF-8 byte order mark, stripped before sniffing.
const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// View data produced by [`TomlCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TomlView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary), shown
    /// as a fallback when `parsed` is `None`.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// The file parsed as TOML and converted to a JSON value, or `None` if
    /// it is not valid TOML.
    pub parsed: Option<Value>,
}

/// Whether every character of `key` is a valid TOML bare-key character.
fn is_bare_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Whether `part` is a valid single segment of a (possibly dotted) TOML
/// key: a bare key, or a basic/literal quoted key.
fn is_key_segment(part: &str) -> bool {
    let part = part.trim();
    is_bare_key(part)
        || (part.len() >= 2 && part.starts_with('"') && part.ends_with('"'))
        || (part.len() >= 2 && part.starts_with('\'') && part.ends_with('\''))
}

/// Whether `line` is a TOML table header (`[section]`) or array-of-tables
/// header (`[[section]]`), e.g. `[package]` or `[[bin]]`.
fn is_table_header(line: &str) -> bool {
    let inner = line
        .strip_prefix("[[")
        .and_then(|rest| rest.strip_suffix("]]"))
        .or_else(|| {
            line.strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
        });
    inner.is_some_and(|inner| !inner.is_empty() && inner.split('.').all(is_key_segment))
}

/// Whether `value` opens a value type that, on its own, is a strong TOML
/// marker: a quoted string, a boolean, or an ISO 8601 date/date-time.
/// Deliberately excludes bare numbers, inline arrays (`[`), and inline
/// tables (`{`), since a bare `key = [...]` or `key = 1` line is common
/// top-level assignment syntax in other languages this project already
/// sniffs (e.g. Python, JavaScript) and would false-positive on them.
fn is_toml_value_start(value: &str) -> bool {
    value == "true"
        || value == "false"
        || value.starts_with('"')
        || value.starts_with('\'')
        || looks_like_date(value)
}

/// Whether `value` opens with an ISO 8601 date (`YYYY-MM-DD...`), TOML's
/// bare (unquoted) date/date-time literal syntax.
fn looks_like_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 5 && bytes[..4].iter().all(u8::is_ascii_digit) && bytes[4] == b'-'
}

/// Whether `line` is a `key = value` assignment with a TOML-flavoured key
/// and a value that opens like a TOML literal.
fn is_key_value_line(line: &str) -> bool {
    let Some((key, value)) = line.split_once('=') else {
        return false;
    };
    let key = key.trim();
    let value = value.trim();
    !key.is_empty()
        && !value.is_empty()
        && key.split('.').all(is_key_segment)
        && is_toml_value_start(value)
}

/// Whether `prefix` looks like TOML: some line in it (once a leading BOM is
/// stripped) is a table header or a recognisably-TOML key/value
/// assignment — markers not used by any sibling plugin.
fn looks_like_toml(prefix: &[u8]) -> bool {
    let without_bom = prefix.strip_prefix(UTF8_BOM).unwrap_or(prefix);
    let Ok(text) = std::str::from_utf8(without_bom) else {
        return false;
    };
    text.lines().map(str::trim).any(|line| {
        !line.is_empty()
            && !line.starts_with('#')
            && (is_table_header(line) || is_key_value_line(line))
    })
}

/// A tree-line label: the root has none, a table entry is labelled by its
/// key, an array entry by its index.
enum Label<'a> {
    Root,
    Key(&'a str),
    Index(usize),
}

/// The English plural suffix for `count`.
fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// Renders a TOML scalar (anything but a table or array) as it would
/// appear in TOML source.
fn scalar_text(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => format!("{value:?}"),
        Value::Array(_) | Value::Object(_) => unreachable!("tables and arrays handled by caller"),
    }
}

/// Appends `value` to `lines` as an indented tree: one line per node, with
/// tables and arrays expanded into their children at one deeper indent.
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

/// The TOML plugin's core half.
#[derive(Debug, Default)]
pub struct TomlCore;

impl PluginCore for TomlCore {
    fn name(&self) -> &'static str {
        "toml"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_toml(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let parsed = std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| toml::from_str::<toml::Value>(text).ok())
            .and_then(|value| serde_json::to_value(value).ok());
        let view = TomlView {
            content,
            truncated,
            parsed,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The TOML plugin's presentation half.
#[derive(Debug, Default)]
pub struct TomlPresentation;

impl PluginPresentation for TomlPresentation {
    fn name(&self) -> &'static str {
        "toml"
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: TomlView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(value) = &view.parsed {
            push_tree_lines(value, 0, &Label::Root, &mut lines);
        } else {
            lines.push("could not parse as TOML; showing raw content".to_owned());
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
    use super::{MAX_VIEW_BYTES, TomlCore, TomlPresentation, TomlView};
    use plugin_api::{PluginCore, PluginPresentation};
    use serde_json::json;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-toml-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_table_headers_and_key_value_lines_as_toml() {
        assert!(TomlCore.sniff(b"[package]\nname = \"widgets\"\n"));
        assert!(TomlCore.sniff(b"[[bin]]\nname = \"main\"\n"));
        assert!(TomlCore.sniff(b"name = \"widgets\"\nversion = \"0.1.0\"\n"));
        assert!(TomlCore.sniff(b"enabled = true\n"));
        assert!(TomlCore.sniff(b"born = 1979-05-27T07:32:00Z\n"));
        assert!(TomlCore.sniff(b"# a comment\n[package]\n"));
        assert!(TomlCore.sniff("\u{feff}[package]\n".as_bytes()));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_toml() {
        assert!(!TomlCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!TomlCore.sniff(b"{\"a\": 1}\n"));
        assert!(!TomlCore.sniff(b"just a regular line of text\n"));
        assert!(!TomlCore.sniff(b"numbers = [1, 2, 3]\n"));
        assert!(!TomlCore.sniff(b"count = 5\n"));
        assert!(!TomlCore.sniff(b""));
        assert!(!TomlCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_toml_file_and_parses_it() {
        let path = unique_temp_file("doc.toml");
        std::fs::write(
            &path,
            "name = \"widgets\"\n[[tags]]\nvalue = \"a\"\n[[tags]]\nvalue = \"b\"\n",
        )
        .unwrap();

        let data = TomlCore.view(&path).unwrap();
        let view: TomlView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(
            view.parsed,
            Some(json!({
                "name": "widgets",
                "tags": [{"value": "a"}, {"value": "b"}],
            }))
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_an_invalid_toml_file_with_no_parsed_value() {
        let path = unique_temp_file("invalid.toml");
        std::fs::write(&path, "not = = toml").unwrap();

        let data = TomlCore.view(&path).unwrap();
        let view: TomlView = serde_json::from_value(data).unwrap();

        assert_eq!(view.parsed, None);
        assert_eq!(view.content, "not = = toml");

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_content_of_a_file_larger_than_the_view_limit_but_still_parses_it() {
        let path = unique_temp_file("large.toml");
        let entries: Vec<String> = (0..MAX_VIEW_BYTES)
            .map(|i| format!("k{i} = {i}\n"))
            .collect();
        let content = entries.join("");
        std::fs::write(&path, &content).unwrap();

        let data = TomlCore.view(&path).unwrap();
        let view: TomlView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);
        assert!(view.parsed.is_some());

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_a_tree_of_nested_tables_and_arrays() {
        let data = serde_json::to_value(TomlView {
            content: String::new(),
            truncated: false,
            parsed: Some(json!({"name": "widgets", "tags": ["a", "b"]})),
        })
        .unwrap();

        let lines = TomlPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "{} (2 keys)",
                "  \"name\": \"widgets\"",
                "  \"tags\": [] (2 items)",
                "    [0]: \"a\"",
                "    [1]: \"b\"",
            ]
        );
    }

    #[test]
    fn presents_raw_content_when_not_parseable() {
        let data = serde_json::to_value(TomlView {
            content: "not = = toml".to_owned(),
            truncated: true,
            parsed: None,
        })
        .unwrap();

        let lines = TomlPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "could not parse as TOML; showing raw content",
                "not = = toml",
                "… (truncated)",
            ]
        );
    }
}
