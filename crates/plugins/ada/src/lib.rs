//! Ada file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`AdaCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `procedure`/`function`/`package` (or `package
    /// body`) constructs found via their opening line, in source order.
    pub declarations: Vec<String>,
}

/// Whether `line`, trimmed, opens an Ada subprogram or package construct —
/// `procedure NAME ... is`, `function NAME ... is`, `package NAME is`, or
/// `package body NAME is` — and if so, the construct's name. Ada ends such
/// a line with a bare ` is` (no brace or colon the way C-family/Pascal-like
/// languages would use instead), so this doubles as both the sniff marker
/// and the declaration extractor.
fn ada_construct_name(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.ends_with(" is") {
        return None;
    }
    let keyword_len = if lower.starts_with("package body ") {
        "package body ".len()
    } else if lower.starts_with("procedure ") {
        "procedure ".len()
    } else if lower.starts_with("function ") {
        "function ".len()
    } else if lower.starts_with("package ") {
        "package ".len()
    } else {
        return None;
    };
    let rest = trimmed.get(keyword_len..)?;
    let name: String = rest
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

/// Parses top-level construct names out of `content`, in source order.
fn parse_declarations(content: &str) -> Vec<String> {
    content.lines().filter_map(ada_construct_name).collect()
}

/// Whether `text` looks like Ada source: markers not used by this project's
/// other source-language plugins. `with Ada.` is a with-clause naming the
/// Ada standard library, a vocabulary no other sniffed language shares;
/// `end record;` closes an Ada record type definition; a bare `begin` line
/// opens a subprogram's statement part (distinct from Ruby's own bare `end`
/// check, which never matches Ada's semicolon-terminated `end;`/`end
/// NAME;`); and a `procedure`/`function`/`package` construct header ending
/// in ` is` (see [`ada_construct_name`]) is Ada's own syntax for beginning
/// one of these constructs.
fn has_ada_syntax(text: &str) -> bool {
    text.contains("with Ada.")
        || text.contains("end record;")
        || text
            .lines()
            .any(|line| line.trim() == "begin" || ada_construct_name(line).is_some())
}

/// The Ada plugin's core half.
#[derive(Debug, Default)]
pub struct AdaCore;

impl PluginCore for AdaCore {
    fn name(&self) -> &'static str {
        "ada"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_ada_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let declarations = parse_declarations(&content);
        let view = AdaView {
            content,
            truncated,
            declarations,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Ada plugin's presentation half.
#[derive(Debug, Default)]
pub struct AdaPresentation;

impl PluginPresentation for AdaPresentation {
    fn name(&self) -> &'static str {
        "ada"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: AdaView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.declarations.is_empty() {
            lines.push(format!("declarations: {}", view.declarations.join(", ")));
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
    use super::{AdaCore, AdaPresentation, AdaView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-ada-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_ada_markers_as_ada() {
        assert!(AdaCore.sniff(b"with Ada.Text_IO; use Ada.Text_IO;\n"));
        assert!(AdaCore.sniff(b"procedure Hello is\nbegin\n   Put_Line (\"hi\");\nend Hello;\n"));
        assert!(AdaCore.sniff(
            b"function Add (X, Y : Integer) return Integer is\nbegin\n   return X + Y;\nend Add;\n"
        ));
        assert!(AdaCore.sniff(b"package Greetings is\nend Greetings;\n"));
        assert!(AdaCore.sniff(b"type Point is record\n   X, Y : Integer;\nend record;\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_ada() {
        assert!(!AdaCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!AdaCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!AdaCore.sniff(b"class Greeter\n  def greet\n    puts 'hi'\n  end\nend\n"));
        assert!(!AdaCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!AdaCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!AdaCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!AdaCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!AdaCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!AdaCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!AdaCore.sniff(b"let x : int = 5\n"));
        assert!(!AdaCore.sniff(b"just a regular line of text\n"));
        assert!(!AdaCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_ada_file_and_extracts_declarations() {
        let path = unique_temp_file("hello.adb");
        std::fs::write(
            &path,
            "with Ada.Text_IO; use Ada.Text_IO;\n\nprocedure Hello is\n   function Greeting return String is\n   begin\n      return \"Hello, world!\";\n   end Greeting;\nbegin\n   Put_Line (Greeting);\nend Hello;\n",
        )
        .unwrap();

        let data = AdaCore.view(&path).unwrap();
        let view: AdaView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.declarations, vec!["Hello", "Greeting"]);
        assert!(view.content.contains("Put_Line"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.adb");
        let mut content = "procedure Large is\nbegin\n".to_owned();
        content.push_str(&"   --  padding\n".repeat(MAX_VIEW_BYTES));
        content.push_str("end Large;\n");
        std::fs::write(&path, content).unwrap();

        let data = AdaCore.view(&path).unwrap();
        let view: AdaView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_declarations_and_content() {
        let data = serde_json::to_value(AdaView {
            content: "procedure Hello is\nbegin\n   null;\nend Hello;".to_owned(),
            truncated: false,
            declarations: vec!["Hello".to_owned()],
        })
        .unwrap();

        let lines = AdaPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "declarations: Hello",
                "procedure Hello is",
                "begin",
                "   null;",
                "end Hello;"
            ]
        );
    }
}
