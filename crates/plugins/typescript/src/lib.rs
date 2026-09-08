//! TypeScript file type plugin: core and presentation halves.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["ts", "tsx", "mts", "cts"];

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

/// Words that may stand between the start of a line and the declaration it
/// carries. A module that exports what it declares - which is most of them -
/// puts at least one of these in front of every name worth listing.
const MODIFIERS: [&str; 9] = [
    "export",
    "default",
    "declare",
    "abstract",
    "async",
    "public",
    "private",
    "protected",
    "static",
];

/// `line` with its leading modifiers removed, so a declaration can be
/// recognised by its keyword wherever the modifiers left it.
fn without_modifiers(line: &str) -> &str {
    let mut rest = line.trim_start();
    while let Some((word, tail)) = rest.split_once(char::is_whitespace) {
        if MODIFIERS.contains(&word) {
            rest = tail.trim_start();
        } else {
            break;
        }
    }
    rest
}

/// Extracts the identifier following `keyword` (`"function"`, `"class"`, or
/// `"interface"`) at the start of `line`, if present.
fn top_level_name<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = without_modifiers(line)
        .strip_prefix(keyword)?
        .strip_prefix(' ')?;
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

/// TypeScript primitive/utility type names checked by
/// [`has_typed_declaration`].
const TS_TYPE_NAMES: [&str; 5] = ["string", "number", "boolean", "void", "unknown"];

/// Whether `text` contains a genuine TypeScript type annotation: a field or
/// binding declaration terminated with `;` (`name: string;`), or a function
/// return-type annotation immediately followed by the body's opening brace
/// (`): void {`). A bare `identifier: Type` substring is not enough on its
/// own: e.g. Nim's return-type annotations (`proc f(x: string): string =`)
/// and parameter lists share that shape but end in `=`, not `;` or `{`.
fn has_typed_declaration(text: &str) -> bool {
    TS_TYPE_NAMES.iter().any(|ty| {
        let field = format!(": {ty};");
        let return_type = format!("): {ty} {{");
        text.contains(field.as_str()) || text.contains(return_type.as_str())
    })
}

/// Whether `text` looks like TypeScript source: markers that do not also
/// appear in plain JavaScript, so this plugin does not shadow
/// `plugin-javascript`'s sniff. Type annotations, interfaces, enums, and
/// `import`/`export type` are TypeScript-only; bare `function`/`class`
/// declarations are left to the JavaScript plugin.
fn has_typescript_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        top_level_name(line, "interface").is_some() || top_level_name(line, "enum").is_some()
    }) || text
        .lines()
        .any(|line| line.trim_start().starts_with("type ") && line.contains(" = "))
        || has_typed_declaration(text)
        || text.contains("implements ")
        || text.contains("import type ")
        || text.contains("export type ")
        || text.contains("as const")
}

/// The TypeScript plugin's core half.
#[derive(Debug, Default)]
pub struct TypeScriptCore;

impl PluginCore for TypeScriptCore {
    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

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

    fn icon(&self) -> Icon {
        Icon {
            label: "TS",
            tint: 0x0031_78c6,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
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
    fn does_not_sniff_a_csharp_file_with_public_modifiers_as_typescript() {
        assert!(!TypeScriptCore.sniff(
            b"using System;\n\nnamespace HelloApp\n{\n    public class Hello\n    {\n        public static void Main(string[] args)\n        {\n            Console.WriteLine(\"Hello, world!\");\n        }\n    }\n}\n"
        ));
    }

    #[test]
    fn does_not_sniff_a_java_file_with_public_and_private_modifiers_as_typescript() {
        assert!(!TypeScriptCore.sniff(
            b"import java.util.Objects;\n\npublic class Hello {\n    private final String name;\n\n    public Hello(String name) {\n        this.name = name;\n    }\n\n    public static void main(String[] args) {\n        Hello hello = new Hello(\"World\");\n        System.out.println(\"Hello, \" + hello.name + \"!\");\n    }\n}\n"
        ));
    }

    #[test]
    fn does_not_sniff_a_nim_file_with_a_string_return_type_as_typescript() {
        assert!(!TypeScriptCore.sniff(
            b"import std/strformat\n\nproc greet(name: string): string =\n  &\"Hello, {name}\"\n\necho greet(\"World\")\n"
        ));
    }

    #[test]
    fn does_not_sniff_a_solidity_file_with_a_public_state_variable_as_typescript() {
        assert!(!TypeScriptCore.sniff(
            b"pragma solidity ^0.8.0;\n\ncontract Greeter {\n    mapping(address => string) public greetings;\n\n    function setGreeting(string memory greeting) public {\n        greetings[msg.sender] = greeting;\n    }\n}\n"
        ));
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

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::TypeScriptCore),
            plugin_api::PluginPresentation::extensions(&crate::TypeScriptPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn extracts_declarations_that_carry_modifiers() {
        // A module that exports what it declares - which is most of them -
        // reported nothing at all while this matched at line start only.
        let source = concat!(
            "export interface TaskSpec {\n  id: string;\n}\n\n",
            "export type TaskId = string;\n\n",
            "export abstract class Scheduler {\n}\n\n",
            "export default class Runner {\n}\n\n",
            "export async function run(): Promise<void> {}\n",
            "function local(): void {}\n",
        );

        let (functions, classes, interfaces) = super::parse_definitions(source);

        assert_eq!(functions, vec!["run", "local"]);
        assert_eq!(classes, vec!["Scheduler", "Runner"]);
        assert_eq!(interfaces, vec!["TaskSpec"]);
    }
}
