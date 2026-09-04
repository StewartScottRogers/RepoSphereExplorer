//! Erlang file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`ErlangCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErlangView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level function clause heads found in the content.
    pub functions: Vec<String>,
    /// The module name declared by a `-module(Name).` attribute, if present.
    pub modules: Vec<String>,
}

/// Extracts the identifier declared by a `-module(Name).` attribute line.
fn parse_module_name(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("-module(")?;
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Extracts the function name from a line that looks like a top-level Erlang
/// function clause head, e.g. `greet(Name) ->`. Erlang function clauses
/// start at column zero (unlike the bodies they open), so lines with
/// leading whitespace are not matched, nor are attribute lines (which start
/// with `-`, e.g. `-module(...)`/`-export(...)`) or lines not starting with
/// a lowercase atom.
fn parse_function_name(line: &str) -> Option<&str> {
    if line.starts_with(['-', ' ', '\t']) {
        return None;
    }
    let open = line.find('(')?;
    let name = &line[..open];
    let first = name.chars().next()?;
    (first.is_lowercase() && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_'))
        .then_some(name)
}

/// Parses the module name and top-level function names out of `content`, in
/// source order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut modules = Vec::new();
    for line in content.lines() {
        if let Some(name) = parse_module_name(line) {
            modules.push(name.to_owned());
        } else if let Some(name) = parse_function_name(line) {
            functions.push(name.to_owned());
        }
    }
    (functions, modules)
}

/// Whether `text`'s first line is a shebang naming the `escript` interpreter,
/// the standard way to run an Erlang source file as a script.
fn has_erlang_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with("#!") && line.contains("escript"))
}

/// Whether `text` looks like Erlang source: markers not used by this
/// project's other source-language plugins. `-module(`/`-export(`/
/// `-record(`/`-behaviour(`/`-behavior(` are Erlang's parenthesized,
/// dash-prefixed module attributes; `-spec ` is its function type-signature
/// attribute; and `io:format(` is Erlang's qualified console-output call,
/// distinct from every sibling plugin's own console-output marker. None of
/// these overlap another plugin's checks, so this plugin needs no ordering
/// constraint against a specific sibling.
fn has_erlang_syntax(text: &str) -> bool {
    text.contains("-module(")
        || text.contains("-export(")
        || text.contains("-record(")
        || text.contains("-behaviour(")
        || text.contains("-behavior(")
        || text.contains("-spec ")
        || text.contains("io:format(")
}

/// The Erlang plugin's core half.
#[derive(Debug, Default)]
pub struct ErlangCore;

impl PluginCore for ErlangCore {
    fn name(&self) -> &'static str {
        "erlang"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_erlang_shebang(text) || has_erlang_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, modules) = parse_definitions(&content);
        let view = ErlangView {
            content,
            truncated,
            functions,
            modules,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Erlang plugin's presentation half.
#[derive(Debug, Default)]
pub struct ErlangPresentation;

impl PluginPresentation for ErlangPresentation {
    fn name(&self) -> &'static str {
        "erlang"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: ErlangView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.modules.is_empty() {
            lines.push(format!("modules: {}", view.modules.join(", ")));
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
    use super::{ErlangCore, ErlangPresentation, ErlangView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-erlang-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_an_escript_shebang_line_as_erlang() {
        assert!(ErlangCore.sniff(b"#!/usr/bin/env escript\nmain(_) -> ok.\n"));
    }

    #[test]
    fn sniffs_common_erlang_markers_as_erlang() {
        assert!(ErlangCore.sniff(b"-module(greeter).\n-export([hi/0]).\n"));
        assert!(ErlangCore.sniff(b"-record(person, {name, age}).\n"));
        assert!(ErlangCore.sniff(b"-behaviour(gen_server).\n"));
        assert!(ErlangCore.sniff(b"-behavior(gen_server).\n"));
        assert!(ErlangCore.sniff(b"-spec hi() -> ok.\n"));
        assert!(ErlangCore.sniff(b"greet() ->\n    io:format(\"hi~n\").\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_erlang() {
        assert!(!ErlangCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!ErlangCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!ErlangCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!ErlangCore.sniff(b"defmodule Greeter do\n  def hi, do: :ok\nend\n"));
        assert!(!ErlangCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!ErlangCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!ErlangCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!ErlangCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!ErlangCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!ErlangCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!ErlangCore.sniff(b"x <- 5\nresult <- data %>% filter(x > 1)\n"));
        assert!(!ErlangCore.sniff(b"import scala.collection.mutable.ListBuffer\n"));
        assert!(!ErlangCore.sniff(b"(ns greeter.core)\n(defn hi [] :ok)\n"));
        assert!(!ErlangCore.sniff(b"just a regular line of text\n"));
        assert!(!ErlangCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_erlang_file_and_extracts_definitions() {
        let path = unique_temp_file("greeter.erl");
        std::fs::write(
            &path,
            "-module(greeter).\n-export([greet/1]).\n\ngreet(Name) ->\n    io:format(\"Hello, ~s!~n\", [Name]).\n\nshout(Name) ->\n    string:uppercase(Name).\n",
        )
        .unwrap();

        let data = ErlangCore.view(&path).unwrap();
        let view: ErlangView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.modules, vec!["greeter"]);
        assert_eq!(view.functions, vec!["greet", "shout"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.erl");
        let mut content = "-module(large).\n".to_owned();
        content.push_str(&"% ".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = ErlangCore.view(&path).unwrap();
        let view: ErlangView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_modules_functions_and_content() {
        let data = serde_json::to_value(ErlangView {
            content: "-module(a).\ngreet() -> ok.".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            modules: vec!["a".to_owned()],
        })
        .unwrap();

        let lines = ErlangPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "modules: a",
                "functions: greet",
                "-module(a).",
                "greet() -> ok."
            ]
        );
    }
}
