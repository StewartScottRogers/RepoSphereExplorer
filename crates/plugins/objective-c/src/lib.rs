//! Objective-C file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`ObjectiveCCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectiveCView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of `@interface`/`@implementation` declarations found in the
    /// content.
    pub classes: Vec<String>,
    /// Selectors of method declarations (lines opening with `-` or `+`)
    /// found in the content.
    pub methods: Vec<String>,
}

/// Extracts the identifier following an `@interface `/`@implementation `
/// declaration at the start of `line`, if present.
fn parse_class_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed
        .strip_prefix("@interface ")
        .or_else(|| trimmed.strip_prefix("@implementation "))?;
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Extracts the leading selector segment from a method declaration line,
/// e.g. `greet:` from `- (void)greet:(NSString *)name;` or `new` from
/// `+ (instancetype)new;`. Only the first segment is captured, matching the
/// level of detail this project's other source-language plugins extract for
/// their own top-level definitions.
fn parse_method_name(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let sign = trimmed.chars().next()?;
    if sign != '-' && sign != '+' {
        return None;
    }
    let after_sign = trimmed[1..].trim_start();
    let after_open_paren = after_sign.strip_prefix('(')?;
    let close = after_open_paren.find(')')?;
    let after_type = after_open_paren[close + 1..].trim_start();
    let end = after_type
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == ':'))
        .unwrap_or(after_type.len());
    let name = &after_type[..end];
    (!name.is_empty()).then(|| name.to_owned())
}

/// Parses the class names and method selectors out of `content`, in source
/// order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut classes = Vec::new();
    let mut methods = Vec::new();
    for line in content.lines() {
        if let Some(name) = parse_class_name(line) {
            classes.push(name.to_owned());
        } else if let Some(name) = parse_method_name(line) {
            methods.push(name);
        }
    }
    (classes, methods)
}

/// Whether `text` looks like Objective-C (or Objective-C++) source: markers
/// not used by any sibling plugin. `#import` is Objective-C's header
/// directive, distinct from C/C++'s `#include`; `@interface`/
/// `@implementation`/`@protocol`/`@property` are Objective-C's compiler
/// directives; `NSLog(` is Foundation's console-output call; and `@"` opens
/// an `NSString` literal. Objective-C source commonly also contains a bare
/// `int main(` (matching the C plugin's marker) and Objective-C++ source
/// commonly contains a bare `class ` declaration (matching the C++ plugin's
/// marker), so this plugin must be checked before `cpp`/`c` in
/// `CORE_PLUGINS` to claim such files by these stronger markers first;
/// placed just ahead of `cpp` in that list.
fn has_objective_c_syntax(text: &str) -> bool {
    text.contains("#import <")
        || text.contains("#import \"")
        || text.contains("@interface ")
        || text.contains("@implementation ")
        || text.contains("@protocol ")
        || text.contains("@property")
        || text.contains("NSLog(")
        || text.contains("@\"")
}

/// The Objective-C plugin's core half.
#[derive(Debug, Default)]
pub struct ObjectiveCCore;

impl PluginCore for ObjectiveCCore {
    fn name(&self) -> &'static str {
        "objective-c"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_objective_c_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (classes, methods) = parse_definitions(&content);
        let view = ObjectiveCView {
            content,
            truncated,
            classes,
            methods,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Objective-C plugin's presentation half.
#[derive(Debug, Default)]
pub struct ObjectiveCPresentation;

impl PluginPresentation for ObjectiveCPresentation {
    fn name(&self) -> &'static str {
        "objective-c"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: ObjectiveCView = match serde_json::from_value(data.clone()) {
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
    use super::{MAX_VIEW_BYTES, ObjectiveCCore, ObjectiveCPresentation, ObjectiveCView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-objective-c-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_objective_c_markers_as_objective_c() {
        assert!(ObjectiveCCore.sniff(b"#import <Foundation/Foundation.h>\n"));
        assert!(ObjectiveCCore.sniff(b"#import \"Greeter.h\"\n"));
        assert!(ObjectiveCCore.sniff(b"@interface Greeter : NSObject\n@end\n"));
        assert!(ObjectiveCCore.sniff(b"@implementation Greeter\n@end\n"));
        assert!(ObjectiveCCore.sniff(b"@protocol Greeting\n@end\n"));
        assert!(ObjectiveCCore.sniff(b"@property (nonatomic, strong) NSString *name;\n"));
        assert!(ObjectiveCCore.sniff(b"NSLog(@\"hi\");\n"));
        assert!(ObjectiveCCore.sniff(b"NSString *greeting = @\"hi\";\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_objective_c() {
        assert!(!ObjectiveCCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!ObjectiveCCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!ObjectiveCCore.sniff(b"interface Greeter {\n  greet(): void;\n}\n"));
        assert!(!ObjectiveCCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!ObjectiveCCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(
            !ObjectiveCCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n")
        );
        assert!(!ObjectiveCCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!ObjectiveCCore.sniff(
            b"#include <iostream>\n\nclass Greeter {\npublic:\n    void greet() { std::cout << \"hi\"; }\n};\n"
        ));
        assert!(!ObjectiveCCore.sniff(
            b"using System;\n\npublic class Greeter {\n    public static void Main() {\n        Console.WriteLine(\"hi\");\n    }\n}\n"
        ));
        assert!(!ObjectiveCCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(
            !ObjectiveCCore
                .sniff(b"import Foundation\n\nfunc greet() -> String {\n  return \"hi\"\n}\n")
        );
        assert!(!ObjectiveCCore.sniff(b"just a regular line of text\n"));
        assert!(!ObjectiveCCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_objective_c_file_and_extracts_definitions() {
        let path = unique_temp_file("Greeter.m");
        std::fs::write(
            &path,
            "#import <Foundation/Foundation.h>\n\n@interface Greeter : NSObject\n- (void)greet:(NSString *)name;\n@end\n\n@implementation Greeter\n- (void)greet:(NSString *)name {\n    NSLog(@\"Hello, %@!\", name);\n}\n+ (instancetype)new {\n    return [[Greeter alloc] init];\n}\n@end\n",
        )
        .unwrap();

        let data = ObjectiveCCore.view(&path).unwrap();
        let view: ObjectiveCView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.classes, vec!["Greeter", "Greeter"]);
        assert_eq!(view.methods, vec!["greet:", "greet:", "new"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("Large.m");
        let mut content = "@interface Large : NSObject\n".to_owned();
        content.push_str(&"// ".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = ObjectiveCCore.view(&path).unwrap();
        let view: ObjectiveCView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_classes_methods_and_content() {
        let data = serde_json::to_value(ObjectiveCView {
            content: "@interface A : NSObject\n- (void)greet;".to_owned(),
            truncated: false,
            classes: vec!["A".to_owned()],
            methods: vec!["greet".to_owned()],
        })
        .unwrap();

        let lines = ObjectiveCPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "classes: A",
                "methods: greet",
                "@interface A : NSObject",
                "- (void)greet;"
            ]
        );
    }
}
