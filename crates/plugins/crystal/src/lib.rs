//! Crystal file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`CrystalCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrystalView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `def` method definitions found in the content.
    pub functions: Vec<String>,
    /// Names of top-level `class`/`struct`/`module` declarations found in
    /// the content.
    pub types: Vec<String>,
}

/// Extracts the identifier that follows `keyword` at the start of `line`,
/// e.g. `top_level_name("def greet", "def")` returns `Some("greet")`.
/// Crystal method names may end in `?`, `!`, or `=`, which are included in
/// the match; a trailing type parameter list or return-type annotation is
/// excluded since those characters stop the match.
fn top_level_name<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(keyword)?.strip_prefix(' ')?;
    let end = rest
        .find(|ch: char| {
            !(ch.is_alphanumeric() || ch == '_' || ch == '?' || ch == '!' || ch == '=')
        })
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level method and type names out of `content`, in source
/// order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut types = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        if let Some(name) = top_level_name(trimmed, "class")
            .or_else(|| top_level_name(trimmed, "struct"))
            .or_else(|| top_level_name(trimmed, "module"))
        {
            types.push(name.to_owned());
        } else if let Some(name) = top_level_name(trimmed, "def") {
            functions.push(name.to_owned());
        }
    }
    (functions, types)
}

/// Whether `text`'s first line is a shebang naming the `crystal` interpreter.
fn has_crystal_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with("#!") && line.contains("crystal"))
}

/// Whether `text` looks like Crystal source: markers not used by this
/// project's other source-language plugins, in particular `ruby` (whose
/// syntax Crystal otherwise closely resembles), `rust`/`julia` (which also
/// sniff on a bare `struct ` keyword), `nim` (which also has a `macro `
/// keyword), and `kotlin` (which also uses a bare `fun ` keyword) — so
/// none of `struct `, `macro `, or `fun ` alone are checked here.
/// `getter`/`setter`/`property` are Crystal's built-in property macros,
/// distinct from Ruby's `attr_accessor`/`attr_reader`/`attr_writer` and
/// from Objective-C's `@property`; `lib ` at top level is Crystal's C
/// binding block, a keyword the other plugins here don't use;
/// `uninitialized` and `pointerof(` are Crystal-only low-level features;
/// a leading `@[` is a Crystal annotation, distinct from `#[` (Rust) and
/// `@Name` (Java/Kotlin/Scala). This plugin is placed just ahead of
/// `ruby` in `CORE_PLUGINS` so it claims Crystal files before Ruby's
/// broader `def`/`end`/`class` markers would.
fn has_crystal_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("lib ")
            || trimmed.starts_with("getter ")
            || trimmed.starts_with("getter!")
            || trimmed.starts_with("getter?")
            || trimmed.starts_with("setter ")
            || trimmed.starts_with("property ")
            || trimmed.starts_with("property!")
            || trimmed.starts_with("property?")
            || trimmed.starts_with("@[")
    }) || text.contains("uninitialized")
        || text.contains("pointerof(")
}

/// The Crystal plugin's core half.
#[derive(Debug, Default)]
pub struct CrystalCore;

impl PluginCore for CrystalCore {
    fn name(&self) -> &'static str {
        "crystal"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_crystal_shebang(text) || has_crystal_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, types) = parse_definitions(&content);
        let view = CrystalView {
            content,
            truncated,
            functions,
            types,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Crystal plugin's presentation half.
#[derive(Debug, Default)]
pub struct CrystalPresentation;

impl PluginPresentation for CrystalPresentation {
    fn name(&self) -> &'static str {
        "crystal"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: CrystalView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.types.is_empty() {
            lines.push(format!("types: {}", view.types.join(", ")));
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
    use super::{CrystalCore, CrystalPresentation, CrystalView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-crystal-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_crystal_shebang_line_as_crystal() {
        assert!(CrystalCore.sniff(b"#!/usr/bin/env crystal\nputs \"hi\"\n"));
    }

    #[test]
    fn sniffs_common_crystal_markers_as_crystal() {
        assert!(CrystalCore.sniff(b"struct Point\n  getter x : Int32\nend\n"));
        assert!(CrystalCore.sniff(b"class Greeter\n  property name : String\nend\n"));
        assert!(CrystalCore.sniff(b"class Counter\n  setter count : Int32\nend\n"));
        assert!(CrystalCore.sniff(b"lib LibC\n  fun getpid : Int32\nend\n"));
        assert!(CrystalCore.sniff(b"@[Link(\"m\")]\nlib LibM\nend\n"));
        assert!(CrystalCore.sniff(b"x = uninitialized Int32\n"));
        assert!(CrystalCore.sniff(b"ptr = pointerof(x)\n"));
    }

    #[test]
    fn does_not_sniff_ruby_or_other_languages_as_crystal() {
        assert!(!CrystalCore.sniff(
            b"require 'json'\n\nclass Greeter\n  attr_accessor :name\n\n  def greet\n    puts 'hi'\n  end\nend\n"
        ));
        assert!(!CrystalCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!CrystalCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!CrystalCore.sniff(b"interface Named {\n  name: string;\n}\n"));
        assert!(!CrystalCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!CrystalCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!CrystalCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!CrystalCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!CrystalCore.sniff(b"fun greet() {\n    println(\"hi\")\n}\n"));
        assert!(!CrystalCore.sniff(b"pub struct Point {\n    x: i32,\n    y: i32,\n}\n"));
        assert!(!CrystalCore.sniff(b"struct Point\n    x\n    y\nend\n"));
        assert!(!CrystalCore.sniff(b"macro greet(name): untyped =\n  echo name\n"));
        assert!(!CrystalCore.sniff(b"just a regular line of text\n"));
        assert!(!CrystalCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_crystal_file_and_extracts_definitions() {
        let path = unique_temp_file("greeter.cr");
        std::fs::write(
            &path,
            "struct Point\n  getter x : Int32\n  getter y : Int32\n\n  def initialize(@x : Int32, @y : Int32)\n  end\nend\n\ndef greet(name : String) : String\n  \"Hello, #{name}!\"\nend\n\nputs greet(\"world\")\n",
        )
        .unwrap();

        let data = CrystalCore.view(&path).unwrap();
        let view: CrystalView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.types, vec!["Point"]);
        assert_eq!(view.functions, vec!["initialize", "greet"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.cr");
        let mut content = "def pad\n".to_owned();
        content.push_str(&"#".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = CrystalCore.view(&path).unwrap();
        let view: CrystalView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_types_functions_and_content() {
        let data = serde_json::to_value(CrystalView {
            content: "struct A\nend".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            types: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = CrystalPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["types: A", "functions: greet", "struct A", "end"]
        );
    }
}
