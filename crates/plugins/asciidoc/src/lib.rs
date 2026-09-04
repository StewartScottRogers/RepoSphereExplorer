//! `AsciiDoc` file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// Admonition labels `AsciiDoc` recognizes at the start of a paragraph —
/// markers not used by any sibling plugin.
const ADMONITION_LABELS: &[&str] = &["NOTE:", "TIP:", "IMPORTANT:", "WARNING:", "CAUTION:"];

/// View data produced by [`AsciiDocCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsciiDocView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Titles of the document title and section headings (`= Title`,
    /// `== Section`, ...) found in the content, in source order.
    pub headings: Vec<String>,
}

/// Extracts the heading text from a line beginning with one to six `=`
/// characters followed by a space (an `AsciiDoc` document title or section
/// heading), or `None` if `line` is not such a heading.
fn parse_heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let equals_len = trimmed.chars().take_while(|&ch| ch == '=').count();
    if equals_len == 0 || equals_len > 6 {
        return None;
    }
    let title = trimmed[equals_len..].strip_prefix(' ')?.trim();
    (!title.is_empty()).then(|| title.to_owned())
}

/// Parses document title and section heading text out of `content`, in
/// source order.
fn parse_headings(content: &str) -> Vec<String> {
    content.lines().filter_map(parse_heading).collect()
}

/// Whether `line` is an `AsciiDoc` attribute entry, e.g. `:toc:` or
/// `:author: Jane Doe`: a `:` at the start of the line, followed by a
/// non-empty run of alphanumerics/`-`/`!`, followed by a closing `:`.
fn is_attribute_entry(line: &str) -> bool {
    let Some(rest) = line.strip_prefix(':') else {
        return false;
    };
    let name_len = rest
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || *ch == '-' || *ch == '!')
        .count();
    name_len > 0 && rest[name_len..].starts_with(':')
}

/// Whether `text` looks like `AsciiDoc`: a document title or section heading
/// (one to six `=` characters followed by a space), an attribute entry
/// (`:name:` at the start of a line), an admonition label, or an
/// `include::` directive — markers not used by any sibling plugin.
fn has_asciidoc_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        parse_heading(line).is_some()
            || is_attribute_entry(trimmed)
            || ADMONITION_LABELS
                .iter()
                .any(|label| trimmed.starts_with(label))
            || trimmed.starts_with("include::")
    })
}

/// The `AsciiDoc` plugin's core half.
#[derive(Debug, Default)]
pub struct AsciiDocCore;

impl PluginCore for AsciiDocCore {
    fn name(&self) -> &'static str {
        "asciidoc"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_asciidoc_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let headings = parse_headings(&content);
        let view = AsciiDocView {
            content,
            truncated,
            headings,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The `AsciiDoc` plugin's presentation half.
#[derive(Debug, Default)]
pub struct AsciiDocPresentation;

impl PluginPresentation for AsciiDocPresentation {
    fn name(&self) -> &'static str {
        "asciidoc"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: AsciiDocView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.headings.is_empty() {
            lines.push(format!("headings: {}", view.headings.join(", ")));
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
    use super::{AsciiDocCore, AsciiDocPresentation, AsciiDocView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-asciidoc-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_asciidoc_syntax_as_asciidoc() {
        assert!(AsciiDocCore.sniff(b"= Document Title\nAuthor Name\n\nSome text.\n"));
        assert!(AsciiDocCore.sniff(b"== A Section\n\nSome text.\n"));
        assert!(AsciiDocCore.sniff(b":toc:\n:author: Jane Doe\n\nSome text.\n"));
        assert!(AsciiDocCore.sniff(b"Some text.\n\nNOTE: this is important.\n"));
        assert!(AsciiDocCore.sniff(b"Some text.\n\ninclude::chapter1.adoc[]\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_asciidoc() {
        assert!(!AsciiDocCore.sniff(b"let x = 5;\nprintln!(\"{x}\");\n"));
        assert!(!AsciiDocCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!AsciiDocCore.sniff(b"# A Markdown Heading\n\nSome text.\n"));
        assert!(!AsciiDocCore.sniff(b"just a regular line of text\n"));
        assert!(!AsciiDocCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_asciidoc_file_and_extracts_headings() {
        let path = unique_temp_file("doc.adoc");
        std::fs::write(
            &path,
            "= Document Title\n:toc:\n\n== Introduction\n\nSome intro text.\n\n=== Background\n\nMore text.\n\nNOTE: worth remembering.\n",
        )
        .unwrap();

        let data = AsciiDocCore.view(&path).unwrap();
        let view: AsciiDocView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(
            view.headings,
            vec!["Document Title", "Introduction", "Background"]
        );
        assert!(view.content.contains("NOTE: worth remembering."));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.adoc");
        let mut content = "= Document Title\n\n".to_owned();
        content.push_str(&"a ".repeat(MAX_VIEW_BYTES));
        std::fs::write(&path, content).unwrap();

        let data = AsciiDocCore.view(&path).unwrap();
        let view: AsciiDocView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_headings_and_content() {
        let data = serde_json::to_value(AsciiDocView {
            content: "= Title\n\nBody text.".to_owned(),
            truncated: false,
            headings: vec!["Title".to_owned()],
        })
        .unwrap();

        let lines = AsciiDocPresentation.present(&data);

        assert_eq!(lines, vec!["headings: Title", "= Title", "", "Body text."]);
    }
}
