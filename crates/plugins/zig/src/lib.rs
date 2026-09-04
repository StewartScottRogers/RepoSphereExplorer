//! Zig file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`ZigCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZigView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `fn`/`pub fn` declarations found in the content.
    pub functions: Vec<String>,
    /// Names of top-level `const X = struct { ... }` declarations found in
    /// the content.
    pub structs: Vec<String>,
}

/// Extracts the identifier following a `fn `/`pub fn ` keyword at the start
/// of `line`, e.g. `pub fn greet(name: []const u8) void` returns
/// `Some("greet")`.
fn parse_function_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("pub ").unwrap_or(line);
    let rest = rest.strip_prefix("fn ")?;
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Extracts the identifier from a top-level `const X = struct { ... }`
/// (or `pub const X = struct { ... }`) declaration at the start of `line`.
fn parse_struct_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("pub ").unwrap_or(line);
    let rest = rest.strip_prefix("const ")?;
    let (name, value) = rest.split_once(" = ")?;
    value.trim_start().starts_with("struct").then_some(name)
}

/// Parses top-level function and struct names out of `content`, in source
/// order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut structs = Vec::new();
    for line in content.lines() {
        if let Some(name) = parse_struct_name(line) {
            structs.push(name.to_owned());
        } else if let Some(name) = parse_function_name(line) {
            functions.push(name.to_owned());
        }
    }
    (functions, structs)
}

/// Whether `text` looks like Zig source: markers not used by this project's
/// other source-language plugins. `@import(` is Zig's builtin
/// module-import call; `std.debug.print(` is Zig's qualified console-output
/// call, distinct from every sibling plugin's own console-output marker;
/// `comptime ` and `usingnamespace ` are Zig-only keywords; and `anyerror!`
/// / `!void` are Zig's error-union return-type syntax. Real Zig source also
/// commonly matches the Rust plugin's bare `fn `/`pub fn `/`fn main(`
/// checks, so this plugin is placed just ahead of `rust` in `CORE_PLUGINS`
/// so a genuine Zig file's function declarations are reached only after one
/// of these stronger, Zig-only markers has already matched.
fn has_zig_syntax(text: &str) -> bool {
    text.contains("@import(")
        || text.contains("std.debug.print(")
        || text.contains("comptime ")
        || text.contains("usingnamespace ")
        || text.contains("anyerror!")
        || text.contains("!void")
}

/// The Zig plugin's core half.
#[derive(Debug, Default)]
pub struct ZigCore;

impl PluginCore for ZigCore {
    fn name(&self) -> &'static str {
        "zig"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_zig_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, structs) = parse_definitions(&content);
        let view = ZigView {
            content,
            truncated,
            functions,
            structs,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Zig plugin's presentation half.
#[derive(Debug, Default)]
pub struct ZigPresentation;

impl PluginPresentation for ZigPresentation {
    fn name(&self) -> &'static str {
        "zig"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: ZigView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
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
    use super::{MAX_VIEW_BYTES, ZigCore, ZigPresentation, ZigView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-zig-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_zig_markers_as_zig() {
        assert!(ZigCore.sniff(b"const std = @import(\"std\");\n"));
        assert!(ZigCore.sniff(b"pub fn main() void {\n    std.debug.print(\"hi\\n\", .{});\n}\n"));
        assert!(ZigCore.sniff(b"comptime var x: i32 = 0;\n"));
        assert!(ZigCore.sniff(b"usingnamespace @import(\"other.zig\");\n"));
        assert!(ZigCore.sniff(b"pub fn main() anyerror!void {\n}\n"));
        assert!(ZigCore.sniff(b"fn risky() !void {\n}\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_zig() {
        assert!(!ZigCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!ZigCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!ZigCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!ZigCore.sniff(b"fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!ZigCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!ZigCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!ZigCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!ZigCore.sniff(b"void greet() {\n  return;\n}\n"));
        assert!(!ZigCore.sniff(b"defmodule Greeter do\n  def hi, do: :ok\nend\n"));
        assert!(!ZigCore.sniff(b"just a regular line of text\n"));
        assert!(!ZigCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_zig_file_and_extracts_definitions() {
        let path = unique_temp_file("greeter.zig");
        std::fs::write(
            &path,
            "const std = @import(\"std\");\n\nconst Point = struct {\n    x: i32,\n    y: i32,\n};\n\nfn greet(name: []const u8) void {\n    std.debug.print(\"Hello, {s}!\\n\", .{name});\n}\n\npub fn main() void {\n    greet(\"world\");\n}\n",
        )
        .unwrap();

        let data = ZigCore.view(&path).unwrap();
        let view: ZigView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.structs, vec!["Point"]);
        assert_eq!(view.functions, vec!["greet", "main"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.zig");
        let mut content = "const std = @import(\"std\");\n".to_owned();
        content.push_str(&"// ".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = ZigCore.view(&path).unwrap();
        let view: ZigView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_structs_functions_and_content() {
        let data = serde_json::to_value(ZigView {
            content: "const A = struct {};\nfn greet() void {}".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            structs: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = ZigPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "structs: A",
                "functions: greet",
                "const A = struct {};",
                "fn greet() void {}"
            ]
        );
    }
}
