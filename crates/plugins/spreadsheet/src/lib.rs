//! Spreadsheet file type plugin: core and presentation halves.
//!
//! Covers `.xlsx` and `.ods` as one plugin, per the issue's direction that
//! this document family renders as a single tabular-grid view rather than a
//! plugin per container format.

use calamine::{Data, Ods, Reader, Xlsx};
use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::io::Cursor;
use std::path::Path;

/// Maximum number of rows read into the view per sheet; sheets with more
/// are truncated.
const MAX_ROWS: usize = 200;

/// The XLSX-specific internal part path, unique to OOXML spreadsheets (as
/// opposed to `word/document.xml` for a word-processing document or
/// `ppt/presentation.xml` for a presentation) — a marker not used by any
/// sibling plugin.
const XLSX_PART_MARKER: &[u8] = b"xl/workbook.xml";

/// The ODF spreadsheet mimetype, which the format's spec mandates be stored
/// as the archive's first entry, uncompressed — guaranteeing it appears at a
/// fixed, early offset in any real `.ods` file.
const ODS_MIME_MARKER: &[u8] = b"application/vnd.oasis.opendocument.spreadsheet";

/// One sheet's sampled rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpreadsheetSheet {
    /// The sheet's name.
    pub name: String,
    /// Up to [`MAX_ROWS`] rows, each cell rendered to its display text.
    pub rows: Vec<Vec<String>>,
    /// Total number of rows in the sheet.
    pub row_count: usize,
    /// Whether `rows` was cut off at [`MAX_ROWS`].
    pub truncated: bool,
}

/// View data produced by [`SpreadsheetCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpreadsheetView {
    /// Every sheet in the workbook, in workbook order.
    pub sheets: Vec<SpreadsheetSheet>,
}

/// Whether `haystack` contains `needle` anywhere as a contiguous byte run.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Renders a calamine cell value to its display text.
fn render_cell(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(s) | Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Float(f) => f.to_string(),
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::DateTime(dt) => dt.to_string(),
        Data::Error(err) => format!("#ERROR: {err}"),
    }
}

/// Reads every sheet's name and up to [`MAX_ROWS`] rows from an already
/// opened workbook.
fn read_sheets<R: Reader<Cursor<Vec<u8>>>>(workbook: &mut R) -> io::Result<Vec<SpreadsheetSheet>>
where
    R::Error: std::fmt::Display,
{
    let to_io_err = |err: R::Error| io::Error::new(io::ErrorKind::InvalidData, err.to_string());

    let mut sheets = Vec::new();
    for name in workbook.sheet_names() {
        let range = workbook.worksheet_range(&name).map_err(to_io_err)?;
        let row_count = range.rows().count();
        let rows = range
            .rows()
            .take(MAX_ROWS)
            .map(|row| row.iter().map(render_cell).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let truncated = rows.len() < row_count;
        sheets.push(SpreadsheetSheet {
            name,
            rows,
            row_count,
            truncated,
        });
    }
    Ok(sheets)
}

/// Reads every sheet in the spreadsheet at `path`, dispatching to the OOXML
/// or ODF reader by its file extension.
fn read_workbook(path: &Path) -> io::Result<Vec<SpreadsheetSheet>> {
    let bytes = std::fs::read(path)?;
    let is_ods = contains_bytes(&bytes, ODS_MIME_MARKER);
    if is_ods {
        let mut workbook: Ods<_> = Ods::new(Cursor::new(bytes))
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
        read_sheets(&mut workbook)
    } else {
        let mut workbook: Xlsx<_> = Xlsx::new(Cursor::new(bytes))
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
        read_sheets(&mut workbook)
    }
}

/// Renders `sheet` as an aligned, spreadsheet-like grid: one line per
/// sampled row, columns padded to their widest cell.
fn present_sheet(sheet: &SpreadsheetSheet) -> Vec<String> {
    let column_count = sheet.rows.iter().map(Vec::len).max().unwrap_or(0);

    let mut widths = vec![0usize; column_count];
    for row in &sheet.rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }

    sheet
        .rows
        .iter()
        .map(|row| {
            (0..column_count)
                .map(|index| {
                    let cell = row.get(index).map_or("", String::as_str);
                    format!("{cell:<width$}", width = widths[index])
                })
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .collect()
}

/// The spreadsheet plugin's core half. Recognises `.xlsx` and `.ods`
/// spreadsheets.
#[derive(Debug, Default)]
pub struct SpreadsheetCore;

impl PluginCore for SpreadsheetCore {
    fn name(&self) -> &'static str {
        "spreadsheet"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        prefix.starts_with(b"PK\x03\x04")
            && (contains_bytes(prefix, XLSX_PART_MARKER) || contains_bytes(prefix, ODS_MIME_MARKER))
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let sheets = read_workbook(path)?;
        let view = SpreadsheetView { sheets };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The spreadsheet plugin's presentation half.
#[derive(Debug, Default)]
pub struct SpreadsheetPresentation;

impl PluginPresentation for SpreadsheetPresentation {
    fn name(&self) -> &'static str {
        "spreadsheet"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: SpreadsheetView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };

        if view.sheets.is_empty() {
            return vec!["no sheets".to_owned()];
        }

        let mut lines = Vec::new();
        for (index, sheet) in view.sheets.iter().enumerate() {
            if index > 0 {
                lines.push(String::new());
            }
            lines.push(format!("{} ({} rows)", sheet.name, sheet.row_count));
            lines.extend(present_sheet(sheet));
            if sheet.truncated {
                lines.push("… (truncated)".to_owned());
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{SpreadsheetCore, SpreadsheetPresentation, SpreadsheetSheet, SpreadsheetView};
    use plugin_api::{PluginCore, PluginPresentation};
    use std::fmt::Write as _;
    use std::io::Write as _;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-spreadsheet-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write_test_xlsx(path: &std::path::Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();

        writer.start_file("[Content_Types].xml", opts).unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#,
            )
            .unwrap();

        writer.start_file("_rels/.rels", opts).unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#,
            )
            .unwrap();

        writer.start_file("xl/workbook.xml", opts).unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#,
            )
            .unwrap();

        writer
            .start_file("xl/_rels/workbook.xml.rels", opts)
            .unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#,
            )
            .unwrap();

        writer.start_file("xl/worksheets/sheet1.xml", opts).unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<sheetData>
<row r="1"><c r="A1" t="str"><v>name</v></c><c r="B1" t="str"><v>age</v></c></row>
<row r="2"><c r="A2" t="str"><v>Alice</v></c><c r="B2"><v>30</v></c></row>
<row r="3"><c r="A3" t="str"><v>Bob</v></c><c r="B3"><v>25</v></c></row>
</sheetData>
</worksheet>"#,
            )
            .unwrap();

        writer.finish().unwrap();
    }

    fn write_test_ods(path: &std::path::Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(
                "mimetype",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer
            .write_all(b"application/vnd.oasis.opendocument.spreadsheet")
            .unwrap();
        writer
            .start_file(
                "META-INF/manifest.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">
<manifest:file-entry manifest:full-path="/" manifest:version="1.2" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>
<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
</manifest:manifest>"#,
            )
            .unwrap();
        writer
            .start_file("content.xml", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
<office:body><office:spreadsheet>
<table:table table:name="Sheet1">
<table:table-row><table:table-cell office:value-type="string"><text:p>name</text:p></table:table-cell><table:table-cell office:value-type="string"><text:p>age</text:p></table:table-cell></table:table-row>
<table:table-row><table:table-cell office:value-type="string"><text:p>Alice</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="30"><text:p>30</text:p></table:table-cell></table:table-row>
<table:table-row><table:table-cell office:value-type="string"><text:p>Bob</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="25"><text:p>25</text:p></table:table-cell></table:table-row>
</table:table>
</office:spreadsheet></office:body>
</office:document-content>"#,
            )
            .unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn sniffs_xlsx_by_its_workbook_part_name() {
        let path = unique_temp_file("sniff.xlsx");
        write_test_xlsx(&path);
        let prefix = std::fs::read(&path).unwrap();

        assert!(SpreadsheetCore.sniff(&prefix));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn sniffs_ods_by_its_mimetype_entry() {
        let path = unique_temp_file("sniff.ods");
        write_test_ods(&path);
        let prefix = std::fs::read(&path).unwrap();

        assert!(SpreadsheetCore.sniff(&prefix));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn does_not_sniff_a_plain_zip_archive() {
        let path = unique_temp_file("plain.zip");
        let file = std::fs::File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("hello.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"hello").unwrap();
        writer.finish().unwrap();
        let prefix = std::fs::read(&path).unwrap();

        assert!(!SpreadsheetCore.sniff(&prefix));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_real_xlsx_workbook() {
        let path = unique_temp_file("test.xlsx");
        write_test_xlsx(&path);

        let data = SpreadsheetCore.view(&path).unwrap();
        let view: SpreadsheetView = serde_json::from_value(data).unwrap();

        assert_eq!(view.sheets.len(), 1);
        let sheet = &view.sheets[0];
        assert_eq!(sheet.name, "Sheet1");
        assert_eq!(
            sheet.rows,
            vec![
                vec!["name".to_owned(), "age".to_owned()],
                vec!["Alice".to_owned(), "30".to_owned()],
                vec!["Bob".to_owned(), "25".to_owned()],
            ]
        );
        assert_eq!(sheet.row_count, 3);
        assert!(!sheet.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_real_ods_workbook() {
        let path = unique_temp_file("test.ods");
        write_test_ods(&path);

        let data = SpreadsheetCore.view(&path).unwrap();
        let view: SpreadsheetView = serde_json::from_value(data).unwrap();

        assert_eq!(view.sheets.len(), 1);
        let sheet = &view.sheets[0];
        assert_eq!(sheet.name, "Sheet1");
        assert_eq!(
            sheet.rows,
            vec![
                vec!["name".to_owned(), "age".to_owned()],
                vec!["Alice".to_owned(), "30".to_owned()],
                vec!["Bob".to_owned(), "25".to_owned()],
            ]
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_rows_of_a_sheet_larger_than_the_view_limit() {
        use super::MAX_ROWS;

        let path = unique_temp_file("large.xlsx");
        let file = std::fs::File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();

        writer.start_file("[Content_Types].xml", opts).unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#,
            )
            .unwrap();

        writer.start_file("_rels/.rels", opts).unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#,
            )
            .unwrap();

        writer.start_file("xl/workbook.xml", opts).unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#,
            )
            .unwrap();

        writer
            .start_file("xl/_rels/workbook.xml.rels", opts)
            .unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#,
            )
            .unwrap();

        let row_count = MAX_ROWS + 10;
        let mut sheet_xml = String::from(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<sheetData>"#,
        );
        for row in 1..=row_count {
            write!(
                sheet_xml,
                r#"<row r="{row}"><c r="A{row}"><v>{row}</v></c></row>"#
            )
            .unwrap();
        }
        sheet_xml.push_str("</sheetData></worksheet>");

        writer.start_file("xl/worksheets/sheet1.xml", opts).unwrap();
        writer.write_all(sheet_xml.as_bytes()).unwrap();
        writer.finish().unwrap();

        let data = SpreadsheetCore.view(&path).unwrap();
        let view: SpreadsheetView = serde_json::from_value(data).unwrap();

        let sheet = &view.sheets[0];
        assert_eq!(sheet.row_count, row_count);
        assert_eq!(sheet.rows.len(), MAX_ROWS);
        assert!(sheet.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_sheets_with_aligned_columns() {
        let data = serde_json::to_value(SpreadsheetView {
            sheets: vec![SpreadsheetSheet {
                name: "Sheet1".to_owned(),
                rows: vec![
                    vec!["name".to_owned(), "age".to_owned()],
                    vec!["Alice".to_owned(), "30".to_owned()],
                ],
                row_count: 2,
                truncated: false,
            }],
        })
        .unwrap();

        let lines = SpreadsheetPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["Sheet1 (2 rows)", "name  | age", "Alice | 30 ",]
        );
    }
}
