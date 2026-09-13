//! Python bytecode file type plugin: core and presentation halves.
//!
//! A `.pyc` is a sixteen-byte header and then one marshalled code
//! object. The header is the interesting half: four magic bytes naming
//! the `CPython` release that wrote it, then either the source's
//! modification time and length, or a hash of the source - and the same
//! four bytes at the same offset mean different things depending on
//! which. Telling those apart is most of reading one.
//!
//! The code object after it is read far enough to say what the module
//! defines: its name, what it references, and the functions inside it.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
///
/// `pyo` was optimised bytecode until Python 3.5 stopped writing it; a
/// file with that name is still one of these.
pub const EXTENSIONS: &[&str] = &["pyc", "pyo"];

/// The two bytes every magic number ends with.
///
/// `CPython` chose a carriage return and a line feed on purpose: a `.pyc`
/// transferred in text mode by an FTP client would have them mangled,
/// and the file would be rejected rather than half-read.
const MAGIC_TAIL: &[u8] = &[0x0d, 0x0a];

/// The header, before the marshalled code object.
const HEADER: usize = 16;

/// The first magic number of each `CPython` release series.
///
/// `CPython` bumps the magic within a series whenever the bytecode
/// changes, so a file's number is looked up as "the newest series whose
/// first number it is at least". That reads a release candidate
/// correctly instead of calling it unknown.
const SERIES: &[(u16, &str)] = &[
    (3111, "3.0"),
    (3151, "3.1"),
    (3160, "3.2"),
    (3190, "3.3"),
    (3250, "3.4"),
    (3320, "3.5"),
    (3360, "3.6"),
    (3390, "3.7"),
    (3400, "3.8"),
    (3420, "3.9"),
    (3430, "3.10"),
    (3450, "3.11"),
    (3500, "3.12"),
    (3550, "3.13"),
    (3600, "3.14"),
];

/// One code object, as much of it as a reader wants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeObject {
    /// Its name: `<module>` for the file itself, otherwise a function
    /// or class name.
    pub name: String,
    /// How many positional arguments it takes.
    pub argument_count: usize,
    /// The line it starts on.
    pub first_line: usize,
    /// The names it references - globals, attributes, builtins. This is
    /// what the code reaches for outside itself.
    pub names: Vec<String>,
    /// Its local variables and arguments.
    pub locals: Vec<String>,
    /// Its constants, rendered.
    pub constants: Vec<String>,
    /// The code objects nested inside it, by name.
    pub nested: Vec<String>,
}

/// View data produced by [`PycCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PycView {
    /// The magic number, as `CPython` counts it.
    pub magic: u16,
    /// The release series that magic belongs to.
    pub python_version: String,
    /// The header's flag word.
    pub flags: u32,
    /// How the interpreter decides this file is out of date.
    pub invalidation: String,
    /// The source's modification time, for a timestamp-based file.
    pub source_mtime: Option<u32>,
    /// The source's length in bytes, likewise.
    pub source_size: Option<u32>,
    /// The hash of the source, for a hash-based file.
    pub source_hash: Option<String>,
    /// The source file the code object names.
    pub source_file: Option<String>,
    /// The module's own code object.
    pub module: CodeObject,
    /// Every code object nested in it, in order.
    pub functions: Vec<CodeObject>,
}

/// Whether `prefix` opens like Python bytecode.
///
/// The last two magic bytes are fixed, the first two name the release,
/// and the whole header has to be there. A file whose magic is not one
/// this knows is still recognised - the shape is unmistakable - and the
/// release is reported as unknown rather than guessed.
fn looks_like_it(prefix: &[u8]) -> bool {
    prefix.len() >= HEADER
        && prefix[2..4] == *MAGIC_TAIL
        && u16::from_le_bytes([prefix[0], prefix[1]]) >= SERIES[0].0
}

/// The release series a magic number belongs to.
fn version_of(magic: u16) -> String {
    SERIES
        .iter()
        .rev()
        .find(|(first, _)| magic >= *first)
        .map_or_else(
            || format!("an unrecognised release (magic {magic})"),
            |(_, name)| (*name).to_owned(),
        )
}

/// A marshalled value, in the shapes a code object is made of.
#[derive(Debug, Clone)]
enum Marshalled {
    /// A string, whatever flavour marshal wrote it in.
    Text(String),
    /// A number.
    Number(i64),
    /// A float.
    Real(f64),
    /// `None`, `True`, `False` and the rest of the singletons.
    Word(&'static str),
    /// A tuple, list, set or frozenset.
    Sequence(Vec<Marshalled>),
    /// A code object.
    Code(Box<Code>),
    /// Anything read but not modelled.
    Opaque,
}

/// A code object as marshal writes it.
#[derive(Debug, Clone, Default)]
struct Code {
    /// Positional argument count.
    argument_count: i32,
    /// Where in the source it starts.
    first_line: i32,
    /// Its constants.
    constants: Vec<Marshalled>,
    /// The names it references.
    names: Vec<Marshalled>,
    /// Its locals and arguments.
    locals: Vec<Marshalled>,
    /// The file it was compiled from.
    filename: String,
    /// Its own name.
    name: String,
}

/// The flag saying a value is also entered in the reference table.
const FLAG_REF: u8 = 0x80;

/// A cursor over a marshalled stream.
struct Marshal<'bytes> {
    /// The bytes after the header.
    bytes: &'bytes [u8],
    /// How far in we are.
    at: usize,
    /// Every value entered in the reference table, in order.
    ///
    /// Marshal writes a repeated string once and refers back to it
    /// afterwards, and it does so in stream order rather than source
    /// order: a module's own filename is written *inside* the first
    /// function it defines, because the constants come before the
    /// filename in a code object. Without this table the module's
    /// filename reads as nothing at all.
    refs: Vec<Marshalled>,
}

impl Marshal<'_> {
    /// One byte.
    fn byte(&mut self) -> Option<u8> {
        let value = *self.bytes.get(self.at)?;
        self.at += 1;
        Some(value)
    }

    /// Four bytes, little-endian and signed, the way marshal writes a
    /// length or a small integer.
    fn i32(&mut self) -> Option<i32> {
        let run = self.bytes.get(self.at..self.at + 4)?;
        self.at += 4;
        Some(i32::from_le_bytes(run.try_into().ok()?))
    }

    /// A run of `count` bytes.
    fn take(&mut self, count: usize) -> Option<&[u8]> {
        let run = self.bytes.get(self.at..self.at + count)?;
        self.at += count;
        Some(run)
    }

    /// A run of `count` bytes as text.
    fn text(&mut self, count: usize) -> Option<String> {
        Some(String::from_utf8_lossy(self.take(count)?).into_owned())
    }

    /// The next value.
    fn value(&mut self) -> Option<Marshalled> {
        let tag = self.byte()?;
        // The slot is claimed before the value is read, because a value
        // may refer to one written earlier inside itself.
        let slot = (tag & FLAG_REF != 0).then(|| {
            self.refs.push(Marshalled::Opaque);
            self.refs.len() - 1
        });
        let value = self.body(tag & !FLAG_REF);
        if let (Some(slot), Some(value)) = (slot, value.as_ref()) {
            self.refs[slot] = value.clone();
        }
        value
    }

    /// The value a tag introduces, the reference flag already taken off.
    fn body(&mut self, tag: u8) -> Option<Marshalled> {
        match tag {
            b'0' | b'N' => Some(Marshalled::Word("None")),
            b'F' => Some(Marshalled::Word("False")),
            b'T' => Some(Marshalled::Word("True")),
            b'.' => Some(Marshalled::Word("...")),
            b'S' => Some(Marshalled::Word("StopIteration")),
            b'i' => Some(Marshalled::Number(i64::from(self.i32()?))),
            b'l' => self.long(),
            b'g' => {
                let run = self.take(8)?;
                Some(Marshalled::Real(f64::from_le_bytes(run.try_into().ok()?)))
            }
            b'y' => {
                self.take(16)?;
                Some(Marshalled::Opaque)
            }
            // A bytes object; its length is written as four bytes.
            b's' => {
                let length = self.i32()?;
                self.take(usize::try_from(length).ok()?)?;
                Some(Marshalled::Opaque)
            }
            b'u' | b't' | b'a' | b'A' => {
                let length = self.i32()?;
                Some(Marshalled::Text(self.text(usize::try_from(length).ok()?)?))
            }
            // Short ASCII, whose length fits in one byte. Most names are
            // written this way.
            b'z' | b'Z' => {
                let length = self.byte()? as usize;
                Some(Marshalled::Text(self.text(length)?))
            }
            // A back-reference to a value already written.
            b'r' => {
                let index = self.i32()?;
                self.refs.get(usize::try_from(index).ok()?).cloned()
            }
            b'(' | b'[' | b'<' | b'>' => {
                let length = self.i32()?;
                self.sequence(usize::try_from(length).ok()?)
            }
            b')' => {
                let length = self.byte()? as usize;
                self.sequence(length)
            }
            b'c' => self.code(),
            _ => None,
        }
    }

    /// A variable-length integer, which marshal writes as fifteen-bit
    /// digits with the count's sign carrying the number's.
    fn long(&mut self) -> Option<Marshalled> {
        let size = self.i32()?;
        let digits = usize::try_from(size.abs()).ok()?;
        let mut value: i64 = 0;
        for index in 0..digits {
            let run = self.take(2)?;
            let digit = i64::from(u16::from_le_bytes([run[0], run[1]]));
            // Beyond four digits the number no longer fits, and a
            // constant that large is not what a reader is looking at.
            if index < 4 {
                value |= digit << (15 * index);
            }
        }
        Some(Marshalled::Number(if size < 0 { -value } else { value }))
    }

    /// `count` values in a row.
    fn sequence(&mut self, count: usize) -> Option<Marshalled> {
        let mut items = Vec::with_capacity(count.min(64));
        for _ in 0..count {
            items.push(self.value()?);
        }
        Some(Marshalled::Sequence(items))
    }

    /// A code object.
    ///
    /// The field order has been the same since Python 3.11, when
    /// `nlocals` went and `qualname` and the exception table arrived,
    /// and both fixtures - one from 3.11, one from 3.14 - are read to
    /// the last byte by it.
    fn code(&mut self) -> Option<Marshalled> {
        let mut code = Code {
            argument_count: self.i32()?,
            ..Code::default()
        };
        self.i32()?; // positional-only arguments
        self.i32()?; // keyword-only arguments
        self.i32()?; // stack size
        self.i32()?; // flags
        self.value()?; // the bytecode itself
        code.constants = as_items(self.value()?);
        code.names = as_items(self.value()?);
        code.locals = as_items(self.value()?);
        self.value()?; // what kind each of those locals is
        code.filename = as_text(&self.value()?);
        code.name = as_text(&self.value()?);
        self.value()?; // qualified name
        code.first_line = self.i32()?;
        self.value()?; // the line table
        self.value()?; // the exception table
        Some(Marshalled::Code(Box::new(code)))
    }
}

/// A sequence's items, or nothing when the value is not one.
fn as_items(value: Marshalled) -> Vec<Marshalled> {
    match value {
        Marshalled::Sequence(items) => items,
        _ => Vec::new(),
    }
}

/// A value's text, or empty when it is not text.
fn as_text(value: &Marshalled) -> String {
    match value {
        Marshalled::Text(text) => text.clone(),
        _ => String::new(),
    }
}

/// How a value reads in the pane.
fn render(value: &Marshalled) -> String {
    match value {
        Marshalled::Text(text) if text.len() > 48 => format!("'{}...'", &text[..45]),
        Marshalled::Text(text) => format!("'{text}'"),
        Marshalled::Number(number) => number.to_string(),
        // Debug formatting, so a whole-numbered float still reads as
        // one: `0.0` rather than a `0` nothing tells from the integer
        // constant beside it.
        Marshalled::Real(real) => format!("{real:?}"),
        Marshalled::Word(word) => (*word).to_owned(),
        Marshalled::Sequence(items) => format!(
            "({})",
            items.iter().map(render).collect::<Vec<String>>().join(", ")
        ),
        Marshalled::Code(code) => format!("<code {}>", code.name),
        Marshalled::Opaque => "...".to_owned(),
    }
}

/// The names in a sequence of marshalled strings.
fn texts(values: &[Marshalled]) -> Vec<String> {
    values
        .iter()
        .filter_map(|value| match value {
            Marshalled::Text(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// A [`CodeObject`] from a parsed code.
fn described(code: &Code) -> CodeObject {
    CodeObject {
        name: code.name.clone(),
        argument_count: usize::try_from(code.argument_count).unwrap_or(0),
        first_line: usize::try_from(code.first_line).unwrap_or(0),
        names: texts(&code.names),
        locals: texts(&code.locals),
        constants: code.constants.iter().map(render).collect(),
        nested: code
            .constants
            .iter()
            .filter_map(|value| match value {
                Marshalled::Code(inner) => Some(inner.name.clone()),
                _ => None,
            })
            .collect(),
    }
}

/// Everything [`PycView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<PycView> {
    let bytes = std::fs::read(path)?;
    let malformed = || io::Error::new(io::ErrorKind::InvalidData, "not readable Python bytecode");
    if !looks_like_it(&bytes) {
        return Err(malformed());
    }
    let magic = u16::from_le_bytes([bytes[0], bytes[1]]);
    let flags = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let hash_based = flags & 0b1 != 0;

    let mut marshal = Marshal {
        bytes: &bytes[HEADER..],
        at: 0,
        refs: Vec::new(),
    };
    let Some(Marshalled::Code(module)) = marshal.value() else {
        return Err(malformed());
    };

    let functions = module
        .constants
        .iter()
        .filter_map(|value| match value {
            Marshalled::Code(inner) => Some(described(inner)),
            _ => None,
        })
        .collect();

    Ok(PycView {
        magic,
        python_version: version_of(magic),
        flags,
        invalidation: if hash_based {
            if flags & 0b10 == 0 {
                "hash-based, unchecked: the interpreter takes it on trust".to_owned()
            } else {
                "hash-based, checked: the source is hashed and compared".to_owned()
            }
        } else {
            "timestamp-based: the source's time and length are compared".to_owned()
        },
        source_mtime: (!hash_based)
            .then(|| u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]])),
        source_size: (!hash_based)
            .then(|| u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]])),
        source_hash: hash_based.then(|| {
            bytes[8..16].iter().fold(String::new(), |mut out, byte| {
                use std::fmt::Write as _;
                let _ = write!(out, "{byte:02x}");
                out
            })
        }),
        source_file: (!module.filename.is_empty()).then(|| module.filename.clone()),
        module: described(&module),
        functions,
    })
}

/// The Python bytecode plugin's core half.
#[derive(Debug, Default)]
pub struct PycCore;

impl PluginCore for PycCore {
    fn name(&self) -> &'static str {
        "pyc"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_it(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Python bytecode plugin's presentation half.
#[derive(Debug, Default)]
pub struct PycPresentation;

impl PluginPresentation for PycPresentation {
    fn name(&self) -> &'static str {
        "pyc"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "PYC",
            tint: 0x00ff_d43b,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: PycView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "Python {} bytecode, magic {}",
            view.python_version, view.magic
        )];
        lines.push(view.invalidation.clone());
        match (view.source_mtime, view.source_size) {
            (Some(mtime), Some(size)) => {
                lines.push(format!("Source last written at {mtime}, {size} byte(s)"));
            }
            _ => {
                if let Some(hash) = &view.source_hash {
                    lines.push(format!("Source hash {hash}"));
                }
            }
        }
        if let Some(source) = &view.source_file {
            lines.push(format!("Compiled from {source}"));
        }
        if !view.module.names.is_empty() {
            lines.push(format!(
                "Module references {}",
                view.module.names.join(", ")
            ));
        }
        if !view.module.constants.is_empty() {
            lines.push("Module constants:".to_owned());
            for constant in &view.module.constants {
                lines.push(format!("  {constant}"));
            }
        }
        if view.functions.is_empty() {
            lines.push("Defines no functions.".to_owned());
        } else {
            lines.push("Defines:".to_owned());
            for function in &view.functions {
                lines.push(format!(
                    "  {}({} argument(s)) at line {}",
                    function.name, function.argument_count, function.first_line
                ));
                if !function.locals.is_empty() {
                    lines.push(format!("    locals: {}", function.locals.join(", ")));
                }
                if !function.names.is_empty() {
                    lines.push(format!("    calls: {}", function.names.join(", ")));
                }
                for nested in &function.nested {
                    lines.push(format!("    nested: {nested}"));
                }
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{PycCore, PycPresentation, PycView, looks_like_it, version_of};
    use plugin_api::{PluginCore, PluginPresentation};

    fn sample(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/pyc")
            .join(name)
    }

    fn view_of(name: &str) -> PycView {
        serde_json::from_value(PycCore.view(&sample(name)).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&PycCore),
            PluginPresentation::extensions(&PycPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn sniffs_the_magic_and_a_whole_header() {
        assert!(looks_like_it(&[
            0xa7, 0x0d, 0x0d, 0x0a, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ]));
        assert!(
            !looks_like_it(&[0xa7, 0x0d, 0x0d, 0x0a, 0, 0, 0, 0]),
            "half a header is not one"
        );
        assert!(
            !looks_like_it(&[0xa7, 0x0d, 0x0a, 0x0d, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            "the carriage return comes before the line feed"
        );
        assert!(!looks_like_it(b""));
    }

    #[test]
    fn a_magic_number_names_the_release_series_it_falls_in() {
        assert_eq!(version_of(3495), "3.11", "a mid-series 3.11 magic");
        assert_eq!(version_of(3450), "3.11", "the first 3.11 magic");
        assert_eq!(
            version_of(3449),
            "3.10",
            "one below it is the series before"
        );
        assert_eq!(version_of(3627), "3.14");
        assert_eq!(version_of(3413), "3.8");
    }

    #[test]
    fn reads_a_timestamp_based_header() {
        let view = view_of("column.cpython-311.pyc");

        assert_eq!(view.python_version, "3.11");
        assert_eq!(view.flags, 0);
        assert!(view.invalidation.starts_with("timestamp-based"));
        assert!(view.source_mtime.is_some());
        assert_eq!(
            view.source_size,
            Some(305),
            "the header records how long the source was"
        );
        assert_eq!(view.source_hash, None);
    }

    #[test]
    fn reads_a_hash_based_header() {
        let view = view_of("column.cpython-314.pyc");

        assert_eq!(view.python_version, "3.14");
        assert_eq!(view.flags, 3, "hash-based and checked");
        assert!(view.invalidation.contains("checked"));
        assert_eq!(view.source_mtime, None);
        assert_eq!(view.source_size, None);
        assert_eq!(
            view.source_hash.as_deref().map(str::len),
            Some(16),
            "eight bytes of hash, written out"
        );
    }

    #[test]
    fn reads_the_module_the_same_way_from_either_release() {
        for name in ["column.cpython-311.pyc", "column.cpython-314.pyc"] {
            let view = view_of(name);

            assert_eq!(view.module.name, "<module>", "{name}");
            assert_eq!(view.source_file.as_deref(), Some("column.py"), "{name}");
            assert_eq!(
                view.module.names,
                vec!["__doc__", "LIMIT", "mean"],
                "{name}"
            );
            assert!(
                view.module.constants.iter().any(|one| one == "1024"),
                "{name}: the module-level constant"
            );
            assert!(
                !view.module.constants.iter().any(|one| one == "0"),
                "{name}: a float constant has to read as a float, or nothing                  in the pane tells it from the integer beside it"
            );
        }
    }

    #[test]
    fn a_back_reference_resolves_to_the_value_it_points_at() {
        // The module's filename is written once, inside the first
        // function's code object, because a code object's constants come
        // before its filename. The module's own filename is therefore a
        // back-reference, and reading it is the only way this is not
        // empty.
        let view = view_of("column.cpython-311.pyc");

        assert_eq!(view.source_file.as_deref(), Some("column.py"));
        assert_eq!(view.functions[0].name, "mean");
    }

    #[test]
    fn reads_the_function_inside_it() {
        let view = view_of("column.cpython-311.pyc");

        assert_eq!(view.functions.len(), 1);
        let mean = &view.functions[0];
        assert_eq!(mean.name, "mean");
        assert_eq!(mean.argument_count, 2, "one of them has a default");
        assert_eq!(mean.first_line, 6);
        assert_eq!(mean.names, vec!["float", "len"]);
        assert_eq!(
            mean.locals,
            vec!["readings", "default", "total", "reading"],
            "arguments first, then the locals"
        );
        assert!(mean.nested.is_empty());
        assert!(
            view.module.nested.iter().any(|one| one == "mean"),
            "the module holds the function's code as a constant"
        );
    }

    #[test]
    fn presents_the_two_kinds_of_header_differently() {
        let stamped =
            PycPresentation.present(&PycCore.view(&sample("column.cpython-311.pyc")).unwrap());
        let hashed =
            PycPresentation.present(&PycCore.view(&sample("column.cpython-314.pyc")).unwrap());

        assert!(stamped[0].starts_with("Python 3.11 bytecode"));
        assert!(
            stamped
                .iter()
                .any(|line| line.contains("Source last written at"))
        );
        assert!(!stamped.iter().any(|line| line.contains("Source hash")));

        assert!(hashed[0].starts_with("Python 3.14 bytecode"));
        assert!(hashed.iter().any(|line| line.contains("Source hash")));
        assert!(
            !hashed
                .iter()
                .any(|line| line.contains("Source last written"))
        );

        for lines in [&stamped, &hashed] {
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("mean(2 argument(s))"))
            );
            assert!(lines.iter().any(|line| line.contains("calls: float, len")));
        }
    }

    #[test]
    fn a_file_that_is_not_bytecode_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really-a.pyc");
        std::fs::write(&path, b"\xa7\x0d\x0d\x0a and then nothing of the sort").unwrap();

        assert!(PycCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
