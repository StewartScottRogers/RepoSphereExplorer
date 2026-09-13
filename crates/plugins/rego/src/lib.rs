//! Rego policy file type plugin: core and presentation halves.
//!
//! Rego rules are not statements that run in order. Each one either
//! holds or it does not, and the answer is whatever holds. Two things
//! follow, and both are what a reader needs told:
//!
//! A `default` is the answer when no rule holds, which is the whole
//! reason a policy is safe while it is still incomplete. And a rule
//! written with `contains` is *partial*: every body that holds adds to
//! a set rather than deciding it, so a partial `deny` collects every
//! reason a request was refused instead of stopping at the first.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["rego"];

/// How much of a policy is read.
const READ_CAP: usize = 1024 * 1024;

/// How many of each kind are listed before the rest are only counted.
const SHOWN: usize = 48;

/// One rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// Its name.
    pub name: String,
    /// Whether it is partial: every body that holds contributes to a
    /// set, rather than one deciding the answer.
    pub partial: bool,
    /// How many bodies it has. More than one is an "or".
    pub bodies: usize,
    /// What it is when nothing holds.
    pub default: Option<String>,
}

/// View data produced by [`RegoCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegoView {
    /// The package the policy belongs to.
    pub package: Option<String>,
    /// What it imports.
    pub imports: Vec<String>,
    /// The rules it defines.
    pub rules: Vec<Rule>,
    /// The functions it defines, which take arguments.
    pub functions: Vec<String>,
    /// Whether it reads the request under `input`.
    pub reads_input: bool,
    /// Whether it reads stored data under `data`.
    pub reads_data: bool,
    /// Whether the policy was longer than this reads.
    pub truncated: bool,
}

/// Whether `text` reads like a Rego policy.
///
/// A `package` clause is Go's and CUE's as well, so a Rego-only marker
/// is asked for alongside it.
fn looks_like_it(text: &str) -> bool {
    let mut package = false;
    let mut rego_only = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with("package ") {
            package = true;
        }
        if trimmed.starts_with("default ") {
            rego_only += 2;
        }
        for marker in [
            "input.",
            "data.",
            " if {",
            "contains ",
            ":= true",
            ":= false",
        ] {
            if trimmed.contains(marker) {
                rego_only += 1;
            }
        }
        if package && rego_only >= 2 {
            return true;
        }
    }
    false
}

/// The name a rule head declares, up to whatever ends it.
fn head_name(line: &str) -> Option<&str> {
    let end = line.find(|character: char| {
        !character.is_alphanumeric() && character != '_' && character != '.'
    })?;
    (end > 0).then(|| &line[..end])
}

/// Everything [`RegoView`] holds, read from `source`.
fn parse(source: &str, truncated: bool) -> RegoView {
    let mut view = RegoView {
        package: None,
        imports: Vec::new(),
        rules: Vec::new(),
        functions: Vec::new(),
        reads_input: source.contains("input."),
        reads_data: source.contains("data."),
        truncated,
    };
    for line in source.lines() {
        // Only a rule head sits at the left margin; a body is indented.
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('}') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("package ") {
            view.package = Some(rest.trim().to_owned());
        } else if let Some(rest) = trimmed.strip_prefix("import ") {
            view.imports.push(rest.trim().to_owned());
        } else if let Some(rest) = trimmed.strip_prefix("default ") {
            read_default(rest, &mut view);
        } else {
            read_head(trimmed, &mut view);
        }
    }
    view
}

/// Reads a `default name := value` line, which may come before or after
/// the rule it belongs to.
fn read_default(rest: &str, view: &mut RegoView) {
    let Some(name) = head_name(rest.trim()) else {
        return;
    };
    let value = rest
        .split_once(":=")
        .map(|(_, value)| value.trim().to_owned());
    match view.rules.iter_mut().find(|rule| rule.name == name) {
        Some(rule) => rule.default = value,
        None => {
            if view.rules.len() < SHOWN {
                view.rules.push(Rule {
                    name: name.to_owned(),
                    partial: false,
                    bodies: 0,
                    default: value,
                });
            }
        }
    }
}

/// Reads a rule head, which is anything else at the left margin.
fn read_head(trimmed: &str, view: &mut RegoView) {
    let Some(name) = head_name(trimmed) else {
        return;
    };
    let after = &trimmed[name.len()..];
    // A function takes arguments; a rule does not.
    if after.starts_with('(') {
        if !view.functions.iter().any(|had| had == name) && view.functions.len() < SHOWN {
            view.functions.push(name.to_owned());
        }
        return;
    }
    // A head has to go on to say something: `x := y`, `x contains y`,
    // `x if {`, or the older `x[y] {`.
    let after = after.trim_start();
    let partial = after.starts_with("contains ");
    if !(partial
        || after.starts_with(":=")
        || after.starts_with("if ")
        || after.starts_with('[')
        || after.starts_with('{')
        || after.starts_with("= "))
    {
        return;
    }
    match view.rules.iter_mut().find(|rule| rule.name == name) {
        Some(rule) => {
            rule.bodies += 1;
            rule.partial = rule.partial || partial;
        }
        None => {
            if view.rules.len() < SHOWN {
                view.rules.push(Rule {
                    name: name.to_owned(),
                    partial,
                    bodies: 1,
                    default: None,
                });
            }
        }
    }
}

/// Everything [`RegoView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<RegoView> {
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
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a Rego policy",
        ));
    }
    Ok(parse(source, truncated))
}

/// The Rego plugin's core half.
#[derive(Debug, Default)]
pub struct RegoCore;

impl PluginCore for RegoCore {
    fn name(&self) -> &'static str {
        "rego"
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

/// The Rego plugin's presentation half.
#[derive(Debug, Default)]
pub struct RegoPresentation;

impl PluginPresentation for RegoPresentation {
    fn name(&self) -> &'static str {
        "rego"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "REGO",
            tint: 0x007d_9199,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: RegoView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![match &view.package {
            Some(package) => format!("Rego policy {package}: {} rule(s)", view.rules.len()),
            None => "Rego, belonging to no package".to_owned(),
        }];
        if view.truncated {
            lines.push("Longer than this reads; what follows is the start.".to_owned());
        }
        lines.push(match (view.reads_input, view.reads_data) {
            (true, true) => "Decides on the request and on stored data.".to_owned(),
            (true, false) => "Decides on the request alone.".to_owned(),
            (false, true) => "Decides on stored data alone.".to_owned(),
            (false, false) => "Reads neither the request nor stored data.".to_owned(),
        });
        if !view.imports.is_empty() {
            lines.push("Imports:".to_owned());
            for import in &view.imports {
                lines.push(format!("  {import}"));
            }
        }
        for rule in &view.rules {
            let mut said = format!("  {}", rule.name);
            if rule.partial {
                said.push_str(" - partial: every body that holds adds to the set");
            } else if rule.bodies > 1 {
                use std::fmt::Write as _;
                let _ = write!(said, " - {} bodies, any of which decides it", rule.bodies);
            }
            lines.push(said);
            if let Some(default) = &rule.default {
                lines.push(format!("      {default} when nothing holds"));
            } else if rule.bodies > 0 {
                lines.push("      undefined when nothing holds".to_owned());
            }
        }
        if !view.functions.is_empty() {
            lines.push(format!("Functions: {}", view.functions.join(", ")));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{RegoCore, RegoPresentation, RegoView, head_name, looks_like_it};
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/rego/authz.rego")
    }

    fn view_of() -> RegoView {
        serde_json::from_value(RegoCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&RegoCore),
            PluginPresentation::extensions(&RegoPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn a_package_clause_alone_is_not_enough() {
        assert!(looks_like_it(
            "package a\n\ndefault allow := false\n\nallow if {\n  input.x\n}\n"
        ));
        assert!(
            !looks_like_it("package main\n\nimport \"fmt\"\n\nfunc main() {}\n"),
            "that is Go"
        );
        assert!(!looks_like_it(""));
    }

    #[test]
    fn a_head_name_stops_at_what_ends_it() {
        assert_eq!(head_name("allow if {"), Some("allow"));
        assert_eq!(head_name("within_shift(moment) if {"), Some("within_shift"));
        assert_eq!(head_name("deny contains message if {"), Some("deny"));
        assert_eq!(head_name("{ nothing }"), None);
    }

    #[test]
    fn reads_the_package_and_the_imports() {
        let view = view_of();

        assert_eq!(view.package.as_deref(), Some("readings.authz"));
        assert_eq!(view.imports.len(), 5);
        assert!(view.imports.iter().any(|one| one == "future.keywords.if"));
        assert!(view.imports.iter().any(|one| one == "data.readings.roles"));
    }

    #[test]
    fn a_rule_with_several_bodies_is_counted_once() {
        let view = view_of();

        let allow = view
            .rules
            .iter()
            .find(|one| one.name == "allow")
            .expect("the allow rule");
        assert_eq!(
            allow.bodies, 3,
            "three ways to be allowed, and any one of them decides it"
        );
        assert!(!allow.partial);
    }

    #[test]
    fn a_partial_rule_is_told_from_a_complete_one() {
        let view = view_of();

        let deny = view
            .rules
            .iter()
            .find(|one| one.name == "deny")
            .expect("the deny rule");
        assert!(
            deny.partial,
            "`contains` makes it a set: every reason, not the first"
        );
        assert_eq!(deny.bodies, 2);
    }

    #[test]
    fn reads_the_defaults_whether_they_come_before_or_after_the_rule() {
        let view = view_of();

        let named = |name: &str| {
            view.rules
                .iter()
                .find(|one| one.name == name)
                .and_then(|one| one.default.clone())
        };
        assert_eq!(named("allow").as_deref(), Some("false"));
        assert_eq!(named("max_rows").as_deref(), Some("1000"));
        assert_eq!(
            named("reason").as_deref(),
            Some("\"no rule matched\""),
            "its `default` comes first and its rule later, and both are the same rule"
        );
    }

    #[test]
    fn a_function_is_told_from_a_rule() {
        let view = view_of();

        assert_eq!(view.functions, vec!["within_shift"]);
        assert!(
            !view.rules.iter().any(|one| one.name == "within_shift"),
            "it takes an argument, so it is not a rule"
        );
    }

    #[test]
    fn says_what_the_policy_decides_on() {
        let view = view_of();

        assert!(view.reads_input);
        assert!(view.reads_data);
    }

    #[test]
    fn presents_what_happens_when_nothing_holds() {
        let data = RegoCore.view(&fixture()).unwrap();

        let lines = RegoPresentation.present(&data);

        assert!(lines[0].starts_with("Rego policy readings.authz"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("false when nothing holds"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("partial: every body that holds adds to the set"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("3 bodies, any of which decides it"))
        );
    }

    #[test]
    fn a_file_that_is_not_rego_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.rego");
        std::fs::write(&path, b"nothing of the sort at all").unwrap();

        assert!(RegoCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
