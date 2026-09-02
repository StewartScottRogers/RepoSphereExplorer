//! C# file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`CSharpCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CSharpView {
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
        "if" | "for" | "foreach" | "while" | "switch" | "catch" | "using" | "lock"
    )
}

/// Extracts the method name from a line that looks like a top-level C#
/// method definition, e.g. `public void Greet() {` or
/// `public static void Main(string[] args) {`. Prototypes and calls (which
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
/// regardless of which accessibility/other modifiers (`public`, `internal`,
/// `sealed`, `abstract`, ...) precede the `class` keyword.
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

/// Whether `text` looks like C# source: markers not used by this project's
/// other source-language plugins, in particular the C++ plugin, whose
/// `class `/`namespace ` markers a C# file may also contain. Checking
/// C#-only syntax here, and registering this plugin ahead of `cpp`, lets a
/// C# file that also has a `namespace` block still be claimed by this
/// plugin first.
fn has_csharp_syntax(text: &str) -> bool {
    text.lines()
        .any(|line| line.trim_start().starts_with("using System"))
        || text.contains("Console.WriteLine(")
        || text.contains("Console.Write(")
        || text.contains("public class ")
        || text.contains("internal class ")
        || text.contains("public static void Main(")
        || text.contains("static void Main(")
        || text.contains("{ get; set; }")
}

/// The C# plugin's core half.
#[derive(Debug, Default)]
pub struct CSharpCore;

impl PluginCore for CSharpCore {
    fn name(&self) -> &'static str {
        "csharp"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_csharp_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (methods, classes) = parse_definitions(&content);
        let view = CSharpView {
            content,
            truncated,
            methods,
            classes,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The C# plugin's presentation half.
#[derive(Debug, Default)]
pub struct CSharpPresentation;

impl PluginPresentation for CSharpPresentation {
    fn name(&self) -> &'static str {
        "csharp"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: CSharpView = match serde_json::from_value(data.clone()) {
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
    use super::{CSharpCore, CSharpPresentation, CSharpView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-csharp-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_using_system_and_console_markers_as_csharp() {
        assert!(CSharpCore.sniff(
            b"using System;\n\nclass Program {\n    static void Main() {\n        Console.WriteLine(\"hi\");\n    }\n}\n"
        ));
    }

    #[test]
    fn sniffs_common_csharp_markers_as_csharp() {
        assert!(CSharpCore.sniff(b"public class Greeter {\n}\n"));
        assert!(CSharpCore.sniff(b"internal class Widget {\n}\n"));
        assert!(CSharpCore.sniff(b"public string Name { get; set; }\n"));
        assert!(CSharpCore.sniff(b"public static void Main(string[] args) {\n}\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_csharp() {
        assert!(!CSharpCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!CSharpCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!CSharpCore.sniff(b"interface Named {\n  name: string;\n}\n"));
        assert!(!CSharpCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!CSharpCore.sniff(b"package main\n\nfunc main() {}\n"));
        assert!(!CSharpCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!CSharpCore.sniff(
            b"#include <iostream>\n\nint main() {\n    std::cout << \"hi\" << std::endl;\n    return 0;\n}\n"
        ));
        assert!(!CSharpCore.sniff(b"just a regular line of text\n"));
        assert!(!CSharpCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_csharp_file_and_extracts_definitions() {
        let path = unique_temp_file("Greeter.cs");
        std::fs::write(
            &path,
            "using System;\n\nnamespace App\n{\n    public class Greeter\n    {\n        public void Greet() {\n            Console.WriteLine(\"Hello, world!\");\n        }\n    }\n\n    public class Program\n    {\n        public static void Main() {\n            new Greeter().Greet();\n        }\n    }\n}\n",
        )
        .unwrap();

        let data = CSharpCore.view(&path).unwrap();
        let view: CSharpView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.classes, vec!["Greeter", "Program"]);
        assert_eq!(view.methods, vec!["Greet", "Main"]);
        assert!(view.content.contains("Hello, world!"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("Large.cs");
        let mut content = "public void Pad() {\n".to_owned();
        content.push_str(&"/".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = CSharpCore.view(&path).unwrap();
        let view: CSharpView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_classes_methods_and_content() {
        let data = serde_json::to_value(CSharpView {
            content: "public class A {\n}".to_owned(),
            truncated: false,
            methods: vec!["Greet".to_owned()],
            classes: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = CSharpPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["classes: A", "methods: Greet", "public class A {", "}"]
        );
    }
}
