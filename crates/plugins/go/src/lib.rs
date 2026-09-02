//! Go file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`GoCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `func` declarations found in the content
    /// (including methods, keyed by their method name without the
    /// receiver).
    pub functions: Vec<String>,
    /// Names of top-level `type X struct` declarations found in the
    /// content.
    pub structs: Vec<String>,
    /// Names of top-level `type X interface` declarations found in the
    /// content.
    pub interfaces: Vec<String>,
}

/// Extracts the identifier that follows an alphanumeric/underscore run at
/// the start of `text`, if any.
fn leading_identifier(text: &str) -> Option<&str> {
    let end = text
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(text.len());
    (end > 0).then(|| &text[..end])
}

/// Extracts the function or method name from a top-level `func` line, e.g.
/// `func Greet(name string) string {` or
/// `func (g *Greeter) Greet() string {`. The receiver, if present, is
/// skipped.
fn parse_func_name(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("func ")?.trim_start();
    let rest = if let Some(after_receiver) = rest.strip_prefix('(') {
        let close = after_receiver.find(')')?;
        after_receiver[close + 1..].trim_start()
    } else {
        rest
    };
    leading_identifier(rest)
}

/// Extracts the type name from a top-level `type X struct {` or
/// `type X interface {` line, if it declares the given `keyword`.
fn parse_type_name<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.trim_start().strip_prefix("type ")?.trim_start();
    let name = leading_identifier(rest)?;
    rest[name.len()..]
        .trim_start()
        .starts_with(keyword)
        .then_some(name)
}

/// Parses top-level function, struct, and interface names out of
/// `content`, in source order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut structs = Vec::new();
    let mut interfaces = Vec::new();
    for line in content.lines() {
        if let Some(name) = parse_func_name(line) {
            functions.push(name.to_owned());
        } else if let Some(name) = parse_type_name(line, "struct") {
            structs.push(name.to_owned());
        } else if let Some(name) = parse_type_name(line, "interface") {
            interfaces.push(name.to_owned());
        }
    }
    (functions, structs, interfaces)
}

/// Whether `text` looks like Go source: `package` declarations, `func`
/// declarations (plain or with a receiver), and short variable
/// declarations (`:=`) are markers not used by this project's other
/// source-language plugins.
fn has_go_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("package ") || line.starts_with("func ") || line.starts_with("import (")
    }) || text.contains(":=")
        || text.contains("fmt.Println(")
        || text.contains("fmt.Printf(")
}

/// The Go plugin's core half.
#[derive(Debug, Default)]
pub struct GoCore;

impl PluginCore for GoCore {
    fn name(&self) -> &'static str {
        "go"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_go_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, structs, interfaces) = parse_definitions(&content);
        let view = GoView {
            content,
            truncated,
            functions,
            structs,
            interfaces,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Go plugin's presentation half.
#[derive(Debug, Default)]
pub struct GoPresentation;

impl PluginPresentation for GoPresentation {
    fn name(&self) -> &'static str {
        "go"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: GoView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.interfaces.is_empty() {
            lines.push(format!("interfaces: {}", view.interfaces.join(", ")));
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
    use super::{GoCore, GoPresentation, GoView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-go-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_package_or_func_declaration_as_go() {
        assert!(GoCore.sniff(b"package main\n\nfunc main() {}\n"));
        assert!(GoCore.sniff(b"func Greet(name string) string {\n\treturn name\n}\n"));
        assert!(GoCore.sniff(b"func (g *Greeter) Greet() string {\n\treturn g.name\n}\n"));
    }

    #[test]
    fn sniffs_common_go_markers_as_go() {
        assert!(GoCore.sniff(b"count := 0\n"));
        assert!(GoCore.sniff(b"import (\n\t\"fmt\"\n)\n"));
        assert!(GoCore.sniff(b"fmt.Println(\"hi\")\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_go() {
        assert!(!GoCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!GoCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!GoCore.sniff(b"interface Named {\n  name: string;\n}\n"));
        assert!(!GoCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!GoCore.sniff(b"just a regular line of text\n"));
        assert!(!GoCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_go_file_and_extracts_definitions() {
        let path = unique_temp_file("greet.go");
        std::fs::write(
            &path,
            "package main\n\ntype Named interface {\n\tName() string\n}\n\ntype Greeter struct {\n\tname string\n}\n\nfunc (g *Greeter) Name() string {\n\treturn g.name\n}\n\nfunc Greet(person Named) string {\n\treturn \"Hello, \" + person.Name() + \"!\"\n}\n",
        )
        .unwrap();

        let data = GoCore.view(&path).unwrap();
        let view: GoView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.interfaces, vec!["Named"]);
        assert_eq!(view.structs, vec!["Greeter"]);
        assert_eq!(view.functions, vec!["Name", "Greet"]);
        assert!(view.content.contains("Hello, "));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.go");
        let mut content = "func pad() {\n".to_owned();
        content.push_str(&"/".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = GoCore.view(&path).unwrap();
        let view: GoView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_interfaces_structs_functions_and_content() {
        let data = serde_json::to_value(GoView {
            content: "type A struct {\n}".to_owned(),
            truncated: false,
            functions: vec!["Greet".to_owned()],
            structs: vec!["A".to_owned()],
            interfaces: vec!["Named".to_owned()],
        })
        .unwrap();

        let lines = GoPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "interfaces: Named",
                "structs: A",
                "functions: Greet",
                "type A struct {",
                "}"
            ]
        );
    }
}
