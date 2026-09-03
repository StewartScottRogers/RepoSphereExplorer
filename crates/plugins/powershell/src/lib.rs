//! PowerShell file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// Interpreter names recognised by [`shebang_interpreter`] as PowerShell.
const POWERSHELL_INTERPRETERS: &[&str] = &["pwsh", "powershell"];

/// View data produced by [`PowerShellCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerShellView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level function declarations found in the content.
    pub functions: Vec<String>,
}

/// Extracts the interpreter name from `line`, a shebang line's remainder
/// after `#!`, handling both a direct path (`/usr/bin/pwsh`) and an
/// `env`-indirected one (`/usr/bin/env pwsh`).
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

/// Whether `text`'s first line is a shebang naming a known PowerShell
/// interpreter (see [`POWERSHELL_INTERPRETERS`]).
fn has_powershell_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .and_then(shebang_interpreter)
        .is_some_and(|name| POWERSHELL_INTERPRETERS.contains(&name))
}

/// Whether `text` looks like PowerShell source: markers not used by this
/// project's other source-language plugins. `<#` opens a PowerShell
/// block/help comment, a syntax no sibling plugin uses; `[CmdletBinding()]`
/// and a `param(`/`param (` block are PowerShell's advanced-function
/// declaration syntax; `$PSScriptRoot`, `$PSVersionTable`, and
/// `$ErrorActionPreference` are PowerShell automatic/preference variables
/// that no other sniffed language defines.
fn has_powershell_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("<#")
            || trimmed.eq_ignore_ascii_case("[CmdletBinding()]")
            || trimmed.starts_with("param(")
            || trimmed.starts_with("param (")
    }) || text.contains("$PSScriptRoot")
        || text.contains("$PSVersionTable")
        || text.contains("$ErrorActionPreference")
}

/// Extracts the identifier from a top-level `function Name` declaration
/// line, whose name may contain a `Verb-Noun` hyphen per PowerShell's
/// cmdlet-naming convention.
fn function_name(line: &str) -> Option<&str> {
    let is_name_char = |ch: char| ch.is_alphanumeric() || ch == '_' || ch == '-';
    let rest = line.strip_prefix("function ")?.trim_start();
    let end = rest.find(|ch| !is_name_char(ch)).unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level function names out of `content`, in source order.
fn parse_definitions(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| function_name(line.trim_start()))
        .map(str::to_owned)
        .collect()
}

/// The PowerShell plugin's core half.
#[derive(Debug, Default)]
pub struct PowerShellCore;

impl PluginCore for PowerShellCore {
    fn name(&self) -> &'static str {
        "powershell"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_powershell_shebang(text) || has_powershell_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let functions = parse_definitions(&content);
        let view = PowerShellView {
            content,
            truncated,
            functions,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The PowerShell plugin's presentation half.
#[derive(Debug, Default)]
pub struct PowerShellPresentation;

impl PluginPresentation for PowerShellPresentation {
    fn name(&self) -> &'static str {
        "powershell"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: PowerShellView = match serde_json::from_value(data.clone()) {
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
    use super::{MAX_VIEW_BYTES, PowerShellCore, PowerShellPresentation, PowerShellView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-powershell-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_powershell_shebang_line_as_powershell() {
        assert!(PowerShellCore.sniff(b"#!/usr/bin/pwsh\nWrite-Host hi\n"));
        assert!(PowerShellCore.sniff(b"#!/usr/bin/env pwsh\nWrite-Host hi\n"));
        assert!(PowerShellCore.sniff(b"#!/usr/bin/env powershell\nWrite-Host hi\n"));
    }

    #[test]
    fn sniffs_common_powershell_markers_as_powershell() {
        assert!(PowerShellCore.sniff(b"<#\n.SYNOPSIS\n  Does a thing.\n#>\n"));
        assert!(PowerShellCore.sniff(b"[CmdletBinding()]\nparam(\n    [string]$Name\n)\n"));
        assert!(PowerShellCore.sniff(b"param (\n    [string]$Name\n)\n"));
        assert!(PowerShellCore.sniff(b"$root = $PSScriptRoot\n"));
        assert!(PowerShellCore.sniff(b"Write-Host $PSVersionTable.PSVersion\n"));
        assert!(PowerShellCore.sniff(b"$ErrorActionPreference = 'Stop'\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_powershell() {
        assert!(!PowerShellCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!PowerShellCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!PowerShellCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!PowerShellCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!PowerShellCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!PowerShellCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!PowerShellCore.sniff(b"if [ -f \"$1\" ]; then\n    echo found\nfi\n"));
        assert!(!PowerShellCore.sniff(b"#!/bin/bash\necho hi\n"));
        assert!(!PowerShellCore.sniff(b"use strict;\nmy $name = 'world';\n"));
        assert!(!PowerShellCore.sniff(b"const greeting = `hello ${name}`;\n"));
        assert!(!PowerShellCore.sniff(b"just a regular line of text\n"));
        assert!(!PowerShellCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_powershell_script_and_extracts_definitions() {
        let path = unique_temp_file("greeter.ps1");
        std::fs::write(
            &path,
            "[CmdletBinding()]\nparam(\n    [string]$Name = 'world'\n)\n\nfunction Get-Greeting {\n    \"Hello, $Name!\"\n}\n\nfunction Show-Farewell($who) {\n    \"Bye, $who!\"\n}\n\nWrite-Host (Get-Greeting)\n",
        )
        .unwrap();

        let data = PowerShellCore.view(&path).unwrap();
        let view: PowerShellView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.functions, vec!["Get-Greeting", "Show-Farewell"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.ps1");
        let mut content = "[CmdletBinding()]\n".to_owned();
        content.push_str(&"#".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = PowerShellCore.view(&path).unwrap();
        let view: PowerShellView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_functions_and_content() {
        let data = serde_json::to_value(PowerShellView {
            content: "function Get-Greeting {\n    \"hi\"\n}".to_owned(),
            truncated: false,
            functions: vec!["Get-Greeting".to_owned()],
        })
        .unwrap();

        let lines = PowerShellPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "functions: Get-Greeting",
                "function Get-Greeting {",
                "    \"hi\"",
                "}"
            ]
        );
    }
}
