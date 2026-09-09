//! `BibTeX` file type plugin: core and presentation halves.
//!
//! An `@type{key,` entry header is a marker no sibling claims, so the
//! sniff is a single unambiguous one.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["bib", "bibtex"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One bibliography entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The entry type, lowercased: `article`, `book`, `inproceedings`.
    pub kind: String,
    /// The citation key.
    pub key: String,
    /// Its title field, when it has one.
    pub title: Option<String>,
    /// Its author field, when it has one.
    pub author: Option<String>,
    /// Its year field, when it has one.
    pub year: Option<String>,
    /// Every field name it sets, in order.
    pub fields: Vec<String>,
}

/// View data produced by [`BibtexCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BibtexView {
    /// Every entry, in file order.
    pub entries: Vec<Entry>,
    /// The names defined by `@string{name = "..."}`.
    pub strings: Vec<String>,
    /// The keys that appear more than once, which most readers take
    /// last-wins and which are almost always a mistake.
    pub duplicate_keys: Vec<String>,
    /// The entries referenced by a `crossref` field.
    pub crossrefs: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Splits the body of an entry into its `name = value` fields.
///
/// Written as a scan rather than a split on commas: a title routinely
/// holds one, and `{Knuth, Donald E.}` is a single author value.
fn fields_of(body: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut name = String::new();
    let mut value = String::new();
    let mut in_value = false;
    let mut depth = 0usize;
    let mut quoted = false;

    for c in body.chars() {
        if in_value {
            match c {
                '{' => {
                    depth += 1;
                    if depth == 1 {
                        continue;
                    }
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        continue;
                    }
                }
                '"' if depth == 0 => {
                    quoted = !quoted;
                    continue;
                }
                ',' if depth == 0 && !quoted => {
                    found.push((name.trim().to_lowercase(), value.trim().to_owned()));
                    name.clear();
                    value.clear();
                    in_value = false;
                    continue;
                }
                _ => {}
            }
            value.push(c);
        } else if c == '=' {
            in_value = true;
        } else if c == ',' {
            name.clear();
        } else {
            name.push(c);
        }
    }
    if in_value && !name.trim().is_empty() {
        found.push((name.trim().to_lowercase(), value.trim().to_owned()));
    }
    found
}

/// `text` with its `%` line comments removed.
///
/// Needed before the scan, not after: this file's own header comment
/// mentions `@string`, and a scanner that does not know a comment when it
/// sees one read that as the start of an entry.
fn without_comments(text: &str) -> String {
    text.lines()
        .map(|line| match line.find('%') {
            Some(at) if !line[..at].ends_with('\\') => &line[..at],
            _ => line,
        })
        .collect::<Vec<_>>()
        .join(
            "
",
        )
}

/// Everything [`BibtexView`] holds, read from `text`.
fn parse(text: &str) -> BibtexView {
    let text = &without_comments(text);
    let mut view = BibtexView {
        entries: Vec::new(),
        strings: Vec::new(),
        duplicate_keys: Vec::new(),
        crossrefs: Vec::new(),
        content: String::new(),
        truncated: false,
    };

    let chars: Vec<char> = text.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] != '@' {
            index += 1;
            continue;
        }
        // `@type{ ... }`, matched by brace depth.
        let kind_start = index + 1;
        let Some(open) = chars.iter().skip(kind_start).position(|&c| c == '{') else {
            break;
        };
        let open = kind_start + open;
        let kind: String = chars[kind_start..open]
            .iter()
            .collect::<String>()
            .trim()
            .to_lowercase();
        if kind.is_empty() || !kind.chars().all(|c| c.is_ascii_alphabetic()) {
            // An `@` that is not an entry header - an e-mail address, or
            // prose. Step over it rather than inventing an entry.
            index = kind_start;
            continue;
        }

        let mut depth = 0usize;
        let mut close = None;
        for (offset, &c) in chars.iter().enumerate().skip(open) {
            match c {
                '{' => depth += 1,
                '}' => {
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
        let body: String = chars[open + 1..close].iter().collect();
        index = close + 1;

        if kind == "string" {
            if let Some(name) = body.split('=').next() {
                view.strings.push(name.trim().to_owned());
            }
            continue;
        }
        if kind == "comment" || kind == "preamble" {
            continue;
        }

        let key = body.split(',').next().unwrap_or_default().trim().to_owned();
        let fields = fields_of(&body);
        let get = |wanted: &str| {
            fields
                .iter()
                .find(|(name, _)| name == wanted)
                .map(|(_, value)| value.clone())
        };
        if let Some(crossref) = get("crossref") {
            view.crossrefs.push(crossref);
        }
        if view.entries.iter().any(|entry| entry.key == key) && !view.duplicate_keys.contains(&key)
        {
            view.duplicate_keys.push(key.clone());
        }
        view.entries.push(Entry {
            kind,
            key,
            title: get("title"),
            author: get("author"),
            year: get("year"),
            fields: fields.into_iter().map(|(name, _)| name).collect(),
        });
    }
    view
}

/// Whether `text` looks like a `BibTeX` database.
fn looks_like_it(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with('@')
            && trimmed.contains('{')
            && trimmed[1..]
                .split('{')
                .next()
                .is_some_and(|kind| !kind.is_empty() && kind.chars().all(char::is_alphabetic))
    })
}

/// The `BibTeX` plugin's core half.
#[derive(Debug, Default)]
pub struct BibtexCore;

impl PluginCore for BibtexCore {
    fn name(&self) -> &'static str {
        "bibtex"
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

/// The `BibTeX` plugin's presentation half.
#[derive(Debug, Default)]
pub struct BibtexPresentation;

impl PluginPresentation for BibtexPresentation {
    fn name(&self) -> &'static str {
        "bibtex"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "BIB",
            tint: 0x0033_6699,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: BibtexView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("{} entr(y/ies)", view.entries.len()));
        for entry in &view.entries {
            lines.push(format!("  @{} {{{}}}", entry.kind, entry.key));
            if let Some(title) = &entry.title {
                lines.push(format!("    {title}"));
            }
            let by = entry.author.as_deref().unwrap_or("author unstated");
            let year = entry.year.as_deref().unwrap_or("year unstated");
            lines.push(format!("    {by}, {year}"));
            lines.push(format!("    fields: {}", entry.fields.join(", ")));
        }
        if !view.strings.is_empty() {
            lines.push(format!("String definitions: {}", view.strings.join(", ")));
        }
        if !view.crossrefs.is_empty() {
            lines.push(format!("Cross-references: {}", view.crossrefs.join(", ")));
        }
        if !view.duplicate_keys.is_empty() {
            lines.push(format!(
                "Repeated keys, which readers resolve differently: {}",
                view.duplicate_keys.join(", ")
            ));
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{BibtexCore, BibtexPresentation, BibtexView, fields_of, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_an_entry_header() {
        assert!(BibtexCore.sniff(b"@article{knuth1984,\n"));
        assert!(BibtexCore.sniff(b"@book{lamport1986,\n"));
        assert!(!BibtexCore.sniff(b"email@example.com\n"));
        assert!(!BibtexCore.sniff(b""));
    }

    #[test]
    fn a_comma_inside_braces_does_not_end_a_field() {
        let fields = fields_of("k, author = {Knuth, Donald E.}, year = {1984}");

        assert_eq!(
            fields[0],
            ("author".to_owned(), "Knuth, Donald E.".to_owned())
        );
        assert_eq!(fields[1].1, "1984");
    }

    #[test]
    fn reads_an_entry_and_its_named_fields() {
        let view = parse(
            "@article{knuth1984,\n  author = {Knuth, Donald E.},\n\
             \x20 title = {Literate Programming},\n  year = {1984},\n}\n",
        );

        assert_eq!(view.entries.len(), 1);
        assert_eq!(view.entries[0].kind, "article");
        assert_eq!(view.entries[0].key, "knuth1984");
        assert_eq!(
            view.entries[0].title.as_deref(),
            Some("Literate Programming")
        );
        assert_eq!(view.entries[0].year.as_deref(), Some("1984"));
        assert!(view.entries[0].fields.contains(&"author".to_owned()));
    }

    #[test]
    fn an_at_sign_in_a_comment_does_not_open_an_entry() {
        // This file's own header comment names `@string`, and a scanner
        // that does not know a comment when it sees one read that as an
        // entry whose key was the rest of the paragraph.
        let view = parse(
            "% mentions @string and @article in prose
@book{a, title = {T}}
",
        );

        assert_eq!(view.entries.len(), 1);
        assert_eq!(view.entries[0].key, "a");
    }

    #[test]
    fn a_string_definition_is_not_an_entry() {
        let view = parse("@string{acm = {ACM}}\n@book{a, title = {T}}\n");

        assert_eq!(view.strings, vec!["acm".to_owned()]);
        assert_eq!(view.entries.len(), 1);
    }

    #[test]
    fn reports_a_repeated_key_and_a_crossref() {
        let view = parse(
            "@inproceedings{a, crossref = {proc}}\n@book{a, title = {T}}\n\
             @proceedings{proc, title = {P}}\n",
        );

        assert_eq!(view.duplicate_keys, vec!["a".to_owned()]);
        assert_eq!(view.crossrefs, vec!["proc".to_owned()]);
    }

    #[test]
    fn presents_each_entry_with_its_author_and_year() {
        let data = serde_json::to_value(parse(
            "@book{a, author = {Ada}, title = {T}, year = {1843}}",
        ))
        .unwrap();

        let lines = BibtexPresentation.present(&data);

        assert_eq!(lines[0], "1 entr(y/ies)");
        assert!(lines.iter().any(|line| line.contains("Ada, 1843")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/bibtex/references.bib");

        let data = BibtexCore.view(&path).unwrap();
        let view: BibtexView = serde_json::from_value(data).unwrap();

        assert!(view.entries.len() >= 4);
        assert!(view.entries.iter().any(|entry| entry.kind == "article"));
        assert!(view.entries.iter().any(|entry| entry.kind == "book"));
        assert!(view.entries.iter().all(|entry| entry.title.is_some()));
        assert!(!view.strings.is_empty());
        assert!(!view.crossrefs.is_empty());
        assert!(!view.duplicate_keys.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::BibtexCore),
            plugin_api::PluginPresentation::extensions(&crate::BibtexPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
