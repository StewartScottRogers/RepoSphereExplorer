//! Apache ORC file type plugin: core and presentation halves.
//!
//! The Optimized Row Columnar (ORC) format: rows in stripes, columns
//! compressed within them, and a footer at the end saying what is where.
//! This reads the row count, the schema with every column's type, the
//! stripe count and their row counts, the compression, the writer, and
//! the first rows as a table.

use arrow::array::RecordBatch;
use arrow::datatypes::DataType;
use arrow::util::pretty::pretty_format_batches;
use orc_rust::ArrowReaderBuilder;
use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::File;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["orc"];

/// The magic an ORC file opens with, and repeats in its postscript.
const MAGIC: &[u8] = b"ORC";

/// How many rows are rendered as a table.
const SHOWN_ROWS: usize = 8;

/// One column of the schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    /// Its name.
    pub name: String,
    /// Its type, as Arrow spells it after the reader has mapped it.
    pub kind: String,
    /// Whether it may hold nulls.
    pub nullable: bool,
}

/// View data produced by [`OrcCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrcView {
    /// How many rows the file holds.
    pub rows: u64,
    /// Every column, in order.
    pub columns: Vec<Column>,
    /// How many stripes the rows are cut into.
    pub stripes: usize,
    /// The rows in each stripe, which says how evenly it was cut.
    pub rows_per_stripe: Vec<u64>,
    /// The compression the file was written with.
    pub compression: String,
    /// What wrote it.
    pub writer: String,
    /// The first rows, rendered as a table.
    pub first_rows: Vec<String>,
    /// Columns holding a nested type, which most readers of this format
    /// flatten or refuse.
    pub nested_columns: Vec<String>,
}

/// Whether `prefix` opens like an ORC file.
fn looks_like_it(prefix: &[u8]) -> bool {
    prefix.starts_with(MAGIC)
}

/// Whether `kind` is a type with columns of its own inside it.
fn is_nested(kind: &DataType) -> bool {
    matches!(
        kind,
        DataType::List(_)
            | DataType::LargeList(_)
            | DataType::FixedSizeList(_, _)
            | DataType::Struct(_)
            | DataType::Union(_, _)
            | DataType::Map(_, _)
    )
}

/// Everything [`OrcView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<OrcView> {
    let handle = File::open(path)?;
    let builder = ArrowReaderBuilder::try_new(handle)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let metadata = builder.file_metadata().clone();
    let schema = builder.schema();

    let columns: Vec<Column> = schema
        .fields()
        .iter()
        .map(|field| Column {
            name: field.name().clone(),
            kind: field.data_type().to_string(),
            nullable: field.is_nullable(),
        })
        .collect();
    let nested_columns = schema
        .fields()
        .iter()
        .filter(|field| is_nested(field.data_type()))
        .map(|field| format!("{} ({})", field.name(), field.data_type()))
        .collect();

    let rows_per_stripe: Vec<u64> = metadata
        .stripe_metadatas()
        .iter()
        .map(orc_rust::stripe::StripeMetadata::number_of_rows)
        .collect();

    // Only the first rows are read: the point is to show the shape, and a
    // stripe is tens of thousands of rows.
    let reader = builder.with_batch_size(SHOWN_ROWS).build();
    let first: Option<RecordBatch> = reader
        .into_iter()
        .next()
        .transpose()
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let first_rows = first.map_or_else(Vec::new, |batch| {
        pretty_format_batches(&[batch])
            .map(|table| table.to_string().lines().map(str::to_owned).collect())
            .unwrap_or_default()
    });

    Ok(OrcView {
        rows: metadata.number_of_rows(),
        columns,
        stripes: rows_per_stripe.len(),
        rows_per_stripe,
        compression: metadata
            .compression()
            .map_or_else(|| "none".to_owned(), |said| said.to_string()),
        // ORC writes no free-text writer name; what it records is which
        // version of the format it was written to.
        writer: format!("ORC format version {}", metadata.file_format_version()),
        first_rows,
        nested_columns,
    })
}

/// The Apache ORC plugin's core half.
#[derive(Debug, Default)]
pub struct OrcCore;

impl PluginCore for OrcCore {
    fn name(&self) -> &'static str {
        "orc"
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

/// The Apache ORC plugin's presentation half.
#[derive(Debug, Default)]
pub struct OrcPresentation;

impl PluginPresentation for OrcPresentation {
    fn name(&self) -> &'static str {
        "orc"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "ORC",
            tint: 0x00d2_2128,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: OrcView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "ORC: {} row(s) in {} stripe(s), {} compression",
            view.rows, view.stripes, view.compression
        )];
        lines.push(format!("Written by {}", view.writer));
        if view.rows_per_stripe.len() > 1 {
            lines.push(format!(
                "Rows per stripe: {}",
                view.rows_per_stripe
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        lines.push(format!("{} column(s):", view.columns.len()));
        for column in &view.columns {
            let nullable = if column.nullable { "" } else { ", never null" };
            lines.push(format!("  {} - {}{nullable}", column.name, column.kind));
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
    use super::{OrcCore, OrcPresentation, OrcView, looks_like_it};
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/orc/readings.orc")
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&OrcCore),
            PluginPresentation::extensions(&OrcPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn sniffs_the_magic() {
        assert!(looks_like_it(b"ORC\x11\x00\x00"));
    }

    #[test]
    fn does_not_claim_anything_else() {
        assert!(!looks_like_it(b"PAR1"), "that is Parquet");
        assert!(!looks_like_it(b"ARROW1"));
        assert!(!looks_like_it(b""));
        assert!(!OrcCore.sniff(b"#!/bin/sh\necho hello\n"));
    }

    #[test]
    fn reads_the_rows_the_stripes_and_the_compression() {
        let data = OrcCore.view(&fixture()).unwrap();
        let view: OrcView = serde_json::from_value(data).unwrap();

        assert_eq!(view.rows, 20_000);
        assert!(view.stripes >= 1);
        assert_eq!(view.rows_per_stripe.iter().sum::<u64>(), 20_000);
        assert!(
            view.compression.to_lowercase().starts_with("zlib"),
            "the reader names the block size alongside the algorithm: {}",
            view.compression
        );
        assert!(view.writer.to_lowercase().contains("orc format version"));
    }

    #[test]
    fn reads_the_schema_with_types_and_nullability() {
        let data = OrcCore.view(&fixture()).unwrap();
        let view: OrcView = serde_json::from_value(data).unwrap();

        assert_eq!(view.columns.len(), 7);
        assert_eq!(view.columns[0].name, "id");
        assert!(
            view.columns
                .iter()
                .any(|column| column.kind.contains("Timestamp"))
        );
        assert!(
            view.columns
                .iter()
                .any(|column| column.kind.contains("Boolean"))
        );
    }

    #[test]
    fn names_the_nested_columns() {
        let data = OrcCore.view(&fixture()).unwrap();
        let view: OrcView = serde_json::from_value(data).unwrap();

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
        let data = OrcCore.view(&fixture()).unwrap();
        let view: OrcView = serde_json::from_value(data).unwrap();

        assert!(!view.first_rows.is_empty());
        assert!(
            view.first_rows.len() < 20,
            "eight rows and a border, not twenty thousand"
        );
        assert!(view.first_rows.iter().any(|row| row.contains("station")));
    }

    #[test]
    fn presents_the_nested_warning_with_its_reason() {
        let data = OrcCore.view(&fixture()).unwrap();

        let lines = OrcPresentation.present(&data);

        assert!(lines[0].starts_with("ORC: 20000 row(s)"));
        assert!(lines.iter().any(|line| line.contains("flatten or drop")));
    }

    #[test]
    fn a_file_that_is_not_orc_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.orc");
        std::fs::write(&path, b"ORC and then nothing of the sort at all").unwrap();

        assert!(OrcCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
