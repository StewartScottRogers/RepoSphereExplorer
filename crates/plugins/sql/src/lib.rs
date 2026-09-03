//! SQL file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`SqlCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of tables named in `CREATE TABLE` statements, in source order.
    pub tables: Vec<String>,
    /// Kinds of statement found (e.g. `SELECT`, `CREATE TABLE`), in source
    /// order, without duplicates.
    pub statements: Vec<String>,
}

/// The statement-introducing keywords this plugin recognises, each paired
/// with the label recorded in [`SqlView::statements`] when a line starts
/// with it (case-insensitively).
const STATEMENT_MARKERS: &[(&str, &str)] = &[
    ("SELECT ", "SELECT"),
    ("INSERT INTO ", "INSERT"),
    ("UPDATE ", "UPDATE"),
    ("DELETE FROM ", "DELETE"),
    ("CREATE TABLE ", "CREATE TABLE"),
    ("CREATE INDEX ", "CREATE INDEX"),
    ("CREATE UNIQUE INDEX ", "CREATE INDEX"),
    ("CREATE VIEW ", "CREATE VIEW"),
    ("ALTER TABLE ", "ALTER TABLE"),
    ("DROP TABLE ", "DROP TABLE"),
];

/// Whether `text` starts with `prefix`, ignoring ASCII case.
fn starts_with_ci(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

/// Strips `prefix` from the start of `text`, ignoring ASCII case.
fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    starts_with_ci(text, prefix).then(|| &text[prefix.len()..])
}

/// Whether `text` looks like SQL: statement keywords (`SELECT`, `INSERT
/// INTO`, `UPDATE`, `DELETE FROM`, `CREATE TABLE`/`INDEX`/`VIEW`, `ALTER
/// TABLE`, `DROP TABLE`) at the start of a line, or a `PRIMARY KEY`/`FOREIGN
/// KEY` constraint anywhere. None of these appear as markers in this
/// project's other source-language plugins.
fn has_sql_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        STATEMENT_MARKERS
            .iter()
            .any(|(marker, _)| starts_with_ci(trimmed, marker))
    }) || {
        let upper = text.to_ascii_uppercase();
        upper.contains("PRIMARY KEY") || upper.contains("FOREIGN KEY")
    }
}

/// Extracts the table name from a `CREATE TABLE [IF NOT EXISTS] name`
/// line, if `trimmed` is one.
fn table_name(trimmed: &str) -> Option<String> {
    let rest = strip_prefix_ci(trimmed, "CREATE TABLE ")?;
    let rest = strip_prefix_ci(rest, "IF NOT EXISTS ").unwrap_or(rest);
    let name: String = rest
        .trim_start()
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

/// Parses table names and statement kinds out of `content`, in source
/// order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut tables = Vec::new();
    let mut statements = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        if let Some(name) = table_name(trimmed) {
            tables.push(name);
        }
        for (marker, label) in STATEMENT_MARKERS {
            if starts_with_ci(trimmed, marker) && !statements.iter().any(|s| s == label) {
                statements.push((*label).to_owned());
            }
        }
    }
    (tables, statements)
}

/// The SQL plugin's core half.
#[derive(Debug, Default)]
pub struct SqlCore;

impl PluginCore for SqlCore {
    fn name(&self) -> &'static str {
        "sql"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_sql_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (tables, statements) = parse_definitions(&content);
        let view = SqlView {
            content,
            truncated,
            tables,
            statements,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The SQL plugin's presentation half.
#[derive(Debug, Default)]
pub struct SqlPresentation;

impl PluginPresentation for SqlPresentation {
    fn name(&self) -> &'static str {
        "sql"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: SqlView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.tables.is_empty() {
            lines.push(format!("tables: {}", view.tables.join(", ")));
        }
        if !view.statements.is_empty() {
            lines.push(format!("statements: {}", view.statements.join(", ")));
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
    use super::{MAX_VIEW_BYTES, SqlCore, SqlPresentation, SqlView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-sql-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_sql_statements_as_sql() {
        assert!(SqlCore.sniff(b"SELECT * FROM users;\n"));
        assert!(SqlCore.sniff(b"select id from users;\n"));
        assert!(SqlCore.sniff(
            b"CREATE TABLE users (\n    id INTEGER PRIMARY KEY,\n    name TEXT NOT NULL\n);\n"
        ));
        assert!(SqlCore.sniff(b"INSERT INTO users (id, name) VALUES (1, 'a');\n"));
        assert!(SqlCore.sniff(b"UPDATE users SET name = 'b' WHERE id = 1;\n"));
        assert!(SqlCore.sniff(b"DELETE FROM users WHERE id = 1;\n"));
        assert!(SqlCore.sniff(b"ALTER TABLE users ADD COLUMN age INTEGER;\n"));
        assert!(SqlCore.sniff(b"DROP TABLE users;\n"));
        assert!(SqlCore.sniff(
            b"CREATE TABLE orders (\n    user_id INTEGER,\n    FOREIGN KEY (user_id) REFERENCES users(id)\n);\n"
        ));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_sql() {
        assert!(!SqlCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!SqlCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!SqlCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!SqlCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!SqlCore.sniff(b"#!/usr/bin/env bash\necho hi\n"));
        assert!(!SqlCore.sniff(b"just a regular line of text\n"));
        assert!(!SqlCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_sql_file_and_extracts_definitions() {
        let path = unique_temp_file("schema.sql");
        std::fs::write(
            &path,
            "CREATE TABLE IF NOT EXISTS users (\n    id INTEGER PRIMARY KEY,\n    name TEXT NOT NULL\n);\n\nSELECT * FROM users;\n",
        )
        .unwrap();

        let data = SqlCore.view(&path).unwrap();
        let view: SqlView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.tables, vec!["users"]);
        assert_eq!(view.statements, vec!["CREATE TABLE", "SELECT"]);
        assert!(view.content.contains("SELECT"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.sql");
        let mut content = "SELECT 1;\n".to_owned();
        content.push_str(&"-".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = SqlCore.view(&path).unwrap();
        let view: SqlView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_tables_statements_and_content() {
        let data = serde_json::to_value(SqlView {
            content: "CREATE TABLE a (id INTEGER);\nSELECT * FROM a;".to_owned(),
            truncated: false,
            tables: vec!["a".to_owned()],
            statements: vec!["CREATE TABLE".to_owned(), "SELECT".to_owned()],
        })
        .unwrap();

        let lines = SqlPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "tables: a",
                "statements: CREATE TABLE, SELECT",
                "CREATE TABLE a (id INTEGER);",
                "SELECT * FROM a;"
            ]
        );
    }
}
