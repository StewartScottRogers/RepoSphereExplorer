//! Shell script file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// Interpreter names recognised by [`shebang_interpreter`] as a shell.
const SHELL_INTERPRETERS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh"];

/// View data produced by [`ShellCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level function declarations found in the content.
    pub functions: Vec<String>,
}

/// Extracts the interpreter name from `line`, a shebang line's remainder
/// after `#!`, handling both a direct path (`/bin/bash`) and an
/// `env`-indirected one (`/usr/bin/env bash`).
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

/// Whether `text`'s first line is a shebang naming a known shell
/// interpreter (see [`SHELL_INTERPRETERS`]).
fn has_shell_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .and_then(shebang_interpreter)
        .is_some_and(|name| SHELL_INTERPRETERS.contains(&name))
}

/// Whether `text` looks like shell script source: markers not used by this
/// project's other source-language plugins. `elif`, `esac`, a bare `fi` or
/// `done` line, and a `case ... in` opener are POSIX shell keywords no
/// sibling plugin sniffs for; `$(` command substitution is likewise a shell
/// idiom (unlike `${...}` parameter expansion, which JavaScript/TypeScript
/// template literals also use, so it is deliberately not checked here).
fn has_shell_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == "fi"
            || trimmed == "done"
            || trimmed == "esac"
            || trimmed == "then"
            || trimmed.starts_with("elif ")
            || trimmed.ends_with("; then")
            || (trimmed.starts_with("case ") && trimmed.ends_with(" in"))
    }) || text.contains("$(")
}

/// Extracts the identifier from a top-level function declaration line, in
/// either the `function name` form or the `name()` form.
fn function_name(line: &str) -> Option<&str> {
    let is_name_char = |ch: char| ch.is_alphanumeric() || ch == '_' || ch == '-';
    if let Some(rest) = line.strip_prefix("function ") {
        let rest = rest.trim_start();
        let end = rest.find(|ch| !is_name_char(ch)).unwrap_or(rest.len());
        return (end > 0).then(|| &rest[..end]);
    }
    let end = line.find("()")?;
    let name = &line[..end];
    (!name.is_empty() && name.chars().all(is_name_char)).then_some(name)
}

/// Parses top-level function names out of `content`, in source order.
fn parse_definitions(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| function_name(line.trim_start()))
        .map(str::to_owned)
        .collect()
}

/// The shell script plugin's core half.
#[derive(Debug, Default)]
pub struct ShellCore;

impl PluginCore for ShellCore {
    fn name(&self) -> &'static str {
        "shell"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_shell_shebang(text) || has_shell_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let functions = parse_definitions(&content);
        let view = ShellView {
            content,
            truncated,
            functions,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The shell script plugin's presentation half.
#[derive(Debug, Default)]
pub struct ShellPresentation;

impl PluginPresentation for ShellPresentation {
    fn name(&self) -> &'static str {
        "shell"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: ShellView = match serde_json::from_value(data.clone()) {
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
    use super::{MAX_VIEW_BYTES, ShellCore, ShellPresentation, ShellView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-shell-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_shell_shebang_line_as_shell() {
        assert!(ShellCore.sniff(b"#!/bin/bash\necho hi\n"));
        assert!(ShellCore.sniff(b"#!/bin/sh\necho hi\n"));
        assert!(ShellCore.sniff(b"#!/usr/bin/env bash\necho hi\n"));
        assert!(ShellCore.sniff(b"#!/usr/bin/env zsh\necho hi\n"));
    }

    #[test]
    fn sniffs_common_shell_markers_as_shell() {
        assert!(ShellCore.sniff(b"if [ -f \"$1\" ]; then\n    echo found\nfi\n"));
        assert!(ShellCore.sniff(b"case \"$1\" in\n    a) echo a ;;\n    *) echo other ;;\nesac\n"));
        assert!(ShellCore.sniff(b"for f in *.txt; do\n    echo \"$f\"\ndone\n"));
        assert!(ShellCore.sniff(b"name=$(basename \"$path\")\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_shell() {
        assert!(!ShellCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!ShellCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!ShellCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!ShellCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!ShellCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!ShellCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!ShellCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!ShellCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!ShellCore.sniff(b"use strict;\nmy $name = 'world';\n"));
        assert!(!ShellCore.sniff(b"const greeting = `hello ${name}`;\n"));
        assert!(!ShellCore.sniff(b"just a regular line of text\n"));
        assert!(!ShellCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_shell_script_and_extracts_definitions() {
        let path = unique_temp_file("greeter.sh");
        std::fs::write(
            &path,
            "#!/bin/bash\nset -euo pipefail\n\ngreet() {\n    echo \"Hello, $1!\"\n}\n\nfunction farewell {\n    echo \"Bye, $1!\"\n}\n\ngreet world\n",
        )
        .unwrap();

        let data = ShellCore.view(&path).unwrap();
        let view: ShellView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.functions, vec!["greet", "farewell"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.sh");
        let mut content = "#!/bin/bash\n".to_owned();
        content.push_str(&"#".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = ShellCore.view(&path).unwrap();
        let view: ShellView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_functions_and_content() {
        let data = serde_json::to_value(ShellView {
            content: "greet() {\n    echo hi\n}".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
        })
        .unwrap();

        let lines = ShellPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["functions: greet", "greet() {", "    echo hi", "}"]
        );
    }
}
