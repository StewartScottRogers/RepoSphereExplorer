//! reStructuredText file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`RestructuredTextCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestructuredTextView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// The document's first section title, if present.
    pub title: Option<String>,
    /// Directive names (e.g. `code-block`, `note`, `image`) used in the
    /// document, in source order.
    pub directives: Vec<String>,
}

/// Whether `line` is a reStructuredText section-title underline (or
/// overline): two or more repeats of the same punctuation character, with
/// nothing else on the line.
fn is_section_underline(line: &str) -> bool {
    let trimmed = line.trim_end();
    if trimmed.chars().count() < 3 {
        return false;
    }
    let mut chars = trimmed.chars();
    let first = chars.next().expect("checked non-empty above");
    first.is_ascii_punctuation() && chars.all(|c| c == first)
}

/// Extracts the document's first section title: a non-blank line
/// immediately followed by a matching [`is_section_underline`] line at
/// least as long as the title text.
fn parse_title(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    lines.windows(2).find_map(|pair| {
        let title = pair[0].trim();
        let underline = pair[1].trim_end();
        (!title.is_empty() && is_section_underline(underline) && underline.len() >= title.len())
            .then(|| title.to_owned())
    })
}

/// Extracts directive names (the text between `.. ` and `::`) from
/// `content`, in source order. Also matches substitution definitions
/// (`.. |name| replace::`), but not hyperlink targets (`.. _label:`), which
/// have no `::`.
fn parse_directives(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix(".. ")?;
            let end = rest.find("::")?;
            let name = rest[..end].trim();
            (!name.is_empty()).then(|| name.to_owned())
        })
        .collect()
}

/// Whether `text` contains a directive marker (`.. name::`), the one marker
/// not used by any sibling plugin.
fn has_rst_directive(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with(".. ") && trimmed.contains("::")
    })
}

/// Whether `text` contains a hyperlink target (`.. _label:`).
fn has_rst_hyperlink_target(text: &str) -> bool {
    text.lines()
        .any(|line| line.trim_start().starts_with(".. _"))
}

/// Whether `text` looks like reStructuredText: a directive, a hyperlink
/// target, or a section title/underline pair. Deliberately does not sniff a
/// bare underline line on its own, since a future Markdown plugin's setext
/// headers use the same `===`/`---` adornment; requiring the preceding title
/// line here already avoids most such false positives, and this plugin
/// would need to be reordered against a Markdown plugin sniffing the same
/// pattern if one is added later.
fn has_rst_syntax(text: &str) -> bool {
    has_rst_directive(text) || has_rst_hyperlink_target(text) || parse_title(text).is_some()
}

/// The reStructuredText plugin's core half.
#[derive(Debug, Default)]
pub struct RestructuredTextCore;

impl PluginCore for RestructuredTextCore {
    fn name(&self) -> &'static str {
        "restructuredtext"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_rst_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let title = parse_title(&content);
        let directives = parse_directives(&content);
        let view = RestructuredTextView {
            content,
            truncated,
            title,
            directives,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The reStructuredText plugin's presentation half.
#[derive(Debug, Default)]
pub struct RestructuredTextPresentation;

impl PluginPresentation for RestructuredTextPresentation {
    fn name(&self) -> &'static str {
        "restructuredtext"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: RestructuredTextView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(title) = &view.title {
            lines.push(format!("title: {title}"));
        }
        if !view.directives.is_empty() {
            lines.push(format!("directives: {}", view.directives.join(", ")));
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
    use super::{
        MAX_VIEW_BYTES, RestructuredTextCore, RestructuredTextPresentation, RestructuredTextView,
    };
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-restructuredtext-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_rst_markers_as_restructuredtext() {
        assert!(RestructuredTextCore.sniff(b"Title\n=====\n\nbody text\n"));
        assert!(RestructuredTextCore.sniff(b".. code-block:: rust\n\n    fn main() {}\n"));
        assert!(RestructuredTextCore.sniff(b".. note::\n\n   Careful!\n"));
        assert!(RestructuredTextCore.sniff(b".. _my-label:\n\nSee `my-label`_.\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_restructuredtext() {
        assert!(!RestructuredTextCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!RestructuredTextCore.sniff(b"<!DOCTYPE html>\n<html></html>\n"));
        assert!(!RestructuredTextCore.sniff(b"<?xml version=\"1.0\"?>\n<root></root>\n"));
        assert!(!RestructuredTextCore.sniff(b"just a regular line of text\n"));
        assert!(!RestructuredTextCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_rst_file_and_extracts_title_and_directives() {
        let path = unique_temp_file("doc.rst");
        std::fs::write(
            &path,
            "My Document\n===========\n\n.. code-block:: rust\n\n    fn main() {}\n\n.. note::\n\n   Careful!\n",
        )
        .unwrap();

        let data = RestructuredTextCore.view(&path).unwrap();
        let view: RestructuredTextView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.title.as_deref(), Some("My Document"));
        assert_eq!(view.directives, vec!["code-block", "note"]);
        assert!(view.content.contains("fn main"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.rst");
        let mut content = "Title\n=====\n\n".to_owned();
        content.push_str(&"a paragraph of body text\n".repeat(MAX_VIEW_BYTES));
        std::fs::write(&path, content).unwrap();

        let data = RestructuredTextCore.view(&path).unwrap();
        let view: RestructuredTextView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_title_directives_and_content() {
        let data = serde_json::to_value(RestructuredTextView {
            content: "My Document\n===========\n\nhi\n".to_owned(),
            truncated: false,
            title: Some("My Document".to_owned()),
            directives: vec!["note".to_owned()],
        })
        .unwrap();

        let lines = RestructuredTextPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "title: My Document",
                "directives: note",
                "My Document",
                "===========",
                "",
                "hi"
            ]
        );
    }
}
