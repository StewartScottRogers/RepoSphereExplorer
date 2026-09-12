//! Zstandard file type plugin: core and presentation halves.
//!
//! A Zstandard file is one or more frames, each opening with a header
//! that says how much memory a reader will need, whether the frame was
//! built against a dictionary, and - when the writer chose to say - how
//! big the content will be. All of that is readable without
//! decompressing anything, which is what this plugin does.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["zst", "zstd", "tzst"];

/// The four bytes a frame opens with, little-endian.
const MAGIC: u32 = 0xFD2F_B528;

/// A skippable frame's magic is any of these sixteen.
const SKIPPABLE_FIRST: u32 = 0x184D_2A50;

/// The four bytes a dictionary opens with, little-endian.
const DICTIONARY_MAGIC: u32 = 0xEC30_A437;

/// One frame of the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    /// `content` for a real frame, `skippable` for one a reader steps
    /// over without looking inside.
    pub kind: String,
    /// How much memory a reader has to keep to decompress it, in bytes.
    pub window_size: u64,
    /// The size the frame says it will decompress to, when it says.
    pub content_size: Option<u64>,
    /// The identifier of the dictionary it needs, when it needs one.
    pub dictionary_id: Option<u32>,
    /// Whether a checksum follows the frame's blocks.
    pub checksum: bool,
    /// How many bytes the frame occupies in the file.
    pub compressed_size: u64,
}

/// View data produced by [`ZstdCore::view`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZstdView {
    /// `frames` for compressed content, `dictionary` for the dictionary
    /// such content is compressed against.
    pub content: String,
    /// The identifier this file *provides*, when it is a dictionary - the
    /// number a frame names to say it needs this one.
    pub dictionary_provides: Option<u32>,
    /// Every frame, in order.
    pub frames: Vec<Frame>,
    /// The file's own size.
    pub compressed_size: u64,
    /// What the frames together say they decompress to, when all of them
    /// say.
    pub content_size: Option<u64>,
    /// How much smaller it got, when the content size is known.
    pub saved_percent: Option<f64>,
    /// Frames needing a dictionary, which will not decompress without
    /// the very one they name.
    pub needs_a_dictionary: Vec<String>,
    /// Frames that do not say how big they will be, which a reader has
    /// to allocate for blind.
    pub unstated_sizes: usize,
}

/// A reader over the bytes, which stops rather than panicking.
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

    /// A little-endian unsigned number of `width` bytes.
    fn number(&mut self, width: usize) -> Option<u64> {
        let end = self.at.checked_add(width)?;
        let slice = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(slice.iter().enumerate().fold(0u64, |value, (shift, byte)| {
            value | (u64::from(*byte) << (shift * 8))
        }))
    }

    /// Steps over `count` bytes.
    fn skip(&mut self, count: u64) -> Option<()> {
        let count = usize::try_from(count).ok()?;
        self.at = self
            .at
            .checked_add(count)
            .filter(|at| *at <= self.bytes.len())?;
        Some(())
    }
}

/// Reads one frame, leaving the reader at the frame after it.
fn frame_at(reader: &mut Reader) -> Option<Frame> {
    let start = reader.at;
    let magic = u32::try_from(reader.number(4)?).ok()?;

    if (SKIPPABLE_FIRST..=SKIPPABLE_FIRST + 15).contains(&magic) {
        // A skippable frame says its own length and nothing else; a
        // reader steps over it without looking inside.
        let length = reader.number(4)?;
        reader.skip(length)?;
        return Some(Frame {
            kind: "skippable".to_owned(),
            window_size: 0,
            content_size: None,
            dictionary_id: None,
            checksum: false,
            compressed_size: (reader.at - start) as u64,
        });
    }
    if magic != MAGIC {
        return None;
    }

    let descriptor = reader.byte()?;
    let content_size_flag = descriptor >> 6;
    let single_segment = descriptor & 0x20 != 0;
    let checksum = descriptor & 0x04 != 0;
    let dictionary_flag = descriptor & 0x03;

    // With a single segment, the window is the content, and no window
    // descriptor is written at all.
    let window_size = if single_segment {
        0
    } else {
        let byte = reader.byte()?;
        let exponent = u32::from(byte >> 3);
        let mantissa = u64::from(byte & 0x07);
        let base = 1u64 << (10 + exponent);
        base + (base / 8) * mantissa
    };

    let dictionary_id = match dictionary_flag {
        0 => None,
        1 => Some(u32::try_from(reader.number(1)?).ok()?),
        2 => Some(u32::try_from(reader.number(2)?).ok()?),
        _ => Some(u32::try_from(reader.number(4)?).ok()?),
    };

    // A single segment forces a content size even when the flag is zero.
    let content_size = match (content_size_flag, single_segment) {
        (0, false) => None,
        (0, true) => Some(reader.number(1)?),
        // The two-byte form has two hundred and fifty-six added to it.
        (1, _) => Some(reader.number(2)? + 256),
        (2, _) => Some(reader.number(4)?),
        _ => Some(reader.number(8)?),
    };

    // Walk the blocks to reach the end of the frame.
    loop {
        let header = reader.number(3)?;
        let last = header & 1 == 1;
        let block_kind = (header >> 1) & 0x03;
        let size = header >> 3;
        match block_kind {
            // Raw and compressed blocks store `size` bytes; a run-length
            // block stores one byte and repeats it `size` times.
            0 | 2 => reader.skip(size)?,
            1 => reader.skip(1)?,
            _ => return None,
        }
        if last {
            break;
        }
    }
    if checksum {
        reader.skip(4)?;
    }

    Some(Frame {
        kind: "content".to_owned(),
        window_size: if single_segment {
            content_size.unwrap_or(0)
        } else {
            window_size
        },
        content_size,
        dictionary_id,
        checksum,
        compressed_size: (reader.at - start) as u64,
    })
}

/// Whether `prefix` opens like a Zstandard frame.
fn looks_like_it(prefix: &[u8]) -> bool {
    let Some(head) = prefix.get(..4) else {
        return false;
    };
    let magic = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    magic == MAGIC
        || magic == DICTIONARY_MAGIC
        || (SKIPPABLE_FIRST..=SKIPPABLE_FIRST + 15).contains(&magic)
}

/// Everything [`ZstdView`] holds, read from `bytes`.
fn parse(bytes: &[u8]) -> Option<ZstdView> {
    // A dictionary is not frames at all: it is the table those frames
    // were compressed against, and without it they will not open.
    if let Some(head) = bytes.get(..8)
        && u32::from_le_bytes([head[0], head[1], head[2], head[3]]) == DICTIONARY_MAGIC
    {
        return Some(ZstdView {
            content: "dictionary".to_owned(),
            dictionary_provides: Some(u32::from_le_bytes([head[4], head[5], head[6], head[7]])),
            frames: Vec::new(),
            compressed_size: bytes.len() as u64,
            content_size: None,
            saved_percent: None,
            needs_a_dictionary: Vec::new(),
            unstated_sizes: 0,
        });
    }

    let mut reader = Reader { bytes, at: 0 };
    let mut frames = Vec::new();
    while reader.at < bytes.len() {
        let Some(frame) = frame_at(&mut reader) else {
            break;
        };
        frames.push(frame);
    }
    if frames.is_empty() {
        return None;
    }

    let compressed_size = bytes.len() as u64;
    let all_stated = frames
        .iter()
        .filter(|frame| frame.kind == "content")
        .all(|frame| frame.content_size.is_some());
    let content_size = all_stated.then(|| {
        frames
            .iter()
            .filter_map(|frame| frame.content_size)
            .sum::<u64>()
    });

    #[expect(
        clippy::cast_precision_loss,
        reason = "a size beyond a double's exact range is off by less than a byte in a \
                  percentage"
    )]
    let saved_percent = content_size.filter(|size| *size > 0).map(|size| {
        let original = size as f64;
        let stored = compressed_size as f64;
        ((original - stored) / original * 1000.0).round() / 10.0
    });

    Some(ZstdView {
        content: "frames".to_owned(),
        dictionary_provides: None,
        needs_a_dictionary: frames
            .iter()
            .filter_map(|frame| {
                frame.dictionary_id.map(|id| {
                    format!("dictionary {id}, without which this frame will not decompress")
                })
            })
            .collect(),
        unstated_sizes: frames
            .iter()
            .filter(|frame| frame.kind == "content" && frame.content_size.is_none())
            .count(),
        frames,
        compressed_size,
        content_size,
        saved_percent,
    })
}

/// The Zstandard plugin's core half.
#[derive(Debug, Default)]
pub struct ZstdCore;

impl PluginCore for ZstdCore {
    fn name(&self) -> &'static str {
        "zstd"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_it(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        // The frames run end to end, so the walk needs the whole file.
        let bytes = std::fs::read(path)?;
        let view = parse(&bytes).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "no readable Zstandard frame")
        })?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Zstandard plugin's presentation half.
#[derive(Debug, Default)]
pub struct ZstdPresentation;

impl PluginPresentation for ZstdPresentation {
    fn name(&self) -> &'static str {
        "zstd"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "ZST",
            tint: 0x0000_84c8,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: ZstdView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        if let Some(id) = view.dictionary_provides {
            return vec![
                format!(
                    "Zstandard dictionary {id}, {} byte(s)",
                    view.compressed_size
                ),
                "Not compressed content: this is the table content was".to_owned(),
                "compressed against. A frame naming this identifier will not".to_owned(),
                "decompress without it, and nothing else will open it either.".to_owned(),
            ];
        }
        let mut lines = vec![match (view.content_size, view.saved_percent) {
            (Some(size), Some(saved)) => format!(
                "Zstandard: {} byte(s) from {size}, {saved}% saved",
                view.compressed_size
            ),
            _ => format!(
                "Zstandard: {} byte(s); the frames do not say how big they \
                 decompress to",
                view.compressed_size
            ),
        }];
        lines.push(format!("{} frame(s):", view.frames.len()));
        for frame in &view.frames {
            let checksum = if frame.checksum { ", checksummed" } else { "" };
            lines.push(format!(
                "  {} - {} byte(s), window {}{checksum}",
                frame.kind, frame.compressed_size, frame.window_size
            ));
        }
        if !view.needs_a_dictionary.is_empty() {
            lines.push("Built against a dictionary, so the bytes alone are not".to_owned());
            lines.push("enough to get the content back:".to_owned());
            for said in &view.needs_a_dictionary {
                lines.push(format!("  {said}"));
            }
        }
        if view.unstated_sizes > 0 {
            lines.push(format!(
                "{} frame(s) do not declare a content size, so a reader has",
                view.unstated_sizes
            ));
            lines.push("to grow its buffer as it goes rather than allocating once.".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{ZstdCore, ZstdPresentation, ZstdView, looks_like_it};
    use plugin_api::{PluginCore, PluginPresentation};

    fn sample(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/zstd")
            .join(name)
    }

    fn view_of(name: &str) -> ZstdView {
        serde_json::from_value(ZstdCore.view(&sample(name)).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&ZstdCore),
            PluginPresentation::extensions(&ZstdPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn sniffs_a_frame_and_a_skippable_one() {
        assert!(looks_like_it(&[0x28, 0xb5, 0x2f, 0xfd]));
        assert!(
            looks_like_it(&[0x5a, 0x2a, 0x4d, 0x18]),
            "a skippable frame is still this format"
        );
        assert!(!looks_like_it(&[0x1f, 0x8b, 0x08, 0x00]), "that is gzip");
        assert!(!looks_like_it(b""));
    }

    #[test]
    fn reads_the_content_size_and_the_checksum_flag() {
        let view = view_of("readings.csv.zst");

        assert_eq!(view.frames.len(), 1);
        assert!(view.frames[0].checksum);
        assert!(view.frames[0].content_size.is_some());
        assert!(
            view.content_size
                .is_some_and(|size| size > view.compressed_size)
        );
        assert!(view.saved_percent.is_some_and(|saved| saved > 50.0));
        assert_eq!(view.unstated_sizes, 0);
    }

    #[test]
    fn reads_the_window_size_a_reader_will_need() {
        let view = view_of("readings.csv.zst");

        assert!(
            view.frames[0].window_size >= 1024,
            "the window is at least a kilobyte in every frame there is"
        );
    }

    #[test]
    fn names_the_dictionary_a_frame_cannot_do_without() {
        let view = view_of("needs-a-dictionary.zst");

        assert_eq!(view.needs_a_dictionary.len(), 1);
        assert!(view.frames[0].dictionary_id.is_some_and(|id| id != 0));
        assert!(view.needs_a_dictionary[0].contains("will not decompress"));
    }

    #[test]
    fn a_frame_without_a_dictionary_says_so_by_saying_nothing() {
        let view = view_of("readings.csv.zst");

        assert!(view.frames[0].dictionary_id.is_none());
        assert!(view.needs_a_dictionary.is_empty());
    }

    #[test]
    fn a_bare_dictionary_is_recognised_as_one() {
        let view = view_of("readings.dict");

        assert_eq!(view.content, "dictionary");
        assert!(view.dictionary_provides.is_some());
        assert!(view.frames.is_empty(), "a dictionary holds no frames");
    }

    #[test]
    fn the_dictionary_a_frame_names_is_the_one_shipped_beside_it() {
        let frame = view_of("needs-a-dictionary.zst");
        let dictionary = view_of("readings.dict");

        assert_eq!(
            frame.frames[0].dictionary_id, dictionary.dictionary_provides,
            "the frame names the identifier the dictionary provides"
        );
    }

    #[test]
    fn presents_a_dictionary_as_what_it_is() {
        let data = ZstdCore.view(&sample("readings.dict")).unwrap();

        let lines = ZstdPresentation.present(&data);

        assert!(lines[0].starts_with("Zstandard dictionary "));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Not compressed content"))
        );
    }

    #[test]
    fn presents_the_dictionary_warning_with_its_reason() {
        let data = ZstdCore.view(&sample("needs-a-dictionary.zst")).unwrap();

        let lines = ZstdPresentation.present(&data);

        assert!(lines[0].starts_with("Zstandard:"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("bytes alone are not"))
        );
    }

    #[test]
    fn a_file_that_is_not_zstandard_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.zst");
        std::fs::write(&path, b"nothing like a frame").unwrap();

        assert!(ZstdCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
