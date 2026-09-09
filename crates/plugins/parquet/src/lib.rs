//! Parquet file type plugin: core and presentation halves.

use parquet::file::reader::{FileReader, SerializedFileReader};
use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::File;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["parquet"];

/// Maximum number of rows read into the view; files with more are truncated.
const MAX_ROWS: usize = 200;

/// A parsed table: column names and the sampled rows beneath them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParquetTable {
    /// The column names, in schema order.
    pub headers: Vec<String>,
    /// Up to [`MAX_ROWS`] rows, each cell rendered to its display text.
    pub rows: Vec<Vec<String>>,
}

/// View data produced by [`ParquetCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParquetView {
    /// Total number of rows in the file, per its footer metadata.
    pub row_count: usize,
    /// Whether `table.rows` was cut off at [`MAX_ROWS`].
    pub truncated: bool,
    /// The file's schema and its first rows.
    pub table: ParquetTable,
}

/// Whether `prefix` opens with Parquet's `PAR1` magic number, the format's
/// only reserved marker — Parquet repeats it at the end of the file too,
/// but `sniff` only ever sees a bounded prefix from the start.
fn looks_like_parquet(prefix: &[u8]) -> bool {
    prefix.starts_with(b"PAR1")
}

/// Reads `path`'s schema and up to [`MAX_ROWS`] rows as a table.
fn read_table(path: &Path) -> io::Result<(usize, ParquetTable)> {
    let file = File::open(path)?;
    let reader = SerializedFileReader::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;

    let metadata = reader.metadata().file_metadata();
    let row_count = usize::try_from(metadata.num_rows()).unwrap_or(0);
    let headers = metadata
        .schema()
        .get_fields()
        .iter()
        .map(|field| field.name().to_owned())
        .collect::<Vec<_>>();

    let row_iter = reader
        .get_row_iter(None)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    let mut rows = Vec::new();
    for record in row_iter.take(MAX_ROWS) {
        let row =
            record.map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
        rows.push(
            row.get_column_iter()
                .map(|(_, field)| field.to_string())
                .collect::<Vec<_>>(),
        );
    }

    Ok((row_count, ParquetTable { headers, rows }))
}

/// Renders `table` as an aligned, spreadsheet-like grid: a header row, a
/// rule beneath it, then one line per sampled row.
fn present_table(table: &ParquetTable) -> Vec<String> {
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

/// The Parquet plugin's core half.
#[derive(Debug, Default)]
pub struct ParquetCore;

impl PluginCore for ParquetCore {
    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn name(&self) -> &'static str {
        "parquet"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_parquet(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let (row_count, table) = read_table(path)?;
        let truncated = table.rows.len() < row_count;
        let view = ParquetView {
            row_count,
            truncated,
            table,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Parquet plugin's presentation half.
#[derive(Debug, Default)]
pub struct ParquetPresentation;

impl PluginPresentation for ParquetPresentation {
    fn name(&self) -> &'static str {
        "parquet"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "PARQ",
            tint: 0x001d_4ed8,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: ParquetView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!("{} rows", view.row_count)];
        lines.extend(present_table(&view.table));
        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_ROWS, ParquetCore, ParquetPresentation, ParquetTable, ParquetView};
    use parquet::column::writer::ColumnWriter;
    use parquet::data_type::ByteArray;
    use parquet::file::properties::WriterProperties;
    use parquet::file::writer::SerializedFileWriter;
    use parquet::schema::parser::parse_message_type;
    use plugin_api::{PluginCore, PluginPresentation};
    use std::sync::Arc;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-parquet-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// Writes a real two-column (`name`, `age`) Parquet file with `names`
    /// and `ages` as its single row group, using the crate's low-level
    /// column-writer API (the `arrow` feature this plugin doesn't depend
    /// on is not required to produce a valid file).
    fn write_test_parquet(path: &std::path::Path, names: &[&str], ages: &[i64]) {
        let message_type = "
            message schema {
                REQUIRED BYTE_ARRAY name (UTF8);
                REQUIRED INT64 age;
            }
        ";
        let schema = Arc::new(parse_message_type(message_type).unwrap());
        let props = Arc::new(WriterProperties::builder().build());
        let file = std::fs::File::create(path).unwrap();
        let mut writer = SerializedFileWriter::new(file, schema, props).unwrap();
        let mut row_group_writer = writer.next_row_group().unwrap();

        let Some(mut name_writer) = row_group_writer.next_column().unwrap() else {
            panic!("expected a name column");
        };
        match name_writer.untyped() {
            ColumnWriter::ByteArrayColumnWriter(typed) => {
                let values = names
                    .iter()
                    .map(|name| ByteArray::from(*name))
                    .collect::<Vec<_>>();
                typed.write_batch(&values, None, None).unwrap();
            }
            _ => panic!("expected a byte array column"),
        }
        name_writer.close().unwrap();

        let Some(mut age_writer) = row_group_writer.next_column().unwrap() else {
            panic!("expected an age column");
        };
        match age_writer.untyped() {
            ColumnWriter::Int64ColumnWriter(typed) => {
                typed.write_batch(ages, None, None).unwrap();
            }
            _ => panic!("expected an int64 column"),
        }
        age_writer.close().unwrap();

        row_group_writer.close().unwrap();
        writer.close().unwrap();
    }

    #[test]
    fn sniffs_the_parquet_magic_number() {
        assert!(ParquetCore.sniff(b"PAR1rest of header"));
        assert!(!ParquetCore.sniff(b"not a parquet file"));
        assert!(!ParquetCore.sniff(b""));
    }

    #[test]
    fn views_a_real_parquet_file_and_parses_it_as_a_table() {
        let path = unique_temp_file("people.parquet");
        write_test_parquet(&path, &["Alice", "Bob"], &[30, 25]);

        let data = ParquetCore.view(&path).unwrap();
        let view: ParquetView = serde_json::from_value(data).unwrap();

        assert_eq!(view.row_count, 2);
        assert!(!view.truncated);
        assert_eq!(
            view.table,
            ParquetTable {
                headers: vec!["name".to_owned(), "age".to_owned()],
                rows: vec![
                    vec!["\"Alice\"".to_owned(), "30".to_owned()],
                    vec!["\"Bob\"".to_owned(), "25".to_owned()],
                ],
            }
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_rows_of_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.parquet");
        let names = vec!["row"; MAX_ROWS + 10];
        let ages = vec![1i64; MAX_ROWS + 10];
        write_test_parquet(&path, &names, &ages);

        let data = ParquetCore.view(&path).unwrap();
        let view: ParquetView = serde_json::from_value(data).unwrap();

        assert_eq!(view.row_count, MAX_ROWS + 10);
        assert_eq!(view.table.rows.len(), MAX_ROWS);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_a_table_with_aligned_columns() {
        let data = serde_json::to_value(ParquetView {
            row_count: 2,
            truncated: false,
            table: ParquetTable {
                headers: vec!["name".to_owned(), "age".to_owned()],
                rows: vec![
                    vec!["Alice".to_owned(), "30".to_owned()],
                    vec!["Bob".to_owned(), "25".to_owned()],
                ],
            },
        })
        .unwrap();

        let lines = ParquetPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "2 rows",
                "name  | age",
                "------+----",
                "Alice | 30 ",
                "Bob   | 25 ",
            ]
        );
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::ParquetCore),
            plugin_api::PluginPresentation::extensions(&crate::ParquetPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
    /// The fixture in `samples/`, which is where a person checking the
    /// application - and quality assurance after them - meets this plugin.
    fn repository_fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/parquet/pipeline-runs.parquet")
    }

    #[test]
    fn the_repository_fixture_fills_every_column_type_and_truncates() {
        // The fixture this replaced held two string columns and two rows.
        // Nothing in it could tell whether a number, a boolean, a
        // timestamp or a missing value rendered at all, and nothing in it
        // reached MAX_ROWS, so `truncated` was only ever exercised by a
        // file written inside a test.
        let data = ParquetCore.view(&repository_fixture()).unwrap();
        let view: ParquetView = serde_json::from_value(data).unwrap();

        assert_eq!(view.row_count, 264);
        assert!(
            view.truncated,
            "the fixture has more rows than the view shows, which is the point of it"
        );
        assert_eq!(view.table.rows.len(), MAX_ROWS);
        assert_eq!(
            view.table.headers,
            vec![
                "run_id".to_owned(),
                "commit".to_owned(),
                "branch".to_owned(),
                "stage".to_owned(),
                "started_at".to_owned(),
                "duration_seconds".to_owned(),
                "exit_code".to_owned(),
                "passed".to_owned(),
                "runner".to_owned(),
            ]
        );

        let first = view.table.rows.first().expect("the view holds rows");
        assert_eq!(first.len(), view.table.headers.len());
        assert!(
            first[4].starts_with("2026-08-01"),
            "a timestamp column should render as a date, not as a count of milliseconds: {}",
            first[4]
        );

        // The runner column is empty for a run that never got a machine,
        // so the fixture proves a missing value renders as something.
        assert!(
            view.table.rows.iter().any(|row| row[8] == "null"),
            "no row has a missing runner, so nothing shows how a null cell reads"
        );
    }
}
