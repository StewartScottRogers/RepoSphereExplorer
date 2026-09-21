//! Go module file (`go.mod`) file type plugin: core and presentation halves.
//!
//! A `go.mod` names the module a checkout publishes as, pins the Go
//! version and toolchain it builds with, and lists what it depends on. A
//! `replace` directive pointing at a sibling path or a fork is how a
//! reader finds out that a checkout is one leg of a larger repository, or
//! is running on a patched dependency - a fact available nowhere else.

use plugin_api::{Icon, PluginCore, PluginPresentation, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;
use syntax::Language;

/// The lowercase extensions this type claims, without their dot. `go.mod`
/// has no format-specific extension of its own the way `go.sum` does, but
/// `Path::extension()` of the literal filename `go.mod` is `mod`, and
/// nothing else in this catalogue claims it.
pub const EXTENSIONS: &[&str] = &["mod"];

/// How much of a module file is read. Real ones are a few dozen lines;
/// this is generous headroom rather than an expectation of reaching it.
const READ_CAP: usize = 1024 * 1024;

/// One `require`d module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirement {
    /// The module path.
    pub module: String,
    /// The version required.
    pub version: String,
    /// Whether it is marked `// indirect`: needed by a dependency, not by
    /// this module's own code.
    pub indirect: bool,
}

/// One `replace` directive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replace {
    /// The module path being replaced.
    pub from: String,
    /// The version being replaced, when the directive names one.
    pub from_version: Option<String>,
    /// What it is replaced with: a filesystem path, or another module.
    pub to: String,
    /// The replacement's version, absent for a path replacement.
    pub to_version: Option<String>,
}

/// One `exclude`d module version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exclude {
    /// The module path.
    pub module: String,
    /// The version excluded from the version graph.
    pub version: String,
}

/// One `retract`ed version or version range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retract {
    /// The version, or the low end of a `[low, high]` range.
    pub low: String,
    /// The high end of a range, absent for a single retracted version.
    pub high: Option<String>,
    /// The trailing comment explaining why, when there is one.
    pub reason: Option<String>,
}

/// One `godebug` setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Godebug {
    /// The setting's name.
    pub key: String,
    /// The value it is pinned to.
    pub value: String,
}

/// View data produced by [`GomodCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GomodView {
    /// The module path the repository publishes as.
    pub module: Option<String>,
    /// The Go version from the `go` directive.
    pub go_version: Option<String>,
    /// The toolchain from the `toolchain` directive.
    pub toolchain: Option<String>,
    /// Every `require`d module, direct and indirect alike.
    pub requirements: Vec<Requirement>,
    /// Every `replace` directive.
    pub replacements: Vec<Replace>,
    /// Every `exclude`d module version.
    pub excludes: Vec<Exclude>,
    /// Every `retract`ed version or range.
    pub retracts: Vec<Retract>,
    /// Every `godebug` setting.
    pub godebug: Vec<Godebug>,
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the file was longer than this reads.
    pub truncated: bool,
}

/// Splits a trailing `// comment` off `line`, returning the code with
/// surrounding whitespace trimmed and the comment text, when there is one.
fn split_comment(line: &str) -> (&str, Option<&str>) {
    match line.find("//") {
        Some(at) => (line[..at].trim(), Some(line[at + 2..].trim())),
        None => (line.trim(), None),
    }
}

/// The directive keywords a top-level line may start with, each followed
/// by a space or introducing a parenthesised block.
const DIRECTIVES: &[&str] = &["require", "exclude", "replace", "retract", "godebug"];

/// Whether `text` reads like a Go module file: a `module` directive, and
/// nothing but recognised directives and block contents around it.
///
/// This also catches a file cut off mid-directive: a block left open at
/// the end of the text fails it, the same as any other line this format
/// does not recognise.
fn looks_like_it(text: &str) -> bool {
    let mut in_block = false;
    let mut has_module = false;
    for raw in text.lines() {
        let (code, _comment) = split_comment(raw);
        if code.is_empty() {
            continue;
        }
        if in_block {
            if code == ")" {
                in_block = false;
            }
            continue;
        }
        match code.split_once(' ') {
            Some(("module", rest)) if !rest.trim().is_empty() => has_module = true,
            Some(("go" | "toolchain", _)) => {}
            Some((directive, "(")) if DIRECTIVES.contains(&directive) => in_block = true,
            Some((directive, _)) if DIRECTIVES.contains(&directive) => {}
            _ => return false,
        }
    }
    has_module && !in_block
}

/// Parses one `require`/`exclude`/`replace`/`retract`/`godebug` block or
/// single-line entry into `view`.
fn record_entry(directive: &str, code: &str, comment: Option<&str>, view: &mut GomodView) {
    match directive {
        "require" => {
            let mut parts = code.split_whitespace();
            let (Some(module), Some(version)) = (parts.next(), parts.next()) else {
                return;
            };
            view.requirements.push(Requirement {
                module: module.to_owned(),
                version: version.to_owned(),
                indirect: comment.is_some_and(|text| text == "indirect"),
            });
        }
        "exclude" => {
            let mut parts = code.split_whitespace();
            let (Some(module), Some(version)) = (parts.next(), parts.next()) else {
                return;
            };
            view.excludes.push(Exclude {
                module: module.to_owned(),
                version: version.to_owned(),
            });
        }
        "replace" => {
            let Some((left, right)) = code.split_once("=>") else {
                return;
            };
            let mut from_parts = left.split_whitespace();
            let Some(from) = from_parts.next() else {
                return;
            };
            let mut to_parts = right.split_whitespace();
            let Some(to) = to_parts.next() else {
                return;
            };
            view.replacements.push(Replace {
                from: from.to_owned(),
                from_version: from_parts.next().map(str::to_owned),
                to: to.to_owned(),
                to_version: to_parts.next().map(str::to_owned),
            });
        }
        "retract" => {
            let reason = comment.map(str::to_owned);
            if let Some(inner) = code
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            {
                let mut bounds = inner.split(',').map(str::trim);
                let Some(low) = bounds.next() else {
                    return;
                };
                view.retracts.push(Retract {
                    low: low.to_owned(),
                    high: bounds.next().map(str::to_owned),
                    reason,
                });
            } else if !code.is_empty() {
                view.retracts.push(Retract {
                    low: code.to_owned(),
                    high: None,
                    reason,
                });
            }
        }
        "godebug" => {
            if let Some((key, value)) = code.split_once('=') {
                view.godebug.push(Godebug {
                    key: key.trim().to_owned(),
                    value: value.trim().to_owned(),
                });
            }
        }
        _ => {}
    }
}

/// Everything [`GomodView`] holds, read from `text`.
fn parse(text: &str) -> GomodView {
    let mut view = GomodView {
        module: None,
        go_version: None,
        toolchain: None,
        requirements: Vec::new(),
        replacements: Vec::new(),
        excludes: Vec::new(),
        retracts: Vec::new(),
        godebug: Vec::new(),
        content: String::new(),
        truncated: false,
    };

    let mut block: Option<&str> = None;
    for raw in text.lines() {
        let (code, comment) = split_comment(raw);
        if code.is_empty() {
            continue;
        }
        if let Some(directive) = block {
            if code == ")" {
                block = None;
            } else {
                record_entry(directive, code, comment, &mut view);
            }
            continue;
        }
        match code.split_once(' ') {
            Some(("module", rest)) => view.module = Some(rest.trim().to_owned()),
            Some(("go", rest)) => view.go_version = Some(rest.trim().to_owned()),
            Some(("toolchain", rest)) => view.toolchain = Some(rest.trim().to_owned()),
            Some((directive, "(")) if DIRECTIVES.contains(&directive) => block = Some(directive),
            Some((directive, rest)) if DIRECTIVES.contains(&directive) => {
                record_entry(directive, rest.trim(), comment, &mut view);
            }
            _ => {}
        }
    }
    view
}

/// Everything [`GomodView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<GomodView> {
    let bytes = std::fs::read(path)?;
    let truncated = bytes.len() > READ_CAP;
    let slice = &bytes[..bytes.len().min(READ_CAP)];
    let content = String::from_utf8_lossy(slice).into_owned();
    if !looks_like_it(&content) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a Go module file",
        ));
    }
    let mut view = parse(&content);
    view.content = content;
    view.truncated = truncated;
    Ok(view)
}

/// The Go module file plugin's core half.
#[derive(Debug, Default)]
pub struct GomodCore;

/// How this format is coloured, for the shared tokeniser. GUIDANCE.md §3.6:
/// the plugin describes its own format, the pane paints what it is told.
const GOMOD: Language = Language {
    line_comment: &["//"],
    block_comment: &[],
    quotes: &[],
    keywords: &[
        "module",
        "go",
        "toolchain",
        "require",
        "exclude",
        "replace",
        "retract",
        "godebug",
        "indirect",
    ],
    types: &[],
    calls: false,
    ignore_case: false,
};

impl PluginCore for GomodCore {
    fn name(&self) -> &'static str {
        "gomod"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // It is text, and the text plugin recognises any of it. This is
        // the narrower reading of the same bytes (D13).
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

/// The Go module file plugin's presentation half.
#[derive(Debug, Default)]
pub struct GomodPresentation;

impl PluginPresentation for GomodPresentation {
    fn classify(&self, text: &str) -> Vec<Span> {
        syntax::classify(text, &GOMOD)
    }

    fn name(&self) -> &'static str {
        "gomod"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "MOD",
            tint: 0x0000_add8,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: GomodView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "Go module: {}",
            view.module.as_deref().unwrap_or("(none)")
        )];
        if let Some(go_version) = &view.go_version {
            lines.push(format!("Go version: {go_version}"));
        }
        if let Some(toolchain) = &view.toolchain {
            lines.push(format!("Toolchain: {toolchain}"));
        }
        if view.truncated {
            lines.push("Longer than this reads; what follows is the start.".to_owned());
        }

        if !view.requirements.is_empty() {
            lines.push("Requirements:".to_owned());
            for requirement in &view.requirements {
                let mark = if requirement.indirect {
                    "  (indirect)"
                } else {
                    ""
                };
                lines.push(format!(
                    "  {} {}{mark}",
                    requirement.module, requirement.version
                ));
            }
        }
        if !view.replacements.is_empty() {
            lines.push("Replaced:".to_owned());
            for replace in &view.replacements {
                let to_version = replace
                    .to_version
                    .as_deref()
                    .map(|version| format!(" {version}"))
                    .unwrap_or_default();
                lines.push(format!("  {} => {}{to_version}", replace.from, replace.to));
            }
        }
        if !view.excludes.is_empty() {
            lines.push("Excluded:".to_owned());
            for exclude in &view.excludes {
                lines.push(format!("  {} {}", exclude.module, exclude.version));
            }
        }
        if !view.retracts.is_empty() {
            lines.push("Retracted:".to_owned());
            for retract in &view.retracts {
                let range = match &retract.high {
                    Some(high) => format!("[{}, {high}]", retract.low),
                    None => retract.low.clone(),
                };
                let reason = retract
                    .reason
                    .as_deref()
                    .map(|reason| format!("  ({reason})"))
                    .unwrap_or_default();
                lines.push(format!("  {range}{reason}"));
            }
        }
        if !view.godebug.is_empty() {
            lines.push("godebug settings:".to_owned());
            for setting in &view.godebug {
                lines.push(format!("  {}={}", setting.key, setting.value));
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{GomodCore, GomodPresentation, GomodView, looks_like_it};
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/gomod/go.mod")
    }

    fn view_of() -> GomodView {
        serde_json::from_value(GomodCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&GomodCore),
            PluginPresentation::extensions(&GomodPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn every_line_has_to_be_one_of_these() {
        assert!(looks_like_it("module example.com/a\n\ngo 1.22\n"));
        assert!(
            !looks_like_it("module example.com/a\n\ndef greet():\n    return 1\n"),
            "a line that is not a go.mod directive means it is not this file"
        );
        assert!(
            !looks_like_it("go 1.22\n"),
            "no module directive is not enough"
        );
        assert!(!looks_like_it(""));
    }

    #[test]
    fn a_ruby_style_module_declaration_is_not_mistaken_for_one() {
        assert!(!looks_like_it(
            "module Greeter\n  def hi\n    puts 'hi'\n  end\nend\n"
        ));
    }

    #[test]
    fn a_block_left_open_at_the_end_is_not_a_complete_file() {
        assert!(!looks_like_it(
            "module example.com/a\n\ngo 1.22\n\nrequire (\n\texample.com/b v1.0.0\n"
        ));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let view = view_of();

        assert_eq!(view.module.as_deref(), Some("github.com/example/pipeline"));
        assert_eq!(view.go_version.as_deref(), Some("1.24"));
        assert_eq!(view.toolchain.as_deref(), Some("go1.24.1"));
        assert!(view.requirements.iter().any(|entry| !entry.indirect));
        assert!(view.requirements.iter().any(|entry| entry.indirect));
        assert!(
            view.replacements
                .iter()
                .any(|replace| replace.to.starts_with(".."))
        );
        assert!(
            view.replacements
                .iter()
                .any(|replace| replace.to_version.is_some())
        );
        assert!(!view.excludes.is_empty());
        assert!(!view.retracts.is_empty());
        assert!(!view.godebug.is_empty());
        assert!(!view.truncated);
    }

    #[test]
    fn presents_the_module_and_its_directives() {
        let data = GomodCore.view(&fixture()).unwrap();

        let lines = GomodPresentation.present(&data);

        assert!(lines[0].contains("github.com/example/pipeline"));
        assert!(lines.iter().any(|line| line.contains("indirect")));
        assert!(lines.iter().any(|line| line.contains("=>")));
        assert!(lines.iter().any(|line| line.contains("godebug settings")));
        assert!(lines.iter().any(|line| line.trim_start().contains('=')));
    }

    #[test]
    fn a_file_that_is_not_a_go_module_file_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really-go.mod");
        std::fs::write(&path, b"nothing of the sort at all").unwrap();

        assert!(GomodCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
