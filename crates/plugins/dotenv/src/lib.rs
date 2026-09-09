//! dotenv file type plugin: core and presentation halves.
//!
//! Recognised by name as well as by shape. The point of the view is the
//! keys, never the values: a `.env` is where secrets live, so what it
//! reports is which keys look like credentials, not what they hold.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["env"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Variable {
    /// The name on the left of the `=`.
    pub name: String,
    /// Whether the value was quoted.
    pub quoted: bool,
    /// Whether the value refers to another variable, as `${OTHER}`.
    pub interpolated: bool,
    /// Whether the line carried an `export ` prefix.
    pub exported: bool,
    /// How many physical lines the value spanned.
    pub lines: usize,
}

/// View data produced by [`DotenvCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DotenvView {
    /// Every variable, in file order. **Names only**: the values are
    /// deliberately not carried, because this is the file secrets live in.
    pub variables: Vec<Variable>,
    /// The names whose spelling says they hold a credential.
    pub secret_looking: Vec<String>,
    /// How many comment lines the file carries.
    pub comments: usize,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The words that make a name look like a credential.
const SECRET_WORDS: &[&str] = &[
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "TOKEN",
    "KEY",
    "CREDENTIAL",
    "PRIVATE",
    "AUTH",
    "SIGNING",
];

/// Everything [`DotenvView`] holds, read from `text`.
fn parse(text: &str) -> DotenvView {
    let mut view = DotenvView {
        variables: Vec::new(),
        secret_looking: Vec::new(),
        comments: 0,
        truncated: false,
    };

    let lines: Vec<&str> = text.lines().collect();
    let mut index = 0usize;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        index += 1;
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            view.comments += 1;
            continue;
        }
        let stripped = trimmed.strip_prefix("export ").map(str::trim);
        let exported = stripped.is_some();
        let assignment = stripped.unwrap_or(trimmed);
        let Some((name, value)) = assignment.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || name.contains(char::is_whitespace) {
            continue;
        }
        let value = value.trim();
        let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'');
        let mut spanned = 1usize;
        let mut whole = value.to_owned();
        // A quoted value may run to a closing quote on a later line.
        if let Some(quote) = quote
            && !(value.len() >= 2 && value.ends_with(quote))
        {
            while index < lines.len() {
                whole.push('\n');
                whole.push_str(lines[index]);
                spanned += 1;
                let closed = lines[index].trim_end().ends_with(quote);
                index += 1;
                if closed {
                    break;
                }
            }
        }

        let upper = name.to_uppercase();
        if SECRET_WORDS.iter().any(|word| upper.contains(word)) {
            view.secret_looking.push(name.to_owned());
        }
        view.variables.push(Variable {
            name: name.to_owned(),
            quoted: quote.is_some(),
            interpolated: whole.contains("${") || whole.contains("$("),
            exported,
            lines: spanned,
        });
    }
    view
}

/// Whether `text` looks like a dotenv file: two or more `NAME=value`
/// assignments whose names are shouted, which is the convention every
/// example follows and which keeps this off ordinary shell scripts.
fn looks_like_it(text: &str) -> bool {
    // A shebang means the file is meant to be executed, which an
    // environment file never is. The shell plugin claims it first in
    // practice; this keeps the sniff honest on its own.
    if text.starts_with("#!") {
        return false;
    }
    let mut shouted = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let assignment = trimmed.strip_prefix("export ").unwrap_or(trimmed).trim();
        let Some((name, _)) = assignment.split_once('=') else {
            // A quoted value continues onto lines carrying no `=` at all.
            // Skipping them is what lets a file describe itself across
            // more than one line and still be recognised.
            continue;
        };
        let name = name.trim();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            return false;
        }
        shouted += 1;
    }
    shouted >= 2
}

/// The dotenv plugin's core half.
#[derive(Debug, Default)]
pub struct DotenvCore;

impl PluginCore for DotenvCore {
    fn name(&self) -> &'static str {
        "dotenv"
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
        // The text is read, parsed and dropped. Unlike every other text
        // plugin, this one does not carry `content` on the view: a `.env`
        // is where secrets live, and a view that held the file would put
        // them on the wire, in the pane and in any log of either.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The dotenv plugin's presentation half.
#[derive(Debug, Default)]
pub struct DotenvPresentation;

impl PluginPresentation for DotenvPresentation {
    fn name(&self) -> &'static str {
        "dotenv"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "ENV",
            tint: 0x00ec_d53f,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: DotenvView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("{} variable(s)", view.variables.len()));
        lines.push("Names only. This is the file secrets live in, so the".to_owned());
        lines.push("values are not read into the view at all.".to_owned());
        for variable in &view.variables {
            let mut notes = Vec::new();
            if variable.exported {
                notes.push("exported");
            }
            if variable.quoted {
                notes.push("quoted");
            }
            if variable.interpolated {
                notes.push("interpolated");
            }
            if variable.lines > 1 {
                notes.push("multi-line");
            }
            let suffix = if notes.is_empty() {
                String::new()
            } else {
                format!("  ({})", notes.join(", "))
            };
            lines.push(format!("  {}{suffix}", variable.name));
        }
        lines.push(format!("Comments: {}", view.comments));
        if !view.secret_looking.is_empty() {
            lines.push(format!(
                "Named like credentials, so check this file is ignored: {}",
                view.secret_looking.join(", ")
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
    use super::{DotenvCore, DotenvPresentation, DotenvView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_shouted_assignments() {
        assert!(DotenvCore.sniff(b"DATABASE_URL=postgres://x\nRUST_LOG=info\n"));
        assert!(DotenvCore.sniff(b"export A=1\nexport B=2\n"));
    }

    #[test]
    fn does_not_claim_a_shell_script() {
        assert!(!DotenvCore.sniff(b"name=widgets\ncount=3\n"));
        assert!(!DotenvCore.sniff(b"#!/bin/sh\nA=1\nB=2\n"));
        assert!(!DotenvCore.sniff(b""));
    }

    #[test]
    fn the_view_carries_names_and_never_values() {
        let view = parse("SECRET_KEY=hunter2\n");

        let rendered = serde_json::to_string(&view).unwrap();

        assert!(rendered.contains("SECRET_KEY"));
        assert!(
            !rendered.contains("hunter2"),
            "a secret must not reach the wire: {rendered}"
        );
    }

    #[test]
    fn notices_quoting_interpolation_and_export() {
        let view = parse("export A=\"one\"\nB=${A}/two\nC=three\n");

        assert!(view.variables[0].exported);
        assert!(view.variables[0].quoted);
        assert!(view.variables[1].interpolated);
        assert!(!view.variables[2].quoted);
    }

    #[test]
    fn a_quoted_value_may_run_across_lines() {
        let view = parse("KEY=\"line one\nline two\"\nNEXT=1\n");

        assert_eq!(view.variables.len(), 2);
        assert_eq!(view.variables[0].lines, 2);
        assert_eq!(view.variables[1].name, "NEXT");
    }

    #[test]
    fn flags_the_names_that_look_like_credentials() {
        let view = parse("API_TOKEN=x\nDB_PASSWORD=y\nRUST_LOG=info\n");

        assert_eq!(
            view.secret_looking,
            vec!["API_TOKEN".to_owned(), "DB_PASSWORD".to_owned()]
        );
    }

    #[test]
    fn presents_the_warning_about_values() {
        let data = serde_json::to_value(parse("A=1\nAPI_KEY=2\n")).unwrap();

        let lines = DotenvPresentation.present(&data);

        assert!(lines.iter().any(|line| line.contains("secrets live in")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Named like credentials"))
        );
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/dotenv/example.env");

        let data = DotenvCore.view(&path).unwrap();
        let view: DotenvView = serde_json::from_value(data).unwrap();

        assert!(view.variables.len() >= 6);
        assert!(view.variables.iter().any(|v| v.quoted));
        assert!(view.variables.iter().any(|v| v.interpolated));
        assert!(view.variables.iter().any(|v| v.exported));
        assert!(view.variables.iter().any(|v| v.lines > 1));
        assert!(!view.secret_looking.is_empty());
        assert!(view.comments >= 2);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::DotenvCore),
            plugin_api::PluginPresentation::extensions(&crate::DotenvPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
