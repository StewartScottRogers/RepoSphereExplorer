//! Ruby file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`RubyCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RubyView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `def` method definitions found in the content.
    pub functions: Vec<String>,
    /// Names of top-level `class X` declarations found in the content.
    pub classes: Vec<String>,
}

/// Extracts the identifier that follows `keyword` at the start of `line`,
/// e.g. `top_level_name("def greet", "def")` returns `Some("greet")`. Ruby
/// method names may end in `?`, `!`, or `=`, which are included in the
/// match; class names may not.
fn top_level_name<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(keyword)?.strip_prefix(' ')?;
    let end = rest
        .find(|ch: char| {
            !(ch.is_alphanumeric() || ch == '_' || ch == '?' || ch == '!' || ch == '=')
        })
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level method and class names out of `content`, in source
/// order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut classes = Vec::new();
    for line in content.lines() {
        if let Some(name) = top_level_name(line, "class") {
            classes.push(name.to_owned());
        } else if let Some(name) = top_level_name(line, "def") {
            functions.push(name.to_owned());
        }
    }
    (functions, classes)
}

/// Whether `text`'s first line is a shebang naming the `ruby` interpreter.
fn has_ruby_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with("#!") && line.contains("ruby"))
}

/// Whether `text` contains a top-level `def ` or `class ` line. Used to
/// gate the bare `end` marker below: Ruby's block-closing `end` convention
/// is meaningless without a Ruby-style method or class body to close, and
/// other languages (Julia's `function`/`module` blocks, for instance) close
/// blocks with a bare `end` too.
fn has_def_or_class_line(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("def ") || trimmed.starts_with("class ")
    })
}

/// Whether a trimmed line is a Ruby-style `module Name` declaration, as
/// opposed to Haskell's `module Name where` or Elm's `module Name exposing
/// (...)`, which use the same `module ` prefix but are always followed by
/// one of those two keywords.
fn is_ruby_style_module_line(trimmed: &str) -> bool {
    trimmed.starts_with("module ") && !trimmed.contains("where") && !trimmed.contains("exposing")
}

/// Whether `text` looks like Ruby source: markers not used by this
/// project's other source-language plugins. A bare `end` line closes
/// Ruby's `def`/`class`/`module`/`do` blocks, and is only trusted alongside
/// a `def `/`class ` line since other languages close blocks with a bare
/// `end` too; `require '`/`require "` (no parenthesis) is Ruby's require
/// statement, distinct from JavaScript's `require(`; `attr_accessor`/
/// `attr_reader`/`attr_writer`, a Ruby-style `module Name` line (not
/// followed by `where`/`exposing`), and a `do |...|` block parameter list
/// are otherwise Ruby-only idioms. These markers deliberately avoid the
/// Python plugin's bare `def `/`class ` check (which Ruby's own
/// `def`/`class` lines would also match), so this plugin is placed just
/// ahead of `python` in `CORE_PLUGINS` to claim Ruby files first.
fn has_ruby_syntax(text: &str) -> bool {
    let has_def_or_class = has_def_or_class_line(text);
    text.lines().any(|line| {
        let trimmed = line.trim();
        (trimmed == "end" && has_def_or_class)
            || trimmed.starts_with("require '")
            || trimmed.starts_with("require \"")
            || is_ruby_style_module_line(trimmed)
    }) || text.contains("attr_accessor")
        || text.contains("attr_reader")
        || text.contains("attr_writer")
        || text.contains(" do |")
}

/// The Ruby plugin's core half.
#[derive(Debug, Default)]
pub struct RubyCore;

impl PluginCore for RubyCore {
    fn name(&self) -> &'static str {
        "ruby"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_ruby_shebang(text) || has_ruby_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, classes) = parse_definitions(&content);
        let view = RubyView {
            content,
            truncated,
            functions,
            classes,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Ruby plugin's presentation half.
#[derive(Debug, Default)]
pub struct RubyPresentation;

impl PluginPresentation for RubyPresentation {
    fn name(&self) -> &'static str {
        "ruby"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: RubyView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.classes.is_empty() {
            lines.push(format!("classes: {}", view.classes.join(", ")));
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
    use super::{MAX_VIEW_BYTES, RubyCore, RubyPresentation, RubyView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-ruby-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_ruby_shebang_line_as_ruby() {
        assert!(RubyCore.sniff(b"#!/usr/bin/env ruby\nputs 'hi'\n"));
    }

    #[test]
    fn sniffs_common_ruby_markers_as_ruby() {
        assert!(RubyCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(RubyCore.sniff(b"class Greeter\n  def greet\n    puts 'hi'\n  end\nend\n"));
        assert!(RubyCore.sniff(b"require 'json'\n"));
        assert!(RubyCore.sniff(b"class Point\n  attr_accessor :x, :y\nend\n"));
        assert!(RubyCore.sniff(b"module Greetable\nend\n"));
        assert!(RubyCore.sniff(b"[1, 2, 3].each do |n|\n  puts n\nend\n"));
    }

    #[test]
    fn does_not_sniff_python_or_other_languages_as_ruby() {
        assert!(!RubyCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!RubyCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!RubyCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!RubyCore.sniff(b"const load = require('json');\n"));
        assert!(!RubyCore.sniff(b"interface Named {\n  name: string;\n}\n"));
        assert!(!RubyCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!RubyCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!RubyCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!RubyCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!RubyCore.sniff(b"fun greet() {\n    println(\"hi\")\n}\n"));
        assert!(!RubyCore.sniff(b"just a regular line of text\n"));
        assert!(!RubyCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn does_not_sniff_an_elm_module_as_ruby() {
        assert!(
            !RubyCore.sniff(b"module Main exposing (main)\n\nmain =\n    text \"Hello, world!\"\n")
        );
    }

    #[test]
    fn does_not_sniff_a_haskell_module_as_ruby() {
        assert!(
            !RubyCore
                .sniff(b"module Main where\n\nmain :: IO ()\nmain = putStrLn \"Hello, world!\"\n")
        );
    }

    #[test]
    fn does_not_sniff_a_julia_function_as_ruby() {
        assert!(!RubyCore.sniff(
            b"function greet(name)\n    println(\"Hello, $name!\")\nend\n\ngreet(\"world\")\n"
        ));
    }

    #[test]
    fn does_not_sniff_a_tcl_puts_call_as_ruby() {
        assert!(!RubyCore.sniff(b"puts \"Hello, world!\"\n"));
    }

    #[test]
    fn views_a_real_ruby_file_and_extracts_definitions() {
        let path = unique_temp_file("greeter.rb");
        std::fs::write(
            &path,
            "require 'json'\n\nclass Greeter\nend\n\ndef greet(name)\n  \"Hello, #{name}!\"\nend\n\nputs greet('world')\n",
        )
        .unwrap();

        let data = RubyCore.view(&path).unwrap();
        let view: RubyView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.classes, vec!["Greeter"]);
        assert_eq!(view.functions, vec!["greet"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.rb");
        let mut content = "def pad\n".to_owned();
        content.push_str(&"#".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = RubyCore.view(&path).unwrap();
        let view: RubyView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_classes_functions_and_content() {
        let data = serde_json::to_value(RubyView {
            content: "class A\nend".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            classes: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = RubyPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["classes: A", "functions: greet", "class A", "end"]
        );
    }
}
