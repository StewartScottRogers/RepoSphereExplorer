//! Dart file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`DartCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DartView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level function definitions found in the content.
    pub functions: Vec<String>,
    /// Names of top-level `class X` declarations found in the content.
    pub classes: Vec<String>,
}

/// Control-flow keywords that can precede a `(...) {` block without that
/// block being a function definition.
fn is_control_keyword(word: &str) -> bool {
    matches!(word, "if" | "for" | "while" | "switch" | "catch" | "try")
}

/// Extracts the function name from a line that looks like a top-level Dart
/// function definition, e.g. `void greet() {` or
/// `Future<void> main() async {`. Prototypes and calls (which do not end
/// the line with `{`) and control-flow statements are not matched.
fn parse_function_name(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let before_brace = trimmed.strip_suffix('{')?.trim_end();
    let before_brace = before_brace
        .strip_suffix("async")
        .map_or(before_brace, str::trim_end);
    let before_paren = before_brace.strip_suffix(')')?;
    let open = before_paren.rfind('(')?;
    let head = before_paren[..open].trim_end();
    let name_start = head
        .rfind(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .map_or(0, |i| i + 1);
    let name = &head[name_start..];
    (!name.is_empty() && !is_control_keyword(name)).then_some(name)
}

/// Extracts the type name from a top-level `class X` line, if present,
/// regardless of which modifiers or `extends`/`implements`/`with` clauses
/// follow the `class` keyword.
fn parse_class_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let idx = trimmed.find("class ")?;
    let rest = trimmed[idx + "class ".len()..].trim_start();
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level function and class names out of `content`, in source
/// order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut classes = Vec::new();
    for line in content.lines() {
        if let Some(name) = parse_class_name(line) {
            classes.push(name.to_owned());
        } else if let Some(name) = parse_function_name(line) {
            functions.push(name.to_owned());
        }
    }
    (functions, classes)
}

/// Whether `text` looks like Dart source: the `package:`/`dart:` import
/// schemes, the `library`/`part of` directives, and the lowercase
/// `@override` annotation are markers not used by any sibling plugin (Java's
/// equivalent annotation is capitalized `@Override`, which is a distinct,
/// case-sensitive string).
fn has_dart_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("import 'package:")
            || trimmed.starts_with("import 'dart:")
            || trimmed.starts_with("import \"package:")
            || trimmed.starts_with("import \"dart:")
            || trimmed.starts_with("library ")
            || trimmed.starts_with("part of ")
    }) || text.contains("@override")
        || text.contains("extends StatelessWidget")
        || text.contains("extends StatefulWidget")
}

/// The Dart plugin's core half.
#[derive(Debug, Default)]
pub struct DartCore;

impl PluginCore for DartCore {
    fn name(&self) -> &'static str {
        "dart"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_dart_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, classes) = parse_definitions(&content);
        let view = DartView {
            content,
            truncated,
            functions,
            classes,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Dart plugin's presentation half.
#[derive(Debug, Default)]
pub struct DartPresentation;

impl PluginPresentation for DartPresentation {
    fn name(&self) -> &'static str {
        "dart"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: DartView = match serde_json::from_value(data.clone()) {
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
    use super::{DartCore, DartPresentation, DartView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-dart-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_package_and_dart_imports_as_dart() {
        assert!(DartCore.sniff(
            b"import 'package:flutter/material.dart';\nimport 'dart:async';\n\nvoid main() {}\n"
        ));
    }

    #[test]
    fn sniffs_common_dart_markers_as_dart() {
        assert!(DartCore.sniff(b"library my_lib;\n\nvoid main() {}\n"));
        assert!(DartCore.sniff(b"part of 'quiz.dart';\n"));
        assert!(
            DartCore.sniff(b"class Greeter {\n  @override\n  String toString() => 'Greeter';\n}\n")
        );
        assert!(DartCore.sniff(b"class MyWidget extends StatelessWidget {\n}\n"));
        assert!(DartCore.sniff(b"class MyWidget extends StatefulWidget {\n}\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_dart() {
        assert!(!DartCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!DartCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!DartCore.sniff(b"interface Named {\n  name: string;\n}\n"));
        assert!(!DartCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!DartCore.sniff(b"package main\n\nfunc main() {}\n"));
        assert!(!DartCore.sniff(
            b"import java.util.List;\n\nclass Greeter {\n    @Override\n    public String toString() {\n        return \"\";\n    }\n}\n"
        ));
        assert!(!DartCore.sniff(b"#include <stdio.h>\n\nvoid main() {\n    printf(\"hi\");\n}\n"));
        assert!(!DartCore.sniff(b"just a regular line of text\n"));
        assert!(!DartCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_dart_file_and_extracts_definitions() {
        let path = unique_temp_file("greeter.dart");
        std::fs::write(
            &path,
            "import 'dart:core';\n\nclass Greeter {\n  String greet() {\n    return 'Hello, world!';\n  }\n\n  @override\n  String toString() => 'Greeter';\n}\n\nvoid main() {\n  print(Greeter().greet());\n}\n",
        )
        .unwrap();

        let data = DartCore.view(&path).unwrap();
        let view: DartView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.classes, vec!["Greeter"]);
        assert_eq!(view.functions, vec!["greet", "main"]);
        assert!(view.content.contains("Hello, world!"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.dart");
        let mut content = "void pad() {\n".to_owned();
        content.push_str(&"/".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = DartCore.view(&path).unwrap();
        let view: DartView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_classes_functions_and_content() {
        let data = serde_json::to_value(DartView {
            content: "class A {\n}".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            classes: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = DartPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["classes: A", "functions: greet", "class A {", "}"]
        );
    }
}
