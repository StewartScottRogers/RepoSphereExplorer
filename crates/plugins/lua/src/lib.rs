//! Lua file type plugin: core and presentation halves.
//!
//! A Lua file is functions, tables and the names they hang off. This
//! reads the functions with their parameters and whether each is local,
//! global or a method, the local names, the tables, the modules
//! required, the metatables set - and the names assigned without
//! `local`, which every other file in the program can also see.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["lua", "rockspec"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One function the file defines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    /// Its name, including the table it hangs off when it has one.
    pub name: String,
    /// Its parameters, in order.
    pub parameters: Vec<String>,
    /// `local`, `global`, or `method` for one declared with a colon and
    /// so carrying an implicit `self`.
    pub scope: String,
}

/// View data produced by [`LuaCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LuaView {
    /// Every function defined.
    pub functions: Vec<Function>,
    /// The names declared `local` at any depth.
    pub locals: Vec<String>,
    /// The names bound to a table literal.
    pub tables: Vec<String>,
    /// The modules pulled in with `require`.
    pub requires: Vec<String>,
    /// The names passed to `setmetatable`, which is how Lua does
    /// inheritance and operator overloading.
    pub metatables: Vec<String>,
    /// Names assigned without `local`, which in Lua means they are put in
    /// the table every other file in the program can also see.
    pub globals: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The keywords a bare assignment might start with, which are statements
/// rather than the global assignments they otherwise look like.
const KEYWORDS: &[&str] = &[
    "if", "elseif", "else", "for", "while", "repeat", "until", "return", "end", "do", "then",
    "break", "local", "function", "goto",
];

/// `text` with comments removed, long ones included.
///
/// A `--[[ ... ]]` block can hold anything, commented-out code among it,
/// and reading that as code invents functions the file has not got.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("--") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 2..];
        if let Some(long) = after.strip_prefix("[[") {
            match long.find("]]") {
                Some(end) => {
                    rest = &long[end + 2..];
                    continue;
                }
                None => return out,
            }
        }
        match after.find('\n') {
            Some(end) => {
                out.push('\n');
                rest = &after[end + 1..];
            }
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The parameter names between the brackets of `line`, if it has any.
fn parameters_of(line: &str) -> Vec<String> {
    let Some(open) = line.find('(') else {
        return Vec::new();
    };
    let Some(close) = line[open..].find(')').map(|at| at + open) else {
        return Vec::new();
    };
    line[open + 1..close]
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The function `line` declares, if it declares one.
fn function_of(line: &str) -> Option<Function> {
    let (rest, scope) = match line.strip_prefix("local function ") {
        Some(rest) => (rest, "local"),
        None => (line.strip_prefix("function ")?, "global"),
    };
    let name = rest.split('(').next()?.trim().to_owned();
    if name.is_empty() {
        return None;
    }
    Some(Function {
        parameters: parameters_of(line),
        scope: if name.contains(':') { "method" } else { scope }.to_owned(),
        name,
    })
}

/// Every module named by a `require` on `line`.
fn requires_of(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find("require") {
        let after = rest[at + "require".len()..].trim_start();
        let after = after.strip_prefix('(').unwrap_or(after).trim_start();
        if let Some(quote) = after.chars().next().filter(|q| *q == '"' || *q == '\'')
            && let Some(end) = after[1..].find(quote)
        {
            found.push(after[1..=end].to_owned());
        }
        rest = &rest[at + "require".len()..];
    }
    found
}

/// The name assigned on `line`, and whether the line binds a table.
fn assignment_of(line: &str) -> Option<(String, bool)> {
    let (left, right) = line.split_once('=')?;
    // `==`, `<=`, `>=` and `~=` are comparisons, not assignments.
    if right.starts_with('=') || left.ends_with(['=', '<', '>', '~']) {
        return None;
    }
    let name = left.trim().to_owned();
    if name.is_empty() || name.contains([' ', '(', '[']) || KEYWORDS.contains(&name.as_str()) {
        return None;
    }
    Some((name, right.trim().starts_with('{')))
}

/// Everything [`LuaView`] holds, read from `text`.
fn parse(text: &str) -> LuaView {
    let mut view = LuaView {
        functions: Vec::new(),
        locals: Vec::new(),
        tables: Vec::new(),
        requires: Vec::new(),
        metatables: Vec::new(),
        globals: Vec::new(),
        truncated: false,
    };
    for raw in without_comments(text).lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        view.requires.extend(requires_of(line));
        if let Some(at) = line.find("setmetatable(") {
            let inside = &line[at + "setmetatable(".len()..];
            if let Some(first) = inside.split(',').next() {
                view.metatables.push(first.trim().to_owned());
            }
        }
        if let Some(function) = function_of(line) {
            view.functions.push(function);
            continue;
        }
        let (body, is_local) = match line.strip_prefix("local ") {
            Some(rest) => (rest.trim(), true),
            None => (line, false),
        };
        let Some((name, is_table)) = assignment_of(body) else {
            if is_local && !body.contains('=') {
                // `local x` with nothing assigned yet.
                view.locals.push(body.to_owned());
            }
            continue;
        };
        if is_table {
            view.tables.push(name.clone());
        }
        if is_local {
            view.locals.push(name);
        } else if !name.contains('.') && !name.contains(':') {
            // A field of an existing table is not a new global.
            view.globals.push(name);
        }
    }
    view.requires.dedup();
    view.globals.dedup();
    view
}

/// Whether `text` is Lua.
fn looks_like_it(text: &str) -> bool {
    let stripped = without_comments(text);
    let view = parse(text);
    let closes_a_block = stripped.lines().any(|line| line.trim() == "end");
    if (!view.functions.is_empty() || !view.locals.is_empty()) && closes_a_block {
        return true;
    }
    // A rockspec is Lua too, and has none of that: it is a file of
    // top-level table assignments and nothing else.
    let named: Vec<&str> = view.globals.iter().map(String::as_str).collect();
    named.contains(&"package")
        && named.contains(&"version")
        && (named.contains(&"build") || named.contains(&"dependencies"))
}

/// The Lua plugin's core half.
#[derive(Debug, Default)]
pub struct LuaCore;

impl PluginCore for LuaCore {
    fn name(&self) -> &'static str {
        "lua"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        // The declarations are what a reader came for; the bodies are
        // better read in the file itself.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
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

    fn icon(&self) -> Icon {
        Icon {
            label: "LUA",
            tint: 0x0000_007d,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: LuaView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("{} function(s):", view.functions.len()));
        for function in &view.functions {
            lines.push(format!(
                "  {} {}({})",
                function.scope,
                function.name,
                function.parameters.join(", ")
            ));
        }
        if !view.requires.is_empty() {
            lines.push(format!("Requires: {}", view.requires.join(", ")));
        }
        if !view.tables.is_empty() {
            lines.push(format!("Tables: {}", view.tables.join(", ")));
        }
        if !view.metatables.is_empty() {
            lines.push(format!("Given a metatable: {}", view.metatables.join(", ")));
        }
        if !view.locals.is_empty() {
            lines.push(format!("{} local name(s)", view.locals.len()));
        }
        if !view.globals.is_empty() {
            lines.push("Assigned without `local`, so these go in the table every".to_owned());
            lines.push("other file in the program can see and overwrite:".to_owned());
            for name in &view.globals {
                lines.push(format!("  {name}"));
            }
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{LuaCore, LuaPresentation, LuaView, parse, without_comments};
    use plugin_api::{PluginCore, PluginPresentation};

    const MODULE: &str = concat!(
        "-- A small module.\n",
        "local socket = require(\"socket\")\n",
        "local json = require 'json'\n",
        "\n",
        "local M = {}\n",
        "local Cache = {}\n",
        "Cache.__index = Cache\n",
        "\n",
        "--[[\n",
        "function ghost(a, b)\n",
        "    return a + b\n",
        "end\n",
        "]]\n",
        "\n",
        "function Cache.new(size)\n",
        "    local self = setmetatable({}, Cache)\n",
        "    self.size = size\n",
        "    return self\n",
        "end\n",
        "\n",
        "function Cache:get(key)\n",
        "    return self.entries[key]\n",
        "end\n",
        "\n",
        "local function normalise(key)\n",
        "    return key:lower()\n",
        "end\n",
        "\n",
        "leaked = 1\n",
        "\n",
        "return M\n",
    );

    #[test]
    fn sniffs_a_module() {
        assert!(LuaCore.sniff(MODULE.as_bytes()));
    }

    #[test]
    fn sniffs_a_rockspec_which_has_no_functions_at_all() {
        assert!(
            LuaCore.sniff(
                concat!(
                    "package = \"csvstats\"\n",
                    "version = \"1.0-1\"\n",
                    "dependencies = { \"lua >= 5.3\" }\n",
                    "build = { type = \"builtin\" }\n",
                )
                .as_bytes()
            )
        );
    }

    #[test]
    fn does_not_claim_ruby_which_also_ends_its_blocks() {
        assert!(!LuaCore.sniff(b"def add(a, b)\n  a + b\nend\n"));
        assert!(!LuaCore.sniff(b""));
    }

    #[test]
    fn a_long_comment_is_not_code() {
        let stripped = without_comments(MODULE);

        assert!(
            !stripped.contains("ghost"),
            "the commented-out function is not real"
        );
        assert!(
            !parse(MODULE)
                .functions
                .iter()
                .any(|function| function.name == "ghost")
        );
    }

    #[test]
    fn a_colon_declaration_is_a_method() {
        let view = parse(MODULE);

        let get = view
            .functions
            .iter()
            .find(|f| f.name == "Cache:get")
            .unwrap();
        assert_eq!(
            get.scope, "method",
            "a colon declaration carries an implicit self"
        );
        let new = view
            .functions
            .iter()
            .find(|f| f.name == "Cache.new")
            .unwrap();
        assert_eq!(new.scope, "global");
        assert_eq!(new.parameters, vec!["size".to_owned()]);
        assert!(
            view.functions
                .iter()
                .any(|f| f.name == "normalise" && f.scope == "local")
        );
    }

    #[test]
    fn reads_require_with_or_without_brackets() {
        let view = parse(MODULE);

        assert_eq!(view.requires, vec!["socket".to_owned(), "json".to_owned()]);
    }

    #[test]
    fn finds_the_tables_and_the_metatable() {
        let view = parse(MODULE);

        assert!(view.tables.contains(&"M".to_owned()));
        assert!(view.tables.contains(&"Cache".to_owned()));
        assert_eq!(view.metatables, vec!["{}".to_owned()]);
    }

    #[test]
    fn a_field_of_a_table_is_not_a_new_global() {
        let view = parse(MODULE);

        assert_eq!(
            view.globals,
            vec!["leaked".to_owned()],
            "`Cache.__index` sets a field on a local table, not a global"
        );
    }

    #[test]
    fn a_comparison_is_not_an_assignment() {
        let view = parse("local a = 1\nif a == 2 then end\nif a ~= 3 then end\n");

        assert_eq!(view.locals, vec!["a".to_owned()]);
        assert!(view.globals.is_empty());
    }

    #[test]
    fn presents_the_global_warning_with_its_reason() {
        let data = serde_json::to_value(parse(MODULE)).unwrap();

        let lines = LuaPresentation.present(&data);

        assert_eq!(lines[0], "3 function(s):");
        assert!(lines.iter().any(|line| line.contains("see and overwrite")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/lua/src/csvstats.lua");

        let data = LuaCore.view(&path).unwrap();
        let view: LuaView = serde_json::from_value(data).unwrap();

        assert!(view.functions.len() >= 5);
        assert!(view.functions.iter().any(|f| f.scope == "local"));
        assert!(view.functions.iter().any(|f| f.scope == "method"));
        assert!(view.functions.iter().any(|f| !f.parameters.is_empty()));
        assert!(view.requires.len() >= 2);
        assert!(view.tables.len() >= 2);
        assert!(!view.metatables.is_empty());
        assert!(view.locals.len() >= 3);
    }

    #[test]
    fn the_leaky_script_proves_the_global_list() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/lua/tools/report.lua");

        let data = LuaCore.view(&path).unwrap();
        let view: LuaView = serde_json::from_value(data).unwrap();

        assert!(!view.globals.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::LuaCore),
            plugin_api::PluginPresentation::extensions(&crate::LuaPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
