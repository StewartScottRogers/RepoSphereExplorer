//! `EditorConfig` file type plugin: core and presentation halves.
//!
//! Registered before `ini`, whose dialect this is a narrow subset of: a
//! `root = true` declaration or the `indent_style`/`indent_size` keys are
//! markers no general INI file carries.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["editorconfig"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One `[glob]` section and the properties it sets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// The glob between the brackets.
    pub glob: String,
    /// The properties it sets, as `name = value`, in order.
    pub properties: Vec<String>,
}

/// View data produced by [`EditorconfigCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorconfigView {
    /// Whether the file declares itself the root, which stops the search
    /// walking further up the tree.
    pub root: bool,
    /// Every glob section, in order. Later sections win over earlier ones.
    pub rules: Vec<Rule>,
    /// Every property name set anywhere in the file, each once.
    pub properties: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The properties `EditorConfig` defines, which is what tells this dialect
/// from a general INI file.
const KNOWN: &[&str] = &[
    "indent_style",
    "indent_size",
    "tab_width",
    "end_of_line",
    "charset",
    "trim_trailing_whitespace",
    "insert_final_newline",
    "max_line_length",
    "root",
];

/// Everything [`EditorconfigView`] holds, read from `text`.
fn parse(text: &str) -> EditorconfigView {
    let mut view = EditorconfigView {
        root: false,
        rules: Vec::new(),
        properties: Vec::new(),
        content: String::new(),
        truncated: false,
    };
    let mut current: Option<Rule> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        if let Some(glob) = trimmed.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            if let Some(rule) = current.take() {
                view.rules.push(rule);
            }
            current = Some(Rule {
                glob: glob.to_owned(),
                properties: Vec::new(),
            });
            continue;
        }
        let Some((name, value)) = trimmed.split_once('=') else {
            continue;
        };
        let name = name.trim().to_lowercase();
        let value = value.trim();
        if name == "root" && current.is_none() {
            view.root = value.eq_ignore_ascii_case("true");
        }
        if !view.properties.contains(&name) {
            view.properties.push(name.clone());
        }
        if let Some(rule) = current.as_mut() {
            rule.properties.push(format!("{name} = {value}"));
        }
    }
    if let Some(rule) = current {
        view.rules.push(rule);
    }
    view
}

/// Whether `text` looks like an `EditorConfig` file.
fn looks_like_it(text: &str) -> bool {
    let mut root = false;
    let mut known = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        let Some((name, value)) = trimmed.split_once('=') else {
            continue;
        };
        let name = name.trim().to_lowercase();
        if name == "root" && value.trim().eq_ignore_ascii_case("true") {
            root = true;
        }
        if KNOWN.contains(&name.as_str()) {
            known += 1;
        }
    }
    root || known >= 2
}

/// The `EditorConfig` plugin's core half.
#[derive(Debug, Default)]
pub struct EditorconfigCore;

impl PluginCore for EditorconfigCore {
    fn name(&self) -> &'static str {
        "editorconfig"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
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

/// The `EditorConfig` plugin's presentation half.
#[derive(Debug, Default)]
pub struct EditorconfigPresentation;

impl PluginPresentation for EditorconfigPresentation {
    fn name(&self) -> &'static str {
        "editorconfig"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "EDIT",
            tint: 0x00fe_fefe,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: EditorconfigView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(if view.root {
            "Root: yes - the search for further files stops here".to_owned()
        } else {
            "Root: no - files further up the tree still apply".to_owned()
        });
        lines.push(format!("{} rule(s), later ones winning:", view.rules.len()));
        for rule in &view.rules {
            lines.push(format!("  [{}]", rule.glob));
            for property in &rule.properties {
                lines.push(format!("    {property}"));
            }
        }
        lines.push(format!("Properties set: {}", view.properties.join(", ")));

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{EditorconfigCore, EditorconfigPresentation, EditorconfigView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_a_root_declaration_or_its_own_properties() {
        assert!(EditorconfigCore.sniff(b"root = true\n"));
        assert!(EditorconfigCore.sniff(b"[*]\nindent_style = space\nindent_size = 4\n"));
    }

    #[test]
    fn does_not_claim_a_general_ini_file() {
        assert!(!EditorconfigCore.sniff(b"[server]\nport = 8080\nhost = localhost\n"));
        assert!(!EditorconfigCore.sniff(b""));
    }

    #[test]
    fn reads_the_root_declaration_only_above_the_first_section() {
        let view = parse("root = true\n[*]\nindent_size = 2\n");

        assert!(view.root);

        // A `root` inside a section is a property of that section, not a
        // declaration about the file.
        let inner = parse("[*]\nroot = true\n");

        assert!(!inner.root);
    }

    #[test]
    fn reads_each_glob_with_the_properties_it_sets() {
        let view = parse(
            "root = true\n\n[*]\ncharset = utf-8\n\n[*.rs]\nindent_size = 4\n\
             \n[*.{json,yml}]\nindent_size = 2\n",
        );

        assert_eq!(view.rules.len(), 3);
        assert_eq!(view.rules[0].glob, "*");
        assert_eq!(view.rules[2].glob, "*.{json,yml}");
        assert_eq!(view.rules[1].properties, vec!["indent_size = 4".to_owned()]);
    }

    #[test]
    fn a_comment_sets_nothing() {
        let view = parse("# indent_size = 99\n[*]\nindent_size = 2\n");

        assert_eq!(view.rules[0].properties.len(), 1);
    }

    #[test]
    fn presents_the_root_state_first() {
        let data = serde_json::to_value(parse("root = true\n[*]\ncharset = utf-8\n")).unwrap();

        let lines = EditorconfigPresentation.present(&data);

        assert!(lines[0].starts_with("Root: yes"));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/editorconfig/.editorconfig");

        let data = EditorconfigCore.view(&path).unwrap();
        let view: EditorconfigView = serde_json::from_value(data).unwrap();

        assert!(view.root);
        assert!(view.rules.len() >= 4);
        assert!(view.properties.len() >= 5);
        assert!(view.rules.iter().any(|rule| rule.properties.len() >= 3));
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::EditorconfigCore),
            plugin_api::PluginPresentation::extensions(&crate::EditorconfigPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
