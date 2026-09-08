//! C# file type plugin: core and presentation halves.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["cs"];

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

/// Words that start a statement rather than a declaration. Without these,
/// `await repository.SaveAsync(item, token);` reads as a method called
/// `SaveAsync`.
fn is_statement_keyword(word: &str) -> bool {
    matches!(
        word,
        "await" | "return" | "throw" | "yield" | "var" | "new" | "base" | "this"
    )
}

/// The keywords that introduce a type rather than a member.
const TYPE_KEYWORDS: [&str; 5] = ["class", "record", "struct", "interface", "enum"];

/// Extracts the method name from a line that declares one.
///
/// C# convention puts the opening brace on its own line, so requiring a
/// trailing `{` missed every method written in the style the language's own
/// guidelines use. A declaration is recognised instead by its shape: a
/// parameter list, a name, and a return type before it - the whitespace in
/// `public StockLevel Classify` is what a call like `Classify(item);` does
/// not have.
fn parse_method_name(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let head = trimmed
        .strip_suffix('{')
        .or_else(|| trimmed.strip_suffix(';'))
        .unwrap_or(trimmed)
        .trim_end();
    // An expression-bodied member: `public int Count() => items.Count;`.
    // Cutting at the arrow also drops a lambda's body, and what is left of
    // a statement like `items.Sum(item => item.Total);` no longer ends in a
    // parameter list, so it is not mistaken for a declaration.
    let head = head.split("=>").next().unwrap_or(head).trim_end();
    let before_paren = head.strip_suffix(')')?;
    let open = before_paren.rfind('(')?;
    let head = before_paren[..open].trim_end();

    // An assignment is a call, not a declaration, and a type declaration
    // belongs in `classes` even when it carries a parameter list, as a
    // positional record does.
    if head.contains('=')
        || TYPE_KEYWORDS
            .iter()
            .any(|keyword| head.split_whitespace().any(|word| word == *keyword))
    {
        return None;
    }

    let first = head.split_whitespace().next()?;
    if is_control_keyword(first) || is_statement_keyword(first) {
        return None;
    }
    // A declaration names a return type before the method name; a call does
    // not, so it has no whitespace left in its head.
    if !head.contains(char::is_whitespace) {
        return None;
    }

    let name_start = head
        .rfind(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .map_or(0, |index| index + 1);
    let name = &head[name_start..];
    (!name.is_empty() && !is_control_keyword(name)).then_some(name)
}

/// Extracts the type name from a top-level `class X` line, if present,
/// regardless of which accessibility/other modifiers (`public`, `internal`,
/// `sealed`, `abstract`, ...) precede the `class` keyword.
fn parse_class_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let (keyword, index) = TYPE_KEYWORDS
        .iter()
        .filter_map(|keyword| {
            trimmed
                .find(&format!("{keyword} "))
                .map(|index| (*keyword, index))
        })
        .min_by_key(|(_keyword, index)| *index)?;
    // Only as a declaration keyword: `record` and `class` also appear as
    // ordinary words inside a line of prose or a string.
    if index > 0 && !trimmed[..index].ends_with(char::is_whitespace) {
        return None;
    }
    let rest = trimmed[index + keyword.len() + 1..].trim_start();
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

/// Whether `line` is a `Console.WriteLine(...)`/`Console.Write(...)` call
/// written as a C# statement. VB.NET calls the same .NET `Console` methods
/// with identical syntax, but VB.NET statements are never `;`-terminated,
/// so requiring the trailing semicolon excludes VB.NET's
/// `Console.WriteLine("hi")` while still matching C#'s `Console.WriteLine("hi");`.
fn is_csharp_console_statement(line: &str) -> bool {
    let trimmed = line.trim_end();
    (trimmed.contains("Console.WriteLine(") || trimmed.contains("Console.Write("))
        && trimmed.ends_with(';')
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
        || text.lines().any(is_csharp_console_statement)
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
    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

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

    fn icon(&self) -> Icon {
        Icon {
            label: "C#",
            tint: 0x0068_217a,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
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
    fn does_not_sniff_a_vbnet_console_writeline_as_csharp() {
        assert!(!CSharpCore.sniff(
            b"Module Program\n    Sub Main()\n        Console.WriteLine(\"Hello, world!\")\n    End Sub\nEnd Module\n"
        ));
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

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::CSharpCore),
            plugin_api::PluginPresentation::extensions(&crate::CSharpPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn extracts_methods_written_in_the_brace_on_its_own_line_style() {
        // C#'s own guidelines put the opening brace on the next line, so
        // requiring a trailing `{` missed every method written the way the
        // language recommends.
        let source = concat!(
            "public class InventoryService\n{\n",
            "    public InventoryService(IStockRepository repository)\n    {\n    }\n\n",
            "    public async Task<decimal> TotalValueAsync(CancellationToken token)\n",
            "    {\n        var items = await _repository.ListAsync(token);\n",
            "        return items.Sum(item => item.TotalValue);\n    }\n\n",
            "    public StockLevel Classify(StockItem item) {\n    }\n\n",
            "    public int Count() => _items.Count;\n",
            "}\n",
        );

        let (methods, classes) = super::parse_definitions(source);

        assert_eq!(
            methods,
            vec!["InventoryService", "TotalValueAsync", "Classify", "Count"],
            "a call or an assignment is not a declaration"
        );
        assert_eq!(classes, vec!["InventoryService"]);
    }

    #[test]
    fn a_record_or_struct_is_a_type_not_a_method() {
        let source = concat!(
            "public record StockItem(string Sku, int Quantity);\n",
            "public struct Money\n{\n}\n",
            "public interface IStockRepository\n{\n}\n",
            "public enum StockLevel\n{\n}\n",
        );

        let (methods, classes) = super::parse_definitions(source);

        assert!(methods.is_empty(), "got {methods:?}");
        assert_eq!(
            classes,
            vec!["StockItem", "Money", "IStockRepository", "StockLevel"]
        );
    }
}
