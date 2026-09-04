//! Apache Avro file type plugin: core and presentation halves.

use apache_avro::Reader;
use apache_avro::schema::Schema;
use apache_avro::types::Value as AvroValue;
use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::File;
use std::io;
use std::path::Path;

/// Maximum number of rows read into the view; files with more are truncated.
const MAX_ROWS: usize = 200;

/// A parsed table: column names and the sampled rows beneath them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvroTable {
    /// The column names, in schema order.
    pub headers: Vec<String>,
    /// Up to [`MAX_ROWS`] rows, each cell rendered to its display text.
    pub rows: Vec<Vec<String>>,
}

/// View data produced by [`AvroCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvroView {
    /// Total number of records in the file.
    pub row_count: usize,
    /// Whether `table.rows` was cut off at [`MAX_ROWS`].
    pub truncated: bool,
    /// The file's schema and its first records.
    pub table: AvroTable,
}

/// Whether `prefix` opens with the Avro object container file's magic
/// number (`Obj` followed by the format version byte `0x01`) — the only
/// marker reserved for this format, not used by any sibling plugin.
fn looks_like_avro(prefix: &[u8]) -> bool {
    prefix.starts_with(b"Obj\x01")
}

/// The writer schema's top-level field names, in declaration order, or a
/// single `value` column when the schema isn't a record (Avro allows any
/// type, not just records, at the top level).
fn schema_headers(schema: &Schema) -> Vec<String> {
    match schema {
        Schema::Record(record) => record
            .fields
            .iter()
            .map(|field| field.name.clone())
            .collect(),
        _ => vec!["value".to_owned()],
    }
}

/// Renders a single Avro value as display text.
fn format_value(value: &AvroValue) -> String {
    match value {
        AvroValue::Null => "null".to_owned(),
        AvroValue::Boolean(bool) => bool.to_string(),
        AvroValue::Int(int) => int.to_string(),
        AvroValue::Long(long) => long.to_string(),
        AvroValue::Float(float) => float.to_string(),
        AvroValue::Double(double) => double.to_string(),
        AvroValue::String(string) | AvroValue::Enum(_, string) => string.clone(),
        AvroValue::Bytes(bytes) | AvroValue::Fixed(_, bytes) => format!("{bytes:?}"),
        AvroValue::Union(_, inner) => format_value(inner),
        AvroValue::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(format_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        AvroValue::Map(entries) => format!(
            "{{{}}}",
            entries
                .iter()
                .map(|(key, value)| format!("{key}: {}", format_value(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        AvroValue::Record(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|(key, value)| format!("{key}: {}", format_value(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => format!("{other:?}"),
    }
}

/// Renders a single record as one cell per header, in header order.
fn record_to_row(value: &AvroValue, headers: &[String]) -> Vec<String> {
    match value {
        AvroValue::Record(fields) => headers
            .iter()
            .map(|header| {
                fields
                    .iter()
                    .find(|(name, _)| name == header)
                    .map_or_else(String::new, |(_, value)| format_value(value))
            })
            .collect(),
        other => vec![format_value(other)],
    }
}

/// Reads `path`'s schema and every record, sampling up to [`MAX_ROWS`] rows
/// into the returned table while still counting every record in the file.
fn read_table(path: &Path) -> io::Result<(usize, AvroTable)> {
    let file = File::open(path)?;
    let reader = Reader::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    let headers = schema_headers(reader.writer_schema());

    let mut row_count = 0usize;
    let mut rows = Vec::new();
    for record in reader {
        let record =
            record.map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
        row_count += 1;
        if rows.len() < MAX_ROWS {
            rows.push(record_to_row(&record, &headers));
        }
    }

    Ok((row_count, AvroTable { headers, rows }))
}

/// Renders `table` as an aligned, spreadsheet-like grid: a header row, a
/// rule beneath it, then one line per sampled row.
fn present_table(table: &AvroTable) -> Vec<String> {
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

/// The Apache Avro plugin's core half.
#[derive(Debug, Default)]
pub struct AvroCore;

impl PluginCore for AvroCore {
    fn name(&self) -> &'static str {
        "avro"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_avro(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let (row_count, table) = read_table(path)?;
        let truncated = table.rows.len() < row_count;
        let view = AvroView {
            row_count,
            truncated,
            table,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Apache Avro plugin's presentation half.
#[derive(Debug, Default)]
pub struct AvroPresentation;

impl PluginPresentation for AvroPresentation {
    fn name(&self) -> &'static str {
        "avro"
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: AvroView = match serde_json::from_value(data.clone()) {
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
    use super::{AvroCore, AvroPresentation, AvroTable, AvroView, MAX_ROWS};
    use apache_avro::Writer;
    use apache_avro::types::Record;
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-avro-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// Writes a real two-field (`name`, `age`) Avro object container file
    /// with `names` and `ages` as its records.
    fn write_test_avro(path: &std::path::Path, names: &[&str], ages: &[i64]) {
        let raw_schema = r#"
            {
                "type": "record",
                "name": "Person",
                "fields": [
                    {"name": "name", "type": "string"},
                    {"name": "age", "type": "long"}
                ]
            }
        "#;
        let schema = apache_avro::Schema::parse_str(raw_schema).unwrap();
        let file = std::fs::File::create(path).unwrap();
        let mut writer = Writer::new(&schema, file).unwrap();

        for (name, age) in names.iter().zip(ages) {
            let mut record = Record::new(writer.schema()).unwrap();
            record.put("name", *name);
            record.put("age", *age);
            writer.append_value(record).unwrap();
        }

        writer.flush().unwrap();
    }

    #[test]
    fn sniffs_the_avro_magic_number() {
        assert!(AvroCore.sniff(b"Obj\x01rest of header"));
        assert!(!AvroCore.sniff(b"not an avro file"));
        assert!(!AvroCore.sniff(b""));
    }

    #[test]
    fn views_a_real_avro_file_and_parses_it_as_a_table() {
        let path = unique_temp_file("people.avro");
        write_test_avro(&path, &["Alice", "Bob"], &[30, 25]);

        let data = AvroCore.view(&path).unwrap();
        let view: AvroView = serde_json::from_value(data).unwrap();

        assert_eq!(view.row_count, 2);
        assert!(!view.truncated);
        assert_eq!(
            view.table,
            AvroTable {
                headers: vec!["name".to_owned(), "age".to_owned()],
                rows: vec![
                    vec!["Alice".to_owned(), "30".to_owned()],
                    vec!["Bob".to_owned(), "25".to_owned()],
                ],
            }
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_rows_of_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.avro");
        let names = vec!["row"; MAX_ROWS + 10];
        let ages = vec![1i64; MAX_ROWS + 10];
        write_test_avro(&path, &names, &ages);

        let data = AvroCore.view(&path).unwrap();
        let view: AvroView = serde_json::from_value(data).unwrap();

        assert_eq!(view.row_count, MAX_ROWS + 10);
        assert_eq!(view.table.rows.len(), MAX_ROWS);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_a_table_with_aligned_columns() {
        let data = serde_json::to_value(AvroView {
            row_count: 2,
            truncated: false,
            table: AvroTable {
                headers: vec!["name".to_owned(), "age".to_owned()],
                rows: vec![
                    vec!["Alice".to_owned(), "30".to_owned()],
                    vec!["Bob".to_owned(), "25".to_owned()],
                ],
            },
        })
        .unwrap();

        let lines = AvroPresentation.present(&data);

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
}
