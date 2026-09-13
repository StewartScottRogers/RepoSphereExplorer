//! bzip2 file type plugin: core and presentation halves.
//!
//! A bzip2 stream is a three-byte magic, a block-size digit, and then
//! blocks that are not aligned to bytes at all - every marker inside is
//! found on a bit boundary. This reads the block size, how many blocks
//! there are, and how many whole streams the file holds, without
//! decompressing any of it.
//!
//! bzip2 records the uncompressed size nowhere, so the pane says so
//! rather than guessing.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["bz2", "tbz2", "tbz"];

/// The three bytes a stream opens with, before the block-size digit.
const MAGIC: &[u8] = b"BZh";

/// The forty-eight bits that open a block: pi, in binary coded decimal.
const BLOCK_MARKER: u64 = 0x3141_5926_5359;

/// The forty-eight bits that end a stream: the square root of pi.
const STREAM_END: u64 = 0x1772_4538_5090;

/// How many bits those markers are.
const MARKER_BITS: u32 = 48;

/// View data produced by [`Bzip2Core::view`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bzip2View {
    /// The block size the writer was asked for, in bytes.
    pub block_size: usize,
    /// How many blocks the file holds across all its streams.
    pub blocks: usize,
    /// How many whole streams it holds. More than one is what `cat`
    /// leaves behind, and every reader has to be told to expect them.
    pub streams: usize,
    /// The file's own size.
    pub compressed_size: u64,
    /// What the format does not record, and what that costs.
    pub not_recorded: Vec<String>,
}

/// Counts the block and stream markers, which sit on bit boundaries.
///
/// bzip2 packs its blocks bit by bit, so neither marker is byte aligned
/// and neither can be found by searching for six bytes. A window of the
/// last forty-eight bits, advanced one bit at a time, is the whole of it.
fn count_markers(bytes: &[u8]) -> (usize, usize) {
    let mut window = 0u64;
    let mut seen = 0u32;
    let mut blocks = 0usize;
    let mut ends = 0usize;

    for byte in bytes {
        for shift in (0..8).rev() {
            window = ((window << 1) | u64::from((byte >> shift) & 1)) & ((1 << MARKER_BITS) - 1);
            seen += 1;
            if seen < MARKER_BITS {
                continue;
            }
            if window == BLOCK_MARKER {
                blocks += 1;
            } else if window == STREAM_END {
                ends += 1;
            }
        }
    }
    (blocks, ends)
}

/// Whether `prefix` opens like a bzip2 stream.
fn looks_like_it(prefix: &[u8]) -> bool {
    prefix.starts_with(MAGIC)
        && prefix
            .get(3)
            .is_some_and(|digit| (b'1'..=b'9').contains(digit))
}

/// Everything [`Bzip2View`] holds, read from `bytes`.
fn parse(bytes: &[u8], compressed_size: u64) -> Option<Bzip2View> {
    if !looks_like_it(bytes) {
        return None;
    }
    let digit = u32::from(*bytes.get(3)? - b'0');
    let (blocks, ends) = count_markers(bytes);

    Some(Bzip2View {
        block_size: digit as usize * 100_000,
        blocks,
        // Every stream ends with the end marker, so the count of those is
        // the count of whole streams.
        streams: ends.max(1),
        compressed_size,
        not_recorded: vec![
            "the uncompressed size, which bzip2 stores nowhere - the only \
             way to learn it is to decompress the whole thing"
                .to_owned(),
            "the original file name and its modification time, which gzip \
             keeps and this format does not"
                .to_owned(),
        ],
    })
}

/// The bzip2 plugin's core half.
#[derive(Debug, Default)]
pub struct Bzip2Core;

impl PluginCore for Bzip2Core {
    fn name(&self) -> &'static str {
        "bzip2"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_it(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        // The markers are spread through the file, so it is read whole.
        let bytes = std::fs::read(path)?;
        let size = bytes.len() as u64;
        let view = parse(&bytes, size)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not a bzip2 stream"))?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The bzip2 plugin's presentation half.
#[derive(Debug, Default)]
pub struct Bzip2Presentation;

impl PluginPresentation for Bzip2Presentation {
    fn name(&self) -> &'static str {
        "bzip2"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "BZ2",
            tint: 0x00a3_2c2c,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: Bzip2View = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "bzip2: {} byte(s) in {} block(s) of up to {}",
            view.compressed_size, view.blocks, view.block_size
        )];
        if view.streams > 1 {
            lines.push(format!(
                "{} whole streams, one after another - which is what `cat`",
                view.streams
            ));
            lines.push("leaves behind. A reader that stops at the first".to_owned());
            lines.push("end-of-stream marker sees only part of this file.".to_owned());
        } else {
            lines.push("One stream.".to_owned());
        }
        lines.push("Not recorded anywhere in the format:".to_owned());
        for said in &view.not_recorded {
            lines.push(format!("  {said}"));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{Bzip2Core, Bzip2Presentation, Bzip2View, count_markers, looks_like_it};
    use plugin_api::{PluginCore, PluginPresentation};

    fn sample(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/bzip2")
            .join(name)
    }

    fn view_of(name: &str) -> Bzip2View {
        serde_json::from_value(Bzip2Core.view(&sample(name)).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&Bzip2Core),
            PluginPresentation::extensions(&Bzip2Presentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn sniffs_the_magic_and_the_block_size_digit() {
        assert!(looks_like_it(b"BZh9\x31\x41\x59"));
        assert!(looks_like_it(b"BZh1"));
        assert!(!looks_like_it(b"BZh0"), "zero is not a block size");
        assert!(!looks_like_it(b"BZhx"));
        assert!(!looks_like_it(b"BZ"));
        assert!(!looks_like_it(b""));
    }

    #[test]
    fn a_marker_is_found_on_a_bit_boundary() {
        // The block marker, shifted three bits into the stream, which is
        // where bzip2 actually puts it.
        let marker: u64 = 0x3141_5926_5359;
        let mut bytes = vec![0u8; 16];
        for bit in 0..48 {
            if (marker >> (47 - bit)) & 1 == 1 {
                let at = 3 + bit;
                bytes[at / 8] |= 1 << (7 - (at % 8));
            }
        }

        let (blocks, _) = count_markers(&bytes);
        assert_eq!(
            blocks, 1,
            "searching for six bytes would find nothing: it is not aligned"
        );
    }

    #[test]
    fn reads_a_multi_block_archive() {
        let view = view_of("readings.csv.bz2");

        assert_eq!(view.block_size, 100_000);
        assert!(
            view.blocks >= 3,
            "twelve thousand rows at level 1 is several blocks"
        );
        assert_eq!(view.streams, 1);
    }

    #[test]
    fn counts_two_streams_written_one_after_another() {
        let view = view_of("two-streams.bz2");

        assert_eq!(
            view.streams, 2,
            "`cat a.bz2 b.bz2` leaves two whole streams, and both are real"
        );
        assert!(view.blocks >= 2);
    }

    #[test]
    fn says_what_the_format_does_not_record() {
        let view = view_of("readings.csv.bz2");

        assert_eq!(view.not_recorded.len(), 2);
        assert!(
            view.not_recorded
                .iter()
                .any(|said| said.contains("uncompressed size"))
        );
    }

    #[test]
    fn presents_the_two_stream_warning_with_its_reason() {
        let data = Bzip2Core.view(&sample("two-streams.bz2")).unwrap();

        let lines = Bzip2Presentation.present(&data);

        assert!(lines[0].starts_with("bzip2:"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("only part of this file"))
        );
    }

    #[test]
    fn a_file_that_is_not_bzip2_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.bz2");
        std::fs::write(&path, b"BZ and nothing else").unwrap();

        assert!(Bzip2Core.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
