//! Haskell file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`HaskellCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HaskellView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `name :: Type` signatures found in the content,
    /// in source order.
    pub functions: Vec<String>,
}

/// Extracts the name from a top-level `name :: Type` signature line, e.g.
/// `function_name("greet :: String -> String")` returns `Some("greet")`.
fn function_name(line: &str) -> Option<&str> {
    let (name, _rest) = line.split_once("::")?;
    let name = name.trim();
    let is_name_char = |ch: char| ch.is_alphanumeric() || ch == '_' || ch == '\'';
    let starts_ok = name
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_lowercase() || ch == '_');
    if name.is_empty() || !starts_ok || !name.chars().all(is_name_char) {
        return None;
    }
    Some(name)
}

/// Parses top-level type signature names out of `content`, in source order.
fn parse_definitions(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| function_name(line.trim_start()))
        .map(str::to_owned)
        .collect()
}

/// Whether `text`'s first line is a shebang naming the `runghc` or
/// `runhaskell` interpreter.
fn has_haskell_shebang(text: &str) -> bool {
    text.lines().next().is_some_and(|line| {
        line.starts_with("#!") && (line.contains("runghc") || line.contains("runhaskell"))
    })
}

/// Whether `text` looks like Haskell source: markers not used by this
/// project's other source-language plugins. A `{-# LANGUAGE` pragma opens a
/// GHC language extension directive; `import qualified ` is Haskell's
/// qualified-import syntax, distinct from every sibling plugin's own
/// `import`/`require` markers; ` :: ` (surrounded by spaces) is a top-level
/// type signature, distinct from the Rust plugin's `std::` and the C++
/// plugin's `std::`/`::` scope resolution, neither of which has surrounding
/// spaces. This project has no path/extension-based dispatch (per the C
/// plugin's note), so this plugin deliberately does not sniff the `<-`
/// operator that the R plugin already claims for assignment, since Haskell
/// also uses `<-` for monadic bind in `do` notation (see the R plugin's own
/// note on that overlap).
fn has_haskell_syntax(text: &str) -> bool {
    text.contains("{-# LANGUAGE") || text.contains("import qualified ") || text.contains(" :: ")
}

/// The Haskell plugin's core half.
#[derive(Debug, Default)]
pub struct HaskellCore;

impl PluginCore for HaskellCore {
    fn name(&self) -> &'static str {
        "haskell"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_haskell_shebang(text) || has_haskell_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let functions = parse_definitions(&content);
        let view = HaskellView {
            content,
            truncated,
            functions,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Haskell plugin's presentation half.
#[derive(Debug, Default)]
pub struct HaskellPresentation;

impl PluginPresentation for HaskellPresentation {
    fn name(&self) -> &'static str {
        "haskell"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: HaskellView = match serde_json::from_value(data.clone()) {
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
    use super::{HaskellCore, HaskellPresentation, HaskellView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-haskell-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_runghc_shebang_line_as_haskell() {
        assert!(HaskellCore.sniff(b"#!/usr/bin/env runghc\nmain = putStrLn \"hi\"\n"));
        assert!(HaskellCore.sniff(b"#!/usr/bin/runhaskell\nmain = putStrLn \"hi\"\n"));
    }

    #[test]
    fn sniffs_common_haskell_markers_as_haskell() {
        assert!(HaskellCore.sniff(b"{-# LANGUAGE OverloadedStrings #-}\nmain = pure ()\n"));
        assert!(HaskellCore.sniff(b"import qualified Data.Map as Map\n"));
        assert!(HaskellCore.sniff(b"greet :: String -> String\ngreet name = name\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_haskell() {
        assert!(!HaskellCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!HaskellCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!HaskellCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!HaskellCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!HaskellCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!HaskellCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!HaskellCore.sniff(b"use std::io;\nfn main() {\n    println!(\"hi\");\n}\n"));
        assert!(!HaskellCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!HaskellCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!HaskellCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!HaskellCore.sniff(b"fun greet() {\n    println(\"hi\")\n}\n"));
        assert!(
            !HaskellCore.sniff(b"use strict;\nuse warnings;\n\nsub greet {\n    return 1;\n}\n")
        );
        assert!(!HaskellCore.sniff(b"x <- 5\nresult <- data %>% filter(x > 1)\n"));
        assert!(!HaskellCore.sniff(b"just a regular line of text\n"));
        assert!(!HaskellCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_haskell_file_and_extracts_definitions() {
        let path = unique_temp_file("Greeter.hs");
        std::fs::write(
            &path,
            "module Main where\n\ngreet :: String -> String\ngreet name = \"Hello, \" ++ name ++ \"!\"\n\nmain :: IO ()\nmain = putStrLn (greet \"world\")\n",
        )
        .unwrap();

        let data = HaskellCore.view(&path).unwrap();
        let view: HaskellView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.functions, vec!["greet", "main"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("Large.hs");
        let mut content = "main :: IO ()\n".to_owned();
        content.push_str(&"-- ".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = HaskellCore.view(&path).unwrap();
        let view: HaskellView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_functions_and_content() {
        let data = serde_json::to_value(HaskellView {
            content: "greet :: String -> String\ngreet name = name".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
        })
        .unwrap();

        let lines = HaskellPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "functions: greet",
                "greet :: String -> String",
                "greet name = name"
            ]
        );
    }
}
