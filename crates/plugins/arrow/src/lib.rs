//! Apache Arrow file type plugin: core and presentation halves.
//!
//! An Arrow file is a schema followed by record batches, each a block of
//! columns laid out for reading without parsing. This reads the schema
//! with every field's type and whether it may be null, how many batches
//! and rows there are, the dictionary-encoded columns, the custom
//! metadata, and the first rows as a table.

use arrow::array::RecordBatch;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::ipc::reader::FileReader;
use arrow::util::pretty::pretty_format_batches;
use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::File;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["arrow", "feather", "ipc"];

/// How many rows are rendered as a table.
const SHOWN_ROWS: usize = 8;

/// The magic an Arrow file opens and closes with.
const MAGIC: &[u8] = b"ARROW1";

/// The marker every message in the streaming form opens with.
const CONTINUATION: &[u8] = &[0xff, 0xff, 0xff, 0xff];

/// One column of the schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    /// Its name.
    pub name: String,
    /// Its type, as Arrow spells it.
    pub kind: String,
    /// Whether it may hold nulls.
    pub nullable: bool,
    /// Whether its values are held once in a dictionary and referred to
    /// by index, which is how Arrow stores a column of few distinct
    /// values without repeating them.
    pub dictionary_encoded: bool,
}

/// View data produced by [`ArrowCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrowView {
    /// Every column, in order.
    pub columns: Vec<Column>,
    /// How many record batches the file holds.
    pub batches: usize,
    /// How many rows across all of them.
    pub rows: usize,
    /// The rows in each batch, which says how evenly the file is cut.
    pub rows_per_batch: Vec<usize>,
    /// The schema's custom metadata, as `key = value`.
    pub metadata: Vec<String>,
    /// The first rows, rendered as a table.
    pub first_rows: Vec<String>,
    /// Columns holding a nested type, which most readers of this format
    /// flatten or refuse.
    pub nested_columns: Vec<String>,
}

/// Whether `prefix` opens like an Arrow file or a streaming message.
fn looks_like_it(prefix: &[u8]) -> bool {
    prefix.starts_with(MAGIC) || prefix.starts_with(CONTINUATION)
}

/// Whether `kind` is a type with columns of its own inside it.
fn is_nested(kind: &DataType) -> bool {
    matches!(
        kind,
        DataType::List(_)
            | DataType::LargeList(_)
            | DataType::ListView(_)
            | DataType::LargeListView(_)
            | DataType::FixedSizeList(_, _)
            | DataType::Struct(_)
            | DataType::Union(_, _)
            | DataType::Map(_, _)
            | DataType::RunEndEncoded(_, _)
    )
}

/// The column `field` describes.
fn column_of(field: &Field) -> Column {
    Column {
        name: field.name().clone(),
        kind: field.data_type().to_string(),
        nullable: field.is_nullable(),
        dictionary_encoded: matches!(field.data_type(), DataType::Dictionary(_, _)),
    }
}

/// The schema's own metadata, as `key = value`, in a settled order.
fn metadata_of(schema: &Schema) -> Vec<String> {
    let mut pairs: Vec<String> = schema
        .metadata()
        .iter()
        .map(|(key, value)| format!("{key} = {value}"))
        .collect();
    pairs.sort();
    pairs
}

/// Everything [`ArrowView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<ArrowView> {
    let handle = File::open(path)?;
    let reader = FileReader::try_new(handle, None)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let schema = reader.schema();

    let columns: Vec<Column> = schema
        .fields()
        .iter()
        .map(|field| column_of(field))
        .collect();
    let nested_columns = schema
        .fields()
        .iter()
        .filter(|field| is_nested(field.data_type()))
        .map(|field| format!("{} ({})", field.name(), field.data_type()))
        .collect();
    let metadata = metadata_of(&schema);

    let mut batches: Vec<RecordBatch> = Vec::new();
    let mut rows_per_batch = Vec::new();
    for batch in reader {
        let batch = batch.map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        rows_per_batch.push(batch.num_rows());
        batches.push(batch);
    }

    // The table is rendered from the head of the first batch only: the
    // point is to show the shape, and a whole batch is three hundred rows.
    let first_rows = match batches.first() {
        Some(batch) => {
            let head = batch.slice(0, batch.num_rows().min(SHOWN_ROWS));
            pretty_format_batches(&[head])
                .map(|table| table.to_string().lines().map(str::to_owned).collect())
                .unwrap_or_default()
        }
        None => Vec::new(),
    };

    Ok(ArrowView {
        columns,
        batches: rows_per_batch.len(),
        rows: rows_per_batch.iter().sum(),
        rows_per_batch,
        metadata,
        first_rows,
        nested_columns,
    })
}

/// The Apache Arrow plugin's core half.
#[derive(Debug, Default)]
pub struct ArrowCore;

impl PluginCore for ArrowCore {
    fn name(&self) -> &'static str {
        "arrow"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_it(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Apache Arrow plugin's presentation half.
#[derive(Debug, Default)]
pub struct ArrowPresentation;

impl PluginPresentation for ArrowPresentation {
    fn name(&self) -> &'static str {
        "arrow"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "ARR",
            tint: 0x0016_7dff,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: ArrowView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "Arrow: {} row(s) in {} record batch(es)",
            view.rows, view.batches
        )];
        if view.rows_per_batch.len() > 1 {
            lines.push(format!(
                "Rows per batch: {}",
                view.rows_per_batch
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        lines.push(format!("{} column(s):", view.columns.len()));
        for column in &view.columns {
            let nullable = if column.nullable { "" } else { ", never null" };
            let dictionary = if column.dictionary_encoded {
                ", dictionary encoded"
            } else {
                ""
            };
            lines.push(format!(
                "  {} - {}{nullable}{dictionary}",
                column.name, column.kind
            ));
        }
        if !view.metadata.is_empty() {
            lines.push("Custom metadata:".to_owned());
            for pair in &view.metadata {
                lines.push(format!("  {pair}"));
            }
        }
        if !view.first_rows.is_empty() {
            lines.push("First rows:".to_owned());
            for row in &view.first_rows {
                lines.push(format!("  {row}"));
            }
        }
        if !view.nested_columns.is_empty() {
            lines.push("Nested, so a reader that wants a flat table has to".to_owned());
            lines.push("flatten or drop these:".to_owned());
            for column in &view.nested_columns {
                lines.push(format!("  {column}"));
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{ArrowCore, ArrowPresentation, ArrowView, EXTENSIONS, looks_like_it};
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/arrow/readings.arrow")
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&ArrowCore),
            PluginPresentation::extensions(&ArrowPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn sniffs_the_file_magic_and_the_streaming_marker() {
        assert!(looks_like_it(b"ARROW1\x00\x00"));
        assert!(
            looks_like_it(&[0xff, 0xff, 0xff, 0xff, 0x10, 0x00]),
            "the streaming form has no magic, only the continuation marker"
        );
    }

    #[test]
    fn does_not_claim_anything_else() {
        assert!(!looks_like_it(b"PAR1"), "that is Parquet");
        assert!(!looks_like_it(b"ORC\x00"));
        assert!(!looks_like_it(b""));
        assert!(!ArrowCore.sniff(b"#!/bin/sh\necho hello\n"));
    }

    #[test]
    fn it_claims_the_extensions_the_format_is_written_under() {
        assert!(EXTENSIONS.contains(&"arrow"));
        assert!(
            EXTENSIONS.contains(&"feather"),
            "version 2 of Feather is this format under another name"
        );
    }

    #[test]
    fn reads_the_schema_with_types_and_nullability() {
        let data = ArrowCore.view(&fixture()).unwrap();
        let view: ArrowView = serde_json::from_value(data).unwrap();

        assert_eq!(view.columns.len(), 7);
        assert_eq!(view.columns[0].name, "id");
        assert!(
            !view.columns[0].nullable,
            "the identifier column is declared not nullable"
        );
        assert!(
            view.columns[1].dictionary_encoded,
            "station is dictionary encoded"
        );
        assert!(!view.columns[2].dictionary_encoded);
    }

    #[test]
    fn counts_the_batches_and_the_rows_in_each() {
        let data = ArrowCore.view(&fixture()).unwrap();
        let view: ArrowView = serde_json::from_value(data).unwrap();

        assert_eq!(view.batches, 3);
        assert_eq!(view.rows, 900);
        assert_eq!(view.rows_per_batch, vec![300, 300, 300]);
    }

    #[test]
    fn reads_the_custom_metadata() {
        let data = ArrowCore.view(&fixture()).unwrap();
        let view: ArrowView = serde_json::from_value(data).unwrap();

        assert_eq!(view.metadata.len(), 2);
        assert!(
            view.metadata
                .iter()
                .any(|pair| pair.starts_with("written_by = "))
        );
    }

    #[test]
    fn names_the_nested_columns() {
        let data = ArrowCore.view(&fixture()).unwrap();
        let view: ArrowView = serde_json::from_value(data).unwrap();

        assert_eq!(view.nested_columns.len(), 2, "the struct and the list");
        assert!(
            view.nested_columns
                .iter()
                .any(|said| said.starts_with("sensor"))
        );
        assert!(
            view.nested_columns
                .iter()
                .any(|said| said.starts_with("tags"))
        );
    }

    #[test]
    fn shows_the_first_rows_and_no_more() {
        let data = ArrowCore.view(&fixture()).unwrap();
        let view: ArrowView = serde_json::from_value(data).unwrap();

        assert!(!view.first_rows.is_empty());
        assert!(
            view.first_rows.len() < 20,
            "eight rows and a border, not three hundred"
        );
        assert!(view.first_rows.iter().any(|row| row.contains("station")));
    }

    #[test]
    fn presents_the_nested_warning_with_its_reason() {
        let data = ArrowCore.view(&fixture()).unwrap();

        let lines = ArrowPresentation.present(&data);

        assert_eq!(lines[0], "Arrow: 900 row(s) in 3 record batch(es)");
        assert!(lines.iter().any(|line| line.contains("flatten or drop")));
        assert!(lines.iter().any(|line| line.contains("dictionary encoded")));
    }

    #[test]
    fn a_file_that_is_not_arrow_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.arrow");
        std::fs::write(&path, b"ARROW1\x00\x00 and then nothing of the sort").unwrap();

        assert!(ArrowCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
