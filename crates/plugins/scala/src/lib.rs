//! Scala file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`ScalaCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScalaView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `def` declarations found in the content.
    pub functions: Vec<String>,
    /// Names of top-level `class`/`object`/`trait` declarations found in
    /// the content.
    pub types: Vec<String>,
}

/// Extracts the identifier following a `def ` keyword at the start of
/// `line` (after leading whitespace), e.g. `def greet(name: String) =`
/// returns `Some("greet")`.
fn parse_function_name(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("def ")?;
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Extracts the identifier following a `class`/`object`/`trait` keyword at
/// the start of `line` (after leading whitespace and any `case `/`sealed
/// `/`abstract ` modifiers), e.g. `case class Point(x: Int, y: Int)`
/// returns `Some("Point")`.
fn parse_type_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let trimmed = trimmed.strip_prefix("case ").unwrap_or(trimmed);
    let trimmed = trimmed.strip_prefix("sealed ").unwrap_or(trimmed);
    let trimmed = trimmed.strip_prefix("abstract ").unwrap_or(trimmed);
    let rest = trimmed
        .strip_prefix("class ")
        .or_else(|| trimmed.strip_prefix("object "))
        .or_else(|| trimmed.strip_prefix("trait "))?;
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level function and type names out of `content`, in source
/// order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut types = Vec::new();
    for line in content.lines() {
        if let Some(name) = parse_type_name(line) {
            types.push(name.to_owned());
        } else if let Some(name) = parse_function_name(line) {
            functions.push(name.to_owned());
        }
    }
    (functions, types)
}

/// Whether `text`'s first line is a shebang naming the `scala` interpreter.
fn has_scala_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with("#!") && line.contains("scala"))
}

/// Whether `text` looks like Scala source: markers not used by this
/// project's other source-language plugins. `import scala.` mirrors the
/// Java/Kotlin plugins' own `import java.`/`import kotlin.` checks;
/// `case class ` is Scala's case-class idiom, distinct from the Kotlin
/// plugin's `data class `; `extends App` is Scala's classic application
/// entry point; `def main(args: Array[String]` is Scala's main-method
/// signature, distinct from Kotlin's `fun main(` and Java's
/// `public static void main(String`; and `sealed trait ` is checked as a
/// compound marker (never a bare `trait ` line start) so it does not
/// collide with the Rust plugin's own bare `trait `/`pub trait ` line-start
/// check.
fn has_scala_syntax(text: &str) -> bool {
    text.contains("import scala.")
        || text.contains("case class ")
        || text.contains("extends App")
        || text.contains("def main(args: Array[String]")
        || text.contains("sealed trait ")
}

/// The Scala plugin's core half.
#[derive(Debug, Default)]
pub struct ScalaCore;

impl PluginCore for ScalaCore {
    fn name(&self) -> &'static str {
        "scala"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_scala_shebang(text) || has_scala_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, types) = parse_definitions(&content);
        let view = ScalaView {
            content,
            truncated,
            functions,
            types,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Scala plugin's presentation half.
#[derive(Debug, Default)]
pub struct ScalaPresentation;

impl PluginPresentation for ScalaPresentation {
    fn name(&self) -> &'static str {
        "scala"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: ScalaView = match serde_json::from_value(data.clone()) {
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
    use super::{MAX_VIEW_BYTES, ScalaCore, ScalaPresentation, ScalaView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-scala-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_scala_shebang_line_as_scala() {
        assert!(ScalaCore.sniff(b"#!/usr/bin/env scala\nprintln(\"hi\")\n"));
    }

    #[test]
    fn sniffs_common_scala_markers_as_scala() {
        assert!(ScalaCore.sniff(b"import scala.collection.mutable.ListBuffer\n"));
        assert!(ScalaCore.sniff(b"case class Point(x: Int, y: Int)\n"));
        assert!(ScalaCore.sniff(b"object Main extends App {\n  println(\"hi\")\n}\n"));
        assert!(ScalaCore.sniff(
            b"object Main {\n  def main(args: Array[String]): Unit = {\n    println(\"hi\")\n  }\n}\n"
        ));
        assert!(ScalaCore.sniff(b"sealed trait Shape\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_scala() {
        assert!(!ScalaCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!ScalaCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!ScalaCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!ScalaCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!ScalaCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!ScalaCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!ScalaCore.sniff(b"pub trait Greeter {\n  fn greet(&self) -> String;\n}\n"));
        assert!(!ScalaCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!ScalaCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!ScalaCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!ScalaCore.sniff(
            b"import kotlin.math.max\n\ndata class Point(val x: Int, val y: Int)\n\nfun main(args: Array<String>) {\n    println(\"hi\")\n}\n"
        ));
        assert!(!ScalaCore.sniff(
            b"using System;\n\nclass Program {\n    static void Main() {\n        Console.WriteLine(\"hi\");\n    }\n}\n"
        ));
        assert!(!ScalaCore.sniff(b"use strict;\nuse warnings;\n\nsub greet {\n    return 1;\n}\n"));
        assert!(!ScalaCore.sniff(b"x <- 5\nresult <- data %>% filter(x > 1)\n"));
        assert!(!ScalaCore.sniff(b"just a regular line of text\n"));
        assert!(!ScalaCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_scala_file_and_extracts_definitions() {
        let path = unique_temp_file("Greeter.scala");
        std::fs::write(
            &path,
            "import scala.collection.mutable.ListBuffer\n\ncase class Greeting(message: String)\n\nobject Greeter extends App {\n  def greet(name: String): String = s\"Hello, $name!\"\n\n  println(greet(\"world\"))\n}\n",
        )
        .unwrap();

        let data = ScalaCore.view(&path).unwrap();
        let view: ScalaView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.types, vec!["Greeting", "Greeter"]);
        assert_eq!(view.functions, vec!["greet"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("Large.scala");
        let mut content = "object Large {\n".to_owned();
        content.push_str(&"// ".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = ScalaCore.view(&path).unwrap();
        let view: ScalaView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_types_functions_and_content() {
        let data = serde_json::to_value(ScalaView {
            content: "object A {\n}".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            types: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = ScalaPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["types: A", "functions: greet", "object A {", "}"]
        );
    }
}
