//! F# file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`FSharpCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FSharpView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `let [rec] name ... = ` bindings found in the
    /// content, in source order.
    pub functions: Vec<String>,
}

/// Extracts the name from a top-level `let [rec] name ... = ` binding line,
/// e.g. `function_name("let greet name = name")` returns `Some("greet")`.
fn function_name(line: &str) -> Option<&str> {
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
fn parse_definitions(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| function_name(line.trim_start()))
        .map(str::to_owned)
        .collect()
}

/// Whether `text`'s first line is a shebang naming the `dotnet fsi` or
/// `fsharpi` F# Interactive interpreter.
fn has_fsharp_shebang(text: &str) -> bool {
    text.lines().next().is_some_and(|line| {
        line.starts_with("#!") && (line.contains("fsi") || line.contains("fsharpi"))
    })
}

/// Whether `text` looks like F# source: markers not used by this project's
/// other source-language plugins. `[<EntryPoint>]` is F#'s attribute
/// marking the console entry point function, a bracket-angle attribute
/// syntax no sibling plugin sniffs; `printfn ` is F#'s formatted-print
/// function, distinct from OCaml's and C's `printf`. This plugin
/// deliberately does not sniff `<-` (F#'s mutable-assignment operator) or
/// bare `let rec `, since the R plugin already claims `<-` for assignment
/// and a future OCaml plugin would also use `let rec` — see the R plugin's
/// own note on this overlap.
fn has_fsharp_syntax(text: &str) -> bool {
    text.contains("[<EntryPoint>]") || text.contains("printfn ")
}

/// The F# plugin's core half.
#[derive(Debug, Default)]
pub struct FSharpCore;

impl PluginCore for FSharpCore {
    fn name(&self) -> &'static str {
        "fsharp"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_fsharp_shebang(text) || has_fsharp_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let functions = parse_definitions(&content);
        let view = FSharpView {
            content,
            truncated,
            functions,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The F# plugin's presentation half.
#[derive(Debug, Default)]
pub struct FSharpPresentation;

impl PluginPresentation for FSharpPresentation {
    fn name(&self) -> &'static str {
        "fsharp"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: FSharpView = match serde_json::from_value(data.clone()) {
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
    use super::{FSharpCore, FSharpPresentation, FSharpView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-fsharp-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_dotnet_fsi_shebang_line_as_fsharp() {
        assert!(FSharpCore.sniff(b"#!/usr/bin/env -S dotnet fsi\nprintfn \"hi\"\n"));
        assert!(FSharpCore.sniff(b"#!/usr/bin/env fsharpi\nprintfn \"hi\"\n"));
    }

    #[test]
    fn sniffs_common_fsharp_markers_as_fsharp() {
        assert!(FSharpCore.sniff(b"[<EntryPoint>]\nlet main argv =\n    printfn \"hi\"\n    0\n"));
        assert!(FSharpCore.sniff(b"let greet name =\n    printfn \"Hello, %s\" name\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_fsharp() {
        assert!(!FSharpCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!FSharpCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!FSharpCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!FSharpCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!FSharpCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!FSharpCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!FSharpCore.sniff(b"use std::io;\nfn main() {\n    println!(\"hi\");\n}\n"));
        assert!(!FSharpCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!FSharpCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!FSharpCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!FSharpCore.sniff(b"fun greet() {\n    println(\"hi\")\n}\n"));
        assert!(
            !FSharpCore.sniff(b"use strict;\nuse warnings;\n\nsub greet {\n    return 1;\n}\n")
        );
        assert!(!FSharpCore.sniff(b"x <- 5\nresult <- data %>% filter(x > 1)\n"));
        assert!(!FSharpCore.sniff(b"greet :: String -> String\ngreet name = name\n"));
        assert!(!FSharpCore.sniff(b"just a regular line of text\n"));
        assert!(!FSharpCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_fsharp_file_and_extracts_definitions() {
        let path = unique_temp_file("Greeter.fs");
        std::fs::write(
            &path,
            "module Greeter\n\nlet greet name =\n    \"Hello, \" + name + \"!\"\n\n[<EntryPoint>]\nlet main argv =\n    printfn \"%s\" (greet \"world\")\n    0\n",
        )
        .unwrap();

        let data = FSharpCore.view(&path).unwrap();
        let view: FSharpView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.functions, vec!["greet", "main"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("Large.fs");
        let mut content = "[<EntryPoint>]\nlet main argv =\n".to_owned();
        content.push_str(&"    // ".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = FSharpCore.view(&path).unwrap();
        let view: FSharpView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_functions_and_content() {
        let data = serde_json::to_value(FSharpView {
            content: "let greet name =\n    name".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
        })
        .unwrap();

        let lines = FSharpPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["functions: greet", "let greet name =", "    name"]
        );
    }
}
