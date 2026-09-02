//! Kotlin file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`KotlinCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KotlinView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level function definitions found in the content.
    pub functions: Vec<String>,
    /// Names of top-level `class X` declarations found in the content.
    pub classes: Vec<String>,
}

/// Control-flow keywords that can precede a `(...) {` block without that
/// block being a function definition.
fn is_control_keyword(word: &str) -> bool {
    matches!(word, "if" | "for" | "while" | "when" | "catch" | "try")
}

/// Extracts the function name from a line that looks like a top-level
/// Kotlin function definition, e.g. `fun greet() {` or
/// `fun main(args: Array<String>) {`. Prototypes and calls (which do not
/// end the line with `{`) and control-flow statements are not matched.
fn parse_function_name(line: &str) -> Option<&str> {
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
/// regardless of which modifiers (`data`, `open`, `abstract`, `sealed`,
/// ...) precede the `class` keyword.
fn parse_class_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let idx = trimmed.find("class ")?;
    let rest = trimmed[idx + "class ".len()..].trim_start();
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

/// Whether `text` looks like Kotlin source: markers not used by this
/// project's other source-language plugins. `fun `, `data class `, and
/// `companion object` are Kotlin-only keywords/idioms, `import kotlin.`
/// mirrors the Java plugin's `import java.` check, and a bare
/// `println(` call (no `System.out.`/`fmt.` prefix) at the start of a
/// line distinguishes Kotlin's top-level `println` from Java's
/// `System.out.println(` and Go's `fmt.Println(`, both of which also
/// contain the substring `println(` but never at the start of a line.
fn has_kotlin_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("import kotlin.") || trimmed.starts_with("println(")
    }) || text.contains("fun main(")
        || text.contains("data class ")
        || text.contains("companion object")
}

/// The Kotlin plugin's core half.
#[derive(Debug, Default)]
pub struct KotlinCore;

impl PluginCore for KotlinCore {
    fn name(&self) -> &'static str {
        "kotlin"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_kotlin_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, classes) = parse_definitions(&content);
        let view = KotlinView {
            content,
            truncated,
            functions,
            classes,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Kotlin plugin's presentation half.
#[derive(Debug, Default)]
pub struct KotlinPresentation;

impl PluginPresentation for KotlinPresentation {
    fn name(&self) -> &'static str {
        "kotlin"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: KotlinView = match serde_json::from_value(data.clone()) {
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
    use super::{KotlinCore, KotlinPresentation, KotlinView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-kotlin-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_fun_main_and_import_kotlin_markers_as_kotlin() {
        assert!(KotlinCore.sniff(
            b"import kotlin.math.max\n\nfun main(args: Array<String>) {\n    println(\"hi\")\n}\n"
        ));
    }

    #[test]
    fn sniffs_common_kotlin_markers_as_kotlin() {
        assert!(KotlinCore.sniff(b"data class Point(val x: Int, val y: Int)\n"));
        assert!(KotlinCore.sniff(
            b"class Counter {\n    companion object {\n        const val MAX = 10\n    }\n}\n"
        ));
        assert!(KotlinCore.sniff(b"fun greet() {\n    println(\"hi\")\n}\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_kotlin() {
        assert!(!KotlinCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!KotlinCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!KotlinCore.sniff(b"interface Named {\n  name: string;\n}\n"));
        assert!(!KotlinCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!KotlinCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!KotlinCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!KotlinCore.sniff(
            b"#include <iostream>\n\nint main() {\n    std::cout << \"hi\" << std::endl;\n    return 0;\n}\n"
        ));
        assert!(!KotlinCore.sniff(
            b"using System;\n\nclass Program {\n    static void Main() {\n        Console.WriteLine(\"hi\");\n    }\n}\n"
        ));
        assert!(!KotlinCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!KotlinCore.sniff(b"just a regular line of text\n"));
        assert!(!KotlinCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_kotlin_file_and_extracts_definitions() {
        let path = unique_temp_file("Greeter.kt");
        std::fs::write(
            &path,
            "import kotlin.math.max\n\ndata class Greeting(val message: String)\n\nclass Greeter {\n    fun greet() {\n        println(\"Hello, world!\")\n    }\n}\n\nfun main(args: Array<String>) {\n    Greeter().greet()\n}\n",
        )
        .unwrap();

        let data = KotlinCore.view(&path).unwrap();
        let view: KotlinView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.classes, vec!["Greeting", "Greeter"]);
        assert_eq!(view.functions, vec!["greet", "main"]);
        assert!(view.content.contains("Hello, world!"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("Large.kt");
        let mut content = "fun pad() {\n".to_owned();
        content.push_str(&"/".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = KotlinCore.view(&path).unwrap();
        let view: KotlinView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_classes_functions_and_content() {
        let data = serde_json::to_value(KotlinView {
            content: "class A {\n}".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            classes: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = KotlinPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["classes: A", "functions: greet", "class A {", "}"]
        );
    }
}
