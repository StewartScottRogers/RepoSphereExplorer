//! CSV/TSV file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// Number of records a delimiter guess is checked against.
const SNIFF_RECORD_SAMPLE: usize = 3;

/// A parsed table: a header row and the data rows beneath it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvTable {
    /// The first row, treated as column headers.
    pub headers: Vec<String>,
    /// Every row after the header, in file order.
    pub rows: Vec<Vec<String>>,
}

/// View data produced by [`CsvCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsvView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary), shown
    /// as a fallback when `parsed` is `None`.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// The file parsed as a delimited table, or `None` if it doesn't parse
    /// as one.
    pub parsed: Option<CsvTable>,
}

/// Reads up to [`SNIFF_RECORD_SAMPLE`] records from `prefix` split on
/// `delimiter`, stopping at the first read error (expected once a bounded
/// prefix cuts a record short).
fn record_field_counts(prefix: &[u8], delimiter: u8) -> Vec<usize> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(prefix);
    let mut counts = Vec::new();
    for result in reader.records().take(SNIFF_RECORD_SAMPLE) {
        match result {
            Ok(record) => counts.push(record.len()),
            Err(_) => break,
        }
    }
    counts
}

/// Whether `prefix` looks like data delimited by `delimiter`: at least two
/// records were read, every one has the same field count, and that count is
/// more than one (a single column can't be told apart from plain text).
fn looks_like_delimited(prefix: &[u8], delimiter: u8) -> bool {
    let counts = record_field_counts(prefix, delimiter);
    counts.len() >= 2 && counts[0] > 1 && counts.iter().all(|&count| count == counts[0])
}

/// Whether `prefix` looks like CSV or TSV: consistently comma- or
/// tab-delimited rows — a structural check, not a fixed marker, so it
/// doesn't overlap with any sibling plugin's markers. Both delimiters are
/// tried since sniffing has no access to the file's extension.
fn looks_like_csv(prefix: &[u8]) -> bool {
    looks_like_delimited(prefix, b',') || looks_like_delimited(prefix, b'\t')
}

/// The delimiter to parse `path` with: its extension if that names one of
/// this plugin's two formats, falling back to content sniffing (matching
/// [`looks_like_csv`]'s preference for comma) for an extensionless file.
fn delimiter_for(path: &Path, content: &[u8]) -> u8 {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("tsv") => b'\t',
        Some(ext) if ext.eq_ignore_ascii_case("csv") => b',',
        _ if looks_like_delimited(content, b'\t') => b'\t',
        _ => b',',
    }
}

/// Parses `content` as a delimited table, or returns `None` if any record in
/// it fails to parse.
fn parse_table(content: &str, delimiter: u8) -> Option<CsvTable> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_reader(content.as_bytes());
    let headers = reader
        .headers()
        .ok()?
        .iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    for result in reader.records() {
        let record = result.ok()?;
        rows.push(record.iter().map(str::to_owned).collect());
    }
    Some(CsvTable { headers, rows })
}

/// The number of characters `text` displays as (not its byte length, so
/// column widths line up for multi-byte UTF-8 content).
fn display_width(text: &str) -> usize {
    text.chars().count()
}

/// Renders `table` as an aligned, spreadsheet-like grid: a header row, a
/// rule beneath it, then one line per data row.
fn present_table(table: &CsvTable) -> Vec<String> {
    let widest_row = table.rows.iter().map(Vec::len).max().unwrap_or(0);
    let column_count = table.headers.len().max(widest_row);

    let mut widths = vec![0usize; column_count];
    for (index, header) in table.headers.iter().enumerate() {
        widths[index] = widths[index].max(display_width(header));
    }
    for row in &table.rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(display_width(cell));
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

/// The CSV/TSV plugin's core half.
#[derive(Debug, Default)]
pub struct CsvCore;

impl PluginCore for CsvCore {
    fn name(&self) -> &'static str {
        "csv"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_csv(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let delimiter = delimiter_for(path, slice);
        let parsed = parse_table(&content, delimiter);
        let view = CsvView {
            content,
            truncated,
            parsed,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The CSV/TSV plugin's presentation half.
#[derive(Debug, Default)]
pub struct CsvPresentation;

impl PluginPresentation for CsvPresentation {
    fn name(&self) -> &'static str {
        "csv"
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: CsvView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = if let Some(table) = &view.parsed {
            present_table(table)
        } else {
            let mut lines = vec!["could not parse as CSV/TSV; showing raw content".to_owned()];
            lines.extend(view.content.lines().map(str::to_owned));
            lines
        };
        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{CsvCore, CsvPresentation, CsvTable, CsvView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-csv-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_consistent_comma_and_tab_delimited_rows_as_csv() {
        assert!(CsvCore.sniff(b"name,age\nAlice,30\nBob,25\n"));
        assert!(CsvCore.sniff(b"name\tage\nAlice\t30\nBob\t25\n"));
    }

    #[test]
    fn does_not_sniff_plain_text_or_a_single_column_as_csv() {
        assert!(!CsvCore.sniff(b"just a regular line of text\n"));
        assert!(!CsvCore.sniff(b"one\ntwo\nthree\n"));
        assert!(!CsvCore.sniff(b"name,age\n"));
        assert!(!CsvCore.sniff(b"{\"a\": 1}\n"));
        assert!(!CsvCore.sniff(b""));
        assert!(!CsvCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn does_not_sniff_rows_with_inconsistent_field_counts_as_csv() {
        assert!(!CsvCore.sniff(b"a,b,c\nd,e\n"));
    }

    #[test]
    fn views_a_real_csv_file_and_parses_it_as_a_table() {
        let path = unique_temp_file("people.csv");
        std::fs::write(&path, "name,age\nAlice,30\nBob,25\n").unwrap();

        let data = CsvCore.view(&path).unwrap();
        let view: CsvView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(
            view.parsed,
            Some(CsvTable {
                headers: vec!["name".to_owned(), "age".to_owned()],
                rows: vec![
                    vec!["Alice".to_owned(), "30".to_owned()],
                    vec!["Bob".to_owned(), "25".to_owned()],
                ],
            })
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_real_tsv_file_by_extension() {
        let path = unique_temp_file("people.tsv");
        std::fs::write(&path, "name\tage\nAlice\t30\nBob\t25\n").unwrap();

        let data = CsvCore.view(&path).unwrap();
        let view: CsvView = serde_json::from_value(data).unwrap();

        assert_eq!(
            view.parsed,
            Some(CsvTable {
                headers: vec!["name".to_owned(), "age".to_owned()],
                rows: vec![
                    vec!["Alice".to_owned(), "30".to_owned()],
                    vec!["Bob".to_owned(), "25".to_owned()],
                ],
            })
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_content_of_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.csv");
        let mut content = "col\n".to_owned();
        while content.len() <= MAX_VIEW_BYTES {
            content.push_str("value\n");
        }
        std::fs::write(&path, &content).unwrap();

        let data = CsvCore.view(&path).unwrap();
        let view: CsvView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_a_table_with_aligned_columns() {
        let data = serde_json::to_value(CsvView {
            content: String::new(),
            truncated: false,
            parsed: Some(CsvTable {
                headers: vec!["name".to_owned(), "age".to_owned()],
                rows: vec![
                    vec!["Alice".to_owned(), "30".to_owned()],
                    vec!["Bob".to_owned(), "25".to_owned()],
                ],
            }),
        })
        .unwrap();

        let lines = CsvPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["name  | age", "------+----", "Alice | 30 ", "Bob   | 25 ",]
        );
    }

    #[test]
    fn presents_raw_content_when_not_parseable() {
        let data = serde_json::to_value(CsvView {
            content: "a,\"b\nc,d".to_owned(),
            truncated: true,
            parsed: None,
        })
        .unwrap();

        let lines = CsvPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "could not parse as CSV/TSV; showing raw content",
                "a,\"b",
                "c,d",
                "… (truncated)",
            ]
        );
    }
}
