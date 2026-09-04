//! Assembly file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// AT&T-syntax register operands (`%eax`, `%rdi`, ...) — a marker not used
/// by any sibling plugin.
const AT_T_REGISTERS: &[&str] = &[
    "%eax", "%ebx", "%ecx", "%edx", "%esp", "%ebp", "%esi", "%edi", "%rax", "%rbx", "%rcx", "%rdx",
    "%rsp", "%rbp", "%rsi", "%rdi",
];

/// View data produced by [`AssemblyCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssemblyView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of labels (`name:`) found in the content.
    pub labels: Vec<String>,
    /// Names from `.global`/`.globl` exported-symbol directives found in
    /// the content.
    pub globals: Vec<String>,
}

/// Whether `line`, once trimmed, starts with `keyword` case-insensitively.
fn starts_with_ci(line: &str, keyword: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.len() >= keyword.len() && trimmed[..keyword.len()].eq_ignore_ascii_case(keyword)
}

/// Extracts the identifier that follows a case-insensitive `keyword` prefix
/// on `line`, e.g. `.global _start` with keyword `".global "` yields
/// `_start`.
fn parse_name_after<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    if !starts_with_ci(line, keyword) {
        return None;
    }
    let rest = &line.trim_start()[keyword.len()..];
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Whether `trimmed` is a bare label declaration: an identifier (letters,
/// digits, `_`, or `.`, not starting with a digit) immediately followed by
/// `:` and nothing else.
fn parse_label(trimmed: &str) -> Option<&str> {
    let name = trimmed.strip_suffix(':')?;
    if name.is_empty() || name.starts_with(|ch: char| ch.is_ascii_digit()) {
        return None;
    }
    name.chars()
        .all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '.')
        .then_some(name)
}

/// Parses label and `.global`/`.globl` declarations out of `content`, in
/// source order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut labels = Vec::new();
    let mut globals = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(name) = parse_label(trimmed) {
            labels.push(name.to_owned());
        } else if let Some(name) =
            parse_name_after(line, ".global ").or_else(|| parse_name_after(line, ".globl "))
        {
            globals.push(name.to_owned());
        }
    }
    (labels, globals)
}

/// Whether `text` looks like assembly source: a `.global`/`.globl`
/// exported-symbol directive, a `section .text`/`.data`/`.bss` directive
/// (with or without a leading dot, covering both GAS and NASM), an
/// `int 0x80` software-interrupt syscall, a bare `syscall` instruction
/// line, or an AT&T-syntax register operand — markers not used by any
/// sibling plugin; placed just ahead of `text` in `CORE_PLUGINS`, no
/// ordering constraint against a specific sibling since it has no
/// overlapping markers.
fn has_assembly_syntax(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains(".global ")
        || lower.contains(".globl ")
        || lower.contains("section .text")
        || lower.contains("section .data")
        || lower.contains("section .bss")
        || lower.contains("int 0x80")
        || AT_T_REGISTERS.iter().any(|reg| lower.contains(reg))
        || text
            .lines()
            .any(|line| line.trim().eq_ignore_ascii_case("syscall"))
}

/// The Assembly plugin's core half.
#[derive(Debug, Default)]
pub struct AssemblyCore;

impl PluginCore for AssemblyCore {
    fn name(&self) -> &'static str {
        "assembly"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_assembly_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (labels, globals) = parse_definitions(&content);
        let view = AssemblyView {
            content,
            truncated,
            labels,
            globals,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Assembly plugin's presentation half.
#[derive(Debug, Default)]
pub struct AssemblyPresentation;

impl PluginPresentation for AssemblyPresentation {
    fn name(&self) -> &'static str {
        "assembly"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: AssemblyView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.globals.is_empty() {
            lines.push(format!("globals: {}", view.globals.join(", ")));
        }
        if !view.labels.is_empty() {
            lines.push(format!("labels: {}", view.labels.join(", ")));
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
    use super::{AssemblyCore, AssemblyPresentation, AssemblyView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-assembly-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_assembly_markers_as_assembly() {
        assert!(AssemblyCore.sniff(b".global _start\n_start:\n    movq $1, %rax\n"));
        assert!(AssemblyCore.sniff(b".globl main\nmain:\n    ret\n"));
        assert!(
            AssemblyCore.sniff(
                b"section .text\n    global _start\n_start:\n    mov eax, 4\n    int 0x80\n"
            )
        );
        assert!(AssemblyCore.sniff(b".section .data\nmsg:\n    .ascii \"hi\"\n"));
        assert!(AssemblyCore.sniff(b"_start:\n    movq %rdi, %rsi\n    syscall\n"));
        assert!(AssemblyCore.sniff(b"loop:\n  syscall\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_assembly() {
        assert!(!AssemblyCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!AssemblyCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!AssemblyCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!AssemblyCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!AssemblyCore.sniff(b"program greeter\n    implicit none\nend program greeter\n"));
        assert!(!AssemblyCore.sniff(b"my $count = 0;\nprint \"hi\\n\";\n"));
        assert!(!AssemblyCore.sniff(b"just a regular line of text\n"));
        assert!(!AssemblyCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_assembly_file_and_extracts_definitions() {
        let path = unique_temp_file("hello.s");
        std::fs::write(
            &path,
            ".global _start\n.section .text\n_start:\n    movq $1, %rax\n    movq $1, %rdi\n    movq $msg, %rsi\n    movq $13, %rdx\n    syscall\n    movq $60, %rax\n    xor %rdi, %rdi\n    syscall\n\n.section .data\nmsg:\n    .ascii \"Hello, world\\n\"\n",
        )
        .unwrap();

        let data = AssemblyCore.view(&path).unwrap();
        let view: AssemblyView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.globals, vec!["_start"]);
        assert_eq!(view.labels, vec!["_start", "msg"]);
        assert!(view.content.contains("Hello, world"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.s");
        let mut content = ".global _start\n_start:\n".to_owned();
        content.push_str(&"    nop\n".repeat(MAX_VIEW_BYTES));
        std::fs::write(&path, content).unwrap();

        let data = AssemblyCore.view(&path).unwrap();
        let view: AssemblyView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_globals_labels_and_content() {
        let data = serde_json::to_value(AssemblyView {
            content: "_start:\n    ret".to_owned(),
            truncated: false,
            labels: vec!["_start".to_owned()],
            globals: vec!["_start".to_owned()],
        })
        .unwrap();

        let lines = AssemblyPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["globals: _start", "labels: _start", "_start:", "    ret"]
        );
    }
}
