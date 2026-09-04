//! HTML file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// HTML document markers, checked case-insensitively anywhere in the
/// content — markers not used by any sibling plugin. Deliberately does not
/// sniff bare `<div`/`<script`/`<style` tags, since those can appear inside
/// a JavaScript template literal or another markup-adjacent format; a
/// future XML or SVG plugin sniffing those same document-structure tags
/// will need its own stronger markers, or to be ordered after `html`.
const HTML_MARKERS: &[&str] = &[
    "<!doctype html",
    "<html",
    "</html>",
    "<head>",
    "<head ",
    "</head>",
    "<body>",
    "<body ",
    "</body>",
    "<title>",
];

/// View data produced by [`HtmlCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HtmlView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// The document's `<title>` text, if present.
    pub title: Option<String>,
    /// `href` targets from anchor tags, in source order.
    pub links: Vec<String>,
}

/// Extracts the text between the first `<title>` and `</title>` tags in
/// `content`, matched case-insensitively, or `None` if absent.
fn parse_title(content: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    let start = lower.find("<title>")? + "<title>".len();
    let end = lower[start..].find("</title>")? + start;
    let title = content[start..end].trim();
    (!title.is_empty()).then(|| title.to_owned())
}

/// Extracts `href="..."`/`href='...'` attribute values from `content`, in
/// source order, matched case-insensitively.
fn parse_links(content: &str) -> Vec<String> {
    let lower = content.to_ascii_lowercase();
    let mut links = Vec::new();
    let mut search_from = 0;
    while let Some(found) = lower[search_from..].find("href=") {
        let attr_start = search_from + found + "href=".len();
        let Some(quote) = content[attr_start..].chars().next() else {
            break;
        };
        if quote == '"' || quote == '\'' {
            let value_start = attr_start + 1;
            if let Some(end) = content[value_start..].find(quote) {
                links.push(content[value_start..value_start + end].to_owned());
                search_from = value_start + end + 1;
                continue;
            }
        }
        search_from = attr_start;
    }
    links
}

/// Whether `text` looks like an HTML document, per [`HTML_MARKERS`].
fn has_html_syntax(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    HTML_MARKERS.iter().any(|marker| lower.contains(marker))
}

/// The HTML plugin's core half.
#[derive(Debug, Default)]
pub struct HtmlCore;

impl PluginCore for HtmlCore {
    fn name(&self) -> &'static str {
        "html"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_html_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let title = parse_title(&content);
        let links = parse_links(&content);
        let view = HtmlView {
            content,
            truncated,
            title,
            links,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The HTML plugin's presentation half.
#[derive(Debug, Default)]
pub struct HtmlPresentation;

impl PluginPresentation for HtmlPresentation {
    fn name(&self) -> &'static str {
        "html"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: HtmlView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(title) = &view.title {
            lines.push(format!("title: {title}"));
        }
        if !view.links.is_empty() {
            lines.push(format!("links: {}", view.links.join(", ")));
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
    use super::{HtmlCore, HtmlPresentation, HtmlView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-html-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_html_markers_as_html() {
        assert!(HtmlCore.sniff(b"<!DOCTYPE html>\n<html>\n</html>\n"));
        assert!(HtmlCore.sniff(b"<!doctype html>\n<html>\n</html>\n"));
        assert!(HtmlCore.sniff(b"<html lang=\"en\">\n<head></head>\n</html>\n"));
        assert!(HtmlCore.sniff(b"<head>\n<title>Hi</title>\n</head>\n"));
        assert!(HtmlCore.sniff(b"<body class=\"main\">\nhi\n</body>\n"));
        assert!(HtmlCore.sniff(b"<title>Just a title</title>\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_html() {
        assert!(!HtmlCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!HtmlCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!HtmlCore.sniff(b"<?xml version=\"1.0\"?>\n<root></root>\n"));
        assert!(!HtmlCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!HtmlCore.sniff(b"just a regular line of text\n"));
        assert!(!HtmlCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_html_file_and_extracts_title_and_links() {
        let path = unique_temp_file("page.html");
        std::fs::write(
            &path,
            "<!DOCTYPE html>\n<html>\n<head><title>My Page</title></head>\n<body>\n<a href=\"https://example.com\">example</a>\n<a href='/about'>about</a>\n</body>\n</html>\n",
        )
        .unwrap();

        let data = HtmlCore.view(&path).unwrap();
        let view: HtmlView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.title.as_deref(), Some("My Page"));
        assert_eq!(view.links, vec!["https://example.com", "/about"]);
        assert!(view.content.contains("example"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.html");
        let mut content = "<!DOCTYPE html>\n<html><body>\n".to_owned();
        content.push_str(&"<p>a paragraph</p>\n".repeat(MAX_VIEW_BYTES));
        std::fs::write(&path, content).unwrap();

        let data = HtmlCore.view(&path).unwrap();
        let view: HtmlView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_title_links_and_content() {
        let data = serde_json::to_value(HtmlView {
            content: "<html><body>hi</body></html>".to_owned(),
            truncated: false,
            title: Some("My Page".to_owned()),
            links: vec!["/about".to_owned()],
        })
        .unwrap();

        let lines = HtmlPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "title: My Page",
                "links: /about",
                "<html><body>hi</body></html>"
            ]
        );
    }
}
