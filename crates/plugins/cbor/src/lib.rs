//! CBOR file type plugin: core and presentation halves.
//!
//! Concise Binary Object Representation (CBOR): a binary format shaped
//! like JavaScript Object Notation (JSON) but with tags, byte strings
//! and items written without a length. This reads the top-level item,
//! the keys of a map, the tags with what each means, the shape a few
//! levels deep, the indefinite-length items, and anything left over
//! after the first value.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["cbor"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`CborCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CborView {
    /// Whether the file opens with the self-describing tag `d9 d9 f7`.
    pub self_describing: bool,
    /// What the top-level item is: a map, an array, a text string and so on.
    pub top_level: String,
    /// How many entries a top-level map or array holds.
    pub entries: Option<usize>,
    /// The keys of a top-level map, when they are text.
    pub keys: Vec<String>,
    /// The tags the document uses, each with what it means.
    pub tags: Vec<String>,
    /// The shape of the whole document, indented.
    pub shape: Vec<String>,
    /// Items written without a length up front, which a reader has to
    /// consume to the break byte before it knows how big they are.
    pub indefinite_items: Vec<String>,
    /// What is left over after the first item, if anything - a file
    /// holding two items where a reader expects one.
    pub trailing_bytes: usize,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The tags worth naming, from the registry every writer agrees on.
const TAGS: &[(u64, &str)] = &[
    (0, "a date and time, as text"),
    (1, "a point in time, as seconds since the epoch"),
    (2, "an unsigned big number"),
    (3, "a negative big number"),
    (4, "a decimal fraction"),
    (5, "a big float"),
    (21, "bytes to be shown as base64url"),
    (22, "bytes to be shown as base64"),
    (23, "bytes to be shown as hexadecimal"),
    (24, "an item encoded inside a byte string"),
    (32, "a uniform resource identifier"),
    (33, "a base64url string"),
    (34, "a base64 string"),
    (36, "a MIME message"),
    (55799, "the self-describing tag"),
];

/// The names of the eight major types, by their number.
const MAJOR: &[&str] = &[
    "unsigned integer",
    "negative integer",
    "byte string",
    "text string",
    "array",
    "map",
    "tagged value",
    "simple value",
];

/// How far into a document the shape is followed.
const MAX_DEPTH: usize = 4;

/// A reader over the bytes, which stops rather than panicking.
struct Reader<'a> {
    /// The bytes being read.
    bytes: &'a [u8],
    /// How far in the reader has got.
    at: usize,
}

/// One item's head: its major type, its argument, and whether the length
/// was left to a break byte.
struct Head {
    /// The major type, 0 to 7.
    major: u8,
    /// The argument, which is a value, a length or a tag number.
    argument: u64,
    /// Whether the item was written without a length.
    indefinite: bool,
    /// How many bytes the argument took. For major type 7 that is the
    /// whole difference between a float and a simple value.
    width: u8,
}

impl Reader<'_> {
    /// The next byte, or nothing when the bytes have run out.
    fn byte(&mut self) -> Option<u8> {
        let byte = *self.bytes.get(self.at)?;
        self.at += 1;
        Some(byte)
    }

    /// The next `count` bytes as a big-endian number.
    fn number(&mut self, count: usize) -> Option<u64> {
        let mut value = 0u64;
        for _ in 0..count {
            value = (value << 8) | u64::from(self.byte()?);
        }
        Some(value)
    }

    /// The head of the next item.
    fn head(&mut self) -> Option<Head> {
        let first = self.byte()?;
        let major = first >> 5;
        let short = first & 0x1f;
        let (argument, indefinite, width) = match short {
            0..=23 => (u64::from(short), false, 0),
            24 => (self.number(1)?, false, 1),
            25 => (self.number(2)?, false, 2),
            26 => (self.number(4)?, false, 4),
            27 => (self.number(8)?, false, 8),
            31 => (0, true, 0),
            _ => return None,
        };
        Some(Head {
            major,
            argument,
            indefinite,
            width,
        })
    }

    /// Skips `count` bytes, or fails if there are not that many.
    fn skip(&mut self, count: u64) -> Option<()> {
        let count = usize::try_from(count).ok()?;
        self.at = self
            .at
            .checked_add(count)
            .filter(|at| *at <= self.bytes.len())?;
        Some(())
    }
}

/// Reads one item, adding what it finds to `view`.
///
/// Returns `None` when the bytes run out or say something the format does
/// not allow, which is how a file that is not this format is told.
fn item(reader: &mut Reader, view: &mut CborView, depth: usize, label: &str) -> Option<()> {
    let head = reader.head()?;
    let name = MAJOR.get(head.major as usize).copied().unwrap_or("unknown");

    if head.indefinite && head.major != 7 {
        view.indefinite_items
            .push(format!("{label}: {name} with no length up front"));
    }
    if depth < MAX_DEPTH {
        view.shape.push(format!(
            "{}{label}{}",
            "  ".repeat(depth),
            described(&head, name)
        ));
    }

    match head.major {
        0 | 1 => Some(()),
        2 | 3 => {
            if head.indefinite {
                until_break(reader, view, depth)
            } else {
                reader.skip(head.argument)
            }
        }
        4 => read_sequence(reader, view, depth, &head, 1),
        5 => read_sequence(reader, view, depth, &head, 2),
        6 => {
            let meaning = TAGS
                .iter()
                .find(|(number, _)| *number == head.argument)
                .map_or_else(
                    || format!("tag {}", head.argument),
                    |(number, said)| format!("tag {number}: {said}"),
                );
            if !view.tags.contains(&meaning) {
                view.tags.push(meaning);
            }
            item(reader, view, depth, "")
        }
        7 => {
            // A simple value's argument may be a half, single or double
            // precision float, already consumed by `head`.
            Some(())
        }
        _ => None,
    }
}

/// One item said in words: what it is, and what it holds.
fn described(head: &Head, name: &str) -> String {
    if head.indefinite {
        return format!("{name} (indefinite)");
    }
    match head.major {
        0 => format!("unsigned integer ({})", head.argument),
        1 => format!("negative integer (-{})", i128::from(head.argument) + 1),
        // Major type 7 carries the simple values and every float there
        // is; which one depends on how wide the argument was.
        7 => match head.width {
            2 => "half-precision float".to_owned(),
            4 => u32::try_from(head.argument).map_or_else(
                |_| "float".to_owned(),
                |bits| format!("float ({})", f32::from_bits(bits)),
            ),
            8 => format!("float ({})", f64::from_bits(head.argument)),
            _ => match head.argument {
                20 => "false".to_owned(),
                21 => "true".to_owned(),
                22 => "null".to_owned(),
                23 => "undefined".to_owned(),
                other => format!("simple value ({other})"),
            },
        },
        _ => format!("{name} ({})", head.argument),
    }
}

/// Reads an array or a map: `per` items for each entry.
fn read_sequence(
    reader: &mut Reader,
    view: &mut CborView,
    depth: usize,
    head: &Head,
    per: usize,
) -> Option<()> {
    if head.indefinite {
        return until_break(reader, view, depth);
    }
    let entries = usize::try_from(head.argument).ok()?;
    for _ in 0..entries.checked_mul(per)? {
        item(reader, view, depth + 1, "")?;
    }
    Some(())
}

/// Reads items until the break byte.
fn until_break(reader: &mut Reader, view: &mut CborView, depth: usize) -> Option<()> {
    loop {
        if reader.bytes.get(reader.at) == Some(&0xff) {
            reader.at += 1;
            return Some(());
        }
        item(reader, view, depth + 1, "")?;
    }
}

/// The text of the next item, when it is a text string, without moving
/// the reader on.
fn peek_text(bytes: &[u8], at: usize) -> Option<String> {
    let mut reader = Reader { bytes, at };
    let head = reader.head()?;
    if head.major != 3 || head.indefinite {
        return None;
    }
    let length = usize::try_from(head.argument).ok()?;
    let end = reader.at.checked_add(length)?;
    let slice = bytes.get(reader.at..end)?;
    Some(String::from_utf8_lossy(slice).into_owned())
}

/// Everything [`CborView`] holds, read from `bytes`.
fn parse(bytes: &[u8]) -> Option<CborView> {
    let mut view = CborView {
        self_describing: bytes.starts_with(&[0xd9, 0xd9, 0xf7]),
        top_level: "unread".to_owned(),
        entries: None,
        keys: Vec::new(),
        tags: Vec::new(),
        shape: Vec::new(),
        indefinite_items: Vec::new(),
        trailing_bytes: 0,
        truncated: false,
    };
    // The self-describing tag is part of the first item, so it is read
    // like any other tag and the item underneath it is the real one.
    let mut reader = Reader { bytes, at: 0 };
    let start = if view.self_describing { 3 } else { 0 };
    reader.at = start;

    let head = {
        let mut ahead = Reader { bytes, at: start };
        ahead.head()?
    };
    MAJOR
        .get(head.major as usize)
        .copied()
        .unwrap_or("unknown")
        .clone_into(&mut view.top_level);
    if matches!(head.major, 4 | 5) && !head.indefinite {
        view.entries = usize::try_from(head.argument).ok();
    }
    if view.self_describing {
        view.tags
            .push("tag 55799: the self-describing tag".to_owned());
    }

    // The keys of a top-level map, read before the walk consumes them.
    if head.major == 5 && !head.indefinite {
        let mut keys = Reader { bytes, at: start };
        keys.head()?;
        for _ in 0..head.argument.min(64) {
            match peek_text(bytes, keys.at) {
                Some(key) => view.keys.push(key),
                None => break,
            }
            // Step over the key, then over its value.
            item(&mut keys, &mut CborView { ..view.clone() }, MAX_DEPTH, "")?;
            item(&mut keys, &mut CborView { ..view.clone() }, MAX_DEPTH, "")?;
        }
    }

    item(&mut reader, &mut view, 0, "")?;
    view.trailing_bytes = bytes.len().saturating_sub(reader.at);
    Some(view)
}

/// Whether `bytes` are Concise Binary Object Representation.
fn looks_like_it(bytes: &[u8]) -> bool {
    if bytes.starts_with(&[0xd9, 0xd9, 0xf7]) {
        return true;
    }
    // Without the tag, the only honest test is to read the thing: a
    // well-formed item that consumes the whole prefix, and is a container
    // rather than a bare number that every file in the world starts with.
    let Some(view) = parse(bytes) else {
        return false;
    };
    view.trailing_bytes == 0 && matches!(view.top_level.as_str(), "map" | "array")
}

/// The CBOR plugin's core half.
#[derive(Debug, Default)]
pub struct CborCore;

impl PluginCore for CborCore {
    fn name(&self) -> &'static str {
        "cbor"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_it(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let mut view = parse(slice).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "not a well-formed CBOR item")
        })?;
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The CBOR plugin's presentation half.
#[derive(Debug, Default)]
pub struct CborPresentation;

impl PluginPresentation for CborPresentation {
    fn name(&self) -> &'static str {
        "cbor"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "CBOR",
            tint: 0x0046_6f7a,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: CborView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "CBOR: a {}{}",
            view.top_level,
            view.entries
                .map_or_else(String::new, |count| format!(" of {count} entry(ies)"))
        ));
        if view.self_describing {
            lines.push("Opens with the self-describing tag, so the bytes say what".to_owned());
            lines.push("they are without being told.".to_owned());
        }
        if !view.keys.is_empty() {
            lines.push(format!("Keys: {}", view.keys.join(", ")));
        }
        if !view.tags.is_empty() {
            lines.push(format!("{} tag(s):", view.tags.len()));
            for tag in &view.tags {
                lines.push(format!("  {tag}"));
            }
        }
        if !view.shape.is_empty() {
            lines.push("Shape:".to_owned());
            for line in &view.shape {
                lines.push(format!("  {line}"));
            }
        }
        if !view.indefinite_items.is_empty() {
            lines.push("Written without a length up front, so a reader has to".to_owned());
            lines.push("consume each to its break byte before it knows the size:".to_owned());
            for said in &view.indefinite_items {
                lines.push(format!("  {said}"));
            }
        }
        if view.trailing_bytes > 0 {
            lines.push(format!(
                "{} byte(s) after the first item, so this file holds more than",
                view.trailing_bytes
            ));
            lines.push("one value and a reader expecting one will stop early.".to_owned());
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{CborCore, CborPresentation, CborView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    /// `{"a": 1, "b": [2, 3]}`, with no self-describing tag.
    const MAP: &[u8] = &[0xa2, 0x61, b'a', 0x01, 0x61, b'b', 0x82, 0x02, 0x03];

    #[test]
    fn sniffs_a_document_with_the_self_describing_tag() {
        assert!(CborCore.sniff(&[0xd9, 0xd9, 0xf7, 0xa0]));
    }

    #[test]
    fn sniffs_a_map_without_one() {
        assert!(CborCore.sniff(MAP));
    }

    #[test]
    fn does_not_claim_a_bare_number_or_rubbish() {
        assert!(
            !CborCore.sniff(&[0x01]),
            "every file in the world starts like that"
        );
        assert!(!CborCore.sniff(b"#!/bin/sh\necho hello\n"));
        assert!(!CborCore.sniff(b""));
    }

    #[test]
    fn a_truncated_item_is_not_a_document() {
        // A map promising two entries and holding one.
        assert!(!CborCore.sniff(&[0xa2, 0x61, b'a', 0x01]));
    }

    #[test]
    fn reads_the_top_level_and_its_keys() {
        let view = parse(MAP).unwrap();

        assert_eq!(view.top_level, "map");
        assert_eq!(view.entries, Some(2));
        assert_eq!(view.keys, vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(view.trailing_bytes, 0);
    }

    #[test]
    fn a_float_is_said_as_a_number_not_as_its_bits() {
        // `{"r": 12.5}` - a double, which is major type 7 with an eight
        // byte argument.
        let mut bytes = vec![0xa1, 0x61, b'r', 0xfb];
        bytes.extend_from_slice(&12.5f64.to_bits().to_be_bytes());
        let view = parse(&bytes).unwrap();

        assert!(
            view.shape.iter().any(|line| line.contains("float (12.5)")),
            "expected a number among {:?}",
            view.shape
        );
        assert!(!view.shape.iter().any(|line| line.contains("4623")));
    }

    #[test]
    fn the_simple_values_are_said_by_name() {
        let view = parse(&[0xa3, 0x61, b'a', 0xf5, 0x61, b'b', 0xf4, 0x61, b'c', 0xf6]).unwrap();

        assert!(view.shape.iter().any(|line| line.ends_with("true")));
        assert!(view.shape.iter().any(|line| line.ends_with("false")));
        assert!(view.shape.iter().any(|line| line.ends_with("null")));
    }

    #[test]
    fn names_each_tag_it_meets() {
        // `1(1789000000)` - a point in time.
        let bytes = [0xa1, 0x61, b't', 0xc1, 0x1a, 0x6a, 0x9c, 0x1e, 0x00];
        let view = parse(&bytes).unwrap();

        assert!(
            view.tags
                .iter()
                .any(|tag| tag.contains("seconds since the epoch"))
        );
    }

    #[test]
    fn names_an_item_written_without_a_length() {
        // `{"f": [_ 1, 2]}`
        let bytes = [0xa1, 0x61, b'f', 0x9f, 0x01, 0x02, 0xff];
        let view = parse(&bytes).unwrap();

        assert_eq!(view.indefinite_items.len(), 1);
        assert!(view.indefinite_items[0].contains("no length up front"));
    }

    #[test]
    fn counts_what_is_left_after_the_first_item() {
        // Two maps one after another: this is not one document.
        let mut bytes = MAP.to_vec();
        bytes.extend_from_slice(MAP);
        let view = parse(&bytes).unwrap();

        assert_eq!(view.trailing_bytes, MAP.len());
        assert!(
            !CborCore.sniff(&bytes),
            "a second item means this is not one value"
        );
    }

    #[test]
    fn presents_the_indefinite_warning_with_its_reason() {
        let bytes = [0xa1, 0x61, b'f', 0x9f, 0x01, 0xff];
        let data = serde_json::to_value(parse(&bytes).unwrap()).unwrap();

        let lines = CborPresentation.present(&data);

        assert!(lines[0].starts_with("CBOR: a map"));
        assert!(lines.iter().any(|line| line.contains("break byte")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/cbor/readings.cbor");

        let data = CborCore.view(&path).unwrap();
        let view: CborView = serde_json::from_value(data).unwrap();

        assert!(view.self_describing);
        assert_eq!(view.top_level, "map");
        assert_eq!(view.entries, Some(8));
        assert!(view.keys.len() >= 8);
        assert!(view.tags.len() >= 3);
        assert!(view.shape.len() >= 8);
        assert_eq!(view.indefinite_items.len(), 2);
        assert_eq!(view.trailing_bytes, 0);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::CborCore),
            plugin_api::PluginPresentation::extensions(&crate::CborPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
