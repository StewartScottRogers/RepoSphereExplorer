//! The application icon: renders the drawing into the committed `.ico` and
//! `.icns`, and reads icon resources back out of a Windows binary.
//!
//! Two jobs in one crate because they are two ends of the same thread. The
//! drawing is `assets/RepoSphereExplorer.svg`; `cargo run -p icon` rewrites
//! `assets/RepoSphereExplorer.ico` and `assets/RepoSphereExplorer.icns` from
//! it, so the committed assets are reproducible rather than something a
//! person once exported. The reader is here rather than in each front end's
//! test because both front ends ask the same question of their own built
//! binary, and one copy of the answer is enough.
//!
//! Nothing links this crate into a shipped binary: the front ends take it as
//! a development dependency, for their tests.

use object::LittleEndian;
use object::read::pe::{PeFile64, ResourceDirectory, ResourceNameOrId};
use resvg::{tiny_skia, usvg};

/// The square sizes the icon directory carries, smallest first. Windows
/// chooses among them: sixteen for a title bar and a listing, thirty-two for
/// the taskbar, forty-eight for Explorer's medium icons, and the larger three
/// for its larger views and for a high display scale.
pub const SIZES: [u32; 6] = [16, 32, 48, 64, 128, 256];

/// The drawing, as committed.
pub const DRAWING: &str = include_str!("../../../assets/RepoSphereExplorer.svg");

/// The icon directory, as committed - what
/// [`ico`] last wrote and what the two Windows binaries embed.
pub const COMMITTED: &[u8] = include_bytes!("../../../assets/RepoSphereExplorer.ico");

/// The four-character type and pixel size of each image an icon set
/// (`.icns`) carries, smallest first.
///
/// macOS picks by type, not by size, and the same number of pixels appears
/// twice under two types: `ic11` is the doubled sixteen a Retina display
/// draws where `icp4` would go, and `ic13`/`ic14` stand in the same relation
/// to `ic07`/`ic08`. Leaving a type out means a display scale falls back to a
/// blurred neighbour, so the whole set is written.
pub const ICNS_ENTRIES: [(&[u8; 4], u32); 10] = [
    (b"icp4", 16),
    (b"icp5", 32),
    (b"ic11", 32),
    (b"ic12", 64),
    (b"ic07", 128),
    (b"ic13", 256),
    (b"ic08", 256),
    (b"ic14", 512),
    (b"ic09", 512),
    (b"ic10", 1024),
];

/// The icon set, as committed - what [`icns`] last wrote and what the macOS
/// application bundle carries as `Contents/Resources/AppIcon.icns`.
pub const COMMITTED_ICNS: &[u8] = include_bytes!("../../../assets/RepoSphereExplorer.icns");

/// The length of the `BITMAPINFOHEADER` that opens a bitmap entry.
const BITMAP_HEADER: usize = 40;

/// The Windows resource type of a single icon image.
const RT_ICON: u16 = 3;

/// The Windows resource type of the directory naming the images of one icon.
const RT_GROUP_ICON: u16 = 14;

/// Renders `drawing` into a square image `size` pixels on a side, returned as
/// straight - not premultiplied - red, green, blue and alpha (RGBA) bytes.
///
/// # Errors
///
/// When `drawing` is not a scalable vector graphic (SVG) this crate can parse,
/// or `size` is one no pixel buffer can be made for.
pub fn render(drawing: &str, size: u32) -> Result<Vec<u8>, String> {
    Ok(painted(drawing, size)?
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let colour = pixel.demultiply();
            [colour.red(), colour.green(), colour.blue(), colour.alpha()]
        })
        .collect())
}

/// Paints `drawing` onto a square pixel buffer `size` pixels on a side.
fn painted(drawing: &str, size: u32) -> Result<tiny_skia::Pixmap, String> {
    let tree = usvg::Tree::from_str(drawing, &usvg::Options::default())
        .map_err(|err| format!("the drawing does not parse: {err}"))?;
    let mut pixmap = tiny_skia::Pixmap::new(size, size)
        .ok_or_else(|| format!("{size} by {size} pixels is not a size an image can be"))?;
    let edge = f32::from(
        u16::try_from(size).map_err(|_| format!("{size} pixels is wider than any icon"))?,
    );
    let scale = edge / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}

/// Renders `drawing` into a square portable network graphic (PNG) `size`
/// pixels on a side.
///
/// # Errors
///
/// When `drawing` will not render, or the image will not encode.
pub fn png(drawing: &str, size: u32) -> Result<Vec<u8>, String> {
    painted(drawing, size)?
        .encode_png()
        .map_err(|err| format!("the {size} pixel image will not encode: {err}"))
}

/// Renders `drawing` into the bytes of an icon (`.ico`) file holding one
/// image per entry of [`SIZES`].
///
/// Every entry is a device-independent bitmap (DIB). An icon may instead hold
/// a portable network graphic (PNG) per entry, which is smaller and which
/// Windows has read since Vista - but only through the shell and the newer
/// imaging calls. The graphics device interface (GDI) path underneath
/// `System.Drawing`, which the shortcut property pages, some installers and a
/// good deal of Windows tooling still sit on, draws a picture entry as noise
/// at the small sizes and refuses it outright at the large ones. Measured on
/// this drawing: as pictures, every size failed; as bitmaps, every size draws.
/// The largest entry costs a quarter of a megabyte, which is the price of an
/// icon that every reader can read.
///
/// The directory is written here rather than by a library because the one in
/// the tree writes pictures only, and the format is a six byte header, a
/// sixteen byte entry apiece, and the payloads.
///
/// # Errors
///
/// When `drawing` will not render, or an image is too large for an entry.
pub fn ico(drawing: &str) -> Result<Vec<u8>, String> {
    let payloads = SIZES
        .iter()
        .map(|&size| {
            let pixels = render(drawing, size)?;
            let side = usize::try_from(size)
                .map_err(|_| format!("{size} pixels is wider than this machine can address"))?;
            bitmap_entry(&pixels, side)
        })
        .collect::<Result<Vec<_>, String>>()?;

    let count = u16::try_from(SIZES.len()).map_err(|_| "too many sizes for one icon".to_owned())?;
    let mut bytes = Vec::new();
    // Zero, then one for an icon rather than a cursor, then the count.
    bytes.extend_from_slice(&[0, 0, 1, 0]);
    bytes.extend_from_slice(&count.to_le_bytes());
    let header = 6 + 16 * u32::from(count);
    let mut offset = header;
    for (&size, payload) in SIZES.iter().zip(&payloads) {
        let length = u32::try_from(payload.len())
            .map_err(|_| format!("the {size} pixel image is too long for an icon entry"))?;
        // A side is one byte, so 256 - the largest an icon can be - is
        // written as zero.
        let side = u8::try_from(size).unwrap_or(0);
        bytes.extend_from_slice(&[side, side, 0, 0]);
        // One colour plane, and the thirty-two bits a pixel the bitmap holds.
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&32u16.to_le_bytes());
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(&offset.to_le_bytes());
        offset = offset
            .checked_add(length)
            .ok_or_else(|| "the icon is longer than its directory can address".to_owned())?;
    }
    for payload in payloads {
        bytes.extend_from_slice(&payload);
    }
    Ok(bytes)
}

/// Encodes `pixels` - straight RGBA, top row first - as the device-
/// independent bitmap (DIB) an icon entry holds.
///
/// A `BITMAPINFOHEADER` whose height is doubled to cover the mask that
/// follows the colours, then rows of blue, green, red and alpha from the
/// bottom up, then the mask.
///
/// The mask is one bit a pixel - rows padded to four bytes, as every bitmap
/// row is - and is what a reader with no alpha channel cut the shape out
/// with. Every pixel here carries its own alpha, so the mask is zero
/// throughout, meaning "draw it", and the alpha channel does the work.
fn bitmap_entry(pixels: &[u8], side: usize) -> Result<Vec<u8>, String> {
    let width = u32::try_from(side).map_err(|_| format!("{side} pixels is wider than any icon"))?;
    let height = width
        .checked_mul(2)
        .ok_or_else(|| format!("{side} pixels is taller than any icon"))?;
    let mask_row = side.div_ceil(8).div_ceil(4) * 4;
    let mut bytes = Vec::with_capacity(BITMAP_HEADER + side * side * 4 + side * mask_row);
    for field in [
        u32::try_from(BITMAP_HEADER).map_err(|_| "the header is not a header".to_owned())?,
        width,
        height,
    ] {
        bytes.extend_from_slice(&field.to_le_bytes());
    }
    bytes.extend_from_slice(&1u16.to_le_bytes()); // colour planes
    bytes.extend_from_slice(&32u16.to_le_bytes()); // bits a pixel
    // No compression, and no stated image length: a reader of an uncompressed
    // bitmap works it out from the other fields, and the crates that write
    // these leave it zero.
    bytes.extend_from_slice(&[0; 24]);

    for row in (0..side).rev() {
        let start = row * side * 4;
        for pixel in pixels[start..start + side * 4].as_chunks::<4>().0 {
            bytes.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    bytes.resize(bytes.len() + side * mask_row, 0);
    Ok(bytes)
}

/// Renders `drawing` into the bytes of an icon set (`.icns`) file holding one
/// image per entry of [`ICNS_ENTRIES`].
///
/// Every entry is a portable network graphic (PNG), which is what macOS has
/// read inside an icon set since 10.7 and what `iconutil` itself writes. The
/// older run-length encoded types the format began with are not written: they
/// carry no alpha channel of their own, and nothing that reads an icon set on
/// a system this application supports needs them.
///
/// The file is a four byte magic, the length of the whole file, and then one
/// record apiece: a four character type, the length of the record including
/// those eight bytes, and the image. Written here rather than by a library
/// for the same reason [`ico`] is - the format is shorter than the argument
/// for taking a dependency on it.
///
/// # Errors
///
/// When `drawing` will not render, or an image is too large for an entry.
pub fn icns(drawing: &str) -> Result<Vec<u8>, String> {
    let mut records = Vec::new();
    for (kind, size) in ICNS_ENTRIES {
        let payload = png(drawing, size)?;
        let length = u32::try_from(payload.len() + ICNS_RECORD_HEADER)
            .map_err(|_| format!("the {size} pixel image is too long for an icon set entry"))?;
        records.extend_from_slice(kind);
        records.extend_from_slice(&length.to_be_bytes());
        records.extend_from_slice(&payload);
    }
    let total = u32::try_from(records.len() + ICNS_RECORD_HEADER)
        .map_err(|_| "the icon set is longer than its header can address".to_owned())?;
    let mut bytes = Vec::with_capacity(records.len() + ICNS_RECORD_HEADER);
    bytes.extend_from_slice(ICNS_MAGIC);
    bytes.extend_from_slice(&total.to_be_bytes());
    bytes.extend_from_slice(&records);
    Ok(bytes)
}

/// The magic that opens an icon set file, and the type of its outermost
/// record.
const ICNS_MAGIC: &[u8; 4] = b"icns";

/// The four character type and the length that open every icon set record,
/// the outermost one included.
const ICNS_RECORD_HEADER: usize = 8;

/// One image in an icon set, as its record describes it.
pub struct IcnsEntry<'a> {
    /// Its four character type, which is what macOS chooses an image by.
    pub kind: [u8; 4],
    /// The encoded image.
    pub payload: &'a [u8],
}

/// The images `bytes` holds, read from its icon set records.
///
/// # Errors
///
/// When `bytes` is not an icon set file, or one of its records runs past the
/// end of it.
pub fn icns_entries(bytes: &[u8]) -> Result<Vec<IcnsEntry<'_>>, String> {
    let header = bytes
        .get(..ICNS_RECORD_HEADER)
        .ok_or_else(|| "shorter than an icon set header".to_owned())?;
    if &header[..4] != ICNS_MAGIC {
        return Err("not an icon set file: the header does not say so".to_owned());
    }
    let total = usize::try_from(u32::from_be_bytes([
        header[4], header[5], header[6], header[7],
    ]))
    .map_err(|_| "the icon set is longer than this machine can address".to_owned())?;
    if total != bytes.len() {
        return Err(format!(
            "the icon set header claims {total} bytes but the file is {}",
            bytes.len()
        ));
    }

    let mut found = Vec::new();
    let mut at = ICNS_RECORD_HEADER;
    while at < bytes.len() {
        let record = bytes
            .get(at..at + ICNS_RECORD_HEADER)
            .ok_or_else(|| format!("the record at {at} runs past the end of the file"))?;
        let length = usize::try_from(u32::from_be_bytes([
            record[4], record[5], record[6], record[7],
        ]))
        .map_err(|_| format!("the record at {at} is longer than this machine can address"))?;
        if length < ICNS_RECORD_HEADER {
            return Err(format!("the record at {at} is shorter than its own header"));
        }
        let end = at
            .checked_add(length)
            .ok_or_else(|| format!("the record at {at} runs past the end of the file"))?;
        let payload = bytes
            .get(at + ICNS_RECORD_HEADER..end)
            .ok_or_else(|| format!("the record at {at} runs past the end of the file"))?;
        found.push(IcnsEntry {
            kind: [record[0], record[1], record[2], record[3]],
            payload,
        });
        at = end;
    }
    Ok(found)
}

/// How an icon entry's image is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// A device-independent bitmap (DIB), which every Windows imaging call
    /// reads, back to the ones that predate the alpha channel.
    Bitmap,
    /// A portable network graphic (PNG), which Windows has read inside an
    /// icon since Vista and the older calls have not.
    Picture,
}

/// One image in an icon file, as its directory describes it.
pub struct Entry<'a> {
    /// Its width in pixels.
    pub width: u32,
    /// Its height in pixels.
    pub height: u32,
    /// The encoded image, in whichever of the two encodings this entry uses.
    pub payload: &'a [u8],
}

impl Entry<'_> {
    /// How this entry's image is encoded, taken from the payload's first
    /// bytes: a picture names itself, and nothing else in an icon does.
    #[must_use]
    pub fn encoding(&self) -> Encoding {
        if self.payload.starts_with(b"\x89PNG") {
            Encoding::Picture
        } else {
            Encoding::Bitmap
        }
    }
}

/// The images `bytes` holds, read from its icon directory.
///
/// # Errors
///
/// When `bytes` is not an icon file, or one of its entries points outside it.
pub fn entries(bytes: &[u8]) -> Result<Vec<Entry<'_>>, String> {
    let header = bytes
        .get(..6)
        .ok_or_else(|| "shorter than an icon directory header".to_owned())?;
    if header[..4] != [0, 0, 1, 0] {
        return Err("not an icon file: the directory header does not say so".to_owned());
    }
    let count = usize::from(u16::from_le_bytes([header[4], header[5]]));
    (0..count)
        .map(|index| {
            let at = 6 + index * 16;
            let entry = bytes
                .get(at..at + 16)
                .ok_or_else(|| format!("entry {index} runs past the end of the file"))?;
            // A zero side means 256: the directory keeps each in one byte.
            let side = |byte: u8| if byte == 0 { 256 } else { u32::from(byte) };
            let length = usize::try_from(u32::from_le_bytes([
                entry[8], entry[9], entry[10], entry[11],
            ]))
            .map_err(|_| format!("entry {index} is longer than this machine can address"))?;
            let offset = usize::try_from(u32::from_le_bytes([
                entry[12], entry[13], entry[14], entry[15],
            ]))
            .map_err(|_| format!("entry {index} starts past what this machine can address"))?;
            let end = offset
                .checked_add(length)
                .ok_or_else(|| format!("entry {index} runs past the end of the file"))?;
            let payload = bytes
                .get(offset..end)
                .ok_or_else(|| format!("entry {index} runs past the end of the file"))?;
            Ok(Entry {
                width: side(entry[0]),
                height: side(entry[1]),
                payload,
            })
        })
        .collect()
}

/// What a Windows binary's resource directory holds by way of icons.
pub struct IconResources<'a> {
    /// Each icon image, in the order the resource directory lists them. The
    /// bytes are the icon file's own entry payloads, copied in by the
    /// resource compiler.
    pub images: Vec<&'a [u8]>,
    /// How many icon groups the binary carries. One is what an application
    /// with a single icon has; Windows shows the first by number.
    pub groups: usize,
}

/// The icon resources the Windows binary `pe` carries.
///
/// # Errors
///
/// When `pe` is not a 64-bit portable executable, or carries no resource
/// directory at all.
pub fn icon_resources(pe: &[u8]) -> Result<IconResources<'_>, String> {
    let file = PeFile64::parse(pe).map_err(|err| format!("not a Windows binary: {err}"))?;
    let sections = file.section_table();
    let directories = file.data_directories();
    let directory = directories
        .get(object::pe::IMAGE_DIRECTORY_ENTRY_RESOURCE)
        .ok_or_else(|| "the binary carries no resource directory".to_owned())?;
    // Every leaf names its payload by the address it will have once loaded;
    // subtracting the directory's own turns that back into an offset into
    // the bytes below.
    let base = directory.virtual_address.get(LittleEndian);
    let bytes = directory
        .data(pe, &sections)
        .map_err(|err| format!("the resource directory will not read: {err}"))?;
    let resources = ResourceDirectory::new(bytes);
    let root = resources
        .root()
        .map_err(|err| format!("the resource directory will not read: {err}"))?;

    let mut found = IconResources {
        images: Vec::new(),
        groups: 0,
    };
    for kind in root.entries {
        let ResourceNameOrId::Id(id) = kind.name_or_id() else {
            continue;
        };
        if id != RT_ICON && id != RT_GROUP_ICON {
            continue;
        }
        // Type, then name or number, then language: three levels, and the
        // payload hangs off the third.
        let names = table(*kind, resources)?;
        for name in names {
            for language in table(*name, resources)? {
                let leaf = language
                    .data(resources)
                    .map_err(|err| format!("a resource entry will not read: {err}"))?
                    .data()
                    .ok_or_else(|| "a resource language entry holds no payload".to_owned())?;
                if id == RT_GROUP_ICON {
                    found.groups += 1;
                    continue;
                }
                let at = usize::try_from(
                    leaf.offset_to_data
                        .get(LittleEndian)
                        .checked_sub(base)
                        .ok_or_else(|| {
                            "an icon points outside the resource directory".to_owned()
                        })?,
                )
                .map_err(|_| "an icon starts past what this machine can address".to_owned())?;
                let length = usize::try_from(leaf.size.get(LittleEndian))
                    .map_err(|_| "an icon is longer than this machine can address".to_owned())?;
                let end = at
                    .checked_add(length)
                    .ok_or_else(|| "an icon runs past the resource directory".to_owned())?;
                found.images.push(
                    bytes
                        .get(at..end)
                        .ok_or_else(|| "an icon runs past the resource directory".to_owned())?,
                );
            }
        }
    }
    Ok(found)
}

/// The subtable `entry` points at.
fn table(
    entry: object::pe::ImageResourceDirectoryEntry,
    resources: ResourceDirectory<'_>,
) -> Result<&[object::pe::ImageResourceDirectoryEntry], String> {
    entry
        .data(resources)
        .map_err(|err| format!("a resource entry will not read: {err}"))?
        .table()
        .map(|table| table.entries)
        .ok_or_else(|| "a resource entry that should hold a table holds a payload".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        BITMAP_HEADER, COMMITTED, COMMITTED_ICNS, DRAWING, Encoding, Entry, ICNS_ENTRIES, SIZES,
        entries, icns_entries, render,
    };

    /// One entry's pixels, straight RGBA with the top row first, whichever
    /// way it is encoded.
    fn decode(entry: &Entry<'_>) -> Vec<u8> {
        let side = usize::try_from(entry.width).expect("an icon fits in this machine");
        match entry.encoding() {
            Encoding::Picture => {
                image::load_from_memory_with_format(entry.payload, image::ImageFormat::Png)
                    .expect("a picture entry is a portable network graphic")
                    .to_rgba8()
                    .into_raw()
            }
            // The colours are rows of blue, green, red and alpha from the
            // bottom up; the mask after them says nothing this drawing's own
            // alpha channel does not already say.
            Encoding::Bitmap => {
                let colours = &entry.payload[BITMAP_HEADER..];
                let mut pixels = vec![0u8; side * side * 4];
                for row in 0..side {
                    let from = (side - 1 - row) * side * 4;
                    for (column, bgra) in colours[from..from + side * 4]
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .enumerate()
                    {
                        let at = (row * side + column) * 4;
                        pixels[at..at + 4].copy_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
                    }
                }
                pixels
            }
        }
    }

    /// The committed icon, decoded entry by entry.
    fn committed() -> Vec<(u32, Vec<u8>)> {
        entries(COMMITTED)
            .expect("the committed icon is an icon file")
            .into_iter()
            .map(|entry| {
                assert_eq!(entry.width, entry.height, "every entry is square");
                let pixels = decode(&entry);
                let side = usize::try_from(entry.width).expect("an icon fits in this machine");
                assert_eq!(
                    pixels.len(),
                    side * side * 4,
                    "{}: the image is the size the directory claims",
                    entry.width
                );
                (entry.width, pixels)
            })
            .collect()
    }

    #[test]
    fn the_committed_icon_carries_every_size() {
        let sizes: Vec<u32> = committed().into_iter().map(|(size, _)| size).collect();
        assert_eq!(sizes, SIZES, "one entry per size, smallest first");
    }

    /// The encoding is not a detail. The first version of this asset wrote
    /// every entry as a picture; Explorer showed it perfectly, and everything
    /// sitting on the older imaging calls drew noise at the small sizes and
    /// threw at the large ones. This is the check that would have caught it.
    #[test]
    fn every_entry_is_a_bitmap() {
        for entry in entries(COMMITTED).expect("the committed icon is an icon file") {
            assert_eq!(
                entry.encoding(),
                Encoding::Bitmap,
                "the {} pixel entry is a picture, which the older Windows \
                 imaging calls cannot draw; run `cargo run -p icon`",
                entry.width
            );
        }
    }

    /// The committed icon has to be the committed drawing, or the asset is
    /// whatever somebody exported once and the drawing is decoration.
    ///
    /// Compared as a mean difference rather than byte for byte: the
    /// rasteriser uses wide arithmetic whose rounding can differ by a step
    /// between one processor and another, so exact equality would fail on a
    /// machine that had drawn the very same picture.
    #[test]
    fn the_committed_icon_is_the_committed_drawing() {
        for (size, pixels) in committed() {
            let fresh = render(DRAWING, size).expect("the drawing renders");
            assert_eq!(pixels.len(), fresh.len(), "{size}: the same pixel count");
            let difference: u64 = pixels
                .iter()
                .zip(&fresh)
                .map(|(left, right)| u64::from(left.abs_diff(*right)))
                .sum();
            let channels = u64::try_from(pixels.len()).expect("an icon fits in this machine");
            assert!(
                difference <= channels,
                "{size}: the committed image is not the drawing \
                 (mean difference {difference} over {channels} channels); \
                 run `cargo run -p icon`"
            );
        }
    }

    /// The committed icon set, decoded record by record: the four character
    /// type, the side it claims, and its pixels as straight RGBA.
    fn committed_icon_set() -> Vec<([u8; 4], u32, Vec<u8>)> {
        icns_entries(COMMITTED_ICNS)
            .expect("the committed icon set is an icon set file")
            .into_iter()
            .map(|entry| {
                let picture =
                    image::load_from_memory_with_format(entry.payload, image::ImageFormat::Png)
                        .expect("every entry is a portable network graphic")
                        .to_rgba8();
                assert_eq!(
                    picture.width(),
                    picture.height(),
                    "{}: every entry is square",
                    String::from_utf8_lossy(&entry.kind)
                );
                (entry.kind, picture.width(), picture.into_raw())
            })
            .collect()
    }

    #[test]
    fn the_committed_icon_set_carries_every_type() {
        let found: Vec<([u8; 4], u32)> = committed_icon_set()
            .into_iter()
            .map(|(kind, size, _)| (kind, size))
            .collect();
        let wanted: Vec<([u8; 4], u32)> = ICNS_ENTRIES
            .iter()
            .map(|&(kind, size)| (*kind, size))
            .collect();
        assert_eq!(
            found, wanted,
            "one record per type, smallest first, each holding the size its type names; \
             run `cargo run -p icon`"
        );
    }

    /// The same check as [`the_committed_icon_is_the_committed_drawing`], for
    /// the other asset: the icon set has to be the committed drawing, or the
    /// macOS bundle carries a picture nobody can regenerate.
    #[test]
    fn the_committed_icon_set_is_the_committed_drawing() {
        for (kind, size, pixels) in committed_icon_set() {
            let name = String::from_utf8_lossy(&kind).into_owned();
            let fresh = render(DRAWING, size).expect("the drawing renders");
            assert_eq!(pixels.len(), fresh.len(), "{name}: the same pixel count");
            let difference: u64 = pixels
                .iter()
                .zip(&fresh)
                .map(|(left, right)| u64::from(left.abs_diff(*right)))
                .sum();
            let channels = u64::try_from(pixels.len()).expect("an icon fits in this machine");
            assert!(
                difference <= channels,
                "{name}: the committed image is not the drawing \
                 (mean difference {difference} over {channels} channels); \
                 run `cargo run -p icon`"
            );
        }
    }

    /// A drawing that rendered to nothing would pass every other check here.
    #[test]
    fn the_drawing_paints_a_solid_picture() {
        let pixels = render(DRAWING, 32).expect("the drawing renders");
        let opaque = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[3] == 255)
            .count();
        assert!(
            opaque > 32 * 32 / 2,
            "the icon should fill its square, not float in it: {opaque} opaque pixels of 1024"
        );
    }
}
