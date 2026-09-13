//! CUE file type plugin: core and presentation halves.
//!
//! CUE does not separate a schema from the data it validates. Both are
//! values, and checking one against the other is the same operation as
//! combining them - which is why a file holds definitions and concrete
//! instances side by side, and why the interesting thing to report is
//! which of the two each top-level field is.
//!
//! The constraints are written into the fields rather than declared
//! apart from them, so they are read where they sit.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["cue"];

/// How much of a file is read.
const READ_CAP: usize = 1024 * 1024;

/// How many of each kind are listed before the rest are only counted.
const SHOWN: usize = 48;

/// One field of a definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// Its name.
    pub name: String,
    /// Whether it may be left out. A `?` after the name is the whole of
    /// that, and the difference between a field that must be supplied
    /// and one that need not.
    pub optional: bool,
    /// What it is constrained to, as written.
    pub constraint: String,
    /// The value it takes when nothing else says, marked `*` in a
    /// disjunction.
    pub default: Option<String>,
}

/// One definition: a closed value other values are checked against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    /// Its name, with the leading hash.
    pub name: String,
    /// Its fields.
    pub fields: Vec<Field>,
}

/// View data produced by [`CueCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CueView {
    /// The package the file belongs to.
    pub package: Option<String>,
    /// What it imports.
    pub imports: Vec<String>,
    /// The definitions it declares.
    pub definitions: Vec<Definition>,
    /// The top-level fields that are concrete values rather than
    /// definitions, and which definition each is unified with.
    pub instances: Vec<String>,
    /// Whether the file was longer than this reads.
    pub truncated: bool,
}

/// Whether `text` reads like CUE.
///
/// A `package` clause alone is Go's as well, so a definition or a
/// constraint expression is asked for alongside it.
fn looks_like_it(text: &str) -> bool {
    let mut package = false;
    let mut cue_only = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with("package ") {
            package = true;
        }
        // A definition: `#Name:` at the start of a line.
        if trimmed.starts_with('#')
            && trimmed
                .trim_start_matches('#')
                .split(':')
                .next()
                .is_some_and(|name| {
                    !name.is_empty() && name.chars().all(|one| one.is_alphanumeric() || one == '_')
                })
            && trimmed.contains(':')
        {
            cue_only += 2;
        }
        // A constraint: a bound, or a disjunction with a default.
        if trimmed.contains(">=") && trimmed.contains('&') && trimmed.contains(':') {
            cue_only += 1;
        }
        if trimmed.contains("| *") {
            cue_only += 1;
        }
        if package && cue_only >= 1 {
            return true;
        }
        if cue_only >= 3 {
            return true;
        }
    }
    false
}

/// How deeply indented a line is, counting a tab as one.
fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The default in a disjunction, which is the branch marked `*`.
fn default_in(constraint: &str) -> Option<String> {
    constraint.split('|').find_map(|branch| {
        branch
            .trim()
            .strip_prefix('*')
            .map(|value| value.trim().to_owned())
    })
}

/// Everything [`CueView`] holds, read from `source`.
fn parse(source: &str, truncated: bool) -> CueView {
    let lines: Vec<&str> = source.lines().collect();
    let mut view = CueView {
        package: None,
        imports: Vec::new(),
        definitions: Vec::new(),
        instances: Vec::new(),
        truncated,
    };
    let mut in_imports = false;
    let mut current: Option<usize> = None;
    let mut opened_at = 0usize;
    // How many brackets are open. An instance is a field at the top
    // level; without this, every line inside one was taken as another.
    let mut depth = 0usize;

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        let was = depth;
        depth = depth
            .saturating_add(trimmed.matches(['{', '[']).count())
            .saturating_sub(trimmed.matches(['}', ']']).count());
        if in_imports {
            if trimmed.starts_with(')') {
                in_imports = false;
            } else if let Some(path) = quoted(trimmed) {
                view.imports.push(path.to_owned());
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("package ") {
            view.package = Some(rest.trim().to_owned());
            continue;
        }
        if trimmed.starts_with("import (") {
            in_imports = true;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("import ")
            && let Some(path) = quoted(rest)
        {
            view.imports.push(path.to_owned());
            continue;
        }
        // A definition's fields are the lines indented past it, up to
        // the line that closes its brace.
        if let Some(at) = current {
            if indent_of(line) <= opened_at && trimmed.starts_with('}') {
                current = None;
                continue;
            }
            read_field(trimmed, &mut view.definitions[at]);
            continue;
        }
        if was == 0
            && trimmed.starts_with('#')
            && let Some((name, rest)) = trimmed.split_once(':')
            && rest.trim().ends_with('{')
        {
            view.definitions.push(Definition {
                name: name.trim().to_owned(),
                fields: Vec::new(),
            });
            current = Some(view.definitions.len() - 1);
            opened_at = indent_of(line);
            continue;
        }
        // A top-level field that is not a definition is an instance,
        // and what it is unified with is what a reader wants to see.
        if was == 0
            && let Some((name, rest)) = trimmed.split_once(':')
            && !name.starts_with('#')
            && !name.contains(' ')
            && !name.is_empty()
            && view.instances.len() < SHOWN
        {
            let against = rest
                .trim()
                .trim_end_matches('{')
                .trim()
                .trim_end_matches('&')
                .trim();
            view.instances.push(if against.is_empty() {
                name.trim().to_owned()
            } else {
                format!("{} of {against}", name.trim())
            });
        }
    }
    view
}

/// The text inside the first pair of double quotes.
fn quoted(text: &str) -> Option<&str> {
    let start = text.find('"')? + 1;
    let rest = &text[start..];
    Some(&rest[..rest.find('"')?])
}

/// Reads one of a definition's fields.
fn read_field(line: &str, definition: &mut Definition) {
    let Some((name, constraint)) = line.split_once(':') else {
        return;
    };
    let name = name.trim();
    if name.is_empty() || name.contains(' ') || definition.fields.len() >= SHOWN {
        return;
    }
    let constraint = constraint.trim().trim_end_matches(',').trim();
    definition.fields.push(Field {
        name: name.trim_end_matches('?').to_owned(),
        optional: name.ends_with('?'),
        constraint: constraint.to_owned(),
        default: default_in(constraint),
    });
}

/// Everything [`CueView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<CueView> {
    let source = std::fs::read_to_string(path)?;
    let truncated = source.len() > READ_CAP;
    let source = if truncated {
        let mut end = READ_CAP;
        while end > 0 && !source.is_char_boundary(end) {
            end -= 1;
        }
        &source[..end]
    } else {
        source.as_str()
    };
    if !looks_like_it(source) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not CUE"));
    }
    Ok(parse(source, truncated))
}

/// The CUE plugin's core half.
#[derive(Debug, Default)]
pub struct CueCore;

impl PluginCore for CueCore {
    fn name(&self) -> &'static str {
        "cue"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The CUE plugin's presentation half.
#[derive(Debug, Default)]
pub struct CuePresentation;

impl PluginPresentation for CuePresentation {
    fn name(&self) -> &'static str {
        "cue"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "CUE",
            tint: 0x0000_66cc,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: CueView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![match &view.package {
            Some(package) => format!(
                "CUE package {package}: {} definition(s), {} instance(s)",
                view.definitions.len(),
                view.instances.len()
            ),
            None => "CUE, belonging to no package".to_owned(),
        }];
        if view.truncated {
            lines.push("Longer than this reads; what follows is the start.".to_owned());
        }
        if !view.imports.is_empty() {
            lines.push(format!("Imports {}", view.imports.join(", ")));
        }
        for definition in &view.definitions {
            lines.push(format!(
                "{} - closed, so an extra field is an error:",
                definition.name
            ));
            for field in &definition.fields {
                let optional = if field.optional {
                    " (may be left out)"
                } else {
                    ""
                };
                lines.push(format!("  {}: {}{optional}", field.name, field.constraint));
                if let Some(default) = &field.default {
                    lines.push(format!("      {default} unless something says otherwise"));
                }
            }
        }
        if view.instances.is_empty() {
            lines.push("No concrete values: this file is only a schema.".to_owned());
        } else {
            lines.push("Concrete:".to_owned());
            for instance in &view.instances {
                lines.push(format!("  {instance}"));
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{CueCore, CuePresentation, CueView, default_in, looks_like_it};
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/cue/readings.cue")
    }

    fn view_of() -> CueView {
        serde_json::from_value(CueCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&CueCore),
            PluginPresentation::extensions(&CuePresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn a_package_clause_alone_is_go_as_well() {
        assert!(looks_like_it("package a\n\n#Thing: {\n  x: int\n}\n"));
        assert!(
            !looks_like_it("package main\n\nimport \"fmt\"\n\nfunc main() {}\n"),
            "that is Go"
        );
        assert!(!looks_like_it(""));
    }

    #[test]
    fn the_default_is_the_branch_marked_with_a_star() {
        assert_eq!(
            default_in("\"a\" | \"b\" | *\"c\""),
            Some("\"c\"".to_owned())
        );
        assert_eq!(default_in("bool | *true"), Some("true".to_owned()));
        assert_eq!(default_in("int & >=1"), None);
    }

    #[test]
    fn reads_the_package_and_the_imports() {
        let view = view_of();

        assert_eq!(view.package.as_deref(), Some("readings"));
        assert_eq!(view.imports, vec!["strings", "time"]);
    }

    #[test]
    fn reads_the_definitions_and_their_fields() {
        let view = view_of();

        let named: Vec<&str> = view
            .definitions
            .iter()
            .map(|one| one.name.as_str())
            .collect();
        assert_eq!(named, vec!["#Reading", "#Column", "#Retention"]);

        let reading = &view.definitions[0];
        assert_eq!(reading.fields.len(), 5);
        assert_eq!(reading.fields[0].name, "sensor");
        assert!(reading.fields[0].constraint.contains("strings.MinRunes(3)"));
    }

    #[test]
    fn an_optional_field_is_told_from_one_that_must_be_supplied() {
        let view = view_of();

        let reading = &view.definitions[0];
        let note = reading
            .fields
            .iter()
            .find(|one| one.name == "note")
            .expect("the optional field");
        assert!(note.optional, "the `?` is the whole of it");
        assert!(
            reading
                .fields
                .iter()
                .filter(|one| one.name != "note")
                .all(|one| !one.optional),
            "and nothing else is optional"
        );
    }

    #[test]
    fn reads_the_constraints_as_written() {
        let view = view_of();

        let celsius = view.definitions[0]
            .fields
            .iter()
            .find(|one| one.name == "celsius")
            .expect("the bounded field");
        assert_eq!(celsius.constraint, ">=-90.0 & <=60.0");

        let quality = view.definitions[0]
            .fields
            .iter()
            .find(|one| one.name == "quality")
            .expect("the disjunction");
        assert_eq!(quality.constraint, "\"good\" | \"suspect\" | \"missing\"");
        assert_eq!(quality.default, None, "no branch is marked");
    }

    #[test]
    fn reads_the_defaults_a_disjunction_marks() {
        let view = view_of();

        let unit = view.definitions[1]
            .fields
            .iter()
            .find(|one| one.name == "unit")
            .expect("the field with a default");
        assert_eq!(unit.default.as_deref(), Some("\"celsius\""));

        let retention = &view.definitions[2];
        assert_eq!(
            retention
                .fields
                .iter()
                .find(|one| one.name == "compress")
                .and_then(|one| one.default.clone())
                .as_deref(),
            Some("true")
        );
    }

    #[test]
    fn a_field_inside_an_instance_is_not_another_instance() {
        // Without counting brackets, every line of the concrete
        // `column` - each reading, each field of each reading - was
        // reported as a top-level instance of its own.
        let view = view_of();

        assert!(
            !view.instances.iter().any(|one| one.starts_with("sensor ")),
            "{:?}",
            view.instances
        );
        assert!(!view.instances.iter().any(|one| one.contains("celsius")));
    }

    #[test]
    fn reads_the_concrete_instances_and_what_each_is_checked_against() {
        let view = view_of();

        assert_eq!(
            view.instances,
            vec!["column of #Column", "retention of #Retention"],
            "a top-level field that is not a definition is an instance"
        );
    }

    #[test]
    fn presents_what_is_a_schema_and_what_is_a_value() {
        let data = CueCore.view(&fixture()).unwrap();

        let lines = CuePresentation.present(&data);

        assert!(lines[0].starts_with("CUE package readings: 3 definition(s), 2 instance(s)"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("#Reading - closed, so an extra field is an error"))
        );
        assert!(lines.iter().any(|line| line.contains("(may be left out)")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("unless something says otherwise"))
        );
        assert!(lines.iter().any(|line| line.contains("column of #Column")));
    }

    #[test]
    fn a_file_that_is_not_cue_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.cue");
        std::fs::write(&path, b"nothing of the sort at all").unwrap();

        assert!(CueCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
