//! Gleam file type plugin: core and presentation halves.
//!
//! A Gleam module is imports, custom types and functions, and `pub fn`
//! is shared with half a dozen languages - so this is settled by the
//! marks only Gleam has. It reads the imports, every function with its
//! signature and visibility, the custom types and their constructors,
//! the aliases and constants, the tests, the functions whose bodies are
//! not Gleam at all, and the public functions that say nothing about
//! what they return.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["gleam"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One function the module declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    /// Its name.
    pub name: String,
    /// Its parameters, as written.
    pub parameters: Vec<String>,
    /// What it returns, when the signature says.
    pub returns: Option<String>,
    /// Whether it is visible outside this module.
    pub public: bool,
    /// Whether its body is written in Erlang or JavaScript rather than
    /// in Gleam.
    pub external: bool,
}

/// One custom type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomType {
    /// Its name.
    pub name: String,
    /// Its constructors, in the order written.
    pub constructors: Vec<String>,
    /// Whether it is visible outside this module.
    pub public: bool,
    /// Whether its constructors are, which is a separate question - a
    /// public type with private constructors can only be made here.
    pub opaque: bool,
}

/// View data produced by [`GleamCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GleamView {
    /// The modules it imports.
    pub imports: Vec<String>,
    /// Every function.
    pub functions: Vec<Function>,
    /// Every custom type.
    pub types: Vec<CustomType>,
    /// The type aliases declared.
    pub aliases: Vec<String>,
    /// The constants declared.
    pub constants: Vec<String>,
    /// The test functions, which Gleam finds by their name.
    pub tests: Vec<String>,
    /// Public functions whose signature says nothing about what they
    /// return, so a caller has to read the body to find out.
    pub without_return_types: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// `line` with its comment stripped. Gleam has `//`, `///` and `////`.
fn cleaned(line: &str) -> &str {
    match line.find("//") {
        Some(at) => line[..at].trim_end(),
        None => line,
    }
    .trim()
}

/// Splits `text` on commas that are not inside brackets.
fn split_top_level(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for letter in text.chars() {
        match letter {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth = depth.saturating_sub(1),
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

/// Where the outermost bracket pair of `line` opens and closes.
fn bracket_span(line: &str) -> Option<(usize, usize)> {
    let open = line.find('(')?;
    let mut depth = 0usize;
    for (offset, letter) in line[open..].char_indices() {
        match letter {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open, open + offset));
                }
            }
            _ => {}
        }
    }
    None
}

/// The function `line` declares, if it declares one.
fn function_of(line: &str, external: bool) -> Option<Function> {
    let (rest, public) = match line.strip_prefix("pub fn ") {
        Some(rest) => (rest, true),
        None => (line.strip_prefix("fn ")?, false),
    };
    let name = rest.split('(').next()?.trim().to_owned();
    if name.is_empty() {
        return None;
    }
    let span = bracket_span(line);
    let parameters = span
        .map(|(open, close)| split_top_level(&line[open + 1..close]))
        .unwrap_or_default();
    // The return type is whatever follows the parameter list's own
    // closing bracket. It routinely has brackets of its own -
    // `Result(Float, Error)` - so it cannot be found by looking for the
    // last arrow and rejecting anything bracketed.
    let returns = span
        .map(|(_, close)| line[close + 1..].trim())
        .and_then(|tail| tail.strip_prefix("->"))
        .map(|said| said.trim().trim_end_matches('{').trim().to_owned())
        .filter(|said| !said.is_empty());
    Some(Function {
        name,
        parameters,
        returns,
        public,
        external,
    })
}

/// The name a `type`, `const` or alias line declares.
fn declared(line: &str, keyword: &str) -> Option<(String, bool)> {
    // `pub`, then optionally `opaque`, then the keyword. Both modifiers
    // are stripped here so each spelling reaches the same place.
    let (rest, public) = match line.strip_prefix("pub ") {
        Some(rest) => (rest, true),
        None => (line, false),
    };
    let rest = rest.strip_prefix("opaque ").unwrap_or(rest);
    let rest = rest.strip_prefix(&format!("{keyword} "))?;
    let name = rest.split(['(', '{', ' ', '=']).next()?.trim();
    (!name.is_empty()).then(|| (name.to_owned(), public))
}

/// Everything [`GleamView`] holds, read from `text`.
fn parse(text: &str) -> GleamView {
    let mut view = GleamView {
        imports: Vec::new(),
        functions: Vec::new(),
        types: Vec::new(),
        aliases: Vec::new(),
        constants: Vec::new(),
        tests: Vec::new(),
        without_return_types: Vec::new(),
        truncated: false,
    };
    // Whether the previous line was an `@external` attribute, and how
    // deep in braces the walk is, so a constructor can be told from a
    // statement in a function body.
    let mut external = false;
    let mut in_type: Option<usize> = None;
    let mut depth = 0usize;

    for raw in text.lines() {
        let line = cleaned(raw);
        if line.is_empty() {
            continue;
        }
        if line.starts_with("@external") {
            external = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("import ") {
            view.imports.push(
                rest.split([' ', '.'])
                    .next()
                    .unwrap_or(rest)
                    .trim()
                    .to_owned(),
            );
            continue;
        }
        if let Some(function) = function_of(line, external) {
            if function.name.starts_with("main") || function.name.ends_with("_test") {
                view.tests.push(function.name.clone());
            }
            view.functions.push(function);
            external = false;
            in_type = None;
        } else if let Some((name, public)) = declared(line, "type") {
            // `pub type Name =` is an alias; `pub type Name {` opens a
            // custom type with constructors below it.
            if line.contains('=') {
                view.aliases.push(name);
                in_type = None;
            } else {
                view.types.push(CustomType {
                    name,
                    constructors: Vec::new(),
                    public,
                    opaque: line.contains("opaque type "),
                });
                in_type = Some(view.types.len() - 1);
            }
        } else if let Some((name, _)) = declared(line, "const") {
            view.constants.push(name);
        } else if let Some(at) = in_type
            && depth == 1
            && line.chars().next().is_some_and(char::is_uppercase)
        {
            let constructor = line.split('(').next().unwrap_or(line).trim().to_owned();
            view.types[at].constructors.push(constructor);
        }

        depth = depth + line.matches('{').count()
            - line
                .matches('}')
                .count()
                .min(depth + line.matches('{').count());
        if depth == 0 {
            in_type = None;
        }
    }

    view.without_return_types = view
        .functions
        .iter()
        .filter(|function| function.public && function.returns.is_none())
        .map(|function| function.name.clone())
        .collect();
    view
}

/// Whether `text` is Gleam.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    // `pub fn` and `import` are shared with half a dozen languages. The
    // give-aways are Gleam's own module paths and its `@external` form.
    let gleam_shaped =
        text.contains("import gleam/") || text.contains("@external(") || text.contains("gleeunit");
    gleam_shaped && (!view.functions.is_empty() || !view.types.is_empty())
}

/// The Gleam plugin's core half.
#[derive(Debug, Default)]
pub struct GleamCore;

impl PluginCore for GleamCore {
    fn name(&self) -> &'static str {
        "gleam"
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

/// The Gleam plugin's presentation half.
#[derive(Debug, Default)]
pub struct GleamPresentation;

impl PluginPresentation for GleamPresentation {
    fn name(&self) -> &'static str {
        "gleam"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "GLM",
            tint: 0x00ff_af2f,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: GleamView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.imports.is_empty() {
            lines.push(format!("Imports: {}", view.imports.join(", ")));
        }
        if !view.types.is_empty() {
            lines.push(format!("{} type(s):", view.types.len()));
            for kind in &view.types {
                let visibility = if kind.opaque {
                    " (opaque: only this module can make one)"
                } else if kind.public {
                    ""
                } else {
                    " (private)"
                };
                lines.push(format!(
                    "  {}{visibility}: {}",
                    kind.name,
                    if kind.constructors.is_empty() {
                        "no constructors".to_owned()
                    } else {
                        kind.constructors.join(", ")
                    }
                ));
            }
        }
        if !view.aliases.is_empty() {
            lines.push(format!("Aliases: {}", view.aliases.join(", ")));
        }
        if !view.constants.is_empty() {
            lines.push(format!("Constants: {}", view.constants.join(", ")));
        }
        lines.push(format!("{} function(s):", view.functions.len()));
        for function in &view.functions {
            let returns = function
                .returns
                .as_ref()
                .map_or_else(String::new, |said| format!(" -> {said}"));
            let external = if function.external {
                "  [external]"
            } else {
                ""
            };
            lines.push(format!(
                "  {}{}({}){returns}{external}",
                if function.public { "pub " } else { "" },
                function.name,
                function.parameters.join(", ")
            ));
        }
        if !view.tests.is_empty() {
            lines.push(format!("Tests: {}", view.tests.join(", ")));
        }
        if !view.without_return_types.is_empty() {
            lines.push("Public, and the signature says nothing about what comes".to_owned());
            lines.push("back, so a caller has to read the body to find out:".to_owned());
            for name in &view.without_return_types {
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
    use super::{GleamCore, GleamPresentation, GleamView, bracket_span, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const MODULE: &str = concat!(
        "//// Summary statistics for a column.\n",
        "\n",
        "import gleam/float\n",
        "import gleam/list\n",
        "import gleam/result\n",
        "\n",
        "pub type Column {\n",
        "  Column(name: String, values: List(Float))\n",
        "}\n",
        "\n",
        "pub opaque type Handle {\n",
        "  Handle(id: Int)\n",
        "}\n",
        "\n",
        "pub type Error {\n",
        "  Empty\n",
        "  NotANumber(String)\n",
        "}\n",
        "\n",
        "pub type Summary = #(Float, Float)\n",
        "\n",
        "const default_places = 3\n",
        "\n",
        "pub fn mean(column: Column) -> Result(Float, Error) {\n",
        "  case column.values {\n",
        "    [] -> Error(Empty)\n",
        "    values -> Ok(sum(values) /. length_of(values))\n",
        "  }\n",
        "}\n",
        "\n",
        "pub fn describe(column: Column) {\n",
        "  column.name\n",
        "}\n",
        "\n",
        "fn sum(values: List(Float)) -> Float {\n",
        "  list.fold(values, 0.0, float.add)\n",
        "}\n",
        "\n",
        "@external(erlang, \"math\", \"sqrt\")\n",
        "fn square_root(value: Float) -> Float\n",
        "\n",
        "pub fn mean_test() {\n",
        "  let column = Column(\"n\", [1.0, 2.0, 3.0])\n",
        "  assert Ok(2.0) = mean(column)\n",
        "}\n",
    );

    #[test]
    fn sniffs_a_module() {
        assert!(GleamCore.sniff(MODULE.as_bytes()));
    }

    #[test]
    fn does_not_claim_rust_which_also_writes_pub_fn() {
        assert!(
            !GleamCore.sniff(b"use std::fmt;\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n")
        );
        assert!(!GleamCore.sniff(b""));
    }

    #[test]
    fn a_generic_return_type_keeps_its_brackets() {
        let declared = super::function_of(
            "pub fn mean(column: Column) -> Result(Float, Error) {",
            false,
        )
        .unwrap();

        assert_eq!(declared.returns.as_deref(), Some("Result(Float, Error)"));
        assert_eq!(declared.parameters, vec!["column: Column".to_owned()]);
    }

    #[test]
    fn a_nested_bracket_does_not_end_the_parameter_list() {
        let line = "fn sum(values: List(Float), places: Int) -> Float {";
        let (open, close) = bracket_span(line).unwrap();

        assert_eq!(
            &line[open + 1..close],
            "values: List(Float), places: Int",
            "the bracket in `List(Float)` is not the end of the list"
        );
    }

    #[test]
    fn reads_the_imports_and_the_alias() {
        let view = parse(MODULE);

        assert_eq!(
            view.imports,
            vec![
                "gleam/float".to_owned(),
                "gleam/list".to_owned(),
                "gleam/result".to_owned()
            ],
            "the path is the module's name; truncating it at the slash loses it"
        );
        assert_eq!(view.aliases, vec!["Summary".to_owned()]);
        assert_eq!(view.constants, vec!["default_places".to_owned()]);
    }

    #[test]
    fn reads_the_constructors_of_each_type() {
        let view = parse(MODULE);

        assert_eq!(view.types.len(), 3);
        let error = view.types.iter().find(|t| t.name == "Error").unwrap();
        assert_eq!(
            error.constructors,
            vec!["Empty".to_owned(), "NotANumber".to_owned()]
        );
    }

    #[test]
    fn an_opaque_type_is_marked_as_one() {
        let view = parse(MODULE);

        let handle = view.types.iter().find(|t| t.name == "Handle").unwrap();
        assert!(handle.opaque);
        let column = view.types.iter().find(|t| t.name == "Column").unwrap();
        assert!(!column.opaque && column.public);
    }

    #[test]
    fn an_external_function_is_marked_as_one() {
        let view = parse(MODULE);

        let root = view
            .functions
            .iter()
            .find(|f| f.name == "square_root")
            .unwrap();
        assert!(root.external, "its body is Erlang, not Gleam");
        let sum = view.functions.iter().find(|f| f.name == "sum").unwrap();
        assert!(
            !sum.external,
            "the attribute above `square_root` is spent on it"
        );
    }

    #[test]
    fn names_the_public_function_with_no_return_type() {
        let view = parse(MODULE);

        assert_eq!(
            view.without_return_types,
            vec!["describe".to_owned(), "mean_test".to_owned()],
            "`mean` says `-> Result(Float, Error)`; these say nothing"
        );
    }

    #[test]
    fn finds_the_test_by_its_name() {
        let view = parse(MODULE);

        assert_eq!(view.tests, vec!["mean_test".to_owned()]);
    }

    #[test]
    fn presents_the_opaque_type_and_the_missing_return() {
        let data = serde_json::to_value(parse(MODULE)).unwrap();

        let lines = GleamPresentation.present(&data);

        assert!(
            lines
                .iter()
                .any(|line| line.contains("only this module can make one"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("read the body to find out"))
        );
        assert!(lines.iter().any(|line| line.contains("[external]")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/gleam/src/csvstats.gleam");

        let data = GleamCore.view(&path).unwrap();
        let view: GleamView = serde_json::from_value(data).unwrap();

        assert!(view.imports.len() >= 3);
        assert!(view.functions.len() >= 5);
        assert!(view.functions.iter().any(|f| f.public));
        assert!(view.functions.iter().any(|f| !f.public));
        assert!(view.functions.iter().any(|f| f.external));
        assert!(view.functions.iter().any(|f| f.returns.is_some()));
        assert!(view.types.len() >= 2);
        assert!(view.types.iter().any(|t| t.opaque));
        assert!(view.types.iter().any(|t| t.constructors.len() >= 2));
        assert!(!view.aliases.is_empty());
        assert!(!view.constants.is_empty());
        assert!(!view.without_return_types.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::GleamCore),
            plugin_api::PluginPresentation::extensions(&crate::GleamPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
