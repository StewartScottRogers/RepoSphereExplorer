//! Java properties file type plugin: core and presentation halves.
//!
//! The format Java's `Properties.load` reads: `key=value` or `key:value`,
//! `\` continuing a line, `#` and `!` comments, and `\uXXXX` escapes for
//! anything outside Latin-1. Registered after `ini`, which claims the
//! dialect with `[section]` headers; a properties file has none.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["properties"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One property.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Property {
    /// The key, with its escapes resolved.
    pub key: String,
    /// The value, with continuations joined and escapes resolved.
    pub value: String,
    /// How many physical lines the value spanned.
    pub lines: usize,
}

/// View data produced by [`PropertiesCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropertiesView {
    /// Every property, in file order.
    pub properties: Vec<Property>,
    /// The keys whose values continued across a line break.
    pub continued: Vec<String>,
    /// The keys whose values carried a `\uXXXX` escape.
    pub escaped: Vec<String>,
    /// The keys with an empty value, which Java reads as an empty string
    /// and not as absent.
    pub empty: Vec<String>,
    /// How many comment lines the file carries.
    pub comments: usize,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Whether `line` ends with an odd number of backslashes, and so continues.
fn continues(line: &str) -> bool {
    line.len() - line.trim_end_matches('\\').len() % 2 == line.len()
        && (line.len() - line.trim_end_matches('\\').len()) % 2 == 1
}

/// Resolves `\uXXXX`, `\n`, `\t` and `\:`-style escapes in `text`.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('u') => {
                let digits: String = chars.by_ref().take(4).collect();
                if let Some(resolved) = u32::from_str_radix(&digits, 16)
                    .ok()
                    .and_then(char::from_u32)
                {
                    out.push(resolved);
                } else {
                    // An escape naming no character is left as written:
                    // showing it is more use to a reader than dropping it.
                    out.push_str("\\u");
                    out.push_str(&digits);
                }
            }
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Splits a logical line into its key and value at the first unescaped
/// `=`, `:` or run of whitespace, which is what Java's reader does.
fn split_property(line: &str) -> Option<(String, String)> {
    let chars: Vec<char> = line.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '\\' => index += 1,
            '=' | ':' => {
                let key = chars[..index].iter().collect::<String>();
                let value = chars[index + 1..].iter().collect::<String>();
                return Some((key.trim().to_owned(), value.trim_start().to_owned()));
            }
            c if c.is_whitespace() && index > 0 => {
                let key = chars[..index].iter().collect::<String>();
                let rest = chars[index..].iter().collect::<String>();
                let rest = rest.trim_start();
                let value = rest.strip_prefix(['=', ':']).unwrap_or(rest);
                return Some((key.trim().to_owned(), value.trim_start().to_owned()));
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// Everything [`PropertiesView`] holds, read from `text`.
fn parse(text: &str) -> PropertiesView {
    let mut view = PropertiesView {
        properties: Vec::new(),
        continued: Vec::new(),
        escaped: Vec::new(),
        empty: Vec::new(),
        comments: 0,
        content: String::new(),
        truncated: false,
    };

    let raw_lines: Vec<&str> = text.lines().collect();
    let mut index = 0;
    while index < raw_lines.len() {
        let first = raw_lines[index].trim_start();
        if first.is_empty() {
            index += 1;
            continue;
        }
        if first.starts_with('#') || first.starts_with('!') {
            view.comments += 1;
            index += 1;
            continue;
        }

        let mut logical = String::new();
        let mut spanned = 0usize;
        loop {
            let line = raw_lines[index].trim_start();
            spanned += 1;
            if continues(line) {
                logical.push_str(line.trim_end().trim_end_matches('\\'));
                index += 1;
                if index >= raw_lines.len() {
                    break;
                }
            } else {
                logical.push_str(line);
                index += 1;
                break;
            }
        }

        let Some((key, value)) = split_property(&logical) else {
            continue;
        };
        let key = unescape(&key);
        if spanned > 1 {
            view.continued.push(key.clone());
        }
        if logical.contains("\\u") {
            view.escaped.push(key.clone());
        }
        if value.is_empty() {
            view.empty.push(key.clone());
        }
        view.properties.push(Property {
            key,
            value: unescape(&value),
            lines: spanned,
        });
    }
    view
}

/// Whether `prefix` looks like a properties file: two or more assignments
/// and no `[section]` header, which would make it an INI file instead.
fn looks_like_properties(prefix: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(prefix) else {
        return false;
    };
    let mut assignments = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            return false;
        }
        if trimmed.contains('=') || trimmed.contains(':') {
            assignments += 1;
        }
    }
    assignments >= 2
}

/// The properties plugin's core half.
#[derive(Debug, Default)]
pub struct PropertiesCore;

impl PluginCore for PropertiesCore {
    fn name(&self) -> &'static str {
        "properties"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_properties(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let mut view = parse(&content);
        view.content = content;
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The properties plugin's presentation half.
#[derive(Debug, Default)]
pub struct PropertiesPresentation;

impl PluginPresentation for PropertiesPresentation {
    fn name(&self) -> &'static str {
        "properties"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "PROP",
            tint: 0x00b0_7219,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: PropertiesView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!("{} propert(y/ies)", view.properties.len())];

        for property in &view.properties {
            let value = property.value.replace('\n', "\\n");
            lines.push(format!("  {} = {value}", property.key));
        }

        lines.push(format!("Comments: {}", view.comments));
        if !view.continued.is_empty() {
            lines.push(format!(
                "Continued across lines: {}",
                view.continued.join(", ")
            ));
        }
        if !view.escaped.is_empty() {
            lines.push(format!(
                "Carrying Unicode escapes: {}",
                view.escaped.join(", ")
            ));
        }
        if !view.empty.is_empty() {
            lines.push(format!(
                "Empty, which Java reads as \"\" and not as absent: {}",
                view.empty.join(", ")
            ));
        }
        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{PropertiesCore, PropertiesPresentation, PropertiesView, parse, unescape};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_several_assignments_and_refuses_a_section_header() {
        assert!(PropertiesCore.sniff(b"a=1\nb=2\n"));
        assert!(PropertiesCore.sniff(b"a:1\nb:2\n"));
        assert!(!PropertiesCore.sniff(b"[section]\na=1\nb=2\n"));
        assert!(!PropertiesCore.sniff(b"a=1\n"));
        assert!(!PropertiesCore.sniff(b""));
    }

    #[test]
    fn reads_both_separators_and_the_whitespace_one() {
        let view = parse("equals=1\ncolon:2\nspace 3\n");

        assert_eq!(view.properties.len(), 3);
        assert_eq!(view.properties[0].value, "1");
        assert_eq!(view.properties[1].value, "2");
        assert_eq!(view.properties[2].key, "space");
        assert_eq!(view.properties[2].value, "3");
    }

    #[test]
    fn joins_a_value_that_continues_across_lines() {
        let view = parse("message=one \\\n  two \\\n  three\n");

        assert_eq!(view.properties.len(), 1);
        assert_eq!(view.properties[0].lines, 3);
        assert_eq!(view.continued, vec!["message".to_owned()]);
    }

    #[test]
    fn resolves_unicode_escapes() {
        assert_eq!(unescape("caf\\u00e9"), "café");

        let view = parse("greeting=caf\\u00e9\nother=plain\n");

        assert_eq!(view.properties[0].value, "café");
        assert_eq!(view.escaped, vec!["greeting".to_owned()]);
    }

    #[test]
    fn an_empty_value_is_recorded_rather_than_dropped() {
        let view = parse("blank=\nfilled=1\n");

        assert_eq!(view.empty, vec!["blank".to_owned()]);
        assert_eq!(view.properties.len(), 2);
    }

    #[test]
    fn counts_comments_in_both_styles() {
        let view = parse("# one\n! two\na=1\nb=2\n");

        assert_eq!(view.comments, 2);
    }

    #[test]
    fn presents_the_keys_and_what_is_unusual_about_them() {
        let data = serde_json::to_value(parse("# c\na=1\nblank=\n")).unwrap();

        let lines = PropertiesPresentation.present(&data);

        assert!(lines[0].starts_with("2 propert"));
        assert!(lines.iter().any(|line| line.contains("Comments: 1")));
        assert!(lines.iter().any(|line| line.contains("Empty")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/properties/messages.properties");

        let data = PropertiesCore.view(&path).unwrap();
        let view: PropertiesView = serde_json::from_value(data).unwrap();

        assert!(view.properties.len() >= 6);
        assert!(!view.continued.is_empty());
        assert!(!view.escaped.is_empty());
        assert!(!view.empty.is_empty());
        assert!(view.comments >= 2);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::PropertiesCore),
            plugin_api::PluginPresentation::extensions(&crate::PropertiesPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
