//! Org-mode file type plugin: core and presentation halves.
//!
//! Read line by line. `#+TITLE:` keywords, `#+BEGIN_SRC` blocks and
//! `:PROPERTIES:` drawers are markers no sibling claims; a bare `* ` line
//! is not enough on its own.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["org"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One headline in the outline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Headline {
    /// How many stars it carries.
    pub level: u8,
    /// The headline text, with its TODO state and tags removed.
    pub text: String,
    /// The TODO state, when it opens with one.
    pub todo: Option<String>,
    /// The `:tag:` names at the end of the line.
    pub tags: Vec<String>,
}

/// View data produced by [`OrgmodeCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgmodeView {
    /// The `#+TITLE:` keyword.
    pub title: Option<String>,
    /// Every `#+KEYWORD:` name set in the file.
    pub keywords: Vec<String>,
    /// The outline, in document order.
    pub headlines: Vec<Headline>,
    /// The languages named on `#+BEGIN_SRC` blocks.
    pub source_blocks: Vec<String>,
    /// How many tables the file holds.
    pub tables: usize,
    /// The keys defined in `:PROPERTIES:` drawers.
    pub properties: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The TODO states Org recognises without configuration.
const TODO_STATES: &[&str] = &["TODO", "DONE", "NEXT", "WAITING", "CANCELLED"];

/// Everything [`OrgmodeView`] holds, read from `text`.
fn parse(text: &str) -> OrgmodeView {
    let mut view = OrgmodeView {
        title: None,
        keywords: Vec::new(),
        headlines: Vec::new(),
        source_blocks: Vec::new(),
        tables: 0,
        properties: Vec::new(),
        content: String::new(),
        truncated: false,
    };
    let mut in_table = false;
    let mut in_properties = false;

    for line in text.lines() {
        let trimmed = line.trim_end();
        let bare = trimmed.trim_start();

        if bare.eq_ignore_ascii_case(":PROPERTIES:") {
            in_properties = true;
            continue;
        }
        if bare.eq_ignore_ascii_case(":END:") {
            in_properties = false;
            continue;
        }
        if in_properties {
            if let Some(rest) = bare.strip_prefix(':')
                && let Some((key, _)) = rest.split_once(':')
            {
                view.properties.push(key.to_owned());
            }
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("#+") {
            if let Some((keyword, value)) = rest.split_once(':') {
                let keyword = keyword.trim().to_uppercase();
                if keyword == "TITLE" {
                    view.title = Some(value.trim().to_owned());
                }
                if keyword.starts_with("BEGIN_SRC") {
                    view.source_blocks
                        .push(value.split_whitespace().next().unwrap_or("").to_owned());
                } else if !keyword.starts_with("END_") && !view.keywords.contains(&keyword) {
                    view.keywords.push(keyword);
                }
            } else {
                let keyword = rest.trim().to_uppercase();
                if let Some(language) = keyword.strip_prefix("BEGIN_SRC") {
                    view.source_blocks.push(language.trim().to_lowercase());
                }
            }
            continue;
        }

        if trimmed.starts_with('|') {
            if !in_table {
                view.tables += 1;
                in_table = true;
            }
            continue;
        }
        in_table = false;

        let stars = trimmed.len() - trimmed.trim_start_matches('*').len();
        if stars > 0 && trimmed[stars..].starts_with(' ') {
            let rest = trimmed[stars..].trim();
            let (todo, rest) = TODO_STATES
                .iter()
                .find_map(|state| {
                    rest.strip_prefix(state)
                        .filter(|remainder| remainder.starts_with(' '))
                        .map(|remainder| (Some((*state).to_owned()), remainder.trim()))
                })
                .unwrap_or((None, rest));
            let (text_part, tags) = match rest.rsplit_once(" :") {
                Some((before, tail)) if tail.ends_with(':') => (
                    before.trim(),
                    tail.trim_end_matches(':')
                        .split(':')
                        .filter(|tag| !tag.is_empty())
                        .map(str::to_owned)
                        .collect(),
                ),
                _ => (rest, Vec::new()),
            };
            if let Ok(level) = u8::try_from(stars) {
                view.headlines.push(Headline {
                    level,
                    text: text_part.to_owned(),
                    todo,
                    tags,
                });
            }
        }
    }
    view
}

/// Whether `text` looks like an Org file.
fn looks_like_it(text: &str) -> bool {
    let mut headlines = 0usize;
    for line in text.lines() {
        let upper = line.trim().to_uppercase();
        if upper.starts_with("#+TITLE:")
            || upper.starts_with("#+BEGIN_SRC")
            || upper.starts_with("#+STARTUP:")
            || upper == ":PROPERTIES:"
        {
            return true;
        }
        let stars = line.len() - line.trim_start_matches('*').len();
        if stars > 0 && line[stars..].starts_with(' ') {
            headlines += 1;
        }
    }
    headlines >= 2
}

/// The Org-mode plugin's core half.
#[derive(Debug, Default)]
pub struct OrgmodeCore;

impl PluginCore for OrgmodeCore {
    fn name(&self) -> &'static str {
        "orgmode"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
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

/// The Org-mode plugin's presentation half.
#[derive(Debug, Default)]
pub struct OrgmodePresentation;

impl PluginPresentation for OrgmodePresentation {
    fn name(&self) -> &'static str {
        "orgmode"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "ORG",
            tint: 0x0077_aa99,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: OrgmodeView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(title) = &view.title {
            lines.push(format!("Title: {title}"));
        }
        if !view.keywords.is_empty() {
            lines.push(format!("Keywords: {}", view.keywords.join(", ")));
        }
        if !view.headlines.is_empty() {
            let done = view
                .headlines
                .iter()
                .filter(|headline| headline.todo.as_deref() == Some("DONE"))
                .count();
            lines.push(format!("Outline ({}, {done} done):", view.headlines.len()));
            for headline in &view.headlines {
                let indent = "  ".repeat(usize::from(headline.level));
                let state = headline
                    .todo
                    .as_ref()
                    .map_or_else(String::new, |todo| format!("{todo} "));
                let tags = if headline.tags.is_empty() {
                    String::new()
                } else {
                    format!("  :{}:", headline.tags.join(":"))
                };
                lines.push(format!("{indent}{state}{}{tags}", headline.text));
            }
        }
        if !view.source_blocks.is_empty() {
            lines.push(format!("Source blocks: {}", view.source_blocks.join(", ")));
        }
        if view.tables > 0 {
            lines.push(format!("Tables: {}", view.tables));
        }
        if !view.properties.is_empty() {
            lines.push(format!("Properties: {}", view.properties.join(", ")));
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{OrgmodeCore, OrgmodePresentation, OrgmodeView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_the_keywords_only_org_has() {
        assert!(OrgmodeCore.sniff(b"#+TITLE: Notes\n"));
        assert!(OrgmodeCore.sniff(b"#+BEGIN_SRC sh\nls\n#+END_SRC\n"));
        assert!(OrgmodeCore.sniff(b":PROPERTIES:\n:ID: 1\n:END:\n"));
        assert!(OrgmodeCore.sniff(b"* One\n* Two\n"));
    }

    #[test]
    fn does_not_claim_a_line_of_stars() {
        assert!(!OrgmodeCore.sniff(b"*bold* text\n"));
        assert!(!OrgmodeCore.sniff(b""));
    }

    #[test]
    fn reads_headlines_with_their_state_and_tags() {
        let view = parse("* TODO Write it :work:urgent:\n** DONE Test it :work:\n");

        assert_eq!(view.headlines.len(), 2);
        assert_eq!(view.headlines[0].todo.as_deref(), Some("TODO"));
        assert_eq!(view.headlines[0].text, "Write it");
        assert_eq!(
            view.headlines[0].tags,
            vec!["work".to_owned(), "urgent".to_owned()]
        );
        assert_eq!(view.headlines[1].level, 2);
        assert_eq!(view.headlines[1].todo.as_deref(), Some("DONE"));
    }

    #[test]
    fn reads_the_title_source_blocks_tables_and_properties() {
        let view = parse(
            "#+TITLE: Notes\n#+AUTHOR: Ada\n\n* One\n:PROPERTIES:\n:ID: abc\n:END:\n\n\
             #+BEGIN_SRC rust\nfn f() {}\n#+END_SRC\n\n| a | b |\n| 1 | 2 |\n",
        );

        assert_eq!(view.title.as_deref(), Some("Notes"));
        assert!(view.keywords.contains(&"AUTHOR".to_owned()));
        assert_eq!(view.source_blocks, vec!["rust".to_owned()]);
        assert_eq!(view.tables, 1);
        assert_eq!(view.properties, vec!["ID".to_owned()]);
    }

    #[test]
    fn presents_the_outline() {
        let data = serde_json::to_value(parse("#+TITLE: N\n* TODO a\n* DONE b\n")).unwrap();

        let lines = OrgmodePresentation.present(&data);

        assert_eq!(lines[0], "Title: N");
        assert!(lines.iter().any(|line| line.contains("1 done")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/orgmode/plan.org");

        let data = OrgmodeCore.view(&path).unwrap();
        let view: OrgmodeView = serde_json::from_value(data).unwrap();

        assert!(view.title.is_some());
        assert!(view.keywords.len() >= 2);
        assert!(view.headlines.len() >= 4);
        assert!(view.headlines.iter().any(|h| h.todo.is_some()));
        assert!(view.headlines.iter().any(|h| !h.tags.is_empty()));
        assert!(!view.source_blocks.is_empty());
        assert!(view.tables >= 1);
        assert!(!view.properties.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::OrgmodeCore),
            plugin_api::PluginPresentation::extensions(&crate::OrgmodePresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
