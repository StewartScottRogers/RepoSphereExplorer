//! Julia file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`JuliaCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JuliaView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `function name(...)` definitions found in the
    /// content.
    pub functions: Vec<String>,
    /// Names of top-level `struct Name`/`mutable struct Name` definitions
    /// found in the content.
    pub structs: Vec<String>,
}

/// Extracts the name from a line that looks like a top-level Julia function
/// definition, e.g. `function greet(name::String)` or
/// `function main()`.
fn parse_function_name(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("function ")?;
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Extracts the name from a line that looks like a top-level Julia struct
/// definition, e.g. `struct Point` or `mutable struct Counter`.
fn parse_struct_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let trimmed = trimmed.strip_prefix("mutable ").unwrap_or(trimmed);
    let rest = trimmed.strip_prefix("struct ")?;
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level function and struct names out of `content`, in source
/// order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut structs = Vec::new();
    for line in content.lines() {
        if let Some(name) = parse_struct_name(line) {
            structs.push(name.to_owned());
        } else if let Some(name) = parse_function_name(line) {
            functions.push(name.to_owned());
        }
    }
    (functions, structs)
}

/// Whether `text`'s first line is a shebang naming the `julia` interpreter.
fn has_julia_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with("#!") && line.contains("julia"))
}

/// Whether `text` looks like Julia source: a bare `using ` import line, the
/// `import Base:` method-extension form, or a `Vector{`/`Dict{`/`Array{`
/// curly-brace parametric type instantiation — markers not used by any
/// sibling plugin. A bare `using ` line is also produced by a C# `using
/// System;` line, so this plugin is placed after `csharp` in `CORE_PLUGINS`,
/// where genuine C# files are already claimed by their own stronger markers
/// first.
fn has_julia_syntax(text: &str) -> bool {
    text.lines()
        .any(|line| line.trim_start().starts_with("using "))
        || text.contains("import Base:")
        || text.contains("Vector{")
        || text.contains("Dict{")
        || text.contains("Array{")
}

/// The Julia plugin's core half.
#[derive(Debug, Default)]
pub struct JuliaCore;

impl PluginCore for JuliaCore {
    fn name(&self) -> &'static str {
        "julia"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_julia_shebang(text) || has_julia_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, structs) = parse_definitions(&content);
        let view = JuliaView {
            content,
            truncated,
            functions,
            structs,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Julia plugin's presentation half.
#[derive(Debug, Default)]
pub struct JuliaPresentation;

impl PluginPresentation for JuliaPresentation {
    fn name(&self) -> &'static str {
        "julia"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: JuliaView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.structs.is_empty() {
            lines.push(format!("structs: {}", view.structs.join(", ")));
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
    use super::{JuliaCore, JuliaPresentation, JuliaView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-julia-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_julia_shebang_line_as_julia() {
        assert!(JuliaCore.sniff(b"#!/usr/bin/env julia\n\nprintln(\"hi\")\n"));
    }

    #[test]
    fn sniffs_common_julia_markers_as_julia() {
        assert!(JuliaCore.sniff(b"using LinearAlgebra\n\nfunction main()\nend\n"));
        assert!(JuliaCore.sniff(b"import Base: +, -\n"));
        assert!(JuliaCore.sniff(b"xs = Vector{Int64}()\n"));
        assert!(JuliaCore.sniff(b"counts = Dict{String, Int}()\n"));
        assert!(JuliaCore.sniff(b"grid = Array{Float64}(undef, 3, 3)\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_julia() {
        assert!(!JuliaCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!JuliaCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!JuliaCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!JuliaCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!JuliaCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!JuliaCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!JuliaCore.sniff(b"require 'json'\nputs 'hi'\n"));
        assert!(!JuliaCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!JuliaCore.sniff(b"just a regular line of text\n"));
        assert!(!JuliaCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_julia_file_and_extracts_definitions() {
        let path = unique_temp_file("greeter.jl");
        std::fs::write(
            &path,
            "using Printf\n\nstruct Greeter\n    name::String\nend\n\nfunction greet(g::Greeter)\n    @printf(\"Hello, %s!\\n\", g.name)\nend\n\nfunction main()\n    g = Greeter(\"world\")\n    greet(g)\nend\n\nmain()\n",
        )
        .unwrap();

        let data = JuliaCore.view(&path).unwrap();
        let view: JuliaView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.structs, vec!["Greeter"]);
        assert_eq!(view.functions, vec!["greet", "main"]);
        assert!(view.content.contains("Hello, %s!"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.jl");
        let mut content = "function pad()\n".to_owned();
        content.push_str(&"#".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = JuliaCore.view(&path).unwrap();
        let view: JuliaView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_structs_functions_and_content() {
        let data = serde_json::to_value(JuliaView {
            content: "struct A\nend".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            structs: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = JuliaPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["structs: A", "functions: greet", "struct A", "end"]
        );
    }
}
