//! Lua file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`LuaCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuaView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `function`/`local function` declarations found in
    /// the content.
    pub functions: Vec<String>,
}

/// Extracts the identifier declared by a `function name(` or `local
/// function name(` line, including dotted (`M.foo`) and colon
/// (`obj:method`) method names.
fn function_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed
        .strip_prefix("local function ")
        .or_else(|| trimmed.strip_prefix("function "))?
        .trim_start();
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '.' || ch == ':'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level function names out of `content`, in source order.
fn parse_definitions(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(function_name)
        .map(str::to_owned)
        .collect()
}

/// Whether `text`'s first line is a shebang naming a Lua interpreter
/// (`lua`, `lua5.1`, `luajit`, and similar, directly or via `env`).
fn has_lua_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with("#!") && line.contains("lua"))
}

/// Whether `trimmed` opens a Lua comment: `--` followed by a space, the end
/// of the line, or a block-comment `[[` opener. Requiring a space or `[[`
/// after `--` (rather than accepting any `--` prefix) keeps this from
/// matching a leading decrement statement like `--counter;`, which a
/// C-family sibling plugin's source could contain.
fn is_lua_comment(trimmed: &str) -> bool {
    trimmed == "--" || trimmed.starts_with("-- ") || trimmed.starts_with("--[[")
}

/// Whether `text` looks like Lua source: markers not used by this project's
/// other source-language plugins. A `local ` declaration (`local x = 1`,
/// `local function f()`) is Lua's variable/function scoping keyword; a `--`
/// comment opener (see [`is_lua_comment`]) is Lua's only comment syntax; an
/// `elseif ` keyword is Lua's `if`/`elseif` chain, distinct from every
/// sibling's own `elif`. Deliberately does not sniff a bare `end` line,
/// since the Ruby plugin already claims that marker to distinguish itself
/// from Python's overlapping `def`/`class` syntax; nor a line ending in
/// `then`, since the shell plugin already claims a `; then` ending and a
/// bare `then`-ending check without the semicolon would still overlap it.
/// Placed just ahead of `text` in `CORE_PLUGINS`, no ordering constraint
/// against a specific sibling since it has no overlapping markers.
fn has_lua_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("local ") || trimmed.starts_with("elseif ") || is_lua_comment(trimmed)
    })
}

/// The Lua plugin's core half.
#[derive(Debug, Default)]
pub struct LuaCore;

impl PluginCore for LuaCore {
    fn name(&self) -> &'static str {
        "lua"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_lua_shebang(text) || has_lua_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let functions = parse_definitions(&content);
        let view = LuaView {
            content,
            truncated,
            functions,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Lua plugin's presentation half.
#[derive(Debug, Default)]
pub struct LuaPresentation;

impl PluginPresentation for LuaPresentation {
    fn name(&self) -> &'static str {
        "lua"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: LuaView = match serde_json::from_value(data.clone()) {
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
    use super::{LuaCore, LuaPresentation, LuaView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-lua-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_lua_shebang_line_as_lua() {
        assert!(LuaCore.sniff(b"#!/usr/bin/env lua\nprint('hi')\n"));
        assert!(LuaCore.sniff(b"#!/usr/bin/lua5.3\nprint('hi')\n"));
    }

    #[test]
    fn sniffs_common_lua_markers_as_lua() {
        assert!(LuaCore.sniff(b"local x = 1\n"));
        assert!(LuaCore.sniff(b"local function greet()\n  return 1\nend\n"));
        assert!(LuaCore.sniff(b"-- a comment\nprint('hi')\n"));
        assert!(LuaCore.sniff(b"--[[ block comment ]]\nprint('hi')\n"));
        assert!(LuaCore.sniff(b"if x then\n  print('a')\nelseif y then\n  print('b')\nend\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_as_lua() {
        assert!(!LuaCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!LuaCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!LuaCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!LuaCore.sniff(b"const load = require('json');\n"));
        assert!(!LuaCore.sniff(b"interface Named {\n  name: string;\n}\n"));
        assert!(!LuaCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!LuaCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!LuaCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    --counter;\n    return 0;\n}\n"
        ));
        assert!(!LuaCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!LuaCore.sniff(b"if [ \"$1\" = \"x\" ]; then\n  echo hi\nfi\n"));
        assert!(!LuaCore.sniff(b"just a regular line of text\n"));
        assert!(!LuaCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_lua_file_and_extracts_definitions() {
        let path = unique_temp_file("greeter.lua");
        std::fs::write(
            &path,
            "local function greet(name)\n  return 'Hello, ' .. name\nend\n\nfunction M.wave()\n  print('wave')\nend\n\nprint(greet('world'))\n",
        )
        .unwrap();

        let data = LuaCore.view(&path).unwrap();
        let view: LuaView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.functions, vec!["greet", "M.wave"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.lua");
        let mut content = "local function pad()\n".to_owned();
        content.push_str(&"-- ".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = LuaCore.view(&path).unwrap();
        let view: LuaView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_functions_and_content() {
        let data = serde_json::to_value(LuaView {
            content: "local function f()\nend".to_owned(),
            truncated: false,
            functions: vec!["f".to_owned()],
        })
        .unwrap();

        let lines = LuaPresentation.present(&data);

        assert_eq!(lines, vec!["functions: f", "local function f()", "end"]);
    }
}
