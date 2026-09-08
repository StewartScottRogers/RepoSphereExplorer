//! PDF file type plugin: core and presentation halves.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["pdf"];

/// View data produced by [`PdfCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfView {
    /// Number of pages in the document.
    pub page_count: usize,
    /// The document's title, if its Info dictionary declares one.
    pub title: Option<String>,
    /// The pages' text, in reading order, one entry per page. Empty when
    /// nothing could be extracted - a scanned document is pictures of text,
    /// not text.
    #[serde(default)]
    pub pages: Vec<String>,
    /// Whether the text was cut off at [`MAX_TEXT_BYTES`].
    #[serde(default)]
    pub truncated: bool,
}

/// Most text carried for a preview. A pane shows a screenful; a thousand-page
/// document should not put its whole self on the wire to fill it.
const MAX_TEXT_BYTES: usize = 32 * 1024;

/// The text of each page in reading order, and whether it was cut short.
/// A page whose text cannot be extracted contributes an empty entry rather
/// than failing the whole preview.
fn page_texts(doc: &lopdf::Document) -> (Vec<String>, bool) {
    let mut pages = Vec::new();
    let mut budget = MAX_TEXT_BYTES;
    let mut truncated = false;
    let mut numbers: Vec<u32> = doc.get_pages().keys().copied().collect();
    numbers.sort_unstable();
    for number in numbers {
        if budget == 0 {
            truncated = true;
            break;
        }
        let mut text = doc.extract_text(&[number]).unwrap_or_default();
        if text.len() > budget {
            // On a character boundary, so the text stays valid UTF-8.
            let mut cut = budget;
            while cut > 0 && !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            truncated = true;
        }
        budget -= text.len().min(budget);
        pages.push(text);
    }
    (pages, truncated)
}

/// The PDF plugin's core half.
#[derive(Debug, Default)]
pub struct PdfCore;

impl PluginCore for PdfCore {
    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

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
        let (pages, truncated) = page_texts(&doc);
        let view = PdfView {
            page_count,
            title,
            pages,
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
        EXTENSIONS
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        match serde_json::from_value::<PdfView>(data.clone()) {
            Ok(view) => {
                let mut lines = vec![format!("{} pages", view.page_count)];
                if let Some(title) = view.title {
                    lines.push(format!("title: {title}"));
                }
                for (index, text) in view.pages.iter().enumerate() {
                    lines.push(String::new());
                    lines.push(format!("--- page {} ---", index + 1));
                    if text.trim().is_empty() {
                        // A scanned page is a picture of text, and this
                        // plugin does not rasterise.
                        lines.push("(no extractable text)".to_owned());
                    } else {
                        lines.extend(text.lines().map(str::to_owned));
                    }
                }
                if view.truncated {
                    lines.push(String::new());
                    lines.push("... (truncated)".to_owned());
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

    /// A one-page PDF whose page really carries `text`, so extraction has
    /// something to find. The fixture in `samples/` has empty pages, which
    /// exercises the "nothing to extract" path but proves nothing about the
    /// extraction itself.
    fn write_pdf_with_text(path: &std::path::Path, text: &str) {
        use lopdf::content::{Content, Operation};
        use lopdf::{Document, Object, Stream, dictionary};

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
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![10.into(), 100.into()]),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let stream = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => stream,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![Object::Reference(page)],
                "Count" => 1,
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
    fn a_page_s_own_text_is_what_the_preview_shows() {
        let path = unique_temp_file("with-text.pdf");
        write_pdf_with_text(&path, "Hello from inside the document");

        let data = PdfCore.view(&path).unwrap();
        let lines = PdfPresentation.present(&data);

        assert!(
            lines
                .iter()
                .any(|line| line.contains("Hello from inside the document")),
            "the page's own words, not just a marker: {lines:?}"
        );

        std::fs::remove_file(&path).unwrap();
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
    fn presents_page_count_and_title() {
        let data = serde_json::to_value(PdfView {
            page_count: 2,
            title: Some("Report".to_owned()),
            pages: Vec::new(),
            truncated: false,
        })
        .unwrap();

        let lines = PdfPresentation.present(&data);

        assert_eq!(lines, vec!["2 pages", "title: Report"]);
    }

    #[test]
    fn a_document_previews_its_page_text_with_a_marker_per_page() {
        let bytes = include_bytes!("../../../../samples/pdf/plugin-handbook.pdf");
        let path = unique_temp_file("text.pdf");
        std::fs::write(&path, bytes).unwrap();

        let data = PdfCore.view(&path).unwrap();
        let lines = PdfPresentation.present(&data);
        let view: PdfView = serde_json::from_value(data).unwrap();

        assert!(
            lines.len() > 1,
            "a page count on its own is what this replaced: {lines:?}"
        );
        for page in 1..=view.page_count {
            assert!(
                lines
                    .iter()
                    .any(|line| line == &format!("--- page {page} ---")),
                "page {page} has a marker: {lines:?}"
            );
        }

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn text_past_the_cap_is_marked_as_truncated() {
        let data = serde_json::json!({
            "page_count": 1,
            "title": serde_json::Value::Null,
            "pages": ["some text"],
            "truncated": true,
        });

        let lines = PdfPresentation.present(&data);

        assert!(
            lines.iter().any(|line| line == "... (truncated)"),
            "the reader is told the text is cut off: {lines:?}"
        );
    }

    #[test]
    fn a_page_with_no_extractable_text_says_so_rather_than_failing() {
        let data = serde_json::json!({
            "page_count": 1,
            "title": serde_json::Value::Null,
            "pages": [""],
            "truncated": false,
        });

        let lines = PdfPresentation.present(&data);

        assert!(lines.iter().any(|line| line == "1 pages"));
        assert!(
            lines.iter().any(|line| line == "(no extractable text)"),
            "a scanned page is pictures of text: {lines:?}"
        );
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::PdfCore),
            plugin_api::PluginPresentation::extensions(&crate::PdfPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
