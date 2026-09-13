//! Starlark build file file type plugin: core and presentation halves.
//!
//! Starlark is Python's syntax with the parts that make a build
//! unpredictable taken out, and it is used two ways. A `BUILD` file is
//! declarative: `load` statements bringing rules in, then calls to
//! those rules. A `.bzl` file is the language proper: functions, a rule
//! implementation, a `rule()` call.
//!
//! What a reader wants from either is the same three things - what it
//! brings in, what it declares, and who is allowed to use it - so those
//! are what this reads. Visibility especially: a target that is public
//! is a promise to everybody in the repository, and one that is not is
//! a private detail.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
///
/// A `BUILD` file has no extension at all, and is recognised by what is
/// in it rather than by its name.
pub const EXTENSIONS: &[&str] = &["bzl", "bazel", "star"];

/// How much of a build file is read.
const READ_CAP: usize = 1024 * 1024;

/// How many of each kind are listed before the rest are only counted.
const SHOWN: usize = 64;

/// One `load` statement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Load {
    /// The file it loads from.
    pub from: String,
    /// The names it brings in.
    pub symbols: Vec<String>,
}

/// One target the file declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    /// The rule that makes it.
    pub rule: String,
    /// Its name.
    pub name: String,
    /// What it depends on.
    pub dependencies: Vec<String>,
    /// Who may depend on it, when it says.
    pub visibility: Vec<String>,
    /// The patterns its sources are globbed with.
    pub globs: Vec<String>,
}

/// View data produced by [`StarlarkCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StarlarkView {
    /// Which of the two kinds it is.
    pub kind: String,
    /// What it brings in.
    pub loads: Vec<Load>,
    /// What it declares.
    pub targets: Vec<Target>,
    /// The default visibility set for the package, when one is.
    pub default_visibility: Vec<String>,
    /// The functions a `.bzl` file defines.
    pub functions: Vec<String>,
    /// The rules a `.bzl` file declares with `rule()`.
    pub rules: Vec<String>,
    /// Whether the file was longer than this reads.
    pub truncated: bool,
}

/// Whether `text` reads like a Starlark build file.
///
/// The `load(` statement is Starlark's and nobody else's - Python has
/// no such thing - and a rule call is a name followed by an open
/// bracket with a `name = ` inside it.
fn looks_like_it(text: &str) -> bool {
    let mut markers = 0usize;
    let mut names = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with("load(") {
            markers += 2;
        }
        for marker in ["visibility = [", "glob(", "package(", "srcs = ", "deps = "] {
            if trimmed.contains(marker) {
                markers += 1;
            }
        }
        // An extension has none of those. What it has instead is the
        // rule-authoring vocabulary, which is equally Starlark's own.
        for marker in [
            "= rule(",
            "attr.",
            "ctx.actions.",
            "DefaultInfo",
            "depset(",
            "= provider(",
            "= aspect(",
        ] {
            if trimmed.contains(marker) {
                markers += 1;
            }
        }
        if trimmed.starts_with("name = \"") {
            names += 1;
        }
        // A rule call opens a bracket and nothing else on the line.
        if trimmed.ends_with('(')
            && trimmed[..trimmed.len() - 1]
                .chars()
                .all(|character| character.is_alphanumeric() || character == '_')
            && !trimmed.starts_with("def ")
        {
            markers += 1;
        }
        if markers + names >= 4 {
            return true;
        }
    }
    false
}

/// The strings inside `text`, in order.
fn strings_in(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('"') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('"') else { break };
        found.push(after[..close].to_owned());
        rest = &after[close + 1..];
    }
    found
}

/// Where the call opening at `from` ends, as an exclusive line index.
///
/// A target's attributes are the ones inside its own brackets. Without
/// this bound a target with no `visibility` picked up the *next*
/// target's, and reported a private test as public.
fn call_ends(lines: &[&str], from: usize) -> usize {
    let mut depth = 0usize;
    for (at, line) in lines.iter().enumerate().skip(from) {
        depth += line.matches('(').count();
        depth = depth.saturating_sub(line.matches(')').count());
        if depth == 0 {
            return at + 1;
        }
    }
    lines.len()
}

/// The strings in the `name = [ ... ]` list inside the call at `from`.
fn list_after(lines: &[&str], from: usize, name: &str) -> Vec<String> {
    let ends = call_ends(lines, from);
    let Some(at) = lines
        .iter()
        .enumerate()
        .take(ends)
        .skip(from)
        .find(|(_, line)| line.trim().starts_with(&format!("{name} = ")))
        .map(|(at, _)| at)
    else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut depth = 0usize;
    for line in lines.iter().skip(at) {
        depth += line.matches('[').count();
        found.extend(strings_in(line));
        depth = depth.saturating_sub(line.matches(']').count());
        if depth == 0 {
            break;
        }
    }
    found
}

/// Everything [`StarlarkView`] holds, read from `source`.
fn parse(source: &str, truncated: bool) -> StarlarkView {
    let lines: Vec<&str> = source.lines().collect();
    let mut view = StarlarkView {
        kind: if lines
            .iter()
            .any(|line| line.trim_start().starts_with("def ") || line.contains("= rule("))
        {
            "an extension: Starlark as a language".to_owned()
        } else {
            "a build file: Starlark used declaratively".to_owned()
        },
        loads: Vec::new(),
        targets: Vec::new(),
        default_visibility: Vec::new(),
        functions: Vec::new(),
        rules: Vec::new(),
        truncated,
    };

    for (at, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with("load(") {
            let mut named = strings_in(&joined(&lines, at));
            if named.is_empty() {
                continue;
            }
            let from = named.remove(0);
            view.loads.push(Load {
                from,
                symbols: named,
            });
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("def ") {
            let name = rest[..rest.find('(').unwrap_or(rest.len())].to_owned();
            if view.functions.len() < SHOWN {
                view.functions.push(name);
            }
            continue;
        }
        if let Some((name, _)) = trimmed.split_once(" = rule(") {
            if view.rules.len() < SHOWN {
                view.rules.push(name.trim().to_owned());
            }
            continue;
        }
        if trimmed.starts_with("package(") {
            let call = joined(&lines, at);
            view.default_visibility = match call.find("default_visibility") {
                Some(found) => strings_in(&call[found..]),
                None => Vec::new(),
            };
            continue;
        }
        // A rule call: a bare name and an open bracket.
        if trimmed.ends_with('(')
            && !trimmed.starts_with("def ")
            && trimmed[..trimmed.len() - 1]
                .chars()
                .all(|character| character.is_alphanumeric() || character == '_')
            && !trimmed.is_empty()
        {
            read_target(&lines, at, &mut view);
        }
    }
    view
}

/// A call joined onto one line, for the statements written across
/// several.
fn joined(lines: &[&str], from: usize) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for line in lines.iter().skip(from) {
        out.push_str(line.trim());
        out.push(' ');
        depth += line.matches('(').count();
        depth = depth.saturating_sub(line.matches(')').count());
        if depth == 0 {
            break;
        }
    }
    out
}

/// Reads the target whose rule call opens at `at`.
fn read_target(lines: &[&str], at: usize, view: &mut StarlarkView) {
    let rule = lines[at].trim().trim_end_matches('(').to_owned();
    let body = joined(lines, at);
    let Some(name) = body
        .find("name = \"")
        .map(|found| &body[found + 8..])
        .and_then(|rest| rest.find('"').map(|end| rest[..end].to_owned()))
    else {
        return;
    };
    if view.targets.len() >= SHOWN {
        return;
    }
    // A glob's patterns are the ones inside `glob(...)`, which is not
    // the same as everything in `srcs`: a literal source is not a glob.
    let mut globs = Vec::new();
    let mut rest = body.as_str();
    while let Some(found) = rest.find("glob(") {
        let after = &rest[found + 5..];
        let end = after.find(')').unwrap_or(after.len());
        globs.extend(strings_in(&after[..end]));
        rest = &after[end..];
    }
    view.targets.push(Target {
        rule,
        name,
        dependencies: list_after(lines, at, "deps"),
        visibility: list_after(lines, at, "visibility"),
        globs,
    });
}

/// Everything [`StarlarkView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<StarlarkView> {
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
            "not a Starlark build file",
        ));
    }
    Ok(parse(source, truncated))
}

/// The Starlark plugin's core half.
#[derive(Debug, Default)]
pub struct StarlarkCore;

impl PluginCore for StarlarkCore {
    fn name(&self) -> &'static str {
        "starlark"
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

/// The Starlark plugin's presentation half.
#[derive(Debug, Default)]
pub struct StarlarkPresentation;

impl PluginPresentation for StarlarkPresentation {
    fn name(&self) -> &'static str {
        "starlark"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "BZL",
            tint: 0x0043_a047,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: StarlarkView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!("Starlark, {}", view.kind)];
        if view.truncated {
            lines.push("Longer than this reads; what follows is the start.".to_owned());
        }
        if view.default_visibility.is_empty() {
            lines.push("No package default visibility.".to_owned());
        } else {
            lines.push(format!(
                "Package default visibility: {}",
                view.default_visibility.join(", ")
            ));
        }
        if !view.loads.is_empty() {
            lines.push("Loads:".to_owned());
            for load in &view.loads {
                lines.push(format!("  {} for {}", load.from, load.symbols.join(", ")));
            }
        }
        if !view.functions.is_empty() {
            lines.push(format!("Defines {}", view.functions.join(", ")));
        }
        if !view.rules.is_empty() {
            lines.push(format!("Declares the rule {}", view.rules.join(", ")));
        }
        if !view.targets.is_empty() {
            lines.push(format!("{} target(s):", view.targets.len()));
            for target in &view.targets {
                lines.push(format!("  {} {}", target.rule, target.name));
                if !target.globs.is_empty() {
                    lines.push(format!("      globs {}", target.globs.join(", ")));
                }
                if !target.dependencies.is_empty() {
                    lines.push(format!("      on {}", target.dependencies.join(", ")));
                }
                lines.push(match target.visibility.first() {
                    Some(_) if target.visibility.iter().any(|one| one.contains("public")) => {
                        "      public: anything in the repository may use it".to_owned()
                    }
                    Some(_) => format!("      visible to {}", target.visibility.join(", ")),
                    None => "      package default visibility".to_owned(),
                });
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{StarlarkCore, StarlarkPresentation, StarlarkView, looks_like_it, strings_in};
    use plugin_api::{PluginCore, PluginPresentation};

    fn sample(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/starlark")
            .join(name)
    }

    fn view_of(name: &str) -> StarlarkView {
        serde_json::from_value(StarlarkCore.view(&sample(name)).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&StarlarkCore),
            PluginPresentation::extensions(&StarlarkPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn recognises_a_build_file_by_what_is_in_it() {
        assert!(looks_like_it(
            "load(\"@rules_cc//cc:defs.bzl\", \"cc_library\")\n\ncc_library(\n    name = \"a\",\n    srcs = [\"a.cc\"],\n)\n"
        ));
        assert!(
            !looks_like_it("def main():\n    print('hello')\n"),
            "Python is not this, and it has no `load`"
        );
        assert!(!looks_like_it(""));
    }

    #[test]
    fn strings_come_out_in_order() {
        assert_eq!(strings_in("load(\"a\", \"b\", \"c\")"), vec!["a", "b", "c"]);
        assert!(strings_in("no strings here").is_empty());
    }

    #[test]
    fn reads_the_loads_and_what_each_brings_in() {
        let view = view_of("BUILD.bazel");

        assert_eq!(view.loads.len(), 3);
        assert_eq!(view.loads[0].from, "@rules_cc//cc:defs.bzl");
        assert_eq!(
            view.loads[0].symbols,
            vec!["cc_binary", "cc_library", "cc_test"]
        );
        assert_eq!(view.loads[2].from, "//tools:readings.bzl");
    }

    #[test]
    fn reads_every_target_with_the_rule_that_makes_it() {
        let view = view_of("BUILD.bazel");

        let named: Vec<(&str, &str)> = view
            .targets
            .iter()
            .map(|one| (one.rule.as_str(), one.name.as_str()))
            .collect();
        assert_eq!(
            named,
            vec![
                ("cc_library", "column"),
                ("cc_binary", "csvstats"),
                ("cc_test", "column_test"),
                ("py_binary", "summarise"),
                ("readings_fixture", "sample_readings"),
                ("filegroup", "docs"),
            ],
            "a rule loaded from the repository itself is still a rule"
        );
    }

    #[test]
    fn reads_the_dependencies_and_the_globs() {
        let view = view_of("BUILD.bazel");

        let column = &view.targets[0];
        assert_eq!(
            column.dependencies,
            vec!["@abseil-cpp//absl/status", "@abseil-cpp//absl/strings"]
        );
        assert!(
            column.globs.contains(&"src/column/*.cc".to_owned()),
            "{:?}",
            column.globs
        );
        assert!(
            column
                .globs
                .contains(&"include/csvstats/internal_*.h".to_owned()),
            "an excluded pattern is still a pattern the file names"
        );
    }

    #[test]
    fn an_attribute_belongs_to_the_call_it_is_written_in() {
        // A target with no `visibility` of its own must not pick up the
        // next target's. Before the call was bounded, the test target
        // reported the visibility of the `py_binary` below it - and so
        // reported a private test as public.
        let view = view_of("BUILD.bazel");

        let test = view
            .targets
            .iter()
            .find(|one| one.name == "column_test")
            .expect("the test target");
        let after = view
            .targets
            .iter()
            .find(|one| one.name == "summarise")
            .expect("the target below it");

        assert!(test.visibility.is_empty());
        assert_eq!(after.visibility, vec!["//visibility:public"]);
    }

    #[test]
    fn reads_visibility_three_ways() {
        let view = view_of("BUILD.bazel");

        assert_eq!(view.default_visibility, vec!["//visibility:private"]);
        assert_eq!(view.targets[0].visibility, vec!["//visibility:public"]);
        assert!(
            view.targets[2].visibility.is_empty(),
            "the test takes the package default"
        );
        assert_eq!(view.targets[5].visibility, vec!["//docs:__pkg__"]);
    }

    #[test]
    fn tells_an_extension_from_a_build_file() {
        let build = view_of("BUILD.bazel");
        let extension = view_of("readings.bzl");

        assert!(build.kind.starts_with("a build file"));
        assert!(extension.kind.starts_with("an extension"));
        assert_eq!(extension.functions, vec!["_readings_fixture_impl"]);
        assert_eq!(extension.rules, vec!["readings_fixture"]);
        assert_eq!(extension.loads.len(), 1);
        assert_eq!(extension.loads[0].symbols, vec!["paths"]);
    }

    #[test]
    fn presents_what_is_public_in_words() {
        let data = StarlarkCore.view(&sample("BUILD.bazel")).unwrap();

        let lines = StarlarkPresentation.present(&data);

        assert!(lines[0].starts_with("Starlark, a build file"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Package default visibility: //visibility:private"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("anything in the repository may use it"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("visible to //docs:__pkg__"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("package default visibility"))
        );
    }

    #[test]
    fn a_file_that_is_not_starlark_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.bzl");
        std::fs::write(&path, b"nothing of the sort at all").unwrap();

        assert!(StarlarkCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
