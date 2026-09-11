//! D file type plugin: core and presentation halves.
//!
//! A D module says what it is called, what it imports, and how much the
//! compiler is allowed to check. This reads the module name, the
//! imports, the functions with their attributes and whether each is a
//! template, the structs and classes, the named templates, the mixins,
//! the unittest blocks - and the functions carrying no safety attribute,
//! which are `@system` by default.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["d", "di"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One function the module declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DFunction {
    /// Its name.
    pub name: String,
    /// The attributes written on it: `@safe`, `pure`, `nothrow` and so on.
    pub attributes: Vec<String>,
    /// Whether it is a template, which is a function with two bracket
    /// lists rather than one.
    pub template: bool,
}

/// View data produced by [`DlangCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DlangView {
    /// The module's own name.
    pub module: Option<String>,
    /// The modules it imports.
    pub imports: Vec<String>,
    /// Every function, with the attributes it carries.
    pub functions: Vec<DFunction>,
    /// The structs declared.
    pub structs: Vec<String>,
    /// The classes and interfaces declared.
    pub classes: Vec<String>,
    /// The named templates declared with the `template` keyword.
    pub templates: Vec<String>,
    /// The mixins used, which put code in that is not written here.
    pub mixins: Vec<String>,
    /// How many `unittest` blocks the module carries.
    pub unittests: usize,
    /// Functions with none of `@safe`, `@trusted` or `@system` on them.
    /// D's default is `@system`, which checks nothing.
    pub unchecked: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The attributes worth recording when they appear on a function.
const ATTRIBUTES: &[&str] = &[
    "@safe",
    "@trusted",
    "@system",
    "@nogc",
    "@property",
    "@pure",
    "pure",
    "nothrow",
    "const",
    "immutable",
    "shared",
    "static",
    "override",
    "final",
    "abstract",
    "deprecated",
];

/// The three that say how much the compiler checks.
const SAFETY: &[&str] = &["@safe", "@trusted", "@system"];

/// The keywords that open something other than a function, so a line
/// beginning with one is never read as a declaration.
const NOT_A_FUNCTION: &[&str] = &[
    "if",
    "else",
    "for",
    "foreach",
    "foreach_reverse",
    "while",
    "switch",
    "case",
    "return",
    "with",
    "do",
    "try",
    "catch",
    "finally",
    "scope",
    "assert",
    "version",
    "debug",
    "synchronized",
    "in",
    "out",
    "body",
];

/// `text` with its comments removed, nesting ones included.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some(at) = rest.find("/*").into_iter().chain(rest.find("//")).min() else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..at]);
        if rest[at..].starts_with("//") {
            match rest[at..].find('\n') {
                Some(end) => {
                    out.push('\n');
                    rest = &rest[at + end + 1..];
                }
                None => return out,
            }
        } else {
            match rest[at + 2..].find("*/") {
                Some(end) => rest = &rest[at + 2 + end + 2..],
                None => return out,
            }
        }
    }
}

/// The declaration on `line`, stripped of the attributes it opens with.
///
/// Returns the attributes found and the rest of the line.
fn attributes_of(line: &str) -> (Vec<String>, String) {
    let mut found = Vec::new();
    let mut rest = line.trim();
    while let Some(word) = rest
        .split_whitespace()
        .next()
        .filter(|word| ATTRIBUTES.contains(word) || *word == "public" || *word == "private")
    {
        if ATTRIBUTES.contains(&word) {
            found.push(word.to_owned());
        }
        rest = rest[word.len()..].trim_start();
    }
    // D also allows the attributes after the bracket list.
    for word in rest.split_whitespace() {
        let word = word.trim_end_matches(['{', ';']);
        if ATTRIBUTES.contains(&word) && !found.contains(&word.to_owned()) {
            found.push(word.to_owned());
        }
    }
    (found, rest.to_owned())
}

/// The function `rest` declares, if it declares one.
fn function_of(rest: &str, attributes: Vec<String>) -> Option<DFunction> {
    let open = rest.find('(')?;
    let head = rest[..open].trim();
    // `ReturnType name` - two words at least, and the last is the name.
    let mut words = head.split_whitespace();
    let first = words.next()?;
    if NOT_A_FUNCTION.contains(&first) {
        return None;
    }
    let name = head.split_whitespace().next_back()?;
    if name == first && first != "this" && first != "~this" {
        // One word before the bracket is a call - except `this`, which is
        // how D spells a constructor, and `~this` its destructor.
        return None;
    }
    if !name
        .chars()
        .all(|letter| letter.is_alphanumeric() || letter == '_' || letter == '~')
    {
        return None;
    }
    // A template has a second bracket list: `T largest(T)(T[] values)`.
    let after = &rest[open..];
    let template = after
        .find(')')
        .is_some_and(|close| after[close + 1..].trim_start().starts_with('('));
    Some(DFunction {
        name: name.to_owned(),
        attributes,
        template,
    })
}

/// Everything [`DlangView`] holds, read from `text`.
fn parse(text: &str) -> DlangView {
    let mut view = DlangView {
        module: None,
        imports: Vec::new(),
        functions: Vec::new(),
        structs: Vec::new(),
        classes: Vec::new(),
        templates: Vec::new(),
        mixins: Vec::new(),
        unittests: 0,
        unchecked: Vec::new(),
        truncated: false,
    };
    for raw in without_comments(text).lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("module ") {
            view.module = Some(rest.trim_end_matches(';').trim().to_owned());
            continue;
        }
        if let Some(rest) = line.strip_prefix("import ") {
            view.imports.extend(
                rest.trim_end_matches(';')
                    .split(',')
                    .map(|name| name.split(':').next().unwrap_or(name).trim().to_owned())
                    .filter(|name| !name.is_empty()),
            );
            continue;
        }
        if line.starts_with("unittest") {
            view.unittests += 1;
            continue;
        }
        if let Some(rest) = line.strip_prefix("mixin ") {
            view.mixins
                .push(rest.trim_end_matches(';').trim().to_owned());
            continue;
        }

        let (attributes, rest) = attributes_of(line);
        // A class and an interface go on the same list, so this is a
        // match rather than an array of borrows into the same field.
        for keyword in ["struct ", "class ", "interface ", "template "] {
            let Some(tail) = rest.strip_prefix(keyword) else {
                continue;
            };
            let name = tail
                .split(['(', '{', ':', ' '])
                .next()
                .unwrap_or(tail)
                .trim();
            if name.is_empty() {
                continue;
            }
            match keyword {
                "struct " => view.structs.push(name.to_owned()),
                "template " => view.templates.push(name.to_owned()),
                _ => view.classes.push(name.to_owned()),
            }
        }
        if rest.starts_with("struct ")
            || rest.starts_with("class ")
            || rest.starts_with("interface ")
            || rest.starts_with("template ")
        {
            continue;
        }
        // A statement ends at a semicolon; a declaration does not, and it
        // has a bracket list. D puts attributes *after* that list as
        // readily as before it - `T largest(T)(T[] v) @safe nothrow` - so
        // the line cannot be required to end at the bracket or the brace.
        if rest.ends_with(';') || !rest.contains('(') {
            continue;
        }
        if let Some(function) = function_of(&rest, attributes) {
            view.functions.push(function);
        }
    }

    view.unchecked = view
        .functions
        .iter()
        .filter(|function| {
            !function
                .attributes
                .iter()
                .any(|attribute| SAFETY.contains(&attribute.as_str()))
        })
        .map(|function| function.name.clone())
        .collect();
    view
}

/// Whether `text` is D.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    // Braces and semicolons are most of C. D announces itself: a `module`
    // line, an `import std.` , or a `unittest` block, which no other
    // language in this registry writes that way.
    let d_shaped = view.module.is_some()
        || view.unittests > 0
        || view.imports.iter().any(|name| name.starts_with("std."))
        || text.contains("@safe")
        || text.contains("nothrow");
    d_shaped && (!view.functions.is_empty() || !view.structs.is_empty() || view.unittests > 0)
}

/// The D plugin's core half.
#[derive(Debug, Default)]
pub struct DlangCore;

impl PluginCore for DlangCore {
    fn name(&self) -> &'static str {
        "dlang"
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

/// The D plugin's presentation half.
#[derive(Debug, Default)]
pub struct DlangPresentation;

impl PluginPresentation for DlangPresentation {
    fn name(&self) -> &'static str {
        "dlang"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "D",
            tint: 0x00b0_3931,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: DlangView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "module {}",
            view.module.as_deref().unwrap_or("(unnamed)")
        ));
        if !view.imports.is_empty() {
            lines.push(format!("Imports: {}", view.imports.join(", ")));
        }
        lines.push(format!("{} function(s):", view.functions.len()));
        for function in &view.functions {
            let attributes = if function.attributes.is_empty() {
                String::new()
            } else {
                format!(" [{}]", function.attributes.join(" "))
            };
            let template = if function.template { " (template)" } else { "" };
            lines.push(format!("  {}{template}{attributes}", function.name));
        }
        for group in [
            ("Structs", &view.structs),
            ("Classes", &view.classes),
            ("Templates", &view.templates),
            ("Mixins", &view.mixins),
        ] {
            if !group.1.is_empty() {
                lines.push(format!("{}: {}", group.0, group.1.join(", ")));
            }
        }
        if view.unittests > 0 {
            lines.push(format!("{} unittest block(s)", view.unittests));
        }
        if !view.unchecked.is_empty() {
            lines.push("Carry none of @safe, @trusted or @system, so they are".to_owned());
            lines.push("@system by default and the compiler checks nothing:".to_owned());
            for name in &view.unchecked {
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
    use super::{DlangCore, DlangPresentation, DlangView, parse, without_comments};
    use plugin_api::{PluginCore, PluginPresentation};

    const SOURCE: &str = concat!(
        "module csvstats.column;\n",
        "\n",
        "import std.algorithm : sum;\n",
        "import std.math, std.array;\n",
        "\n",
        "/*\n",
        "double ghost(double[] values) @safe\n",
        "{\n",
        "    return 0;\n",
        "}\n",
        "*/\n",
        "\n",
        "struct Column\n",
        "{\n",
        "    string name;\n",
        "    double[] values;\n",
        "\n",
        "    double mean() @safe pure const\n",
        "    {\n",
        "        return values.sum / values.length;\n",
        "    }\n",
        "}\n",
        "\n",
        "class Reader\n",
        "{\n",
        "    Column[] read(string path)\n",
        "    {\n",
        "        return [];\n",
        "    }\n",
        "}\n",
        "\n",
        "T largest(T)(T[] values) @safe nothrow\n",
        "{\n",
        "    return values[0];\n",
        "}\n",
        "\n",
        "mixin template Named()\n",
        "{\n",
        "    string name;\n",
        "}\n",
        "\n",
        "unittest\n",
        "{\n",
        "    assert(largest([1, 9, 4]) == 9);\n",
        "}\n",
    );

    #[test]
    fn sniffs_a_module() {
        assert!(DlangCore.sniff(SOURCE.as_bytes()));
    }

    #[test]
    fn does_not_claim_c_which_also_has_braces_and_semicolons() {
        assert!(!DlangCore.sniff(b"#include <stdio.h>\nint main(void) { return 0; }\n"));
        assert!(!DlangCore.sniff(b""));
    }

    #[test]
    fn a_block_comment_is_not_code() {
        assert!(!without_comments(SOURCE).contains("ghost"));
        assert!(
            !parse(SOURCE)
                .functions
                .iter()
                .any(|function| function.name == "ghost")
        );
    }

    #[test]
    fn reads_the_module_and_its_imports() {
        let view = parse(SOURCE);

        assert_eq!(view.module.as_deref(), Some("csvstats.column"));
        assert_eq!(
            view.imports,
            vec![
                "std.algorithm".to_owned(),
                "std.math".to_owned(),
                "std.array".to_owned()
            ],
            "a selective import names the module, not the symbol"
        );
    }

    #[test]
    fn a_second_bracket_list_marks_a_template() {
        let view = parse(SOURCE);

        let largest = view.functions.iter().find(|f| f.name == "largest").unwrap();
        assert!(largest.template);
        let mean = view.functions.iter().find(|f| f.name == "mean").unwrap();
        assert!(!mean.template);
        assert!(mean.attributes.contains(&"@safe".to_owned()));
        assert!(mean.attributes.contains(&"pure".to_owned()));
    }

    #[test]
    fn a_constructor_is_a_function() {
        let view = parse(concat!(
            "module m;
",
            "class Reader
",
            "{
",
            "    this(string path) @safe
",
            "    {
",
            "        writeln(path);
",
            "    }
",
            "}
",
        ));

        assert!(
            view.functions.iter().any(|f| f.name == "this"),
            "`this` is how D spells a constructor; `writeln(path);` is a call"
        );
        assert_eq!(view.functions.len(), 1);
    }

    #[test]
    fn a_struct_is_not_read_as_a_function() {
        let view = parse(SOURCE);

        assert_eq!(view.structs, vec!["Column".to_owned()]);
        assert_eq!(view.classes, vec!["Reader".to_owned()]);
        assert_eq!(view.unittests, 1);
        assert!(!view.mixins.is_empty());
    }

    #[test]
    fn names_the_functions_the_compiler_will_not_check() {
        let view = parse(SOURCE);

        assert_eq!(
            view.unchecked,
            vec!["read".to_owned()],
            "`mean` and `largest` are @safe; `read` says nothing, so it is @system"
        );
    }

    #[test]
    fn presents_the_unchecked_functions_with_the_reason() {
        let data = serde_json::to_value(parse(SOURCE)).unwrap();

        let lines = DlangPresentation.present(&data);

        assert_eq!(lines[0], "module csvstats.column");
        assert!(lines.iter().any(|line| line.contains("checks nothing")));
        assert!(lines.iter().any(|line| line.contains("(template)")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/dlang/source/csvstats.d");

        let data = DlangCore.view(&path).unwrap();
        let view: DlangView = serde_json::from_value(data).unwrap();

        assert!(view.module.is_some());
        assert!(view.imports.len() >= 3);
        assert!(view.functions.len() >= 5);
        assert!(view.functions.iter().any(|f| f.template));
        assert!(view.functions.iter().any(|f| !f.attributes.is_empty()));
        assert!(!view.structs.is_empty());
        assert!(!view.classes.is_empty());
        assert!(!view.templates.is_empty());
        assert!(!view.mixins.is_empty());
        assert!(view.unittests >= 2);
        assert!(!view.unchecked.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::DlangCore),
            plugin_api::PluginPresentation::extensions(&crate::DlangPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
