//! INI configuration file type plugin: core and presentation halves.
//!
//! Registered after `toml`, which is the stricter reader of the same
//! shape: TOML insists on quoted strings, booleans or dates on the right
//! of an assignment, and an INI file that satisfies that is a TOML file.
//! What reaches here is the looser dialect - bare values, `;` comments,
//! duplicate keys - that no parser agrees on and every application has its
//! own version of.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
///
/// `conf` is deliberately not among them. An extension is a hint that
/// chooses between plugins which all recognised the content
/// (GUIDANCE.md section 3.3), and `.conf` is worn by nginx, Apache,
/// systemd and a dozen others, so as a hint it points nowhere. A
/// `.conf` file that really is in this dialect still lands here, by
/// being sniffed rather than by being named.
pub const EXTENSIONS: &[&str] = &["ini", "cfg", "inf"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One `[section]` and what it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    /// The name between the brackets. Empty for the keys that appear
    /// before any section header at all.
    pub name: String,
    /// The keys in this section, in file order, including repeats.
    pub keys: Vec<String>,
    /// How many of its values are quoted.
    pub quoted: usize,
}

/// View data produced by [`IniCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IniView {
    /// Every section, in order. The first has an empty name when the file
    /// opens with keys before any header.
    pub sections: Vec<Section>,
    /// Keys that appear more than once in the same section, which most
    /// readers resolve last-wins and no two agree on.
    pub duplicates: Vec<String>,
    /// How many comment lines the file carries.
    pub comments: usize,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The name inside a `[section]` header, if `line` is one.
fn section_header(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    // `[[a]]` is TOML's array of tables, not an INI section.
    if inner.starts_with('[') || inner.is_empty() {
        return None;
    }
    Some(inner.trim().to_owned())
}

/// The key and value of an assignment, if `line` is one.
fn assignment(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
        return None;
    }
    let (key, value) = trimmed
        .split_once('=')
        .or_else(|| trimmed.split_once(':'))?;
    let key = key.trim();
    if key.is_empty() || key.contains('[') {
        return None;
    }
    Some((key.to_owned(), value.trim().to_owned()))
}

/// Whether `line` is a comment.
fn is_comment(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with(';') || trimmed.starts_with('#')
}

/// Everything [`IniView`] holds, read from `text`.
fn parse(text: &str) -> IniView {
    let mut view = IniView {
        sections: Vec::new(),
        duplicates: Vec::new(),
        comments: 0,
        content: String::new(),
        truncated: false,
    };
    let mut current = Section {
        name: String::new(),
        keys: Vec::new(),
        quoted: 0,
    };

    for line in text.lines() {
        if is_comment(line) {
            view.comments += 1;
            continue;
        }
        if let Some(name) = section_header(line) {
            if !current.name.is_empty() || !current.keys.is_empty() {
                view.sections.push(std::mem::replace(
                    &mut current,
                    Section {
                        name: name.clone(),
                        keys: Vec::new(),
                        quoted: 0,
                    },
                ));
            } else {
                current.name = name;
            }
            continue;
        }
        if let Some((key, value)) = assignment(line) {
            if current.keys.contains(&key) && !view.duplicates.contains(&key) {
                view.duplicates.push(key.clone());
            }
            if (value.starts_with('"') && value.ends_with('"') && value.len() >= 2)
                || (value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2)
            {
                current.quoted += 1;
            }
            current.keys.push(key);
        }
    }

    if !current.name.is_empty() || !current.keys.is_empty() {
        view.sections.push(current);
    }
    view
}

/// Whether `prefix` looks like an INI file: a section header, or two or
/// more assignments. One assignment on its own is every other
/// configuration format there has ever been.
fn looks_like_ini(prefix: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(prefix) else {
        return false;
    };
    let mut assignments = 0usize;
    for line in text.lines() {
        if is_comment(line) {
            continue;
        }
        if section_header(line).is_some() {
            return true;
        }
        if assignment(line).is_some() {
            assignments += 1;
        }
    }
    assignments >= 2
}

/// The INI plugin's core half.
#[derive(Debug, Default)]
pub struct IniCore;

impl PluginCore for IniCore {
    fn name(&self) -> &'static str {
        "ini"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_ini(prefix)
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

/// The INI plugin's presentation half.
#[derive(Debug, Default)]
pub struct IniPresentation;

impl PluginPresentation for IniPresentation {
    fn name(&self) -> &'static str {
        "ini"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "INI",
            tint: 0x006d_6d6d,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: IniView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!("{} section(s)", view.sections.len())];

        for section in &view.sections {
            let name = if section.name.is_empty() {
                "(before any section)"
            } else {
                &section.name
            };
            lines.push(format!(
                "[{name}] - {} key(s), {} quoted",
                section.keys.len(),
                section.quoted
            ));
            for key in &section.keys {
                lines.push(format!("  {key}"));
            }
        }

        lines.push(format!("Comments: {}", view.comments));
        if !view.duplicates.is_empty() {
            lines.push(format!(
                "Repeated keys, which readers resolve differently: {}",
                view.duplicates.join(", ")
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
    use super::{IniCore, IniPresentation, IniView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_a_section_header_or_several_assignments() {
        assert!(IniCore.sniff(b"[server]\nport=8080\n"));
        assert!(IniCore.sniff(b"port=8080\nhost=localhost\n"));
    }

    #[test]
    fn does_not_claim_a_single_assignment_or_an_array_of_tables() {
        assert!(!IniCore.sniff(b"port=8080\n"));
        assert!(!IniCore.sniff(b"[[bin]]\n"));
        assert!(!IniCore.sniff(b"just prose\n"));
        assert!(!IniCore.sniff(b""));
    }

    #[test]
    fn keys_before_any_header_get_a_section_of_their_own() {
        let view = parse("global=1\nother=2\n[named]\nkey=3\n");

        assert_eq!(view.sections.len(), 2);
        assert_eq!(view.sections[0].name, "");
        assert_eq!(view.sections[0].keys.len(), 2);
        assert_eq!(view.sections[1].name, "named");
    }

    #[test]
    fn counts_comments_in_both_styles() {
        let view = parse("; one\n# two\n[a]\nk=v\n");

        assert_eq!(view.comments, 2);
    }

    #[test]
    fn reports_a_repeated_key_rather_than_silently_taking_the_last() {
        let view = parse("[a]\nk=1\nk=2\n");

        assert_eq!(view.duplicates, vec!["k".to_owned()]);
        assert_eq!(view.sections[0].keys.len(), 2);
    }

    #[test]
    fn counts_quoted_values() {
        let view = parse("[a]\nplain=1\nquoted=\"two\"\nsingle='three'\n");

        assert_eq!(view.sections[0].quoted, 2);
    }

    #[test]
    fn presents_sections_keys_and_repeats() {
        let data = serde_json::to_value(parse("; c\n[a]\nk=1\nk=2\n")).unwrap();

        let lines = IniPresentation.present(&data);

        assert_eq!(lines[0], "1 section(s)");
        assert!(lines.iter().any(|line| line.starts_with("[a]")));
        assert!(lines.iter().any(|line| line.contains("Repeated keys")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/ini/service.ini");

        let data = IniCore.view(&path).unwrap();
        let view: IniView = serde_json::from_value(data).unwrap();

        assert!(view.sections.len() >= 3);
        assert!(view.sections.iter().any(|section| section.name.is_empty()));
        assert!(view.sections.iter().any(|section| section.quoted > 0));
        assert!(!view.duplicates.is_empty());
        assert!(view.comments >= 2);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::IniCore),
            plugin_api::PluginPresentation::extensions(&crate::IniPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
