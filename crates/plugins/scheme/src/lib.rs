//! Scheme/Lisp file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`SchemeCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemeView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `define`/`defun` declarations found in the
    /// content, in source order.
    pub declarations: Vec<String>,
}

/// Whether `ch` may appear in a Scheme/Lisp symbol (a rough approximation,
/// not the full grammar: covers hyphenated names and the conventional
/// `?`/`!` predicate/mutation suffixes).
fn is_symbol_char(ch: char) -> bool {
    ch.is_alphanumeric()
        || matches!(
            ch,
            '-' | '_' | '?' | '!' | '*' | '.' | '+' | '<' | '>' | '='
        )
}

/// Extracts the identifier named by a `(define ` or `(defun ` opener at the
/// start of `line` (after leading whitespace). Handles both Scheme's
/// variable form (`(define name value)`) and its function form (`(define
/// (name args...) body)`), as well as Common Lisp's `(defun name (args)
/// body)`.
fn parse_declaration_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed
        .strip_prefix("(define ")
        .or_else(|| trimmed.strip_prefix("(defun "))?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('(').unwrap_or(rest);
    let end = rest
        .find(|ch: char| !is_symbol_char(ch))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level `define`/`defun` declaration names out of `content`, in
/// source order.
fn parse_declarations(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(parse_declaration_name)
        .map(str::to_owned)
        .collect()
}

/// Whether `text` looks like Scheme/Lisp source: markers not used by this
/// project's other source-language plugins. `(define ` is Scheme's
/// variable/function definition form, `(defun ` is Common Lisp's
/// function-definition form (Clojure's equivalent form is the distinct
/// `(defn `), `(lambda ` is both dialects' anonymous-function form, and
/// `#lang ` is the language-declaration header used by Scheme dialects such
/// as Racket; none of these overlap a sibling plugin's checks, so this
/// plugin needs no ordering constraint against a specific sibling.
fn has_scheme_syntax(text: &str) -> bool {
    text.contains("(define ")
        || text.contains("(defun ")
        || text.contains("(lambda ")
        || text.contains("#lang ")
}

/// The Scheme/Lisp plugin's core half.
#[derive(Debug, Default)]
pub struct SchemeCore;

impl PluginCore for SchemeCore {
    fn name(&self) -> &'static str {
        "scheme"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_scheme_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let declarations = parse_declarations(&content);
        let view = SchemeView {
            content,
            truncated,
            declarations,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Scheme/Lisp plugin's presentation half.
#[derive(Debug, Default)]
pub struct SchemePresentation;

impl PluginPresentation for SchemePresentation {
    fn name(&self) -> &'static str {
        "scheme"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: SchemeView = match serde_json::from_value(data.clone()) {
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
    use super::{MAX_VIEW_BYTES, SchemeCore, SchemePresentation, SchemeView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-scheme-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_scheme_and_lisp_markers_as_scheme() {
        assert!(SchemeCore.sniff(b"(define x 5)\n"));
        assert!(SchemeCore.sniff(b"(define (greet name)\n  (display name))\n"));
        assert!(SchemeCore.sniff(b"(defun greet (name)\n  (format t \"~a\" name))\n"));
        assert!(SchemeCore.sniff(b"(map (lambda (x) (* x x)) '(1 2 3))\n"));
        assert!(SchemeCore.sniff(b"#lang racket\n(display \"hi\")\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_scheme() {
        assert!(!SchemeCore.sniff(b"(ns my.app.core)\n(defn greet [name]\n  (str name))\n"));
        assert!(!SchemeCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!SchemeCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!SchemeCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!SchemeCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!SchemeCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!SchemeCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!SchemeCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!SchemeCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!SchemeCore.sniff(b"just a regular line of text\n"));
        assert!(!SchemeCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_scheme_file_and_extracts_declarations() {
        let path = unique_temp_file("greeter.scm");
        std::fs::write(
            &path,
            "(define greeting \"Hello\")\n\n(define (greet name)\n  (string-append greeting \", \" name \"!\"))\n\n(display (greet \"World\"))\n",
        )
        .unwrap();

        let data = SchemeCore.view(&path).unwrap();
        let view: SchemeView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.declarations, vec!["greeting", "greet"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_real_lisp_file_and_extracts_declarations() {
        let path = unique_temp_file("greeter.lisp");
        std::fs::write(
            &path,
            "(defun greet (name)\n  (format nil \"Hello, ~a!\" name))\n\n(print (greet \"World\"))\n",
        )
        .unwrap();

        let data = SchemeCore.view(&path).unwrap();
        let view: SchemeView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.declarations, vec!["greet"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.scm");
        let mut content = "(define x 1)\n".to_owned();
        content.push_str(&"; ".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = SchemeCore.view(&path).unwrap();
        let view: SchemeView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_declarations_and_content() {
        let data = serde_json::to_value(SchemeView {
            content: "(define x 1)\n(display x)".to_owned(),
            truncated: false,
            declarations: vec!["x".to_owned()],
        })
        .unwrap();

        let lines = SchemePresentation.present(&data);

        assert_eq!(
            lines,
            vec!["declarations: x", "(define x 1)", "(display x)"]
        );
    }
}
