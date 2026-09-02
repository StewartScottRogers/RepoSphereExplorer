//! TypeScript file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`TypeScriptCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeScriptView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `function` declarations found in the content.
    pub functions: Vec<String>,
    /// Names of top-level `class` declarations found in the content.
    pub classes: Vec<String>,
    /// Names of top-level `interface` declarations found in the content.
    pub interfaces: Vec<String>,
}

/// Extracts the identifier following `keyword` (`"function"`, `"class"`, or
/// `"interface"`) at the start of `line`, if present.
fn top_level_name<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(keyword)?.strip_prefix(' ')?;
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level function, class, and interface names out of `content`,
/// in source order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut classes = Vec::new();
    let mut interfaces = Vec::new();
    for line in content.lines() {
        if let Some(name) = top_level_name(line, "function") {
            functions.push(name.to_owned());
        } else if let Some(name) = top_level_name(line, "class") {
            classes.push(name.to_owned());
        } else if let Some(name) = top_level_name(line, "interface") {
            interfaces.push(name.to_owned());
        }
    }
    (functions, classes, interfaces)
}

/// Whether `text` looks like TypeScript source: markers that do not also
/// appear in plain JavaScript, so this plugin does not shadow
/// `plugin-javascript`'s sniff. Type annotations, interfaces, enums,
/// visibility modifiers, and `import`/`export type` are TypeScript-only;
/// bare `function`/`class` declarations are left to the JavaScript plugin.
fn has_typescript_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        top_level_name(line, "interface").is_some() || top_level_name(line, "enum").is_some()
    }) || text
        .lines()
        .any(|line| line.trim_start().starts_with("type ") && line.contains(" = "))
        || text.contains(": string")
        || text.contains(": number")
        || text.contains(": boolean")
        || text.contains(": void")
        || text.contains(": unknown")
        || text.contains("implements ")
        || text.contains("public ")
        || text.contains("private ")
        || text.contains("protected ")
        || text.contains("readonly ")
        || text.contains("import type ")
        || text.contains("export type ")
        || text.contains("as const")
}

/// The TypeScript plugin's core half.
#[derive(Debug, Default)]
pub struct TypeScriptCore;

impl PluginCore for TypeScriptCore {
    fn name(&self) -> &'static str {
        "typescript"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_typescript_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, classes, interfaces) = parse_definitions(&content);
        let view = TypeScriptView {
            content,
            truncated,
            functions,
            classes,
            interfaces,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The TypeScript plugin's presentation half.
#[derive(Debug, Default)]
pub struct TypeScriptPresentation;

impl PluginPresentation for TypeScriptPresentation {
    fn name(&self) -> &'static str {
        "typescript"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: TypeScriptView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.interfaces.is_empty() {
            lines.push(format!("interfaces: {}", view.interfaces.join(", ")));
        }
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
    use super::{MAX_VIEW_BYTES, TypeScriptCore, TypeScriptPresentation, TypeScriptView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-typescript-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_an_interface_or_enum_declaration_as_typescript() {
        assert!(TypeScriptCore.sniff(b"interface Greeter {\n  greet(): void;\n}\n"));
        assert!(TypeScriptCore.sniff(b"enum Color {\n  Red,\n  Green,\n}\n"));
    }

    #[test]
    fn sniffs_a_type_alias_as_typescript() {
        assert!(TypeScriptCore.sniff(b"type Name = string;\n"));
    }

    #[test]
    fn sniffs_type_annotations_and_modifiers_as_typescript() {
        assert!(TypeScriptCore.sniff(b"function greet(name: string): void {}\n"));
        assert!(TypeScriptCore.sniff(b"class Greeter {\n  private readonly name: string;\n}\n"));
        assert!(TypeScriptCore.sniff(b"class Greeter implements Named {}\n"));
        assert!(TypeScriptCore.sniff(b"import type { Foo } from './foo';\n"));
    }

    #[test]
    fn does_not_sniff_plain_javascript_or_text_as_typescript() {
        assert!(!TypeScriptCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!TypeScriptCore.sniff(b"const add = (a, b) => a + b;\n"));
        assert!(!TypeScriptCore.sniff(b"just a regular line of text\n"));
        assert!(!TypeScriptCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_typescript_file_and_extracts_definitions() {
        let path = unique_temp_file("greet.ts");
        std::fs::write(
            &path,
            "interface Named {\n  name: string;\n}\n\n\nclass Greeter implements Named {\n  constructor(public name: string) {}\n}\n\n\nfunction greet(person: Named): string {\n  return `Hello, ${person.name}!`;\n}\n",
        )
        .unwrap();

        let data = TypeScriptCore.view(&path).unwrap();
        let view: TypeScriptView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.interfaces, vec!["Named"]);
        assert_eq!(view.classes, vec!["Greeter"]);
        assert_eq!(view.functions, vec!["greet"]);
        assert!(view.content.contains("Hello, ${person.name}!"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.ts");
        let mut content = "function pad(): void {\n".to_owned();
        content.push_str(&"/".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = TypeScriptCore.view(&path).unwrap();
        let view: TypeScriptView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_interfaces_classes_functions_and_content() {
        let data = serde_json::to_value(TypeScriptView {
            content: "class A {\n}".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            classes: vec!["A".to_owned()],
            interfaces: vec!["Named".to_owned()],
        })
        .unwrap();

        let lines = TypeScriptPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "interfaces: Named",
                "classes: A",
                "functions: greet",
                "class A {",
                "}"
            ]
        );
    }
}
