//! Ninja build file file type plugin: core and presentation halves.
//!
//! Ninja is not written by people. It is what a build system emits, and
//! it is read exactly once - when something has gone wrong and somebody
//! needs to know what the generator actually asked for.
//!
//! So the mapping is what matters: which rule builds a given output,
//! from which inputs, with which command. Everything else in the file
//! is in service of that.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["ninja"];

/// How much of a build file is read. A generated one can be enormous.
const READ_CAP: usize = 4 * 1024 * 1024;

/// How many of each kind are listed before the rest are only counted.
const SHOWN: usize = 64;

/// One rule: a command, and how the output is described while it runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// Its name.
    pub name: String,
    /// The command it runs.
    pub command: Option<String>,
    /// What is printed while it runs.
    pub description: Option<String>,
    /// Where it writes the header dependencies it discovered.
    pub depfile: Option<String>,
    /// Whether it restats its output, so an unchanged output does not
    /// make everything downstream rebuild.
    pub restat: bool,
}

/// One build statement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Statement {
    /// What it produces.
    pub outputs: Vec<String>,
    /// The rule that produces it.
    pub rule: String,
    /// What it is produced from.
    pub inputs: Vec<String>,
    /// What must be built first without being an input - after a `|`.
    pub implicit: Vec<String>,
    /// What must merely exist first - after a `||`.
    pub order_only: Vec<String>,
}

/// View data produced by [`NinjaCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NinjaView {
    /// The oldest Ninja that will read it.
    pub required_version: Option<String>,
    /// Where it puts what it builds.
    pub build_directory: Option<String>,
    /// The variables it sets at the top level.
    pub variables: Vec<String>,
    /// The rules it declares.
    pub rules: Vec<Rule>,
    /// The build statements.
    pub statements: Vec<Statement>,
    /// How many statements there are in total.
    pub statement_count: usize,
    /// The targets that exist only to be named, and produce nothing.
    pub phony_targets: Vec<String>,
    /// What is built when nothing is named on the command line.
    pub defaults: Vec<String>,
    /// Files pulled in with `include`, which share this file's
    /// variables.
    pub includes: Vec<String>,
    /// Files pulled in with `subninja`, which do not.
    pub subninjas: Vec<String>,
    /// Whether the file was longer than this reads.
    pub truncated: bool,
}

/// Whether `text` reads like a Ninja build file.
///
/// A `rule` and a `build` at the start of lines, and the `build`
/// statement's colon, are the shape. A makefile has colons too, and no
/// `rule` keyword.
fn looks_like_it(text: &str) -> bool {
    let mut rules = 0usize;
    let mut builds = 0usize;
    for line in text.lines() {
        if line.starts_with("rule ") {
            rules += 1;
        }
        if line.starts_with("build ") && line.contains(':') {
            builds += 1;
        }
        if rules >= 1 && builds >= 1 {
            return true;
        }
    }
    false
}

/// The lines with each continuation joined onto the line it continues.
///
/// Ninja continues a line with a trailing `$`, which is also its escape
/// character - `$$` is a literal dollar and continues nothing.
fn logical_lines(source: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut pending: Option<String> = None;
    for line in source.lines() {
        let continues = line.ends_with('$') && !line.ends_with("$$");
        let body = if continues {
            &line[..line.len() - 1]
        } else {
            line
        };
        match &mut pending {
            Some(carried) => carried.push_str(body.trim_start()),
            None => pending = Some(body.to_owned()),
        }
        if !continues && let Some(whole) = pending.take() {
            found.push(whole);
        }
    }
    if let Some(whole) = pending {
        found.push(whole);
    }
    found
}

/// The paths in a run of text, split on unescaped spaces.
///
/// A path with a space in it is escaped as `$ `, which is why this
/// cannot simply split on whitespace.
fn paths_in(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut current = String::new();
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '$' && characters.peek() == Some(&' ') {
            characters.next();
            current.push(' ');
            continue;
        }
        if character.is_whitespace() {
            if !current.is_empty() {
                found.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(character);
    }
    if !current.is_empty() {
        found.push(current);
    }
    found
}

/// A build statement's inputs, split into the three kinds Ninja has.
fn split_inputs(text: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    // The order matters: `||` has to be found before `|`.
    let (before, order_only) = match text.split_once("||") {
        Some((before, after)) => (before, paths_in(after)),
        None => (text, Vec::new()),
    };
    let (plain, implicit) = match before.split_once('|') {
        Some((plain, after)) => (plain, paths_in(after)),
        None => (before, Vec::new()),
    };
    (paths_in(plain), implicit, order_only)
}

/// Everything [`NinjaView`] holds, read from `source`.
fn parse(source: &str, truncated: bool) -> NinjaView {
    let lines = logical_lines(source);
    let mut view = NinjaView {
        required_version: None,
        build_directory: None,
        variables: Vec::new(),
        rules: Vec::new(),
        statements: Vec::new(),
        statement_count: 0,
        phony_targets: Vec::new(),
        defaults: Vec::new(),
        includes: Vec::new(),
        subninjas: Vec::new(),
        truncated,
    };
    // An indented `name = value` belongs to whatever was declared last.
    let mut in_rule: Option<usize> = None;

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            if let Some(at) = in_rule
                && let Some((name, value)) = trimmed.split_once('=')
            {
                read_rule_binding(&mut view.rules[at], name.trim(), value.trim());
            }
            continue;
        }
        in_rule = None;
        if let Some(rest) = trimmed.strip_prefix("rule ") {
            view.rules.push(Rule {
                name: rest.trim().to_owned(),
                command: None,
                description: None,
                depfile: None,
                restat: false,
            });
            in_rule = Some(view.rules.len() - 1);
        } else if let Some(rest) = trimmed.strip_prefix("build ") {
            read_statement(rest, &mut view);
        } else if let Some(rest) = trimmed.strip_prefix("default ") {
            view.defaults.extend(paths_in(rest));
        } else if let Some(rest) = trimmed.strip_prefix("include ") {
            view.includes.push(rest.trim().to_owned());
        } else if let Some(rest) = trimmed.strip_prefix("subninja ") {
            view.subninjas.push(rest.trim().to_owned());
        } else if let Some((name, value)) = trimmed.split_once('=') {
            let name = name.trim();
            let value = value.trim();
            match name {
                "ninja_required_version" => {
                    view.required_version = Some(value.to_owned());
                }
                "builddir" => view.build_directory = Some(value.to_owned()),
                _ => {}
            }
            if view.variables.len() < SHOWN {
                view.variables.push(format!("{name} = {value}"));
            }
        }
    }
    view
}

/// Reads one of a rule's indented bindings.
fn read_rule_binding(rule: &mut Rule, name: &str, value: &str) {
    match name {
        "command" => rule.command = Some(value.to_owned()),
        "description" => rule.description = Some(value.to_owned()),
        "depfile" => rule.depfile = Some(value.to_owned()),
        "restat" => rule.restat = value != "0" && !value.eq_ignore_ascii_case("false"),
        _ => {}
    }
}

/// Reads one build statement, given the text after `build `.
fn read_statement(rest: &str, view: &mut NinjaView) {
    let Some((outputs, after)) = rest.split_once(':') else {
        return;
    };
    let after = after.trim();
    let (rule, inputs) = after.split_once(char::is_whitespace).unwrap_or((after, ""));
    let (inputs, implicit, order_only) = split_inputs(inputs);
    let outputs = paths_in(outputs);
    view.statement_count += 1;
    if rule == "phony" {
        view.phony_targets.extend(outputs.iter().cloned());
    }
    if view.statements.len() < SHOWN {
        view.statements.push(Statement {
            outputs,
            rule: rule.to_owned(),
            inputs,
            implicit,
            order_only,
        });
    }
}

/// Everything [`NinjaView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<NinjaView> {
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
            "not a Ninja build file",
        ));
    }
    Ok(parse(source, truncated))
}

/// The Ninja plugin's core half.
#[derive(Debug, Default)]
pub struct NinjaCore;

impl PluginCore for NinjaCore {
    fn name(&self) -> &'static str {
        "ninja"
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

/// The Ninja plugin's presentation half.
#[derive(Debug, Default)]
pub struct NinjaPresentation;

impl PluginPresentation for NinjaPresentation {
    fn name(&self) -> &'static str {
        "ninja"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "NJ",
            tint: 0x0033_3333,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: NinjaView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "Ninja: {} rule(s), {} build statement(s)",
            view.rules.len(),
            view.statement_count
        )];
        if view.truncated {
            lines.push("Longer than this reads; what follows is the start.".to_owned());
        }
        if let Some(version) = &view.required_version {
            lines.push(format!("Needs Ninja {version} or newer"));
        }
        if let Some(directory) = &view.build_directory {
            lines.push(format!("Writes into {directory}"));
        }
        lines.push(if view.defaults.is_empty() {
            "No default: naming nothing builds everything.".to_owned()
        } else {
            format!("Naming nothing builds {}", view.defaults.join(", "))
        });
        if !view.includes.is_empty() {
            lines.push(format!(
                "Includes {}, which share these variables",
                view.includes.join(", ")
            ));
        }
        if !view.subninjas.is_empty() {
            lines.push(format!(
                "Pulls in {}, which do not",
                view.subninjas.join(", ")
            ));
        }
        if !view.rules.is_empty() {
            lines.push("Rules:".to_owned());
            for rule in &view.rules {
                lines.push(format!("  {}", rule.name));
                if let Some(command) = &rule.command {
                    lines.push(format!("      {command}"));
                }
                if rule.depfile.is_some() {
                    lines.push("      reads the headers the compiler found".to_owned());
                }
                if rule.restat {
                    lines.push(
                        "      restats: an unchanged output stops the rebuild here".to_owned(),
                    );
                }
            }
        }
        if !view.statements.is_empty() {
            lines.push("Builds:".to_owned());
            for statement in &view.statements {
                // A statement whose rule takes no input - a generator
                // that makes its output from nothing - would otherwise
                // leave a trailing space where the inputs would be.
                let mut said = format!("  {} <- {}", statement.outputs.join(" "), statement.rule);
                if !statement.inputs.is_empty() {
                    said.push(' ');
                    said.push_str(&statement.inputs.join(" "));
                }
                lines.push(said);
                if !statement.implicit.is_empty() {
                    lines.push(format!("      also needs {}", statement.implicit.join(" ")));
                }
                if !statement.order_only.is_empty() {
                    lines.push(format!(
                        "      after {} exists",
                        statement.order_only.join(" ")
                    ));
                }
            }
            if view.statement_count > view.statements.len() {
                lines.push(format!(
                    "  ... and {} more",
                    view.statement_count - view.statements.len()
                ));
            }
        }
        if !view.phony_targets.is_empty() {
            lines.push(format!(
                "Names that build nothing themselves: {}",
                view.phony_targets.join(", ")
            ));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NinjaCore, NinjaPresentation, NinjaView, logical_lines, looks_like_it, paths_in,
        split_inputs,
    };
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/ninja/build.ninja")
    }

    fn view_of() -> NinjaView {
        serde_json::from_value(NinjaCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&NinjaCore),
            PluginPresentation::extensions(&NinjaPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn needs_both_a_rule_and_a_build() {
        assert!(looks_like_it(
            "rule cc\n  command = gcc\n\nbuild a.o: cc a.c\n"
        ));
        assert!(
            !looks_like_it("all: a.o\n\ta.o: a.c\n"),
            "a makefile has colons and no rules"
        );
        assert!(!looks_like_it(""));
    }

    #[test]
    fn a_trailing_dollar_continues_the_line_and_a_double_one_does_not() {
        assert_eq!(
            logical_lines("build a: cc $\n    b c\n"),
            vec!["build a: cc b c"]
        );
        assert_eq!(
            logical_lines("command = echo $$\nnext = 1\n"),
            vec!["command = echo $$", "next = 1"],
            "`$$` is a literal dollar and continues nothing"
        );
    }

    #[test]
    fn an_escaped_space_stays_inside_its_path() {
        assert_eq!(paths_in("a.o b.o"), vec!["a.o", "b.o"]);
        assert_eq!(
            paths_in("my$ file.o other.o"),
            vec!["my file.o", "other.o"],
            "`$ ` is a space in a path, not a separator"
        );
    }

    #[test]
    fn the_three_kinds_of_input_are_kept_apart() {
        let (inputs, implicit, order_only) = split_inputs("a.c b.c | header.h || generated/");

        assert_eq!(inputs, vec!["a.c", "b.c"]);
        assert_eq!(implicit, vec!["header.h"]);
        assert_eq!(
            order_only,
            vec!["generated/"],
            "`||` has to be found before `|`, or the split takes the first bar"
        );
    }

    #[test]
    fn reads_the_header_and_the_variables() {
        let view = view_of();

        assert_eq!(view.required_version.as_deref(), Some("1.11"));
        assert_eq!(view.build_directory.as_deref(), Some("out"));
        assert!(
            view.variables
                .iter()
                .any(|one| one.starts_with("cxx = clang++"))
        );
        assert!(
            view.variables
                .iter()
                .any(|one| one.starts_with("cxxflags = "))
        );
    }

    #[test]
    fn reads_the_rules_with_their_commands() {
        let view = view_of();

        assert_eq!(view.rules.len(), 4);
        let compile = &view.rules[0];
        assert_eq!(compile.name, "cxx_compile");
        assert!(
            compile
                .command
                .as_deref()
                .is_some_and(|one| one.contains("-c $in"))
        );
        assert_eq!(compile.description.as_deref(), Some("CXX $out"));
        assert_eq!(compile.depfile.as_deref(), Some("$out.d"));
        assert!(!compile.restat);

        let generate = view
            .rules
            .iter()
            .find(|one| one.name == "generate_readings")
            .expect("the generator rule");
        assert!(generate.restat, "it says `restat = 1`");
        assert_eq!(generate.depfile, None);
    }

    #[test]
    fn reads_the_build_statements() {
        let view = view_of();

        assert_eq!(view.statement_count, 9);
        let first = &view.statements[0];
        assert_eq!(first.outputs, vec!["$builddir/column.o"]);
        assert_eq!(first.rule, "cxx_compile");
        assert_eq!(first.inputs, vec!["src/column.cc"]);

        let archive = view
            .statements
            .iter()
            .find(|one| one.rule == "archive")
            .expect("the archive statement");
        assert_eq!(archive.inputs.len(), 3);
    }

    #[test]
    fn reads_the_phony_targets_the_default_and_what_is_pulled_in() {
        let view = view_of();

        assert_eq!(view.phony_targets, vec!["test", "all"]);
        assert_eq!(view.defaults, vec!["all"]);
        assert_eq!(view.includes, vec!["rules/toolchain.ninja"]);
        assert_eq!(view.subninjas, vec!["tools/build.ninja"]);
    }

    #[test]
    fn presents_the_mapping_a_reader_came_for() {
        let data = NinjaCore.view(&fixture()).unwrap();

        let lines = NinjaPresentation.present(&data);

        assert!(lines[0].starts_with("Ninja: 4 rule(s), 9 build statement(s)"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Naming nothing builds all"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("$builddir/column.o <- cxx_compile src/column.cc"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "  $builddir/readings.csv <- generate_readings"),
            "a statement with no inputs ends at its rule, with no trailing space"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("which share these variables"))
        );
        assert!(lines.iter().any(|line| line.contains("restats")));
    }

    #[test]
    fn a_file_that_is_not_ninja_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.ninja");
        std::fs::write(&path, b"nothing of the sort at all").unwrap();

        assert!(NinjaCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
