//! Zig file type plugin: core and presentation halves.
//!
//! A Zig file declares its imports at the top and everything else as a
//! binding. This reads the imports, the functions with their signatures
//! and whether each is public, the structs, enumerations and error sets,
//! the tests, the comptime blocks - and the functions that take memory
//! from an allocator with nothing anywhere in them to give it back.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["zig"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One function the file declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZigFunction {
    /// Its name.
    pub name: String,
    /// Its parameters, in order, as written.
    pub parameters: Vec<String>,
    /// What it returns. A leading `!` is an error union.
    pub returns: String,
    /// Whether it is visible outside this file.
    pub public: bool,
}

/// View data produced by [`ZigCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZigView {
    /// Each `@import`, as the name it is bound to and what it imports.
    pub imports: Vec<String>,
    /// Every function, public and private.
    pub functions: Vec<ZigFunction>,
    /// The names bound to a struct.
    pub structs: Vec<String>,
    /// The names bound to an enumeration.
    pub enums: Vec<String>,
    /// The names bound to an error set.
    pub error_sets: Vec<String>,
    /// The `test` blocks, by the name each is given.
    pub tests: Vec<String>,
    /// How many bare `comptime` blocks the file has.
    pub comptime_blocks: usize,
    /// Functions that take memory from an allocator without a `defer` or
    /// `errdefer` anywhere in them to give it back.
    pub allocates_without_releasing: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// `line` with its comment stripped. Zig has only the line comment.
fn cleaned(line: &str) -> &str {
    match line.find("//") {
        Some(at) => line[..at].trim_end(),
        None => line,
    }
    .trim()
}

/// The name bound by a `const NAME = ...` or `var NAME = ...` line.
fn bound_name(line: &str) -> Option<&str> {
    let rest = line
        .strip_prefix("pub const ")
        .or_else(|| line.strip_prefix("const "))
        .or_else(|| line.strip_prefix("pub var "))
        .or_else(|| line.strip_prefix("var "))?;
    let name = rest.split(['=', ':']).next()?.trim();
    (!name.is_empty()).then_some(name)
}

/// What `@import("x")` on `line` imports, if it imports anything.
fn imported(line: &str) -> Option<&str> {
    let at = line.find("@import(")?;
    let after = &line[at + "@import(".len()..];
    let quoted = after.strip_prefix('"')?;
    let end = quoted.find('"')?;
    Some(&quoted[..end])
}

/// The parameters between the outermost brackets of a function line.
fn parameters_of(line: &str) -> Vec<String> {
    let Some(open) = line.find('(') else {
        return Vec::new();
    };
    // Brackets nest: `fn f(comptime T: type, x: fn (u8) void) void`.
    let mut depth = 0usize;
    let mut close = None;
    for (at, letter) in line[open..].char_indices() {
        match letter {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + at);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(close) = close else {
        return Vec::new();
    };
    split_top_level(&line[open + 1..close])
}

/// Splits on commas that are not inside brackets.
fn split_top_level(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for letter in text.chars() {
        match letter {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                if !current.trim().is_empty() {
                    out.push(current.trim().to_owned());
                }
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(letter);
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_owned());
    }
    out
}

/// The function `line` declares, if it declares one.
fn function_of(line: &str) -> Option<ZigFunction> {
    let (rest, public) = match line.strip_prefix("pub fn ") {
        Some(rest) => (rest, true),
        None => (line.strip_prefix("fn ")?, false),
    };
    let name = rest.split('(').next()?.trim().to_owned();
    if name.is_empty() {
        return None;
    }
    let parameters = parameters_of(line);
    // What is between the closing bracket and the opening brace.
    let returns = line
        .rfind(')')
        .map(|at| line[at + 1..].trim_end_matches('{').trim().to_owned())
        .filter(|said| !said.is_empty())
        .unwrap_or_else(|| "unstated".to_owned());
    Some(ZigFunction {
        name,
        parameters,
        returns,
        public,
    })
}

/// Everything [`ZigView`] holds, read from `text`.
fn parse(text: &str) -> ZigView {
    let mut view = ZigView {
        imports: Vec::new(),
        functions: Vec::new(),
        structs: Vec::new(),
        enums: Vec::new(),
        error_sets: Vec::new(),
        tests: Vec::new(),
        comptime_blocks: 0,
        allocates_without_releasing: Vec::new(),
        truncated: false,
    };
    // The function currently being read, its brace depth on entry, and
    // what its body has been seen to do.
    let mut inside: Option<(String, usize, bool, bool)> = None;
    let mut depth = 0usize;

    for raw in text.lines() {
        let line = cleaned(raw);
        if line.is_empty() {
            depth = brace_depth(line, depth);
            continue;
        }
        if let Some(what) = imported(line) {
            let name = bound_name(line).unwrap_or(what);
            view.imports.push(format!("{name} = {what}"));
        }
        if let Some(name) = bound_name(line) {
            let body = line.split_once('=').map_or("", |(_, rest)| rest.trim());
            if body.starts_with("struct") {
                view.structs.push(name.to_owned());
            } else if body.starts_with("enum") {
                view.enums.push(name.to_owned());
            } else if body.starts_with("error") {
                view.error_sets.push(name.to_owned());
            }
        }
        if let Some(rest) = line.strip_prefix("test ") {
            view.tests.push(
                rest.trim()
                    .trim_end_matches('{')
                    .trim()
                    .trim_matches('"')
                    .to_owned(),
            );
        }
        if line == "comptime {" {
            view.comptime_blocks += 1;
        }

        if let Some(function) = function_of(line) {
            inside = Some((function.name.clone(), depth, false, false));
            view.functions.push(function);
        } else if let Some((_, _, allocates, releases)) = inside.as_mut() {
            if line.contains(".alloc(") || line.contains(".create(") || line.contains(".dupe(") {
                *allocates = true;
            }
            if line.contains("defer ") || line.contains("errdefer ") {
                *releases = true;
            }
        }

        let was = depth;
        depth = brace_depth(line, depth);
        if let Some((name, opened_at, allocates, releases)) = inside.clone()
            && was > opened_at
            && depth <= opened_at
        {
            if allocates && !releases {
                view.allocates_without_releasing.push(name);
            }
            inside = None;
        }
    }
    view
}

/// `depth` after the braces on `line`.
fn brace_depth(line: &str, depth: usize) -> usize {
    let opens = line.matches('{').count();
    let closes = line.matches('}').count();
    depth + opens - closes.min(depth + opens)
}

/// Whether `text` is Zig.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    // `fn` and braces are most of C. The give-aways are Zig's builtins and
    // its `const x = @import(...)` binding, which nothing else writes.
    let zig_shaped = text.contains("@import(")
        || text.contains("errdefer")
        || text.contains("comptime ")
        || text.contains("anytype");
    zig_shaped && (!view.functions.is_empty() || !view.tests.is_empty())
}

/// The Zig plugin's core half.
#[derive(Debug, Default)]
pub struct ZigCore;

impl PluginCore for ZigCore {
    fn name(&self) -> &'static str {
        "zig"
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
        // The declarations are what a reader came for; the bodies read
        // better in the file itself.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Zig plugin's presentation half.
#[derive(Debug, Default)]
pub struct ZigPresentation;

impl PluginPresentation for ZigPresentation {
    fn name(&self) -> &'static str {
        "zig"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "ZIG",
            tint: 0x00f7_a41d,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: ZigView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.imports.is_empty() {
            lines.push(format!("Imports: {}", view.imports.join(", ")));
        }
        lines.push(format!("{} function(s):", view.functions.len()));
        for function in &view.functions {
            lines.push(format!(
                "  {}{}({}) {}",
                if function.public { "pub " } else { "" },
                function.name,
                function.parameters.join(", "),
                function.returns
            ));
        }
        for group in [
            ("Structs", &view.structs),
            ("Enumerations", &view.enums),
            ("Error sets", &view.error_sets),
        ] {
            if !group.1.is_empty() {
                lines.push(format!("{}: {}", group.0, group.1.join(", ")));
            }
        }
        if !view.tests.is_empty() {
            lines.push(format!("{} test(s):", view.tests.len()));
            for name in &view.tests {
                lines.push(format!("  {name}"));
            }
        }
        if view.comptime_blocks > 0 {
            lines.push(format!("{} comptime block(s)", view.comptime_blocks));
        }
        if !view.allocates_without_releasing.is_empty() {
            lines.push("Takes memory from an allocator with no defer or errdefer".to_owned());
            lines.push("anywhere in the function to give it back:".to_owned());
            for name in &view.allocates_without_releasing {
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
    use super::{ZigCore, ZigPresentation, ZigView, parameters_of, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const SOURCE: &str = concat!(
        "const std = @import(\"std\");\n",
        "const testing = @import(\"std\").testing;\n",
        "\n",
        "pub const Error = error{ Empty, TooLarge };\n",
        "\n",
        "pub const Colour = enum(u8) { red, green, blue };\n",
        "\n",
        "pub const Column = struct {\n",
        "    name: []const u8,\n",
        "    values: []f64,\n",
        "\n",
        "    pub fn mean(self: Column) !f64 {\n",
        "        if (self.values.len == 0) return Error.Empty;\n",
        "        var total: f64 = 0;\n",
        "        for (self.values) |value| total += value;\n",
        "        return total / @as(f64, @floatFromInt(self.values.len));\n",
        "    }\n",
        "};\n",
        "\n",
        "fn tidy(allocator: std.mem.Allocator, count: usize) ![]u8 {\n",
        "    const buffer = try allocator.alloc(u8, count);\n",
        "    errdefer allocator.free(buffer);\n",
        "    return buffer;\n",
        "}\n",
        "\n",
        "fn leaky(allocator: std.mem.Allocator, count: usize) ![]u8 {\n",
        "    const buffer = try allocator.alloc(u8, count);\n",
        "    return buffer;\n",
        "}\n",
        "\n",
        "pub fn largest(comptime T: type, values: []const T) T {\n",
        "    var best = values[0];\n",
        "    for (values) |value| if (value > best) { best = value; };\n",
        "    return best;\n",
        "}\n",
        "\n",
        "comptime {\n",
        "    _ = Colour;\n",
        "}\n",
        "\n",
        "test \"mean of nothing is an error\" {\n",
        "    const column = Column{ .name = \"n\", .values = &.{} };\n",
        "    try testing.expectError(Error.Empty, column.mean());\n",
        "}\n",
    );

    #[test]
    fn sniffs_a_source_file() {
        assert!(ZigCore.sniff(SOURCE.as_bytes()));
    }

    #[test]
    fn does_not_claim_c_which_also_has_braces_and_functions() {
        assert!(!ZigCore.sniff(b"int main(void) {\n    return 0;\n}\n"));
        assert!(!ZigCore.sniff(b""));
    }

    #[test]
    fn a_nested_bracket_does_not_end_the_parameter_list() {
        assert_eq!(
            parameters_of("fn apply(comptime T: type, f: fn (T) T, x: T) T {"),
            vec![
                "comptime T: type".to_owned(),
                "f: fn (T) T".to_owned(),
                "x: T".to_owned()
            ],
            "the bracket in `fn (T) T` is not the end of the list"
        );
    }

    #[test]
    fn separates_the_public_functions_from_the_private_ones() {
        let view = parse(SOURCE);

        assert_eq!(view.functions.len(), 4);
        let mean = view.functions.iter().find(|f| f.name == "mean").unwrap();
        assert!(mean.public);
        assert_eq!(
            mean.returns, "!f64",
            "an error union is part of the signature"
        );
        assert!(view.functions.iter().any(|f| f.name == "tidy" && !f.public));
    }

    #[test]
    fn reads_the_types_apart_from_one_another() {
        let view = parse(SOURCE);

        assert_eq!(view.structs, vec!["Column".to_owned()]);
        assert_eq!(view.enums, vec!["Colour".to_owned()]);
        assert_eq!(view.error_sets, vec!["Error".to_owned()]);
        assert_eq!(view.imports.len(), 2);
        assert_eq!(view.comptime_blocks, 1);
        assert_eq!(view.tests, vec!["mean of nothing is an error".to_owned()]);
    }

    #[test]
    fn an_errdefer_counts_as_giving_the_memory_back() {
        let view = parse(SOURCE);

        assert_eq!(
            view.allocates_without_releasing,
            vec!["leaky".to_owned()],
            "`tidy` allocates too, and has an errdefer for it"
        );
    }

    #[test]
    fn presents_the_leak_with_its_reason() {
        let data = serde_json::to_value(parse(SOURCE)).unwrap();

        let lines = ZigPresentation.present(&data);

        assert!(lines.iter().any(|line| line.contains("give it back")));
        assert!(lines.iter().any(|line| line.contains("pub largest")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/zig/src/root.zig");

        let data = ZigCore.view(&path).unwrap();
        let view: ZigView = serde_json::from_value(data).unwrap();

        assert!(!view.imports.is_empty());
        assert!(view.functions.len() >= 4);
        assert!(view.functions.iter().any(|f| f.public));
        assert!(view.functions.iter().any(|f| !f.public));
        assert!(view.functions.iter().any(|f| f.returns.starts_with('!')));
        assert!(!view.structs.is_empty());
        assert!(!view.enums.is_empty());
        assert!(!view.error_sets.is_empty());
        assert!(view.tests.len() >= 2);
        assert!(view.comptime_blocks >= 1);
        assert!(!view.allocates_without_releasing.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::ZigCore),
            plugin_api::PluginPresentation::extensions(&crate::ZigPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
