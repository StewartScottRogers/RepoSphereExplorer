//! Java class file file type plugin: core and presentation halves.
//!
//! A class file is a constant pool with a small amount of structure
//! hung off it: the class name, its superclass, its interfaces, its
//! fields and methods are all indices into that pool. So reading one
//! means reading the pool first and everything else second.
//!
//! Types are written as descriptors - `(Ljava/util/List;)D` - which say
//! exactly what the runtime needs and nothing a person wants. Both are
//! reported: the descriptor because it is what is in the file, and the
//! reading of it because that is what the pane is for.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["class"];

/// The four bytes every class file opens with.
const MAGIC: &[u8] = &[0xca, 0xfe, 0xba, 0xbe];

/// The lowest major version any Java release ever wrote. Below this the
/// four magic bytes are somebody else's.
const OLDEST_MAJOR: u16 = 45;

/// The highest major version this expects to meet. A file past it is
/// read anyway - the format has been stable - but the Java release is
/// reported as unknown rather than guessed.
const NEWEST_MAJOR: u16 = 80;

/// One field or method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// Its name.
    pub name: String,
    /// Its type, as the class file writes it.
    pub descriptor: String,
    /// The same type, read out.
    pub reads_as: String,
    /// The access flags it carries, spelled out.
    pub flags: Vec<String>,
    /// Its generic signature, when the compiler kept one. A descriptor
    /// loses type parameters; this is where they survive.
    pub signature: Option<String>,
}

/// View data produced by [`JavaclassCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JavaclassView {
    /// The class file version, as major and minor.
    pub version: String,
    /// The Java release that version implies.
    pub java_release: String,
    /// The class this file defines, in source notation.
    pub class_name: String,
    /// What it extends.
    pub superclass: Option<String>,
    /// What it implements.
    pub interfaces: Vec<String>,
    /// The class's own access flags, spelled out.
    pub flags: Vec<String>,
    /// Its fields.
    pub fields: Vec<Member>,
    /// Its methods.
    pub methods: Vec<Member>,
    /// How many entries the constant pool holds.
    pub constant_pool_size: usize,
    /// The source file it was compiled from, when the compiler kept it.
    pub source_file: Option<String>,
    /// The annotations on the class itself that survive to runtime.
    pub annotations: Vec<String>,
    /// The nested classes it names, in source notation.
    pub nested_classes: Vec<String>,
}

/// Whether `prefix` opens like a class file.
///
/// The magic alone is not enough: a Mach-O fat binary opens with the
/// same four bytes, and what follows there is an architecture count
/// rather than a version pair. A plausible major version is the
/// difference.
fn looks_like_it(prefix: &[u8]) -> bool {
    prefix.starts_with(MAGIC)
        && prefix.len() >= 8
        && (OLDEST_MAJOR..=NEWEST_MAJOR).contains(&u16::from_be_bytes([prefix[6], prefix[7]]))
}

/// The Java release a major version implies.
///
/// The two ran level from Java 1.0 to 1.4; from Java 5 the major
/// version is the release plus forty-four.
fn release_of(major: u16) -> String {
    match major {
        45 => "Java 1.1 or earlier".to_owned(),
        46..=48 => format!("Java 1.{}", major - 44),
        49..=NEWEST_MAJOR => format!("Java {}", major - 44),
        _ => format!("an unrecognised release (major {major})"),
    }
}

/// One constant pool entry, in the only shapes this reads.
#[derive(Debug, Clone)]
enum Constant {
    /// A string, which is what every name and descriptor ultimately is.
    Utf8(String),
    /// A class, naming the entry holding its name.
    Class(u16),
    /// Anything else. Its bytes are stepped over, not kept: nothing this
    /// reads needs a field reference or a method handle.
    Other,
}

/// A cursor over a class file, reading big-endian.
struct Reader<'bytes> {
    /// The whole file.
    bytes: &'bytes [u8],
    /// How far in we are.
    at: usize,
}

impl<'bytes> Reader<'bytes> {
    /// One byte.
    fn u8(&mut self) -> Option<u8> {
        let value = *self.bytes.get(self.at)?;
        self.at += 1;
        Some(value)
    }

    /// Two bytes.
    fn u16(&mut self) -> Option<u16> {
        let value = u16::from_be_bytes([*self.bytes.get(self.at)?, *self.bytes.get(self.at + 1)?]);
        self.at += 2;
        Some(value)
    }

    /// Four bytes.
    fn u32(&mut self) -> Option<u32> {
        let value = u32::from_be_bytes(self.bytes.get(self.at..self.at + 4)?.try_into().ok()?);
        self.at += 4;
        Some(value)
    }

    /// `count` bytes, stepped over.
    fn skip(&mut self, count: usize) -> Option<()> {
        self.at = self
            .at
            .checked_add(count)
            .filter(|at| *at <= self.bytes.len())?;
        Some(())
    }

    /// A run of `count` bytes.
    ///
    /// The run borrows the file rather than the cursor, so an attribute
    /// body outlives the read that found it.
    fn take(&mut self, count: usize) -> Option<&'bytes [u8]> {
        let run = self.bytes.get(self.at..self.at + count)?;
        self.at += count;
        Some(run)
    }
}

/// How many bytes a constant pool entry of each tag carries after it.
///
/// Long and Double take two pool slots each, a decision the
/// specification calls a mistake and keeps for compatibility, so the
/// count is returned alongside.
fn constant_at(reader: &mut Reader) -> Option<(Constant, u16)> {
    let tag = reader.u8()?;
    Some(match tag {
        1 => {
            let length = reader.u16()? as usize;
            let run = reader.take(length)?;
            (Constant::Utf8(String::from_utf8_lossy(run).into_owned()), 1)
        }
        7 | 8 | 16 | 19 | 20 => (
            if tag == 7 {
                Constant::Class(reader.u16()?)
            } else {
                reader.skip(2)?;
                Constant::Other
            },
            1,
        ),
        15 => {
            reader.skip(3)?;
            (Constant::Other, 1)
        }
        3 | 4 | 9 | 10 | 11 | 12 | 17 | 18 => {
            reader.skip(4)?;
            (Constant::Other, 1)
        }
        5 | 6 => {
            reader.skip(8)?;
            (Constant::Other, 2)
        }
        _ => return None,
    })
}

/// The pool's strings, by index, with everything else left empty.
struct Pool {
    /// One slot per pool index, index zero unused as the format leaves it.
    slots: Vec<Constant>,
}

impl Pool {
    /// The string at `index`, if that slot holds one.
    fn text(&self, index: u16) -> Option<&str> {
        match self.slots.get(index as usize)? {
            Constant::Utf8(text) => Some(text),
            _ => None,
        }
    }

    /// The class name at `index`, in source notation.
    fn class(&self, index: u16) -> Option<String> {
        match self.slots.get(index as usize)? {
            Constant::Class(name) => self.text(*name).map(|name| name.replace('/', ".")),
            _ => None,
        }
    }
}

/// What is carrying a set of access flags.
///
/// Four of the bits mean different things depending on the answer -
/// `0x0020` is `synchronized` on a method and `ACC_SUPER` on a class,
/// `0x0040` is `volatile` on a field and `bridge` on a method - so
/// spelling them out needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Carrier {
    /// The class itself.
    Class,
    /// One of its fields.
    Field,
    /// One of its methods.
    Method,
}

/// The flags something carries, spelled out.
///
/// `ACC_SUPER` is deliberately absent: it is set on every class
/// compiled this century and means nothing to anybody now.
fn flags_of(bits: u16, carrier: Carrier) -> Vec<String> {
    /// The bits that mean the same thing wherever they appear.
    const SHARED: &[(u16, &str)] = &[
        (0x0001, "public"),
        (0x0002, "private"),
        (0x0004, "protected"),
        (0x0008, "static"),
        (0x0010, "final"),
        (0x1000, "synthetic"),
    ];
    let mut said: Vec<String> = SHARED
        .iter()
        .filter(|(bit, _)| bits & bit != 0)
        .map(|(_, name)| (*name).to_owned())
        .collect();
    let particular: &[(u16, &str)] = match carrier {
        Carrier::Class => &[
            (0x0200, "interface"),
            (0x0400, "abstract"),
            (0x2000, "annotation"),
            (0x4000, "enum"),
        ],
        Carrier::Field => &[
            (0x0040, "volatile"),
            (0x0080, "transient"),
            (0x4000, "enum"),
        ],
        Carrier::Method => &[
            (0x0020, "synchronized"),
            (0x0040, "bridge"),
            (0x0080, "varargs"),
            (0x0100, "native"),
            (0x0400, "abstract"),
            (0x0800, "strictfp"),
        ],
    };
    said.extend(
        particular
            .iter()
            .filter(|(bit, _)| bits & bit != 0)
            .map(|(_, name)| (*name).to_owned()),
    );
    said
}

/// A field or method descriptor read out.
fn descriptor_reads_as(descriptor: &str) -> String {
    if let Some(rest) = descriptor.strip_prefix('(') {
        let Some((arguments, returns)) = rest.split_once(')') else {
            return descriptor.to_owned();
        };
        let mut taken = Vec::new();
        let mut cursor = arguments;
        while !cursor.is_empty() {
            let (one, rest) = one_type(cursor);
            taken.push(one);
            cursor = rest;
        }
        let (returned, _) = one_type(returns);
        return format!("{returned} ({})", taken.join(", "));
    }
    one_type(descriptor).0
}

/// The first type in `descriptor`, and what is left after it.
fn one_type(descriptor: &str) -> (String, &str) {
    let mut rest = descriptor;
    let mut arrays = 0usize;
    while let Some(after) = rest.strip_prefix('[') {
        arrays += 1;
        rest = after;
    }
    let (name, after) = match rest.chars().next() {
        Some('B') => ("byte".to_owned(), &rest[1..]),
        Some('C') => ("char".to_owned(), &rest[1..]),
        Some('D') => ("double".to_owned(), &rest[1..]),
        Some('F') => ("float".to_owned(), &rest[1..]),
        Some('I') => ("int".to_owned(), &rest[1..]),
        Some('J') => ("long".to_owned(), &rest[1..]),
        Some('S') => ("short".to_owned(), &rest[1..]),
        Some('Z') => ("boolean".to_owned(), &rest[1..]),
        Some('V') => ("void".to_owned(), &rest[1..]),
        Some('L') => match rest[1..].find(';') {
            Some(end) => (
                rest[1..=end].rsplit('/').next().unwrap_or("").to_owned(),
                &rest[end + 2..],
            ),
            None => (rest.to_owned(), ""),
        },
        _ => (rest.to_owned(), ""),
    };
    (name + &"[]".repeat(arrays), after)
}

/// The attributes of one member or of the class, as name and body.
fn attributes_of<'bytes>(
    reader: &mut Reader<'bytes>,
    pool: &Pool,
) -> Option<Vec<(String, &'bytes [u8])>> {
    let count = reader.u16()?;
    let mut found = Vec::new();
    for _ in 0..count {
        let name = reader.u16()?;
        let length = reader.u32()? as usize;
        let body = reader.take(length)?;
        found.push((pool.text(name).unwrap_or_default().to_owned(), body));
    }
    Some(found)
}

/// The fields or methods at the cursor.
fn members_of(reader: &mut Reader, pool: &Pool, carrier: Carrier) -> Option<Vec<Member>> {
    let count = reader.u16()?;
    let mut found = Vec::new();
    for _ in 0..count {
        let flags = reader.u16()?;
        let name = reader.u16()?;
        let descriptor = reader.u16()?;
        let attributes = attributes_of(reader, pool)?;
        let descriptor = pool.text(descriptor).unwrap_or_default().to_owned();
        found.push(Member {
            name: pool.text(name).unwrap_or_default().to_owned(),
            reads_as: descriptor_reads_as(&descriptor),
            descriptor,
            flags: flags_of(flags, carrier),
            signature: attributes
                .iter()
                .find(|(name, _)| name == "Signature")
                .and_then(|(_, body)| body.get(..2))
                .map(|index| u16::from_be_bytes([index[0], index[1]]))
                .and_then(|index| pool.text(index))
                .map(ToOwned::to_owned),
        });
    }
    Some(found)
}

/// Everything [`JavaclassView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<JavaclassView> {
    let bytes = std::fs::read(path)?;
    parse(&bytes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not a readable Java class file"))
}

/// [`JavaclassView`] from a class file's bytes, or `None` if malformed.
fn parse(bytes: &[u8]) -> Option<JavaclassView> {
    if !looks_like_it(bytes) {
        return None;
    }
    let mut reader = Reader { bytes, at: 4 };
    let minor = reader.u16()?;
    let major = reader.u16()?;

    let count = reader.u16()?;
    let mut slots = vec![Constant::Other];
    while slots.len() < count as usize {
        let (constant, width) = constant_at(&mut reader)?;
        slots.push(constant);
        for _ in 1..width {
            slots.push(Constant::Other);
        }
    }
    let pool = Pool { slots };

    let flags = reader.u16()?;
    let this_class = reader.u16()?;
    let super_class = reader.u16()?;
    let interface_count = reader.u16()?;
    let mut interfaces = Vec::new();
    for _ in 0..interface_count {
        let index = reader.u16()?;
        if let Some(name) = pool.class(index) {
            interfaces.push(name);
        }
    }

    let fields = members_of(&mut reader, &pool, Carrier::Field)?;
    let methods = members_of(&mut reader, &pool, Carrier::Method)?;
    let attributes = attributes_of(&mut reader, &pool)?;

    let class_name = pool.class(this_class).unwrap_or_default();
    let nested_classes: Vec<String> = attributes
        .iter()
        .filter(|(name, _)| name == "InnerClasses")
        .flat_map(|(_, body)| nested_in(body, &pool, &class_name))
        .collect();
    Some(JavaclassView {
        version: format!("{major}.{minor}"),
        java_release: release_of(major),
        superclass: pool.class(super_class),
        class_name,
        interfaces,
        flags: flags_of(flags, Carrier::Class),
        fields,
        methods,
        constant_pool_size: count as usize,
        source_file: attributes
            .iter()
            .find(|(name, _)| name == "SourceFile")
            .and_then(|(_, body)| body.get(..2))
            .map(|index| u16::from_be_bytes([index[0], index[1]]))
            .and_then(|index| pool.text(index))
            .map(ToOwned::to_owned),
        annotations: attributes
            .iter()
            .filter(|(name, _)| name == "RuntimeVisibleAnnotations")
            .flat_map(|(_, body)| annotations_in(body, &pool))
            .collect(),
        nested_classes,
    })
}

/// The annotation types an attribute names.
///
/// Only the types, not their values: what a reader wants from
/// `@Deprecated` is that it is there.
fn annotations_in(body: &[u8], pool: &Pool) -> Vec<String> {
    let Some(count) = body.get(..2) else {
        return Vec::new();
    };
    let count = u16::from_be_bytes([count[0], count[1]]);
    let mut found = Vec::new();
    let mut at = 2usize;
    for _ in 0..count {
        let Some(head) = body.get(at..at + 4) else {
            break;
        };
        let index = u16::from_be_bytes([head[0], head[1]]);
        let pairs = u16::from_be_bytes([head[2], head[3]]);
        if let Some(descriptor) = pool.text(index) {
            found.push(format!("@{}", one_type(descriptor).0));
        }
        // An element value is a recursive grammar - a value can be
        // another annotation, or an array of them - and stepping over
        // one means implementing all of it. An annotation that carries
        // values is therefore the last one reported, rather than the
        // first of a run of wrong ones.
        if pairs > 0 {
            break;
        }
        at += 4;
    }
    found
}

/// The nested classes an `InnerClasses` attribute names, in source
/// notation.
///
/// A nested class's own file carries an entry naming *itself*, because
/// that is how the attribute records the relationship, so `itself` is
/// skipped: a class does not contain itself.
fn nested_in(body: &[u8], pool: &Pool, itself: &str) -> Vec<String> {
    let Some(count) = body.get(..2) else {
        return Vec::new();
    };
    let count = u16::from_be_bytes([count[0], count[1]]);
    let mut found = Vec::new();
    for index in 0..count as usize {
        let at = 2 + index * 8;
        let Some(entry) = body.get(at..at + 8) else {
            break;
        };
        let inner = u16::from_be_bytes([entry[0], entry[1]]);
        if let Some(name) = pool.class(inner)
            && name != itself
        {
            found.push(name);
        }
    }
    found
}

/// The Java class file plugin's core half.
#[derive(Debug, Default)]
pub struct JavaclassCore;

impl PluginCore for JavaclassCore {
    fn name(&self) -> &'static str {
        "javaclass"
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

/// The Java class file plugin's presentation half.
#[derive(Debug, Default)]
pub struct JavaclassPresentation;

impl PluginPresentation for JavaclassPresentation {
    fn name(&self) -> &'static str {
        "javaclass"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "CLS",
            tint: 0x0053_82a1,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: JavaclassView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "Java class file {} ({})",
            view.version, view.java_release
        )];
        for annotation in &view.annotations {
            lines.push(annotation.clone());
        }
        let mut declaration = view.flags.join(" ");
        if !declaration.is_empty() {
            declaration.push(' ');
        }
        declaration.push_str("class ");
        declaration.push_str(&view.class_name);
        lines.push(declaration);
        if let Some(superclass) = &view.superclass {
            lines.push(format!("  extends {superclass}"));
        }
        if !view.interfaces.is_empty() {
            lines.push(format!("  implements {}", view.interfaces.join(", ")));
        }
        if let Some(source) = &view.source_file {
            lines.push(format!("Compiled from {source}"));
        }
        lines.push(format!("{} constant pool entries", view.constant_pool_size));
        if view.fields.is_empty() {
            lines.push("No fields.".to_owned());
        } else {
            lines.push("Fields:".to_owned());
            for field in &view.fields {
                lines.push(format!(
                    "  {}{} {}   {}",
                    say_flags(&field.flags),
                    field.reads_as,
                    field.name,
                    field.descriptor
                ));
            }
        }
        lines.push("Methods:".to_owned());
        for method in &view.methods {
            lines.push(format!(
                "  {}{} {}   {}",
                say_flags(&method.flags),
                method.reads_as,
                method.name,
                method.descriptor
            ));
            if let Some(signature) = &method.signature {
                lines.push(format!("      generic: {signature}"));
            }
        }
        for nested in &view.nested_classes {
            lines.push(format!("Nested: {nested}"));
        }
        lines
    }
}

/// Access flags with a trailing space, or nothing when there are none.
fn say_flags(flags: &[String]) -> String {
    if flags.is_empty() {
        String::new()
    } else {
        format!("{} ", flags.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        JavaclassCore, JavaclassPresentation, JavaclassView, descriptor_reads_as, looks_like_it,
        release_of,
    };
    use plugin_api::{PluginCore, PluginPresentation};

    fn sample(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/javaclass")
            .join(name)
    }

    fn view_of(name: &str) -> JavaclassView {
        serde_json::from_value(JavaclassCore.view(&sample(name)).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&JavaclassCore),
            PluginPresentation::extensions(&JavaclassPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn the_magic_alone_is_not_enough() {
        assert!(looks_like_it(&[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 65]));
        assert!(
            !looks_like_it(&[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 2]),
            "a Mach-O fat binary opens the same way and counts architectures there"
        );
        assert!(!looks_like_it(&[0xca, 0xfe, 0xba, 0xbe]), "no version yet");
        assert!(!looks_like_it(b"PK\x03\x04"));
        assert!(!looks_like_it(b""));
    }

    #[test]
    fn a_major_version_names_its_release() {
        assert_eq!(release_of(65), "Java 21");
        assert_eq!(release_of(52), "Java 8");
        assert_eq!(release_of(46), "Java 1.2");
        assert_eq!(release_of(45), "Java 1.1 or earlier");
    }

    #[test]
    fn a_descriptor_reads_out_as_a_type() {
        assert_eq!(descriptor_reads_as("Ljava/lang/String;"), "String");
        assert_eq!(descriptor_reads_as("I"), "int");
        assert_eq!(descriptor_reads_as("[[B"), "byte[][]");
        assert_eq!(
            descriptor_reads_as("(Ljava/util/List;)D"),
            "double (List)",
            "the return type comes first, the way it is written in source"
        );
        assert_eq!(
            descriptor_reads_as("(ILjava/lang/String;[J)V"),
            "void (int, String, long[])"
        );
        assert_eq!(descriptor_reads_as("()V"), "void ()");
    }

    #[test]
    fn reads_the_version_the_class_and_what_it_extends() {
        let view = view_of("Column.class");

        assert_eq!(view.version, "65.0");
        assert_eq!(view.java_release, "Java 21");
        assert_eq!(view.class_name, "com.example.csvstats.Column");
        assert_eq!(view.superclass.as_deref(), Some("java.lang.Object"));
        assert_eq!(view.interfaces, vec!["com.example.csvstats.Readable"]);
        assert_eq!(view.flags, vec!["public", "final"]);
        assert!(view.constant_pool_size > 20);
        assert_eq!(view.source_file.as_deref(), Some("Column.java"));
    }

    #[test]
    fn reads_the_fields_with_their_flags_and_descriptors() {
        let view = view_of("Column.class");

        let limit = &view.fields[0];
        assert_eq!(limit.name, "LIMIT");
        assert_eq!(limit.descriptor, "I");
        assert_eq!(limit.reads_as, "int");
        assert_eq!(limit.flags, vec!["private", "static", "final"]);

        let name = &view.fields[1];
        assert_eq!(name.name, "name");
        assert_eq!(name.reads_as, "String");
        assert_eq!(name.flags, vec!["private", "final"]);
    }

    #[test]
    fn a_generic_method_keeps_the_signature_its_descriptor_lost() {
        let view = view_of("Column.class");

        let mean = view
            .methods
            .iter()
            .find(|method| method.name == "mean")
            .expect("the generic method");

        assert_eq!(mean.descriptor, "(Ljava/util/List;)D");
        assert_eq!(mean.reads_as, "double (List)");
        assert_eq!(
            mean.signature.as_deref(),
            Some("<T:Ljava/lang/Number;>(Ljava/util/List<TT;>;)D"),
            "the descriptor erased the type parameter; the signature kept it"
        );
        assert!(
            view.methods
                .iter()
                .all(|method| method.signature.is_none() || method.name == "mean"),
            "only the generic one has a signature"
        );
    }

    #[test]
    fn reads_the_annotation_and_the_nested_class() {
        let view = view_of("Column.class");

        assert_eq!(view.annotations, vec!["@Deprecated"]);
        assert_eq!(
            view.nested_classes,
            vec!["com.example.csvstats.Column$Summary"]
        );
    }

    #[test]
    fn the_nested_class_names_the_one_it_sits_in() {
        let view = view_of("Column$Summary.class");

        assert_eq!(view.class_name, "com.example.csvstats.Column$Summary");
        assert_eq!(view.fields[0].name, "mean");
        assert_eq!(view.fields[0].reads_as, "double");
        assert_eq!(view.source_file.as_deref(), Some("Column.java"));
        assert!(
            view.nested_classes.is_empty(),
            "its InnerClasses entry names itself, and a class does not contain itself"
        );
    }

    #[test]
    fn presents_the_declaration_the_way_it_was_written() {
        let data = JavaclassCore.view(&sample("Column.class")).unwrap();

        let lines = JavaclassPresentation.present(&data);

        assert!(lines[0].starts_with("Java class file 65.0 (Java 21)"));
        assert!(lines.iter().any(|line| line == "@Deprecated"));
        assert!(
            lines
                .iter()
                .any(|line| line == "public final class com.example.csvstats.Column")
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("implements com.example.csvstats.Readable"))
        );
        assert!(lines.iter().any(|line| line.contains("double (List) mean")));
        assert!(lines.iter().any(|line| line.contains("generic: <T:")));
    }

    #[test]
    fn a_file_that_is_not_a_class_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really-a.class");
        std::fs::write(&path, b"\xca\xfe\xba\xbe\x00\x00\x00\x41 and then nothing").unwrap();

        assert!(JavaclassCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
