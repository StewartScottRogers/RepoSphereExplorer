//! Markdown file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// Returns the ATX heading level (1-6) of `line` — the count of leading
/// `#` characters — if `line` is a well-formed ATX heading (the hashes
/// followed by a space), or `None` otherwise. Rejects a bare `#[...]`
/// Rust attribute or a `#fff` colour literal, neither of which has a
/// space after the hashes.
fn atx_heading_level(line: &str) -> Option<usize> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    line[hashes..].starts_with(' ').then_some(hashes)
}

/// Whether `text` looks like a Markdown document: a fenced code block, an
/// ATX heading, a blockquote, or a Markdown link/image — markers not used
/// by any sibling plugin.
fn has_markdown_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("```")
            || trimmed.starts_with("~~~")
            || trimmed.starts_with("> ")
            || atx_heading_level(trimmed).is_some()
    }) || text.contains("](")
}

/// Extracts the text of the first level-1 ATX heading (`# Title`) in
/// `content`, or `None` if there isn't one.
fn parse_title(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let trimmed = line.trim_start();
        if atx_heading_level(trimmed) != Some(1) {
            return None;
        }
        let title = trimmed[1..].trim();
        (!title.is_empty()).then(|| title.to_owned())
    })
}

/// Extracts Markdown link and image destinations (the `url` in
/// `[text](url)` or `![alt](url)`) from `content`, in source order.
fn parse_links(content: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut rest = content;
    while let Some(found) = rest.find("](") {
        let paren_start = found + "](".len();
        let Some(end) = rest[paren_start..].find(')') else {
            break;
        };
        let url = &rest[paren_start..paren_start + end];
        if !url.is_empty() {
            links.push(url.to_owned());
        }
        rest = &rest[paren_start + end + 1..];
    }
    links
}

/// Replaces `[text](url)`/`![alt](url)` spans in `text` with `text (url)`.
fn inline_links(text: &str) -> String {
    let mut result = String::new();
    let mut rest = text;
    loop {
        let Some(bracket) = rest.find('[') else {
            result.push_str(rest);
            break;
        };
        let Some(close) = rest[bracket..].find("](") else {
            result.push_str(rest);
            break;
        };
        let label_end = bracket + close;
        let paren_start = label_end + "](".len();
        let Some(paren_end) = rest[paren_start..].find(')') else {
            result.push_str(rest);
            break;
        };
        let label = &rest[bracket + 1..label_end];
        let url = &rest[paren_start..paren_start + paren_end];
        result.push_str(&rest[..bracket]);
        result.push_str(label);
        result.push_str(" (");
        result.push_str(url);
        result.push(')');
        rest = &rest[paren_start + paren_end + 1..];
    }
    result
}

/// Strips the unambiguous inline Markdown markers — `**strong**`,
/// `__strong__`, and `` `code` `` — and inlines link/image syntax, leaving
/// single `*`/`_` emphasis alone since those characters are common enough
/// in ordinary prose (multiplication, `snake_case`) to strip safely.
fn render_inline(text: &str) -> String {
    let text = text.replace("**", "").replace("__", "").replace('`', "");
    inline_links(&text)
}

/// Turns raw Markdown `content` into rendered preview lines: headings are
/// underlined rather than hash-prefixed, fenced code blocks lose their
/// fence and are indented, blockquotes and list items get plain-text
/// markers, and inline emphasis/link syntax is resolved — a preview, not
/// the highlighted source the `text` plugin would show.
fn render_preview(content: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut in_code_block = false;
    for raw_line in content.lines() {
        let trimmed = raw_line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            lines.push(format!("    {raw_line}"));
            continue;
        }
        if let Some(level) = atx_heading_level(trimmed) {
            let heading = render_inline(trimmed[level..].trim());
            let underline_len = heading.chars().count().max(1);
            lines.push(heading);
            if level == 1 {
                lines.push("=".repeat(underline_len));
            } else if level == 2 {
                lines.push("-".repeat(underline_len));
            }
            continue;
        }
        if let Some(quote) = trimmed.strip_prefix("> ") {
            lines.push(format!("  | {}", render_inline(quote)));
            continue;
        }
        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
        {
            lines.push(format!("  \u{2022} {}", render_inline(item)));
            continue;
        }
        lines.push(render_inline(raw_line));
    }
    lines
}

/// View data produced by [`MarkdownCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkdownView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// The document's first level-1 heading text, if present.
    pub title: Option<String>,
    /// Link and image destinations, in source order.
    pub links: Vec<String>,
}

/// The Markdown plugin's core half.
#[derive(Debug, Default)]
pub struct MarkdownCore;

impl PluginCore for MarkdownCore {
    fn name(&self) -> &'static str {
        "markdown"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_markdown_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let title = parse_title(&content);
        let links = parse_links(&content);
        let view = MarkdownView {
            content,
            truncated,
            title,
            links,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Markdown plugin's presentation half.
#[derive(Debug, Default)]
pub struct MarkdownPresentation;

impl PluginPresentation for MarkdownPresentation {
    fn name(&self) -> &'static str {
        "markdown"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: MarkdownView = match serde_json::from_value(data.clone()) {
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
        lines.extend(render_preview(&view.content));
        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_VIEW_BYTES, MarkdownCore, MarkdownPresentation, MarkdownView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-markdown-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_markdown_markers_as_markdown() {
        assert!(MarkdownCore.sniff(b"# Heading\n\nSome text.\n"));
        assert!(MarkdownCore.sniff(b"## Subheading\n"));
        assert!(MarkdownCore.sniff(b"```rust\nfn main() {}\n```\n"));
        assert!(MarkdownCore.sniff(b"~~~\ncode\n~~~\n"));
        assert!(MarkdownCore.sniff(b"> a quoted line\n"));
        assert!(MarkdownCore.sniff(b"see [the docs](https://example.com) for more\n"));
        assert!(MarkdownCore.sniff(b"![alt text](image.png)\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_markdown() {
        assert!(!MarkdownCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!MarkdownCore.sniff(b"#[derive(Debug)]\nstruct Foo;\n"));
        assert!(!MarkdownCore.sniff(b"<!DOCTYPE html>\n<html></html>\n"));
        assert!(!MarkdownCore.sniff(b"just a regular line of text\n"));
        assert!(!MarkdownCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_markdown_file_and_extracts_title_and_links() {
        let path = unique_temp_file("doc.md");
        std::fs::write(
            &path,
            "# My Document\n\nSee [the site](https://example.com) or ![a diagram](diagram.png).\n",
        )
        .unwrap();

        let data = MarkdownCore.view(&path).unwrap();
        let view: MarkdownView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.title.as_deref(), Some("My Document"));
        assert_eq!(
            view.links,
            vec!["https://example.com".to_owned(), "diagram.png".to_owned()]
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.md");
        let mut content = "# Heading\n\n".to_owned();
        content.push_str(&"a paragraph of body text\n".repeat(MAX_VIEW_BYTES));
        std::fs::write(&path, content).unwrap();

        let data = MarkdownCore.view(&path).unwrap();
        let view: MarkdownView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn renders_a_preview_distinct_from_the_raw_source() {
        let data = serde_json::to_value(MarkdownView {
            content: "# Title\n\nSome **bold** and `code` and [a link](https://example.com).\n\n- one\n- two\n\n> quoted\n\n```\nlet x = 1;\n```\n".to_owned(),
            truncated: false,
            title: Some("Title".to_owned()),
            links: vec!["https://example.com".to_owned()],
        })
        .unwrap();

        let lines = MarkdownPresentation.present(&data);

        assert_eq!(lines[0], "title: Title");
        assert_eq!(lines[1], "links: https://example.com");
        assert_eq!(lines[2], "Title");
        assert_eq!(lines[3], "=====");
        assert_eq!(lines[4], "");
        assert_eq!(
            lines[5],
            "Some bold and code and a link (https://example.com)."
        );
        assert_eq!(lines[6], "");
        assert_eq!(lines[7], "  \u{2022} one");
        assert_eq!(lines[8], "  \u{2022} two");
        assert_eq!(lines[9], "");
        assert_eq!(lines[10], "  | quoted");
        assert_eq!(lines[11], "");
        assert_eq!(lines[12], "    let x = 1;");
        assert!(!lines.iter().any(|line| line.contains("```")));
        assert!(!lines.iter().any(|line| line.starts_with('#')));
    }
}
