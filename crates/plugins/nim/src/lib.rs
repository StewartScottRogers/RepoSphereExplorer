//! Nim file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`NimCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NimView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `proc` declarations found in the content, in
    /// source order.
    pub procedures: Vec<String>,
}

/// Whether `line` is a top-level Nim procedure declaration, e.g.
/// `proc greet(name: string): string =` or `proc greet*: string =`. A
/// forward declaration without a body (no trailing `=`) is not matched.
fn is_proc_declaration(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("proc ") && trimmed.ends_with('=')
}

/// Extracts the name from a top-level `proc` declaration line, stopping at
/// the first of `(`, `:`, `*`, or whitespace, so both parameterised
/// (`proc greet(name: string) =`) and exported (`proc greet*: string =`)
/// forms yield the bare procedure name.
fn parse_proc_name(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let before_eq = trimmed.strip_suffix('=')?.trim_end();
    let rest = before_eq.strip_prefix("proc ")?;
    let end = rest
        .find(|ch: char| matches!(ch, '(' | ':' | '*') || ch.is_whitespace())
        .unwrap_or(rest.len());
    let name = &rest[..end];
    (!name.is_empty()).then_some(name)
}

/// Parses top-level `proc` declaration names out of `content`, in source
/// order.
fn parse_procedures(content: &str) -> Vec<String> {
    content
        .lines()
        .filter(|line| is_proc_declaration(line))
        .filter_map(parse_proc_name)
        .map(str::to_owned)
        .collect()
}

/// Whether `text`'s first line is a shebang naming the `nim` interpreter,
/// e.g. `#!/usr/bin/env -S nim r`.
fn has_nim_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with("#!") && line.contains("nim"))
}

/// Whether `text` looks like Nim source: markers not used by this project's
/// other source-language plugins. `import std/` is Nim's standard-library
/// module path prefix; `{.` opens a Nim compiler pragma (e.g.
/// `{.push checks: off.}`), a construct no sibling plugin's own brace
/// checks overlap with; and a top-level `proc` declaration ending the line
/// with `=` (see [`is_proc_declaration`]) is Nim's procedure syntax, with
/// no equivalent keyword in this project's other source-language plugins.
fn has_nim_syntax(text: &str) -> bool {
    text.contains("import std/") || text.contains("{.") || text.lines().any(is_proc_declaration)
}

/// The Nim plugin's core half.
#[derive(Debug, Default)]
pub struct NimCore;

impl PluginCore for NimCore {
    fn name(&self) -> &'static str {
        "nim"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_nim_shebang(text) || has_nim_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let procedures = parse_procedures(&content);
        let view = NimView {
            content,
            truncated,
            procedures,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Nim plugin's presentation half.
#[derive(Debug, Default)]
pub struct NimPresentation;

impl PluginPresentation for NimPresentation {
    fn name(&self) -> &'static str {
        "nim"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: NimView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.procedures.is_empty() {
            lines.push(format!("procedures: {}", view.procedures.join(", ")));
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
    use super::{MAX_VIEW_BYTES, NimCore, NimPresentation, NimView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-nim-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_nim_shebang_line_as_nim() {
        assert!(NimCore.sniff(b"#!/usr/bin/env -S nim r\necho \"hi\"\n"));
    }

    #[test]
    fn sniffs_common_nim_markers_as_nim() {
        assert!(NimCore.sniff(b"import std/strutils\n"));
        assert!(NimCore.sniff(b"{.push checks: off.}\n"));
        assert!(NimCore.sniff(b"proc greet(name: string): string =\n  result = name\n"));
        assert!(NimCore.sniff(b"proc greet*: string =\n  result = \"hi\"\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_nim() {
        assert!(!NimCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!NimCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!NimCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!NimCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!NimCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!NimCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!NimCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!NimCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!NimCore.sniff(
            b"import kotlin.math.max\n\ndata class Point(val x: Int, val y: Int)\n\nfun main(args: Array<String>) {\n    println(\"hi\")\n}\n"
        ));
        assert!(!NimCore.sniff(b"import scala.collection.mutable.ListBuffer\n"));
        assert!(!NimCore.sniff(
            b"using System;\n\nclass Program {\n    static void Main() {\n        Console.WriteLine(\"hi\");\n    }\n}\n"
        ));
        assert!(!NimCore.sniff(b"use strict;\nuse warnings;\n\nsub greet {\n    return 1;\n}\n"));
        assert!(!NimCore.sniff(b"def greet() {\n    println 'hi'\n}\n"));
        assert!(!NimCore.sniff(b"just a regular line of text\n"));
        assert!(!NimCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_nim_file_and_extracts_procedures() {
        let path = unique_temp_file("greeter.nim");
        std::fs::write(
            &path,
            "import std/strutils\n\nproc greet(name: string): string =\n  result = \"Hello, \" & name.capitalizeAscii() & \"!\"\n\nproc farewell*: string =\n  result = \"Bye!\"\n\necho greet(\"world\")\n",
        )
        .unwrap();

        let data = NimCore.view(&path).unwrap();
        let view: NimView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.procedures, vec!["greet", "farewell"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.nim");
        let mut content = "proc large*: void =\n".to_owned();
        content.push_str(&"# ".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = NimCore.view(&path).unwrap();
        let view: NimView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_procedures_and_content() {
        let data = serde_json::to_value(NimView {
            content: "proc greet*: void =\n  discard".to_owned(),
            truncated: false,
            procedures: vec!["greet".to_owned()],
        })
        .unwrap();

        let lines = NimPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["procedures: greet", "proc greet*: void =", "  discard"]
        );
    }
}
