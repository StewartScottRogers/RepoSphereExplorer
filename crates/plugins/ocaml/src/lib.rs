//! OCaml file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`OCamlCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OCamlView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `let [rec] name ... = ` bindings found in the
    /// content, in source order.
    pub bindings: Vec<String>,
}

/// Extracts the name from a top-level `let [rec] name ... = ` binding line,
/// e.g. `binding_name("let greet name = name")` returns `Some("greet")`.
fn binding_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("let ")?;
    let rest = rest.strip_prefix("rec ").unwrap_or(rest);
    let end = rest.find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '\''))?;
    let name = &rest[..end];
    let starts_ok = name
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_lowercase() || ch == '_');
    if name.is_empty() || !starts_ok {
        return None;
    }
    rest[end..].contains('=').then_some(name)
}

/// Parses top-level binding names out of `content`, in source order.
fn parse_bindings(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| binding_name(line.trim_start()))
        .map(str::to_owned)
        .collect()
}

/// Whether `text`'s first line is a shebang naming the `ocaml`/`ocamlrun`
/// interpreter.
fn has_ocaml_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with("#!") && line.contains("ocaml"))
}

/// Whether `text` looks like OCaml source: markers not used by this
/// project's other source-language plugins. `Printf.printf ` is OCaml's
/// qualified formatted-print call, distinct from F#'s unqualified `printfn`
/// and C's unqualified `printf`; `;;` is OCaml's top-level phrase
/// terminator; `module type ` introduces a module signature, a construct no
/// sibling plugin's `module `/`defmodule `/`-module(` markers overlap with.
/// A bare `let rec ` line is also checked, per the F# plugin's own note that
/// it deliberately leaves this marker unclaimed for a future OCaml plugin.
fn has_ocaml_syntax(text: &str) -> bool {
    text.contains("Printf.printf ")
        || text.contains(";;")
        || text.contains("module type ")
        || text
            .lines()
            .any(|line| line.trim_start().starts_with("let rec "))
}

/// The OCaml plugin's core half.
#[derive(Debug, Default)]
pub struct OCamlCore;

impl PluginCore for OCamlCore {
    fn name(&self) -> &'static str {
        "ocaml"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_ocaml_shebang(text) || has_ocaml_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let bindings = parse_bindings(&content);
        let view = OCamlView {
            content,
            truncated,
            bindings,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The OCaml plugin's presentation half.
#[derive(Debug, Default)]
pub struct OCamlPresentation;

impl PluginPresentation for OCamlPresentation {
    fn name(&self) -> &'static str {
        "ocaml"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: OCamlView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.bindings.is_empty() {
            lines.push(format!("bindings: {}", view.bindings.join(", ")));
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
    use super::{MAX_VIEW_BYTES, OCamlCore, OCamlPresentation, OCamlView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-ocaml-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_an_ocaml_shebang_line_as_ocaml() {
        assert!(OCamlCore.sniff(b"#!/usr/bin/env ocaml\nprint_string \"hi\"\n"));
        assert!(OCamlCore.sniff(b"#!/usr/bin/ocamlrun ocaml\nprint_string \"hi\"\n"));
    }

    #[test]
    fn sniffs_common_ocaml_markers_as_ocaml() {
        assert!(OCamlCore.sniff(b"let () =\n  Printf.printf \"hi\\n\"\n"));
        assert!(OCamlCore.sniff(b"let greet name = \"Hello, \" ^ name\n;;\n"));
        assert!(OCamlCore.sniff(b"module type S = sig\n  val greet : string -> string\nend\n"));
        assert!(OCamlCore.sniff(b"let rec fact n = if n = 0 then 1 else n * fact (n - 1)\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_ocaml() {
        assert!(!OCamlCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!OCamlCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!OCamlCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!OCamlCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!OCamlCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!OCamlCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!OCamlCore.sniff(b"use std::io;\nfn main() {\n    println!(\"hi\");\n}\n"));
        assert!(!OCamlCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!OCamlCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!OCamlCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!OCamlCore.sniff(b"fun greet() {\n    println(\"hi\")\n}\n"));
        assert!(!OCamlCore.sniff(b"use strict;\nuse warnings;\n\nsub greet {\n    return 1;\n}\n"));
        assert!(!OCamlCore.sniff(b"x <- 5\nresult <- data %>% filter(x > 1)\n"));
        assert!(!OCamlCore.sniff(b"greet :: String -> String\ngreet name = name\n"));
        assert!(!OCamlCore.sniff(
            b"module Greeter\n\nlet greet name =\n    \"Hello, \" + name + \"!\"\n\n[<EntryPoint>]\nlet main argv =\n    printfn \"%s\" (greet \"world\")\n    0\n"
        ));
        assert!(!OCamlCore.sniff(b"just a regular line of text\n"));
        assert!(!OCamlCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_ocaml_file_and_extracts_bindings() {
        let path = unique_temp_file("greeter.ml");
        std::fs::write(
            &path,
            "let greet name =\n  \"Hello, \" ^ name ^ \"!\"\n;;\n\nlet () =\n  Printf.printf \"%s\\n\" (greet \"world\")\n;;\n",
        )
        .unwrap();

        let data = OCamlCore.view(&path).unwrap();
        let view: OCamlView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.bindings, vec!["greet"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.ml");
        let mut content = "let () =\n  Printf.printf \"hi\\n\"\n;;\n".to_owned();
        content.push_str(&"(* filler *) ".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = OCamlCore.view(&path).unwrap();
        let view: OCamlView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_bindings_and_content() {
        let data = serde_json::to_value(OCamlView {
            content: "let greet name =\n  name".to_owned(),
            truncated: false,
            bindings: vec!["greet".to_owned()],
        })
        .unwrap();

        let lines = OCamlPresentation.present(&data);

        assert_eq!(lines, vec!["bindings: greet", "let greet name =", "  name"]);
    }
}
