//! Python file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`PythonCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PythonView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `def` functions found in the content.
    pub functions: Vec<String>,
    /// Names of top-level `class` definitions found in the content.
    pub classes: Vec<String>,
}

/// Extracts the identifier following `keyword` (`"def"` or `"class"`) at the
/// start of `line`, if present.
fn top_level_name<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(keyword)?.strip_prefix(' ')?;
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
        if let Some(name) = top_level_name(line, "def") {
            functions.push(name.to_owned());
        } else if let Some(name) = top_level_name(line, "class") {
            classes.push(name.to_owned());
        }
    }
    (functions, classes)
}

/// Whether `prefix`'s first line is a `python`-flavoured shebang.
fn has_python_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with("#!") && line.contains("python"))
}

/// Whether a `def`/`class` header line ends in a bare `:` (after stripping
/// a trailing `#` comment), Python's block-opening syntax. C++/Dart's
/// `class Name {`, Groovy's `def name(...) {`, and TypeScript's `class Foo
/// implements Bar {` share the same leading keyword but open with `{`
/// instead, so this rules them out.
fn ends_with_colon_header(line: &str) -> bool {
    line.split('#')
        .next()
        .unwrap_or("")
        .trim_end()
        .ends_with(':')
}

/// Whether any line in `text` opens a top-level `def` or `class`, the
/// content markers used since Python source carries no magic bytes.
fn has_python_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        (top_level_name(line, "def").is_some() || top_level_name(line, "class").is_some())
            && ends_with_colon_header(line)
    })
}

/// The Python plugin's core half.
#[derive(Debug, Default)]
pub struct PythonCore;

impl PluginCore for PythonCore {
    fn name(&self) -> &'static str {
        "python"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_python_shebang(text) || has_python_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, classes) = parse_definitions(&content);
        let view = PythonView {
            content,
            truncated,
            functions,
            classes,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Python plugin's presentation half.
#[derive(Debug, Default)]
pub struct PythonPresentation;

impl PluginPresentation for PythonPresentation {
    fn name(&self) -> &'static str {
        "python"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: PythonView = match serde_json::from_value(data.clone()) {
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
    use super::{MAX_VIEW_BYTES, PythonCore, PythonPresentation, PythonView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-python-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_shebang_line_as_python() {
        assert!(PythonCore.sniff(b"#!/usr/bin/env python3\nprint('hi')\n"));
    }

    #[test]
    fn sniffs_top_level_def_and_class_as_python() {
        assert!(PythonCore.sniff(b"def greet():\n    pass\n"));
        assert!(PythonCore.sniff(b"class Greeter:\n    pass\n"));
    }

    #[test]
    fn does_not_sniff_plain_text_as_python() {
        assert!(!PythonCore.sniff(b"just a regular line of text\n"));
        assert!(!PythonCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn does_not_sniff_a_cpp_class_as_python() {
        assert!(!PythonCore.sniff(
            b"#include <iostream>\n\nclass Greeter {\npublic:\n    void greet() {\n        std::cout << \"Hello, World!\" << std::endl;\n    }\n};\n\nint main() {\n    Greeter().greet();\n    return 0;\n}\n"
        ));
    }

    #[test]
    fn does_not_sniff_a_dart_class_as_python() {
        assert!(!PythonCore.sniff(
            b"class Greeter {\n  void greet() {\n    print('Hello, World!');\n  }\n}\n\nvoid main() {\n  Greeter().greet();\n}\n"
        ));
    }

    #[test]
    fn does_not_sniff_a_groovy_def_as_python() {
        assert!(
            !PythonCore.sniff(
                b"def greet(name) {\n    println \"Hello, ${name}!\"\n}\n\ngreet('World')\n"
            )
        );
    }

    #[test]
    fn does_not_sniff_a_typescript_class_as_python() {
        assert!(!PythonCore.sniff(
            b"interface Greetable {\n  greet(): string;\n}\n\nclass Greeter implements Greetable {\n  greet(): string {\n    return \"Hello, World!\";\n  }\n}\n"
        ));
    }

    #[test]
    fn views_a_real_python_file_and_extracts_definitions() {
        let path = unique_temp_file("greet.py");
        std::fs::write(
            &path,
            "import sys\n\n\nclass Greeter:\n    pass\n\n\ndef greet(name):\n    return f\"Hello, {name}!\"\n",
        )
        .unwrap();

        let data = PythonCore.view(&path).unwrap();
        let view: PythonView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.classes, vec!["Greeter"]);
        assert_eq!(view.functions, vec!["greet"]);
        assert!(view.content.contains("Hello, {name}!"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.py");
        let mut content = "def pad():\n".to_owned();
        content.push_str(&"#".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = PythonCore.view(&path).unwrap();
        let view: PythonView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_classes_functions_and_content() {
        let data = serde_json::to_value(PythonView {
            content: "class A:\n    pass".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            classes: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = PythonPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["classes: A", "functions: greet", "class A:", "    pass"]
        );
    }
}
