//! Word processing document file type plugin: core and presentation halves.
//!
//! Covers `.docx`, `.odt`, and `.rtf` as one plugin, per the issue's
//! direction that this document family renders as a single paginated
//! word-processing view rather than a plugin per container format.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::io::Read as _;
use std::path::Path;

/// The DOCX-specific internal part path, unique to Word documents (as
/// opposed to `xl/workbook.xml` for a spreadsheet or `ppt/presentation.xml`
/// for a presentation) — a marker not used by any sibling plugin. A ZIP
/// entry's filename is always stored uncompressed in its local file header
/// regardless of the entry's own compression, so this is visible directly
/// in a raw byte prefix as long as the entry falls within the sniffed
/// window; a document whose preceding parts push it past that window will
/// not be recognised, an accepted content-sniffing limitation shared with
/// this project's other structurally-sniffed formats.
const DOCX_PART_MARKER: &[u8] = b"word/document.xml";

/// The ODF text-document mimetype, which the format's spec mandates be
/// stored as the archive's first entry, uncompressed — guaranteeing it
/// appears at a fixed, early offset in any real `.odt` file.
const ODT_MIME_MARKER: &[u8] = b"application/vnd.oasis.opendocument.text";

/// The RTF format's fixed opening control word.
const RTF_MARKER: &[u8] = b"{\\rtf1";

/// Destination control words whose group content is metadata (fonts,
/// colours, styles, document info, the generator string, embedded
/// pictures) rather than body text, so [`extract_rtf`] skips it.
const RTF_SKIP_DESTINATIONS: &[&str] = &[
    "fonttbl",
    "colortbl",
    "stylesheet",
    "info",
    "generator",
    "pict",
];

/// View data produced by [`WordDocumentCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordDocumentView {
    /// The document's extracted plain-text paragraphs.
    pub paragraphs: Vec<String>,
}

/// Whether `haystack` contains `needle` anywhere as a contiguous byte run.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// The word-document plugin's core half. Recognises `.docx`, `.odt`, and
/// `.rtf` word-processing documents.
#[derive(Debug, Default)]
pub struct WordDocumentCore;

impl PluginCore for WordDocumentCore {
    fn name(&self) -> &'static str {
        "word-document"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        prefix.starts_with(RTF_MARKER)
            || (prefix.starts_with(b"PK\x03\x04")
                && (contains_bytes(prefix, DOCX_PART_MARKER)
                    || contains_bytes(prefix, ODT_MIME_MARKER)))
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let paragraphs = if bytes.starts_with(RTF_MARKER) {
            extract_rtf(&bytes)
        } else {
            extract_zip_document(path)?
        };
        let view = WordDocumentView { paragraphs };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// Extracts paragraph text from a ZIP-packaged word-processing document
/// (`.docx` or `.odt`) at `path`, reading whichever of the two known
/// document parts is present.
fn extract_zip_document(path: &Path) -> io::Result<Vec<String>> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let part_name = if archive.by_name("word/document.xml").is_ok() {
        "word/document.xml"
    } else {
        "content.xml"
    };
    let mut xml = String::new();
    archive
        .by_name(part_name)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
        .read_to_string(&mut xml)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    Ok(extract_paragraphs(&xml))
}

/// Strips XML tags from `xml`, keeping text nodes and decoding the five
/// predefined XML entities, splitting into one paragraph per `<w:p>`
/// (DOCX) or `<text:p>` (ODF) closing tag — the two document formats' own
/// paragraph elements, which is all this generic walk needs to tell
/// paragraphs apart since both wrap their text runs in child elements this
/// same walk already strips.
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
                    && (bare == "w:p" || bare == "text:p")
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

/// Converts RTF control-word markup to plain text: skips the content of
/// any font/colour/style/info/generator/picture destination group, decodes
/// `\'XX` hex-escaped ASCII bytes and the `\{`/`\}`/`\\` control symbols,
/// and turns `\par`/`\line` into paragraph breaks.
fn extract_rtf(bytes: &[u8]) -> Vec<String> {
    let mut text = String::new();
    let mut depth: i32 = 0;
    let mut skip_from: Option<i32> = None;
    let mut chars = bytes.iter().copied().peekable();

    while let Some(b) = chars.next() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                if skip_from == Some(depth) {
                    skip_from = None;
                }
                depth -= 1;
            }
            b'\\' => match chars.peek().copied() {
                Some(b'\'') => {
                    chars.next();
                    if let (Some(hi), Some(lo)) = (chars.next(), chars.next()) {
                        let hex = [hi, lo];
                        if let Ok(hex_str) = std::str::from_utf8(&hex)
                            && let Ok(byte) = u8::from_str_radix(hex_str, 16)
                            && skip_from.is_none()
                            && byte.is_ascii()
                        {
                            text.push(byte as char);
                        }
                    }
                }
                Some(c) if c.is_ascii_alphabetic() => {
                    let mut word = String::new();
                    while let Some(&c) = chars.peek() {
                        if c.is_ascii_alphabetic() {
                            word.push(c as char);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    if chars.peek() == Some(&b'-') {
                        chars.next();
                    }
                    while let Some(&c) = chars.peek() {
                        if c.is_ascii_digit() {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    if chars.peek() == Some(&b' ') {
                        chars.next();
                    }
                    if skip_from.is_none() && (word == "par" || word == "line") {
                        text.push('\n');
                    }
                    if skip_from.is_none() && RTF_SKIP_DESTINATIONS.contains(&word.as_str()) {
                        skip_from = Some(depth);
                    }
                }
                Some(b'{' | b'}' | b'\\') => {
                    if let Some(c) = chars.next()
                        && skip_from.is_none()
                    {
                        text.push(c as char);
                    }
                }
                _ => {
                    chars.next();
                }
            },
            _ => {
                if skip_from.is_none() {
                    text.push(b as char);
                }
            }
        }
    }

    text.split('\n')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The word-document plugin's presentation half.
#[derive(Debug, Default)]
pub struct WordDocumentPresentation;

impl PluginPresentation for WordDocumentPresentation {
    fn name(&self) -> &'static str {
        "word-document"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        match serde_json::from_value::<WordDocumentView>(data.clone()) {
            Ok(view) => view.paragraphs,
            Err(err) => vec![format!("could not read view data: {err}")],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WordDocumentCore, WordDocumentPresentation, WordDocumentView};
    use plugin_api::{PluginCore, PluginPresentation};
    use std::io::Write as _;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-word-document-test-{}-{name}",
            std::process::id()
        ))
    }

    fn write_test_docx(path: &std::path::Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0"?><w:document><w:body>
                <w:p><w:r><w:t>Hello, Word &amp; friends.</w:t></w:r></w:p>
                <w:p><w:r><w:t>Second paragraph.</w:t></w:r></w:p>
                </w:body></w:document>"#,
            )
            .unwrap();
        writer.finish().unwrap();
    }

    fn write_test_odt(path: &std::path::Path) {
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
            .write_all(b"application/vnd.oasis.opendocument.text")
            .unwrap();
        writer
            .start_file("content.xml", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0"?><office:document-content><office:body><office:text>
                <text:p>Hello, ODF world.</text:p>
                <text:p>Another paragraph.</text:p>
                </office:text></office:body></office:document-content>"#,
            )
            .unwrap();
        writer.finish().unwrap();
    }

    fn write_test_rtf(path: &std::path::Path) {
        std::fs::write(
            path,
            br"{\rtf1\ansi\deff0{\fonttbl{\f0 Times New Roman;}}{\*\generator Test;}\pard Hello, RTF world.\par Second paragraph.\par}",
        )
        .unwrap();
    }

    #[test]
    fn sniffs_rtf_by_its_opening_control_word() {
        assert!(WordDocumentCore.sniff(br"{\rtf1\ansi rest"));
        assert!(!WordDocumentCore.sniff(b"not rtf"));
    }

    #[test]
    fn sniffs_docx_by_its_document_part_name() {
        let path = unique_temp_file("sniff.docx");
        write_test_docx(&path);
        let prefix = std::fs::read(&path).unwrap();

        assert!(WordDocumentCore.sniff(&prefix));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn sniffs_odt_by_its_mimetype_entry() {
        let path = unique_temp_file("sniff.odt");
        write_test_odt(&path);
        let prefix = std::fs::read(&path).unwrap();

        assert!(WordDocumentCore.sniff(&prefix));

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

        assert!(!WordDocumentCore.sniff(&prefix));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_real_docx_document() {
        let path = unique_temp_file("test.docx");
        write_test_docx(&path);

        let data = WordDocumentCore.view(&path).unwrap();
        let view: WordDocumentView = serde_json::from_value(data).unwrap();

        assert_eq!(
            view.paragraphs,
            vec!["Hello, Word & friends.", "Second paragraph."]
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_real_odt_document() {
        let path = unique_temp_file("test.odt");
        write_test_odt(&path);

        let data = WordDocumentCore.view(&path).unwrap();
        let view: WordDocumentView = serde_json::from_value(data).unwrap();

        assert_eq!(
            view.paragraphs,
            vec!["Hello, ODF world.", "Another paragraph."]
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_real_rtf_document_skipping_metadata_groups() {
        let path = unique_temp_file("test.rtf");
        write_test_rtf(&path);

        let data = WordDocumentCore.view(&path).unwrap();
        let view: WordDocumentView = serde_json::from_value(data).unwrap();

        assert_eq!(
            view.paragraphs,
            vec!["Hello, RTF world.", "Second paragraph."]
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_paragraphs_as_lines() {
        let data = serde_json::to_value(WordDocumentView {
            paragraphs: vec!["one".to_owned(), "two".to_owned()],
        })
        .unwrap();

        let lines = WordDocumentPresentation.present(&data);

        assert_eq!(lines, vec!["one", "two"]);
    }
}
