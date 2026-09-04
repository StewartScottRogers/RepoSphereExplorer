//! Makefile file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// References to GNU Make's implicit built-in variables (`$(CC)`, and so
/// on) — conventional names essentially unique to Makefiles.
const BUILTIN_VAR_REFS: &[&str] = &[
    "$(CC)",
    "$(CXX)",
    "$(CFLAGS)",
    "$(CXXFLAGS)",
    "$(LDFLAGS)",
    "$(RM)",
    "$(AR)",
];

/// View data produced by [`MakefileCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakefileView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names from `target: prerequisites` rule declarations found in the
    /// content, in source order.
    pub targets: Vec<String>,
    /// Names from `NAME = value`/`NAME := value`/`NAME ?= value`/
    /// `NAME += value` variable assignments found in the content, in
    /// source order.
    pub variables: Vec<String>,
}

/// Extracts the target name from a `name: prerequisites` rule-header line,
/// or `None` if `trimmed` is not such a line (a comment, a special
/// `.`-prefixed directive, or a `name := value` assignment, whose `:=`
/// would otherwise look like a rule's `:`).
fn parse_target_name(trimmed: &str) -> Option<&str> {
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('.') {
        return None;
    }
    let colon = trimmed.find(':')?;
    if trimmed[colon + 1..].starts_with('=') {
        return None;
    }
    let name = trimmed[..colon].trim();
    name.split_whitespace().next()
}

/// Extracts the variable name from a `NAME = value`/`NAME := value`/
/// `NAME ?= value`/`NAME += value` assignment line, or `None` if `trimmed`
/// is not such a line.
fn parse_variable_name(trimmed: &str) -> Option<&str> {
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let eq = trimmed.find('=')?;
    let mut end = eq;
    if end > 0 && matches!(trimmed.as_bytes()[end - 1], b':' | b'?' | b'+') {
        end -= 1;
    }
    let name = trimmed[..end].trim();
    (!name.is_empty() && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_')).then_some(name)
}

/// Parses target and variable declarations out of `content`, in source
/// order. Skips tab-indented recipe lines, which never start a
/// declaration.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut targets = Vec::new();
    let mut variables = Vec::new();
    for line in content.lines() {
        if line.starts_with('\t') {
            continue;
        }
        let trimmed = line.trim();
        if let Some(name) = parse_target_name(trimmed) {
            targets.push(name.to_owned());
        } else if let Some(name) = parse_variable_name(trimmed) {
            variables.push(name.to_owned());
        }
    }
    (targets, variables)
}

/// Whether `line`, a candidate recipe line with its leading tab already
/// stripped, looks like a Make recipe command rather than an unrelated
/// tab-indented line (in particular, an assembly instruction under a
/// `label:` line, which the `assembly` plugin already claims): an
/// echo-suppressing `@` prefix, an error-ignoring `-` prefix, or a
/// `$(...)` variable reference, none of which a bare assembly mnemonic
/// line uses.
fn looks_like_recipe_line(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('\t') else {
        return false;
    };
    let rest = rest.trim_start();
    rest.starts_with('@') || rest.starts_with('-') || rest.contains("$(")
}

/// Whether `text` contains a recipe line (per [`looks_like_recipe_line`])
/// immediately following a rule-header line (per [`parse_target_name`]).
fn has_recipe_after_target(text: &str) -> bool {
    let mut prev_was_target = false;
    for line in text.lines() {
        if prev_was_target && looks_like_recipe_line(line) {
            return true;
        }
        if !line.starts_with('\t') {
            prev_was_target = parse_target_name(line.trim()).is_some();
        }
    }
    false
}

/// Whether `text` looks like Makefile source: a `.PHONY:` declaration, a
/// `$(MAKE)` recursive invocation, a reference to one of Make's implicit
/// built-in variables (see [`BUILTIN_VAR_REFS`]), a GNU Make conditional
/// directive (`ifeq (`, `ifneq (`, `ifdef `, `ifndef `), or a recipe line
/// following a rule header — markers not used by any sibling plugin.
fn has_makefile_syntax(text: &str) -> bool {
    text.contains(".PHONY:")
        || text.contains("$(MAKE)")
        || BUILTIN_VAR_REFS.iter().any(|marker| text.contains(marker))
        || text.lines().any(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("ifeq (")
                || trimmed.starts_with("ifneq (")
                || trimmed.starts_with("ifdef ")
                || trimmed.starts_with("ifndef ")
        })
        || has_recipe_after_target(text)
}

/// The Makefile plugin's core half.
#[derive(Debug, Default)]
pub struct MakefileCore;

impl PluginCore for MakefileCore {
    fn name(&self) -> &'static str {
        "makefile"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_makefile_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (targets, variables) = parse_definitions(&content);
        let view = MakefileView {
            content,
            truncated,
            targets,
            variables,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Makefile plugin's presentation half.
#[derive(Debug, Default)]
pub struct MakefilePresentation;

impl PluginPresentation for MakefilePresentation {
    fn name(&self) -> &'static str {
        "makefile"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: MakefileView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.targets.is_empty() {
            lines.push(format!("targets: {}", view.targets.join(", ")));
        }
        if !view.variables.is_empty() {
            lines.push(format!("variables: {}", view.variables.join(", ")));
        }
        lines.extend(view.content.lines().map(str::to_owned));
        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_VIEW_BYTES, MakefileCore, MakefilePresentation, MakefileView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-makefile-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_makefile_markers_as_makefile() {
        assert!(MakefileCore.sniff(b".PHONY: all clean\n\nall:\n\techo hi\n"));
        assert!(MakefileCore.sniff(b"sub:\n\t$(MAKE) -C sub\n"));
        assert!(MakefileCore.sniff(b"all:\n\t$(CC) $(CFLAGS) -o app main.c\n"));
        assert!(MakefileCore.sniff(b"ifeq ($(OS),Windows_NT)\ndetected := windows\nendif\n"));
        assert!(MakefileCore.sniff(b"ifdef DEBUG\nCFLAGS += -g\nendif\n"));
        assert!(MakefileCore.sniff(b"all: main.o\n\t@$(CC) -o app main.o\n"));
        assert!(MakefileCore.sniff(b"clean:\n\t-rm -f *.o app\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_makefile() {
        assert!(!MakefileCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!MakefileCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!MakefileCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!MakefileCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!MakefileCore.sniff(b"program greeter\n    implicit none\nend program greeter\n"));
        assert!(!MakefileCore.sniff(b"if [ -f foo ]; then\n  echo hi\nfi\n"));
        assert!(!MakefileCore.sniff(b"just a regular line of text\n"));
        assert!(!MakefileCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn does_not_sniff_assembly_labels_with_tab_indented_instructions_as_makefile() {
        assert!(
            !MakefileCore
                .sniff(b".global _start\n_start:\n\tmovq $1, %rax\n\tmovq $0, %rdi\n\tsyscall\n")
        );
    }

    #[test]
    fn views_a_real_makefile_and_extracts_targets_and_variables() {
        let path = unique_temp_file("Makefile");
        std::fs::write(
            &path,
            "CC := gcc\nCFLAGS := -Wall -O2\n\n.PHONY: all clean\n\nall: main.o\n\t$(CC) $(CFLAGS) -o app main.o\n\nmain.o: main.c\n\t$(CC) $(CFLAGS) -c main.c\n\nclean:\n\trm -f *.o app\n",
        )
        .unwrap();

        let data = MakefileCore.view(&path).unwrap();
        let view: MakefileView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.variables, vec!["CC", "CFLAGS"]);
        assert_eq!(view.targets, vec!["all", "main.o", "clean"]);
        assert!(view.content.contains("main.o: main.c"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.mk");
        let mut content = "CC := gcc\n".to_owned();
        content.push_str(&"# a comment line\n".repeat(MAX_VIEW_BYTES));
        std::fs::write(&path, content).unwrap();

        let data = MakefileCore.view(&path).unwrap();
        let view: MakefileView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_targets_variables_and_content() {
        let data = serde_json::to_value(MakefileView {
            content: "all:\n\t$(CC) -o app main.c".to_owned(),
            truncated: false,
            targets: vec!["all".to_owned()],
            variables: vec!["CC".to_owned()],
        })
        .unwrap();

        let lines = MakefilePresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "targets: all",
                "variables: CC",
                "all:",
                "\t$(CC) -o app main.c",
            ]
        );
    }
}
