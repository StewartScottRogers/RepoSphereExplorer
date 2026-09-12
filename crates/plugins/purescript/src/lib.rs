//! PureScript file type plugin: core and presentation halves.
//!
//! A PureScript module opens with its name and what it exports, and
//! Haskell opens the same way - so this is settled by the marks only
//! PureScript has. It reads the module and its export list, the imports,
//! every top-level signature, the types, classes and instances, the
//! foreign imports whose JavaScript the compiler never sees, and the
//! names defined with no signature above them.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["purs"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One top-level signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    /// The name it declares.
    pub name: String,
    /// The type, as written, with a multi-line signature joined up.
    pub kind: String,
    /// Whether the module exports it.
    pub exported: bool,
}

/// View data produced by [`PurescriptCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurescriptView {
    /// The module's name.
    pub module: Option<String>,
    /// What the module exports, or empty when it exports everything.
    pub exports: Vec<String>,
    /// The modules it imports.
    pub imports: Vec<String>,
    /// Every top-level type signature.
    pub signatures: Vec<Signature>,
    /// The `data` and `newtype` declarations.
    pub types: Vec<String>,
    /// The type classes declared.
    pub classes: Vec<String>,
    /// The instances declared.
    pub instances: Vec<String>,
    /// The `foreign import` declarations, which are JavaScript this
    /// module trusts without the compiler having seen it.
    pub foreign_imports: Vec<String>,
    /// Names defined at the top level with no type signature above them,
    /// which the compiler infers and nobody wrote down.
    pub without_signatures: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// `text` with its comments removed. PureScript has `--` and `{- -}`.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let opener = ["{-", "--"]
            .iter()
            .filter_map(|mark| rest.find(mark).map(|at| (at, *mark)))
            .min_by_key(|(at, _)| *at);
        let Some((at, mark)) = opener else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..at]);
        let after = &rest[at + mark.len()..];
        let closer = if mark == "{-" { "-}" } else { "\n" };
        match after.find(closer) {
            Some(end) => {
                if closer == "\n" {
                    out.push('\n');
                }
                rest = &after[end + closer.len()..];
            }
            None => return out,
        }
    }
}

/// Joins a declaration that runs onto indented continuation lines.
///
/// A signature is routinely written over three or four lines, and read
/// one at a time the type is whatever fitted on the first.
fn logical_lines(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        let indented = raw.starts_with([' ', '\t']);
        match (indented, out.last_mut()) {
            (true, Some(previous)) => {
                previous.push(' ');
                previous.push_str(raw.trim());
            }
            _ => out.push(raw.trim().to_owned()),
        }
    }
    out
}

/// The name a `data`, `newtype`, `class` or `instance` line declares.
fn declared(line: &str, keyword: &str) -> Option<String> {
    let rest = line.strip_prefix(keyword)?;
    if !rest.starts_with(' ') {
        return None;
    }
    let name = rest.trim().split([' ', '=', '(']).next()?.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// The names in a module's export list, if it has one.
fn exports_of(line: &str) -> Vec<String> {
    let Some(open) = line.find('(') else {
        return Vec::new();
    };
    let Some(close) = line.rfind(')') else {
        return Vec::new();
    };
    if close < open {
        return Vec::new();
    }
    line[open + 1..close]
        .split(',')
        .map(str::trim)
        // `Column(..)` exports a type and its constructors; the name is
        // what matters here.
        .map(|name| name.split('(').next().unwrap_or(name).trim())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Everything [`PurescriptView`] holds, read from `text`.
fn parse(text: &str) -> PurescriptView {
    let mut view = PurescriptView {
        module: None,
        exports: Vec::new(),
        imports: Vec::new(),
        signatures: Vec::new(),
        types: Vec::new(),
        classes: Vec::new(),
        instances: Vec::new(),
        foreign_imports: Vec::new(),
        without_signatures: Vec::new(),
        truncated: false,
    };
    // Names defined at the top level, so a definition can be told from a
    // signature afterwards.
    let mut defined: Vec<String> = Vec::new();

    for line in logical_lines(&without_comments(text)) {
        if let Some(rest) = line.strip_prefix("module ") {
            view.module = rest.split_whitespace().next().map(str::to_owned);
            view.exports = exports_of(&line);
            continue;
        }
        if let Some(rest) = line.strip_prefix("import ") {
            if let Some(name) = rest.split_whitespace().next() {
                view.imports.push(name.to_owned());
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("foreign import ") {
            let name = rest.split([' ', ':']).next().unwrap_or(rest).trim();
            if !name.is_empty() {
                view.foreign_imports.push(name.to_owned());
            }
            continue;
        }
        // All three go on the same list, so this is a loop over keywords
        // rather than an array of borrows into the same field.
        for keyword in ["data", "newtype", "type"] {
            if let Some(name) = declared(&line, keyword) {
                view.types.push(name);
            }
        }
        if let Some(name) = declared(&line, "class") {
            view.classes.push(name);
            continue;
        }
        if line.starts_with("instance ") || line.starts_with("derive instance ") {
            // `instance showColumn :: Show Column where` - the head is
            // what a reader recognises it by.
            let head = line
                .trim_start_matches("derive ")
                .trim_start_matches("instance ")
                .split(" where")
                .next()
                .unwrap_or("")
                .trim();
            if !head.is_empty() {
                view.instances.push(head.to_owned());
            }
            continue;
        }
        if line.starts_with("data ") || line.starts_with("newtype ") || line.starts_with("type ") {
            continue;
        }
        // `name :: Type` is a signature; `name arguments = body` is a
        // definition. The `::` has to come before any `=`.
        if let Some(at) = line.find(" :: ")
            && line[..at].split_whitespace().count() == 1
            && line.find('=').is_none_or(|equals| equals > at)
        {
            let name = line[..at].trim().to_owned();
            view.signatures.push(Signature {
                exported: view.exports.is_empty() || view.exports.contains(&name),
                name,
                kind: line[at + 4..].trim().to_owned(),
            });
            continue;
        }
        if let Some(at) = line.find('=')
            && let Some(name) = line[..at].split_whitespace().next()
            && name
                .chars()
                .next()
                .is_some_and(|first| first.is_lowercase() || first == '_')
        {
            defined.push(name.to_owned());
        }
    }

    view.without_signatures = defined
        .iter()
        .filter(|name| {
            !view
                .signatures
                .iter()
                .any(|signature| &&signature.name == name)
        })
        .cloned()
        .collect();
    view.without_signatures.dedup();
    view
}

/// Whether `text` is PureScript.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    if view.module.is_none() {
        return false;
    }
    // Haskell writes `module ... where` too. PureScript's own marks are
    // `Effect`, `foreign import`, and importing `Prelude` unqualified.
    let purescript_shaped = !view.foreign_imports.is_empty()
        || text.contains("Effect ")
        || text.contains("Effect(")
        || view
            .signatures
            .iter()
            .any(|signature| signature.kind.contains("Effect"));
    purescript_shaped && (!view.signatures.is_empty() || !view.types.is_empty())
}

/// The PureScript plugin's core half.
#[derive(Debug, Default)]
pub struct PurescriptCore;

impl PluginCore for PurescriptCore {
    fn name(&self) -> &'static str {
        "purescript"
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

/// The PureScript plugin's presentation half.
#[derive(Debug, Default)]
pub struct PurescriptPresentation;

impl PluginPresentation for PurescriptPresentation {
    fn name(&self) -> &'static str {
        "purescript"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "PS",
            tint: 0x0014_161b,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: PurescriptView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "module {}",
            view.module.as_deref().unwrap_or("(unnamed)")
        ));
        if view.exports.is_empty() {
            lines.push("Exports everything: no export list.".to_owned());
        } else {
            lines.push(format!("Exports: {}", view.exports.join(", ")));
        }
        if !view.imports.is_empty() {
            lines.push(format!("Imports: {}", view.imports.join(", ")));
        }
        for group in [
            ("Types", &view.types),
            ("Classes", &view.classes),
            ("Instances", &view.instances),
        ] {
            if !group.1.is_empty() {
                lines.push(format!("{}: {}", group.0, group.1.join(", ")));
            }
        }
        lines.push(format!("{} signature(s):", view.signatures.len()));
        for signature in &view.signatures {
            let hidden = if signature.exported {
                ""
            } else {
                "  [not exported]"
            };
            lines.push(format!(
                "  {} :: {}{hidden}",
                signature.name, signature.kind
            ));
        }
        if !view.foreign_imports.is_empty() {
            lines.push("Declared foreign, so the compiler has not seen what these".to_owned());
            lines.push("actually do and takes the type on trust:".to_owned());
            for name in &view.foreign_imports {
                lines.push(format!("  {name}"));
            }
        }
        if !view.without_signatures.is_empty() {
            lines.push("Defined with no signature above them, so their type is".to_owned());
            lines.push("whatever the compiler worked out and nobody wrote down:".to_owned());
            for name in &view.without_signatures {
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
    use super::{
        PurescriptCore, PurescriptPresentation, PurescriptView, exports_of, logical_lines, parse,
    };
    use plugin_api::{PluginCore, PluginPresentation};

    const MODULE: &str = concat!(
        "-- | Summary statistics for a column.\n",
        "module Data.CsvStats\n",
        "  ( Column(..)\n",
        "  , mean\n",
        "  , deviation\n",
        "  ) where\n",
        "\n",
        "import Prelude\n",
        "import Data.Array (length)\n",
        "import Effect (Effect)\n",
        "\n",
        "{- An older definition lived here:\n",
        "ghost :: Int -> Int\n",
        "-}\n",
        "\n",
        "newtype Column = Column { name :: String, values :: Array Number }\n",
        "\n",
        "class Describable a where\n",
        "  describe :: a -> String\n",
        "\n",
        "instance describableColumn :: Describable Column where\n",
        "  describe (Column c) = c.name\n",
        "\n",
        "foreign import parseFloat :: String -> Number\n",
        "\n",
        "mean\n",
        "  :: Column\n",
        "  -> Number\n",
        "mean (Column c) = sum c.values / toNumber (length c.values)\n",
        "\n",
        "deviation :: Column -> Number\n",
        "deviation c = 0.0\n",
        "\n",
        "helper x = x + 1\n",
        "\n",
        "report :: Column -> Effect Unit\n",
        "report c = log (describe c)\n",
    );

    #[test]
    fn sniffs_a_module() {
        assert!(PurescriptCore.sniff(MODULE.as_bytes()));
    }

    #[test]
    fn does_not_claim_haskell_which_opens_the_same_way() {
        assert!(
            !PurescriptCore.sniff(
                concat!(
                    "module Main where\n",
                    "import Data.List (sort)\n",
                    "main :: IO ()\n",
                    "main = print (sort [3, 1, 2])\n",
                )
                .as_bytes()
            )
        );
        assert!(!PurescriptCore.sniff(b""));
    }

    #[test]
    fn a_signature_may_run_over_several_lines() {
        let joined = logical_lines("mean\n  :: Column\n  -> Number\nnext = 1\n");

        assert_eq!(
            joined,
            vec!["mean :: Column -> Number".to_owned(), "next = 1".to_owned()],
            "the indented lines belong to the signature above them"
        );
    }

    #[test]
    fn reads_the_module_and_its_export_list() {
        let view = parse(MODULE);

        assert_eq!(view.module.as_deref(), Some("Data.CsvStats"));
        assert_eq!(
            view.exports,
            vec![
                "Column".to_owned(),
                "mean".to_owned(),
                "deviation".to_owned()
            ],
            "`Column(..)` exports the type; the name is what matters here"
        );
        assert_eq!(view.imports.len(), 3);
    }

    #[test]
    fn an_export_list_is_told_from_an_import_list() {
        assert_eq!(
            exports_of("module M (a, b) where"),
            vec!["a".to_owned(), "b".to_owned()]
        );
        assert!(exports_of("module M where").is_empty());
    }

    #[test]
    fn a_block_comment_is_not_a_signature() {
        let view = parse(MODULE);

        assert!(
            !view
                .signatures
                .iter()
                .any(|signature| signature.name == "ghost")
        );
    }

    #[test]
    fn reads_the_type_the_class_the_instance_and_the_foreign_import() {
        let view = parse(MODULE);

        assert!(view.types.contains(&"Column".to_owned()));
        assert_eq!(view.classes, vec!["Describable".to_owned()]);
        assert_eq!(
            view.instances,
            vec!["describableColumn :: Describable Column".to_owned()]
        );
        assert_eq!(view.foreign_imports, vec!["parseFloat".to_owned()]);
    }

    #[test]
    fn names_what_has_no_signature() {
        let view = parse(MODULE);

        assert_eq!(
            view.without_signatures,
            vec!["helper".to_owned()],
            "`mean`, `deviation` and `report` all have one above them"
        );
    }

    #[test]
    fn marks_what_the_export_list_leaves_out() {
        let view = parse(MODULE);

        let report = view.signatures.iter().find(|s| s.name == "report").unwrap();
        assert!(
            !report.exported,
            "the export list names three things, not four"
        );
        let mean = view.signatures.iter().find(|s| s.name == "mean").unwrap();
        assert!(mean.exported);
    }

    #[test]
    fn presents_both_warnings_with_their_reasons() {
        let data = serde_json::to_value(parse(MODULE)).unwrap();

        let lines = PurescriptPresentation.present(&data);

        assert_eq!(lines[0], "module Data.CsvStats");
        assert!(
            lines
                .iter()
                .any(|line| line.contains("takes the type on trust"))
        );
        assert!(lines.iter().any(|line| line.contains("nobody wrote down")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/purescript/src/Data/CsvStats.purs");

        let data = PurescriptCore.view(&path).unwrap();
        let view: PurescriptView = serde_json::from_value(data).unwrap();

        assert!(view.module.is_some());
        assert!(view.exports.len() >= 3);
        assert!(view.imports.len() >= 4);
        assert!(view.signatures.len() >= 5);
        assert!(view.signatures.iter().any(|s| !s.exported));
        assert!(view.types.len() >= 2);
        assert!(!view.classes.is_empty());
        assert!(!view.instances.is_empty());
        assert!(!view.foreign_imports.is_empty());
        assert!(!view.without_signatures.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::PurescriptCore),
            plugin_api::PluginPresentation::extensions(&crate::PurescriptPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
