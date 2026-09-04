//! GraphQL schema file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`GraphQlCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQlView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `type`/`input`/`enum`/`interface`/`union`
    /// definitions found in the content, in source order.
    pub types: Vec<String>,
}

/// Top-level SDL keywords whose definitions [`parse_types`] names.
const TYPE_SYSTEM_KEYWORDS: &[&str] = &[
    "type ",
    "input ",
    "enum ",
    "interface ",
    "union ",
    "scalar ",
];

/// Extracts the identifier that follows an alphanumeric/underscore run at
/// the start of `text`, if any.
fn leading_identifier(text: &str) -> Option<&str> {
    let end = text
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(text.len());
    (end > 0).then(|| &text[..end])
}

/// Extracts the definition name from a top-level SDL declaration line, e.g.
/// `type Query {` or `input Filter {` or `scalar DateTime`, given the
/// declaration's `keyword` (including its trailing space).
fn parse_type_name<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.trim_start().strip_prefix(keyword)?;
    leading_identifier(rest)
}

/// Parses the names of top-level SDL type-system definitions out of
/// `content`, in source order.
fn parse_types(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            TYPE_SYSTEM_KEYWORDS
                .iter()
                .find_map(|keyword| parse_type_name(line, keyword))
        })
        .map(str::to_owned)
        .collect()
}

/// Whether `text` looks like a GraphQL schema (SDL): a top-level
/// `type `/`input ` object declaration opened with `{`, a `schema {`
/// definition, a top-level `scalar `/`directive @` declaration, or a
/// top-level `extend type ` declaration — markers not used by any sibling
/// plugin. Deliberately does not sniff a bare `interface `/`enum ` line,
/// since the TypeScript plugin already claims those.
fn has_graphql_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        ((trimmed.starts_with("type ") || trimmed.starts_with("input ")) && line.contains('{'))
            || trimmed.starts_with("schema {")
            || trimmed.starts_with("scalar ")
            || trimmed.starts_with("extend type ")
    }) || text.contains("directive @")
}

/// The GraphQL schema plugin's core half.
#[derive(Debug, Default)]
pub struct GraphQlCore;

impl PluginCore for GraphQlCore {
    fn name(&self) -> &'static str {
        "graphql"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_graphql_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let types = parse_types(&content);
        let view = GraphQlView {
            content,
            truncated,
            types,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The GraphQL schema plugin's presentation half.
#[derive(Debug, Default)]
pub struct GraphQlPresentation;

impl PluginPresentation for GraphQlPresentation {
    fn name(&self) -> &'static str {
        "graphql"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: GraphQlView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.types.is_empty() {
            lines.push(format!("types: {}", view.types.join(", ")));
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
    use super::{GraphQlCore, GraphQlPresentation, GraphQlView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-graphql-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_graphql_schema_syntax_as_graphql() {
        assert!(GraphQlCore.sniff(b"type Query {\n  hero: Character\n}\n"));
        assert!(GraphQlCore.sniff(b"type User implements Node {\n  id: ID!\n}\n"));
        assert!(GraphQlCore.sniff(b"input Filter {\n  name: String\n}\n"));
        assert!(GraphQlCore.sniff(b"schema {\n  query: Query\n}\n"));
        assert!(GraphQlCore.sniff(b"scalar DateTime\n"));
        assert!(GraphQlCore.sniff(b"extend type Query {\n  more: String\n}\n"));
        assert!(GraphQlCore.sniff(b"directive @deprecated(reason: String) on FIELD_DEFINITION\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_graphql() {
        assert!(!GraphQlCore.sniff(b"interface Named {\n  name: string;\n}\n"));
        assert!(!GraphQlCore.sniff(b"enum Color { Red, Green, Blue }\n"));
        assert!(!GraphQlCore.sniff(b"type Name = string;\n"));
        assert!(!GraphQlCore.sniff(b"package main\n\nfunc main() {}\n"));
        assert!(!GraphQlCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!GraphQlCore.sniff(b"just a regular line of text\n"));
        assert!(!GraphQlCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_graphql_schema_file_and_extracts_type_names() {
        let path = unique_temp_file("schema.graphql");
        std::fs::write(
            &path,
            "scalar DateTime\n\ntype Query {\n  hero: Character\n}\n\ninput Filter {\n  name: String\n}\n\nenum Episode {\n  NEWHOPE\n  EMPIRE\n}\n\ninterface Character {\n  id: ID!\n}\n\nunion SearchResult = Character | Filter\n",
        )
        .unwrap();

        let data = GraphQlCore.view(&path).unwrap();
        let view: GraphQlView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(
            view.types,
            vec![
                "DateTime",
                "Query",
                "Filter",
                "Episode",
                "Character",
                "SearchResult"
            ]
        );
        assert!(view.content.contains("type Query {"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.graphql");
        let mut content = "type Query {\n  hero: String\n}\n".to_owned();
        content.push_str(&"# ".repeat(MAX_VIEW_BYTES));
        std::fs::write(&path, content).unwrap();

        let data = GraphQlCore.view(&path).unwrap();
        let view: GraphQlView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_types_and_content() {
        let data = serde_json::to_value(GraphQlView {
            content: "type Query {\n  hero: String\n}".to_owned(),
            truncated: false,
            types: vec!["Query".to_owned()],
        })
        .unwrap();

        let lines = GraphQlPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["types: Query", "type Query {", "  hero: String", "}"]
        );
    }
}
