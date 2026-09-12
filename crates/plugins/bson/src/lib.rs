//! BSON file type plugin: core and presentation halves.
//!
//! Binary JavaScript Object Notation (BSON): documents written one
//! after another, each declaring its own length. This reads how many
//! there are, the fields of the first with their types, the object
//! identifiers, the dates, what each binary subtype says it holds, how
//! deep the nesting goes, and the fields only some documents carry.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["bson"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One top-level field of the first document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// Its name.
    pub name: String,
    /// Its type, said in words.
    pub kind: String,
}

/// View data produced by [`BsonCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BsonView {
    /// How many documents the file holds, one after another.
    pub documents: usize,
    /// The top-level fields of the first document.
    pub fields: Vec<Field>,
    /// The object identifiers found, as hexadecimal.
    pub object_ids: Vec<String>,
    /// The dates found, as milliseconds since the epoch.
    pub dates: Vec<i64>,
    /// The binary fields, each with what its subtype says it holds.
    pub binary_subtypes: Vec<String>,
    /// How deeply the first document nests.
    pub depth: usize,
    /// Fields whose names differ between the first document and a later
    /// one, which is what makes a collection awkward to read as a table.
    pub ragged_fields: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`], in which case
    /// the count describes what was read rather than what is there.
    pub truncated: bool,
}

/// The element types, by their marker byte.
const TYPES: &[(u8, &str)] = &[
    (0x01, "double"),
    (0x02, "string"),
    (0x03, "embedded document"),
    (0x04, "array"),
    (0x05, "binary"),
    (0x06, "undefined"),
    (0x07, "object identifier"),
    (0x08, "boolean"),
    (0x09, "date"),
    (0x0a, "null"),
    (0x0b, "regular expression"),
    (0x0d, "JavaScript"),
    (0x0f, "JavaScript with scope"),
    (0x10, "32-bit integer"),
    (0x11, "timestamp"),
    (0x12, "64-bit integer"),
    (0x13, "128-bit decimal"),
    (0xff, "minimum key"),
    (0x7f, "maximum key"),
];

/// What each binary subtype says the bytes hold.
const SUBTYPES: &[(u8, &str)] = &[
    (0x00, "plain bytes"),
    (0x01, "a function"),
    (0x02, "plain bytes, the old way"),
    (0x03, "a universally unique identifier, the old way"),
    (0x04, "a universally unique identifier"),
    (0x05, "a message digest"),
    (0x06, "an encrypted value"),
    (0x07, "a compressed column"),
    (0x08, "a sensitive value"),
];

/// A reader over the bytes, which stops rather than panicking.
struct Reader<'a> {
    /// The bytes being read.
    bytes: &'a [u8],
    /// How far in the reader has got.
    at: usize,
}

impl Reader<'_> {
    /// The next `count` bytes.
    fn take(&mut self, count: usize) -> Option<&[u8]> {
        let end = self.at.checked_add(count)?;
        let slice = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(slice)
    }

    /// A little-endian 32-bit signed integer.
    fn int32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    /// A little-endian 64-bit signed integer.
    fn int64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    /// A name, which runs to its null terminator.
    fn name(&mut self) -> Option<String> {
        let end = self.bytes[self.at..].iter().position(|byte| *byte == 0)?;
        let slice = self.take(end)?;
        let name = String::from_utf8_lossy(slice).into_owned();
        self.at += 1;
        Some(name)
    }
}

/// Reads one element's value, adding what it finds to `view`.
fn value(reader: &mut Reader, kind: u8, view: &mut BsonView, depth: usize) -> Option<()> {
    view.depth = view.depth.max(depth);
    match kind {
        0x01 | 0x09 | 0x11 | 0x12 => {
            let raw = reader.int64()?;
            if kind == 0x09 {
                view.dates.push(raw);
            }
            Some(())
        }
        0x02 | 0x0d => {
            let length = usize::try_from(reader.int32()?).ok()?;
            reader.take(length).map(|_| ())
        }
        0x03 | 0x04 => document(reader, view, depth + 1, false).map(|_| ()),
        0x05 => {
            let length = usize::try_from(reader.int32()?).ok()?;
            let subtype = *reader.take(1)?.first()?;
            let said = SUBTYPES
                .iter()
                .find(|(number, _)| *number == subtype)
                .map_or_else(
                    || format!("subtype 0x{subtype:02x}, which this reader does not know"),
                    |(_, meaning)| (*meaning).to_owned(),
                );
            if !view.binary_subtypes.contains(&said) {
                view.binary_subtypes.push(said);
            }
            reader.take(length).map(|_| ())
        }
        0x07 => {
            let raw = reader.take(12)?;
            let hexadecimal = raw.iter().fold(String::new(), |mut out, byte| {
                use std::fmt::Write as _;
                let _ = write!(out, "{byte:02x}");
                out
            });
            view.object_ids.push(hexadecimal);
            Some(())
        }
        0x08 => reader.take(1).map(|_| ()),
        0x06 | 0x0a | 0xff | 0x7f => Some(()),
        0x10 => reader.int32().map(|_| ()),
        0x13 => reader.take(16).map(|_| ()),
        0x0b => {
            reader.name()?;
            reader.name().map(|_| ())
        }
        0x0f => {
            let length = usize::try_from(reader.int32()?).ok()?;
            reader.take(length.checked_sub(4)?).map(|_| ())
        }
        _ => None,
    }
}

/// Reads one document, returning the names of its fields.
fn document(
    reader: &mut Reader,
    view: &mut BsonView,
    depth: usize,
    top_level: bool,
) -> Option<Vec<String>> {
    let start = reader.at;
    let length = usize::try_from(reader.int32()?).ok()?;
    if length < 5 || start.checked_add(length)? > reader.bytes.len() {
        return None;
    }
    let mut names = Vec::new();
    loop {
        let kind = *reader.take(1)?.first()?;
        if kind == 0 {
            break;
        }
        let name = reader.name()?;
        let said = TYPES
            .iter()
            .find(|(number, _)| *number == kind)
            .map(|(_, meaning)| (*meaning).to_owned())?;
        if top_level && view.fields.len() < 64 {
            view.fields.push(Field {
                name: name.clone(),
                kind: said,
            });
        }
        names.push(name);
        value(reader, kind, view, depth)?;
    }
    // The declared length is the authority: trust it over where the walk
    // happened to stop, and refuse a document that disagrees.
    (reader.at == start + length).then_some(names)
}

/// Everything [`BsonView`] holds, read from `bytes`.
fn parse(bytes: &[u8]) -> Option<BsonView> {
    let mut view = BsonView {
        documents: 0,
        fields: Vec::new(),
        object_ids: Vec::new(),
        dates: Vec::new(),
        binary_subtypes: Vec::new(),
        depth: 0,
        ragged_fields: Vec::new(),
        truncated: false,
    };
    let mut reader = Reader { bytes, at: 0 };
    let mut first: Option<Vec<String>> = None;

    while reader.at < bytes.len() {
        // A document promising more bytes than remain is the tail of a
        // file cut off at the view limit, not a malformed one.
        let mut ahead = Reader {
            bytes,
            at: reader.at,
        };
        let Some(length) = ahead.int32().and_then(|raw| usize::try_from(raw).ok()) else {
            break;
        };
        if reader.at + length > bytes.len() {
            break;
        }
        let is_first = view.documents == 0;
        let names = document(&mut reader, &mut view, 1, is_first)?;
        view.documents += 1;
        match &first {
            None => first = Some(names),
            Some(expected) => {
                for name in &names {
                    if !expected.contains(name) && !view.ragged_fields.contains(name) {
                        view.ragged_fields.push(name.clone());
                    }
                }
                for name in expected {
                    if !names.contains(name) && !view.ragged_fields.contains(name) {
                        view.ragged_fields.push(name.clone());
                    }
                }
            }
        }
    }
    (view.documents > 0).then_some(view)
}

/// Whether `bytes` are Binary JSON.
fn looks_like_it(bytes: &[u8]) -> bool {
    // The leading length has to describe a document that is actually
    // there, and the document has to end where it said it would. Four
    // bytes of anything can look like a length; a document that closes
    // exactly on its own terminator cannot.
    parse(bytes).is_some_and(|view| !view.fields.is_empty())
}

/// The BSON plugin's core half.
#[derive(Debug, Default)]
pub struct BsonCore;

impl PluginCore for BsonCore {
    fn name(&self) -> &'static str {
        "bson"
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
            io::Error::new(
                io::ErrorKind::InvalidData,
                "not a well-formed BSON document",
            )
        })?;
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The BSON plugin's presentation half.
#[derive(Debug, Default)]
pub struct BsonPresentation;

impl PluginPresentation for BsonPresentation {
    fn name(&self) -> &'static str {
        "bson"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "BSON",
            tint: 0x0058_9636,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: BsonView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "BSON: {} document(s){}",
            view.documents,
            if view.truncated { ", so far" } else { "" }
        ));
        lines.push(format!("Nested {} level(s) deep", view.depth));
        if !view.fields.is_empty() {
            lines.push("First document:".to_owned());
            for field in &view.fields {
                lines.push(format!("  {} - {}", field.name, field.kind));
            }
        }
        if !view.object_ids.is_empty() {
            lines.push(format!(
                "{} object identifier(s): {}",
                view.object_ids.len(),
                view.object_ids.join(", ")
            ));
        }
        if !view.dates.is_empty() {
            lines.push(format!(
                "{} date(s), as milliseconds since the epoch: {}",
                view.dates.len(),
                view.dates
                    .iter()
                    .map(i64::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !view.binary_subtypes.is_empty() {
            lines.push("Binary fields, and what each subtype says it holds:".to_owned());
            for said in &view.binary_subtypes {
                lines.push(format!("  {said}"));
            }
        }
        if !view.ragged_fields.is_empty() {
            lines.push("Present in some documents and not others, so anything".to_owned());
            lines.push("reading the collection as a table has to allow for it:".to_owned());
            for name in &view.ragged_fields {
                lines.push(format!("  {name}"));
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
    use super::{BsonCore, BsonPresentation, BsonView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    /// `{"a": 1}` as a 32-bit integer: length, type, name, value, end.
    fn one_document() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&12i32.to_le_bytes());
        out.push(0x10);
        out.extend_from_slice(b"a\0");
        out.extend_from_slice(&1i32.to_le_bytes());
        out.push(0x00);
        out
    }

    #[test]
    fn sniffs_a_document() {
        assert!(BsonCore.sniff(&one_document()));
    }

    #[test]
    fn does_not_claim_bytes_whose_length_is_a_coincidence() {
        // Four plausible length bytes, then nothing that reads as an
        // element.
        assert!(!BsonCore.sniff(&[0x0c, 0x00, 0x00, 0x00, 0x99, 0x99, 0x99, 0x99]));
        assert!(!BsonCore.sniff(b"#!/bin/sh\necho hello\n"));
        assert!(!BsonCore.sniff(b""));
    }

    #[test]
    fn a_document_that_does_not_end_where_it_promised_is_refused() {
        let mut bytes = one_document();
        // Claim four more bytes than are there.
        bytes[0] += 4;
        bytes.extend_from_slice(&[0, 0, 0, 0]);

        assert!(
            parse(&bytes).is_none(),
            "the walk ends before the declared length"
        );
    }

    #[test]
    fn reads_every_field_of_the_first_document() {
        let view = parse(&one_document()).unwrap();

        assert_eq!(view.documents, 1);
        assert_eq!(view.fields.len(), 1);
        assert_eq!(view.fields[0].name, "a");
        assert_eq!(view.fields[0].kind, "32-bit integer");
    }

    #[test]
    fn counts_documents_written_one_after_another() {
        let mut bytes = one_document();
        bytes.extend_from_slice(&one_document());
        bytes.extend_from_slice(&one_document());

        assert_eq!(parse(&bytes).unwrap().documents, 3);
    }

    #[test]
    fn names_a_field_only_some_documents_carry() {
        let mut bytes = one_document();
        // A second document with a different field.
        let mut other = Vec::new();
        other.extend_from_slice(&12i32.to_le_bytes());
        other.push(0x10);
        other.extend_from_slice(b"b\0");
        other.extend_from_slice(&2i32.to_le_bytes());
        other.push(0x00);
        bytes.extend_from_slice(&other);

        let view = parse(&bytes).unwrap();
        assert_eq!(view.documents, 2);
        assert_eq!(view.ragged_fields, vec!["b".to_owned(), "a".to_owned()]);
    }

    #[test]
    fn presents_the_ragged_fields_with_their_consequence() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/bson/readings.bson");
        let data = BsonCore.view(&path).unwrap();

        let lines = BsonPresentation.present(&data);

        assert!(lines[0].starts_with("BSON: 4 document(s)"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("universally unique identifier"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("object identifier(s)"))
        );
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/bson/readings.bson");

        let data = BsonCore.view(&path).unwrap();
        let view: BsonView = serde_json::from_value(data).unwrap();

        assert_eq!(view.documents, 4);
        assert!(view.fields.len() >= 10);
        assert!(view.fields.iter().any(|f| f.kind == "date"));
        assert!(view.fields.iter().any(|f| f.kind == "embedded document"));
        assert!(view.fields.iter().any(|f| f.kind == "array"));
        assert!(view.fields.iter().any(|f| f.kind == "null"));
        assert_eq!(view.object_ids.len(), 4);
        assert_eq!(view.dates.len(), 4);
        assert_eq!(
            view.ragged_fields,
            vec!["reviewed_by".to_owned(), "review_note".to_owned()],
            "one reading was annotated by hand and the others were not"
        );
        assert_eq!(view.binary_subtypes.len(), 2);
        assert!(view.depth >= 2);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::BsonCore),
            plugin_api::PluginPresentation::extensions(&crate::BsonPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
