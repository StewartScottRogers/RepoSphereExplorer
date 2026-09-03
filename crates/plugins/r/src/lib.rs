//! R file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`RCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `name <- function(...)` definitions found in the
    /// content, in source order.
    pub functions: Vec<String>,
}

/// Extracts the name from a top-level `name <- function(` assignment line,
/// e.g. `function_name("greet <- function(who) {")` returns `Some("greet")`.
fn function_name(line: &str) -> Option<&str> {
    let (name, rest) = line.split_once("<-")?;
    let name = name.trim();
    let is_name_char = |ch: char| ch.is_alphanumeric() || ch == '_' || ch == '.';
    if name.is_empty() || !name.chars().all(is_name_char) {
        return None;
    }
    rest.trim_start().starts_with("function(").then_some(name)
}

/// Parses top-level function names out of `content`, in source order.
fn parse_definitions(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| function_name(line.trim_start()))
        .map(str::to_owned)
        .collect()
}

/// Whether `text`'s first line is a shebang naming the `Rscript`
/// interpreter.
fn has_rscript_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with("#!") && line.contains("Rscript"))
}

/// Whether `text` looks like R source: markers not used by this project's
/// other source-language plugins. The `<-` assignment operator is R's
/// idiomatic assignment form (as opposed to `=`) and is not sniffed by any
/// sibling plugin; `%>%` is the magrittr/tidyverse pipe operator; a
/// top-level `library(` or `require(...)` call loads a package. This
/// project has no path/extension-based dispatch (per the C plugin's note),
/// so a future Haskell, OCaml, or F# plugin sniffing `<-` for a monadic
/// bind or list-comprehension generator must avoid this same marker, or be
/// ordered after `r`.
fn has_r_syntax(text: &str) -> bool {
    text.contains("<-")
        || text.contains("%>%")
        || text
            .lines()
            .any(|line| line.trim_start().starts_with("library("))
}

/// The R plugin's core half.
#[derive(Debug, Default)]
pub struct RCore;

impl PluginCore for RCore {
    fn name(&self) -> &'static str {
        "r"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_rscript_shebang(text) || has_r_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let functions = parse_definitions(&content);
        let view = RView {
            content,
            truncated,
            functions,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The R plugin's presentation half.
#[derive(Debug, Default)]
pub struct RPresentation;

impl PluginPresentation for RPresentation {
    fn name(&self) -> &'static str {
        "r"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: RView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
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
    use super::{MAX_VIEW_BYTES, RCore, RPresentation, RView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-r-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_an_rscript_shebang_line_as_r() {
        assert!(RCore.sniff(b"#!/usr/bin/env Rscript\nprint(\"hi\")\n"));
        assert!(RCore.sniff(b"#!/usr/bin/Rscript\nprint(\"hi\")\n"));
    }

    #[test]
    fn sniffs_common_r_markers_as_r() {
        assert!(RCore.sniff(b"x <- 5\n"));
        assert!(RCore.sniff(b"library(dplyr)\n"));
        assert!(RCore.sniff(b"result <- data %>% filter(x > 1)\n"));
        assert!(RCore.sniff(b"greet <- function(name) {\n  print(name)\n}\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_r() {
        assert!(!RCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!RCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!RCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!RCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!RCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!RCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!RCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!RCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!RCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!RCore.sniff(b"fun greet() {\n    println(\"hi\")\n}\n"));
        assert!(!RCore.sniff(b"use strict;\nuse warnings;\n\nsub greet {\n    return 1;\n}\n"));
        assert!(!RCore.sniff(b"just a regular line of text\n"));
        assert!(!RCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_r_file_and_extracts_definitions() {
        let path = unique_temp_file("greeter.R");
        std::fs::write(
            &path,
            "library(methods)\n\ngreet <- function(name) {\n  paste(\"Hello,\", name)\n}\n\nprint(greet(\"world\"))\n",
        )
        .unwrap();

        let data = RCore.view(&path).unwrap();
        let view: RView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.functions, vec!["greet"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.R");
        let mut content = "x <- 1\n".to_owned();
        content.push_str(&"#".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = RCore.view(&path).unwrap();
        let view: RView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_functions_and_content() {
        let data = serde_json::to_value(RView {
            content: "greet <- function(name) {\n}".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
        })
        .unwrap();

        let lines = RPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["functions: greet", "greet <- function(name) {", "}"]
        );
    }
}
