//! The application icon: renders the drawing into the committed `.ico`, and
//! reads icon resources back out of a Windows binary.
//!
//! Two jobs in one crate because they are two ends of the same thread. The
//! drawing is `assets/RepoSphereExplorer.svg`; `cargo run -p icon` rewrites
//! `assets/RepoSphereExplorer.ico` from it, so the committed asset is
//! reproducible rather than something a person once exported. The reader is
//! here rather than in each front end's test because both front ends ask the
//! same question of their own built binary, and one copy of the answer is
//! enough.
//!
//! Nothing links this crate into a shipped binary: the front ends take it as
//! a development dependency, for their tests.

use image::ExtendedColorType;
use image::codecs::ico::{IcoEncoder, IcoFrame};
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
    Ok(pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let colour = pixel.demultiply();
            [colour.red(), colour.green(), colour.blue(), colour.alpha()]
        })
        .collect())
}

/// Renders `drawing` into the bytes of an icon (`.ico`) file holding one
/// image per entry of [`SIZES`].
///
/// Every entry is a portable network graphic (PNG), which Windows has read at
/// any icon size since Vista, rather than the older device-independent bitmap
/// with its separate transparency mask.
///
/// # Errors
///
/// When `drawing` will not render, or the images will not encode.
pub fn ico(drawing: &str) -> Result<Vec<u8>, String> {
    let frames = SIZES
        .iter()
        .map(|&size| {
            let pixels = render(drawing, size)?;
            IcoFrame::as_png(&pixels, size, size, ExtendedColorType::Rgba8)
                .map_err(|err| format!("the {size} pixel image will not encode: {err}"))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut bytes = Vec::new();
    IcoEncoder::new(&mut bytes)
        .encode_images(&frames)
        .map_err(|err| format!("the icon directory will not encode: {err}"))?;
    Ok(bytes)
}

/// One image in an icon file, as its directory describes it.
pub struct Entry<'a> {
    /// Its width in pixels.
    pub width: u32,
    /// Its height in pixels.
    pub height: u32,
    /// The encoded image - a PNG, for every entry [`ico`] writes.
    pub payload: &'a [u8],
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
    use super::{COMMITTED, DRAWING, SIZES, entries, render};

    /// The committed icon, decoded entry by entry.
    fn committed() -> Vec<(u32, Vec<u8>)> {
        entries(COMMITTED)
            .expect("the committed icon is an icon file")
            .into_iter()
            .map(|entry| {
                assert_eq!(entry.width, entry.height, "every entry is square");
                let image =
                    image::load_from_memory_with_format(entry.payload, image::ImageFormat::Png)
                        .expect("every entry is a portable network graphic")
                        .to_rgba8();
                assert_eq!(
                    (image.width(), image.height()),
                    (entry.width, entry.height),
                    "the image is the size the directory claims"
                );
                (entry.width, image.into_raw())
            })
            .collect()
    }

    #[test]
    fn the_committed_icon_carries_every_size() {
        let sizes: Vec<u32> = committed().into_iter().map(|(size, _)| size).collect();
        assert_eq!(sizes, SIZES, "one entry per size, smallest first");
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
