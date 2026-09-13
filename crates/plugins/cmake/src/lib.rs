//! `CMake` script file type plugin: core and presentation halves.
//!
//! A `CMake` script is a list of command calls, and what a reader wants
//! from one is what it produces and what it can be told to do
//! differently. The options are the second half of that: they are the
//! knobs, and their defaults are what happens if nobody touches them.

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
/// `CMakeLists.txt` is a `.txt`, which the text plugin owns and should
/// keep; this one is reached by what is in the file.
pub const EXTENSIONS: &[&str] = &["cmake"];

/// How much of a script is read.
const READ_CAP: usize = 1024 * 1024;

/// How many of each kind are listed before the rest are only counted.
const SHOWN: usize = 64;

/// One target the script adds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    /// Its name.
    pub name: String,
    /// What kind it is: an executable, a library or a custom target.
    pub kind: String,
    /// The sources it is built from.
    pub sources: Vec<String>,
}

/// One option the script declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Knob {
    /// Its name.
    pub name: String,
    /// What it is for.
    pub description: String,
    /// What it is unless somebody says otherwise.
    pub default: String,
}

/// View data produced by [`CmakeCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CmakeView {
    /// The oldest `CMake` that will read it.
    pub minimum_version: Option<String>,
    /// The project's name.
    pub project: Option<String>,
    /// Its version.
    pub version: Option<String>,
    /// What it says it is.
    pub description: Option<String>,
    /// The languages it is written in.
    pub languages: Vec<String>,
    /// What it builds.
    pub targets: Vec<Target>,
    /// The packages it goes looking for, and whether each is required.
    pub packages: Vec<String>,
    /// The knobs a person can turn at configure time.
    pub options: Vec<Knob>,
    /// The directories it descends into.
    pub subdirectories: Vec<String>,
    /// The tests it registers.
    pub tests: Vec<String>,
    /// Whether the script was longer than this reads.
    pub truncated: bool,
}

/// Whether `text` reads like a `CMake` script.
fn looks_like_it(text: &str) -> bool {
    let mut markers = 0usize;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        for command in [
            "cmake_minimum_required(",
            "project(",
            "add_executable(",
            "add_library(",
            "target_link_libraries(",
            "find_package(",
            "add_subdirectory(",
            "set(CMAKE_",
        ] {
            if trimmed.starts_with(command) {
                markers += 1;
            }
        }
        if markers >= 2 {
            return true;
        }
    }
    false
}

/// A command call joined onto one line, for the ones written across
/// several.
fn joined(lines: &[&str], from: usize) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for line in lines.iter().skip(from) {
        let text = strip_comment(line);
        out.push_str(text.trim());
        out.push(' ');
        depth += text.matches('(').count();
        depth = depth.saturating_sub(text.matches(')').count());
        if depth == 0 {
            break;
        }
    }
    out
}

/// A line without its trailing comment.
///
/// A `#` inside a quoted string is not a comment, and a URL holding one
/// is the case that matters.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    for (at, character) in line.char_indices() {
        match character {
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..at],
            _ => {}
        }
    }
    line
}

/// The arguments of the call whose text is `call`, split on whitespace
/// but keeping quoted runs whole.
fn arguments(call: &str) -> Vec<String> {
    let Some(open) = call.find('(') else {
        return Vec::new();
    };
    let inside = &call[open + 1..];
    let inside = &inside[..inside.rfind(')').unwrap_or(inside.len())];
    let mut found = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for character in inside.chars() {
        match character {
            '"' => quoted = !quoted,
            _ if character.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    found.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }
    if !current.is_empty() {
        found.push(current);
    }
    found
}

/// The value after `keyword` in a call's arguments.
fn after<'a>(arguments: &'a [String], keyword: &str) -> Option<&'a str> {
    let at = arguments.iter().position(|one| one == keyword)?;
    arguments.get(at + 1).map(String::as_str)
}

/// Every argument after `keyword`, to the end or the next keyword.
fn every_after(arguments: &[String], keyword: &str, stop: &[&str]) -> Vec<String> {
    let Some(at) = arguments.iter().position(|one| one == keyword) else {
        return Vec::new();
    };
    arguments[at + 1..]
        .iter()
        .take_while(|one| !stop.contains(&one.as_str()))
        .cloned()
        .collect()
}

/// The keywords `project()` understands, which end its language list.
const PROJECT_KEYWORDS: &[&str] = &["VERSION", "DESCRIPTION", "HOMEPAGE_URL", "LANGUAGES"];

/// Everything [`CmakeView`] holds, read from `source`.
fn parse(source: &str, truncated: bool) -> CmakeView {
    let lines: Vec<&str> = source.lines().collect();
    let mut view = CmakeView {
        minimum_version: None,
        project: None,
        version: None,
        description: None,
        languages: Vec::new(),
        targets: Vec::new(),
        packages: Vec::new(),
        options: Vec::new(),
        subdirectories: Vec::new(),
        tests: Vec::new(),
        truncated,
    };
    for (at, line) in lines.iter().enumerate() {
        let trimmed = strip_comment(line).trim_start();
        let Some(command) = trimmed.split('(').next().map(str::trim) else {
            continue;
        };
        if !trimmed.contains('(') {
            continue;
        }
        let call = joined(&lines, at);
        let given = arguments(&call);
        read_command(command, &given, &mut view);
    }
    view
}

/// Reads one command call into `view`.
fn read_command(command: &str, given: &[String], view: &mut CmakeView) {
    match command {
        "cmake_minimum_required" => {
            view.minimum_version = after(given, "VERSION").map(ToOwned::to_owned);
        }
        "project" => {
            view.project = given.first().cloned();
            view.version = after(given, "VERSION").map(ToOwned::to_owned);
            view.description = after(given, "DESCRIPTION").map(ToOwned::to_owned);
            view.languages = every_after(given, "LANGUAGES", PROJECT_KEYWORDS);
        }
        "option" => {
            if view.options.len() < SHOWN
                && let Some(name) = given.first()
            {
                view.options.push(Knob {
                    name: name.clone(),
                    description: given.get(1).cloned().unwrap_or_default(),
                    default: given.get(2).cloned().unwrap_or_else(|| "OFF".to_owned()),
                });
            }
        }
        "find_package" => {
            if view.packages.len() < SHOWN
                && let Some(name) = given.first()
            {
                let required = given.iter().any(|one| one == "REQUIRED");
                let version = given.get(1).filter(|one| {
                    one.chars()
                        .next()
                        .is_some_and(|first| first.is_ascii_digit())
                });
                let mut said = name.clone();
                if let Some(version) = version {
                    said.push(' ');
                    said.push_str(version);
                }
                said.push_str(if required {
                    " (required)"
                } else {
                    " (optional)"
                });
                view.packages.push(said);
            }
        }
        "add_executable" | "add_library" | "add_custom_target" => {
            if view.targets.len() < SHOWN
                && let Some(name) = given.first()
            {
                // A library's second argument may be its linkage rather
                // than a source, and neither is a file.
                let sources = given[1..]
                    .iter()
                    .filter(|one| one.contains('.') || one.contains('/'))
                    .cloned()
                    .collect();
                view.targets.push(Target {
                    name: name.clone(),
                    kind: match command {
                        "add_executable" => "executable".to_owned(),
                        "add_custom_target" => "custom".to_owned(),
                        _ => given
                            .get(1)
                            .filter(|one| {
                                ["STATIC", "SHARED", "MODULE", "INTERFACE", "OBJECT"]
                                    .contains(&one.as_str())
                            })
                            .map_or_else(
                                || "library".to_owned(),
                                |linkage| format!("{} library", linkage.to_lowercase()),
                            ),
                    },
                    sources,
                });
            }
        }
        "add_subdirectory" => {
            if view.subdirectories.len() < SHOWN
                && let Some(name) = given.first()
            {
                view.subdirectories.push(name.clone());
            }
        }
        // `add_test(NAME x COMMAND y)` registers x; the older
        // `add_test(x y)` registers its first argument.
        "add_test" if view.tests.len() < SHOWN => {
            if let Some(name) = after(given, "NAME") {
                view.tests.push(name.to_owned());
            } else if let Some(name) = given.first() {
                view.tests.push(name.clone());
            }
        }
        _ => {}
    }
}

/// Everything [`CmakeView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<CmakeView> {
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
            "not a CMake script",
        ));
    }
    Ok(parse(source, truncated))
}

/// The `CMake` plugin's core half.
#[derive(Debug, Default)]
pub struct CmakeCore;

impl PluginCore for CmakeCore {
    fn name(&self) -> &'static str {
        "cmake"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A script is text, and the one everybody has is called
        // `CMakeLists.txt` - so the `txt` extension would hand it to
        // the text plugin however early this one is registered. Saying
        // it is the narrower reading is what settles that (D13).
        &["text"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The `CMake` plugin's presentation half.
#[derive(Debug, Default)]
pub struct CmakePresentation;

impl PluginPresentation for CmakePresentation {
    fn name(&self) -> &'static str {
        "cmake"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "CMK",
            tint: 0x0006_4f8b,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: CmakeView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![match (&view.project, &view.version) {
            (Some(name), Some(version)) => format!("CMake project {name} {version}"),
            (Some(name), None) => format!("CMake project {name}"),
            _ => "CMake script, with no project of its own".to_owned(),
        }];
        if let Some(description) = &view.description {
            lines.push(description.clone());
        }
        if let Some(minimum) = &view.minimum_version {
            lines.push(format!("Needs CMake {minimum} or newer"));
        }
        if !view.languages.is_empty() {
            lines.push(format!("Languages: {}", view.languages.join(", ")));
        }
        if view.options.is_empty() {
            lines.push("No options: it builds one way.".to_owned());
        } else {
            lines.push("Options, and what happens if nobody sets them:".to_owned());
            for knob in &view.options {
                lines.push(format!("  {} = {}", knob.name, knob.default));
                if !knob.description.is_empty() {
                    lines.push(format!("      {}", knob.description));
                }
            }
        }
        if !view.packages.is_empty() {
            lines.push("Looks for:".to_owned());
            for package in &view.packages {
                lines.push(format!("  {package}"));
            }
        }
        if !view.targets.is_empty() {
            lines.push("Builds:".to_owned());
            for target in &view.targets {
                lines.push(format!("  {} {}", target.kind, target.name));
                if !target.sources.is_empty() {
                    lines.push(format!("      from {}", target.sources.join(", ")));
                }
            }
        }
        if !view.tests.is_empty() {
            lines.push(format!("Tests: {}", view.tests.join(", ")));
        }
        if !view.subdirectories.is_empty() {
            lines.push(format!("Descends into {}", view.subdirectories.join(", ")));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{CmakeCore, CmakePresentation, CmakeView, arguments, looks_like_it, strip_comment};
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/cmake/CMakeLists.txt")
    }

    fn view_of() -> CmakeView {
        serde_json::from_value(CmakeCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&CmakeCore),
            PluginPresentation::extensions(&CmakePresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn it_says_it_specialises_the_text_reading() {
        // Without this the fixture went to `text`: it is called
        // `CMakeLists.txt`, and the extension hint hands a `.txt` to
        // whoever claims that.
        assert_eq!(CmakeCore.specialises(), &["text"]);
    }

    #[test]
    fn recognises_the_commands_rather_than_the_file_name() {
        assert!(looks_like_it(
            "cmake_minimum_required(VERSION 3.25)\nproject(a)\n"
        ));
        assert!(!looks_like_it("project management notes\n"));
        assert!(!looks_like_it(""));
    }

    #[test]
    fn a_hash_inside_a_quoted_string_is_not_a_comment() {
        assert_eq!(strip_comment("set(A 1) # why"), "set(A 1) ");
        assert_eq!(
            strip_comment(r#"set(URL "https://example.com/#anchor")"#),
            r#"set(URL "https://example.com/#anchor")"#
        );
    }

    #[test]
    fn arguments_keep_a_quoted_run_whole() {
        assert_eq!(
            arguments(r#"option(A "Build the test suite" ON)"#),
            vec!["A", "Build the test suite", "ON"]
        );
        assert!(arguments("no brackets").is_empty());
    }

    #[test]
    fn reads_the_project_declaration() {
        let view = view_of();

        assert_eq!(view.minimum_version.as_deref(), Some("3.25"));
        assert_eq!(view.project.as_deref(), Some("csvstats"));
        assert_eq!(view.version.as_deref(), Some("1.0.3"));
        assert_eq!(
            view.description.as_deref(),
            Some("Summary statistics for a column of readings")
        );
        assert_eq!(
            view.languages,
            vec!["CXX", "C"],
            "the language list ends at the next keyword, or at the bracket"
        );
    }

    #[test]
    fn reads_the_options_with_their_defaults() {
        let view = view_of();

        assert_eq!(view.options.len(), 3);
        assert_eq!(view.options[0].name, "CSVSTATS_BUILD_TESTS");
        assert_eq!(view.options[0].description, "Build the test suite");
        assert_eq!(view.options[0].default, "ON");
        assert_eq!(view.options[1].default, "OFF");
    }

    #[test]
    fn reads_the_packages_and_whether_each_is_required() {
        let view = view_of();

        assert_eq!(view.packages.len(), 3);
        assert_eq!(view.packages[0], "Threads (required)");
        assert_eq!(
            view.packages[1], "fmt 10.2 (required)",
            "a version comes second, and only when it is one"
        );
        assert_eq!(view.packages[2], "ZLIB (required)");
    }

    #[test]
    fn reads_the_targets_with_their_kinds_and_sources() {
        let view = view_of();

        let library = &view.targets[0];
        assert_eq!(library.name, "csvstats_core");
        assert_eq!(library.kind, "static library");
        assert_eq!(
            library.sources,
            vec!["src/column.cc", "src/reader.cc", "src/summary.cc"],
            "STATIC is a linkage, not a source"
        );

        let executable = view
            .targets
            .iter()
            .find(|one| one.name == "csvstats")
            .expect("the executable");
        assert_eq!(executable.kind, "executable");
        assert_eq!(executable.sources, vec!["src/main.cc"]);
    }

    #[test]
    fn reads_the_tests_by_the_name_they_are_registered_under() {
        let view = view_of();

        assert_eq!(
            view.tests,
            vec!["column", "summary"],
            "`add_test(NAME x COMMAND y)` registers x, not y"
        );
    }

    #[test]
    fn reads_the_subdirectories_it_descends_into() {
        let view = view_of();

        assert_eq!(view.subdirectories, vec!["tools", "docs", "test"]);
    }

    #[test]
    fn presents_the_knobs_and_what_they_do_by_default() {
        let data = CmakeCore.view(&fixture()).unwrap();

        let lines = CmakePresentation.present(&data);

        assert!(lines[0].starts_with("CMake project csvstats 1.0.3"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("what happens if nobody sets them"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("CSVSTATS_WITH_ZLIB = OFF"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("static library csvstats_core"))
        );
    }

    #[test]
    fn a_file_that_is_not_cmake_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.cmake");
        std::fs::write(&path, b"nothing of the sort at all").unwrap();

        assert!(CmakeCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
