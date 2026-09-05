//! EPUB file type plugin: core and presentation halves.
//!
//! Covers `.epub` e-books: a ZIP-packaged set of XHTML chapters described
//! by an OPF package document, rendered as the plain-text paragraphs of
//! every spine chapter, in reading order.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::io::Read as _;
use std::path::Path;

/// The EPUB Open Container Format mimetype, which the spec mandates be
/// stored as the archive's first entry, uncompressed — guaranteeing it
/// appears at a fixed, early offset in any real `.epub` file, and a marker
/// not used by any sibling plugin.
const EPUB_MIME_MARKER: &[u8] = b"application/epub+zip";

/// Block-level XHTML elements whose closing tag ends a paragraph.
const XHTML_BLOCK_CLOSERS: &[&str] = &["p", "h1", "h2", "h3", "h4", "h5", "h6", "li", "blockquote"];

/// View data produced by [`EpubCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpubView {
    /// The book's extracted plain-text paragraphs, in spine reading order
    /// across every chapter.
    pub paragraphs: Vec<String>,
}

/// Whether `haystack` contains `needle` anywhere as a contiguous byte run.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
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
        let paragraphs = extract_book(path)?;
        let view = EpubView { paragraphs };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// Reads a zip entry named `name` from `archive` as UTF-8 text.
fn read_zip_entry<R: io::Read + io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> io::Result<String> {
    let mut contents = String::new();
    archive
        .by_name(name)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
        .read_to_string(&mut contents)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    Ok(contents)
}

/// Reads every spine chapter of the EPUB at `path`, in reading order, and
/// extracts its plain-text paragraphs.
fn extract_book(path: &Path) -> io::Result<Vec<String>> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    let container_xml = read_zip_entry(&mut archive, "META-INF/container.xml")?;
    let opf_path = find_attribute(&container_xml, "rootfile", "full-path").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "no rootfile in META-INF/container.xml",
        )
    })?;

    let opf_xml = read_zip_entry(&mut archive, &opf_path)?;
    let opf_dir = Path::new(&opf_path)
        .parent()
        .map(|dir| dir.to_string_lossy().into_owned())
        .unwrap_or_default();

    let manifest = parse_manifest(&opf_xml);
    let spine = parse_spine(&opf_xml);

    let mut paragraphs = Vec::new();
    for idref in spine {
        let Some(href) = manifest.get(&idref) else {
            continue;
        };
        let chapter_path = if opf_dir.is_empty() {
            href.clone()
        } else {
            format!("{opf_dir}/{href}")
        };
        let xhtml = read_zip_entry(&mut archive, &chapter_path)?;
        paragraphs.extend(extract_paragraphs(&xhtml));
    }
    Ok(paragraphs)
}

/// Returns the substring of `xml` between the first `<tag`'s closing `>`
/// and its matching `</tag>`.
fn extract_section<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open_needle = format!("<{tag}");
    let start = xml.find(&open_needle)?;
    let after_open = xml[start..].find('>')? + start + 1;
    let close_needle = format!("</{tag}>");
    let end = xml[after_open..].find(&close_needle)? + after_open;
    Some(&xml[after_open..end])
}

/// Finds every `<tag ...>` or `<tag .../>` opening in `xml`, returning each
/// one's attribute content. Guards against a longer tag name that happens
/// to share `tag` as a prefix (e.g. `item` vs. `itemref`).
fn find_tags<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let needle = format!("<{tag}");
    let mut tags = Vec::new();
    let mut remaining = xml;
    while let Some(pos) = remaining.find(&needle) {
        let after_needle = &remaining[pos + needle.len()..];
        let boundary_ok = matches!(
            after_needle.as_bytes().first(),
            Some(b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'>')
        );
        if !boundary_ok {
            remaining = after_needle;
            continue;
        }
        let Some(end_rel) = after_needle.find('>') else {
            break;
        };
        tags.push(&after_needle[..end_rel]);
        remaining = &after_needle[end_rel + 1..];
    }
    tags
}

/// Finds the value of `attr="..."` inside `tag_content`.
fn find_attr_value(tag_content: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let start = tag_content.find(&needle)? + needle.len();
    let end = tag_content[start..].find('"')? + start;
    Some(tag_content[start..end].to_owned())
}

/// Finds the value of `attr` on the first `<tag ...>` in `xml`.
fn find_attribute(xml: &str, tag: &str, attr: &str) -> Option<String> {
    find_tags(xml, tag)
        .first()
        .and_then(|tag_content| find_attr_value(tag_content, attr))
}

/// Parses the OPF package document's `<manifest>`, mapping each `<item>`'s
/// `id` to its `href`.
fn parse_manifest(opf: &str) -> HashMap<String, String> {
    let Some(section) = extract_section(opf, "manifest") else {
        return HashMap::new();
    };
    find_tags(section, "item")
        .into_iter()
        .filter_map(|tag| {
            let id = find_attr_value(tag, "id")?;
            let href = find_attr_value(tag, "href")?;
            Some((id, href))
        })
        .collect()
}

/// Parses the OPF package document's `<spine>`, returning each `<itemref>`'s
/// `idref`, in reading order.
fn parse_spine(opf: &str) -> Vec<String> {
    let Some(section) = extract_section(opf, "spine") else {
        return Vec::new();
    };
    find_tags(section, "itemref")
        .into_iter()
        .filter_map(|tag| find_attr_value(tag, "idref"))
        .collect()
}

/// Strips XML tags from `xhtml`, keeping text nodes and decoding the five
/// predefined XML entities, splitting into one paragraph per block-level
/// element's closing tag (see [`XHTML_BLOCK_CLOSERS`]).
fn extract_paragraphs(xhtml: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current = String::new();
    let mut in_tag = false;
    let mut tag = String::new();
    for c in xhtml.chars() {
        if in_tag {
            if c == '>' {
                in_tag = false;
                let name = tag.split_whitespace().next().unwrap_or("");
                if let Some(bare) = name.strip_prefix('/')
                    && XHTML_BLOCK_CLOSERS.contains(&bare)
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

/// The EPUB plugin's presentation half.
#[derive(Debug, Default)]
pub struct EpubPresentation;

impl PluginPresentation for EpubPresentation {
    fn name(&self) -> &'static str {
        "epub"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        match serde_json::from_value::<EpubView>(data.clone()) {
            Ok(view) => view.paragraphs,
            Err(err) => vec![format!("could not read view data: {err}")],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EpubCore, EpubPresentation, EpubView};
    use plugin_api::{PluginCore, PluginPresentation};
    use std::io::Write as _;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-epub-test-{}-{name}",
            std::process::id()
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

        writer
            .start_file(
                "META-INF/container.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0"?>
                <container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
                  <rootfiles>
                    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
                  </rootfiles>
                </container>"#,
            )
            .unwrap();

        writer
            .start_file(
                "OEBPS/content.opf",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0"?>
                <package version="3.0" xmlns="http://www.idpf.org/2007/opf">
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

        writer
            .start_file(
                "OEBPS/chap1.xhtml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0"?><html><body>
                <h1>Chapter One</h1>
                <p>Hello, EPUB &amp; friends.</p>
                </body></html>"#,
            )
            .unwrap();

        writer
            .start_file(
                "OEBPS/chap2.xhtml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer
            .write_all(br#"<?xml version="1.0"?><html><body><p>Second chapter.</p></body></html>"#)
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
    fn views_a_real_epub_reading_chapters_in_spine_order() {
        let path = unique_temp_file("test.epub");
        write_test_epub(&path);

        let data = EpubCore.view(&path).unwrap();
        let view: EpubView = serde_json::from_value(data).unwrap();

        assert_eq!(
            view.paragraphs,
            vec!["Chapter One", "Hello, EPUB & friends.", "Second chapter."]
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_paragraphs_as_lines() {
        let data = serde_json::to_value(EpubView {
            paragraphs: vec!["one".to_owned(), "two".to_owned()],
        })
        .unwrap();

        let lines = EpubPresentation.present(&data);

        assert_eq!(lines, vec!["one", "two"]);
    }
}
