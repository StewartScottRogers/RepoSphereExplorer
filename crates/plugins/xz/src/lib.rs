//! xz file type plugin: core and presentation halves.
//!
//! An xz file is a header, some blocks, an index and a footer. The index
//! records every block's compressed and uncompressed size, so the ratio
//! is readable without decompressing anything - and the first block's
//! header names the filter chain, which is what a reader needs to know
//! it can handle the file at all.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["xz", "txz", "lzma"];

/// The six bytes a stream opens with.
const MAGIC: &[u8] = &[0xFD, b'7', b'z', b'X', b'Z', 0x00];

/// The two bytes it closes with.
const FOOTER_MAGIC: &[u8] = b"YZ";

/// The filters, by the identifier the block header writes.
const FILTERS: &[(u64, &str)] = &[
    (0x21, "LZMA2, which does the compressing"),
    (
        0x03,
        "delta, which stores each byte as its difference from an earlier one",
    ),
    (0x04, "x86 branch calls, rewritten to compress better"),
    (0x05, "PowerPC branch calls, rewritten to compress better"),
    (0x06, "Itanium branch calls, rewritten to compress better"),
    (0x07, "ARM branch calls, rewritten to compress better"),
    (0x08, "ARM Thumb branch calls, rewritten to compress better"),
    (0x09, "SPARC branch calls, rewritten to compress better"),
    (0x0a, "ARM64 branch calls, rewritten to compress better"),
    (0x0b, "RISC-V branch calls, rewritten to compress better"),
];

/// The integrity checks, by the number in the stream flags.
const CHECKS: &[(u8, &str)] = &[
    (0x00, "none, so nothing notices a corrupted file"),
    (0x01, "CRC-32"),
    (0x04, "CRC-64"),
    (0x0a, "SHA-256"),
];

/// View data produced by [`XzCore::view`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XzView {
    /// How many blocks the index records.
    pub blocks: usize,
    /// Which integrity check the stream uses.
    pub check: String,
    /// The filter chain of the first block, in the order it is applied
    /// when decompressing.
    pub filters: Vec<String>,
    /// The dictionary size LZMA2 was given, when the properties say.
    pub dictionary_size: Option<u64>,
    /// What the index says the blocks decompress to.
    pub uncompressed_size: u64,
    /// The file's own size.
    pub compressed_size: u64,
    /// How much smaller it got.
    pub saved_percent: f64,
}

/// Reads one of xz's own multibyte integers, which are seven bits at a
/// time with the top bit meaning "another byte follows".
fn multibyte(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for step in 0..9u32 {
        let byte = *bytes.get(*at)?;
        *at += 1;
        value |= u64::from(byte & 0x7F) << (step * 7);
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

/// The filter chain and dictionary size of the block starting at `at`.
fn block_filters(bytes: &[u8], at: usize) -> (Vec<String>, Option<u64>) {
    let mut filters = Vec::new();
    let mut dictionary = None;
    let Some(&size_byte) = bytes.get(at) else {
        return (filters, dictionary);
    };
    if size_byte == 0 {
        // A zero here is the index, not a block: this stream has none.
        return (filters, dictionary);
    }
    let Some(&flags) = bytes.get(at + 1) else {
        return (filters, dictionary);
    };
    let mut cursor = at + 2;
    // The two size fields are present only when their flag bits are.
    if flags & 0x40 != 0 && multibyte(bytes, &mut cursor).is_none() {
        return (filters, dictionary);
    }
    if flags & 0x80 != 0 && multibyte(bytes, &mut cursor).is_none() {
        return (filters, dictionary);
    }

    for _ in 0..=(flags & 0x03) {
        let Some(id) = multibyte(bytes, &mut cursor) else {
            break;
        };
        let Some(properties) = multibyte(bytes, &mut cursor) else {
            break;
        };
        filters.push(
            FILTERS
                .iter()
                .find(|(number, _)| *number == id)
                .map_or_else(|| format!("filter {id:#x}"), |(_, said)| (*said).to_owned()),
        );
        if id == 0x21 && properties == 1 {
            // LZMA2's single property byte encodes the dictionary size.
            if let Some(&byte) = bytes.get(cursor) {
                let bits = u64::from(byte & 0x3F);
                // Forty and above means the whole address space; below
                // that the bits encode a size that doubles every two.
                dictionary = Some(if bits >= 40 {
                    u64::MAX
                } else {
                    (2 | (bits & 1)) << (bits / 2 + 11)
                });
            }
        }
        cursor += usize::try_from(properties).unwrap_or(0);
    }
    (filters, dictionary)
}

/// Whether `prefix` opens like an xz stream.
fn looks_like_it(prefix: &[u8]) -> bool {
    prefix.starts_with(MAGIC)
}

/// Everything [`XzView`] holds, read from `bytes`.
fn parse(bytes: &[u8]) -> Option<XzView> {
    if !looks_like_it(bytes) || bytes.len() < 32 {
        return None;
    }
    if !bytes.ends_with(FOOTER_MAGIC) {
        return None;
    }
    // The footer is twelve bytes: a check, the index's size, the stream
    // flags, and the magic.
    let footer = bytes.len() - 12;
    let backward_size = u32::from_le_bytes(bytes.get(footer + 4..footer + 8)?.try_into().ok()?);
    let stated = usize::try_from(backward_size).ok()?;
    let index_at = footer.checked_sub(stated.checked_add(1)?.checked_mul(4)?)?;

    // The index opens with a zero byte, then the record count, then a
    // pair of sizes for each block.
    let mut cursor = index_at;
    if bytes.get(cursor) != Some(&0) {
        return None;
    }
    cursor += 1;
    let count = multibyte(bytes, &mut cursor)?;
    let mut uncompressed_size = 0u64;
    for _ in 0..count.min(1 << 20) {
        // The unpadded size, then the uncompressed one.
        multibyte(bytes, &mut cursor)?;
        uncompressed_size = uncompressed_size.saturating_add(multibyte(bytes, &mut cursor)?);
    }

    let check_id = bytes.get(7)? & 0x0F;
    let (filters, dictionary_size) = block_filters(bytes, 12);
    let compressed_size = bytes.len() as u64;

    #[expect(
        clippy::cast_precision_loss,
        reason = "a size beyond a double's exact range is off by less than a byte in a \
                  percentage"
    )]
    let saved_percent = if uncompressed_size == 0 {
        0.0
    } else {
        let original = uncompressed_size as f64;
        let stored = compressed_size as f64;
        ((original - stored) / original * 1000.0).round() / 10.0
    };

    Some(XzView {
        blocks: usize::try_from(count).ok()?,
        check: CHECKS
            .iter()
            .find(|(number, _)| *number == check_id)
            .map_or_else(
                || format!("check {check_id}"),
                |(_, said)| (*said).to_owned(),
            ),
        filters,
        dictionary_size,
        uncompressed_size,
        compressed_size,
        saved_percent,
    })
}

/// The xz plugin's core half.
#[derive(Debug, Default)]
pub struct XzCore;

impl PluginCore for XzCore {
    fn name(&self) -> &'static str {
        "xz"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_it(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        // The index is at the end and the block header at the start, so
        // both ends are needed; the file is read whole.
        let bytes = std::fs::read(path)?;
        let view = parse(&bytes)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not an xz stream"))?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The xz plugin's presentation half.
#[derive(Debug, Default)]
pub struct XzPresentation;

impl PluginPresentation for XzPresentation {
    fn name(&self) -> &'static str {
        "xz"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "XZ",
            tint: 0x0060_9926,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: XzView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![
            format!(
                "xz: {} byte(s) from {}, {}% saved",
                view.compressed_size, view.uncompressed_size, view.saved_percent
            ),
            format!("{} block(s), checked with {}", view.blocks, view.check),
        ];
        if view.filters.is_empty() {
            lines.push("The block header names no filters.".to_owned());
        } else {
            lines.push("Filter chain:".to_owned());
            for said in &view.filters {
                lines.push(format!("  {said}"));
            }
        }
        if let Some(size) = view.dictionary_size {
            lines.push(format!(
                "Dictionary of {size} byte(s), which is what decompressing will"
            ));
            lines.push("want in memory.".to_owned());
        }
        if view.filters.len() > 1 {
            lines.push("More than one filter, so a reader that only knows LZMA2".to_owned());
            lines.push("cannot open this file.".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{XzCore, XzPresentation, XzView, looks_like_it, multibyte};
    use plugin_api::{PluginCore, PluginPresentation};

    fn sample(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/xz")
            .join(name)
    }

    fn view_of(name: &str) -> XzView {
        serde_json::from_value(XzCore.view(&sample(name)).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&XzCore),
            PluginPresentation::extensions(&XzPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn sniffs_the_magic() {
        assert!(looks_like_it(&[0xFD, b'7', b'z', b'X', b'Z', 0x00]));
        assert!(!looks_like_it(b"7z\xbc\xaf\x27\x1c"), "that is 7-Zip");
        assert!(!looks_like_it(b""));
    }

    #[test]
    fn a_multibyte_integer_is_seven_bits_at_a_time() {
        let mut at = 0;
        assert_eq!(multibyte(&[0x7f], &mut at), Some(127));
        at = 0;
        // 0x80 0x01 is one hundred and twenty-eight, not thirty-two thousand.
        assert_eq!(multibyte(&[0x80, 0x01], &mut at), Some(128));
        assert_eq!(at, 2);
    }

    #[test]
    fn reads_the_sizes_from_the_index() {
        let view = view_of("readings.csv.xz");

        assert!(view.blocks >= 1);
        assert!(view.uncompressed_size > view.compressed_size);
        assert!(
            view.saved_percent > 50.0,
            "comma separated text compresses well"
        );
    }

    #[test]
    fn reads_the_check_the_stream_flags_name() {
        assert_eq!(view_of("readings.csv.xz").check, "CRC-64");
        assert_eq!(view_of("filtered.xz").check, "SHA-256");
    }

    #[test]
    fn reads_a_chain_of_more_than_one_filter() {
        let plain = view_of("readings.csv.xz");
        assert_eq!(plain.filters.len(), 1);
        assert!(plain.filters[0].starts_with("LZMA2"));

        let filtered = view_of("filtered.xz");
        assert_eq!(filtered.filters.len(), 2, "a delta filter and then LZMA2");
        assert!(filtered.filters[0].starts_with("delta"));
        assert!(filtered.filters[1].starts_with("LZMA2"));
    }

    #[test]
    fn reads_the_dictionary_size_lzma2_was_given() {
        let view = view_of("readings.csv.xz");

        assert!(
            view.dictionary_size.is_some_and(|size| size >= 1 << 20),
            "the default preset asks for at least a megabyte: {:?}",
            view.dictionary_size
        );
    }

    #[test]
    fn presents_the_chain_and_what_it_costs_a_reader() {
        let data = XzCore.view(&sample("filtered.xz")).unwrap();

        let lines = XzPresentation.present(&data);

        assert!(lines[0].starts_with("xz:"));
        assert!(lines.iter().any(|line| line.contains("only knows LZMA2")));
        assert!(lines.iter().any(|line| line.contains("SHA-256")));
    }

    #[test]
    fn a_file_that_is_not_xz_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.xz");
        std::fs::write(&path, b"\xfd7zXZ\x00 and then nothing").unwrap();

        assert!(XzCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
