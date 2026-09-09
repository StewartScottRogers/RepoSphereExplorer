//! `AsciiDoc` file type plugin: core and presentation halves.
//!
//! Read line by line. An `AsciiDoc` document header, `include::` directives
//! and `[source]` block attributes are markers no sibling claims; the
//! extension settles what is left.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["adoc", "asciidoc", "asc"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One section heading, and how deep it sits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    /// 1 for `=`, 5 for `=====`.
    pub level: u8,
    /// The heading text.
    pub text: String,
}

/// View data produced by [`AsciidocCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsciidocView {
    /// The document title from its `= Title` header.
    pub title: Option<String>,
    /// The `:name: value` attributes set in the header, in order.
    pub attributes: Vec<String>,
    /// Every section heading below the title.
    pub sections: Vec<Section>,
    /// The targets of `include::` directives.
    pub includes: Vec<String>,
    /// The languages named on `[source,lang]` blocks.
    pub source_blocks: Vec<String>,
    /// The admonitions used, such as `NOTE` or `WARNING`.
    pub admonitions: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The five admonition labels `AsciiDoc` defines.
const ADMONITIONS: &[&str] = &["NOTE", "TIP", "IMPORTANT", "CAUTION", "WARNING"];

/// The level and text of an `== Heading`, if `line` is one.
fn heading(line: &str) -> Option<(u8, String)> {
    let equals = line.len() - line.trim_start_matches('=').len();
    if equals == 0 || equals > 6 {
        return None;
    }
    let rest = &line[equals..];
    if !rest.starts_with(' ') {
        return None;
    }
    u8::try_from(equals)
        .ok()
        .map(|level| (level, rest.trim().to_owned()))
}

/// Everything [`AsciidocView`] holds, read from `text`.
fn parse(text: &str) -> AsciidocView {
    let mut view = AsciidocView {
        title: None,
        attributes: Vec::new(),
        sections: Vec::new(),
        includes: Vec::new(),
        source_blocks: Vec::new(),
        admonitions: Vec::new(),
        content: String::new(),
        truncated: false,
    };

    for line in text.lines() {
        let trimmed = line.trim_end();

        if let Some((level, heading)) = heading(trimmed) {
            if level == 1 && view.title.is_none() {
                view.title = Some(heading);
            } else {
                view.sections.push(Section {
                    level,
                    text: heading,
                });
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix(':')
            && let Some((name, _)) = rest.split_once(':')
            && !name.is_empty()
            && !name.contains(' ')
        {
            view.attributes.push(name.to_owned());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("include::") {
            view.includes
                .push(rest.split('[').next().unwrap_or(rest).trim().to_owned());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("[source")
            && let Some(inner) = rest.strip_suffix(']')
        {
            let language = inner
                .trim_start_matches(',')
                .split(',')
                .next()
                .unwrap_or("");
            view.source_blocks.push(language.trim().to_owned());
            continue;
        }
        for label in ADMONITIONS {
            if trimmed.starts_with(&format!("{label}: ")) || trimmed == format!("[{label}]") {
                view.admonitions.push((*label).to_owned());
            }
        }
    }
    view
}

/// Whether `text` looks like `AsciiDoc`.
fn looks_like_it(text: &str) -> bool {
    let mut headings = 0usize;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.starts_with("include::")
            || trimmed.starts_with("[source")
            || trimmed == "----"
            || ADMONITIONS
                .iter()
                .any(|label| trimmed.starts_with(&format!("{label}: ")))
        {
            return true;
        }
        if heading(trimmed).is_some() {
            headings += 1;
        }
    }
    headings >= 2
}

/// The `AsciiDoc` plugin's core half.
#[derive(Debug, Default)]
pub struct AsciidocCore;

impl PluginCore for AsciidocCore {
    fn name(&self) -> &'static str {
        "asciidoc"
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

/// The `AsciiDoc` plugin's presentation half.
#[derive(Debug, Default)]
pub struct AsciidocPresentation;

impl PluginPresentation for AsciidocPresentation {
    fn name(&self) -> &'static str {
        "asciidoc"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "ADOC",
            tint: 0x00e4_0046,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: AsciidocView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(title) = &view.title {
            lines.push(format!("Title: {title}"));
        }
        if !view.attributes.is_empty() {
            lines.push(format!("Attributes: {}", view.attributes.join(", ")));
        }
        if !view.sections.is_empty() {
            lines.push(format!("Outline ({}):", view.sections.len()));
            for section in &view.sections {
                lines.push(format!(
                    "{}{}",
                    "  ".repeat(usize::from(section.level)),
                    section.text
                ));
            }
        }
        if !view.includes.is_empty() {
            lines.push(format!("Includes: {}", view.includes.join(", ")));
        }
        if !view.source_blocks.is_empty() {
            lines.push(format!("Source blocks: {}", view.source_blocks.join(", ")));
        }
        if !view.admonitions.is_empty() {
            lines.push(format!("Admonitions: {}", view.admonitions.join(", ")));
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{AsciidocCore, AsciidocPresentation, AsciidocView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_the_markers_only_asciidoc_has() {
        assert!(AsciidocCore.sniff(b"include::other.adoc[]\n"));
        assert!(AsciidocCore.sniff(b"[source,rust]\n----\nfn main() {}\n----\n"));
        assert!(AsciidocCore.sniff(b"NOTE: mind the gap\n"));
        assert!(AsciidocCore.sniff(b"= Title\n\n== Section\n"));
    }

    #[test]
    fn does_not_claim_a_line_of_equals_signs() {
        assert!(!AsciidocCore.sniff(b"Heading\n=======\n"));
        assert!(!AsciidocCore.sniff(b"just prose\n"));
        assert!(!AsciidocCore.sniff(b""));
    }

    #[test]
    fn the_first_level_one_heading_is_the_title() {
        let view = parse("= The Title\n\n== One\n\n=== Two\n");

        assert_eq!(view.title.as_deref(), Some("The Title"));
        assert_eq!(view.sections.len(), 2);
        assert_eq!(view.sections[1].level, 3);
    }

    #[test]
    fn reads_attributes_includes_sources_and_admonitions() {
        let view = parse(
            "= T\n:author: Ada\n:revnumber: 1.0\n\ninclude::part.adoc[]\n\n\
             [source,rust]\n----\nfn f() {}\n----\n\nWARNING: careful\n",
        );

        assert_eq!(
            view.attributes,
            vec!["author".to_owned(), "revnumber".to_owned()]
        );
        assert_eq!(view.includes, vec!["part.adoc".to_owned()]);
        assert_eq!(view.source_blocks, vec!["rust".to_owned()]);
        assert_eq!(view.admonitions, vec!["WARNING".to_owned()]);
    }

    #[test]
    fn presents_what_it_found() {
        let data = serde_json::to_value(parse("= T\n:a: 1\n\n== S\n")).unwrap();

        let lines = AsciidocPresentation.present(&data);

        assert_eq!(lines[0], "Title: T");
        assert!(lines.iter().any(|line| line.starts_with("Attributes:")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/asciidoc/handbook.adoc");

        let data = AsciidocCore.view(&path).unwrap();
        let view: AsciidocView = serde_json::from_value(data).unwrap();

        assert!(view.title.is_some());
        assert!(view.attributes.len() >= 2);
        assert!(view.sections.len() >= 3);
        assert!(!view.includes.is_empty());
        assert!(!view.source_blocks.is_empty());
        assert!(view.admonitions.len() >= 2);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::AsciidocCore),
            plugin_api::PluginPresentation::extensions(&crate::AsciidocPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
