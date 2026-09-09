//! roff manual page file type plugin: core and presentation halves.
//!
//! A `.TH` title macro is the marker: it is the first thing in every
//! manual page and nothing else uses it. Numeric extensions - `.1`, `.8` -
//! are claimed too, since no sibling wants them.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["1", "2", "3", "5", "7", "8", "man", "roff", "troff"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One section of the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    /// The heading text, as `.SH` gives it.
    pub name: String,
    /// How many lines it holds before the next heading.
    pub lines: usize,
}

/// View data produced by [`RoffCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoffView {
    /// The page name from `.TH`.
    pub name: Option<String>,
    /// Its manual section number.
    pub section: Option<String>,
    /// The date `.TH` carries.
    pub date: Option<String>,
    /// The source and manual fields of `.TH`, when it names them.
    pub source: Option<String>,
    /// The `.SH` headings, in order.
    pub sections: Vec<Section>,
    /// The synopsis line, joined from the SYNOPSIS section.
    pub synopsis: Option<String>,
    /// The pages cross-referenced in SEE ALSO, as `name(section)`.
    pub see_also: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Splits a macro line into its arguments, honouring double quotes.
fn arguments(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    found.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        found.push(current);
    }
    found
}

/// Strips roff's inline font escapes, so a heading reads as words.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // `\fB`, `\fI`, `\fR`, `\fP`: a font change and its letter.
            Some('f') => {
                chars.next();
            }
            Some('-') => out.push('-'),
            // `\&` is a zero-width break, and a trailing backslash escapes
            // nothing at all. Both contribute no character.
            Some('&') | None => {}
            Some(other) => out.push(other),
        }
    }
    out
}

/// Everything [`RoffView`] holds, read from `text`.
fn parse(text: &str) -> RoffView {
    let mut view = RoffView {
        name: None,
        section: None,
        date: None,
        source: None,
        sections: Vec::new(),
        synopsis: None,
        see_also: Vec::new(),
        content: String::new(),
        truncated: false,
    };
    let mut current: Option<String> = None;
    let mut body: Vec<String> = Vec::new();

    let close = |current: &mut Option<String>, body: &mut Vec<String>, view: &mut RoffView| {
        let Some(name) = current.take() else {
            body.clear();
            return;
        };
        if name.eq_ignore_ascii_case("SYNOPSIS") && view.synopsis.is_none() {
            let joined = body.join(" ").trim().to_owned();
            if !joined.is_empty() {
                view.synopsis = Some(joined);
            }
        }
        if name.eq_ignore_ascii_case("SEE ALSO") {
            for line in body.iter() {
                for token in line.split(',') {
                    let token = token.trim().trim_end_matches('.');
                    if token.ends_with(')') && token.contains('(') {
                        view.see_also.push(token.to_owned());
                    }
                }
            }
        }
        view.sections.push(Section {
            name,
            lines: body.len(),
        });
        body.clear();
    };

    for raw in text.lines() {
        let line = raw.trim_end();

        if let Some(rest) = line.strip_prefix(".TH") {
            let fields = arguments(rest);
            view.name = fields.first().map(|field| plain(field));
            view.section = fields.get(1).map(|field| plain(field));
            view.date = fields.get(2).map(|field| plain(field));
            view.source = fields.get(3).map(|field| plain(field));
            continue;
        }
        if let Some(rest) = line.strip_prefix(".SH") {
            close(&mut current, &mut body, &mut view);
            let heading = arguments(rest).join(" ");
            current = Some(plain(&heading).trim().to_owned());
            continue;
        }
        if line.starts_with(".\\\"") || line.starts_with(".ig") {
            // A comment macro.
            continue;
        }
        if current.is_some() {
            let stripped = if let Some(rest) = line.strip_prefix('.') {
                // A formatting macro: keep its arguments, drop its name.
                let mut fields = arguments(rest);
                if fields.is_empty() {
                    continue;
                }
                fields.remove(0);
                fields.join(" ")
            } else {
                line.to_owned()
            };
            let stripped = plain(&stripped).trim().to_owned();
            if !stripped.is_empty() {
                body.push(stripped);
            }
        }
    }
    close(&mut current, &mut body, &mut view);
    view
}

/// Whether `text` looks like a manual page.
fn looks_like_it(text: &str) -> bool {
    let mut headings = 0usize;
    for line in text.lines() {
        if line.starts_with(".TH ") || line.starts_with(".Dd ") {
            return true;
        }
        if line.starts_with(".SH ") {
            headings += 1;
        }
    }
    headings >= 2
}

/// The roff manual page plugin's core half.
#[derive(Debug, Default)]
pub struct RoffCore;

impl PluginCore for RoffCore {
    fn name(&self) -> &'static str {
        "roff"
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

/// The roff manual page plugin's presentation half.
#[derive(Debug, Default)]
pub struct RoffPresentation;

impl PluginPresentation for RoffPresentation {
    fn name(&self) -> &'static str {
        "roff"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "MAN",
            tint: 0x0059_5eab,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: RoffView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(name) = &view.name {
            let section = view.section.as_deref().unwrap_or("?");
            lines.push(format!("{name}({section})"));
        }
        if let Some(date) = &view.date {
            lines.push(format!("Dated: {date}"));
        }
        if let Some(source) = &view.source {
            lines.push(format!("Source: {source}"));
        }
        if let Some(synopsis) = &view.synopsis {
            lines.push(format!("Synopsis: {synopsis}"));
        }
        if !view.sections.is_empty() {
            lines.push(format!("Sections ({}):", view.sections.len()));
            for section in &view.sections {
                lines.push(format!("  {}  ({} line(s))", section.name, section.lines));
            }
        }
        if !view.see_also.is_empty() {
            lines.push(format!("See also: {}", view.see_also.join(", ")));
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{RoffCore, RoffPresentation, RoffView, arguments, parse, plain};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_a_title_macro_or_several_headings() {
        assert!(RoffCore.sniff(b".TH EXPLORE 1 \"September 2026\"\n"));
        assert!(RoffCore.sniff(b".SH NAME\n.SH SYNOPSIS\n"));
    }

    #[test]
    fn does_not_claim_a_file_of_leading_dots() {
        assert!(!RoffCore.sniff(b".gitignore\n.env\n"));
        assert!(!RoffCore.sniff(b"prose about a .TH somewhere\n"));
        assert!(!RoffCore.sniff(b""));
    }

    #[test]
    fn a_quoted_argument_stays_one_argument() {
        assert_eq!(
            arguments(" EXPLORE 1 \"September 2026\" \"Repos Explorer\""),
            vec!["EXPLORE", "1", "September 2026", "Repos Explorer"]
        );
    }

    #[test]
    fn font_escapes_are_stripped_so_a_heading_reads_as_words() {
        assert_eq!(plain("\\fBexplore\\fR \\-\\-help"), "explore --help");
    }

    #[test]
    fn reads_the_title_macro() {
        let view = parse(".TH EXPLORE 1 \"September 2026\" \"Repos Explorer\"\n");

        assert_eq!(view.name.as_deref(), Some("EXPLORE"));
        assert_eq!(view.section.as_deref(), Some("1"));
        assert_eq!(view.date.as_deref(), Some("September 2026"));
        assert_eq!(view.source.as_deref(), Some("Repos Explorer"));
    }

    #[test]
    fn reads_the_sections_the_synopsis_and_the_cross_references() {
        let view = parse(
            ".TH EXPLORE 1\n.SH NAME\nexplore \\- browse repositories\n\
             .SH SYNOPSIS\n.B explore\n[\\fIoptions\\fR]\n\
             .SH DESCRIPTION\nIt browses.\n.SH SEE ALSO\n\
             .BR git (1),\n.BR ls (1)\n",
        );

        assert_eq!(
            view.sections
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            vec!["NAME", "SYNOPSIS", "DESCRIPTION", "SEE ALSO"]
        );
        assert!(
            view.synopsis
                .as_deref()
                .is_some_and(|s| s.contains("explore"))
        );
        assert_eq!(view.see_also.len(), 2);
    }

    #[test]
    fn a_comment_macro_contributes_nothing() {
        let view = parse(".TH A 1\n.SH NAME\n.\\\" not part of the page\nreal line\n");

        assert_eq!(view.sections[0].lines, 1);
    }

    #[test]
    fn presents_the_page_name_first() {
        let data = serde_json::to_value(parse(".TH EXPLORE 1\n.SH NAME\na\n")).unwrap();

        let lines = RoffPresentation.present(&data);

        assert_eq!(lines[0], "EXPLORE(1)");
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/roff/explore.1");

        let data = RoffCore.view(&path).unwrap();
        let view: RoffView = serde_json::from_value(data).unwrap();

        assert!(view.name.is_some());
        assert!(view.section.is_some());
        assert!(view.date.is_some());
        assert!(view.source.is_some());
        assert!(view.sections.len() >= 5);
        assert!(view.synopsis.is_some());
        assert!(view.see_also.len() >= 2);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::RoffCore),
            plugin_api::PluginPresentation::extensions(&crate::RoffPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
