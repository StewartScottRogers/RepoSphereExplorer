//! C++ file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`CppCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CppView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level function definitions found in the content.
    pub functions: Vec<String>,
    /// Names of top-level `class X {` declarations found in the content.
    pub classes: Vec<String>,
}

/// Control-flow keywords that can precede a `(...) {` block without that
/// block being a function definition.
fn is_control_keyword(word: &str) -> bool {
    matches!(word, "if" | "for" | "while" | "switch" | "catch")
}

/// Extracts the function name from a line that looks like a top-level C++
/// function definition, e.g. `int main() {` or `void Greeter::greet() {`.
/// Prototypes and calls (which do not end the line with `{`) and
/// control-flow statements are not matched.
fn parse_function_name(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let before_brace = trimmed.strip_suffix('{')?.trim_end();
    let before_paren = before_brace.strip_suffix(')')?;
    let open = before_paren.rfind('(')?;
    let head = before_paren[..open].trim_end();
    let name_start = head
        .rfind(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == ':'))
        .map_or(0, |i| i + 1);
    let name = &head[name_start..];
    (!name.is_empty() && !is_control_keyword(name)).then_some(name)
}

/// Extracts the type name from a top-level `class X {` line, if present.
fn parse_class_name(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("class ")?.trim_start();
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level function and class names out of `content`, in source
/// order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut classes = Vec::new();
    for line in content.lines() {
        if let Some(name) = parse_class_name(line) {
            classes.push(name.to_owned());
        } else if let Some(name) = parse_function_name(line) {
            functions.push(name.to_owned());
        }
    }
    (functions, classes)
}

/// Whether `text` looks like C++ source: markers not used by this project's
/// other source-language plugins, in particular the C plugin, whose
/// `int main(`/`printf(`/`malloc(`/`NULL` markers a C++ file may also
/// contain. Checking C++-only syntax here, and registering this plugin
/// ahead of `c`, lets a C++ file that has both kinds of marker still be
/// claimed by this plugin first.
fn has_cpp_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("#include <iostream>")
            || line.starts_with("#include <vector>")
            || line.starts_with("#include <string>")
            || line.starts_with("class ")
            || line.starts_with("namespace ")
    }) || text.contains("std::")
        || text.contains("cout <<")
        || text.contains("cin >>")
        || text.contains("nullptr")
        || text.contains("public:")
        || text.contains("private:")
        || text.contains("protected:")
        || text.contains("template<")
        || text.contains("template <")
}

/// The C++ plugin's core half.
#[derive(Debug, Default)]
pub struct CppCore;

impl PluginCore for CppCore {
    fn name(&self) -> &'static str {
        "cpp"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_cpp_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, classes) = parse_definitions(&content);
        let view = CppView {
            content,
            truncated,
            functions,
            classes,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The C++ plugin's presentation half.
#[derive(Debug, Default)]
pub struct CppPresentation;

impl PluginPresentation for CppPresentation {
    fn name(&self) -> &'static str {
        "cpp"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: CppView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.classes.is_empty() {
            lines.push(format!("classes: {}", view.classes.join(", ")));
        }
        if !view.functions.is_empty() {
            lines.push(format!("functions: {}", view.functions.join(", ")));
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
    use super::{CppCore, CppPresentation, CppView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-cpp-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_iostream_and_class_markers_as_cpp() {
        assert!(
            CppCore.sniff(b"#include <iostream>\n\nint main() {\n    std::cout << \"hi\";\n}\n")
        );
        assert!(CppCore.sniff(b"class Greeter {\npublic:\n    void greet();\n};\n"));
    }

    #[test]
    fn sniffs_common_cpp_markers_as_cpp() {
        assert!(CppCore.sniff(b"namespace app {\n}\n"));
        assert!(CppCore.sniff(b"auto *p = nullptr;\n"));
        assert!(CppCore.sniff(b"template<typename T>\nT max(T a, T b);\n"));
    }

    #[test]
    fn prefers_cpp_over_c_markers_when_both_present() {
        // A C++ file commonly also contains C-ish markers (int main(,
        // printf(, NULL); the C++-specific syntax must still be enough to
        // claim it, since this plugin is registered ahead of `c`.
        assert!(CppCore.sniff(
            b"#include <iostream>\n\nint main() {\n    std::cout << \"hi\" << std::endl;\n    return 0;\n}\n"
        ));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_cpp() {
        assert!(!CppCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!CppCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!CppCore.sniff(b"interface Named {\n  name: string;\n}\n"));
        assert!(!CppCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!CppCore.sniff(b"package main\n\nfunc main() {}\n"));
        assert!(!CppCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!CppCore.sniff(b"just a regular line of text\n"));
        assert!(!CppCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_cpp_file_and_extracts_definitions() {
        let path = unique_temp_file("greet.cpp");
        std::fs::write(
            &path,
            "#include <iostream>\n\nclass Greeter {\npublic:\n    void greet() {\n        std::cout << \"Hello, world!\" << std::endl;\n    }\n};\n\nint main() {\n    Greeter g;\n    g.greet();\n    return 0;\n}\n",
        )
        .unwrap();

        let data = CppCore.view(&path).unwrap();
        let view: CppView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.classes, vec!["Greeter"]);
        assert_eq!(view.functions, vec!["greet", "main"]);
        assert!(view.content.contains("Hello, world!"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.cpp");
        let mut content = "int pad() {\n".to_owned();
        content.push_str(&"/".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = CppCore.view(&path).unwrap();
        let view: CppView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_classes_functions_and_content() {
        let data = serde_json::to_value(CppView {
            content: "class A {\n};".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            classes: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = CppPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["classes: A", "functions: greet", "class A {", "};"]
        );
    }
}
