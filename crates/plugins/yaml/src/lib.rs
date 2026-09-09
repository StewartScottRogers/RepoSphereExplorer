//! YAML file type plugin: core and presentation halves.
//!
//! Read line by line rather than through a parser, on purpose. The thing a
//! reader wants from a YAML file in a repository is its shape - how many
//! documents, what the top-level keys are, where the anchors are - and a
//! full parse would refuse the file outright on a syntax error, which is
//! exactly when somebody most wants to look at it.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["yaml", "yml"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One document in the stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// The keys at the document's own indentation level, in order.
    pub keys: Vec<String>,
    /// Whether the document's top level is a sequence rather than a
    /// mapping.
    pub sequence: bool,
    /// How many lines it holds.
    pub lines: usize,
}

/// View data produced by [`YamlCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YamlView {
    /// Every document in the stream, in order.
    pub documents: Vec<Document>,
    /// The anchors defined, as `&name`.
    pub anchors: Vec<String>,
    /// The aliases referenced, as `*name`.
    pub aliases: Vec<String>,
    /// The explicit tags used, as `!tag` or `!!tag`.
    pub tags: Vec<String>,
    /// The deepest indentation reached, in levels of two spaces.
    pub depth: usize,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Whether `line` starts a new document.
fn is_document_start(line: &str) -> bool {
    line == "---" || line.starts_with("--- ")
}

/// Whether `line` ends a document.
fn is_document_end(line: &str) -> bool {
    line == "..."
}

/// The key of a `key: value` or `key:` line at the top level, if it is one.
///
/// Requires the colon to be followed by a space or the end of the line,
/// which is what separates a YAML key from a `http://` in prose.
fn top_level_key(line: &str) -> Option<String> {
    if line.starts_with(' ') || line.starts_with('#') || line.starts_with('-') {
        return None;
    }
    let (key, rest) = line.split_once(':')?;
    if !(rest.is_empty() || rest.starts_with(' ')) {
        return None;
    }
    let key = key.trim().trim_matches(['"', '\'']);
    if key.is_empty() || key.contains(' ') && !key.starts_with('"') {
        return None;
    }
    Some(key.to_owned())
}

/// How many two-space levels `line` is indented by.
fn indent_levels(line: &str) -> usize {
    (line.len() - line.trim_start().len()) / 2
}

/// Every `&anchor`, `*alias` and `!tag` token in `line`.
fn markers(line: &str, sigil: char) -> Vec<String> {
    let mut found = Vec::new();
    for (index, character) in line.char_indices() {
        if character != sigil {
            continue;
        }
        // A sigil only counts at the start of a token.
        if index > 0 && !line[..index].ends_with([' ', '[', '{', ',', '-']) {
            continue;
        }
        let rest = &line[index + character.len_utf8()..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '!')
            .collect();
        if !name.is_empty() {
            found.push(format!("{sigil}{name}"));
        }
    }
    found
}

/// Everything [`YamlView`] holds, read from `text`.
fn parse(text: &str) -> YamlView {
    let mut view = YamlView {
        documents: Vec::new(),
        anchors: Vec::new(),
        aliases: Vec::new(),
        tags: Vec::new(),
        depth: 0,
        content: String::new(),
        truncated: false,
    };
    let mut current = Document {
        keys: Vec::new(),
        sequence: false,
        lines: 0,
    };
    let mut started = false;

    for raw in text.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim();

        if is_document_start(trimmed) {
            if started || !current.keys.is_empty() || current.lines > 0 {
                view.documents.push(std::mem::replace(
                    &mut current,
                    Document {
                        keys: Vec::new(),
                        sequence: false,
                        lines: 0,
                    },
                ));
            }
            started = true;
            continue;
        }
        if is_document_end(trimmed) {
            continue;
        }

        current.lines += 1;
        view.depth = view.depth.max(indent_levels(line));

        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            if let Some(key) = top_level_key(line) {
                current.keys.push(key);
            } else if line.starts_with("- ") || line == "-" {
                current.sequence = true;
            }
        }

        for marker in markers(line, '&') {
            if !view.anchors.contains(&marker) {
                view.anchors.push(marker);
            }
        }
        for marker in markers(line, '*') {
            if !view.aliases.contains(&marker) {
                view.aliases.push(marker);
            }
        }
        for marker in markers(line, '!') {
            if !view.tags.contains(&marker) {
                view.tags.push(marker);
            }
        }
    }

    if started || current.lines > 0 {
        view.documents.push(current);
    }
    view
}

/// Whether `prefix` looks like YAML.
///
/// A `---` document marker settles it. Otherwise two or more top-level
/// `key: value` lines, or one alongside a `- ` sequence item: one such line
/// on its own appears in too many other formats to claim.
fn looks_like_yaml(prefix: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(prefix) else {
        return false;
    };
    let mut keys = 0usize;
    let mut items = 0usize;

    for raw in text.lines() {
        let trimmed = raw.trim();
        if is_document_start(trimmed) {
            return true;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if top_level_key(raw.trim_end()).is_some() {
            keys += 1;
        } else if trimmed.starts_with("- ") {
            items += 1;
        }
    }
    keys >= 2 || (keys >= 1 && items >= 1)
}

/// The YAML plugin's core half.
#[derive(Debug, Default)]
pub struct YamlCore;

impl PluginCore for YamlCore {
    fn name(&self) -> &'static str {
        "yaml"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_yaml(prefix)
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

/// The YAML plugin's presentation half.
#[derive(Debug, Default)]
pub struct YamlPresentation;

impl PluginPresentation for YamlPresentation {
    fn name(&self) -> &'static str {
        "yaml"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "YAML",
            tint: 0x00cb_171e,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: YamlView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();

        lines.push(match view.documents.len() {
            1 => "1 document".to_owned(),
            count => format!("{count} documents"),
        });

        for (index, document) in view.documents.iter().enumerate() {
            let shape = if document.sequence {
                "sequence"
            } else {
                "mapping"
            };
            lines.push(format!("  [{index}] {shape}, {} line(s)", document.lines));
            for key in &document.keys {
                lines.push(format!("    {key}"));
            }
        }

        lines.push(format!("Deepest nesting: {} level(s)", view.depth));
        if !view.anchors.is_empty() {
            lines.push(format!("Anchors: {}", view.anchors.join(", ")));
        }
        if !view.aliases.is_empty() {
            lines.push(format!("Aliases: {}", view.aliases.join(", ")));
        }
        if !view.tags.is_empty() {
            lines.push(format!("Tags: {}", view.tags.join(", ")));
        }
        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{YamlCore, YamlPresentation, YamlView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_a_document_marker_or_several_keys() {
        assert!(YamlCore.sniff(b"---\nname: widgets\n"));
        assert!(YamlCore.sniff(b"name: widgets\nversion: 1\n"));
        assert!(YamlCore.sniff(b"jobs:\n  - one\n"));
    }

    #[test]
    fn does_not_claim_one_colon_line_from_whoever_owns_it() {
        // A single `key: value` line appears in a Makefile rule, an HTTP
        // header, a log line and half the prose in this repository.
        assert!(!YamlCore.sniff(b"Note: this is a sentence.\n"));
        assert!(!YamlCore.sniff(b"see https://example.com/a\n"));
        assert!(!YamlCore.sniff(b""));
        assert!(!YamlCore.sniff(&[0xFF, 0xFE, 0x00]));
    }

    #[test]
    fn counts_the_documents_in_a_stream() {
        let view = parse("---\na: 1\n---\nb: 2\n---\nc: 3\n");

        assert_eq!(view.documents.len(), 3);
        assert_eq!(view.documents[0].keys, vec!["a".to_owned()]);
        assert_eq!(view.documents[2].keys, vec!["c".to_owned()]);
    }

    #[test]
    fn a_stream_with_no_marker_is_one_document() {
        let view = parse("name: widgets\nversion: 1\n");

        assert_eq!(view.documents.len(), 1);
        assert_eq!(view.documents[0].keys.len(), 2);
    }

    #[test]
    fn tells_a_sequence_document_from_a_mapping() {
        let view = parse("---\n- one\n- two\n");

        assert!(view.documents[0].sequence);
        assert!(view.documents[0].keys.is_empty());
    }

    #[test]
    fn finds_anchors_aliases_and_tags() {
        let view =
            parse("base: &defaults\n  a: 1\nuse:\n  <<: *defaults\nwhen: !!timestamp 2026-01-01\n");

        assert_eq!(view.anchors, vec!["&defaults".to_owned()]);
        assert_eq!(view.aliases, vec!["*defaults".to_owned()]);
        assert!(view.tags.iter().any(|tag| tag.starts_with("!!")));
    }

    #[test]
    fn measures_the_deepest_nesting() {
        let view = parse("a:\n  b:\n    c:\n      d: 1\n");

        assert_eq!(view.depth, 3);
    }

    #[test]
    fn a_comment_is_not_a_key() {
        let view = parse("# name: not a key\nreal: yes\n");

        assert_eq!(view.documents[0].keys, vec!["real".to_owned()]);
    }

    #[test]
    fn presents_the_shape_of_the_stream() {
        let data = serde_json::to_value(parse("---\na: 1\nb: &x 2\n---\n- one\n")).unwrap();

        let lines = YamlPresentation.present(&data);

        assert_eq!(lines[0], "2 documents");
        assert!(lines.iter().any(|line| line.contains("mapping")));
        assert!(lines.iter().any(|line| line.contains("sequence")));
        assert!(lines.iter().any(|line| line.starts_with("Anchors:")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/yaml/pipeline-settings.yaml");

        let data = YamlCore.view(&path).unwrap();
        let view: YamlView = serde_json::from_value(data).unwrap();

        assert!(view.documents.len() >= 2);
        assert!(!view.anchors.is_empty());
        assert!(!view.aliases.is_empty());
        assert!(!view.tags.is_empty());
        assert!(view.depth >= 3);
        assert!(view.documents.iter().any(|document| document.sequence));
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::YamlCore),
            plugin_api::PluginPresentation::extensions(&crate::YamlPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
