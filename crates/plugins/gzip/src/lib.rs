//! gzip file type plugin: core and presentation halves.
//!
//! A gzip member is a header, deflated bytes, and a trailer. The header
//! can carry the original file name, the time it was written, a comment
//! and which system wrote it; the trailer carries the uncompressed size
//! and a checksum. All of that is readable without decompressing
//! anything, which is the point of this plugin.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["gz", "tgz"];

/// The two bytes a gzip member opens with.
const MAGIC: &[u8] = &[0x1f, 0x8b];

/// The only compression method gzip has ever had.
const DEFLATE: u8 = 8;

/// The systems a writer may name, by the number it writes.
const SYSTEMS: &[(u8, &str)] = &[
    (0, "FAT"),
    (1, "Amiga"),
    (2, "VMS"),
    (3, "Unix"),
    (4, "VM/CMS"),
    (5, "Atari TOS"),
    (6, "HPFS"),
    (7, "Macintosh"),
    (8, "Z-System"),
    (9, "CP/M"),
    (10, "TOPS-20"),
    (11, "NTFS"),
    (12, "QDOS"),
    (13, "Acorn RISCOS"),
    (255, "unstated"),
];

/// View data produced by [`GzipCore::view`].
///
/// Not `Eq`: the ratio is a float, and two ratios being equal is not a
/// question worth answering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GzipView {
    /// The name the file had before it was compressed, when the header
    /// carries one.
    pub original_name: Option<String>,
    /// When it was last written, as seconds since the epoch. Zero means
    /// the writer did not say.
    pub modified: Option<u32>,
    /// What the extra-flags byte says about the compression level.
    pub level: String,
    /// The system that wrote it.
    pub operating_system: String,
    /// The comment in the header, when there is one.
    pub comment: Option<String>,
    /// The uncompressed size from the trailer, modulo four gigabytes -
    /// which is all the trailer has room for.
    pub uncompressed_size: u32,
    /// The compressed size, which is the file itself.
    pub compressed_size: u64,
    /// How much smaller it got, as a percentage.
    pub saved_percent: f64,
    /// The checksum of the uncompressed bytes, as hexadecimal.
    pub checksum: String,
    /// Notes about what the header does not say, each with what it costs.
    pub not_stated: Vec<String>,
}

/// The bytes of a null-terminated field starting at `at`.
fn terminated(bytes: &[u8], at: usize) -> Option<(String, usize)> {
    let end = bytes.get(at..)?.iter().position(|byte| *byte == 0)?;
    let text = String::from_utf8_lossy(&bytes[at..at + end]).into_owned();
    Some((text, at + end + 1))
}

/// Whether `prefix` opens like a gzip member.
fn looks_like_it(prefix: &[u8]) -> bool {
    prefix.starts_with(MAGIC) && prefix.get(2) == Some(&DEFLATE)
}

/// Everything [`GzipView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<GzipView> {
    let mut handle = std::fs::File::open(path)?;
    let compressed_size = handle.metadata()?.len();

    // The header is at most a few hundred bytes unless the writer put a
    // novel in the comment, and nothing past it needs reading.
    let mut head = vec![0u8; 4096];
    let got = handle.read(&mut head)?;
    head.truncate(got);
    if !looks_like_it(&head) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a gzip member",
        ));
    }

    let flags = *head.get(3).ok_or_else(short)?;
    let modified = u32::from_le_bytes(head.get(4..8).ok_or_else(short)?.try_into().map_err(bad)?);
    let extra_flags = *head.get(8).ok_or_else(short)?;
    let system = *head.get(9).ok_or_else(short)?;

    let mut at = 10usize;
    if flags & 0x04 != 0 {
        // An extra field, whose own length comes first.
        let length = u16::from_le_bytes(
            head.get(at..at + 2)
                .ok_or_else(short)?
                .try_into()
                .map_err(bad)?,
        );
        at += 2 + usize::from(length);
    }
    let original_name = if flags & 0x08 != 0 {
        let (name, next) = terminated(&head, at).ok_or_else(short)?;
        at = next;
        Some(name)
    } else {
        None
    };
    let comment = if flags & 0x10 != 0 {
        let (said, next) = terminated(&head, at).ok_or_else(short)?;
        at = next;
        Some(said)
    } else {
        None
    };
    let _ = at;

    // The trailer is the last eight bytes: a checksum and the size.
    let mut trailer = [0u8; 8];
    handle.seek(SeekFrom::End(-8))?;
    handle.read_exact(&mut trailer)?;
    let checksum = u32::from_le_bytes(trailer[..4].try_into().map_err(bad)?);
    let uncompressed_size = u32::from_le_bytes(trailer[4..].try_into().map_err(bad)?);

    let mut not_stated = Vec::new();
    if original_name.is_none() {
        not_stated.push(
            "no original name, so what this was called before it was \
             compressed is not recoverable from the file"
                .to_owned(),
        );
    }
    if modified == 0 {
        not_stated.push(
            "no modification time, which is what a writer records when it \
             is asked to be reproducible"
                .to_owned(),
        );
    }

    let saved_percent = if uncompressed_size == 0 {
        0.0
    } else {
        let original = f64::from(uncompressed_size);
        #[expect(
            clippy::cast_precision_loss,
            reason = "a size beyond a double's exact range is off by less than a byte in \
                      a percentage"
        )]
        let stored = compressed_size as f64;
        ((original - stored) / original * 1000.0).round() / 10.0
    };

    Ok(GzipView {
        original_name,
        modified: (modified != 0).then_some(modified),
        level: match extra_flags {
            2 => "asked for the smallest output".to_owned(),
            4 => "asked for the fastest compression".to_owned(),
            0 => "unstated".to_owned(),
            other => format!("extra-flags byte {other}"),
        },
        operating_system: SYSTEMS
            .iter()
            .find(|(number, _)| *number == system)
            .map_or_else(
                || format!("system {system}"),
                |(_, said)| (*said).to_owned(),
            ),
        comment,
        uncompressed_size,
        compressed_size,
        saved_percent,
        checksum: format!("{checksum:08x}"),
        not_stated,
    })
}

/// The error for a member that stops before its header does.
fn short() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "the header stops part-way")
}

/// The error for bytes that will not fit where they should.
fn bad<E: std::fmt::Display>(err: E) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err.to_string())
}

/// The gzip plugin's core half.
#[derive(Debug, Default)]
pub struct GzipCore;

impl PluginCore for GzipCore {
    fn name(&self) -> &'static str {
        "gzip"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_it(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(bad)
    }
}

/// The gzip plugin's presentation half.
#[derive(Debug, Default)]
pub struct GzipPresentation;

impl PluginPresentation for GzipPresentation {
    fn name(&self) -> &'static str {
        "gzip"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "GZ",
            tint: 0x0079_8b3c,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: GzipView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "gzip: {} byte(s) from {}, {}% saved",
            view.compressed_size, view.uncompressed_size, view.saved_percent
        )];
        if let Some(name) = &view.original_name {
            lines.push(format!("Was called {name}"));
        }
        if let Some(when) = view.modified {
            lines.push(format!("Written at {when} seconds since the epoch"));
        }
        lines.push(format!("Compressor {}", view.level));
        lines.push(format!("Written on {}", view.operating_system));
        lines.push(format!("Checksum {}", view.checksum));
        if let Some(said) = &view.comment {
            lines.push("Comment:".to_owned());
            lines.push(format!("  {said}"));
        }
        if !view.not_stated.is_empty() {
            lines.push("The header leaves these out:".to_owned());
            for said in &view.not_stated {
                lines.push(format!("  {said}"));
            }
        }
        lines.push(
            "The size in the trailer is modulo four gigabytes, so a larger \
             member reports the remainder."
                .to_owned(),
        );
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{GzipCore, GzipPresentation, GzipView, looks_like_it};
    use plugin_api::{PluginCore, PluginPresentation};

    fn sample(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/gzip")
            .join(name)
    }

    fn view_of(name: &str) -> GzipView {
        serde_json::from_value(GzipCore.view(&sample(name)).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&GzipCore),
            PluginPresentation::extensions(&GzipPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn sniffs_the_magic_and_the_method() {
        assert!(looks_like_it(&[0x1f, 0x8b, 0x08, 0x00]));
        assert!(
            !looks_like_it(&[0x1f, 0x8b, 0x09, 0x00]),
            "deflate is the only method gzip has ever had"
        );
        assert!(!looks_like_it(b"PK\x03\x04"));
        assert!(!looks_like_it(b""));
    }

    #[test]
    fn reads_the_name_the_time_and_the_comment() {
        let view = view_of("readings.csv.gz");

        assert_eq!(view.original_name.as_deref(), Some("readings.csv"));
        assert!(view.modified.is_some());
        assert!(
            view.comment
                .as_deref()
                .is_some_and(|said| said.contains("the floor"))
        );
        assert!(view.not_stated.is_empty(), "this member states all of it");
    }

    #[test]
    fn reads_the_sizes_and_the_ratio_from_the_trailer() {
        let view = view_of("readings.csv.gz");

        assert!(u64::from(view.uncompressed_size) > view.compressed_size);
        assert!(
            view.saved_percent > 50.0,
            "comma separated text compresses well"
        );
        assert_eq!(view.checksum.len(), 8);
    }

    #[test]
    fn reads_the_level_and_the_system() {
        let view = view_of("readings.csv.gz");

        assert_eq!(view.level, "asked for the smallest output");
        assert_eq!(view.operating_system, "Unix");
    }

    #[test]
    fn names_what_a_bare_header_leaves_out() {
        // A member with no name and no time: the flags byte is zero and
        // so are the four time bytes.
        let mut bytes = vec![0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0, 3];
        bytes.extend_from_slice(&[0x4b, 0x04, 0x00]);
        bytes.extend_from_slice(&0x8843_d7f2u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        let path = std::env::temp_dir().join("bare.gz");
        std::fs::write(&path, &bytes).unwrap();

        let view: GzipView = serde_json::from_value(GzipCore.view(&path).unwrap()).unwrap();
        assert_eq!(view.not_stated.len(), 2);
        assert!(
            view.not_stated
                .iter()
                .any(|said| said.contains("original name"))
        );
        assert!(
            view.not_stated
                .iter()
                .any(|said| said.contains("reproducible"))
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_compressed_archive_is_still_read_as_gzip() {
        let view = view_of("readings.tar.gz");

        assert_eq!(view.original_name.as_deref(), Some("readings.tar"));
        assert!(
            view.uncompressed_size >= 10_240,
            "the archive inside is padded to blocks"
        );
    }

    #[test]
    fn presents_the_comment_and_the_trailer_caveat() {
        let data = GzipCore.view(&sample("readings.csv.gz")).unwrap();

        let lines = GzipPresentation.present(&data);

        assert!(lines[0].starts_with("gzip:"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Was called readings.csv"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("modulo four gigabytes"))
        );
    }

    #[test]
    fn a_file_that_is_not_gzip_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.gz");
        std::fs::write(&path, b"not compressed at all").unwrap();

        assert!(GzipCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
