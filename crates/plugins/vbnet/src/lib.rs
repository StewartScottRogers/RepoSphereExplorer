//! VB.NET file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`VbNetCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VbNetView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `Sub`/`Function` procedures found in the content,
    /// in source order.
    pub procedures: Vec<String>,
}

/// Modifiers that can precede a `Sub`/`Function` declaration.
const MODIFIERS: &[&str] = &[
    "Public ",
    "Private ",
    "Protected ",
    "Friend ",
    "Shared ",
    "Overridable ",
    "Overrides ",
    "MustOverride ",
    "NotOverridable ",
    "Async ",
];

/// Extracts the name from a `[modifiers] Sub name(` or
/// `[modifiers] Function name(` declaration line, e.g.
/// `procedure_name("Public Sub Main()")` returns `Some("Main")`.
fn procedure_name(line: &str) -> Option<&str> {
    let mut rest = line;
    loop {
        let stripped = MODIFIERS.iter().find_map(|m| rest.strip_prefix(m));
        match stripped {
            Some(next) => rest = next,
            None => break,
        }
    }
    let rest = rest
        .strip_prefix("Sub ")
        .or_else(|| rest.strip_prefix("Function "))?;
    let end = rest.find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))?;
    let name = &rest[..end];
    if name.is_empty() {
        return None;
    }
    rest[end..].starts_with('(').then_some(name)
}

/// Parses top-level `Sub`/`Function` procedure names out of `content`, in
/// source order.
fn parse_definitions(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| procedure_name(line.trim_start()))
        .map(str::to_owned)
        .collect()
}

/// Whether `text` looks like VB.NET source: markers not used by any sibling
/// plugin. `End Sub`/`End Function`/`End Module`/`End Class` are VB.NET's
/// two-word block closers (no sibling plugin's language closes a block this
/// way); `Imports System` is VB's module-level import statement (unlike C#'s
/// semicolon-terminated `using System;` or Java's lowercase `import java.`);
/// a bare `Dim ` variable declaration and the ` As New ` object-instantiation
/// operator are both Visual Basic-only vocabulary no other sniffed language
/// shares.
fn has_vbnet_syntax(text: &str) -> bool {
    text.contains("End Sub")
        || text.contains("End Function")
        || text.contains("End Module")
        || text.contains("End Class")
        || text.contains("Imports System")
        || text.contains(" As New ")
        || text
            .lines()
            .any(|line| line.trim_start().starts_with("Dim "))
}

/// The VB.NET plugin's core half.
#[derive(Debug, Default)]
pub struct VbNetCore;

impl PluginCore for VbNetCore {
    fn name(&self) -> &'static str {
        "vbnet"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_vbnet_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let procedures = parse_definitions(&content);
        let view = VbNetView {
            content,
            truncated,
            procedures,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The VB.NET plugin's presentation half.
#[derive(Debug, Default)]
pub struct VbNetPresentation;

impl PluginPresentation for VbNetPresentation {
    fn name(&self) -> &'static str {
        "vbnet"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: VbNetView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.procedures.is_empty() {
            lines.push(format!("procedures: {}", view.procedures.join(", ")));
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
    use super::{MAX_VIEW_BYTES, VbNetCore, VbNetPresentation, VbNetView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-vbnet-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_vbnet_markers_as_vbnet() {
        assert!(VbNetCore.sniff(
            b"Module Module1\n    Sub Main()\n        Console.WriteLine(\"hi\")\n    End Sub\nEnd Module\n"
        ));
        assert!(VbNetCore.sniff(b"Imports System\n\nModule Program\nEnd Module\n"));
        assert!(VbNetCore.sniff(b"Public Class Greeter\n    Dim name As String\nEnd Class\n"));
        assert!(VbNetCore.sniff(b"Dim x As New List(Of Integer)\n"));
        assert!(VbNetCore.sniff(
            b"Public Function Add(a As Integer, b As Integer) As Integer\n    Return a + b\nEnd Function\n"
        ));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_vbnet() {
        assert!(!VbNetCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!VbNetCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!VbNetCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!VbNetCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!VbNetCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!VbNetCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!VbNetCore.sniff(
            b"using System;\n\npublic class Greeter {\n    public static void Main() {\n        Console.WriteLine(\"hi\");\n    }\n}\n"
        ));
        assert!(!VbNetCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!VbNetCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!VbNetCore.sniff(b"let greet name =\n    printfn \"Hello, %s\" name\n"));
        assert!(!VbNetCore.sniff(b"just a regular line of text\n"));
        assert!(!VbNetCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_vbnet_file_and_extracts_definitions() {
        let path = unique_temp_file("Greeter.vb");
        std::fs::write(
            &path,
            "Imports System\n\nModule Program\n    Public Function Greet(name As String) As String\n        Return \"Hello, \" & name & \"!\"\n    End Function\n\n    Sub Main()\n        Console.WriteLine(Greet(\"world\"))\n    End Sub\nEnd Module\n",
        )
        .unwrap();

        let data = VbNetCore.view(&path).unwrap();
        let view: VbNetView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.procedures, vec!["Greet", "Main"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("Large.vb");
        let mut content = "Module Program\n    Sub Main()\n".to_owned();
        content.push_str(&"        ' comment\n".repeat(MAX_VIEW_BYTES + 10));
        content.push_str("    End Sub\nEnd Module\n");
        std::fs::write(&path, content).unwrap();

        let data = VbNetCore.view(&path).unwrap();
        let view: VbNetView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_procedures_and_content() {
        let data = serde_json::to_value(VbNetView {
            content: "Sub Main()\n    Console.WriteLine(\"hi\")\nEnd Sub".to_owned(),
            truncated: false,
            procedures: vec!["Main".to_owned()],
        })
        .unwrap();

        let lines = VbNetPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "procedures: Main",
                "Sub Main()",
                "    Console.WriteLine(\"hi\")",
                "End Sub"
            ]
        );
    }
}
