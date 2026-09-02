//! Rust file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`RustCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RustView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `fn` declarations found in the content.
    pub functions: Vec<String>,
    /// Names of top-level `struct` declarations found in the content.
    pub structs: Vec<String>,
    /// Names of top-level `trait` declarations found in the content.
    pub traits: Vec<String>,
}

/// Extracts the identifier following `keyword` (`"fn"`, `"struct"`, or
/// `"trait"`) at the start of `line`, if present. A leading `pub ` is
/// stripped first, so `pub fn greet` is recognised the same as `fn greet`.
fn top_level_name<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let line = line.strip_prefix("pub ").unwrap_or(line);
    let rest = line.strip_prefix(keyword)?.strip_prefix(' ')?;
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level function, struct, and trait names out of `content`, in
/// source order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut structs = Vec::new();
    let mut traits = Vec::new();
    for line in content.lines() {
        if let Some(name) = top_level_name(line, "fn") {
            functions.push(name.to_owned());
        } else if let Some(name) = top_level_name(line, "struct") {
            structs.push(name.to_owned());
        } else if let Some(name) = top_level_name(line, "trait") {
            traits.push(name.to_owned());
        }
    }
    (functions, structs, traits)
}

/// Whether `text` looks like Rust source: keywords and markers not used by
/// this project's other source-language plugins (Python's `def`/`class`,
/// JavaScript's `function`/`=>`/`CommonJS` and ES-module markers, and
/// TypeScript's type annotations and `implements`/visibility modifiers).
fn has_rust_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("fn ")
            || line.starts_with("pub fn ")
            || line.starts_with("struct ")
            || line.starts_with("pub struct ")
            || line.starts_with("enum ")
            || line.starts_with("pub enum ")
            || line.starts_with("trait ")
            || line.starts_with("pub trait ")
            || line.starts_with("impl ")
    }) || text.contains("fn main(")
        || text.contains("let mut ")
        || text.contains("println!(")
        || text.contains("#[derive(")
        || text.contains("use std::")
}

/// The Rust plugin's core half.
#[derive(Debug, Default)]
pub struct RustCore;

impl PluginCore for RustCore {
    fn name(&self) -> &'static str {
        "rust"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_rust_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, structs, traits) = parse_definitions(&content);
        let view = RustView {
            content,
            truncated,
            functions,
            structs,
            traits,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Rust plugin's presentation half.
#[derive(Debug, Default)]
pub struct RustPresentation;

impl PluginPresentation for RustPresentation {
    fn name(&self) -> &'static str {
        "rust"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: RustView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.traits.is_empty() {
            lines.push(format!("traits: {}", view.traits.join(", ")));
        }
        if !view.structs.is_empty() {
            lines.push(format!("structs: {}", view.structs.join(", ")));
        }
        if !view.functions.is_empty() {
            lines.push(format!("functions: {}", view.functions.join(", ")));
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
    use super::{MAX_VIEW_BYTES, RustCore, RustPresentation, RustView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-rust-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_top_level_fn_struct_or_trait_as_rust() {
        assert!(RustCore.sniff(b"fn main() {\n    println!(\"hi\");\n}\n"));
        assert!(RustCore.sniff(b"pub struct Point {\n    x: i32,\n}\n"));
        assert!(RustCore.sniff(b"pub trait Named {\n    fn name(&self) -> String;\n}\n"));
    }

    #[test]
    fn sniffs_common_rust_markers_as_rust() {
        assert!(RustCore.sniff(b"impl Named for Point {}\n"));
        assert!(RustCore.sniff(b"let mut count = 0;\n"));
        assert!(RustCore.sniff(b"#[derive(Debug)]\nstruct Point;\n"));
        assert!(RustCore.sniff(b"use std::collections::HashMap;\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_rust() {
        assert!(!RustCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!RustCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!RustCore.sniff(b"interface Named {\n  name: string;\n}\n"));
        assert!(!RustCore.sniff(b"just a regular line of text\n"));
        assert!(!RustCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_rust_file_and_extracts_definitions() {
        let path = unique_temp_file("greet.rs");
        std::fs::write(
            &path,
            "pub trait Named {\n    fn name(&self) -> String;\n}\n\n\npub struct Greeter {\n    pub name: String,\n}\n\n\nimpl Named for Greeter {\n    fn name(&self) -> String {\n        self.name.clone()\n    }\n}\n\n\npub fn greet(person: &dyn Named) -> String {\n    format!(\"Hello, {}!\", person.name())\n}\n",
        )
        .unwrap();

        let data = RustCore.view(&path).unwrap();
        let view: RustView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.traits, vec!["Named"]);
        assert_eq!(view.structs, vec!["Greeter"]);
        assert_eq!(view.functions, vec!["greet"]);
        assert!(view.content.contains("Hello, {}!"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.rs");
        let mut content = "fn pad() {\n".to_owned();
        content.push_str(&"/".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = RustCore.view(&path).unwrap();
        let view: RustView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_traits_structs_functions_and_content() {
        let data = serde_json::to_value(RustView {
            content: "struct A;".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            structs: vec!["A".to_owned()],
            traits: vec!["Named".to_owned()],
        })
        .unwrap();

        let lines = RustPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "traits: Named",
                "structs: A",
                "functions: greet",
                "struct A;"
            ]
        );
    }
}
