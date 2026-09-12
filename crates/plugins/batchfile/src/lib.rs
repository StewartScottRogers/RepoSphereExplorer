//! Batch file file type plugin: core and presentation halves.
//!
//! A batch file is labels, jumps and whatever it can find on the path.
//! This reads whether echoing is off, whether the script keeps its
//! variables to itself, whether delayed expansion is on, the labels, the
//! calls and jumps, the variables set, the programs run, the errorlevel
//! checks - and the labels nothing reaches, and the jumps with nowhere
//! to land.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["bat", "cmd"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`BatchfileCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchfileView {
    /// The switches the script turns on at the top: `echo off`,
    /// `setlocal`, and `delayed expansion` - which is what makes a
    /// variable set inside a loop readable inside that same loop.
    pub settings: Vec<String>,
    /// The labels it declares, in order.
    pub labels: Vec<String>,
    /// The labels it jumps to with `goto`.
    pub gotos: Vec<String>,
    /// The labels it calls as a subroutine.
    pub calls: Vec<String>,
    /// The variables it sets, by name.
    pub variables: Vec<String>,
    /// The external programs it runs.
    pub commands: Vec<String>,
    /// How many times it checks `errorlevel`.
    pub error_checks: usize,
    /// Labels nothing jumps to or calls.
    pub unreachable_labels: Vec<String>,
    /// Jumps to a label the script has not got, which stop the script
    /// dead with "The system cannot find the batch label specified".
    pub missing_labels: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Words that open a built-in rather than naming an external program.
const BUILT_INS: &[&str] = &[
    "echo", "set", "if", "else", "for", "goto", "call", "exit", "rem", "pause", "setlocal",
    "endlocal", "shift", "cd", "chdir", "md", "mkdir", "rd", "rmdir", "del", "erase", "copy",
    "move", "ren", "rename", "dir", "type", "cls", "title", "color", "path", "prompt", "start",
    "pushd", "popd", "verify", "ver", "vol", "date", "time", "assoc", "ftype", "break",
];

/// `line` with its comment stripped, and without the `@` that hides it.
fn cleaned(line: &str) -> &str {
    let line = line.trim().trim_start_matches('@').trim_start();
    let lower = line.to_ascii_lowercase();
    if lower.starts_with("rem ") || lower == "rem" || line.starts_with("::") {
        return "";
    }
    line
}

/// The label `line` declares, if it declares one.
///
/// A label is a colon at the start of a line, and `::` is the comment
/// form that abuses that - so a second colon is not a label.
fn label_of(line: &str) -> Option<String> {
    let rest = line.strip_prefix(':')?;
    if rest.starts_with(':') {
        return None;
    }
    let name = rest.split([' ', '\t']).next()?.trim();
    (!name.is_empty()).then(|| name.to_ascii_lowercase())
}

/// Where `word` appears in `text` as a word of its own.
fn word_at(text: &str, word: &str) -> Option<usize> {
    let mut from = 0usize;
    while let Some(offset) = text[from..].find(word) {
        let at = from + offset;
        let before_is_boundary = at == 0
            || !text[..at]
                .chars()
                .next_back()
                .is_some_and(|letter| letter.is_alphanumeric() || letter == '_');
        if before_is_boundary {
            return Some(at);
        }
        from = at + word.len();
    }
    None
}

/// Adds `setting` to `into` once.
fn remember(setting: &str, into: &mut Vec<String>) {
    if !into.iter().any(|seen| seen == setting) {
        into.push(setting.to_owned());
    }
}

/// The label named after `keyword` on `line`, if there is one.
fn jump_target(line: &str, keyword: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    // `if errorlevel 1 goto :failed` is the commonest jump in any batch
    // file, so the keyword is looked for anywhere on the line rather than
    // only at the start - as a word, so `cargo` is not a `go`.
    let at = word_at(&lower, keyword)?;
    let rest = &lower[at + keyword.len()..];
    if !rest.starts_with([' ', '\t', ':']) {
        return None;
    }
    let target = rest.trim().trim_start_matches(':').trim();
    let target = target.split([' ', '\t']).next()?;
    // `goto :eof` is a built-in end, not a label somebody forgot.
    (!target.is_empty() && target != "eof").then(|| target.to_owned())
}

/// The variable `line` sets, if it sets one.
fn assignment_of(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let rest = lower.strip_prefix("set ")?;
    // `set /a total=1`, `set /p answer=`, `set "name=value"`.
    let rest = rest.trim();
    let rest = rest
        .strip_prefix("/a ")
        .or_else(|| rest.strip_prefix("/p "))
        .unwrap_or(rest)
        .trim()
        .trim_start_matches('"');
    let name = rest.split('=').next()?.trim();
    (!name.is_empty() && !name.contains(' ')).then(|| name.to_owned())
}

/// Everything [`BatchfileView`] holds, read from `text`.
fn parse(text: &str) -> BatchfileView {
    let mut view = BatchfileView {
        settings: Vec::new(),
        labels: Vec::new(),
        gotos: Vec::new(),
        calls: Vec::new(),
        variables: Vec::new(),
        commands: Vec::new(),
        error_checks: 0,
        unreachable_labels: Vec::new(),
        missing_labels: Vec::new(),
        truncated: false,
    };
    for raw in text.lines() {
        let line = cleaned(raw);
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();

        if lower.starts_with("echo off") {
            remember("echo off", &mut view.settings);
        }
        if lower.starts_with("setlocal") {
            remember("setlocal", &mut view.settings);
            if lower.contains("enabledelayedexpansion") {
                remember("delayed expansion", &mut view.settings);
            }
            continue;
        }
        if lower.contains("errorlevel") {
            view.error_checks += 1;
        }
        if let Some(label) = label_of(line) {
            view.labels.push(label);
            continue;
        }
        if let Some(target) = jump_target(line, "goto") {
            view.gotos.push(target);
            continue;
        }
        if let Some(target) = jump_target(line, "call") {
            // `call :label` is a subroutine; `call other.bat` is a program.
            if word_at(&lower, "call").is_some_and(|at| lower[at..].starts_with("call :")) {
                view.calls.push(target);
            } else if !view.commands.contains(&target) {
                view.commands.push(target);
            }
            continue;
        }
        if let Some(name) = assignment_of(line) {
            if !view.variables.contains(&name) {
                view.variables.push(name);
            }
            continue;
        }
        // Whatever a line starts with, if it is not a built-in, is a
        // program the script expects to find on the path.
        let Some(word) = lower.split([' ', '\t']).next() else {
            continue;
        };
        let word = word.trim_end_matches(|letter: char| !letter.is_alphanumeric() && letter != '.');
        if !word.is_empty()
            && !BUILT_INS.contains(&word)
            && !word.starts_with('%')
            && !word.starts_with('(')
            && !word.starts_with(')')
            && !view.commands.contains(&word.to_owned())
        {
            view.commands.push(word.to_owned());
        }
    }

    let reached: Vec<&String> = view.gotos.iter().chain(view.calls.iter()).collect();
    view.unreachable_labels = view
        .labels
        .iter()
        .filter(|label| !reached.contains(label))
        .cloned()
        .collect();
    view.missing_labels = reached
        .iter()
        .filter(|target| !view.labels.contains(target))
        .map(|target| (*target).clone())
        .collect();
    view.missing_labels.dedup();
    view
}

/// Whether `text` is a Windows batch file.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    // `@echo off` or `setlocal` is batch and nothing else. Without one of
    // those, a label and a goto together will do.
    !view.settings.is_empty() || (!view.labels.is_empty() && !view.gotos.is_empty())
}

/// The Batch file plugin's core half.
#[derive(Debug, Default)]
pub struct BatchfileCore;

impl PluginCore for BatchfileCore {
    fn name(&self) -> &'static str {
        "batchfile"
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
        // The labels and jumps are the shape of the script, and each is
        // on the view already.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Batch file plugin's presentation half.
#[derive(Debug, Default)]
pub struct BatchfilePresentation;

impl PluginPresentation for BatchfilePresentation {
    fn name(&self) -> &'static str {
        "batchfile"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "BAT",
            tint: 0x004d_4d4d,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: BatchfileView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "Batch file{}",
            if view.settings.is_empty() {
                String::new()
            } else {
                format!(": {}", view.settings.join(", "))
            }
        ));
        if !view.labels.is_empty() {
            lines.push(format!(
                "{} label(s): {}",
                view.labels.len(),
                view.labels.join(", ")
            ));
        }
        if !view.calls.is_empty() {
            lines.push(format!("Calls: {}", view.calls.join(", ")));
        }
        if !view.gotos.is_empty() {
            lines.push(format!("Jumps to: {}", view.gotos.join(", ")));
        }
        if !view.variables.is_empty() {
            lines.push(format!("Sets: {}", view.variables.join(", ")));
        }
        if !view.commands.is_empty() {
            lines.push(format!("Runs: {}", view.commands.join(", ")));
        }
        lines.push(format!("{} errorlevel check(s)", view.error_checks));
        if !view.missing_labels.is_empty() {
            lines.push("Jumps to a label this file has not got, which stops the".to_owned());
            lines.push("script where it stands:".to_owned());
            for label in &view.missing_labels {
                lines.push(format!("  {label}"));
            }
        }
        if !view.unreachable_labels.is_empty() {
            lines.push("Nothing jumps to or calls these, so they run only if the".to_owned());
            lines.push("line above falls into them:".to_owned());
            for label in &view.unreachable_labels {
                lines.push(format!("  {label}"));
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
    use super::{BatchfileCore, BatchfilePresentation, BatchfileView, cleaned, label_of, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const SCRIPT: &str = concat!(
        "@echo off\r\n",
        "setlocal enabledelayedexpansion\r\n",
        "\r\n",
        "rem Build and test.\r\n",
        ":: A second comment form.\r\n",
        "\r\n",
        "set \"ROOT=%~dp0\"\r\n",
        "set /a FAILURES=0\r\n",
        "\r\n",
        "call :build\r\n",
        "if errorlevel 1 goto :failed\r\n",
        "\r\n",
        "call :test\r\n",
        "if errorlevel 1 goto :failed\r\n",
        "goto :done\r\n",
        "\r\n",
        ":build\r\n",
        "cargo build --release\r\n",
        "exit /b %errorlevel%\r\n",
        "\r\n",
        ":test\r\n",
        "cargo test --all-features\r\n",
        "exit /b %errorlevel%\r\n",
        "\r\n",
        ":orphan\r\n",
        "echo nothing reaches this\r\n",
        "\r\n",
        ":failed\r\n",
        "echo something went wrong\r\n",
        "exit /b 1\r\n",
        "\r\n",
        ":done\r\n",
        "endlocal\r\n",
    );

    #[test]
    fn sniffs_a_script() {
        assert!(BatchfileCore.sniff(SCRIPT.as_bytes()));
    }

    #[test]
    fn does_not_claim_a_shell_script() {
        assert!(!BatchfileCore.sniff(b"#!/bin/sh\nset -e\necho hello\n"));
        assert!(!BatchfileCore.sniff(b""));
    }

    #[test]
    fn both_comment_forms_are_comments() {
        assert_eq!(cleaned("rem Build and test."), "");
        assert_eq!(cleaned("REM shouting"), "");
        assert_eq!(cleaned(":: A second comment form."), "");
        assert_eq!(cleaned("@echo off"), "echo off");
    }

    #[test]
    fn a_double_colon_is_not_a_label() {
        assert_eq!(label_of(":build"), Some("build".to_owned()));
        assert_eq!(label_of(":: a comment"), None, "`::` is the comment form");
        assert_eq!(label_of("echo :not-a-label"), None);
    }

    #[test]
    fn reads_the_opening_settings() {
        let view = parse(SCRIPT);

        assert_eq!(
            view.settings,
            vec![
                "echo off".to_owned(),
                "setlocal".to_owned(),
                "delayed expansion".to_owned()
            ]
        );
        assert_eq!(
            view.error_checks, 4,
            "two `if errorlevel` and two `exit /b %errorlevel%`"
        );
    }

    #[test]
    fn separates_a_subroutine_call_from_a_program() {
        let view = parse(SCRIPT);

        assert_eq!(view.calls, vec!["build".to_owned(), "test".to_owned()]);
        assert!(view.commands.contains(&"cargo".to_owned()));
        assert!(
            !view.commands.contains(&"build".to_owned()),
            "`call :build` is a subroutine, not a program called build"
        );
    }

    #[test]
    fn reads_the_variables_it_sets() {
        let view = parse(SCRIPT);

        assert_eq!(
            view.variables,
            vec!["root".to_owned(), "failures".to_owned()]
        );
    }

    #[test]
    fn names_the_label_nothing_reaches() {
        let view = parse(SCRIPT);

        assert_eq!(view.unreachable_labels, vec!["orphan".to_owned()]);
        assert!(
            view.missing_labels.is_empty(),
            "every jump in this script has a label to land on"
        );
    }

    #[test]
    fn a_conditional_goto_is_still_a_goto() {
        let view = parse(concat!(
            "@echo off\r\n",
            ":start\r\n",
            "if errorlevel 1 goto :failed\r\n",
            ":failed\r\n",
            "exit /b 1\r\n",
        ));

        assert_eq!(view.gotos, vec!["failed".to_owned()]);
        assert!(
            !view.unreachable_labels.contains(&"failed".to_owned()),
            "`if errorlevel 1 goto :failed` reaches `:failed` as surely as a bare goto"
        );
        assert_eq!(
            view.unreachable_labels,
            vec!["start".to_owned()],
            "nothing jumps to `:start`; the script falls into it, which is what the warning says"
        );
    }

    #[test]
    fn names_a_jump_with_nowhere_to_land() {
        let view = parse("@echo off\r\n:start\r\ngoto :nowhere\r\n");

        assert_eq!(view.missing_labels, vec!["nowhere".to_owned()]);
    }

    #[test]
    fn goto_eof_is_not_a_missing_label() {
        let view = parse("@echo off\r\n:sub\r\necho x\r\ngoto :eof\r\n");

        assert!(
            view.missing_labels.is_empty(),
            "`goto :eof` is a built-in end, not a label somebody forgot"
        );
    }

    #[test]
    fn presents_both_warnings_with_their_reasons() {
        let data = serde_json::to_value(parse(SCRIPT)).unwrap();

        let lines = BatchfilePresentation.present(&data);

        assert!(lines[0].contains("delayed expansion"));
        assert!(lines.iter().any(|line| line.contains("falls into them")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/batchfile/build.bat");

        let data = BatchfileCore.view(&path).unwrap();
        let view: BatchfileView = serde_json::from_value(data).unwrap();

        assert_eq!(view.settings.len(), 3);
        assert!(view.labels.len() >= 5);
        assert!(view.calls.len() >= 2);
        assert!(!view.gotos.is_empty());
        assert!(view.variables.len() >= 3);
        assert!(!view.commands.is_empty());
        assert!(view.error_checks >= 3);
        assert!(!view.unreachable_labels.is_empty());
    }

    #[test]
    fn the_legacy_fixture_proves_the_missing_label() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/batchfile/legacy.bat");

        let data = BatchfileCore.view(&path).unwrap();
        let view: BatchfileView = serde_json::from_value(data).unwrap();

        assert_eq!(
            view.missing_labels,
            vec!["cleanup".to_owned()],
            "`:cleanup` was deleted and the jumps to it were not"
        );
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::BatchfileCore),
            plugin_api::PluginPresentation::extensions(&crate::BatchfilePresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
