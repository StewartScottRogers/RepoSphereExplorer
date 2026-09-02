//! PDF file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// View data produced by [`PdfCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfView {
    /// Number of pages in the document.
    pub page_count: usize,
    /// The document's title, if its Info dictionary declares one.
    pub title: Option<String>,
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
        let view = PdfView { page_count, title };
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

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        match serde_json::from_value::<PdfView>(data.clone()) {
            Ok(view) => {
                let mut lines = vec![format!("{} pages", view.page_count)];
                if let Some(title) = view.title {
                    lines.push(format!("title: {title}"));
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
        })
        .unwrap();

        let lines = PdfPresentation.present(&data);

        assert_eq!(lines, vec!["2 pages", "title: Report"]);
    }
}
