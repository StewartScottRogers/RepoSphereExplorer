//! Tcl file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// Interpreter names recognised by [`shebang_interpreter`] as Tcl.
const TCL_INTERPRETERS: &[&str] = &["tclsh", "wish"];

/// View data produced by [`TclCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TclView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `proc` declarations found in the content.
    pub procs: Vec<String>,
    /// Names of top-level `namespace eval` blocks found in the content.
    pub namespaces: Vec<String>,
}

/// Extracts the interpreter name from `line`, a shebang line's remainder
/// after `#!`, handling both a direct path (`/usr/bin/tclsh`) and an
/// `env`-indirected one (`/usr/bin/env tclsh`).
fn shebang_interpreter(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("#!")?.trim();
    let mut parts = rest.split_whitespace();
    let first = parts.next()?;
    if first.ends_with("/env") || first == "env" {
        parts.next()
    } else {
        Some(first.rsplit('/').next().unwrap_or(first))
    }
}

/// Whether `text`'s first line is a shebang naming a known Tcl interpreter
/// (see [`TCL_INTERPRETERS`]).
fn has_tcl_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .and_then(shebang_interpreter)
        .is_some_and(|name| TCL_INTERPRETERS.contains(&name))
}

/// Whether `text` looks like Tcl source: markers not used by this project's
/// other source-language plugins. A top-level `proc name {args} {` opener
/// ends in a bare `{` (unlike the Nim plugin's `proc name(...): type =`,
/// which ends in `=`); `namespace eval Name {` and `package require Name`
/// are Tcl's own vocabulary, distinct from the Go plugin's bare
/// `package main` and the Perl plugin's semicolon-terminated
/// `package Name;`.
fn has_tcl_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim();
        (trimmed.starts_with("proc ") && trimmed.ends_with('{'))
            || trimmed.starts_with("namespace eval ")
            || trimmed.starts_with("package require ")
    })
}

/// Extracts the identifier that follows `keyword` at the start of `line`,
/// e.g. `top_level_name("proc greet {name} {", "proc ")` returns
/// `Some("greet")`.
fn top_level_name<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(keyword)?;
    let is_name_char = |ch: char| ch.is_alphanumeric() || ch == '_' || ch == ':';
    let end = rest.find(|ch| !is_name_char(ch)).unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level proc and namespace names out of `content`, in source
/// order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut procs = Vec::new();
    let mut namespaces = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        if let Some(name) = top_level_name(trimmed, "proc ") {
            procs.push(name.to_owned());
        } else if let Some(name) = top_level_name(trimmed, "namespace eval ") {
            namespaces.push(name.to_owned());
        }
    }
    (procs, namespaces)
}

/// The Tcl plugin's core half.
#[derive(Debug, Default)]
pub struct TclCore;

impl PluginCore for TclCore {
    fn name(&self) -> &'static str {
        "tcl"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_tcl_shebang(text) || has_tcl_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (procs, namespaces) = parse_definitions(&content);
        let view = TclView {
            content,
            truncated,
            procs,
            namespaces,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Tcl plugin's presentation half.
#[derive(Debug, Default)]
pub struct TclPresentation;

impl PluginPresentation for TclPresentation {
    fn name(&self) -> &'static str {
        "tcl"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: TclView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.namespaces.is_empty() {
            lines.push(format!("namespaces: {}", view.namespaces.join(", ")));
        }
        if !view.procs.is_empty() {
            lines.push(format!("procs: {}", view.procs.join(", ")));
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
    use super::{MAX_VIEW_BYTES, TclCore, TclPresentation, TclView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-tcl-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_tcl_shebang_line_as_tcl() {
        assert!(TclCore.sniff(b"#!/usr/bin/tclsh\nputs hi\n"));
        assert!(TclCore.sniff(b"#!/usr/bin/env tclsh\nputs hi\n"));
        assert!(TclCore.sniff(b"#!/usr/bin/env wish\nputs hi\n"));
    }

    #[test]
    fn sniffs_common_tcl_markers_as_tcl() {
        assert!(TclCore.sniff(b"proc greet {name} {\n    puts \"Hello, $name!\"\n}\n"));
        assert!(TclCore.sniff(b"namespace eval Greeter {\n    variable count 0\n}\n"));
        assert!(TclCore.sniff(b"package require Tcl 8.6\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_tcl() {
        assert!(!TclCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!TclCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!TclCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!TclCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!TclCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!TclCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!TclCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!TclCore.sniff(b"package Greeter;\n\nsub greet {\n    return 1;\n}\n"));
        assert!(!TclCore.sniff(b"proc greet(name: string): string =\n  result = name\n"));
        assert!(!TclCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!TclCore.sniff(b"just a regular line of text\n"));
        assert!(!TclCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_tcl_file_and_extracts_definitions() {
        let path = unique_temp_file("greeter.tcl");
        std::fs::write(
            &path,
            "namespace eval Greeter {\n    variable count 0\n}\n\nproc greet {name} {\n    puts \"Hello, $name!\"\n}\n\ngreet world\n",
        )
        .unwrap();

        let data = TclCore.view(&path).unwrap();
        let view: TclView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.namespaces, vec!["Greeter"]);
        assert_eq!(view.procs, vec!["greet"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.tcl");
        let mut content = "package require Tcl 8.6\n".to_owned();
        content.push_str(&"#".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = TclCore.view(&path).unwrap();
        let view: TclView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_namespaces_procs_and_content() {
        let data = serde_json::to_value(TclView {
            content: "namespace eval A {\n}\nproc greet {} {\n}".to_owned(),
            truncated: false,
            procs: vec!["greet".to_owned()],
            namespaces: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = TclPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "namespaces: A",
                "procs: greet",
                "namespace eval A {",
                "}",
                "proc greet {} {",
                "}"
            ]
        );
    }
}
