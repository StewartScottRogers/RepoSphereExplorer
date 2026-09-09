//! Regenerates `samples/parquet/inventory.parquet`.
//!
//! Run with:
//! `cargo run -p plugin-parquet --example generate_fixture -- samples/parquet/inventory.parquet`

use parquet::column::writer::ColumnWriter;
use parquet::data_type::ByteArray;
use parquet::file::properties::WriterProperties;
use parquet::file::writer::{SerializedFileWriter, SerializedRowGroupWriter};
use parquet::schema::parser::parse_message_type;
use std::env;
use std::fs::File;
use std::sync::Arc;

/// Rows written, kept above `plugin_parquet::MAX_ROWS` (200) so the fixture
/// exercises truncation itself rather than leaving that to a unit test.
const ROW_COUNT: i64 = 250;

/// The schema written to the fixture: one column of each of the types the
/// preview renders differently, plus a nullable one.
const SCHEMA: &str = "
    message schema {
        REQUIRED BYTE_ARRAY item (UTF8);
        REQUIRED INT64 quantity;
        REQUIRED DOUBLE unit_price;
        REQUIRED BOOLEAN in_stock;
        REQUIRED INT64 restocked_at (TIMESTAMP_MILLIS);
        OPTIONAL BYTE_ARRAY notes (UTF8);
    }
";

/// Whether `row`'s `notes` cell carries a value; every fifth row leaves it
/// unset, so the fixture carries a column with missing values.
fn has_note(row: i64) -> bool {
    row % 5 != 4
}

/// Opens `row_group_writer`'s next column, hands its typed writer to
/// `write`, then closes it.
fn write_column(
    row_group_writer: &mut SerializedRowGroupWriter<'_, File>,
    what: &str,
    write: impl FnOnce(&mut ColumnWriter),
) {
    let mut column_writer = row_group_writer
        .next_column()
        .unwrap()
        .unwrap_or_else(|| panic!("expected a {what} column"));
    write(column_writer.untyped());
    column_writer.close().unwrap();
}

fn write_items(row_group_writer: &mut SerializedRowGroupWriter<'_, File>) {
    let items: Vec<ByteArray> = (0..ROW_COUNT)
        .map(|row| ByteArray::from(format!("item-{row:03}").as_str()))
        .collect();
    write_column(row_group_writer, "item", |writer| match writer {
        ColumnWriter::ByteArrayColumnWriter(typed) => {
            typed.write_batch(&items, None, None).unwrap();
        }
        _ => panic!("expected a byte array column"),
    });
}

fn write_quantities(row_group_writer: &mut SerializedRowGroupWriter<'_, File>) {
    let quantities: Vec<i64> = (0..ROW_COUNT).map(|row| row % 40).collect();
    write_column(row_group_writer, "quantity", |writer| match writer {
        ColumnWriter::Int64ColumnWriter(typed) => {
            typed.write_batch(&quantities, None, None).unwrap();
        }
        _ => panic!("expected an int64 column"),
    });
}

fn write_prices(row_group_writer: &mut SerializedRowGroupWriter<'_, File>) {
    let prices: Vec<f64> = (0..ROW_COUNT)
        .map(|row| 1.5 + f64::from(i32::try_from(row).unwrap()) * 0.25)
        .collect();
    write_column(row_group_writer, "unit_price", |writer| match writer {
        ColumnWriter::DoubleColumnWriter(typed) => {
            typed.write_batch(&prices, None, None).unwrap();
        }
        _ => panic!("expected a double column"),
    });
}

fn write_in_stock(row_group_writer: &mut SerializedRowGroupWriter<'_, File>) {
    let in_stock: Vec<bool> = (0..ROW_COUNT).map(|row| row % 3 != 0).collect();
    write_column(row_group_writer, "in_stock", |writer| match writer {
        ColumnWriter::BoolColumnWriter(typed) => {
            typed.write_batch(&in_stock, None, None).unwrap();
        }
        _ => panic!("expected a bool column"),
    });
}

fn write_restocked_at(row_group_writer: &mut SerializedRowGroupWriter<'_, File>) {
    // Milliseconds since the epoch, one day apart, starting 2024-01-01.
    let restocked_at: Vec<i64> = (0..ROW_COUNT)
        .map(|row| 1_704_067_200_000 + row * 86_400_000)
        .collect();
    write_column(row_group_writer, "restocked_at", |writer| match writer {
        ColumnWriter::Int64ColumnWriter(typed) => {
            typed.write_batch(&restocked_at, None, None).unwrap();
        }
        _ => panic!("expected an int64 column"),
    });
}

fn write_notes(row_group_writer: &mut SerializedRowGroupWriter<'_, File>) {
    let mut note_values = Vec::new();
    let mut note_def_levels = Vec::with_capacity(usize::try_from(ROW_COUNT).unwrap());
    for row in 0..ROW_COUNT {
        if has_note(row) {
            note_def_levels.push(1);
            note_values.push(ByteArray::from(format!("checked by qa-{row}").as_str()));
        } else {
            note_def_levels.push(0);
        }
    }
    write_column(row_group_writer, "notes", |writer| match writer {
        ColumnWriter::ByteArrayColumnWriter(typed) => {
            typed
                .write_batch(&note_values, Some(&note_def_levels), None)
                .unwrap();
        }
        _ => panic!("expected a byte array column"),
    });
}

fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: generate_fixture <output path>");

    let schema = Arc::new(parse_message_type(SCHEMA).expect("valid schema"));
    let props = Arc::new(WriterProperties::builder().build());
    let file = File::create(&path).expect("create output file");
    let mut writer = SerializedFileWriter::new(file, schema, props).expect("create writer");
    let mut row_group_writer = writer.next_row_group().expect("open row group");

    write_items(&mut row_group_writer);
    write_quantities(&mut row_group_writer);
    write_prices(&mut row_group_writer);
    write_in_stock(&mut row_group_writer);
    write_restocked_at(&mut row_group_writer);
    write_notes(&mut row_group_writer);

    row_group_writer.close().unwrap();
    writer.close().unwrap();
}
