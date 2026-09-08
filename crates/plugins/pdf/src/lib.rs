//! PDF file type plugin: core and presentation halves.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Bounds a single page's decompressed content, guarding against a
/// decompression bomb hidden in a page's content stream.
const MAX_PAGE_DECOMPRESSED_BYTES: usize = 8 * 1024 * 1024;

/// Caps the combined extracted text so a thousand-page document does not
/// put megabytes on the wire.
const MAX_TEXT_BYTES: usize = 64 * 1024;

/// View data produced by [`PdfCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfView {
    /// Number of pages in the document.
    pub page_count: usize,
    /// The document's title, if its Info dictionary declares one.
    pub title: Option<String>,
    /// The document's page text, in reading order and separated by a page
    /// marker, or `None` if no page yielded extractable text.
    pub text: Option<String>,
    /// Whether `text` was cut off at [`MAX_TEXT_BYTES`].
    pub truncated: bool,
}

/// The PDF plugin's core half.
#[derive(Debug, Default)]
pub struct PdfCore;

impl PluginCore for PdfCore {
    fn name(&self) -> &'static str {
        "pdf"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        prefix.starts_with(b"%PDF-")
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let doc = lopdf::Document::load(path)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
        let page_count = doc.get_pages().len();
        let title = document_title(&doc);
        let (text, truncated) = extract_view_text(&doc);
        let view = PdfView {
            page_count,
            title,
            text,
            truncated,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// Reads the document's `/Info /Title`, if present and valid UTF-8-ish.
fn document_title(doc: &lopdf::Document) -> Option<String> {
    let info_ref = doc.trailer.get(b"Info").ok()?;
    let info = doc.get_dictionary(info_ref.as_reference().ok()?).ok()?;
    let title_bytes = info.get(b"Title").ok()?.as_str().ok()?;
    Some(String::from_utf8_lossy(title_bytes).into_owned())
}

/// Extracts each page's text in reading order, separated by a page marker,
/// and caps the result at [`MAX_TEXT_BYTES`].
///
/// A page whose text cannot be decoded (an encrypted document, a damaged
/// content stream) contributes nothing rather than failing the whole
/// document. If no page yields extractable text at all - for example a
/// scanned document with no text layer - this returns `(None, false)` so
/// the caller falls back to the metadata-only view instead of erroring.
fn extract_view_text(doc: &lopdf::Document) -> (Option<String>, bool) {
    let page_numbers: Vec<u32> = doc.get_pages().keys().copied().collect();
    let page_texts: Vec<String> = page_numbers
        .iter()
        .map(|&page_number| {
            doc.extract_text_with_limit(&[page_number], MAX_PAGE_DECOMPRESSED_BYTES)
                .unwrap_or_default()
        })
        .collect();

    if page_texts.iter().all(|text| text.trim().is_empty()) {
        return (None, false);
    }

    let combined = page_numbers
        .iter()
        .zip(&page_texts)
        .map(|(page_number, page_text)| format!("--- page {page_number} ---\n{}", page_text.trim()))
        .collect::<Vec<_>>()
        .join("\n\n");

    let truncated = combined.len() > MAX_TEXT_BYTES;
    let text = if truncated {
        let mut cut = MAX_TEXT_BYTES;
        while !combined.is_char_boundary(cut) {
            cut -= 1;
        }
        combined[..cut].to_owned()
    } else {
        combined
    };
    (Some(text), truncated)
}

/// The PDF plugin's presentation half.
#[derive(Debug, Default)]
pub struct PdfPresentation;

impl PluginPresentation for PdfPresentation {
    fn name(&self) -> &'static str {
        "pdf"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "PDF",
            tint: 0x00dc_2626,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["pdf"]
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        match serde_json::from_value::<PdfView>(data.clone()) {
            Ok(view) => {
                let mut lines = vec![format!("{} pages", view.page_count)];
                if let Some(title) = view.title {
                    lines.push(format!("title: {title}"));
                }
                if let Some(text) = view.text {
                    lines.extend(text.lines().map(str::to_owned));
                    if view.truncated {
                        lines.push("… (truncated)".to_owned());
                    }
                }
                lines
            }
            Err(err) => vec![format!("could not read view data: {err}")],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PdfCore, PdfPresentation, PdfView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rse-plugin-pdf-test-{}-{name}", std::process::id()))
    }

    fn write_test_pdf(path: &std::path::Path, page_count: u32) {
        use lopdf::{Document, Object, ObjectId, dictionary};

        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();

        let mut kids: Vec<Object> = Vec::new();
        for _ in 0..page_count {
            let new_page: ObjectId = doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
            });
            kids.push(Object::Reference(new_page));
        }

        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => i64::from(page_count),
            }),
        );

        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        doc.save(path).unwrap();
    }

    /// Writes a PDF whose pages each hold a real content stream, drawing
    /// `page_texts[i]` with the standard Helvetica font.
    fn write_test_pdf_with_text(path: &std::path::Path, page_texts: &[&str]) {
        use lopdf::{
            Document, Object, ObjectId, Stream,
            content::{Content, Operation},
            dictionary,
        };

        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();

        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });

        let mut kids: Vec<Object> = Vec::new();
        for page_text in page_texts {
            let content = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new(
                        "Tf",
                        vec![Object::Name(b"F1".to_vec()), Object::Integer(24)],
                    ),
                    Operation::new("Td", vec![Object::Integer(72), Object::Integer(712)]),
                    Operation::new("Tj", vec![Object::string_literal(*page_text)]),
                    Operation::new("ET", vec![]),
                ],
            };
            let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
            let new_page: ObjectId = doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => content_id,
                "Resources" => resources_id,
            });
            kids.push(Object::Reference(new_page));
        }

        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => i64::try_from(page_texts.len()).unwrap(),
            }),
        );

        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        doc.save(path).unwrap();
    }

    #[test]
    fn sniffs_the_pdf_header_magic() {
        assert!(PdfCore.sniff(b"%PDF-1.4\n rest"));
        assert!(!PdfCore.sniff(b"not a pdf"));
    }

    #[test]
    fn views_a_real_pdf_and_counts_its_pages() {
        let path = unique_temp_file("test.pdf");
        write_test_pdf(&path, 3);

        let data = PdfCore.view(&path).unwrap();
        let view: PdfView = serde_json::from_value(data).unwrap();

        assert_eq!(view.page_count, 3);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn falls_back_to_metadata_when_no_text_is_extractable() {
        let path = unique_temp_file("blank-pages.pdf");
        write_test_pdf(&path, 2);

        let data = PdfCore.view(&path).unwrap();
        let view: PdfView = serde_json::from_value(data).unwrap();

        assert_eq!(view.page_count, 2);
        assert!(view.text.is_none());
        assert!(!view.truncated);

        let lines = PdfPresentation.present(&serde_json::to_value(&view).unwrap());
        assert_eq!(lines, vec!["2 pages"]);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn extracts_page_text_separated_by_a_page_marker() {
        let path = unique_temp_file("two-pages-of-text.pdf");
        write_test_pdf_with_text(&path, &["Hello, page one!", "Hello, page two!"]);

        let data = PdfCore.view(&path).unwrap();
        let view: PdfView = serde_json::from_value(data).unwrap();

        let lines = PdfPresentation.present(&serde_json::to_value(&view).unwrap());
        assert!(lines.iter().any(|line| line == "--- page 1 ---"));
        assert!(lines.iter().any(|line| line == "--- page 2 ---"));

        let text = view.text.expect("page text should be extractable");
        assert!(!view.truncated);
        assert!(text.contains("--- page 1 ---"));
        assert!(text.contains("Hello, page one!"));
        assert!(text.contains("--- page 2 ---"));
        assert!(text.contains("Hello, page two!"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_text_past_the_cap_with_a_visible_marker() {
        let path = unique_temp_file("huge-page.pdf");
        let huge_text = "a".repeat(super::MAX_TEXT_BYTES * 2);
        write_test_pdf_with_text(&path, &[huge_text.as_str()]);

        let data = PdfCore.view(&path).unwrap();
        let view: PdfView = serde_json::from_value(data).unwrap();

        assert!(view.truncated);

        let lines = PdfPresentation.present(&serde_json::to_value(&view).unwrap());
        assert_eq!(lines.last().map(String::as_str), Some("… (truncated)"));

        let text = view.text.expect("page text should be extractable");
        assert!(text.len() <= super::MAX_TEXT_BYTES);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn previews_the_repo_sample_pdf_with_its_page_text() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/pdf/example.pdf");

        let data = PdfCore.view(&path).unwrap();
        let view: PdfView = serde_json::from_value(data).unwrap();

        let lines = PdfPresentation.present(&serde_json::to_value(&view).unwrap());
        assert!(lines.iter().any(|line| line == "--- page 1 ---"));

        let text = view.text.expect("sample PDF should have extractable text");
        assert!(text.contains("--- page 1 ---"));
        assert!(text.contains("--- page 2 ---"));
    }

    #[test]
    fn presents_page_count_and_title() {
        let data = serde_json::to_value(PdfView {
            page_count: 2,
            title: Some("Report".to_owned()),
            text: None,
            truncated: false,
        })
        .unwrap();

        let lines = PdfPresentation.present(&data);

        assert_eq!(lines, vec!["2 pages", "title: Report"]);
    }
}
