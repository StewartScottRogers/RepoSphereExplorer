//! .NET assembly file type plugin: core and presentation halves.
//!
//! A .NET assembly is a portable executable with a fourteenth data
//! directory entry pointing at a common language infrastructure (CLI)
//! header, and everything worth reading is behind that: a metadata root,
//! a handful of heaps, and a stream of tables.
//!
//! The tables are the hard part and there is no shortcut. Each row is a
//! run of columns whose *widths depend on the file*: a heap index is two
//! bytes or four depending on how big that heap is, a table index is two
//! or four depending on how many rows that table has, and a coded index
//! is sized from the largest of the tables it can point into. So the
//! only way to reach the assembly's own row is to know the shape of
//! every table before it, which is why the schema below is written out
//! in full.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &[];

/// The two bytes every portable executable opens with.
const DOS_MAGIC: &[u8] = b"MZ";

/// The four that open its real header.
const PE_MAGIC: &[u8] = b"PE\0\0";

/// The data directory entry holding the CLI header. Its presence is
/// what makes a portable executable a .NET assembly.
const CLI_DIRECTORY: usize = 14;

/// The four bytes a metadata root opens with.
const METADATA_MAGIC: u32 = 0x424A_5342;

/// How many of each kind are listed before the rest are only counted.
const SHOWN: usize = 64;

/// One assembly this one needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    /// Its name.
    pub name: String,
    /// The version this one was built against.
    pub version: String,
}

/// View data produced by [`DotnetassemblyCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DotnetassemblyView {
    /// The assembly's name, which is not necessarily the file's.
    pub name: String,
    /// Its version, as four numbers.
    pub version: String,
    /// Its culture, when it has one. A satellite assembly holding
    /// translations does; the assembly it belongs to does not.
    pub culture: Option<String>,
    /// The framework it was built for, from its
    /// `TargetFrameworkAttribute`.
    pub target_framework: Option<String>,
    /// The metadata version string, which names the runtime that wrote
    /// it rather than the one that will run it.
    pub runtime_version: String,
    /// The assemblies it needs, in metadata order.
    pub references: Vec<Reference>,
    /// Its public types, namespace included.
    pub public_types: Vec<String>,
    /// How many types it defines in total, public or not.
    pub type_count: usize,
    /// The method the runtime starts at, when there is one. A library
    /// has none.
    pub entry_point: Option<String>,
    /// Whether it is strong-named: signed with a key, so the runtime
    /// can tell it from another assembly of the same name.
    pub strong_named: bool,
    /// The CLI header's flags, read out.
    pub attributes: Vec<String>,
    /// Which tables the metadata carries, with their row counts.
    pub tables: Vec<String>,
}

/// Two bytes at `at`, little-endian.
fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

/// Four bytes at `at`, little-endian.
fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Eight bytes at `at`, little-endian.
fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

/// Where the portable executable header starts.
fn pe_header_at(bytes: &[u8]) -> Option<usize> {
    if !bytes.starts_with(DOS_MAGIC) {
        return None;
    }
    let at = u32_at(bytes, 0x3C)? as usize;
    (bytes.get(at..at + 4)? == PE_MAGIC).then_some(at)
}

/// Whether `prefix` opens like a .NET assembly.
///
/// Every .NET assembly is a portable executable, so the `MZ` decides
/// nothing on its own - the executable plugin recognises those too. What
/// settles it is the fourteenth data directory entry: an ordinary native
/// binary leaves it zero, and only an assembly fills it in.
fn looks_like_it(prefix: &[u8]) -> bool {
    cli_header_rva(prefix).is_some()
}

/// The relative address of the CLI header, if the file has one.
fn cli_header_rva(bytes: &[u8]) -> Option<u32> {
    let pe = pe_header_at(bytes)?;
    let optional = pe + 24;
    // The optional header's magic says how wide its addresses are, and
    // therefore where the data directories start within it.
    let directories = match u16_at(bytes, optional)? {
        0x010B => optional + 96,
        0x020B => optional + 112,
        _ => return None,
    };
    let rva = u32_at(bytes, directories + CLI_DIRECTORY * 8)?;
    let size = u32_at(bytes, directories + CLI_DIRECTORY * 8 + 4)?;
    (rva != 0 && size != 0).then_some(rva)
}

/// The sections, as address ranges paired with where they sit in the
/// file. Everything the metadata names is a relative address, and only
/// these turn one into an offset.
fn sections_of(bytes: &[u8]) -> Option<Vec<(u32, u32, u32)>> {
    let pe = pe_header_at(bytes)?;
    let count = u16_at(bytes, pe + 6)? as usize;
    let optional_size = u16_at(bytes, pe + 20)? as usize;
    let first = pe + 24 + optional_size;
    Some(
        (0..count)
            .filter_map(|index| {
                let at = first + index * 40;
                Some((
                    u32_at(bytes, at + 12)?, // virtual address
                    u32_at(bytes, at + 8)?,  // virtual size
                    u32_at(bytes, at + 20)?, // pointer to raw data
                ))
            })
            .collect(),
    )
}

/// Where in the file a relative address lands.
fn offset_of(sections: &[(u32, u32, u32)], rva: u32) -> Option<usize> {
    sections
        .iter()
        .find(|(start, size, _)| rva >= *start && rva < start.saturating_add(*size))
        .map(|(start, _, raw)| (raw + (rva - start)) as usize)
}

/// What a column in a metadata table row is.
#[derive(Debug, Clone, Copy)]
enum Column {
    /// A fixed-width number of this many bytes.
    Fixed(usize),
    /// An index into the string heap.
    Str,
    /// An index into the GUID heap.
    Guid,
    /// An index into the blob heap.
    Blob,
    /// An index into one named table.
    Table(u8),
    /// An index that can point into any of several tables, with the
    /// table chosen by the low bits.
    Coded(&'static [u8]),
}

use Column::{Blob, Coded, Fixed, Guid, Str, Table};

/// The table a coded index's tag can name, in tag order. `0xFF` is a
/// tag the specification leaves unused.
const TYPE_DEF_OR_REF: &[u8] = &[0x02, 0x01, 0x1B];
/// Where a constant can be attached.
const HAS_CONSTANT: &[u8] = &[0x04, 0x08, 0x17];
/// Where a custom attribute can be attached, which is nearly anywhere.
const HAS_CUSTOM_ATTRIBUTE: &[u8] = &[
    0x06, 0x04, 0x01, 0x02, 0x08, 0x09, 0x0A, 0x00, 0x0E, 0x17, 0x14, 0x11, 0x1A, 0x1B, 0x20, 0x23,
    0x26, 0x27, 0x28, 0x2A, 0x2C, 0x2B,
];
/// Where marshalling information can be attached.
const HAS_FIELD_MARSHAL: &[u8] = &[0x04, 0x08];
/// Where a security declaration can be attached.
const HAS_DECL_SECURITY: &[u8] = &[0x02, 0x06, 0x20];
/// What a member reference can belong to.
const MEMBER_REF_PARENT: &[u8] = &[0x02, 0x01, 0x1A, 0x06, 0x1B];
/// What a method's semantics can belong to.
const HAS_SEMANTICS: &[u8] = &[0x14, 0x17];
/// A method, defined here or referenced elsewhere.
const METHOD_DEF_OR_REF: &[u8] = &[0x06, 0x0A];
/// What can be forwarded to unmanaged code.
const MEMBER_FORWARDED: &[u8] = &[0x04, 0x06];
/// Where an exported thing actually lives.
const IMPLEMENTATION: &[u8] = &[0x26, 0x23, 0x27];
/// What a custom attribute's constructor can be.
const CUSTOM_ATTRIBUTE_TYPE: &[u8] = &[0xFF, 0xFF, 0x06, 0x0A, 0xFF];
/// Where a type reference's name is resolved from.
const RESOLUTION_SCOPE: &[u8] = &[0x00, 0x1A, 0x23, 0x01];
/// What a generic parameter can belong to.
const TYPE_OR_METHOD_DEF: &[u8] = &[0x02, 0x06];

/// Every metadata table, in order, and the columns each row holds.
///
/// Written out in full because a row's width depends on every table
/// before it: to reach the assembly's row at table `0x20` the reader
/// has to know how wide a `MethodDef` row is, and to know that it has to
/// know how many rows `Param` has.
const SCHEMA: &[&[Column]] = &[
    /* 0x00 Module */ &[Fixed(2), Str, Guid, Guid, Guid],
    /* 0x01 TypeRef */ &[Coded(RESOLUTION_SCOPE), Str, Str],
    /* 0x02 TypeDef */
    &[
        Fixed(4),
        Str,
        Str,
        Coded(TYPE_DEF_OR_REF),
        Table(0x04),
        Table(0x06),
    ],
    /* 0x03 FieldPtr */ &[Table(0x04)],
    /* 0x04 Field */ &[Fixed(2), Str, Blob],
    /* 0x05 MethodPtr */ &[Table(0x06)],
    /* 0x06 MethodDef */ &[Fixed(4), Fixed(2), Fixed(2), Str, Blob, Table(0x08)],
    /* 0x07 ParamPtr */ &[Table(0x08)],
    /* 0x08 Param */ &[Fixed(2), Fixed(2), Str],
    /* 0x09 InterfaceImpl */ &[Table(0x02), Coded(TYPE_DEF_OR_REF)],
    /* 0x0A MemberRef */ &[Coded(MEMBER_REF_PARENT), Str, Blob],
    /* 0x0B Constant */ &[Fixed(2), Coded(HAS_CONSTANT), Blob],
    /* 0x0C CustomAttribute */
    &[
        Coded(HAS_CUSTOM_ATTRIBUTE),
        Coded(CUSTOM_ATTRIBUTE_TYPE),
        Blob,
    ],
    /* 0x0D FieldMarshal */ &[Coded(HAS_FIELD_MARSHAL), Blob],
    /* 0x0E DeclSecurity */ &[Fixed(2), Coded(HAS_DECL_SECURITY), Blob],
    /* 0x0F ClassLayout */ &[Fixed(2), Fixed(4), Table(0x02)],
    /* 0x10 FieldLayout */ &[Fixed(4), Table(0x04)],
    /* 0x11 StandAloneSig */ &[Blob],
    /* 0x12 EventMap */ &[Table(0x02), Table(0x14)],
    /* 0x13 EventPtr */ &[Table(0x14)],
    /* 0x14 Event */ &[Fixed(2), Str, Coded(TYPE_DEF_OR_REF)],
    /* 0x15 PropertyMap */ &[Table(0x02), Table(0x17)],
    /* 0x16 PropertyPtr */ &[Table(0x17)],
    /* 0x17 Property */ &[Fixed(2), Str, Blob],
    /* 0x18 MethodSemantics */ &[Fixed(2), Table(0x06), Coded(HAS_SEMANTICS)],
    /* 0x19 MethodImpl */
    &[
        Table(0x02),
        Coded(METHOD_DEF_OR_REF),
        Coded(METHOD_DEF_OR_REF),
    ],
    /* 0x1A ModuleRef */ &[Str],
    /* 0x1B TypeSpec */ &[Blob],
    /* 0x1C ImplMap */ &[Fixed(2), Coded(MEMBER_FORWARDED), Str, Table(0x1A)],
    /* 0x1D FieldRVA */ &[Fixed(4), Table(0x04)],
    /* 0x1E EncLog */ &[Fixed(4), Fixed(4)],
    /* 0x1F EncMap */ &[Fixed(4)],
    /* 0x20 Assembly */
    &[
        Fixed(4),
        Fixed(2),
        Fixed(2),
        Fixed(2),
        Fixed(2),
        Fixed(4),
        Blob,
        Str,
        Str,
    ],
    /* 0x21 AssemblyProcessor */ &[Fixed(4)],
    /* 0x22 AssemblyOS */ &[Fixed(4), Fixed(4), Fixed(4)],
    /* 0x23 AssemblyRef */
    &[
        Fixed(2),
        Fixed(2),
        Fixed(2),
        Fixed(2),
        Fixed(4),
        Blob,
        Str,
        Str,
        Blob,
    ],
    /* 0x24 AssemblyRefProcessor */ &[Fixed(4), Table(0x23)],
    /* 0x25 AssemblyRefOS */ &[Fixed(4), Fixed(4), Fixed(4), Table(0x23)],
    /* 0x26 File */ &[Fixed(4), Str, Blob],
    /* 0x27 ExportedType */
    &[Fixed(4), Fixed(4), Str, Str, Coded(IMPLEMENTATION)],
    /* 0x28 ManifestResource */ &[Fixed(4), Fixed(4), Str, Coded(IMPLEMENTATION)],
    /* 0x29 NestedClass */ &[Table(0x02), Table(0x02)],
    /* 0x2A GenericParam */
    &[Fixed(2), Fixed(2), Coded(TYPE_OR_METHOD_DEF), Str],
    /* 0x2B MethodSpec */ &[Coded(METHOD_DEF_OR_REF), Blob],
    /* 0x2C GenericParamConstraint */ &[Table(0x2A), Coded(TYPE_DEF_OR_REF)],
];

/// What a table's name is, for the ones worth naming in the pane.
fn table_named(id: usize) -> String {
    const NAMES: &[(usize, &str)] = &[
        (0x00, "Module"),
        (0x01, "TypeRef"),
        (0x02, "TypeDef"),
        (0x04, "Field"),
        (0x06, "MethodDef"),
        (0x08, "Param"),
        (0x09, "InterfaceImpl"),
        (0x0A, "MemberRef"),
        (0x0B, "Constant"),
        (0x0C, "CustomAttribute"),
        (0x0D, "FieldMarshal"),
        (0x0E, "DeclSecurity"),
        (0x0F, "ClassLayout"),
        (0x10, "FieldLayout"),
        (0x11, "StandAloneSig"),
        (0x12, "EventMap"),
        (0x14, "Event"),
        (0x15, "PropertyMap"),
        (0x17, "Property"),
        (0x18, "MethodSemantics"),
        (0x19, "MethodImpl"),
        (0x1A, "ModuleRef"),
        (0x1B, "TypeSpec"),
        (0x1C, "ImplMap"),
        (0x1D, "FieldRVA"),
        (0x20, "Assembly"),
        (0x23, "AssemblyRef"),
        (0x26, "File"),
        (0x27, "ExportedType"),
        (0x28, "ManifestResource"),
        (0x29, "NestedClass"),
        (0x2A, "GenericParam"),
        (0x2B, "MethodSpec"),
        (0x2C, "GenericParamConstraint"),
    ];
    NAMES.iter().find(|(had, _)| *had == id).map_or_else(
        || format!("table 0x{id:02x}"),
        |(_, name)| (*name).to_owned(),
    )
}

/// The tables stream, laid out so a row can be reached.
struct Tables<'bytes> {
    /// The whole file.
    bytes: &'bytes [u8],
    /// Where the string heap starts.
    strings: usize,
    /// Where the blob heap starts.
    blobs: usize,
    /// How many rows each table has.
    rows: [u32; 64],
    /// Where each table's first row sits in the file.
    starts: [usize; 64],
    /// How wide a row of each table is.
    widths: [usize; 64],
    /// Where each column begins within a row, per table.
    columns: Vec<Vec<usize>>,
    /// How wide each of those columns is.
    sizes: Vec<Vec<usize>>,
}

/// How wide an index into `tables` has to be, given a tag of `bits`.
fn coded_width(rows: &[u32; 64], tables: &[u8], bits: u32) -> usize {
    let largest = tables
        .iter()
        .filter(|id| **id != 0xFF)
        .map(|id| rows[*id as usize])
        .max()
        .unwrap_or(0);
    if u64::from(largest) < (1u64 << (16 - bits)) {
        2
    } else {
        4
    }
}

/// How many bits a coded index's tag takes: enough for every table it
/// can name.
fn tag_bits(tables: &[u8]) -> u32 {
    let mut bits = 0;
    while (1usize << bits) < tables.len() {
        bits += 1;
    }
    bits
}

impl<'bytes> Tables<'bytes> {
    /// Reads the tables stream at `at`, given the heap index widths.
    fn new(
        bytes: &'bytes [u8],
        at: usize,
        strings: usize,
        blobs: usize,
        guid_wide: bool,
        string_wide: bool,
        blob_wide: bool,
    ) -> Option<Self> {
        let valid = u64_at(bytes, at + 8)?;
        let mut rows = [0u32; 64];
        let mut cursor = at + 24;
        // A row count is written only for the tables the `valid` word
        // says are present, in table order and with no gaps, so the
        // counts have to be walked in step with the bits.
        for (id, count) in rows.iter_mut().enumerate() {
            if valid & (1u64 << id) != 0 {
                *count = u32_at(bytes, cursor)?;
                cursor += 4;
            }
        }

        let width = |column: Column| match column {
            Fixed(size) => size,
            Str => usize::from(string_wide) * 2 + 2,
            Guid => usize::from(guid_wide) * 2 + 2,
            Blob => usize::from(blob_wide) * 2 + 2,
            Table(id) => {
                if rows[id as usize] < (1 << 16) {
                    2
                } else {
                    4
                }
            }
            Coded(group) => coded_width(&rows, group, tag_bits(group)),
        };

        let mut starts = [0usize; 64];
        let mut widths = [0usize; 64];
        let mut columns = vec![Vec::new(); 64];
        let mut sizes = vec![Vec::new(); 64];
        for id in 0..64usize {
            if valid & (1u64 << id) == 0 {
                continue;
            }
            // A table this reader has no schema for cannot be stepped
            // over, and everything after it would be read at the wrong
            // offset. Stopping is the only honest answer.
            let schema = SCHEMA.get(id)?;
            let mut offset = 0usize;
            for column in *schema {
                columns[id].push(offset);
                let size = width(*column);
                sizes[id].push(size);
                offset += size;
            }
            starts[id] = cursor;
            widths[id] = offset;
            cursor += offset * rows[id] as usize;
        }
        Some(Tables {
            bytes,
            strings,
            blobs,
            rows,
            starts,
            widths,
            columns,
            sizes,
        })
    }

    /// The value of column `column` in row `row` of table `id`. Rows are
    /// numbered from one, the way metadata numbers them.
    fn cell(&self, id: usize, row: u32, column: usize) -> Option<u32> {
        if row == 0 || row > self.rows[id] {
            return None;
        }
        let at = self.starts[id]
            + (row as usize - 1) * self.widths[id]
            + *self.columns.get(id)?.get(column)?;
        match self.sizes[id][column] {
            1 => self.bytes.get(at).map(|byte| u32::from(*byte)),
            2 => u16_at(self.bytes, at).map(u32::from),
            _ => u32_at(self.bytes, at),
        }
    }

    /// The string at a string-heap index.
    fn string(&self, index: u32) -> Option<&'bytes str> {
        let at = self.strings + index as usize;
        let rest = self.bytes.get(at..)?;
        let end = rest.iter().position(|byte| *byte == 0)?;
        std::str::from_utf8(&rest[..end]).ok()
    }

    /// The bytes at a blob-heap index. A blob is prefixed with its
    /// length, compressed into one, two or four bytes.
    fn blob(&self, index: u32) -> Option<&'bytes [u8]> {
        let at = self.blobs + index as usize;
        let (length, taken) = compressed_at(self.bytes, at)?;
        self.bytes.get(at + taken..at + taken + length as usize)
    }
}

/// A compressed unsigned integer, and how many bytes it took. The top
/// bits of the first byte say how long it is.
fn compressed_at(bytes: &[u8], at: usize) -> Option<(u32, usize)> {
    let first = *bytes.get(at)?;
    if first & 0x80 == 0 {
        return Some((u32::from(first), 1));
    }
    if first & 0x40 == 0 {
        let second = *bytes.get(at + 1)?;
        return Some(((u32::from(first & 0x3F) << 8) | u32::from(second), 2));
    }
    let run = bytes.get(at..at + 4)?;
    Some((
        (u32::from(run[0] & 0x1F) << 24)
            | (u32::from(run[1]) << 16)
            | (u32::from(run[2]) << 8)
            | u32::from(run[3]),
        4,
    ))
}

/// The CLI header's flags, read out.
fn attributes_in(flags: u32) -> Vec<String> {
    const NAMED: &[(u32, &str)] = &[
        (0x0000_0001, "IL only"),
        (0x0000_0002, "32-bit only"),
        (0x0000_0008, "strong-name signed"),
        (0x0000_0010, "native entry point"),
        (0x0001_0000, "tracks debug data"),
        (0x0002_0000, "32-bit preferred"),
    ];
    NAMED
        .iter()
        .filter(|(bit, _)| flags & bit != 0)
        .map(|(_, name)| (*name).to_owned())
        .collect()
}

/// The metadata streams, by name, as offsets into the file.
fn streams_at(bytes: &[u8], root: usize) -> Option<Vec<(String, usize, usize)>> {
    let version_length = u32_at(bytes, root + 12)? as usize;
    let after_version = root + 16 + version_length;
    let count = u16_at(bytes, after_version + 2)? as usize;
    let mut at = after_version + 4;
    let mut found = Vec::new();
    for _ in 0..count {
        let offset = u32_at(bytes, at)? as usize;
        let size = u32_at(bytes, at + 4)? as usize;
        let name_at = at + 8;
        let rest = bytes.get(name_at..)?;
        let end = rest.iter().position(|byte| *byte == 0)?;
        let name = String::from_utf8_lossy(&rest[..end]).into_owned();
        found.push((name, root + offset, size));
        // The name is padded out to a four-byte boundary, counting its
        // own terminator.
        at = name_at + (end + 1).div_ceil(4) * 4;
    }
    Some(found)
}

/// The value of the assembly's `TargetFrameworkAttribute`, if it has one.
///
/// The attribute's argument is a serialised string: a two-byte prolog,
/// then a length-prefixed run of UTF-8. Finding the right attribute
/// means walking the custom attributes attached to the assembly row and
/// asking, of each, what type its constructor belongs to.
fn target_framework(tables: &Tables) -> Option<String> {
    // The assembly is row one of table 0x20, and tag 14 of the
    // `HasCustomAttribute` coded index names that table.
    let bits = tag_bits(HAS_CUSTOM_ATTRIBUTE);
    // Tag fourteen names the `Assembly` table, and the row index sits
    // above the tag bits: row one of it is therefore `(1 << bits) | 14`.
    let wanted = (1u32 << bits) | 0x0E;
    for row in 1..=tables.rows[0x0C] {
        if tables.cell(0x0C, row, 0)? != wanted {
            continue;
        }
        let constructor = tables.cell(0x0C, row, 1)?;
        let tag = constructor & ((1 << tag_bits(CUSTOM_ATTRIBUTE_TYPE)) - 1);
        // Tag three is a member reference, which is how an attribute
        // defined in another assembly is named.
        if tag != 3 {
            continue;
        }
        let member = constructor >> tag_bits(CUSTOM_ATTRIBUTE_TYPE);
        let parent = tables.cell(0x0A, member, 0)?;
        // Tag one of `MemberRefParent` is a type reference.
        if parent & ((1 << tag_bits(MEMBER_REF_PARENT)) - 1) != 1 {
            continue;
        }
        let type_ref = parent >> tag_bits(MEMBER_REF_PARENT);
        if tables.string(tables.cell(0x01, type_ref, 1)?) != Some("TargetFrameworkAttribute") {
            continue;
        }
        let blob = tables.blob(tables.cell(0x0C, row, 2)?)?;
        let (length, taken) = compressed_at(blob, 2)?;
        let run = blob.get(2 + taken..2 + taken + length as usize)?;
        return std::str::from_utf8(run).ok().map(ToOwned::to_owned);
    }
    None
}

/// The public types the assembly defines, and how many there are in all.
fn types_in(tables: &Tables) -> (Vec<String>, usize) {
    /// The bits of a type's flags that say who can see it.
    const VISIBILITY: u32 = 0x0000_0007;
    let mut public = Vec::new();
    let count = tables.rows[0x02] as usize;
    for row in 1..=tables.rows[0x02] {
        let Some(flags) = tables.cell(0x02, row, 0) else {
            break;
        };
        // One is public at the top level, two is public and nested.
        if !matches!(flags & VISIBILITY, 1 | 2) {
            continue;
        }
        let (Some(name), Some(namespace)) = (
            tables
                .cell(0x02, row, 1)
                .and_then(|index| tables.string(index)),
            tables
                .cell(0x02, row, 2)
                .and_then(|index| tables.string(index)),
        ) else {
            continue;
        };
        if public.len() < SHOWN {
            public.push(if namespace.is_empty() {
                name.to_owned()
            } else {
                format!("{namespace}.{name}")
            });
        }
    }
    // The first row of `TypeDef` is the compiler's placeholder for
    // everything outside a type, and is not a type anybody wrote.
    (public, count.saturating_sub(1))
}

/// The assemblies this one references.
fn references_in(tables: &Tables) -> Vec<Reference> {
    (1..=tables.rows[0x23])
        .filter_map(|row| {
            Some(Reference {
                name: tables
                    .cell(0x23, row, 6)
                    .and_then(|index| tables.string(index))?
                    .to_owned(),
                version: format!(
                    "{}.{}.{}.{}",
                    tables.cell(0x23, row, 0)?,
                    tables.cell(0x23, row, 1)?,
                    tables.cell(0x23, row, 2)?,
                    tables.cell(0x23, row, 3)?
                ),
            })
        })
        .take(SHOWN)
        .collect()
}

/// The method a token names, as `Type.Method`.
fn method_named(tables: &Tables, token: u32) -> Option<String> {
    // A token's top byte is the table it points into; a managed entry
    // point is always a `MethodDef`.
    if token >> 24 != 0x06 {
        return None;
    }
    let row = token & 0x00FF_FFFF;
    let method = tables
        .cell(0x06, row, 3)
        .and_then(|index| tables.string(index))?;
    // The type a method belongs to is the last type whose method list
    // starts at or before it: the lists are contiguous and in order.
    let owner = (1..=tables.rows[0x02]).rev().find(|type_row| {
        tables
            .cell(0x02, *type_row, 5)
            .is_some_and(|first| first <= row)
    });
    match owner.and_then(|type_row| {
        tables
            .cell(0x02, type_row, 1)
            .and_then(|index| tables.string(index))
    }) {
        Some(type_name) => Some(format!("{type_name}.{method}")),
        None => Some(method.to_owned()),
    }
}

/// Everything [`DotnetassemblyView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<DotnetassemblyView> {
    let bytes = std::fs::read(path)?;
    parse(&bytes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not a readable .NET assembly"))
}

/// [`DotnetassemblyView`] from an assembly's bytes, or `None` if it is
/// not one or its metadata cannot be walked.
fn parse(bytes: &[u8]) -> Option<DotnetassemblyView> {
    let sections = sections_of(bytes)?;
    let cli = offset_of(&sections, cli_header_rva(bytes)?)?;
    let flags = u32_at(bytes, cli + 16)?;
    let entry_token = u32_at(bytes, cli + 20)?;
    let strong_name_size = u32_at(bytes, cli + 36)?;

    let root = offset_of(&sections, u32_at(bytes, cli + 8)?)?;
    if u32_at(bytes, root)? != METADATA_MAGIC {
        return None;
    }
    let version_length = u32_at(bytes, root + 12)? as usize;
    let runtime_version = String::from_utf8_lossy(
        bytes
            .get(root + 16..root + 16 + version_length)?
            .split(|byte| *byte == 0)
            .next()?,
    )
    .into_owned();

    let streams = streams_at(bytes, root)?;
    let find = |wanted: &str| streams.iter().find(|(name, _, _)| name == wanted);
    let (_, tables_at, _) = find("#~").or_else(|| find("#-"))?;
    let strings = find("#Strings")?.1;
    let blobs = find("#Blob").map_or(0, |(_, at, _)| *at);

    let heap_sizes = *bytes.get(tables_at + 6)?;
    let tables = Tables::new(
        bytes,
        *tables_at,
        strings,
        blobs,
        heap_sizes & 0b010 != 0,
        heap_sizes & 0b001 != 0,
        heap_sizes & 0b100 != 0,
    )?;

    let (public_types, type_count) = types_in(&tables);
    let culture = tables
        .cell(0x20, 1, 8)
        .and_then(|index| tables.string(index))
        .filter(|culture| !culture.is_empty())
        .map(ToOwned::to_owned);

    Some(DotnetassemblyView {
        name: tables
            .cell(0x20, 1, 7)
            .and_then(|index| tables.string(index))
            .unwrap_or_default()
            .to_owned(),
        version: format!(
            "{}.{}.{}.{}",
            tables.cell(0x20, 1, 1)?,
            tables.cell(0x20, 1, 2)?,
            tables.cell(0x20, 1, 3)?,
            tables.cell(0x20, 1, 4)?
        ),
        culture,
        target_framework: target_framework(&tables),
        runtime_version,
        references: references_in(&tables),
        public_types,
        type_count,
        entry_point: (entry_token != 0)
            .then(|| method_named(&tables, entry_token))
            .flatten(),
        // The flag says it was signed; the directory entry says the
        // signature is actually there. Both, or it is not strong-named.
        strong_named: flags & 0x0000_0008 != 0 && strong_name_size != 0,
        attributes: attributes_in(flags),
        tables: (0..64usize)
            .filter(|id| tables.rows[*id] > 0)
            .map(|id| format!("{} ({})", table_named(id), tables.rows[id]))
            .collect(),
    })
}

/// The .NET assembly plugin's core half.
#[derive(Debug, Default)]
pub struct DotnetassemblyCore;

impl PluginCore for DotnetassemblyCore {
    fn name(&self) -> &'static str {
        "dotnetassembly"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // An assembly is a portable executable, which `executable`
        // recognises. This is the narrower reading of the same bytes
        // (D13), and it is what wins the file without needing the
        // extension - which `executable` has, and needs, for the
        // native binaries that are all it can read.
        &["executable"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_it(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The .NET assembly plugin's presentation half.
#[derive(Debug, Default)]
pub struct DotnetassemblyPresentation;

impl PluginPresentation for DotnetassemblyPresentation {
    fn name(&self) -> &'static str {
        "dotnetassembly"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "NET",
            tint: 0x0051_2bd4,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: DotnetassemblyView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(".NET assembly {} {}", view.name, view.version)];
        if let Some(framework) = &view.target_framework {
            lines.push(format!("Built for {framework}"));
        }
        lines.push(format!("Metadata written by {}", view.runtime_version));
        lines.push(match &view.culture {
            Some(culture) => format!("Culture {culture}: this holds translations"),
            None => "Culture neutral.".to_owned(),
        });
        lines.push(match &view.entry_point {
            Some(entry) => format!("Starts at {entry}"),
            None => "No entry point, so this is a library.".to_owned(),
        });
        lines.push(if view.strong_named {
            "Strong-named, so the runtime can tell it from another".to_owned()
        } else {
            "Not strong-named.".to_owned()
        });
        if view.strong_named {
            lines.push("assembly of the same name.".to_owned());
        }
        if !view.attributes.is_empty() {
            lines.push(format!("Flags: {}", view.attributes.join(", ")));
        }
        if view.references.is_empty() {
            lines.push("References nothing.".to_owned());
        } else {
            lines.push("References:".to_owned());
            for reference in &view.references {
                lines.push(format!("  {} {}", reference.name, reference.version));
            }
        }
        lines.push(format!("{} type(s) defined", view.type_count));
        if view.public_types.is_empty() {
            lines.push("None of them public: nothing outside can use this.".to_owned());
        } else {
            lines.push("Public:".to_owned());
            for name in &view.public_types {
                lines.push(format!("  {name}"));
            }
        }
        lines.push(format!("Metadata tables: {}", view.tables.join(", ")));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DotnetassemblyCore, DotnetassemblyPresentation, DotnetassemblyView, attributes_in,
        compressed_at, looks_like_it, tag_bits,
    };
    use plugin_api::{PluginCore, PluginPresentation};

    fn sample(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/dotnetassembly")
            .join(name)
    }

    fn view_of(name: &str) -> DotnetassemblyView {
        serde_json::from_value(DotnetassemblyCore.view(&sample(name)).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&DotnetassemblyCore),
            PluginPresentation::extensions(&DotnetassemblyPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn it_claims_no_extension_because_the_executable_plugin_has_those() {
        assert!(
            PluginCore::extensions(&DotnetassemblyCore).is_empty(),
            "`dll` and `exe` belong to `executable`; the CLI header decides this one"
        );
        assert_eq!(DotnetassemblyCore.specialises(), &["executable"]);
    }

    #[test]
    fn the_cli_header_is_what_makes_it_an_assembly() {
        let managed = std::fs::read(sample("CsvStats.dll")).unwrap();

        assert!(looks_like_it(&managed));
        assert!(
            !looks_like_it(b"MZ\x90\x00"),
            "a portable executable is not necessarily a managed one"
        );
        assert!(!looks_like_it(b"\x7fELF"));
        assert!(!looks_like_it(b""));
    }

    #[test]
    fn a_compressed_integer_is_one_two_or_four_bytes() {
        assert_eq!(compressed_at(&[0x03], 0), Some((3, 1)));
        assert_eq!(compressed_at(&[0x80, 0x80], 0), Some((128, 2)));
        assert_eq!(compressed_at(&[0xBF, 0xFF], 0), Some((0x3FFF, 2)));
        assert_eq!(
            compressed_at(&[0xC0, 0x00, 0x40, 0x00], 0),
            Some((0x4000, 4))
        );
    }

    #[test]
    fn a_coded_index_tag_is_wide_enough_for_every_table_it_names() {
        assert_eq!(tag_bits(super::TYPE_DEF_OR_REF), 2, "three tables");
        assert_eq!(tag_bits(super::HAS_CUSTOM_ATTRIBUTE), 5, "twenty-two");
        assert_eq!(tag_bits(super::HAS_SEMANTICS), 1, "two");
    }

    #[test]
    fn the_flag_word_reads_out() {
        assert_eq!(attributes_in(0x0000_0001), vec!["IL only"]);
        assert_eq!(
            attributes_in(0x0000_0009),
            vec!["IL only", "strong-name signed"]
        );
    }

    #[test]
    fn reads_the_library_assembly() {
        let view = view_of("CsvStats.dll");

        assert_eq!(view.name, "CsvStats");
        assert_eq!(view.version, "1.0.3.0");
        assert_eq!(view.culture, None);
        assert_eq!(view.runtime_version, "v4.0.30319");
        assert_eq!(
            view.target_framework.as_deref(),
            Some(".NETCoreApp,Version=v8.0")
        );
    }

    #[test]
    fn reads_the_references_with_the_versions_it_was_built_against() {
        let view = view_of("CsvStats.dll");

        let named: Vec<&str> = view
            .references
            .iter()
            .map(|one| one.name.as_str())
            .collect();
        assert_eq!(
            named,
            vec!["System.Runtime", "System.Collections", "System.Linq"]
        );
        assert_eq!(view.references[0].version, "8.0.0.0");
    }

    #[test]
    fn reads_every_public_type_and_counts_the_rest() {
        let view = view_of("CsvStats.dll");

        assert_eq!(
            view.public_types,
            vec![
                "CsvStats.Measure",
                "CsvStats.IReader",
                "CsvStats.Summary",
                "CsvStats.Column",
            ]
        );
        assert!(
            view.type_count >= view.public_types.len(),
            "a compiler adds types of its own, and they are counted but not listed"
        );
    }

    #[test]
    fn a_library_has_no_entry_point_and_an_executable_does() {
        let library = view_of("CsvStats.dll");
        let program = view_of("CsvStats.Cli.dll");

        assert_eq!(library.entry_point, None, "a library is called into");
        assert_eq!(
            program.entry_point.as_deref(),
            Some("Program.<Main>$"),
            "the entry-point token names a MethodDef row, and the type it              belongs to is the last one whose method list reaches it"
        );
        assert!(
            program.public_types.is_empty(),
            "a console program's Program class is internal, so `no public              types` is the right answer rather than a failure to find any"
        );
        assert_eq!(program.name, "CsvStats.Cli");
        assert!(
            program
                .references
                .iter()
                .any(|one| one.name == "CsvStats" && one.version == "1.0.3.0"),
            "it references the library beside it"
        );
    }

    #[test]
    fn a_satellite_assembly_carries_the_culture_it_translates_into() {
        // The only assembly that has a culture is one holding
        // translations; the assembly it belongs to is neutral, and the
        // pair is what makes the field mean anything.
        let satellite = view_of("fr/CsvStats.resources.dll");
        let neutral = view_of("CsvStats.dll");

        assert_eq!(satellite.culture.as_deref(), Some("fr"));
        assert_eq!(satellite.name, "CsvStats.resources");
        assert_eq!(satellite.version, neutral.version);
        assert_eq!(neutral.culture, None);
        assert!(
            satellite.public_types.is_empty(),
            "a satellite holds resources, not types"
        );
    }

    #[test]
    fn tells_a_strong_named_assembly_from_one_that_is_not() {
        let signed = view_of("CsvStats.dll");
        let unsigned = view_of("CsvStats.Cli.dll");

        assert!(signed.strong_named);
        assert!(signed.attributes.contains(&"strong-name signed".to_owned()));
        assert!(!unsigned.strong_named);
        assert!(
            !unsigned
                .attributes
                .contains(&"strong-name signed".to_owned())
        );
    }

    #[test]
    fn names_the_tables_the_metadata_carries() {
        let view = view_of("CsvStats.dll");

        // Found by looking at the running application, which showed a
        // reader `table 0x28 (1)`. Every table a real assembly carries
        // has a name, and the fallback is for a table nobody has met.
        assert!(
            !view.tables.iter().any(|one| one.starts_with("table 0x")),
            "unnamed: {:?}",
            view.tables
                .iter()
                .filter(|one| one.starts_with("table 0x"))
                .collect::<Vec<&String>>()
        );

        assert!(
            view.tables
                .iter()
                .any(|one| one.starts_with("Assembly (1)"))
        );
        assert!(
            view.tables
                .iter()
                .any(|one| one.starts_with("AssemblyRef (3)"))
        );
        assert!(view.tables.iter().any(|one| one.starts_with("TypeDef (")));
        assert!(
            view.tables.iter().any(|one| one.starts_with("MethodDef (")),
            "reaching TypeDef at all means every table before it was sized right"
        );
    }

    #[test]
    fn presents_what_it_is_and_what_it_needs() {
        let data = DotnetassemblyCore.view(&sample("CsvStats.dll")).unwrap();

        let lines = DotnetassemblyPresentation.present(&data);

        assert!(lines[0].starts_with(".NET assembly CsvStats 1.0.3.0"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Built for .NETCoreApp,Version=v8.0"))
        );
        assert!(lines.iter().any(|line| line.contains("Strong-named")));
        assert!(lines.iter().any(|line| line.contains("CsvStats.Column")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("No entry point, so this is a library"))
        );
    }

    #[test]
    fn a_file_that_is_not_an_assembly_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.dll");
        std::fs::write(&path, b"MZ and then nothing of the sort whatsoever").unwrap();

        assert!(DotnetassemblyCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
