//! Swift file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`SwiftCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwiftView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `func` declarations found in the content.
    pub functions: Vec<String>,
    /// Names of top-level `struct`/`class`/`enum`/`protocol` declarations
    /// found in the content.
    pub types: Vec<String>,
}

/// Extracts the function name from a line that looks like a top-level Swift
/// function definition, e.g. `func greet() {` or
/// `public func greet(name: String) -> String {`. Any modifiers before
/// `func` and any generic parameters or return type after the parameter
/// list are skipped.
fn parse_func_name(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if !trimmed.ends_with('{') {
        return None;
    }
    let idx = trimmed.find("func ")?;
    let rest = trimmed[idx + "func ".len()..].trim_start();
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Extracts the type name from a top-level `struct`/`class`/`enum`/
/// `protocol` line, if present, regardless of which modifiers (`public`,
/// `final`, `private`, ...) precede the keyword.
fn parse_type_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    for keyword in ["struct ", "class ", "enum ", "protocol "] {
        let Some(idx) = trimmed.find(keyword) else {
            continue;
        };
        let rest = trimmed[idx + keyword.len()..].trim_start();
        let end = rest
            .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
            .unwrap_or(rest.len());
        if end > 0 {
            return Some(&rest[..end]);
        }
    }
    None
}

/// Parses top-level function and type names out of `content`, in source
/// order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut types = Vec::new();
    for line in content.lines() {
        if let Some(name) = parse_type_name(line) {
            types.push(name.to_owned());
        } else if let Some(name) = parse_func_name(line) {
            functions.push(name.to_owned());
        }
    }
    (functions, types)
}

/// Whether `text` looks like Swift source: markers not used by this
/// project's other source-language plugins, in particular `func`
/// declarations with an arrow return type (`) -> `), which avoids the Go
/// plugin's bare `func ` marker since Go does not use arrow return syntax.
fn has_swift_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("import Foundation")
            || line.starts_with("import UIKit")
            || line.starts_with("import SwiftUI")
            || line.starts_with("protocol ")
            || line.starts_with("extension ")
    }) || (text.contains("func ") && text.contains(") -> "))
        || text.contains("guard let ")
        || text.contains("guard var ")
        || text.contains("@IBOutlet")
        || text.contains("@escaping")
}

/// The Swift plugin's core half.
#[derive(Debug, Default)]
pub struct SwiftCore;

impl PluginCore for SwiftCore {
    fn name(&self) -> &'static str {
        "swift"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_swift_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, types) = parse_definitions(&content);
        let view = SwiftView {
            content,
            truncated,
            functions,
            types,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Swift plugin's presentation half.
#[derive(Debug, Default)]
pub struct SwiftPresentation;

impl PluginPresentation for SwiftPresentation {
    fn name(&self) -> &'static str {
        "swift"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: SwiftView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.types.is_empty() {
            lines.push(format!("types: {}", view.types.join(", ")));
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
    use super::{MAX_VIEW_BYTES, SwiftCore, SwiftPresentation, SwiftView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-swift-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_import_and_arrow_func_markers_as_swift() {
        assert!(SwiftCore.sniff(
            b"import Foundation\n\nfunc greet(name: String) -> String {\n    return \"Hi, \\(name)\"\n}\n"
        ));
    }

    #[test]
    fn sniffs_common_swift_markers_as_swift() {
        assert!(SwiftCore.sniff(b"func greet() -> String {\n    return \"hi\"\n}\n"));
        assert!(SwiftCore.sniff(b"guard let value = maybeValue else {\n    return\n}\n"));
        assert!(SwiftCore.sniff(b"protocol Greeter {\n    func greet() -> String\n}\n"));
        assert!(SwiftCore.sniff(b"extension String {\n    var shouted: String { self }\n}\n"));
        assert!(SwiftCore.sniff(b"@IBOutlet weak var label: UILabel!\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_swift() {
        assert!(!SwiftCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!SwiftCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!SwiftCore.sniff(b"interface Named {\n  name: string;\n}\n"));
        assert!(!SwiftCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!SwiftCore.sniff(b"package main\n\nfunc main() {}\n"));
        assert!(!SwiftCore.sniff(b"func Greet(name string) string {\n\treturn name\n}\n"));
        assert!(!SwiftCore.sniff(
            b"using System;\n\nclass Program {\n    static void Main() {\n        Console.WriteLine(\"hi\");\n    }\n}\n"
        ));
        assert!(!SwiftCore.sniff(
            b"import java.util.List;\n\nclass Greeter {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!SwiftCore.sniff(b"just a regular line of text\n"));
        assert!(!SwiftCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_swift_file_and_extracts_definitions() {
        let path = unique_temp_file("Greeter.swift");
        std::fs::write(
            &path,
            "import Foundation\n\nprotocol Greeting {\n    func greet() -> String\n}\n\nstruct Greeter: Greeting {\n    let name: String\n\n    func greet() -> String {\n        return \"Hello, \\(name)!\"\n    }\n}\n",
        )
        .unwrap();

        let data = SwiftCore.view(&path).unwrap();
        let view: SwiftView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.types, vec!["Greeting", "Greeter"]);
        assert_eq!(view.functions, vec!["greet"]);
        assert!(view.content.contains("Hello, "));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("Large.swift");
        let mut content = "func pad() -> String {\n".to_owned();
        content.push_str(&"/".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = SwiftCore.view(&path).unwrap();
        let view: SwiftView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_types_functions_and_content() {
        let data = serde_json::to_value(SwiftView {
            content: "struct A {\n}".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            types: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = SwiftPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["types: A", "functions: greet", "struct A {", "}"]
        );
    }
}
