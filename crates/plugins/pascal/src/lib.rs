//! Pascal file type plugin: core and presentation halves.
//!
//! A Pascal unit is in two halves: what other units may see, and how it
//! is done. This reads the header, the units used from each half, the
//! types, the classes and their properties, every routine with its
//! parameters and return type - and the routines promised in the
//! interface that were never written, which will not link.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["pas", "pp", "dpr", "lpr", "inc"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One routine the unit declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routine {
    /// Its name, with the class it belongs to when it has one.
    pub name: String,
    /// `procedure` or `function`.
    pub kind: String,
    /// Its parameters, as written.
    pub parameters: Vec<String>,
    /// What a function returns.
    pub returns: Option<String>,
    /// Whether it was declared in the `interface` section, and so is
    /// visible to every other unit that uses this one.
    pub exported: bool,
}

/// View data produced by [`PascalCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PascalView {
    /// What the file calls itself.
    pub name: Option<String>,
    /// Whether it is a `program`, a `unit` or a `library`.
    pub kind: String,
    /// The units named in `uses`, with the section each was used from.
    pub uses: Vec<String>,
    /// The types declared, records and classes included.
    pub types: Vec<String>,
    /// The classes declared, which are types with methods.
    pub classes: Vec<String>,
    /// The properties declared on those classes.
    pub properties: Vec<String>,
    /// Every routine.
    pub routines: Vec<Routine>,
    /// Routines declared in the interface with no body in the
    /// implementation, which the compiler refuses to link.
    pub declared_but_not_written: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// `text` with its comments removed. Pascal has three kinds.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let opener = ["{", "(*", "//"]
            .iter()
            .filter_map(|mark| rest.find(mark).map(|at| (at, *mark)))
            .min_by_key(|(at, _)| *at);
        let Some((at, mark)) = opener else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..at]);
        let after = &rest[at + mark.len()..];
        let closer = match mark {
            "{" => "}",
            "(*" => "*)",
            _ => "\n",
        };
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

/// The parameters between the brackets of `line`.
fn parameters_of(line: &str) -> Vec<String> {
    let Some(open) = line.find('(') else {
        return Vec::new();
    };
    let Some(close) = line[open..].rfind(')').map(|at| at + open) else {
        return Vec::new();
    };
    line[open + 1..close]
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The words a routine declaration may open with, each with the name to
/// record it under.
const OPENERS: &[(&str, &str)] = &[
    ("procedure ", "procedure"),
    ("function ", "function"),
    ("constructor ", "constructor"),
    ("destructor ", "destructor"),
];

/// The routine `line` declares, if it declares one.
fn routine_of(line: &str, exported: bool) -> Option<Routine> {
    let lower = line.to_ascii_lowercase();
    let (rest, kind) = OPENERS
        .iter()
        .find_map(|(prefix, kind)| lower.strip_prefix(prefix).map(|rest| (rest, *kind)))?;
    // Take the name from the original line, which has its capitals.
    let at = line.len() - rest.len();
    let name = line[at..].split(['(', ':', ';']).next()?.trim().to_owned();
    let name = (!name.is_empty()).then_some(name)?;
    let returns = (kind == "function")
        .then(|| {
            let tail = line.rsplit_once(')').map_or(line, |(_, tail)| tail);
            tail.split(':')
                .nth(1)
                .map(|said| said.trim().trim_end_matches(';').trim().to_owned())
                .filter(|said| !said.is_empty())
        })
        .flatten();
    Some(Routine {
        name,
        kind: kind.to_owned(),
        parameters: parameters_of(line),
        returns,
        exported,
    })
}

/// Splits a finished `uses` clause onto `into`, tagged with its half.
fn finish_uses(collecting: &mut Option<String>, exported: bool, into: &mut Vec<String>) {
    let Some(clause) = collecting.take() else {
        return;
    };
    let section = if exported {
        "interface"
    } else {
        "implementation"
    };
    for used in clause.trim_end_matches(';').split(',') {
        let used = used.trim().trim_end_matches(';').trim();
        if !used.is_empty() {
            into.push(format!("{used} ({section})"));
        }
    }
}

/// Everything [`PascalView`] holds, read from `text`.
fn parse(text: &str) -> PascalView {
    let mut view = PascalView {
        name: None,
        kind: "unstated".to_owned(),
        uses: Vec::new(),
        types: Vec::new(),
        classes: Vec::new(),
        properties: Vec::new(),
        routines: Vec::new(),
        declared_but_not_written: Vec::new(),
        truncated: false,
    };
    // Pascal splits a unit in two: what other units may see, and how it
    // is done. Which half a declaration is in is the whole of its
    // visibility, so the walk has to keep track.
    let mut exported = false;
    let mut in_type_block = false;
    // The body of a `uses` clause, while one is still being read.
    let mut collecting: Option<String> = None;

    for raw in without_comments(text).lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(pending) = collecting.as_mut() {
            pending.push(' ');
            pending.push_str(line);
            if line.trim_end().ends_with(';') {
                finish_uses(&mut collecting, exported, &mut view.uses);
            }
            continue;
        }
        let lower = line.to_ascii_lowercase();
        let word = lower.split([' ', ';']).next().unwrap_or("");

        match word {
            "program" | "unit" | "library" => {
                word.clone_into(&mut view.kind);
                view.name = line
                    .split_whitespace()
                    .nth(1)
                    .map(|name| name.trim_end_matches(';').to_owned());
                continue;
            }
            "interface" => {
                exported = true;
                in_type_block = false;
                continue;
            }
            "implementation" => {
                exported = false;
                in_type_block = false;
                continue;
            }
            "uses" => {
                // The keyword routinely sits on a line of its own with the
                // unit names below it: the clause ends at the semicolon,
                // not at the end of the line.
                collecting = Some(line[4..].to_owned());
                if line.trim_end().ends_with(';') {
                    finish_uses(&mut collecting, exported, &mut view.uses);
                }
                continue;
            }
            "type" => {
                in_type_block = true;
                continue;
            }
            "var" | "const" | "begin" => {
                in_type_block = false;
            }
            _ => {}
        }

        if let Some(rest) = lower.strip_prefix("property ") {
            let at = line.len() - rest.len();
            if let Some(name) = line[at..].split([':', ' ']).next() {
                view.properties.push(name.trim().to_owned());
            }
            continue;
        }
        if let Some(routine) = routine_of(line, exported) {
            view.routines.push(routine);
            continue;
        }
        if in_type_block && let Some((name, definition)) = line.split_once('=') {
            let name = name.trim().to_owned();
            if name.is_empty() || name.contains(' ') {
                continue;
            }
            let definition = definition.trim().to_ascii_lowercase();
            if definition.starts_with("class") || definition.starts_with("object") {
                view.classes.push(name.clone());
            }
            view.types.push(name);
        }
    }

    // A routine promised in the interface and never written is not a
    // style question: the unit will not link.
    for routine in view.routines.iter().filter(|routine| routine.exported) {
        let written = view
            .routines
            .iter()
            .any(|other| !other.exported && other.name.ends_with(&routine.name));
        if !written {
            view.declared_but_not_written
                .push(format!("{} {}", routine.kind, routine.name));
        }
    }
    view
}

/// Whether `text` is Pascal.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    if view.kind == "unstated" {
        return false;
    }
    // A header alone is not enough - `program` and `unit` are ordinary
    // words. It has to close the way Pascal closes, or declare something.
    let stripped = without_comments(text).to_ascii_lowercase();
    stripped.contains("end.") || stripped.contains("implementation") || !view.routines.is_empty()
}

/// The Pascal plugin's core half.
#[derive(Debug, Default)]
pub struct PascalCore;

impl PluginCore for PascalCore {
    fn name(&self) -> &'static str {
        "pascal"
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

/// The Pascal plugin's presentation half.
#[derive(Debug, Default)]
pub struct PascalPresentation;

impl PluginPresentation for PascalPresentation {
    fn name(&self) -> &'static str {
        "pascal"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "PAS",
            tint: 0x0000_5aa0,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: PascalView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "{} {}",
            view.kind,
            view.name.as_deref().unwrap_or("(unnamed)")
        ));
        if !view.uses.is_empty() {
            lines.push(format!("Uses: {}", view.uses.join(", ")));
        }
        if !view.types.is_empty() {
            lines.push(format!("Types: {}", view.types.join(", ")));
        }
        if !view.classes.is_empty() {
            lines.push(format!("Classes: {}", view.classes.join(", ")));
        }
        if !view.properties.is_empty() {
            lines.push(format!("Properties: {}", view.properties.join(", ")));
        }
        lines.push(format!("{} routine(s):", view.routines.len()));
        for routine in &view.routines {
            let returns = routine
                .returns
                .as_ref()
                .map_or_else(String::new, |said| format!(": {said}"));
            let where_it_is = if routine.exported {
                "interface"
            } else {
                "implementation"
            };
            lines.push(format!(
                "  {} {}({}){returns}  [{where_it_is}]",
                routine.kind,
                routine.name,
                routine.parameters.join("; ")
            ));
        }
        if !view.declared_but_not_written.is_empty() {
            lines.push("Promised in the interface and never written, so this unit".to_owned());
            lines.push("will not link:".to_owned());
            for routine in &view.declared_but_not_written {
                lines.push(format!("  {routine}"));
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
    use super::{PascalCore, PascalPresentation, PascalView, parse, without_comments};
    use plugin_api::{PluginCore, PluginPresentation};

    const UNIT: &str = concat!(
        "unit CsvStats;\n",
        "\n",
        "{ An older version lived here:\n",
        "  procedure Ghost(A: Integer);\n",
        "}\n",
        "(* And another:\n",
        "   function Phantom: Integer;\n",
        "*)\n",
        "\n",
        "interface\n",
        "\n",
        "uses\n",
        "  SysUtils, Classes;\n",
        "\n",
        "type\n",
        "  TSample = record\n",
        "    Name: string;\n",
        "    Value: Double;\n",
        "  end;\n",
        "\n",
        "  TColumn = class(TObject)\n",
        "  private\n",
        "    FValues: array of Double;\n",
        "  public\n",
        "    constructor Create(const AName: string);\n",
        "    function Mean: Double;\n",
        "    procedure Add(Value: Double);\n",
        "    property Count: Integer read GetCount;\n",
        "  end;\n",
        "\n",
        "function Largest(const Values: array of Double): Double;\n",
        "procedure Unwritten(A: Integer);\n",
        "\n",
        "implementation\n",
        "\n",
        "uses\n",
        "  Math;\n",
        "\n",
        "constructor TColumn.Create(const AName: string);\n",
        "begin\n",
        "end;\n",
        "\n",
        "function TColumn.Mean: Double;\n",
        "begin\n",
        "  Result := 0;\n",
        "end;\n",
        "\n",
        "procedure TColumn.Add(Value: Double);\n",
        "begin\n",
        "end;\n",
        "\n",
        "function Largest(const Values: array of Double): Double;\n",
        "begin\n",
        "  Result := 0;\n",
        "end;\n",
        "\n",
        "end.\n",
    );

    #[test]
    fn sniffs_a_unit() {
        assert!(PascalCore.sniff(UNIT.as_bytes()));
    }

    #[test]
    fn does_not_claim_prose_that_opens_with_the_word_program() {
        assert!(!PascalCore.sniff(b"program the machine before you use it\n"));
        assert!(!PascalCore.sniff(b""));
    }

    #[test]
    fn all_three_kinds_of_comment_are_comments() {
        let stripped = without_comments(UNIT);

        assert!(!stripped.contains("Ghost"), "a brace comment is a comment");
        assert!(
            !stripped.contains("Phantom"),
            "a bracket-star comment is one too"
        );
        let view = parse(UNIT);
        assert!(!view.routines.iter().any(|r| r.name == "Ghost"));
    }

    #[test]
    fn reads_the_header_and_both_uses_clauses() {
        let view = parse(UNIT);

        assert_eq!(view.kind, "unit");
        assert_eq!(view.name.as_deref(), Some("CsvStats"));
        assert_eq!(
            view.uses,
            vec![
                "SysUtils (interface)".to_owned(),
                "Classes (interface)".to_owned(),
                "Math (implementation)".to_owned()
            ],
            "which half a unit is used from is the whole of what it means"
        );
    }

    #[test]
    fn separates_what_is_exported_from_what_is_not() {
        let view = parse(UNIT);

        let mean: Vec<_> = view
            .routines
            .iter()
            .filter(|r| r.name.ends_with("Mean"))
            .collect();
        assert_eq!(
            mean.len(),
            2,
            "declared in the interface, written in the implementation"
        );
        assert!(mean.iter().any(|r| r.exported));
        assert!(mean.iter().any(|r| !r.exported));
    }

    #[test]
    fn reads_the_types_the_class_and_its_property() {
        let view = parse(UNIT);

        assert!(view.types.contains(&"TSample".to_owned()));
        assert_eq!(view.classes, vec!["TColumn".to_owned()]);
        assert_eq!(view.properties, vec!["Count".to_owned()]);
    }

    #[test]
    fn a_function_keeps_its_return_type() {
        let view = parse(UNIT);

        let largest = view
            .routines
            .iter()
            .find(|r| r.name == "Largest" && r.exported)
            .unwrap();
        assert_eq!(largest.returns.as_deref(), Some("Double"));
        assert_eq!(largest.kind, "function");
        assert_eq!(largest.parameters.len(), 1);
    }

    #[test]
    fn names_what_was_promised_and_never_written() {
        let view = parse(UNIT);

        assert_eq!(
            view.declared_but_not_written,
            vec!["procedure Unwritten".to_owned()],
            "everything else in the interface has a body below"
        );
    }

    #[test]
    fn presents_the_missing_body_with_its_consequence() {
        let data = serde_json::to_value(parse(UNIT)).unwrap();

        let lines = PascalPresentation.present(&data);

        assert_eq!(lines[0], "unit CsvStats");
        assert!(lines.iter().any(|line| line.contains("will not link")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/pascal/csvstats.pas");

        let data = PascalCore.view(&path).unwrap();
        let view: PascalView = serde_json::from_value(data).unwrap();

        assert_eq!(view.kind, "unit");
        assert!(view.name.is_some());
        assert!(view.uses.len() >= 3);
        assert!(view.types.len() >= 2);
        assert!(!view.classes.is_empty());
        assert!(view.properties.len() >= 2);
        assert!(view.routines.len() >= 8);
        assert!(view.routines.iter().any(|r| r.exported));
        assert!(view.routines.iter().any(|r| !r.exported));
        assert!(view.routines.iter().any(|r| r.returns.is_some()));
        assert!(view.routines.iter().any(|r| r.kind == "constructor"));
        assert!(!view.declared_but_not_written.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::PascalCore),
            plugin_api::PluginPresentation::extensions(&crate::PascalPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
