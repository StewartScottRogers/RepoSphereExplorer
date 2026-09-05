//! EPUB file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::io::Read as _;
use std::path::Path;

/// The EPUB mimetype, which the format's spec mandates be stored as the
/// archive's first entry, uncompressed — guaranteeing it appears at a fixed,
/// early offset in any real `.epub` file.
const EPUB_MIME_MARKER: &[u8] = b"application/epub+zip";

/// The fixed location of the file that points at the package document,
/// mandated by the OCF (Open Container Format) part of the EPUB spec.
const CONTAINER_PATH: &str = "META-INF/container.xml";

/// One chapter's extracted text lines, in reading order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpubChapter {
    /// The chapter's 1-based position in the spine (reading order).
    pub number: usize,
    /// Text paragraphs found in the chapter, in document order.
    pub lines: Vec<String>,
}

/// View data produced by [`EpubCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpubView {
    /// The book's title, from the package document's `dc:title`, if present.
    pub title: Option<String>,
    /// The book's author, from the package document's `dc:creator`, if
    /// present.
    pub author: Option<String>,
    /// Every chapter in the book, in spine (reading) order.
    pub chapters: Vec<EpubChapter>,
}

/// Whether `haystack` contains `needle` anywhere as a contiguous byte run.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Returns every complete `<tag_name ...>` opening tag in `xml`, attributes
/// included, stopping at each tag's first `>`. Good enough for the
/// self-closing `<rootfile>`, `<item>`, and `<itemref>` elements this plugin
/// reads, none of which nest a `>` inside an attribute value in practice.
fn extract_tags<'xml>(xml: &'xml str, tag_name: &str) -> Vec<&'xml str> {
    let open = format!("<{tag_name} ");
    let mut tags = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let from_start = &rest[start..];
        let Some(end) = from_start.find('>') else {
            break;
        };
        tags.push(&from_start[..=end]);
        rest = &from_start[end + 1..];
    }
    tags
}

/// Extracts the value of `attr_name="..."` from within `tag`.
fn extract_attr(tag: &str, attr_name: &str) -> Option<String> {
    let needle = format!("{attr_name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')?;
    Some(tag[start..start + end].to_owned())
}

/// Extracts the decoded text content of the first `<tag_name>...</tag_name>`
/// element in `xml`.
fn extract_first_text(xml: &str, tag_name: &str) -> Option<String> {
    let open_start = xml.find(&format!("<{tag_name}"))?;
    let content_start = xml[open_start..].find('>')? + open_start + 1;
    let close = format!("</{tag_name}>");
    let content_end = xml[content_start..].find(&close)? + content_start;
    Some(decode_entities(xml[content_start..content_end].trim()))
}

/// Decodes the five XML predefined character entities.
fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Strips XML tags from `xml`, keeping text nodes and decoding the five
/// predefined XML entities, splitting into one paragraph per `<p>` closing
/// tag — the XHTML content documents EPUB chapters are written in.
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
                if name.strip_prefix('/') == Some("p") {
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

/// The directory a zip entry path sits in, or `""` if it sits at the
/// archive's root.
fn parent_dir(entry_path: &str) -> &str {
    entry_path.rfind('/').map_or("", |i| &entry_path[..i])
}

/// Reads the book's title, author, and chapters from an EPUB package at
/// `path`: follows [`CONTAINER_PATH`] to the package document, then reads
/// its metadata and walks its manifest and spine to read each chapter's
/// content document in reading order.
///
/// Chapter hrefs are resolved only relative to the package document's own
/// directory; a manifest using `../` to reach outside it, or a spine
/// `idref` with no matching manifest item, is skipped — an accepted
/// limitation shared with this project's other structurally-read formats.
fn read_book(path: &Path) -> io::Result<EpubView> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    let mut container_xml = String::new();
    archive
        .by_name(CONTAINER_PATH)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
        .read_to_string(&mut container_xml)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let opf_path = extract_tags(&container_xml, "rootfile")
        .first()
        .and_then(|tag| extract_attr(tag, "full-path"))
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "no rootfile in container.xml")
        })?;

    let mut opf_xml = String::new();
    archive
        .by_name(&opf_path)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
        .read_to_string(&mut opf_xml)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    let title = extract_first_text(&opf_xml, "dc:title");
    let author = extract_first_text(&opf_xml, "dc:creator");

    let manifest: HashMap<String, String> = extract_tags(&opf_xml, "item")
        .into_iter()
        .filter_map(|tag| Some((extract_attr(tag, "id")?, extract_attr(tag, "href")?)))
        .collect();
    let spine: Vec<String> = extract_tags(&opf_xml, "itemref")
        .into_iter()
        .filter_map(|tag| extract_attr(tag, "idref"))
        .collect();

    let opf_dir = parent_dir(&opf_path);
    let mut chapters = Vec::new();
    for idref in &spine {
        let Some(href) = manifest.get(idref) else {
            continue;
        };
        let entry_path = if opf_dir.is_empty() {
            href.clone()
        } else {
            format!("{opf_dir}/{href}")
        };
        let Ok(mut entry) = archive.by_name(&entry_path) else {
            continue;
        };
        let mut xhtml = String::new();
        entry
            .read_to_string(&mut xhtml)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        drop(entry);
        chapters.push(EpubChapter {
            number: chapters.len() + 1,
            lines: extract_paragraphs(&xhtml),
        });
    }

    Ok(EpubView {
        title,
        author,
        chapters,
    })
}

/// The EPUB plugin's core half. Recognises `.epub` e-books.
#[derive(Debug, Default)]
pub struct EpubCore;

impl PluginCore for EpubCore {
    fn name(&self) -> &'static str {
        "epub"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        prefix.starts_with(b"PK\x03\x04") && contains_bytes(prefix, EPUB_MIME_MARKER)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let view = read_book(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The EPUB plugin's presentation half.
#[derive(Debug, Default)]
pub struct EpubPresentation;

impl PluginPresentation for EpubPresentation {
    fn name(&self) -> &'static str {
        "epub"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: EpubView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };

        let mut lines = Vec::new();
        if let Some(title) = &view.title {
            lines.push(format!("Title: {title}"));
        }
        if let Some(author) = &view.author {
            lines.push(format!("Author: {author}"));
        }
        if !lines.is_empty() {
            lines.push(String::new());
        }

        if view.chapters.is_empty() {
            lines.push("no chapters".to_owned());
            return lines;
        }

        for (index, chapter) in view.chapters.iter().enumerate() {
            if index > 0 {
                lines.push(String::new());
            }
            lines.push(format!("Chapter {}", chapter.number));
            if chapter.lines.is_empty() {
                lines.push("(no text)".to_owned());
            } else {
                lines.extend(chapter.lines.iter().cloned());
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{EpubChapter, EpubCore, EpubPresentation, EpubView};
    use plugin_api::{PluginCore, PluginPresentation};
    use std::io::Write as _;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-epub-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write_test_epub(path: &std::path::Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);

        writer
            .start_file(
                "mimetype",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(b"application/epub+zip").unwrap();

        let opts = zip::write::SimpleFileOptions::default();

        writer.start_file("META-INF/container.xml", opts).unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0"?><container><rootfiles>
                <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
                </rootfiles></container>"#,
            )
            .unwrap();

        writer.start_file("OEBPS/content.opf", opts).unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0"?><package><metadata>
                <dc:title>Hello, Book.</dc:title>
                <dc:creator>A. Author &amp; Friend</dc:creator>
                </metadata>
                <manifest>
                <item id="chap1" href="chap1.xhtml" media-type="application/xhtml+xml"/>
                <item id="chap2" href="chap2.xhtml" media-type="application/xhtml+xml"/>
                </manifest>
                <spine>
                <itemref idref="chap1"/>
                <itemref idref="chap2"/>
                </spine>
                </package>"#,
            )
            .unwrap();

        writer.start_file("OEBPS/chap1.xhtml", opts).unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0"?><html><body><p>Hello, Chapter One.</p></body></html>"#,
            )
            .unwrap();

        writer.start_file("OEBPS/chap2.xhtml", opts).unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0"?><html><body><p>Second chapter &amp; more.</p></body></html>"#,
            )
            .unwrap();

        writer.finish().unwrap();
    }

    #[test]
    fn sniffs_epub_by_its_mimetype_entry() {
        let path = unique_temp_file("sniff.epub");
        write_test_epub(&path);
        let prefix = std::fs::read(&path).unwrap();

        assert!(EpubCore.sniff(&prefix));

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

        assert!(!EpubCore.sniff(&prefix));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_real_epub_book() {
        let path = unique_temp_file("test.epub");
        write_test_epub(&path);

        let data = EpubCore.view(&path).unwrap();
        let view: EpubView = serde_json::from_value(data).unwrap();

        assert_eq!(view.title.as_deref(), Some("Hello, Book."));
        assert_eq!(view.author.as_deref(), Some("A. Author & Friend"));
        assert_eq!(
            view.chapters,
            vec![
                EpubChapter {
                    number: 1,
                    lines: vec!["Hello, Chapter One.".to_owned()],
                },
                EpubChapter {
                    number: 2,
                    lines: vec!["Second chapter & more.".to_owned()],
                },
            ]
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_title_author_and_chapters() {
        let data = serde_json::to_value(EpubView {
            title: Some("A Title".to_owned()),
            author: Some("An Author".to_owned()),
            chapters: vec![
                EpubChapter {
                    number: 1,
                    lines: vec!["one".to_owned(), "two".to_owned()],
                },
                EpubChapter {
                    number: 2,
                    lines: vec![],
                },
            ],
        })
        .unwrap();

        let lines = EpubPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "Title: A Title",
                "Author: An Author",
                "",
                "Chapter 1",
                "one",
                "two",
                "",
                "Chapter 2",
                "(no text)",
            ]
        );
    }
}
