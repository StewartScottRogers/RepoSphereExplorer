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
    /// Names of tables named in top-level `CREATE TABLE` statements.
    pub tables: Vec<String>,
}

/// Statement-introducing keywords checked at the start of a (trimmed) line,
/// case-insensitively. None of these overlap with any sibling plugin's
/// sniff markers.
const STATEMENT_KEYWORDS: &[&str] = &[
    "SELECT",
    "INSERT INTO",
    "UPDATE",
    "DELETE FROM",
    "CREATE TABLE",
    "CREATE INDEX",
    "CREATE VIEW",
    "ALTER TABLE",
    "DROP TABLE",
];

/// Strips `prefix` from the start of `s`, case-insensitively.
fn strip_ci_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    (s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix))
        .then(|| &s[prefix.len()..])
}

/// Whether `line`, once trimmed, starts with `keyword` case-insensitively.
fn starts_with_ci(line: &str, keyword: &str) -> bool {
    strip_ci_prefix(line.trim_start(), keyword).is_some()
}

/// Whether `text` looks like SQL source: a line starting with one of
/// [`STATEMENT_KEYWORDS`], or a `PRIMARY KEY`/`FOREIGN KEY` constraint
/// anywhere. None of these markers are used by this project's other
/// source-language plugins.
fn has_sql_syntax(text: &str) -> bool {
    let upper = text.to_ascii_uppercase();
    text.lines()
        .any(|line| STATEMENT_KEYWORDS.iter().any(|kw| starts_with_ci(line, kw)))
        || upper.contains("PRIMARY KEY")
        || upper.contains("FOREIGN KEY")
}

/// Extracts the table name from a `CREATE TABLE [IF NOT EXISTS] name` line,
/// stripping a trailing `(` or whitespace.
fn table_name(line: &str) -> Option<String> {
    let rest = strip_ci_prefix(line.trim_start(), "CREATE TABLE")?.trim_start();
    let rest = strip_ci_prefix(rest, "IF NOT EXISTS").map_or(rest, str::trim_start);
    let end = rest
        .find(|ch: char| ch.is_whitespace() || ch == '(')
        .unwrap_or(rest.len());
    let name = rest[..end].trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Parses the names of tables created by top-level `CREATE TABLE`
/// statements out of `content`, in source order.
fn parse_tables(content: &str) -> Vec<String> {
    content
        .lines()
        .filter(|line| starts_with_ci(line, "CREATE TABLE"))
        .filter_map(table_name)
        .collect()
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
        let tables = parse_tables(&content);
        let view = SqlView {
            content,
            truncated,
            tables,
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
        assert!(SqlCore.sniff(b"select * from users;\n"));
        assert!(SqlCore.sniff(b"INSERT INTO users (id) VALUES (1);\n"));
        assert!(SqlCore.sniff(b"UPDATE users SET name = 'x' WHERE id = 1;\n"));
        assert!(SqlCore.sniff(b"DELETE FROM users WHERE id = 1;\n"));
        assert!(SqlCore.sniff(b"CREATE TABLE users (id INTEGER);\n"));
        assert!(SqlCore.sniff(b"ALTER TABLE users ADD COLUMN age INTEGER;\n"));
        assert!(SqlCore.sniff(b"DROP TABLE users;\n"));
        assert!(SqlCore.sniff(
            b"CREATE TABLE users (\n  id INTEGER,\n  FOREIGN KEY (id) REFERENCES other(id)\n);\n"
        ));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_sql() {
        assert!(!SqlCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!SqlCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!SqlCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!SqlCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!SqlCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!SqlCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!SqlCore.sniff(b"just a regular line of text\n"));
        assert!(!SqlCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_sql_file_and_extracts_table_names() {
        let path = unique_temp_file("schema.sql");
        std::fs::write(
            &path,
            "CREATE TABLE users (\n  id INTEGER PRIMARY KEY,\n  name TEXT\n);\n\nCREATE TABLE IF NOT EXISTS posts (\n  id INTEGER PRIMARY KEY,\n  user_id INTEGER,\n  FOREIGN KEY (user_id) REFERENCES users(id)\n);\n\nSELECT * FROM users;\n",
        )
        .unwrap();

        let data = SqlCore.view(&path).unwrap();
        let view: SqlView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.tables, vec!["users", "posts"]);
        assert!(view.content.contains("SELECT * FROM users;"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.sql");
        let mut content = "SELECT 1;\n".to_owned();
        content.push_str(&"-- ".repeat(MAX_VIEW_BYTES));
        std::fs::write(&path, content).unwrap();

        let data = SqlCore.view(&path).unwrap();
        let view: SqlView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_tables_and_content() {
        let data = serde_json::to_value(SqlView {
            content: "CREATE TABLE users (id INTEGER);".to_owned(),
            truncated: false,
            tables: vec!["users".to_owned()],
        })
        .unwrap();

        let lines = SqlPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["tables: users", "CREATE TABLE users (id INTEGER);"]
        );
    }
}
