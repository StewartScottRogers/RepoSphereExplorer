//! XML file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// XML document markers, checked case-insensitively anywhere in the
/// content — markers not used by any sibling plugin. Deliberately does not
/// sniff bare document-structure tags (e.g. `<root>`), since those overlap
/// with the HTML plugin's tag-based checks and with markup embedded in
/// other formats; placed just after `html` in `CORE_PLUGINS` per that
/// plugin's own note, so a real HTML document is claimed first.
const XML_MARKERS: &[&str] = &["<?xml", "<![cdata[", "xmlns=\"", "xmlns:"];

/// View data produced by [`XmlCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XmlView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// The document's root element name, if present.
    pub root_element: Option<String>,
    /// `xmlns`/`xmlns:prefix` namespace URIs, in source order.
    pub namespaces: Vec<String>,
}

/// Whether `name` is a valid XML name start character: a letter, `_`, or `:`.
fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == ':'
}

/// Whether `name` is a valid XML name continuation character.
fn is_name_char(c: char) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == '-' || c == '.'
}

/// Extracts the name of the first element tag in `content` that is not a
/// processing instruction (`<?...?>`), comment (`<!--...-->`), or `<!DOCTYPE
/// ...>` declaration, or `None` if no such tag exists.
fn parse_root_element(content: &str) -> Option<String> {
    let mut rest = content;
    loop {
        let start = rest.find('<')?;
        rest = &rest[start..];
        if rest.starts_with("<?") {
            let end = rest.find("?>")? + 2;
            rest = &rest[end..];
        } else if rest.starts_with("<!--") {
            let end = rest.find("-->")? + 3;
            rest = &rest[end..];
        } else if rest.starts_with("<!") {
            let end = rest.find('>')? + 1;
            rest = &rest[end..];
        } else {
            let name: String = rest[1..].chars().take_while(|c| is_name_char(*c)).collect();
            return (!name.is_empty()).then_some(name);
        }
    }
}

/// Extracts `xmlns="..."`/`xmlns:prefix="..."` attribute values from
/// `content`, in source order.
fn parse_namespaces(content: &str) -> Vec<String> {
    let mut namespaces = Vec::new();
    let mut search_from = 0;
    while let Some(found) = content[search_from..].find("xmlns") {
        let after_xmlns = search_from + found + "xmlns".len();
        let attr_start = match content[after_xmlns..].chars().next() {
            Some('=') => after_xmlns + 1,
            Some(':') => match content[after_xmlns + 1..].find('=') {
                Some(offset) => after_xmlns + 1 + offset + 1,
                None => break,
            },
            _ => {
                search_from = after_xmlns;
                continue;
            }
        };
        let Some(quote) = content[attr_start..].chars().next() else {
            break;
        };
        if quote == '"' || quote == '\'' {
            let value_start = attr_start + 1;
            if let Some(end) = content[value_start..].find(quote) {
                namespaces.push(content[value_start..value_start + end].to_owned());
                search_from = value_start + end + 1;
                continue;
            }
        }
        search_from = attr_start;
    }
    namespaces
}

/// Whether `text` looks like an XML document, per [`XML_MARKERS`].
fn has_xml_syntax(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    XML_MARKERS.iter().any(|marker| lower.contains(marker))
}

/// The XML plugin's core half.
#[derive(Debug, Default)]
pub struct XmlCore;

impl PluginCore for XmlCore {
    fn name(&self) -> &'static str {
        "xml"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_xml_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let root_element = parse_root_element(&content);
        let namespaces = parse_namespaces(&content);
        let view = XmlView {
            content,
            truncated,
            root_element,
            namespaces,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The XML plugin's presentation half.
#[derive(Debug, Default)]
pub struct XmlPresentation;

impl PluginPresentation for XmlPresentation {
    fn name(&self) -> &'static str {
        "xml"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: XmlView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(root_element) = &view.root_element {
            lines.push(format!("root element: {root_element}"));
        }
        if !view.namespaces.is_empty() {
            lines.push(format!("namespaces: {}", view.namespaces.join(", ")));
        }
        lines.extend(view.content.lines().map(str::to_owned));
        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_VIEW_BYTES, XmlCore, XmlPresentation, XmlView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-xml-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_xml_markers_as_xml() {
        assert!(XmlCore.sniff(b"<?xml version=\"1.0\"?>\n<root></root>\n"));
        assert!(XmlCore.sniff(b"<?XML version=\"1.0\"?>\n<root></root>\n"));
        assert!(XmlCore.sniff(b"<root>\n<![CDATA[hi]]>\n</root>\n"));
        assert!(XmlCore.sniff(b"<root xmlns=\"urn:example\">\n</root>\n"));
        assert!(XmlCore.sniff(b"<ns:root xmlns:ns=\"urn:example\">\n</ns:root>\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_html_as_xml() {
        assert!(!XmlCore.sniff(b"<!DOCTYPE html>\n<html>\n</html>\n"));
        assert!(!XmlCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!XmlCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!XmlCore.sniff(b"just a regular line of text\n"));
        assert!(!XmlCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_xml_file_and_extracts_root_element_and_namespaces() {
        let path = unique_temp_file("doc.xml");
        std::fs::write(
            &path,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<catalog xmlns=\"urn:example:catalog\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n<book id=\"1\">Title</book>\n</catalog>\n",
        )
        .unwrap();

        let data = XmlCore.view(&path).unwrap();
        let view: XmlView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.root_element.as_deref(), Some("catalog"));
        assert_eq!(
            view.namespaces,
            vec![
                "urn:example:catalog".to_owned(),
                "http://purl.org/dc/elements/1.1/".to_owned()
            ]
        );
        assert!(view.content.contains("Title"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn parses_root_element_past_a_doctype_and_comment() {
        let path = unique_temp_file("with-doctype.xml");
        std::fs::write(
            &path,
            "<?xml version=\"1.0\"?>\n<!DOCTYPE catalog SYSTEM \"catalog.dtd\">\n<!-- a comment -->\n<catalog>\n</catalog>\n",
        )
        .unwrap();

        let data = XmlCore.view(&path).unwrap();
        let view: XmlView = serde_json::from_value(data).unwrap();

        assert_eq!(view.root_element.as_deref(), Some("catalog"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.xml");
        let mut content = "<?xml version=\"1.0\"?>\n<root>\n".to_owned();
        content.push_str(&"<item>a value</item>\n".repeat(MAX_VIEW_BYTES));
        std::fs::write(&path, content).unwrap();

        let data = XmlCore.view(&path).unwrap();
        let view: XmlView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_root_element_namespaces_and_content() {
        let data = serde_json::to_value(XmlView {
            content: "<root>hi</root>".to_owned(),
            truncated: false,
            root_element: Some("root".to_owned()),
            namespaces: vec!["urn:example".to_owned()],
        })
        .unwrap();

        let lines = XmlPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "root element: root",
                "namespaces: urn:example",
                "<root>hi</root>"
            ]
        );
    }
}
