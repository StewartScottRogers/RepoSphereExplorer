//! Java file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`JavaCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level method definitions found in the content.
    pub methods: Vec<String>,
    /// Names of top-level `class X` declarations found in the content.
    pub classes: Vec<String>,
}

/// Control-flow keywords that can precede a `(...) {` block without that
/// block being a method definition.
fn is_control_keyword(word: &str) -> bool {
    matches!(
        word,
        "if" | "for" | "while" | "switch" | "catch" | "synchronized" | "try"
    )
}

/// Extracts the method name from a line that looks like a top-level Java
/// method definition, e.g. `public void greet() {` or
/// `public static void main(String[] args) {`. Prototypes and calls (which
/// do not end the line with `{`) and control-flow statements are not
/// matched.
fn parse_method_name(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let before_brace = trimmed.strip_suffix('{')?.trim_end();
    let before_paren = before_brace.strip_suffix(')')?;
    let open = before_paren.rfind('(')?;
    let head = before_paren[..open].trim_end();
    let name_start = head
        .rfind(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .map_or(0, |i| i + 1);
    let name = &head[name_start..];
    (!name.is_empty() && !is_control_keyword(name)).then_some(name)
}

/// Extracts the type name from a top-level `class X` line, if present,
/// regardless of which modifiers (`public`, `private`, `final`,
/// `abstract`, ...) precede the `class` keyword.
fn parse_class_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let idx = trimmed.find("class ")?;
    let rest = trimmed[idx + "class ".len()..].trim_start();
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level method and class names out of `content`, in source
/// order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut methods = Vec::new();
    let mut classes = Vec::new();
    for line in content.lines() {
        if let Some(name) = parse_class_name(line) {
            classes.push(name.to_owned());
        } else if let Some(name) = parse_method_name(line) {
            methods.push(name.to_owned());
        }
    }
    (methods, classes)
}

/// Whether `text` looks like Java source: markers not used by this
/// project's other source-language plugins, in particular the C# plugin,
/// whose bare `public class ` marker a Java file also contains, and the Go
/// plugin, whose bare `package ` marker a Java file's package declaration
/// also starts with. Checking Java-only syntax here, and registering this
/// plugin ahead of `csharp`, lets a Java file that also has a `public
/// class` declaration still be claimed by this plugin first.
fn has_java_syntax(text: &str) -> bool {
    text.lines()
        .any(|line| line.trim_start().starts_with("import java."))
        || text.contains("System.out.println(")
        || text.contains("System.out.print(")
        || text.contains("System.err.println(")
        || text.contains("public static void main(String")
        || text.contains("@Override")
}

/// The Java plugin's core half.
#[derive(Debug, Default)]
pub struct JavaCore;

impl PluginCore for JavaCore {
    fn name(&self) -> &'static str {
        "java"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_java_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (methods, classes) = parse_definitions(&content);
        let view = JavaView {
            content,
            truncated,
            methods,
            classes,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Java plugin's presentation half.
#[derive(Debug, Default)]
pub struct JavaPresentation;

impl PluginPresentation for JavaPresentation {
    fn name(&self) -> &'static str {
        "java"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: JavaView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.classes.is_empty() {
            lines.push(format!("classes: {}", view.classes.join(", ")));
        }
        if !view.methods.is_empty() {
            lines.push(format!("methods: {}", view.methods.join(", ")));
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
    use super::{JavaCore, JavaPresentation, JavaView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-java-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_import_java_and_system_out_markers_as_java() {
        assert!(JavaCore.sniff(
            b"import java.util.List;\n\nclass Greeter {\n    static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
    }

    #[test]
    fn sniffs_common_java_markers_as_java() {
        assert!(JavaCore.sniff(b"public static void main(String[] args) {\n}\n"));
        assert!(JavaCore.sniff(b"System.err.println(\"oops\");\n"));
        assert!(JavaCore.sniff(b"@Override\npublic String toString() {\n    return \"\";\n}\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_java() {
        assert!(!JavaCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!JavaCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!JavaCore.sniff(b"interface Named {\n  name: string;\n}\n"));
        assert!(!JavaCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!JavaCore.sniff(b"package main\n\nfunc main() {}\n"));
        assert!(!JavaCore.sniff(
            b"using System;\n\nclass Program {\n    static void Main() {\n        Console.WriteLine(\"hi\");\n    }\n}\n"
        ));
        assert!(!JavaCore.sniff(
            b"#include <iostream>\n\nint main() {\n    std::cout << \"hi\" << std::endl;\n    return 0;\n}\n"
        ));
        assert!(!JavaCore.sniff(b"just a regular line of text\n"));
        assert!(!JavaCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_java_file_and_extracts_definitions() {
        let path = unique_temp_file("Greeter.java");
        std::fs::write(
            &path,
            "package app;\n\nimport java.util.List;\n\npublic class Greeter {\n    public void greet() {\n        System.out.println(\"Hello, world!\");\n    }\n\n    public static void main(String[] args) {\n        new Greeter().greet();\n    }\n}\n",
        )
        .unwrap();

        let data = JavaCore.view(&path).unwrap();
        let view: JavaView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.classes, vec!["Greeter"]);
        assert_eq!(view.methods, vec!["greet", "main"]);
        assert!(view.content.contains("Hello, world!"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("Large.java");
        let mut content = "public void pad() {\n".to_owned();
        content.push_str(&"/".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = JavaCore.view(&path).unwrap();
        let view: JavaView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_classes_methods_and_content() {
        let data = serde_json::to_value(JavaView {
            content: "public class A {\n}".to_owned(),
            truncated: false,
            methods: vec!["greet".to_owned()],
            classes: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = JavaPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["classes: A", "methods: greet", "public class A {", "}"]
        );
    }
}
