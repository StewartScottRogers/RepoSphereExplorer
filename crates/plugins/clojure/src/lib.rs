//! Clojure file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`ClojureCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClojureView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `defn`/`defn-` function declarations found in the
    /// content.
    pub functions: Vec<String>,
    /// Names of top-level `ns` namespace declarations found in the content.
    pub namespaces: Vec<String>,
}

/// Whether `ch` may appear in a Clojure symbol (a rough approximation, not
/// the full grammar: covers hyphenated names and the conventional `?`/`!`
/// predicate/mutation suffixes).
fn is_symbol_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '-' | '_' | '?' | '!' | '*' | '.' | '\'' | '+')
}

/// Extracts the identifier following a `(defn ` or `(defn- ` opener at the
/// start of `line` (after leading whitespace), e.g. `(defn greet [name] ...)`
/// returns `Some("greet")`.
fn parse_function_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed
        .strip_prefix("(defn- ")
        .or_else(|| trimmed.strip_prefix("(defn "))?;
    let rest = rest.trim_start();
    let end = rest
        .find(|ch: char| !is_symbol_char(ch))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Extracts the identifier following a `(ns ` opener at the start of `line`
/// (after leading whitespace), e.g. `(ns my.app.core (:require ...))`
/// returns `Some("my.app.core")`.
fn parse_namespace_name(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("(ns ")?.trim_start();
    let end = rest
        .find(|ch: char| !is_symbol_char(ch))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level namespace and function names out of `content`, in
/// source order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut namespaces = Vec::new();
    for line in content.lines() {
        if let Some(name) = parse_namespace_name(line) {
            namespaces.push(name.to_owned());
        } else if let Some(name) = parse_function_name(line) {
            functions.push(name.to_owned());
        }
    }
    (functions, namespaces)
}

/// Whether `text`'s first line is a shebang naming the `clojure` interpreter.
fn has_clojure_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with("#!") && line.contains("clojure"))
}

/// Whether `text` looks like Clojure source: markers not used by this
/// project's other source-language plugins. `(ns ` is Clojure's namespace
/// declaration, `(defn `/`(defn- ` are its function-definition forms,
/// `(defmacro ` is its macro-definition form, and `(require '` is its
/// quoted-symbol require call; none of these parenthesized, prefix-form
/// markers overlap a sibling plugin's checks, so this plugin needs no
/// ordering constraint against a specific sibling.
fn has_clojure_syntax(text: &str) -> bool {
    text.contains("(ns ")
        || text.contains("(defn ")
        || text.contains("(defn- ")
        || text.contains("(defmacro ")
        || text.contains("(require '")
}

/// The Clojure plugin's core half.
#[derive(Debug, Default)]
pub struct ClojureCore;

impl PluginCore for ClojureCore {
    fn name(&self) -> &'static str {
        "clojure"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_clojure_shebang(text) || has_clojure_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, namespaces) = parse_definitions(&content);
        let view = ClojureView {
            content,
            truncated,
            functions,
            namespaces,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Clojure plugin's presentation half.
#[derive(Debug, Default)]
pub struct ClojurePresentation;

impl PluginPresentation for ClojurePresentation {
    fn name(&self) -> &'static str {
        "clojure"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: ClojureView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.namespaces.is_empty() {
            lines.push(format!("namespaces: {}", view.namespaces.join(", ")));
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
    use super::{ClojureCore, ClojurePresentation, ClojureView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-clojure-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_clojure_shebang_line_as_clojure() {
        assert!(ClojureCore.sniff(b"#!/usr/bin/env clojure\n(println \"hi\")\n"));
    }

    #[test]
    fn sniffs_common_clojure_markers_as_clojure() {
        assert!(ClojureCore.sniff(b"(ns my.app.core)\n"));
        assert!(ClojureCore.sniff(b"(defn greet [name]\n  (str \"hi \" name))\n"));
        assert!(ClojureCore.sniff(b"(defn- helper [] :ok)\n"));
        assert!(ClojureCore.sniff(b"(defmacro unless [test body]\n  `(if ~test nil ~body))\n"));
        assert!(ClojureCore.sniff(b"(require 'clojure.string)\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_clojure() {
        assert!(!ClojureCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!ClojureCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!ClojureCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!ClojureCore.sniff(b"defmodule Greeter do\n  def hi, do: :ok\nend\n"));
        assert!(!ClojureCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!ClojureCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!ClojureCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!ClojureCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!ClojureCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!ClojureCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!ClojureCore.sniff(b"x <- 5\nresult <- data %>% filter(x > 1)\n"));
        assert!(!ClojureCore.sniff(b"import scala.collection.mutable.ListBuffer\n"));
        assert!(!ClojureCore.sniff(b"just a regular line of text\n"));
        assert!(!ClojureCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_clojure_file_and_extracts_definitions() {
        let path = unique_temp_file("greeter.clj");
        std::fs::write(
            &path,
            "(ns my.app.greeter\n  (:require [clojure.string :as str]))\n\n(defn greet [name]\n  (str \"Hello, \" name \"!\"))\n\n(defn- shout [name]\n  (str/upper-case name))\n",
        )
        .unwrap();

        let data = ClojureCore.view(&path).unwrap();
        let view: ClojureView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.namespaces, vec!["my.app.greeter"]);
        assert_eq!(view.functions, vec!["greet", "shout"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.clj");
        let mut content = "(ns large)\n".to_owned();
        content.push_str(&"; ".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = ClojureCore.view(&path).unwrap();
        let view: ClojureView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_namespaces_functions_and_content() {
        let data = serde_json::to_value(ClojureView {
            content: "(ns a)\n(defn greet [] :ok)".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            namespaces: vec!["a".to_owned()],
        })
        .unwrap();

        let lines = ClojurePresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "namespaces: a",
                "functions: greet",
                "(ns a)",
                "(defn greet [] :ok)"
            ]
        );
    }
}
