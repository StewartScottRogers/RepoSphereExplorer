//! `MessagePack` file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// A decoded `MessagePack` value, kept close to `rmpv::Value`'s own shape
/// rather than converted to a JSON value, since `MessagePack` allows map keys
/// and integer magnitudes (full `u64`) that JSON's object/number types
/// can't represent losslessly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MsgpackValue {
    /// The `nil` value.
    Nil,
    /// A boolean value.
    Bool(bool),
    /// An integer, rendered as decimal text so it's exact for the full
    /// signed and unsigned 64-bit range `MessagePack` allows.
    Int(String),
    /// A 32- or 64-bit float.
    Float(f64),
    /// A UTF-8 string, or a placeholder noting an invalid one (`MessagePack`
    /// strings are not guaranteed valid UTF-8 the way JSON's are).
    Str(String),
    /// A binary blob, kept only as its byte length.
    Bytes(usize),
    /// An ordered list of values.
    Array(Vec<MsgpackValue>),
    /// An ordered list of key/value pairs; unlike a JSON object, a
    /// `MessagePack` map's keys may be any value, not just strings.
    Map(Vec<(MsgpackValue, MsgpackValue)>),
    /// An application-defined extension type: its type id and byte length.
    Ext {
        /// The extension's application-defined type id.
        type_id: i8,
        /// The number of bytes of extension data.
        length: usize,
    },
}

/// View data produced by [`MsgpackCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgpackView {
    /// The file's size in bytes.
    pub byte_count: usize,
    /// The file's leading value, decoded from `MessagePack`, or `None` if it
    /// could not be decoded.
    pub parsed: Option<MsgpackValue>,
}

/// Whether `first` is the format byte of a `MessagePack` map or array: a
/// fixmap/fixarray, or a 16- or 32-bit map/array header. Checked before
/// attempting a full decode so a file that opens with a scalar (a bare
/// integer, string, etc. — far too common in arbitrary binary data to be a
/// useful marker on its own) is rejected cheaply.
fn looks_like_container_marker(first: u8) -> bool {
    matches!(first, 0x80..=0x8f | 0xde | 0xdf | 0x90..=0x9f | 0xdc | 0xdd)
}

/// PNG's fixed 8-byte magic. Its first byte, `0x89`, is also `MessagePack`'s
/// format code for "fixmap with 9 entries", so a PNG's leading bytes pass
/// [`looks_like_container_marker`] and, being far shorter than any real
/// 9-entry map, run out of input while decoding — which [`is_eof_error`]
/// otherwise treats as an inconclusive (bounded-prefix) failure rather than
/// a rejection.
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// HDF5's fixed 8-byte magic, which collides with `MessagePack` in the same
/// way as [`PNG_MAGIC`] and for the same reason.
const HDF5_MAGIC: [u8; 8] = [0x89, b'H', b'D', b'F', 0x0d, 0x0a, 0x1a, 0x0a];

/// Whether `err` is a decode failure caused by running out of bytes, rather
/// than an invalid one — expected when `sniff` only sees a bounded prefix
/// of a larger value.
fn is_eof_error(err: &rmpv::decode::Error) -> bool {
    match err {
        rmpv::decode::Error::InvalidMarkerRead(io_err)
        | rmpv::decode::Error::InvalidDataRead(io_err) => {
            io_err.kind() == io::ErrorKind::UnexpectedEof
        }
        rmpv::decode::Error::DepthLimitExceeded => false,
    }
}

/// Whether `prefix` opens with a `MessagePack` map or array: its first byte
/// is a map/array format code, and decoding it either succeeds as a map or
/// array, or runs out of bytes trying (expected for a value bigger than the
/// bounded prefix `sniff` sees). Deliberately does not match a prefix that
/// opens with a `MessagePack` scalar (a bare integer, string, etc.), since
/// those format bytes overlap too much of arbitrary binary data to be a
/// useful marker on their own. [`PNG_MAGIC`] and [`HDF5_MAGIC`] are excluded
/// explicitly: both are fixed, short magics that share `MessagePack`'s
/// fixmap-with-9-entries format code, so without the exclusion they would
/// otherwise fall into the EOF-as-match branch below.
fn looks_like_msgpack(prefix: &[u8]) -> bool {
    let Some(&first) = prefix.first() else {
        return false;
    };
    if !looks_like_container_marker(first) {
        return false;
    }
    if prefix.starts_with(&PNG_MAGIC) || prefix.starts_with(&HDF5_MAGIC) {
        return false;
    }
    let mut cursor = prefix;
    match rmpv::decode::read_value(&mut cursor) {
        Ok(rmpv::Value::Map(_) | rmpv::Value::Array(_)) => true,
        Ok(_) => false,
        Err(err) => is_eof_error(&err),
    }
}

/// Renders a `MessagePack` scalar, or a compact one-line summary of a
/// container, as text — used both for leaf values and for map key labels
/// (a `MessagePack` map key may itself be a container).
fn render_scalar(value: &MsgpackValue) -> String {
    match value {
        MsgpackValue::Nil => "null".to_owned(),
        MsgpackValue::Bool(value) => value.to_string(),
        MsgpackValue::Int(value) => value.clone(),
        MsgpackValue::Float(value) => value.to_string(),
        MsgpackValue::Str(value) => format!("{value:?}"),
        MsgpackValue::Bytes(length) => format!("<{length} bytes>"),
        MsgpackValue::Array(items) => format!("[{} item{}]", items.len(), plural(items.len())),
        MsgpackValue::Map(entries) => {
            format!("{{{} entr{}}}", entries.len(), plural_y(entries.len()))
        }
        MsgpackValue::Ext { type_id, length } => format!("<ext type {type_id}, {length} bytes>"),
    }
}

/// The English plural suffix for `count`.
fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// The English `y`/`ies` suffix for `count` (as in "entry"/"entries").
fn plural_y(count: usize) -> &'static str {
    if count == 1 { "y" } else { "ies" }
}

/// A tree-line label: the root has none, a map entry is labelled by its
/// (already-rendered) key, an array entry by its index.
enum Label {
    Root,
    Key(String),
    Index(usize),
}

/// Appends `value` to `lines` as an indented tree: one line per node, with
/// arrays and maps expanded into their children at one deeper indent.
fn push_tree_lines(value: &MsgpackValue, depth: usize, label: &Label, lines: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    let prefix = match label {
        Label::Root => String::new(),
        Label::Key(key) => format!("{key}: "),
        Label::Index(index) => format!("[{index}]: "),
    };
    match value {
        MsgpackValue::Array(items) => {
            lines.push(format!(
                "{indent}{prefix}[] ({} item{})",
                items.len(),
                plural(items.len())
            ));
            for (index, child) in items.iter().enumerate() {
                push_tree_lines(child, depth + 1, &Label::Index(index), lines);
            }
        }
        MsgpackValue::Map(entries) => {
            lines.push(format!(
                "{indent}{prefix}{{}} ({} entr{})",
                entries.len(),
                plural_y(entries.len())
            ));
            for (key, child) in entries {
                push_tree_lines(child, depth + 1, &Label::Key(render_scalar(key)), lines);
            }
        }
        scalar => lines.push(format!("{indent}{prefix}{}", render_scalar(scalar))),
    }
}

/// Converts a decoded `rmpv::Value` into a [`MsgpackValue`].
fn convert(value: &rmpv::Value) -> MsgpackValue {
    match value {
        rmpv::Value::Nil => MsgpackValue::Nil,
        rmpv::Value::Boolean(value) => MsgpackValue::Bool(*value),
        rmpv::Value::Integer(value) => MsgpackValue::Int(value.to_string()),
        rmpv::Value::F32(value) => MsgpackValue::Float(f64::from(*value)),
        rmpv::Value::F64(value) => MsgpackValue::Float(*value),
        rmpv::Value::String(value) => match value.as_str() {
            Some(text) => MsgpackValue::Str(text.to_owned()),
            None => MsgpackValue::Str(format!("<invalid utf-8, {} bytes>", value.as_bytes().len())),
        },
        rmpv::Value::Binary(value) => MsgpackValue::Bytes(value.len()),
        rmpv::Value::Array(items) => MsgpackValue::Array(items.iter().map(convert).collect()),
        rmpv::Value::Map(entries) => MsgpackValue::Map(
            entries
                .iter()
                .map(|(key, value)| (convert(key), convert(value)))
                .collect(),
        ),
        rmpv::Value::Ext(type_id, data) => MsgpackValue::Ext {
            type_id: *type_id,
            length: data.len(),
        },
    }
}

/// The `MessagePack` plugin's core half.
#[derive(Debug, Default)]
pub struct MsgpackCore;

impl PluginCore for MsgpackCore {
    fn name(&self) -> &'static str {
        "msgpack"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_msgpack(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let byte_count = bytes.len();
        let mut cursor = bytes.as_slice();
        let parsed = rmpv::decode::read_value(&mut cursor)
            .ok()
            .as_ref()
            .map(convert);
        let view = MsgpackView { byte_count, parsed };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The `MessagePack` plugin's presentation half.
#[derive(Debug, Default)]
pub struct MsgpackPresentation;

impl PluginPresentation for MsgpackPresentation {
    fn name(&self) -> &'static str {
        "msgpack"
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: MsgpackView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        match &view.parsed {
            Some(value) => {
                let mut lines = Vec::new();
                push_tree_lines(value, 0, &Label::Root, &mut lines);
                lines
            }
            None => vec![format!(
                "could not parse as MessagePack ({} bytes)",
                view.byte_count
            )],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MsgpackCore, MsgpackPresentation, MsgpackValue, MsgpackView};
    use plugin_api::{PluginCore, PluginPresentation};
    use rmpv::Value as RmpValue;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-msgpack-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// Writes a real MessagePack-encoded `value` to `path`.
    fn write_test_msgpack(path: &std::path::Path, value: &RmpValue) {
        let mut buf = Vec::new();
        rmpv::encode::write_value(&mut buf, value).unwrap();
        std::fs::write(path, buf).unwrap();
    }

    #[test]
    fn sniffs_map_and_array_containers() {
        let mut map_bytes = Vec::new();
        rmpv::encode::write_value(
            &mut map_bytes,
            &RmpValue::Map(vec![(RmpValue::from("name"), RmpValue::from("Alice"))]),
        )
        .unwrap();
        assert!(MsgpackCore.sniff(&map_bytes));

        let mut array_bytes = Vec::new();
        rmpv::encode::write_value(
            &mut array_bytes,
            &RmpValue::Array(vec![RmpValue::from(1), RmpValue::from(2)]),
        )
        .unwrap();
        assert!(MsgpackCore.sniff(&array_bytes));
    }

    #[test]
    fn sniffs_a_container_truncated_to_a_bounded_prefix() {
        let mut bytes = Vec::new();
        let entries: Vec<RmpValue> = (0..500).map(RmpValue::from).collect();
        rmpv::encode::write_value(&mut bytes, &RmpValue::Array(entries)).unwrap();

        assert!(MsgpackCore.sniff(&bytes[..64]));
    }

    #[test]
    fn does_not_sniff_a_bare_scalar_or_plain_text() {
        let mut int_bytes = Vec::new();
        rmpv::encode::write_value(&mut int_bytes, &RmpValue::from(42)).unwrap();
        assert!(!MsgpackCore.sniff(&int_bytes));

        assert!(!MsgpackCore.sniff(b"just a regular line of text\n"));
        assert!(!MsgpackCore.sniff(b""));
    }

    #[test]
    fn does_not_sniff_a_png_magic_prefix() {
        let png_prefix: &[u8] = &[
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
            b'D', b'R',
        ];
        assert!(!MsgpackCore.sniff(png_prefix));
    }

    #[test]
    fn does_not_sniff_an_hdf5_magic_prefix() {
        let hdf5_prefix: &[u8] = &[
            0x89, b'H', b'D', b'F', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        assert!(!MsgpackCore.sniff(hdf5_prefix));
    }

    #[test]
    fn views_a_real_msgpack_file_and_parses_it() {
        let path = unique_temp_file("person.msgpack");
        write_test_msgpack(
            &path,
            &RmpValue::Map(vec![
                (RmpValue::from("name"), RmpValue::from("Alice")),
                (RmpValue::from("age"), RmpValue::from(30)),
            ]),
        );

        let data = MsgpackCore.view(&path).unwrap();
        let view: MsgpackView = serde_json::from_value(data).unwrap();

        assert_eq!(
            view.parsed,
            Some(MsgpackValue::Map(vec![
                (
                    MsgpackValue::Str("name".to_owned()),
                    MsgpackValue::Str("Alice".to_owned())
                ),
                (
                    MsgpackValue::Str("age".to_owned()),
                    MsgpackValue::Int("30".to_owned())
                ),
            ]))
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_file_that_does_not_decode_as_msgpack() {
        let path = unique_temp_file("invalid.msgpack");
        // A fixmap header declaring one entry, with no bytes following it:
        // decoding runs out of input before it can read that entry's key.
        std::fs::write(&path, b"\x81").unwrap();

        let data = MsgpackCore.view(&path).unwrap();
        let view: MsgpackView = serde_json::from_value(data).unwrap();

        assert_eq!(view.parsed, None);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_a_tree_of_nested_maps_and_arrays() {
        let data = serde_json::to_value(MsgpackView {
            byte_count: 0,
            parsed: Some(MsgpackValue::Map(vec![
                (
                    MsgpackValue::Str("name".to_owned()),
                    MsgpackValue::Str("Alice".to_owned()),
                ),
                (
                    MsgpackValue::Str("tags".to_owned()),
                    MsgpackValue::Array(vec![
                        MsgpackValue::Str("a".to_owned()),
                        MsgpackValue::Str("b".to_owned()),
                    ]),
                ),
            ])),
        })
        .unwrap();

        let lines = MsgpackPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "{} (2 entries)",
                "  \"name\": \"Alice\"",
                "  \"tags\": [] (2 items)",
                "    [0]: \"a\"",
                "    [1]: \"b\"",
            ]
        );
    }

    #[test]
    fn presents_an_error_message_when_not_parseable() {
        let data = serde_json::to_value(MsgpackView {
            byte_count: 1,
            parsed: None,
        })
        .unwrap();

        let lines = MsgpackPresentation.present(&data);

        assert_eq!(lines, vec!["could not parse as MessagePack (1 bytes)"]);
    }
}
