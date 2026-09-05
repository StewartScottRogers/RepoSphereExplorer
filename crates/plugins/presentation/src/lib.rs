//! Presentation file type plugin: core and presentation halves.
//!
//! Covers `.pptx` and `.odp` as one plugin, per the issue's direction that
//! this document family renders as a single slide-deck view rather than a
//! plugin per container format.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::io::{Cursor, Read as _};
use std::path::Path;

/// The PPTX-specific internal part path, unique to OOXML presentations (as
/// opposed to `word/document.xml` for a word-processing document or
/// `xl/workbook.xml` for a spreadsheet) — a marker not used by any sibling
/// plugin.
const PPTX_PART_MARKER: &[u8] = b"ppt/presentation.xml";

/// The ODF presentation mimetype, which the format's spec mandates be
/// stored as the archive's first entry, uncompressed — guaranteeing it
/// appears at a fixed, early offset in any real `.odp` file.
const ODP_MIME_MARKER: &[u8] = b"application/vnd.oasis.opendocument.presentation";

/// The tag `</draw:page>` closes; ODF stores one per slide, in presentation
/// order, directly inside `content.xml`.
const ODP_PAGE_OPEN: &str = "<draw:page";

/// The closing counterpart of [`ODP_PAGE_OPEN`].
const ODP_PAGE_CLOSE: &str = "</draw:page>";

/// One slide's extracted text lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationSlide {
    /// The slide's 1-based position in the deck.
    pub number: usize,
    /// Text paragraphs found on the slide, in document order.
    pub lines: Vec<String>,
}

/// View data produced by [`PresentationCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresentationView {
    /// Every slide in the deck, in presentation order.
    pub slides: Vec<PresentationSlide>,
}

/// Whether `haystack` contains `needle` anywhere as a contiguous byte run.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Strips XML tags from `xml`, keeping text nodes and decoding the five
/// predefined XML entities, splitting into one paragraph per `<a:p>`
/// (`DrawingML`, used by PPTX slides) or `<text:p>` (ODF) closing tag — the
/// two formats' own paragraph elements, which is all this generic walk
/// needs to tell paragraphs apart since both wrap their text runs in child
/// elements this same walk already strips.
fn extract_paragraphs(xml: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current = String::new();
    let mut in_tag = false;
    let mut tag = String::new();
    for c in xml.chars() {
        if in_tag {
            if c == '>' {
                in_tag = false;
                let name = tag.split_whitespace().next().unwrap_or("");
                if let Some(bare) = name.strip_prefix('/')
                    && (bare == "a:p" || bare == "text:p")
                {
                    paragraphs.push(decode_entities(current.trim()));
                    current.clear();
                }
                tag.clear();
            } else {
                tag.push(c);
            }
        } else if c == '<' {
            in_tag = true;
        } else {
            current.push(c);
        }
    }
    paragraphs.retain(|p| !p.is_empty());
    paragraphs
}

/// Decodes the five XML predefined character entities.
fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Parses a PPTX slide part's numeric suffix out of its ZIP entry name
/// (`ppt/slides/slideN.xml`), excluding sibling entries such as
/// `ppt/slides/_rels/slideN.xml.rels`.
fn pptx_slide_number(name: &str) -> Option<u32> {
    name.strip_prefix("ppt/slides/slide")?
        .strip_suffix(".xml")?
        .parse()
        .ok()
}

/// Reads every slide from a PPTX package at `path`. Slides are ordered by
/// their part name's numeric suffix, an approximation of true presentation
/// order (the authoritative order lives in `ppt/presentation.xml`'s slide
/// relationship list) — an accepted limitation shared with this project's
/// other structurally-sniffed formats, since resolving relationship IDs
/// would need a fuller OOXML reader than this plugin's minimal-dependency
/// pattern allows.
fn extract_pptx_slides(path: &Path) -> io::Result<Vec<PresentationSlide>> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    let mut names = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let name = entry.name().to_owned();
        if let Some(number) = pptx_slide_number(&name) {
            names.push((number, name));
        }
    }
    names.sort_by_key(|(number, _)| *number);

    let mut slides = Vec::new();
    for (position, (_, name)) in names.into_iter().enumerate() {
        let mut xml = String::new();
        archive
            .by_name(&name)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
            .read_to_string(&mut xml)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        slides.push(PresentationSlide {
            number: position + 1,
            lines: extract_paragraphs(&xml),
        });
    }
    Ok(slides)
}

/// Splits ODF `content.xml` into one string slice per `<draw:page>` element
/// (ODF's own slide element), in document order — which for `.odp` already
/// matches presentation order, unlike PPTX's separate, relationship-indexed
/// slide parts.
fn split_odp_pages(xml: &str) -> Vec<&str> {
    let mut pages = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(ODP_PAGE_OPEN) {
        let from_start = &rest[start..];
        let Some(end) = from_start.find(ODP_PAGE_CLOSE) else {
            break;
        };
        let page_end = end + ODP_PAGE_CLOSE.len();
        pages.push(&from_start[..page_end]);
        rest = &from_start[page_end..];
    }
    pages
}

/// Reads every slide from an ODP package already read into `bytes`.
fn extract_odp_slides(bytes: &[u8]) -> io::Result<Vec<PresentationSlide>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let mut xml = String::new();
    archive
        .by_name("content.xml")
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
        .read_to_string(&mut xml)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    Ok(split_odp_pages(&xml)
        .into_iter()
        .enumerate()
        .map(|(position, page_xml)| PresentationSlide {
            number: position + 1,
            lines: extract_paragraphs(page_xml),
        })
        .collect())
}

/// The presentation plugin's core half. Recognises `.pptx` and `.odp`
/// slide decks.
#[derive(Debug, Default)]
pub struct PresentationCore;

impl PluginCore for PresentationCore {
    fn name(&self) -> &'static str {
        "presentation"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        prefix.starts_with(b"PK\x03\x04")
            && (contains_bytes(prefix, PPTX_PART_MARKER) || contains_bytes(prefix, ODP_MIME_MARKER))
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let slides = if contains_bytes(&bytes, ODP_MIME_MARKER) {
            extract_odp_slides(&bytes)?
        } else {
            extract_pptx_slides(path)?
        };
        let view = PresentationView { slides };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The presentation plugin's presentation half.
#[derive(Debug, Default)]
pub struct PresentationPresentation;

impl PluginPresentation for PresentationPresentation {
    fn name(&self) -> &'static str {
        "presentation"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: PresentationView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };

        if view.slides.is_empty() {
            return vec!["no slides".to_owned()];
        }

        let mut lines = Vec::new();
        for (index, slide) in view.slides.iter().enumerate() {
            if index > 0 {
                lines.push(String::new());
            }
            lines.push(format!("Slide {}", slide.number));
            if slide.lines.is_empty() {
                lines.push("(no text)".to_owned());
            } else {
                lines.extend(slide.lines.iter().cloned());
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{PresentationCore, PresentationPresentation, PresentationSlide, PresentationView};
    use plugin_api::{PluginCore, PluginPresentation};
    use std::io::Write as _;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-presentation-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write_test_pptx(path: &std::path::Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();

        writer.start_file("ppt/presentation.xml", opts).unwrap();
        writer
            .write_all(br#"<?xml version="1.0"?><p:presentation/>"#)
            .unwrap();

        writer.start_file("ppt/slides/slide1.xml", opts).unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0"?><p:sld><p:cSld><p:spTree><p:sp><p:txBody>
                <a:p><a:r><a:t>Hello, Slide One.</a:t></a:r></a:p>
                </p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#,
            )
            .unwrap();

        writer.start_file("ppt/slides/slide2.xml", opts).unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0"?><p:sld><p:cSld><p:spTree><p:sp><p:txBody>
                <a:p><a:r><a:t>Second slide &amp; more.</a:t></a:r></a:p>
                </p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#,
            )
            .unwrap();

        writer
            .start_file("ppt/slides/_rels/slide1.xml.rels", opts)
            .unwrap();
        writer.write_all(b"<Relationships/>").unwrap();

        writer.finish().unwrap();
    }

    fn write_test_odp(path: &std::path::Path) {
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
            .write_all(b"application/vnd.oasis.opendocument.presentation")
            .unwrap();
        writer
            .start_file("content.xml", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0"?><office:document-content><office:body><office:presentation>
                <draw:page draw:name="page1"><draw:frame><draw:text-box>
                <text:p>Hello, ODP world.</text:p>
                </draw:text-box></draw:frame></draw:page>
                <draw:page draw:name="page2"><draw:frame><draw:text-box>
                <text:p>Second page text.</text:p>
                </draw:text-box></draw:frame></draw:page>
                </office:presentation></office:body></office:document-content>"#,
            )
            .unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn sniffs_pptx_by_its_presentation_part_name() {
        let path = unique_temp_file("sniff.pptx");
        write_test_pptx(&path);
        let prefix = std::fs::read(&path).unwrap();

        assert!(PresentationCore.sniff(&prefix));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn sniffs_odp_by_its_mimetype_entry() {
        let path = unique_temp_file("sniff.odp");
        write_test_odp(&path);
        let prefix = std::fs::read(&path).unwrap();

        assert!(PresentationCore.sniff(&prefix));

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

        assert!(!PresentationCore.sniff(&prefix));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_real_pptx_deck() {
        let path = unique_temp_file("test.pptx");
        write_test_pptx(&path);

        let data = PresentationCore.view(&path).unwrap();
        let view: PresentationView = serde_json::from_value(data).unwrap();

        assert_eq!(
            view.slides,
            vec![
                PresentationSlide {
                    number: 1,
                    lines: vec!["Hello, Slide One.".to_owned()],
                },
                PresentationSlide {
                    number: 2,
                    lines: vec!["Second slide & more.".to_owned()],
                },
            ]
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_real_odp_deck() {
        let path = unique_temp_file("test.odp");
        write_test_odp(&path);

        let data = PresentationCore.view(&path).unwrap();
        let view: PresentationView = serde_json::from_value(data).unwrap();

        assert_eq!(
            view.slides,
            vec![
                PresentationSlide {
                    number: 1,
                    lines: vec!["Hello, ODP world.".to_owned()],
                },
                PresentationSlide {
                    number: 2,
                    lines: vec!["Second page text.".to_owned()],
                },
            ]
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_slides_with_headers() {
        let data = serde_json::to_value(PresentationView {
            slides: vec![
                PresentationSlide {
                    number: 1,
                    lines: vec!["one".to_owned(), "two".to_owned()],
                },
                PresentationSlide {
                    number: 2,
                    lines: vec![],
                },
            ],
        })
        .unwrap();

        let lines = PresentationPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["Slide 1", "one", "two", "", "Slide 2", "(no text)"]
        );
    }
}
