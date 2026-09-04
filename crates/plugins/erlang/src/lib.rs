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
    /// Names of top-level function clause declarations found in the
    /// content.
    pub functions: Vec<String>,
    /// Names declared by a `-module(...).` attribute found in the content.
    pub modules: Vec<String>,
}

/// Extracts the identifier from a `-module(Name).` attribute at the start
/// of `line` (after leading whitespace), e.g. `-module(greeter).` returns
/// `Some("greeter")`.
fn parse_module_name(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("-module(")?;
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Extracts the identifier from a top-level function clause head at the
/// start of `line`, e.g. `greet(Name) ->` returns `Some("greet")`. Erlang
/// function names are atoms, which start with a lowercase letter; nested
/// clauses (`case`/`receive` branches) are conventionally indented, so a
/// leading-whitespace check keeps this to genuine top-level clauses.
fn parse_function_name(line: &str) -> Option<&str> {
    if line.starts_with(char::is_whitespace) {
        return None;
    }
    let mut chars = line.chars();
    if !chars.next().is_some_and(|ch| ch.is_ascii_lowercase()) {
        return None;
    }
    let paren = line.find('(')?;
    let name = &line[..paren];
    if !name.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
        return None;
    }
    line.contains("->").then_some(name)
}

/// Parses the module attribute and top-level function names out of
/// `content`, in source order.
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

/// Whether `text`'s first line is a shebang naming the `escript` runner,
/// the conventional way to make an Erlang source file directly executable.
fn has_erlang_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with("#!") && line.contains("escript"))
}

/// Whether `text` looks like Erlang source: markers not used by this
/// project's other source-language plugins. `-module(` and `-export(` are
/// Erlang's mandatory module and export attributes; `-record(`,
/// `-behaviour(`, and `-spec ` are further attribute forms unique to
/// Erlang's `-name(...)` attribute syntax; `io:format(` is Erlang's
/// qualified-call console function, distinct from every sibling plugin's
/// console markers. None of these overlap another plugin's checks, so this
/// plugin has no ordering constraint against a specific sibling.
fn has_erlang_syntax(text: &str) -> bool {
    text.contains("-module(")
        || text.contains("-export(")
        || text.contains("-record(")
        || text.contains("-behaviour(")
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
        assert!(ErlangCore.sniff(b"-module(greeter).\n-export([greet/1]).\n"));
        assert!(ErlangCore.sniff(b"-export([greet/1]).\n"));
        assert!(ErlangCore.sniff(b"-record(state, {count = 0}).\n"));
        assert!(ErlangCore.sniff(b"-behaviour(gen_server).\n"));
        assert!(ErlangCore.sniff(b"-spec greet(atom()) -> ok.\n"));
        assert!(ErlangCore.sniff(b"greet(Name) ->\n    io:format(\"Hello, ~s!~n\", [Name]).\n"));
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
        assert!(!ErlangCore.sniff(b"def greet(name: String): Unit = println(name)\n"));
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
            content: "-module(a).\ngreet(Name) ->\n    ok.".to_owned(),
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
                "greet(Name) ->",
                "    ok."
            ]
        );
    }
}
