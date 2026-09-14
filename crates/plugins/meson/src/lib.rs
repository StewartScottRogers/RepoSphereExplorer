//! Meson build definition file type plugin: core and presentation
//! halves.
//!
//! Meson says the same things `CMake` does in a different shape: keyword
//! arguments rather than positional ones, `dependency()` rather than
//! `find_package()`, and `get_option()` reading a knob that was
//! declared somewhere else. A reader wants the same three answers -
//! what it builds, what it needs, and what it can be told to do
//! differently - so those are what this reads.

use plugin_api::{Icon, PluginCore, PluginPresentation, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;
use syntax::{Language, Quote};

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
///
/// A build definition is named `meson.build`, whose extension `build`
/// nothing else here claims; `meson.options` is the file declaring the
/// knobs this one reads.
pub const EXTENSIONS: &[&str] = &["build", "options"];

/// How much of a definition is read.
const READ_CAP: usize = 1024 * 1024;

/// How many of each kind are listed before the rest are only counted.
const SHOWN: usize = 64;

/// One thing the definition builds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    /// Its name.
    pub name: String,
    /// What kind it is.
    pub kind: String,
    /// The sources it is built from.
    pub sources: Vec<String>,
}

/// One dependency it looks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    /// What it is called.
    pub name: String,
    /// The versions that will do, when it says.
    pub version: Option<String>,
    /// Whether the build stops without it. A dependency gated on an
    /// option is required only when that option is on, and the pane
    /// says which.
    pub required: String,
}

/// View data produced by [`MesonCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MesonView {
    /// The project's name.
    pub project: Option<String>,
    /// Its version.
    pub version: Option<String>,
    /// Its licence.
    pub licence: Option<String>,
    /// The languages it is written in.
    pub languages: Vec<String>,
    /// The oldest Meson that will read it.
    pub minimum_version: Option<String>,
    /// The defaults it sets for the built-in options.
    pub default_options: Vec<String>,
    /// What it builds.
    pub targets: Vec<Target>,
    /// What it looks for.
    pub dependencies: Vec<Dependency>,
    /// The knobs it reads, which are declared elsewhere.
    pub options_read: Vec<String>,
    /// The directories it enters.
    pub subdirectories: Vec<String>,
    /// The tests it registers.
    pub tests: Vec<String>,
    /// Whether the definition was longer than this reads.
    pub truncated: bool,
}

/// Whether `text` reads like a Meson build definition.
///
/// `project(` on its own is not enough - `CMake` opens the same way - so
/// a Meson-only call is asked for alongside it.
fn looks_like_it(text: &str) -> bool {
    let mut project = false;
    let mut meson_only = 0usize;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with("project(") {
            project = true;
        }
        for call in [
            "dependency(",
            "get_option(",
            "declare_dependency(",
            "include_directories(",
            "install_headers(",
            "subdir(",
            "meson.",
        ] {
            if trimmed.contains(call) {
                meson_only += 1;
            }
        }
        if project && meson_only >= 1 {
            return true;
        }
        if meson_only >= 3 {
            return true;
        }
    }
    false
}

/// A call joined onto one line, for the ones written across several.
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

/// A line without its trailing comment, leaving a `#` inside a quoted
/// string alone.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    for (at, character) in line.char_indices() {
        match character {
            '\'' => quoted = !quoted,
            '#' if !quoted => return &line[..at],
            _ => {}
        }
    }
    line
}

/// The quoted strings in `text`, in order. Meson quotes with single
/// quotes.
fn strings_in(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('\'') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('\'') else { break };
        found.push(after[..close].to_owned());
        rest = &after[close + 1..];
    }
    found
}

/// The value of the keyword argument `name`, as written.
fn keyword<'a>(call: &'a str, name: &str) -> Option<&'a str> {
    let at = call
        .find(&format!("{name} :"))
        .or_else(|| call.find(&format!("{name}:")))?;
    let rest = &call[at..];
    let after = rest.find(':')? + 1;
    let rest = &rest[after..];
    // The value ends at the comma that separates it from the next
    // argument, and a comma inside brackets is not that comma.
    let mut depth = 0usize;
    for (offset, character) in rest.char_indices() {
        match character {
            '[' | '(' => depth += 1,
            ']' | ')' if depth > 0 => depth -= 1,
            ']' | ')' => return Some(rest[..offset].trim()),
            ',' if depth == 0 => return Some(rest[..offset].trim()),
            _ => {}
        }
    }
    Some(rest.trim())
}

/// The positional arguments of a call: the quoted strings before the
/// first keyword argument.
fn positional(call: &str) -> Vec<String> {
    let Some(open) = call.find('(') else {
        return Vec::new();
    };
    let inside = &call[open + 1..];
    let inside = &inside[..inside.rfind(')').unwrap_or(inside.len())];
    let mut found = Vec::new();
    for piece in split_arguments(inside) {
        if piece.contains(':') && !piece.starts_with('\'') {
            break;
        }
        found.extend(strings_in(&piece));
    }
    found
}

/// A call's arguments, split on the commas between them rather than
/// every comma.
fn split_arguments(inside: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut depth = 0usize;
    let mut quoted = false;
    let mut current = String::new();
    for character in inside.chars() {
        match character {
            '\'' => quoted = !quoted,
            '[' | '(' if !quoted => depth += 1,
            ']' | ')' if !quoted => depth = depth.saturating_sub(1),
            ',' if depth == 0 && !quoted => {
                found.push(std::mem::take(&mut current).trim().to_owned());
                continue;
            }
            _ => {}
        }
        current.push(character);
    }
    if !current.trim().is_empty() {
        found.push(current.trim().to_owned());
    }
    found
}

/// Everything [`MesonView`] holds, read from `source`.
fn parse(source: &str, truncated: bool) -> MesonView {
    let lines: Vec<&str> = source.lines().collect();
    let mut view = MesonView {
        project: None,
        version: None,
        licence: None,
        languages: Vec::new(),
        minimum_version: None,
        default_options: Vec::new(),
        targets: Vec::new(),
        dependencies: Vec::new(),
        options_read: Vec::new(),
        subdirectories: Vec::new(),
        tests: Vec::new(),
        truncated,
    };
    for (at, line) in lines.iter().enumerate() {
        let trimmed = strip_comment(line).trim_start();
        if trimmed.is_empty() {
            continue;
        }
        // A target is often assigned: `lib = library('a', ...)`.
        let (assigned, rest) = match trimmed.split_once(" = ") {
            Some((name, rest)) => (Some(name.trim()), rest.trim()),
            None => (None, trimmed),
        };
        let Some(command) = rest.split('(').next().map(str::trim) else {
            continue;
        };
        if !rest.contains('(') {
            continue;
        }
        let call = joined(&lines, at);
        // The joined call still carries whatever preceded it on the
        // line, which the command name has already accounted for.
        read_call(command, &call, assigned, &mut view);
    }
    view
}

/// Reads one call into `view`.
fn read_call(command: &str, call: &str, assigned: Option<&str>, view: &mut MesonView) {
    match command {
        "project" => {
            let named = positional(call);
            view.project = named.first().cloned();
            view.languages = named[1.min(named.len())..].to_vec();
            view.version = keyword(call, "version").map(unquote);
            view.licence = keyword(call, "license").map(unquote);
            view.minimum_version = keyword(call, "meson_version").map(unquote);
            view.default_options = keyword(call, "default_options")
                .map(strings_in)
                .unwrap_or_default();
        }
        "executable" | "library" | "shared_library" | "static_library" | "both_libraries"
        | "jar" => {
            let named = positional(call);
            let Some(name) = named.first().cloned() else {
                return;
            };
            if view.targets.len() < SHOWN {
                view.targets.push(Target {
                    name,
                    kind: command.replace('_', " "),
                    sources: named[1.min(named.len())..].to_vec(),
                });
            }
        }
        "dependency" => {
            let Some(name) = positional(call).first().cloned() else {
                return;
            };
            if view.dependencies.len() < SHOWN {
                let required = keyword(call, "required").unwrap_or("true");
                view.dependencies.push(Dependency {
                    name,
                    version: keyword(call, "version").map(unquote),
                    required: match required {
                        "true" => "required".to_owned(),
                        "false" => "optional".to_owned(),
                        // Anything else is a variable, which means the
                        // dependency is required only when it is true.
                        other => format!("required when {other}"),
                    },
                });
            }
        }
        "subdir" => {
            if let Some(name) = positional(call).first()
                && view.subdirectories.len() < SHOWN
            {
                view.subdirectories.push(name.clone());
            }
        }
        "test" => {
            if let Some(name) = positional(call).first()
                && view.tests.len() < SHOWN
            {
                view.tests.push(name.clone());
            }
        }
        "get_option" => {
            if let Some(name) = positional(call).first() {
                let said = match assigned {
                    Some(into) => format!("{name} into {into}"),
                    None => name.clone(),
                };
                if view.options_read.len() < SHOWN && !view.options_read.contains(&said) {
                    view.options_read.push(said);
                }
            }
        }
        _ => {}
    }
}

/// A value without the quotes round it.
fn unquote(value: &str) -> String {
    value.trim().trim_matches('\'').to_owned()
}

/// Everything [`MesonView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<MesonView> {
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
            "not a Meson build definition",
        ));
    }
    Ok(parse(source, truncated))
}

/// The Meson plugin's core half.
#[derive(Debug, Default)]
pub struct MesonCore;

/// How this language is coloured, for the shared tokeniser. GUIDANCE.md
/// §3.6: the plugin describes its own format, the pane paints what it is
/// told.
const MESON: Language = Language {
    line_comment: &["#"],
    block_comment: &[],
    quotes: &[Quote::simple('"'), Quote::simple('\'')],
    keywords: &[
        "and",
        "break",
        "continue",
        "elif",
        "else",
        "endforeach",
        "endif",
        "false",
        "foreach",
        "if",
        "in",
        "not",
        "or",
        "true",
    ],
    types: &[],
    calls: true,
    ignore_case: false,
};

impl PluginCore for MesonCore {
    fn name(&self) -> &'static str {
        "meson"
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

/// The Meson plugin's presentation half.
#[derive(Debug, Default)]
pub struct MesonPresentation;

impl PluginPresentation for MesonPresentation {
    fn classify(&self, text: &str) -> Vec<Span> {
        syntax::classify(text, &MESON)
    }

    fn name(&self) -> &'static str {
        "meson"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "MSN",
            tint: 0x0040_4a7c,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: MesonView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![match (&view.project, &view.version) {
            (Some(name), Some(version)) => format!("Meson project {name} {version}"),
            (Some(name), None) => format!("Meson project {name}"),
            _ => "Meson build definition, with no project of its own".to_owned(),
        }];
        if let Some(licence) = &view.licence {
            lines.push(format!("Licence {licence}"));
        }
        if !view.languages.is_empty() {
            lines.push(format!("Languages: {}", view.languages.join(", ")));
        }
        if let Some(minimum) = &view.minimum_version {
            lines.push(format!("Needs Meson {minimum}"));
        }
        if !view.default_options.is_empty() {
            lines.push(format!("Defaults {}", view.default_options.join(", ")));
        }
        if !view.dependencies.is_empty() {
            lines.push("Looks for:".to_owned());
            for dependency in &view.dependencies {
                let version = dependency
                    .version
                    .as_deref()
                    .map_or_else(String::new, |version| format!(" {version}"));
                lines.push(format!(
                    "  {}{version} - {}",
                    dependency.name, dependency.required
                ));
            }
        }
        if !view.options_read.is_empty() {
            lines.push(format!(
                "Reads the options {}",
                view.options_read.join(", ")
            ));
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
            lines.push(format!("Enters {}", view.subdirectories.join(", ")));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{MesonCore, MesonPresentation, MesonView, keyword, looks_like_it, positional};
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/meson/meson.build")
    }

    fn view_of() -> MesonView {
        serde_json::from_value(MesonCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&MesonCore),
            PluginPresentation::extensions(&MesonPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn a_project_call_alone_is_not_enough() {
        assert!(looks_like_it(
            "project('a', 'cpp')\nfmt = dependency('fmt')\n"
        ));
        assert!(
            !looks_like_it("cmake_minimum_required(VERSION 3.25)\nproject(a)\n"),
            "CMake opens the same way, and this is not it"
        );
        assert!(!looks_like_it(""));
    }

    #[test]
    fn a_keyword_value_ends_at_the_comma_between_arguments() {
        assert_eq!(
            keyword("f(a, version : '1.2', b : 3)", "version"),
            Some("'1.2'")
        );
        assert_eq!(
            keyword(
                "f(default_options : ['a=1', 'b=2'], x : 1)",
                "default_options"
            ),
            Some("['a=1', 'b=2']"),
            "a comma inside brackets is not the one that ends it"
        );
        assert_eq!(keyword("f(a : 1)", "missing"), None);
    }

    #[test]
    fn positional_arguments_stop_at_the_first_keyword() {
        assert_eq!(
            positional("library('a', 'x.cc', 'y.cc', install : true)"),
            vec!["a", "x.cc", "y.cc"]
        );
    }

    #[test]
    fn reads_the_project_declaration() {
        let view = view_of();

        assert_eq!(view.project.as_deref(), Some("csvstats"));
        assert_eq!(view.version.as_deref(), Some("1.0.3"));
        assert_eq!(view.licence.as_deref(), Some("MIT"));
        assert_eq!(view.languages, vec!["cpp", "c"]);
        assert_eq!(view.minimum_version.as_deref(), Some(">=1.3.0"));
        assert_eq!(
            view.default_options,
            vec!["cpp_std=c++20", "warning_level=3", "werror=false"]
        );
    }

    #[test]
    fn says_which_dependency_is_gated_on_an_option() {
        let view = view_of();

        assert_eq!(view.dependencies.len(), 3);
        assert_eq!(view.dependencies[0].name, "threads");
        assert_eq!(view.dependencies[0].required, "required");
        assert_eq!(view.dependencies[1].name, "fmt");
        assert_eq!(view.dependencies[1].version.as_deref(), Some(">=10.2"));
        assert_eq!(
            view.dependencies[2].required, "required when with_zlib",
            "a dependency required only when an option says so is not simply required"
        );
    }

    #[test]
    fn reads_the_targets_with_their_sources() {
        let view = view_of();

        let library = view
            .targets
            .iter()
            .find(|one| one.name == "csvstats")
            .expect("the library or the executable");
        assert!(
            view.targets.iter().any(|one| one.kind == "library"),
            "{:?}",
            view.targets
        );
        assert!(view.targets.iter().any(|one| one.kind == "executable"));
        assert!(!library.sources.is_empty());
        assert!(
            view.targets
                .iter()
                .any(|one| one.sources.contains(&"src/column.cc".to_owned()))
        );
    }

    #[test]
    fn reads_the_options_it_reads_and_where_they_land() {
        let view = view_of();

        assert!(
            view.options_read
                .iter()
                .any(|one| one == "tests into build_tests"),
            "{:?}",
            view.options_read
        );
        assert!(
            view.options_read
                .iter()
                .any(|one| one == "zlib into with_zlib")
        );
        assert!(
            view.options_read
                .iter()
                .any(|one| one == "prefix into prefix")
        );
    }

    #[test]
    fn reads_the_tests_and_the_subdirectories() {
        let view = view_of();

        assert_eq!(view.tests, vec!["column", "summary"]);
        assert_eq!(view.subdirectories, vec!["tools", "docs"]);
    }

    #[test]
    fn presents_the_dependency_that_is_conditional() {
        let data = MesonCore.view(&fixture()).unwrap();

        let lines = MesonPresentation.present(&data);

        assert!(lines[0].starts_with("Meson project csvstats 1.0.3"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("fmt >=10.2 - required"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("zlib - required when with_zlib"))
        );
    }

    #[test]
    fn a_file_that_is_not_meson_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really-meson.build");
        std::fs::write(&path, b"nothing of the sort at all").unwrap();

        assert!(MesonCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
