//! Protocol Buffers schema file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`ProtobufCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtobufView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `message`/`service` definitions found in the
    /// content, in source order.
    pub definitions: Vec<String>,
}

/// Top-level keywords whose block definitions [`parse_definitions`] names.
const DEFINITION_KEYWORDS: &[&str] = &["message ", "service "];

/// Extracts the identifier that follows an alphanumeric/underscore run at
/// the start of `text`, if any.
fn leading_identifier(text: &str) -> Option<&str> {
    let end = text
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(text.len());
    (end > 0).then(|| &text[..end])
}

/// Extracts the definition name from a top-level block declaration line,
/// e.g. `message Foo {` or `service FooService {`, given the declaration's
/// `keyword` (including its trailing space).
fn parse_definition_name<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.trim_start().strip_prefix(keyword)?;
    leading_identifier(rest)
}

/// Parses the names of top-level `message`/`service` definitions out of
/// `content`, in source order.
fn parse_definitions(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            DEFINITION_KEYWORDS
                .iter()
                .find_map(|keyword| parse_definition_name(line, keyword))
        })
        .map(str::to_owned)
        .collect()
}

/// Whether `text` looks like a Protocol Buffers schema: a `syntax =
/// "proto2"`/`"proto3"` declaration, a top-level `message `/`service `
/// block opened with `{`, an RPC method's `) returns (` signature, or an
/// `import "….proto";` statement — markers not used by any sibling plugin.
///
/// This project has no path/extension-based dispatch (sniffing is
/// content-only), and a real `.proto` file's `package foo.bar;` line
/// matches the Perl plugin's `package Name;` marker, while its `enum Foo {`
/// line matches the Rust plugin's bare `enum ` marker. Rather than avoid
/// those two markers as *this* plugin's own triggers (they're not used
/// here), `protobuf` is registered ahead of both `perl` and `rust` in
/// `CORE_PLUGINS` so a `.proto` file's own stronger, unique markers below
/// get first refusal.
fn has_protobuf_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("syntax = \"proto2\"")
            || trimmed.starts_with("syntax = \"proto3\"")
            || ((trimmed.starts_with("message ") || trimmed.starts_with("service "))
                && line.contains('{'))
            || (trimmed.starts_with("import \"") && trimmed.trim_end().ends_with(".proto\";"))
    }) || text.contains(") returns (")
}

/// The Protocol Buffers plugin's core half.
#[derive(Debug, Default)]
pub struct ProtobufCore;

impl PluginCore for ProtobufCore {
    fn name(&self) -> &'static str {
        "protobuf"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_protobuf_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let definitions = parse_definitions(&content);
        let view = ProtobufView {
            content,
            truncated,
            definitions,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Protocol Buffers plugin's presentation half.
#[derive(Debug, Default)]
pub struct ProtobufPresentation;

impl PluginPresentation for ProtobufPresentation {
    fn name(&self) -> &'static str {
        "protobuf"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: ProtobufView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.definitions.is_empty() {
            lines.push(format!("definitions: {}", view.definitions.join(", ")));
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
    use super::{MAX_VIEW_BYTES, ProtobufCore, ProtobufPresentation, ProtobufView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-protobuf-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_protobuf_schema_syntax_as_protobuf() {
        assert!(ProtobufCore.sniff(b"syntax = \"proto3\";\n\npackage foo;\n"));
        assert!(ProtobufCore.sniff(b"syntax = \"proto2\";\n"));
        assert!(ProtobufCore.sniff(b"message Foo {\n  string name = 1;\n}\n"));
        assert!(ProtobufCore.sniff(b"service FooService {\n  rpc Get (Req) returns (Res);\n}\n"));
        assert!(ProtobufCore.sniff(b"rpc Get (Req) returns (Res);\n"));
        assert!(ProtobufCore.sniff(b"import \"google/protobuf/timestamp.proto\";\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_protobuf() {
        assert!(!ProtobufCore.sniff(b"package Foo::Bar;\n\nuse strict;\n"));
        assert!(!ProtobufCore.sniff(b"enum Color {\n    Red,\n    Green,\n}\n"));
        assert!(!ProtobufCore.sniff(b"package main\n\nfunc main() {}\n"));
        assert!(!ProtobufCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!ProtobufCore.sniff(b"just a regular line of text\n"));
        assert!(!ProtobufCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_protobuf_schema_file_and_extracts_definition_names() {
        let path = unique_temp_file("schema.proto");
        std::fs::write(
            &path,
            "syntax = \"proto3\";\n\npackage example;\n\nenum Status {\n  UNKNOWN = 0;\n  OK = 1;\n}\n\nmessage Foo {\n  string name = 1;\n  Status status = 2;\n}\n\nservice FooService {\n  rpc GetFoo (FooRequest) returns (Foo);\n}\n",
        )
        .unwrap();

        let data = ProtobufCore.view(&path).unwrap();
        let view: ProtobufView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.definitions, vec!["Foo", "FooService"]);
        assert!(view.content.contains("message Foo {"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.proto");
        let mut content =
            "syntax = \"proto3\";\n\nmessage Foo {\n  string name = 1;\n}\n".to_owned();
        content.push_str(&"// ".repeat(MAX_VIEW_BYTES));
        std::fs::write(&path, content).unwrap();

        let data = ProtobufCore.view(&path).unwrap();
        let view: ProtobufView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_definitions_and_content() {
        let data = serde_json::to_value(ProtobufView {
            content: "message Foo {\n  string name = 1;\n}".to_owned(),
            truncated: false,
            definitions: vec!["Foo".to_owned()],
        })
        .unwrap();

        let lines = ProtobufPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "definitions: Foo",
                "message Foo {",
                "  string name = 1;",
                "}"
            ]
        );
    }
}
