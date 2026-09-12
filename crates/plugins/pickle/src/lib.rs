//! Python pickle file type plugin: core and presentation halves.
//!
//! **Nothing here is ever unpickled.** A pickle is a program for a small
//! stack machine, and loading one runs it - `REDUCE` calls whatever
//! `GLOBAL` last named, so a pickle can call any function importable on
//! the machine that opens it. This plugin reads the opcode stream and
//! never executes a byte of it, which is the only safe way to look at
//! one that arrived from somewhere else.
//!
//! It reports the protocol, the opcode count, the modules and classes
//! the stream would import, what it builds at the top, and whether it
//! carries the opcodes that make importing happen.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["pickle", "pkl"];

/// The opcode a protocol 2 and later stream opens with.
const PROTO: u8 = 0x80;

/// The opcode that ends a stream.
const STOP: u8 = b'.';

/// The opcodes that name something to import, with what each does.
const IMPORTING: &[(u8, &str)] = &[
    (b'c', "GLOBAL: names a module and an attribute to import"),
    (
        0x93,
        "STACK_GLOBAL: imports whatever two strings on the stack name",
    ),
    (
        b'R',
        "REDUCE: calls what was imported, with the arguments beneath it",
    ),
    (b'b', "BUILD: hands a built object its state"),
    (b'i', "INST: imports a class and calls it"),
    (b'o', "OBJ: calls a class already on the stack"),
    (0x81, "NEWOBJ: calls a class's __new__"),
    (
        0x92,
        "NEWOBJ_EX: calls a class's __new__ with keyword arguments",
    ),
    (
        0x95,
        "FRAME: not itself a call, but only protocol 4 and later write it",
    ),
];

/// The opcodes that construct an object by calling something.
const CONSTRUCTING: &[u8] = &[b'R', b'o', b'i', b'b', 0x81, 0x92];

/// What the stream builds at the top, by the opcode that made it.
const BUILDS: &[(u8, &str)] = &[
    (b'}', "a dictionary"),
    (b'd', "a dictionary"),
    (b']', "a list"),
    (b'l', "a list"),
    (b')', "a tuple"),
    (b't', "a tuple"),
    (0x8f, "a set"),
    (0x8e, "a frozen set"),
];

/// View data produced by [`PickleCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickleView {
    /// The protocol the stream was written to.
    pub protocol: u8,
    /// How many opcodes it holds.
    pub opcodes: usize,
    /// What it builds, as far as the last opcode before the stop says.
    pub builds: String,
    /// Every `module.attribute` the stream would import, in order.
    pub imports: Vec<String>,
    /// The opcodes that would make something happen on loading, each
    /// with what it does.
    pub executing_opcodes: Vec<String>,
    /// Whether the stream ends where it should.
    pub complete: bool,
}

/// A reader over the opcodes, which stops rather than panicking.
struct Reader<'a> {
    /// The bytes being read.
    bytes: &'a [u8],
    /// How far in the reader has got.
    at: usize,
}

impl Reader<'_> {
    /// The next byte.
    fn byte(&mut self) -> Option<u8> {
        let byte = *self.bytes.get(self.at)?;
        self.at += 1;
        Some(byte)
    }

    /// The next `count` bytes.
    fn take(&mut self, count: usize) -> Option<&[u8]> {
        let end = self.at.checked_add(count)?;
        let slice = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(slice)
    }

    /// A little-endian unsigned number of `width` bytes.
    fn number(&mut self, width: usize) -> Option<usize> {
        let slice = self.take(width)?;
        let mut value = 0usize;
        for (shift, byte) in slice.iter().enumerate() {
            value |= (*byte as usize) << (shift * 8);
        }
        Some(value)
    }

    /// A line, which protocol 0 and 1 use for most arguments.
    fn line(&mut self) -> Option<String> {
        let end = self
            .bytes
            .get(self.at..)?
            .iter()
            .position(|byte| *byte == b'\n')?;
        let said = String::from_utf8_lossy(self.take(end)?).trim().to_owned();
        // Step over the newline the line ended at.
        self.at += 1;
        Some(said)
    }
}

/// What follows an opcode.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Argument {
    /// Nothing: the next byte is the next opcode.
    None,
    /// This many bytes, read and discarded.
    Fixed(usize),
    /// A length of this many bytes, then that many bytes of value.
    Counted(usize),
    /// Everything up to the next newline.
    Line,
}

/// Every opcode, with what follows it.
///
/// Generated from Python's own `pickletools`, which is the only list of
/// these that is certainly right: written by hand, `BINPUT`'s one-byte
/// argument was mistaken for a one-byte length prefix, and the reader
/// walked off the end of every stream that used it.
const OPCODES: &[(u8, Argument)] = &[
    (0x49, Argument::Line),       // INT
    (0x4a, Argument::Fixed(4)),   // BININT
    (0x4b, Argument::Fixed(1)),   // BININT1
    (0x4d, Argument::Fixed(2)),   // BININT2
    (0x4c, Argument::Line),       // LONG
    (0x8a, Argument::Counted(1)), // LONG1
    (0x8b, Argument::Counted(4)), // LONG4
    (0x53, Argument::Line),       // STRING
    (0x54, Argument::Counted(4)), // BINSTRING
    (0x55, Argument::Counted(1)), // SHORT_BINSTRING
    (0x42, Argument::Counted(4)), // BINBYTES
    (0x43, Argument::Counted(1)), // SHORT_BINBYTES
    (0x8e, Argument::Counted(8)), // BINBYTES8
    (0x96, Argument::Counted(8)), // BYTEARRAY8
    (0x97, Argument::None),       // NEXT_BUFFER
    (0x98, Argument::None),       // READONLY_BUFFER
    (0x4e, Argument::None),       // NONE
    (0x88, Argument::None),       // NEWTRUE
    (0x89, Argument::None),       // NEWFALSE
    (0x56, Argument::Line),       // UNICODE
    (0x8c, Argument::Counted(1)), // SHORT_BINUNICODE
    (0x58, Argument::Counted(4)), // BINUNICODE
    (0x8d, Argument::Counted(8)), // BINUNICODE8
    (0x46, Argument::Line),       // FLOAT
    (0x47, Argument::Fixed(8)),   // BINFLOAT
    (0x5d, Argument::None),       // EMPTY_LIST
    (0x61, Argument::None),       // APPEND
    (0x65, Argument::None),       // APPENDS
    (0x6c, Argument::None),       // LIST
    (0x29, Argument::None),       // EMPTY_TUPLE
    (0x74, Argument::None),       // TUPLE
    (0x85, Argument::None),       // TUPLE1
    (0x86, Argument::None),       // TUPLE2
    (0x87, Argument::None),       // TUPLE3
    (0x7d, Argument::None),       // EMPTY_DICT
    (0x64, Argument::None),       // DICT
    (0x73, Argument::None),       // SETITEM
    (0x75, Argument::None),       // SETITEMS
    (0x8f, Argument::None),       // EMPTY_SET
    (0x90, Argument::None),       // ADDITEMS
    (0x91, Argument::None),       // FROZENSET
    (0x30, Argument::None),       // POP
    (0x32, Argument::None),       // DUP
    (0x28, Argument::None),       // MARK
    (0x31, Argument::None),       // POP_MARK
    (0x67, Argument::Line),       // GET
    (0x68, Argument::Fixed(1)),   // BINGET
    (0x6a, Argument::Fixed(4)),   // LONG_BINGET
    (0x70, Argument::Line),       // PUT
    (0x71, Argument::Fixed(1)),   // BINPUT
    (0x72, Argument::Fixed(4)),   // LONG_BINPUT
    (0x94, Argument::None),       // MEMOIZE
    (0x82, Argument::Fixed(1)),   // EXT1
    (0x83, Argument::Fixed(2)),   // EXT2
    (0x84, Argument::Fixed(4)),   // EXT4
    (0x63, Argument::Line),       // GLOBAL
    (0x93, Argument::None),       // STACK_GLOBAL
    (0x52, Argument::None),       // REDUCE
    (0x62, Argument::None),       // BUILD
    (0x69, Argument::Line),       // INST
    (0x6f, Argument::None),       // OBJ
    (0x81, Argument::None),       // NEWOBJ
    (0x92, Argument::None),       // NEWOBJ_EX
    (0x80, Argument::Fixed(1)),   // PROTO
    (0x2e, Argument::None),       // STOP
    (0x95, Argument::Fixed(8)),   // FRAME
    (0x50, Argument::Line),       // PERSID
    (0x51, Argument::None),       // BINPERSID
];

/// What follows `opcode`, or `None` for one this table has not got.
fn argument_of(opcode: u8) -> Option<Argument> {
    OPCODES
        .iter()
        .find(|(number, _)| *number == opcode)
        .map(|(_, argument)| *argument)
}

/// Everything [`PickleView`] holds, read from `bytes`.
///
/// Reads. Never runs.
fn parse(bytes: &[u8]) -> Option<PickleView> {
    let mut view = PickleView {
        protocol: 0,
        opcodes: 0,
        builds: "unread".to_owned(),
        imports: Vec::new(),
        executing_opcodes: Vec::new(),
        complete: false,
    };
    let mut reader = Reader { bytes, at: 0 };
    // The last two short strings seen, because STACK_GLOBAL takes its
    // module and attribute off the stack rather than from its own bytes.
    let mut recent: Vec<String> = Vec::new();
    let mut first_container = None;
    let mut constructs_object = false;

    while let Some(opcode) = reader.byte() {
        view.opcodes += 1;
        if opcode == STOP {
            view.complete = true;
            break;
        }
        if let Some((_, what)) = IMPORTING.iter().find(|(number, _)| *number == opcode)
            && opcode != 0x95
        {
            let said = (*what).to_owned();
            if !view.executing_opcodes.contains(&said) {
                view.executing_opcodes.push(said);
            }
        }
        // The outermost container is created first, so the first of these
        // is the top-level one - the last is whatever was nested deepest.
        // An opcode that calls something beats both: a stream that builds
        // an object builds an object, whatever containers it fills on the
        // way.
        if CONSTRUCTING.contains(&opcode) {
            constructs_object = true;
        } else if first_container.is_none()
            && let Some((_, what)) = BUILDS.iter().find(|(number, _)| *number == opcode)
        {
            first_container = Some(*what);
        }

        // Read the argument, so the next byte really is the next opcode.
        if opcode == PROTO {
            view.protocol = reader.byte()?;
            continue;
        }
        if opcode == b'c' || opcode == b'i' {
            // GLOBAL and INST take two lines: a module then a name.
            let module = reader.line()?;
            let attribute = reader.line()?;
            view.imports.push(format!("{module}.{attribute}"));
            continue;
        }
        if opcode == 0x93 {
            // STACK_GLOBAL takes its module and name off the stack, so
            // they are the last two strings the walk saw.
            let attribute = recent.pop().unwrap_or_default();
            let module = recent.pop().unwrap_or_default();
            view.imports.push(format!("{module}.{attribute}"));
            continue;
        }
        match argument_of(opcode)? {
            Argument::None => {}
            Argument::Fixed(width) => {
                reader.take(width)?;
            }
            Argument::Counted(width) => {
                let length = reader.number(width)?;
                let slice = reader.take(length)?;
                remember(&mut recent, String::from_utf8_lossy(slice).into_owned());
            }
            Argument::Line => {
                let said = reader.line()?;
                remember(&mut recent, said);
            }
        }
    }

    if constructs_object {
        "an object, built by calling something imported".clone_into(&mut view.builds);
    } else if let Some(what) = first_container {
        what.clone_into(&mut view.builds);
    }
    // A stream with no opcodes at all is not a pickle.
    (view.opcodes > 1).then_some(view)
}

/// Keeps the last two short strings, which is all `STACK_GLOBAL` needs.
fn remember(recent: &mut Vec<String>, said: String) {
    recent.push(said);
    if recent.len() > 4 {
        recent.remove(0);
    }
}

/// Whether `bytes` are a pickle.
fn looks_like_it(bytes: &[u8]) -> bool {
    // Protocol 2 and later open with PROTO and a version byte; that is
    // decisive. Protocol 0 and 1 have no header at all, so they are only
    // claimed when the whole prefix reads as opcodes and ends properly.
    if bytes.first() == Some(&PROTO) && bytes.get(1).is_some_and(|version| *version <= 5) {
        return true;
    }
    parse(bytes).is_some_and(|view| view.complete && view.opcodes > 2)
}

/// The Python pickle plugin's core half.
#[derive(Debug, Default)]
pub struct PickleCore;

impl PluginCore for PickleCore {
    fn name(&self) -> &'static str {
        "pickle"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_it(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let view = parse(&bytes).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "not a readable pickle stream")
        })?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Python pickle plugin's presentation half.
#[derive(Debug, Default)]
pub struct PicklePresentation;

impl PluginPresentation for PicklePresentation {
    fn name(&self) -> &'static str {
        "pickle"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "PKL",
            tint: 0x00ff_d43b,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: PickleView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![
            format!(
                "Pickle: protocol {}, {} opcode(s), builds {}",
                view.protocol, view.opcodes, view.builds
            ),
            "Read as opcodes and never executed: loading a pickle runs it.".to_owned(),
        ];
        if !view.complete {
            lines.push("No stop opcode, so this stream is cut off.".to_owned());
        }
        if view.imports.is_empty() {
            lines.push("Imports nothing: it builds only the types built in.".to_owned());
        } else {
            lines.push("Loading this would import:".to_owned());
            for said in &view.imports {
                lines.push(format!("  {said}"));
            }
        }
        if !view.executing_opcodes.is_empty() {
            lines.push("The opcodes that make that happen:".to_owned());
            for said in &view.executing_opcodes {
                lines.push(format!("  {said}"));
            }
            lines.push("Whatever those name will be imported and called by".to_owned());
            lines.push("`pickle.load`. Do not load one from anywhere you would".to_owned());
            lines.push("not run a script from.".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{PickleCore, PicklePresentation, PickleView, looks_like_it, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    fn sample(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/pickle")
            .join(name)
    }

    fn view_of(name: &str) -> PickleView {
        serde_json::from_value(PickleCore.view(&sample(name)).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&PickleCore),
            PluginPresentation::extensions(&PicklePresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn sniffs_a_protocol_header() {
        assert!(looks_like_it(&[0x80, 0x05, b'.']));
        assert!(
            !looks_like_it(&[0x80, 0x63]),
            "ninety-nine is not a protocol anyone has written"
        );
    }

    #[test]
    fn does_not_claim_anything_else() {
        assert!(!looks_like_it(b"#!/bin/sh\necho hello\n"));
        assert!(!looks_like_it(b""));
        assert!(!looks_like_it(&[0x1f, 0x8b, 0x08]));
    }

    #[test]
    fn reads_the_protocol_of_each_fixture() {
        assert_eq!(view_of("mapping-protocol-2.pickle").protocol, 2);
        assert_eq!(view_of("sequence-protocol-4.pickle").protocol, 4);
        assert_eq!(view_of("instance-protocol-5.pickle").protocol, 5);
    }

    #[test]
    fn a_mapping_of_built_in_types_imports_nothing() {
        let view = view_of("mapping-protocol-2.pickle");

        assert_eq!(view.builds, "a dictionary");
        assert!(
            view.imports.is_empty(),
            "strings and floats need no import: {:?}",
            view.imports
        );
        assert!(view.complete);
    }

    #[test]
    fn a_sequence_builds_a_list() {
        let view = view_of("sequence-protocol-4.pickle");

        assert!(view.builds.contains("list") || view.builds.contains("tuple"));
        assert!(view.imports.is_empty());
    }

    #[test]
    fn an_instance_names_the_class_it_would_import() {
        let view = view_of("instance-protocol-5.pickle");

        assert_eq!(view.imports.len(), 1);
        assert!(
            view.imports[0].ends_with(".Station"),
            "expected the class, got {:?}",
            view.imports
        );
        assert!(!view.executing_opcodes.is_empty());
        assert!(view.builds.contains("object"));
    }

    #[test]
    fn a_stream_with_no_stop_is_reported_as_cut_off() {
        let view = parse(&[0x80, 0x05, b'}', b'q', 0x00]).unwrap();

        assert!(!view.complete);
        let data = serde_json::to_value(&view).unwrap();
        let lines = PicklePresentation.present(&data);
        assert!(lines.iter().any(|line| line.contains("cut off")));
    }

    #[test]
    fn says_plainly_that_nothing_is_executed() {
        let data = PickleCore
            .view(&sample("instance-protocol-5.pickle"))
            .unwrap();

        let lines = PicklePresentation.present(&data);

        assert!(lines.iter().any(|line| line.contains("never executed")));
        assert!(lines.iter().any(|line| line.contains("would import")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("not run a script from")),
            "the warning has to say what the risk actually is"
        );
    }

    #[test]
    fn a_file_that_is_not_a_pickle_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.pkl");
        std::fs::write(&path, b"\x80").unwrap();

        assert!(PickleCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
