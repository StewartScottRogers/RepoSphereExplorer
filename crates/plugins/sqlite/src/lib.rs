//! SQLite database file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// Maximum number of rows read into the view per table; tables with more
/// are truncated.
const MAX_ROWS: usize = 200;

/// SQLite's fixed 16-byte header, present at the very start of every
/// database file.
const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";

/// One table's schema and sampled rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqliteTable {
    /// The table's name.
    pub name: String,
    /// The table's `CREATE TABLE` statement, as stored by SQLite.
    pub schema: String,
    /// The column names, in declaration order.
    pub headers: Vec<String>,
    /// Up to [`MAX_ROWS`] rows, each cell rendered to its display text.
    pub rows: Vec<Vec<String>>,
    /// Total number of rows in the table.
    pub row_count: usize,
    /// Whether `rows` was cut off at [`MAX_ROWS`].
    pub truncated: bool,
}

/// View data produced by [`SqliteCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteView {
    /// Every user table in the database, in name order.
    pub tables: Vec<SqliteTable>,
}

/// Whether `prefix` opens with SQLite's fixed file header.
fn looks_like_sqlite(prefix: &[u8]) -> bool {
    prefix.starts_with(SQLITE_MAGIC)
}

/// Wraps `name` in double quotes for use as a SQL identifier, doubling any
/// embedded quote per SQL's escaping rule.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Renders a SQLite value to its display text.
fn render_value(value: &SqlValue) -> String {
    match value {
        SqlValue::Null => "NULL".to_owned(),
        SqlValue::Integer(i) => i.to_string(),
        SqlValue::Real(f) => f.to_string(),
        SqlValue::Text(s) => s.clone(),
        SqlValue::Blob(b) => format!("<blob {} bytes>", b.len()),
    }
}

/// Reads every user table's schema and up to [`MAX_ROWS`] rows, skipping
/// SQLite's own internal `sqlite_%` bookkeeping tables.
fn read_tables(path: &Path) -> io::Result<Vec<SqliteTable>> {
    let to_io_err =
        |err: rusqlite::Error| io::Error::new(io::ErrorKind::InvalidData, err.to_string());

    let conn =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(to_io_err)?;

    let mut list_stmt = conn
        .prepare(
            "SELECT name, sql FROM sqlite_master \
             WHERE type = 'table' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
             ORDER BY name",
        )
        .map_err(to_io_err)?;
    let table_list = list_stmt
        .query_map([], |row| {
            let name: String = row.get(0)?;
            let schema: Option<String> = row.get(1)?;
            Ok((name, schema.unwrap_or_default()))
        })
        .map_err(to_io_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(to_io_err)?;
    drop(list_stmt);

    let mut tables = Vec::new();
    for (name, schema) in table_list {
        let quoted = quote_ident(&name);

        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {quoted}"), [], |row| {
                row.get(0)
            })
            .map_err(to_io_err)?;
        let row_count = usize::try_from(count).unwrap_or(0);

        let mut row_stmt = conn
            .prepare(&format!("SELECT * FROM {quoted} LIMIT {MAX_ROWS}"))
            .map_err(to_io_err)?;
        let headers = row_stmt
            .column_names()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let column_count = headers.len();
        let rows = row_stmt
            .query_map([], |row| {
                (0..column_count)
                    .map(|index| {
                        row.get::<_, SqlValue>(index)
                            .map(|value| render_value(&value))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .map_err(to_io_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(to_io_err)?;

        let truncated = rows.len() < row_count;
        tables.push(SqliteTable {
            name,
            schema,
            headers,
            rows,
            row_count,
            truncated,
        });
    }

    Ok(tables)
}

/// Renders `table` as an aligned, spreadsheet-like grid: a header row, a
/// rule beneath it, then one line per sampled row.
fn present_table(table: &SqliteTable) -> Vec<String> {
    let widest_row = table.rows.iter().map(Vec::len).max().unwrap_or(0);
    let column_count = table.headers.len().max(widest_row);

    let mut widths = vec![0usize; column_count];
    for (index, header) in table.headers.iter().enumerate() {
        widths[index] = widths[index].max(header.chars().count());
    }
    for row in &table.rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }

    let format_row = |cells: &[String]| -> String {
        (0..column_count)
            .map(|index| {
                let cell = cells.get(index).map_or("", String::as_str);
                format!("{cell:<width$}", width = widths[index])
            })
            .collect::<Vec<_>>()
            .join(" | ")
    };

    let mut lines = vec![format_row(&table.headers)];
    lines.push(
        widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join("-+-"),
    );
    lines.extend(table.rows.iter().map(|row| format_row(row)));
    lines
}

/// The SQLite plugin's core half.
#[derive(Debug, Default)]
pub struct SqliteCore;

impl PluginCore for SqliteCore {
    fn name(&self) -> &'static str {
        "sqlite"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_sqlite(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let tables = read_tables(path)?;
        serde_json::to_value(SqliteView { tables })
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The SQLite plugin's presentation half.
#[derive(Debug, Default)]
pub struct SqlitePresentation;

impl PluginPresentation for SqlitePresentation {
    fn name(&self) -> &'static str {
        "sqlite"
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: SqliteView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };

        if view.tables.is_empty() {
            return vec!["no tables".to_owned()];
        }

        let mut lines = Vec::new();
        for (index, table) in view.tables.iter().enumerate() {
            if index > 0 {
                lines.push(String::new());
            }
            lines.push(format!("{} ({} rows)", table.name, table.row_count));
            lines.extend(present_table(table));
            if table.truncated {
                lines.push("… (truncated)".to_owned());
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_ROWS, SqliteCore, SqlitePresentation, SqliteTable, SqliteView};
    use plugin_api::{PluginCore, PluginPresentation};
    use rusqlite::Connection;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-sqlite-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// Writes a real SQLite database at `path` with a single `people`
    /// table (`name`, `age`) holding one row per entry of `names`/`ages`.
    fn write_test_database(path: &std::path::Path, names: &[&str], ages: &[i64]) {
        let conn = Connection::open(path).unwrap();
        conn.execute(
            "CREATE TABLE people (name TEXT NOT NULL, age INTEGER NOT NULL)",
            [],
        )
        .unwrap();
        for (name, age) in names.iter().zip(ages) {
            conn.execute(
                "INSERT INTO people (name, age) VALUES (?1, ?2)",
                rusqlite::params![name, age],
            )
            .unwrap();
        }
    }

    #[test]
    fn sniffs_the_sqlite_magic_header() {
        assert!(SqliteCore.sniff(b"SQLite format 3\0rest of header"));
        assert!(!SqliteCore.sniff(b"not a sqlite file"));
        assert!(!SqliteCore.sniff(b""));
    }

    #[test]
    fn views_a_real_database_and_lists_its_tables() {
        let path = unique_temp_file("people.sqlite");
        write_test_database(&path, &["Alice", "Bob"], &[30, 25]);

        let data = SqliteCore.view(&path).unwrap();
        let view: SqliteView = serde_json::from_value(data).unwrap();

        assert_eq!(view.tables.len(), 1);
        let table = &view.tables[0];
        assert_eq!(table.name, "people");
        assert!(table.schema.contains("CREATE TABLE people"));
        assert_eq!(table.headers, vec!["name".to_owned(), "age".to_owned()]);
        assert_eq!(
            table.rows,
            vec![
                vec!["Alice".to_owned(), "30".to_owned()],
                vec!["Bob".to_owned(), "25".to_owned()],
            ]
        );
        assert_eq!(table.row_count, 2);
        assert!(!table.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn excludes_sqlites_own_internal_tables() {
        let path = unique_temp_file("autoincrement.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "CREATE TABLE items (id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO items (label) VALUES ('first')", [])
            .unwrap();
        drop(conn);

        let data = SqliteCore.view(&path).unwrap();
        let view: SqliteView = serde_json::from_value(data).unwrap();

        assert_eq!(view.tables.len(), 1);
        assert_eq!(view.tables[0].name, "items");

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_rows_of_a_table_larger_than_the_view_limit() {
        let path = unique_temp_file("large.sqlite");
        let names = vec!["row"; MAX_ROWS + 10];
        let ages = vec![1i64; MAX_ROWS + 10];
        write_test_database(&path, &names, &ages);

        let data = SqliteCore.view(&path).unwrap();
        let view: SqliteView = serde_json::from_value(data).unwrap();

        let table = &view.tables[0];
        assert_eq!(table.row_count, MAX_ROWS + 10);
        assert_eq!(table.rows.len(), MAX_ROWS);
        assert!(table.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_tables_with_aligned_columns() {
        let data = serde_json::to_value(SqliteView {
            tables: vec![SqliteTable {
                name: "people".to_owned(),
                schema: "CREATE TABLE people (name TEXT, age INTEGER)".to_owned(),
                headers: vec!["name".to_owned(), "age".to_owned()],
                rows: vec![
                    vec!["Alice".to_owned(), "30".to_owned()],
                    vec!["Bob".to_owned(), "25".to_owned()],
                ],
                row_count: 2,
                truncated: false,
            }],
        })
        .unwrap();

        let lines = SqlitePresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "people (2 rows)",
                "name  | age",
                "------+----",
                "Alice | 30 ",
                "Bob   | 25 ",
            ]
        );
    }
}
