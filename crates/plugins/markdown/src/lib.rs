//! Markdown file type plugin: core and presentation halves.
//!
//! Markdown has no magic bytes, and a `#` at the start of a line is a
//! comment in half the languages this project already sniffs. So the
//! markers here are the ones only Markdown has: a fenced code block, an
//! inline link, a task-list item, a table delimiter row, or a setext
//! underline. The extension settles what is left.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["md", "markdown", "mdown", "mkd"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One heading, and how deep it sits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heading {
    /// 1 for `#`, 6 for `######`.
    pub level: u8,
    /// The heading text, with its marker and trailing hashes removed.
    pub text: String,
}

/// One fenced code block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeBlock {
    /// The info string after the fence, which is conventionally a language.
    /// Empty when the fence carried none.
    pub language: String,
    /// How many lines the block holds, not counting its fences.
    pub lines: usize,
}

/// One link or image reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    /// The bracketed text, or the alt text of an image.
    pub text: String,
    /// What it points at.
    pub target: String,
    /// Whether it was written as an image (`![alt](src)`).
    pub image: bool,
}

/// One task-list item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Whether the box is ticked.
    pub done: bool,
    /// The text after the box.
    pub text: String,
}

/// View data produced by [`MarkdownCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkdownView {
    /// The document's title: its first level-one heading, or the `title`
    /// key of its front matter when it has no heading.
    pub title: Option<String>,
    /// Every heading, in document order.
    pub headings: Vec<Heading>,
    /// Every fenced code block, in document order.
    pub code_blocks: Vec<CodeBlock>,
    /// Every inline link and image, in document order.
    pub links: Vec<Link>,
    /// Every task-list item, in document order.
    pub tasks: Vec<Task>,
    /// The YAML front matter between the opening and closing `---`, when
    /// the document opens with one.
    pub front_matter: Option<String>,
    /// The file's text, so the plain view can show it and the editor can
    /// take it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Whether `line` opens or closes a fenced code block, and its info string.
fn fence(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    for marker in ["```", "~~~"] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            return Some(rest.trim());
        }
    }
    None
}

/// The heading level and text of an ATX heading (`## Text`), if `line` is
/// one. Requires the space `CommonMark` requires, which is what keeps a
/// `#!/bin/sh` line and a `#include` from reading as headings.
fn atx_heading(line: &str) -> Option<(u8, String)> {
    let trimmed = line.trim_start();
    let hashes = trimmed.len() - trimmed.trim_start_matches('#').len();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.starts_with(' ') {
        return None;
    }
    let text = rest.trim().trim_end_matches('#').trim().to_owned();
    u8::try_from(hashes).ok().map(|level| (level, text))
}

/// Whether `line` is a setext underline, and the level it gives the line
/// above: `===` makes it a level one, `---` a level two.
fn setext_level(line: &str) -> Option<u8> {
    let trimmed = line.trim();
    if trimmed.len() < 2 {
        return None;
    }
    if trimmed.chars().all(|c| c == '=') {
        return Some(1);
    }
    if trimmed.chars().all(|c| c == '-') {
        return Some(2);
    }
    None
}

/// The state and text of a task-list item, if `line` is one.
fn task_item(line: &str) -> Option<(bool, String)> {
    let trimmed = line.trim_start();
    let rest = ["- ", "* ", "+ "]
        .iter()
        .find_map(|marker| trimmed.strip_prefix(marker))?;
    let done = if rest.starts_with("[ ]") {
        false
    } else if rest.starts_with("[x]") || rest.starts_with("[X]") {
        true
    } else {
        return None;
    };
    Some((done, rest[3..].trim().to_owned()))
}

/// Whether `line` is a table's delimiter row, e.g. `| --- | :--: |`.
fn is_table_delimiter(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.contains('-')
        && trimmed.starts_with('|')
        && trimmed.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// Every inline link and image in `line`, in the order they appear.
///
/// Hand-scanned rather than pattern-matched: a link's text may itself hold
/// brackets, and the nesting is what a scanner tracks and a flat pattern
/// does not.
fn links_in(line: &str) -> Vec<Link> {
    let bytes: Vec<char> = line.chars().collect();
    let mut found = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != '[' {
            index += 1;
            continue;
        }
        let image = index > 0 && bytes[index - 1] == '!';
        let mut depth = 0usize;
        let mut close = None;
        for (offset, &c) in bytes.iter().enumerate().skip(index) {
            match c {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else { break };
        if bytes.get(close + 1) != Some(&'(') {
            index = close + 1;
            continue;
        }
        let Some(end) = bytes.iter().skip(close + 2).position(|&c| c == ')') else {
            break;
        };
        let end = close + 2 + end;
        found.push(Link {
            text: bytes[index + 1..close].iter().collect(),
            target: bytes[close + 2..end]
                .iter()
                .collect::<String>()
                .trim()
                .to_owned(),
            image,
        });
        index = end + 1;
    }
    found
}

/// Everything [`MarkdownView`] holds, read from `text`.
fn parse(text: &str) -> MarkdownView {
    let lines: Vec<&str> = text.lines().collect();
    let mut view = MarkdownView {
        title: None,
        headings: Vec::new(),
        code_blocks: Vec::new(),
        links: Vec::new(),
        tasks: Vec::new(),
        front_matter: None,
        content: String::new(),
        truncated: false,
    };

    let mut index = 0;

    // Front matter, if the very first line opens one.
    if lines.first().is_some_and(|line| line.trim() == "---")
        && let Some(close) = lines.iter().skip(1).position(|line| {
            let trimmed = line.trim();
            trimmed == "---" || trimmed == "..."
        })
    {
        view.front_matter = Some(lines[1..=close].join(
            "
",
        ));
        index = close + 2;
    }

    let mut in_fence: Option<(String, usize)> = None;
    while index < lines.len() {
        let line = lines[index];

        if let Some((language, count)) = in_fence.take() {
            if fence(line).is_some() {
                view.code_blocks.push(CodeBlock {
                    language,
                    lines: count,
                });
            } else {
                in_fence = Some((language, count + 1));
            }
            index += 1;
            continue;
        }
        if let Some(info) = fence(line) {
            in_fence = Some((info.split_whitespace().next().unwrap_or("").to_owned(), 0));
            index += 1;
            continue;
        }

        if let Some((level, heading)) = atx_heading(line) {
            view.headings.push(Heading {
                level,
                text: heading,
            });
        } else if let Some(level) = setext_level(line) {
            // Only an underline if there is text above it to underline,
            // and that text is not itself a list item or a heading.
            if let Some(above) = index.checked_sub(1).map(|i| lines[i].trim())
                && !above.is_empty()
                && atx_heading(above).is_none()
                && task_item(above).is_none()
                && !above.starts_with('-')
            {
                view.headings.push(Heading {
                    level,
                    text: above.to_owned(),
                });
            }
        }

        if let Some((done, text)) = task_item(line) {
            view.tasks.push(Task { done, text });
        }
        view.links.extend(links_in(line));
        index += 1;
    }

    // An unterminated fence still describes a block a reader can see.
    if let Some((language, count)) = in_fence {
        view.code_blocks.push(CodeBlock {
            language,
            lines: count,
        });
    }

    view.title = view
        .headings
        .iter()
        .find(|heading| heading.level == 1)
        .map(|heading| heading.text.clone())
        .or_else(|| {
            view.front_matter.as_ref().and_then(|matter| {
                matter.lines().find_map(|line| {
                    line.strip_prefix("title:")
                        .map(|value| value.trim().trim_matches('"').to_owned())
                })
            })
        });
    view
}

/// Whether `prefix` looks like Markdown.
///
/// Every marker here is one no sibling plugin claims. An ATX heading is
/// deliberately *not* enough on its own: `# comment` opens a line in a
/// shell script, a Python module and a dozen configuration formats.
fn looks_like_markdown(prefix: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(prefix) else {
        return false;
    };
    let lines: Vec<&str> = text.lines().collect();
    let mut headings = 0usize;

    for (index, line) in lines.iter().enumerate() {
        if fence(line).is_some() || is_table_delimiter(line) || task_item(line).is_some() {
            return true;
        }
        if !links_in(line).is_empty() {
            return true;
        }
        if atx_heading(line).is_some() {
            headings += 1;
        }
        if setext_level(line).is_some()
            && index > 0
            && !lines[index - 1].trim().is_empty()
            && !lines[index - 1].trim().starts_with('-')
        {
            return true;
        }
    }
    // Several headings and nothing that contradicts them is Markdown; one
    // is a comment.
    headings >= 2
}

/// The Markdown plugin's core half.
#[derive(Debug, Default)]
pub struct MarkdownCore;

impl PluginCore for MarkdownCore {
    fn name(&self) -> &'static str {
        "markdown"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_markdown(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let mut view = parse(&content);
        view.content = content;
        view.truncated = truncated;
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

    fn icon(&self) -> Icon {
        Icon {
            label: "MD",
            tint: 0x0008_3fa1,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: MarkdownView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();

        if let Some(title) = &view.title {
            lines.push(format!("Title: {title}"));
        }
        if view.front_matter.is_some() {
            lines.push("Front matter: yes".to_owned());
        }

        if !view.headings.is_empty() {
            lines.push(format!("Outline ({}):", view.headings.len()));
            for heading in &view.headings {
                let indent = "  ".repeat(usize::from(heading.level));
                lines.push(format!("{indent}{}", heading.text));
            }
        }

        if !view.code_blocks.is_empty() {
            lines.push(format!("Code blocks ({}):", view.code_blocks.len()));
            for block in &view.code_blocks {
                let language = if block.language.is_empty() {
                    "no language"
                } else {
                    &block.language
                };
                lines.push(format!("  {language}, {} line(s)", block.lines));
            }
        }

        if !view.tasks.is_empty() {
            let done = view.tasks.iter().filter(|task| task.done).count();
            lines.push(format!("Tasks ({done} of {} done):", view.tasks.len()));
            for task in &view.tasks {
                let box_ = if task.done { "x" } else { " " };
                lines.push(format!("  [{box_}] {}", task.text));
            }
        }

        if !view.links.is_empty() {
            let images = view.links.iter().filter(|link| link.image).count();
            lines.push(format!("Links ({}, {images} image(s)):", view.links.len()));
            for link in &view.links {
                lines.push(format!("  {} -> {}", link.text, link.target));
            }
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{MarkdownCore, MarkdownPresentation, MarkdownView, links_in, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_the_markers_only_markdown_has() {
        assert!(MarkdownCore.sniff(b"see [the guide](guide.md) for more\n"));
        assert!(MarkdownCore.sniff(b"```rust\nfn main() {}\n```\n"));
        assert!(MarkdownCore.sniff(b"- [ ] write it\n- [x] test it\n"));
        assert!(MarkdownCore.sniff(b"| a | b |\n| --- | --- |\n"));
        assert!(MarkdownCore.sniff(b"Title\n=====\n"));
        assert!(MarkdownCore.sniff(b"# One\n\n## Two\n"));
    }

    #[test]
    fn does_not_claim_a_hash_comment_from_the_language_that_owns_it() {
        // A single `# ` line is a comment in a shell script, a Python
        // module and most configuration formats. Claiming those is how a C
        // file came to open as Rust (#272).
        assert!(!MarkdownCore.sniff(b"#!/bin/sh\n# set up the thing\nset -eu\n"));
        assert!(!MarkdownCore.sniff(b"# the port to listen on\nport = 8080\n"));
        assert!(!MarkdownCore.sniff(b"#include <stdio.h>\n"));
        assert!(!MarkdownCore.sniff(b"just a line of prose\n"));
        assert!(!MarkdownCore.sniff(b""));
        assert!(!MarkdownCore.sniff(&[0xFF, 0xFE, 0x00]));
    }

    #[test]
    fn reads_the_heading_outline_with_its_levels() {
        let view = parse("# One\n\n## Two\n\n### Three\n\nSetext\n------\n");

        assert_eq!(
            view.headings
                .iter()
                .map(|h| (h.level, h.text.as_str()))
                .collect::<Vec<_>>(),
            vec![(1, "One"), (2, "Two"), (3, "Three"), (2, "Setext")]
        );
        assert_eq!(view.title.as_deref(), Some("One"));
    }

    #[test]
    fn reads_fenced_blocks_and_their_languages() {
        let view = parse("```rust\nfn main() {}\n```\n\n~~~\nplain\n~~~\n");

        assert_eq!(view.code_blocks.len(), 2);
        assert_eq!(view.code_blocks[0].language, "rust");
        assert_eq!(view.code_blocks[0].lines, 1);
        assert_eq!(view.code_blocks[1].language, "");
    }

    #[test]
    fn a_heading_inside_a_fence_is_code_not_a_heading() {
        let view = parse("# Real\n\n```sh\n# not a heading\n```\n");

        assert_eq!(view.headings.len(), 1);
    }

    #[test]
    fn reads_tasks_and_their_state() {
        let view = parse("- [ ] open\n- [x] done\n- ordinary item\n");

        assert_eq!(view.tasks.len(), 2);
        assert!(!view.tasks[0].done);
        assert!(view.tasks[1].done);
        assert_eq!(view.tasks[1].text, "done");
    }

    #[test]
    fn reads_links_and_tells_an_image_from_a_link() {
        let found = links_in("see [the guide](guide.md) and ![a chart](chart.png)");

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].text, "the guide");
        assert_eq!(found[0].target, "guide.md");
        assert!(!found[0].image);
        assert!(found[1].image);
    }

    #[test]
    fn a_link_whose_text_holds_brackets_is_read_whole() {
        // A flat pattern stops at the first `]`; a scanner counts depth.
        let found = links_in("[an [inner] bracket](target.md)");

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "an [inner] bracket");
        assert_eq!(found[0].target, "target.md");
    }

    #[test]
    fn reads_front_matter_and_falls_back_to_it_for_the_title() {
        let view = parse("---\ntitle: From the front matter\ntags: [a]\n---\n\nBody text.\n");

        assert_eq!(
            view.front_matter.as_deref(),
            Some("title: From the front matter\ntags: [a]")
        );
        assert_eq!(view.title.as_deref(), Some("From the front matter"));
    }

    #[test]
    fn presents_what_it_found() {
        let view = parse("---\ntitle: T\n---\n\n# T\n\n```sh\nls\n```\n\n- [x] done\n\n[a](b)\n");
        let data = serde_json::to_value(&view).unwrap();

        let lines = MarkdownPresentation.present(&data);

        assert!(lines.contains(&"Title: T".to_owned()));
        assert!(lines.contains(&"Front matter: yes".to_owned()));
        assert!(lines.iter().any(|line| line.contains("sh, 1 line(s)")));
        assert!(lines.iter().any(|line| line.starts_with("Tasks (1 of 1")));
        assert!(lines.iter().any(|line| line.contains("a -> b")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/markdown/guide.md");

        let data = MarkdownCore.view(&path).unwrap();
        let view: MarkdownView = serde_json::from_value(data).unwrap();

        assert!(view.title.is_some());
        assert!(view.front_matter.is_some());
        assert!(view.headings.len() >= 4);
        assert!(view.code_blocks.len() >= 2);
        assert!(view.links.iter().any(|link| link.image));
        assert!(view.tasks.iter().any(|task| task.done));
        assert!(view.tasks.iter().any(|task| !task.done));
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::MarkdownCore),
            plugin_api::PluginPresentation::extensions(&crate::MarkdownPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
