//! INI/properties config file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// Minimum number of `key=value`/`key: value` lines required to sniff a file
/// as INI/properties when it has no `[section]` header — a single such line
/// (e.g. a prose sentence like "Note: see below") is too common elsewhere to
/// be reliable on its own.
const MIN_KEY_VALUE_LINES: usize = 2;

/// View data produced by [`ConfigIniCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigIniView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Section names from `[section]` headers, in source order, deduplicated.
    pub sections: Vec<String>,
}

/// Extracts the section name from a `[section]` header line, or `None` if
/// `trimmed` is not such a header.
fn parse_section_header(trimmed: &str) -> Option<&str> {
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    let inner = inner.trim();
    (!inner.is_empty() && !inner.contains(['[', ']'])).then_some(inner)
}

/// Whether `trimmed` is a `key=value`/`key: value` line: a key made only of
/// letters, digits, `_`, `.`, or `-`, followed by a `=` or `:` separator.
fn is_key_value_line(trimmed: &str) -> bool {
    let Some(sep) = trimmed.find(['=', ':']) else {
        return false;
    };
    let key = trimmed[..sep].trim_end();
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Whether `line`, once trimmed, is a comment (`;` or `#` prefixed) or blank.
fn is_comment_or_blank(trimmed: &str) -> bool {
    trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#')
}

/// Whether `text` looks like an INI/properties config file: a `[section]`
/// header, or at least [`MIN_KEY_VALUE_LINES`] `key=value`/`key: value`
/// lines.
fn has_config_ini_syntax(text: &str) -> bool {
    let mut key_value_lines = 0;
    for line in text.lines() {
        let trimmed = line.trim();
        if is_comment_or_blank(trimmed) {
            continue;
        }
        if parse_section_header(trimmed).is_some() {
            return true;
        }
        if is_key_value_line(trimmed) {
            key_value_lines += 1;
        }
    }
    key_value_lines >= MIN_KEY_VALUE_LINES
}

/// Parses `[section]` headers out of `content`, in source order,
/// deduplicated.
fn parse_sections(content: &str) -> Vec<String> {
    let mut sections = Vec::new();
    for line in content.lines() {
        if let Some(name) = parse_section_header(line.trim()) {
            let name = name.to_owned();
            if !sections.contains(&name) {
                sections.push(name);
            }
        }
    }
    sections
}

/// The INI/properties config plugin's core half.
#[derive(Debug, Default)]
pub struct ConfigIniCore;

impl PluginCore for ConfigIniCore {
    fn name(&self) -> &'static str {
        "config-ini"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_config_ini_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let sections = parse_sections(&content);
        let view = ConfigIniView {
            content,
            truncated,
            sections,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The INI/properties config plugin's presentation half.
#[derive(Debug, Default)]
pub struct ConfigIniPresentation;

impl PluginPresentation for ConfigIniPresentation {
    fn name(&self) -> &'static str {
        "config-ini"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: ConfigIniView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.sections.is_empty() {
            lines.push(format!("sections: {}", view.sections.join(", ")));
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
    use super::{ConfigIniCore, ConfigIniPresentation, ConfigIniView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-config-ini-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_section_header_as_config_ini() {
        assert!(ConfigIniCore.sniff(b"[database]\nhost=localhost\n"));
        assert!(ConfigIniCore.sniff(b"; a comment\n[core]\neditor = vim\n"));
    }

    #[test]
    fn sniffs_multiple_key_value_lines_without_a_section_as_config_ini() {
        assert!(ConfigIniCore.sniff(b"DATABASE_URL=postgres://localhost/app\nDEBUG=true\n"));
        assert!(ConfigIniCore.sniff(b"host: localhost\nport: 5432\n"));
    }

    #[test]
    fn does_not_sniff_a_single_key_value_line_as_config_ini() {
        assert!(!ConfigIniCore.sniff(b"Note: see below for details.\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_config_ini() {
        assert!(!ConfigIniCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!ConfigIniCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!ConfigIniCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!ConfigIniCore.sniff(b"just a regular line of text\nwith no structure at all\n"));
        assert!(!ConfigIniCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_config_ini_file_and_extracts_sections() {
        let path = unique_temp_file("app.ini");
        std::fs::write(
            &path,
            "; top-level comment\n[core]\neditor = vim\nautocrlf = false\n\n[user]\nname = Ada Lovelace\nemail = ada@example.com\n",
        )
        .unwrap();

        let data = ConfigIniCore.view(&path).unwrap();
        let view: ConfigIniView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.sections, vec!["core", "user"]);
        assert!(view.content.contains("editor = vim"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.ini");
        let mut content = "[core]\n".to_owned();
        content.push_str(&"key = value\n".repeat(MAX_VIEW_BYTES));
        std::fs::write(&path, content).unwrap();

        let data = ConfigIniCore.view(&path).unwrap();
        let view: ConfigIniView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_sections_and_content() {
        let data = serde_json::to_value(ConfigIniView {
            content: "[core]\neditor = vim".to_owned(),
            truncated: false,
            sections: vec!["core".to_owned()],
        })
        .unwrap();

        let lines = ConfigIniPresentation.present(&data);

        assert_eq!(lines, vec!["sections: core", "[core]", "editor = vim"]);
    }
}
