//! JSON5 file type plugin: core and presentation halves.
//!
//! JSON5 is JavaScript Object Notation (JSON) with the liberties a
//! person writing by hand wants: unquoted keys, single quotes, trailing
//! commas, comments. This reads the shape, the top-level keys, the
//! comments - and which of those liberties the document actually takes,
//! each said in terms of what a strict reader would do with it.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["json5"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`Json5Core::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Json5View {
    /// `object`, `array` or `value`.
    pub shape: String,
    /// The top-level keys, in the order the document writes them.
    pub keys: Vec<String>,
    /// The JSON5 extensions the document actually uses, each said in
    /// terms of what a strict reader would do with it.
    pub extensions: Vec<String>,
    /// The comments, which strict JSON has nowhere to put.
    pub comments: Vec<String>,
    /// Whether a strict JSON reader would accept the document as it is.
    pub strict_json: bool,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// What each liberty costs a strict reader, in its own words.
const LIBERTIES: &[(&str, &str)] = &[
    (
        "unquoted keys",
        "a strict reader wants every key in double quotes",
    ),
    (
        "single-quoted strings",
        "a strict reader wants double quotes",
    ),
    (
        "trailing commas",
        "a strict reader stops at the comma before a closing bracket",
    ),
    ("comments", "a strict reader has nowhere to put them"),
    ("hexadecimal numbers", "a strict reader reads only decimal"),
    (
        "leading or trailing decimal point",
        "a strict reader wants a digit on both sides",
    ),
    (
        "multi-line strings",
        "a strict reader will not let a string cross a line",
    ),
    (
        "plus sign or infinity",
        "a strict reader has no `+1`, `Infinity` or `NaN`",
    ),
];

/// Where the walk is: in ordinary text, a string, or a comment.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Where {
    /// Outside any string or comment.
    Text,
    /// Inside a string opened with the held quote.
    InString(char),
    /// Inside a `//` comment.
    LineComment,
    /// Inside a `/* */` comment.
    BlockComment,
}

/// Reads `text` once, gathering everything that does not need structure.
///
/// A single pass, because whether a `#` or a comma is real depends on
/// whether it is inside a string, and that is only knowable in order.
fn scan(text: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut keys: Vec<String> = Vec::new();
    let mut extensions: Vec<String> = Vec::new();
    let mut comments: Vec<String> = Vec::new();

    let letters: Vec<char> = text.chars().collect();
    let mut state = Where::Text;
    let mut depth = 0usize;
    let mut token = String::new();
    let mut comment = String::new();
    let mut string = String::new();
    let mut escaped = false;
    let mut last_meaningful = ' ';

    let mut at = 0usize;
    while at < letters.len() {
        let letter = letters[at];
        let next = letters.get(at + 1).copied().unwrap_or(' ');
        match state {
            Where::LineComment => {
                if letter == '\n' {
                    remember(comment.trim(), &mut comments);
                    comment.clear();
                    state = Where::Text;
                } else {
                    comment.push(letter);
                }
            }
            Where::BlockComment => {
                if letter == '*' && next == '/' {
                    remember(comment.trim(), &mut comments);
                    comment.clear();
                    state = Where::Text;
                    at += 1;
                } else {
                    comment.push(letter);
                }
            }
            Where::InString(quote) => {
                if in_string(letter, quote, &mut escaped, &mut string, &mut extensions) {
                    // A string followed by a colon was a key.
                    if depth == 1 && next_meaningful(&letters, at + 1) == Some(':') {
                        keys.push(string.clone());
                    }
                    string.clear();
                    state = Where::Text;
                    last_meaningful = '"';
                }
            }
            Where::Text => match letter {
                '/' if next == '/' => {
                    note("comments", &mut extensions);
                    state = Where::LineComment;
                    at += 1;
                }
                '/' if next == '*' => {
                    note("comments", &mut extensions);
                    state = Where::BlockComment;
                    at += 1;
                }
                '"' | '\'' => {
                    flush_token(&mut token, depth, &letters, at, &mut keys, &mut extensions);
                    state = Where::InString(letter);
                }
                '{' | '[' => {
                    flush_token(&mut token, depth, &letters, at, &mut keys, &mut extensions);
                    depth += 1;
                    last_meaningful = letter;
                }
                '}' | ']' => {
                    flush_token(&mut token, depth, &letters, at, &mut keys, &mut extensions);
                    if last_meaningful == ',' {
                        note("trailing commas", &mut extensions);
                    }
                    depth = depth.saturating_sub(1);
                    last_meaningful = letter;
                }
                ',' | ':' => {
                    flush_token(&mut token, depth, &letters, at, &mut keys, &mut extensions);
                    last_meaningful = letter;
                }
                _ if letter.is_whitespace() => {
                    flush_token(&mut token, depth, &letters, at, &mut keys, &mut extensions);
                }
                _ => token.push(letter),
            },
        }
        at += 1;
    }
    (keys, extensions, comments)
}

/// Applies one character inside a string, and says whether it closed.
fn in_string(
    letter: char,
    quote: char,
    escaped: &mut bool,
    string: &mut String,
    extensions: &mut Vec<String>,
) -> bool {
    if *escaped {
        if letter == '\n' {
            note("multi-line strings", extensions);
        }
        *escaped = false;
        return false;
    }
    if letter == '\\' {
        *escaped = true;
        return false;
    }
    if letter == quote {
        if quote == '\'' {
            note("single-quoted strings", extensions);
        }
        return true;
    }
    string.push(letter);
    false
}

/// The next character after `from` that is not whitespace.
fn next_meaningful(letters: &[char], from: usize) -> Option<char> {
    letters[from..]
        .iter()
        .find(|letter| !letter.is_whitespace())
        .copied()
}

/// Adds a finished bare token: a key, a number, or a keyword.
fn flush_token(
    token: &mut String,
    depth: usize,
    letters: &[char],
    at: usize,
    keys: &mut Vec<String>,
    extensions: &mut Vec<String>,
) {
    if token.is_empty() {
        return;
    }
    let word = token.clone();
    token.clear();

    // A colon immediately followed by `//` is the middle of a URL, not a
    // key separator. Without this, `acme_ca https://example.com` in a
    // Caddyfile reads as a JSON5 object with a key called `https`, and
    // this plugin claims a file it has no business with.
    let followed_by_colon = next_meaningful(letters, at) == Some(':')
        && !letters[at..]
            .iter()
            .skip_while(|letter| letter.is_whitespace())
            .take(3)
            .collect::<String>()
            .starts_with("://");
    if followed_by_colon {
        note("unquoted keys", extensions);
        if depth == 1 {
            keys.push(word.clone());
        }
        return;
    }
    let lower = word.to_ascii_lowercase();
    if lower.starts_with("0x") || lower.starts_with("-0x") || lower.starts_with("+0x") {
        note("hexadecimal numbers", extensions);
    }
    if word.starts_with('+') || lower.contains("infinity") || word.contains("NaN") {
        note("plus sign or infinity", extensions);
    }
    if word.starts_with('.') || word.ends_with('.') {
        note("leading or trailing decimal point", extensions);
    }
}

/// Adds `extension` to `into` once, with what it costs a strict reader.
fn note(extension: &str, into: &mut Vec<String>) {
    let Some((_, cost)) = LIBERTIES.iter().find(|(name, _)| *name == extension) else {
        return;
    };
    let entry = format!("{extension}: {cost}");
    if !into.contains(&entry) {
        into.push(entry);
    }
}

/// Adds a comment to `into`, if it says anything.
fn remember(comment: &str, into: &mut Vec<String>) {
    let comment = comment.trim_start_matches(['*', '/', ' ']).trim();
    if !comment.is_empty() {
        into.push(comment.to_owned());
    }
}

/// Everything [`Json5View`] holds, read from `text`.
fn parse(text: &str) -> Json5View {
    let (keys, extensions, comments) = scan(text);
    let opener = text.trim_start().chars().next().unwrap_or(' ');
    Json5View {
        shape: match opener {
            '{' => "object",
            '[' => "array",
            _ => "value",
        }
        .to_owned(),
        keys,
        extensions,
        comments,
        strict_json: serde_json::from_str::<Value>(text).is_ok(),
        truncated: false,
    }
}

/// Whether `text` is JSON5.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    // Strict JSON is JSON, and `json` should have it. This is for the
    // documents a strict reader turns away.
    !view.strict_json
        && !view.extensions.is_empty()
        && matches!(view.shape.as_str(), "object" | "array")
        && !view.keys.is_empty()
}

/// The JSON5 plugin's core half.
#[derive(Debug, Default)]
pub struct Json5Core;

impl PluginCore for Json5Core {
    fn name(&self) -> &'static str {
        "json5"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A `.json5` file that happens to be strict JSON is claimed by
        // `json`, which owns that reading. This is the wider one (D13).
        &["json"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        // The keys, the comments and the liberties taken are what a
        // reader came for, and each is on the view already.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The JSON5 plugin's presentation half.
#[derive(Debug, Default)]
pub struct Json5Presentation;

impl PluginPresentation for Json5Presentation {
    fn name(&self) -> &'static str {
        "json5"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "JSN5",
            tint: 0x0025_9dff,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: Json5View = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("JSON5 {}", view.shape));
        if !view.keys.is_empty() {
            lines.push(format!("{} top-level key(s):", view.keys.len()));
            for key in &view.keys {
                lines.push(format!("  {key}"));
            }
        }
        if !view.comments.is_empty() {
            lines.push(format!("{} comment(s):", view.comments.len()));
            for comment in &view.comments {
                lines.push(format!("  {comment}"));
            }
        }
        if view.strict_json {
            lines.push("A strict JSON reader accepts this document as it stands.".to_owned());
        } else {
            lines.push("A strict JSON reader refuses this document. What it uses:".to_owned());
            for extension in &view.extensions {
                lines.push(format!("  {extension}"));
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
    use super::{Json5Core, Json5Presentation, Json5View, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const DOCUMENT: &str = concat!(
        "{\n",
        "  // Where the service listens.\n",
        "  host: 'localhost',\n",
        "  port: 8080,\n",
        "  /* Read as bytes. */\n",
        "  limit: 0x1000,\n",
        "  ratio: .5,\n",
        "  tags: ['a', 'b',],\n",
        "}\n",
    );

    #[test]
    fn sniffs_a_document() {
        assert!(Json5Core.sniff(DOCUMENT.as_bytes()));
    }

    #[test]
    fn does_not_claim_strict_json() {
        assert!(
            !Json5Core.sniff(br#"{"host": "localhost", "port": 8080}"#),
            "strict JSON is JSON, and `json` should have it"
        );
        assert!(!Json5Core.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_json() {
        assert_eq!(Json5Core.specialises(), &["json"]);
    }

    #[test]
    fn reads_the_top_level_keys_in_order() {
        let view = parse(DOCUMENT);

        assert_eq!(view.shape, "object");
        assert_eq!(
            view.keys,
            vec![
                "host".to_owned(),
                "port".to_owned(),
                "limit".to_owned(),
                "ratio".to_owned(),
                "tags".to_owned()
            ],
            "`a` and `b` are values inside an array, not top-level keys"
        );
    }

    #[test]
    fn names_each_extension_with_what_it_costs() {
        let view = parse(DOCUMENT);

        assert!(!view.strict_json);
        for expected in [
            "unquoted keys",
            "single-quoted strings",
            "trailing commas",
            "comments",
            "hexadecimal numbers",
            "leading or trailing decimal point",
        ] {
            assert!(
                view.extensions
                    .iter()
                    .any(|said| said.starts_with(expected)),
                "expected {expected} among {:?}",
                view.extensions
            );
        }
    }

    #[test]
    fn a_slash_inside_a_string_does_not_open_a_comment() {
        let view = parse("{ path: 'https://example.com/a', b: 1, }");

        assert!(
            view.comments.is_empty(),
            "the `//` in a URL is part of the address"
        );
        assert_eq!(view.keys, vec!["path".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn a_url_is_not_a_key() {
        // A Caddyfile opens with a brace and holds bare words, which is
        // near enough to fool a loose reader.
        let caddyfile = concat!(
            "{
",
            "	email floor@example.com
",
            "	acme_ca https://acme-v02.api.letsencrypt.org/directory
",
            "}
",
        );

        assert!(
            parse(caddyfile).keys.is_empty(),
            "`https` is the middle of an address, not a key"
        );
        assert!(!Json5Core.sniff(caddyfile.as_bytes()));
    }

    #[test]
    fn a_comma_inside_a_string_is_not_a_trailing_one() {
        let view = parse("{ \"a\": \"one, two\" }");

        assert!(
            !view
                .extensions
                .iter()
                .any(|said| said.starts_with("trailing")),
            "the comma is inside the string"
        );
    }

    #[test]
    fn reads_the_comments() {
        let view = parse(DOCUMENT);

        assert_eq!(
            view.comments,
            vec![
                "Where the service listens.".to_owned(),
                "Read as bytes.".to_owned()
            ]
        );
    }

    #[test]
    fn says_so_when_a_document_would_pass_a_strict_reader() {
        let view = parse(r#"{"a": 1}"#);

        assert!(view.strict_json);
        let data = serde_json::to_value(&view).unwrap();
        let lines = Json5Presentation.present(&data);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("accepts this document"))
        );
    }

    #[test]
    fn presents_the_extensions_with_their_reasons() {
        let data = serde_json::to_value(parse(DOCUMENT)).unwrap();

        let lines = Json5Presentation.present(&data);

        assert_eq!(lines[0], "JSON5 object");
        assert!(
            lines
                .iter()
                .any(|line| line.contains("refuses this document"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("nowhere to put them"))
        );
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/json5/config.json5");

        let data = Json5Core.view(&path).unwrap();
        let view: Json5View = serde_json::from_value(data).unwrap();

        assert_eq!(view.shape, "object");
        assert!(view.keys.len() >= 6);
        assert!(view.extensions.len() >= 6);
        assert!(view.comments.len() >= 3);
        assert!(!view.strict_json);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::Json5Core),
            plugin_api::PluginPresentation::extensions(&crate::Json5Presentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
