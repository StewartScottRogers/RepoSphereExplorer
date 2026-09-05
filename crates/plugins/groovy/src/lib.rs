//! Groovy file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`GroovyCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroovyView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level function/method definitions found in the content.
    pub functions: Vec<String>,
    /// Names of top-level `class X` declarations found in the content.
    pub classes: Vec<String>,
}

/// Control-flow keywords that can precede a `(...) {` block without that
/// block being a function definition.
fn is_control_keyword(word: &str) -> bool {
    matches!(word, "if" | "for" | "while" | "switch" | "catch" | "try")
}

/// Extracts the function name from a line that looks like a top-level
/// Groovy method definition, e.g. `def greet() {` or
/// `String greet(String name) {`. Prototypes and calls (which do not end
/// the line with `{`) and control-flow statements are not matched.
fn parse_function_name(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let before_brace = trimmed.strip_suffix('{')?.trim_end();
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
/// regardless of which modifiers (`abstract`, `final`, ...) precede the
/// `class` keyword.
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

/// Whether `text`'s first line is a shebang naming the `groovy` interpreter.
fn has_groovy_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with("#!") && line.contains("groovy"))
}

/// Whether `line` is a top-level `def`-keyword declaration, e.g.
/// `def greet() {` or `static def greet(String name) {`. Ending the line
/// with `{` distinguishes Groovy's brace-bodied `def` from Python's
/// colon-terminated `def greet():` and Ruby's brace-less `def greet` /
/// `end` pairing, neither of which this project's `def`-keyword languages
/// (Python, Ruby) ever close with a bare `{`.
fn is_def_declaration(line: &str) -> bool {
    let trimmed = line.trim_start();
    let trimmed = trimmed.strip_prefix("static ").unwrap_or(trimmed);
    let trimmed = trimmed.strip_prefix("private ").unwrap_or(trimmed);
    let trimmed = trimmed.strip_prefix("protected ").unwrap_or(trimmed);
    let trimmed = trimmed.strip_prefix("public ").unwrap_or(trimmed);
    trimmed
        .strip_prefix("def ")
        .is_some_and(|rest| rest.trim_end().ends_with('{'))
}

/// Whether `text` looks like Groovy source: markers not used by this
/// project's other source-language plugins. `import groovy.` mirrors the
/// Java/Kotlin/Scala plugins' own `import java.`/`import kotlin.`/
/// `import scala.` checks; `@groovy.transform.` and `@Grab(` are
/// qualified-annotation and dependency-grabbing idioms unique to Groovy; a
/// brace-bodied `def` declaration (see [`is_def_declaration`]) is Groovy's
/// method syntax, distinct from Python's and Ruby's own `def`; and a bare
/// `println ` call (a space rather than an opening paren) is Groovy's
/// optional-parentheses statement style, distinct from every other
/// plugin's own `println(`/`puts `/`print(` conventions.
fn has_groovy_syntax(text: &str) -> bool {
    text.contains("import groovy.")
        || text.contains("@groovy.transform.")
        || text.contains("@Grab(")
        || text
            .lines()
            .any(|line| is_def_declaration(line) || line.trim_start().starts_with("println "))
}

/// The Groovy plugin's core half.
#[derive(Debug, Default)]
pub struct GroovyCore;

impl PluginCore for GroovyCore {
    fn name(&self) -> &'static str {
        "groovy"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_groovy_shebang(text) || has_groovy_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, classes) = parse_definitions(&content);
        let view = GroovyView {
            content,
            truncated,
            functions,
            classes,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Groovy plugin's presentation half.
#[derive(Debug, Default)]
pub struct GroovyPresentation;

impl PluginPresentation for GroovyPresentation {
    fn name(&self) -> &'static str {
        "groovy"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: GroovyView = match serde_json::from_value(data.clone()) {
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
    use super::{GroovyCore, GroovyPresentation, GroovyView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-groovy-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_groovy_shebang_line_as_groovy() {
        assert!(GroovyCore.sniff(b"#!/usr/bin/env groovy\nprintln 'hi'\n"));
    }

    #[test]
    fn sniffs_common_groovy_markers_as_groovy() {
        assert!(GroovyCore.sniff(b"import groovy.transform.Immutable\n"));
        assert!(GroovyCore.sniff(b"@groovy.transform.Immutable\nclass Point { int x, y }\n"));
        assert!(GroovyCore.sniff(b"@Grab('org.apache.commons:commons-lang3:3.12.0')\n"));
        assert!(GroovyCore.sniff(b"def greet() {\n    println 'hi'\n}\n"));
        assert!(GroovyCore.sniff(b"static def greet(String name) {\n    return name\n}\n"));
        assert!(GroovyCore.sniff(b"println 'hello, world!'\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_groovy() {
        assert!(!GroovyCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!GroovyCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!GroovyCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!GroovyCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!GroovyCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!GroovyCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!GroovyCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!GroovyCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!GroovyCore.sniff(
            b"import kotlin.math.max\n\ndata class Point(val x: Int, val y: Int)\n\nfun main(args: Array<String>) {\n    println(\"hi\")\n}\n"
        ));
        assert!(!GroovyCore.sniff(b"import scala.collection.mutable.ListBuffer\n"));
        assert!(!GroovyCore.sniff(
            b"using System;\n\nclass Program {\n    static void Main() {\n        Console.WriteLine(\"hi\");\n    }\n}\n"
        ));
        assert!(
            !GroovyCore.sniff(b"use strict;\nuse warnings;\n\nsub greet {\n    return 1;\n}\n")
        );
        assert!(!GroovyCore.sniff(b"just a regular line of text\n"));
        assert!(!GroovyCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_groovy_file_and_extracts_definitions() {
        let path = unique_temp_file("Greeter.groovy");
        std::fs::write(
            &path,
            "import groovy.transform.Immutable\n\n@Immutable\nclass Greeting {\n    String message\n}\n\nclass Greeter {\n    def greet(String name) {\n        println \"Hello, ${name}!\"\n    }\n}\n\nnew Greeter().greet('world')\n",
        )
        .unwrap();

        let data = GroovyCore.view(&path).unwrap();
        let view: GroovyView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.classes, vec!["Greeting", "Greeter"]);
        assert_eq!(view.functions, vec!["greet"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("Large.groovy");
        let mut content = "class Large {\n".to_owned();
        content.push_str(&"// ".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = GroovyCore.view(&path).unwrap();
        let view: GroovyView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_classes_functions_and_content() {
        let data = serde_json::to_value(GroovyView {
            content: "class A {\n}".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            classes: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = GroovyPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["classes: A", "functions: greet", "class A {", "}"]
        );
    }
}
