//! 7-Zip archive file type plugin: core and presentation halves.
//!
//! 7-Zip packs several files into one *solid* block and compresses the
//! block, so a single entry has no compressed size of its own - only the
//! block does, and extracting one entry means decompressing everything
//! ahead of it in that block. This reads the entries, their sizes and
//! times, the methods used, whether the header itself is encrypted, and
//! which entries share a block.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["7z"];

/// The six bytes an archive opens with.
const MAGIC: &[u8] = &[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C];

/// How many entries are listed before the rest are only counted.
const SHOWN: usize = 64;

/// One entry in the archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Its path within the archive.
    pub path: String,
    /// Its size once unpacked. A directory has none.
    pub size: u64,
    /// Whether it is a directory rather than a file.
    pub directory: bool,
    /// When it was last written, as seconds since the epoch.
    pub modified: Option<i64>,
}

/// View data produced by [`SevenzipCore::view`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SevenzipView {
    /// The format version the signature header declares.
    pub format_version: String,
    /// How many entries there are.
    pub entries: usize,
    /// The first of them.
    pub listed: Vec<Entry>,
    /// The total once everything is unpacked.
    pub total_size: u64,
    /// The archive's own size.
    pub compressed_size: u64,
    /// How much smaller it is.
    pub saved_percent: f64,
    /// The compression methods the blocks use.
    pub methods: Vec<String>,
    /// Whether the header itself is encrypted, in which case even the
    /// list of what is inside needs the password.
    pub header_encrypted: bool,
    /// Blocks holding more than one entry, with what that costs.
    pub solid_blocks: Vec<String>,
}

/// Whether `prefix` opens like a 7-Zip archive.
fn looks_like_it(prefix: &[u8]) -> bool {
    prefix.starts_with(MAGIC)
}

/// Everything [`SevenzipView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<SevenzipView> {
    let bytes = std::fs::read(path)?;
    if !looks_like_it(&bytes) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a 7-Zip archive",
        ));
    }
    let compressed_size = bytes.len() as u64;
    let format_version = format!(
        "{}.{}",
        bytes.get(6).copied().unwrap_or(0),
        bytes.get(7).copied().unwrap_or(0)
    );

    let archive = sevenz_rust2::Archive::read(
        &mut io::Cursor::new(&bytes),
        &sevenz_rust2::Password::from(""),
    );
    let archive = match archive {
        Ok(archive) => archive,
        Err(err) => {
            // A header encrypted with a password cannot be read at all
            // without it, and saying so is more useful than an error.
            let said = err.to_string();
            if said.to_lowercase().contains("password") {
                return Ok(SevenzipView {
                    format_version,
                    entries: 0,
                    listed: Vec::new(),
                    total_size: 0,
                    compressed_size,
                    saved_percent: 0.0,
                    methods: Vec::new(),
                    header_encrypted: true,
                    solid_blocks: Vec::new(),
                });
            }
            return Err(io::Error::new(io::ErrorKind::InvalidData, said));
        }
    };

    let (listed, total_size) = entries_of(&archive);
    let (methods, solid_blocks) = blocks_of(&archive);

    #[expect(
        clippy::cast_precision_loss,
        reason = "a size beyond a double's exact range is off by less than a byte in a \
                  percentage"
    )]
    let saved_percent = if total_size == 0 {
        0.0
    } else {
        let original = total_size as f64;
        let stored = compressed_size as f64;
        ((original - stored) / original * 1000.0).round() / 10.0
    };

    Ok(SevenzipView {
        format_version,
        entries: archive.files.len(),
        listed,
        total_size,
        compressed_size,
        saved_percent,
        methods,
        header_encrypted: false,
        solid_blocks,
    })
}

/// The entries worth listing, and the total once everything is unpacked.
fn entries_of(archive: &sevenz_rust2::Archive) -> (Vec<Entry>, u64) {
    let mut listed = Vec::new();
    let mut total_size = 0u64;
    for entry in &archive.files {
        total_size += entry.size;
        if listed.len() < SHOWN {
            listed.push(Entry {
                path: entry.name.clone(),
                size: entry.size,
                directory: entry.is_directory,
                // 7-Zip counts hundred-nanosecond ticks from the year
                // 1601, so the conversion goes through `SystemTime`,
                // which knows where the Unix epoch sits in that.
                modified: entry
                    .has_last_modified_date
                    .then(|| std::time::SystemTime::from(entry.last_modified_date))
                    .and_then(|when| when.duration_since(std::time::UNIX_EPOCH).ok())
                    .and_then(|since| i64::try_from(since.as_secs()).ok()),
            });
        }
    }
    (listed, total_size)
}

/// The methods the blocks use, and the ones holding more than one entry.
fn blocks_of(archive: &sevenz_rust2::Archive) -> (Vec<String>, Vec<String>) {
    let mut methods: Vec<String> = Vec::new();
    let mut solid_blocks = Vec::new();
    for (index, block) in archive.blocks.iter().enumerate() {
        for coder in &block.coders {
            let said = method_named(coder.encoder_method_id());
            if !methods.contains(&said) {
                methods.push(said);
            }
        }
        // How many entries this block unpacks to. More than one means it
        // is solid: they share a compressed stream.
        let packed = archive
            .files
            .iter()
            .enumerate()
            .filter(|(at, _)| archive.stream_map.file_block_index.get(*at) == Some(&Some(index)))
            .count();
        if packed > 1 {
            solid_blocks.push(format!(
                "block {} holds {packed} entries, so extracting any one of them \
                 decompresses the others as well",
                index + 1
            ));
        }
    }
    (methods, solid_blocks)
}

/// The method a coder's identifier bytes name.
///
/// 7-Zip writes the identifier as a run of bytes rather than a number,
/// and the run's length is part of which method it is.
fn method_named(id: &[u8]) -> String {
    match id {
        [0x00] => "stored, not compressed at all".to_owned(),
        [0x21] => "LZMA2".to_owned(),
        [0x03, 0x01, 0x01] => "LZMA".to_owned(),
        [0x04, 0x01, 0x08] => "deflate".to_owned(),
        [0x04, 0x02, 0x02] => "bzip2".to_owned(),
        [0x03, 0x03, 0x01, 0x03] => {
            "BCJ, which rewrites branch calls to compress better".to_owned()
        }
        [0x03, 0x03, 0x01, 0x1B] => {
            "BCJ2, which rewrites branch calls to compress better".to_owned()
        }
        [0x03, 0x04, 0x01] => "PPMd".to_owned(),
        [0x06, 0xF1, 0x07, 0x01] => "AES-256, so the content needs a password".to_owned(),
        other => other.iter().fold(String::from("method"), |mut out, byte| {
            use std::fmt::Write as _;
            let _ = write!(out, " {byte:02x}");
            out
        }),
    }
}

/// The 7-Zip archive plugin's core half.
#[derive(Debug, Default)]
pub struct SevenzipCore;

impl PluginCore for SevenzipCore {
    fn name(&self) -> &'static str {
        "sevenzip"
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

/// The 7-Zip archive plugin's presentation half.
#[derive(Debug, Default)]
pub struct SevenzipPresentation;

impl PluginPresentation for SevenzipPresentation {
    fn name(&self) -> &'static str {
        "sevenzip"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "7Z",
            tint: 0x0034_7d39,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: SevenzipView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        if view.header_encrypted {
            return vec![
                format!("7-Zip archive, format {}", view.format_version),
                "The header itself is encrypted, so even the list of what is".to_owned(),
                "inside needs the password. Nothing else can be said about it.".to_owned(),
            ];
        }
        let mut lines = vec![
            format!(
                "7-Zip: {} entry(ies), {} byte(s) from {}, {}% saved",
                view.entries, view.compressed_size, view.total_size, view.saved_percent
            ),
            format!("Format {}", view.format_version),
        ];
        for entry in &view.listed {
            let what = if entry.directory { "dir " } else { "file" };
            lines.push(format!("  {what} {:>9} {}", entry.size, entry.path));
        }
        if view.entries > view.listed.len() {
            lines.push(format!(
                "  ... and {} more",
                view.entries - view.listed.len()
            ));
        }
        if !view.methods.is_empty() {
            lines.push(format!("Methods: {}", view.methods.join(", ")));
        }
        if view.solid_blocks.is_empty() {
            lines.push("No solid block holds more than one entry, so any single".to_owned());
            lines.push("file can be extracted on its own.".to_owned());
        } else {
            lines.push("Packed solid:".to_owned());
            for said in &view.solid_blocks {
                lines.push(format!("  {said}"));
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{SevenzipCore, SevenzipPresentation, SevenzipView, looks_like_it};
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/sevenzip/readings.7z")
    }

    fn view_of() -> SevenzipView {
        serde_json::from_value(SevenzipCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&SevenzipCore),
            PluginPresentation::extensions(&SevenzipPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn sniffs_the_signature() {
        assert!(looks_like_it(&[
            b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C, 0x00, 0x04
        ]));
        assert!(!looks_like_it(b"PK\x03\x04"), "that is a zip");
        assert!(!looks_like_it(b"7z"), "two bytes is not the signature");
        assert!(!looks_like_it(b""));
    }

    #[test]
    fn reads_the_entries_and_their_sizes() {
        let view = view_of();

        assert!(view.entries >= 4);
        assert!(
            view.listed
                .iter()
                .any(|entry| entry.path.contains("samples.csv"))
        );
        assert!(view.listed.iter().any(|entry| entry.directory));
        assert!(view.total_size > view.compressed_size);
        assert!(view.saved_percent > 0.0);
    }

    #[test]
    fn reads_the_format_version_and_the_methods() {
        let view = view_of();

        assert!(
            view.format_version.starts_with('0'),
            "{}",
            view.format_version
        );
        assert!(!view.methods.is_empty());
    }

    #[test]
    fn says_whether_a_block_holds_more_than_one_entry() {
        let view = view_of();

        // Either answer is true of some archive; what matters is that the
        // pane says which, and says what it costs.
        let data = serde_json::to_value(&view).unwrap();
        let lines = SevenzipPresentation.present(&data);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("decompresses the others")
                    || line.contains("extracted on its own")),
            "the pane has to say one or the other"
        );
    }

    #[test]
    fn an_entry_keeps_the_time_it_was_written() {
        let view = view_of();

        assert!(
            view.listed.iter().any(|entry| entry.modified.is_some()),
            "7-Zip records a modification time for every entry it packs"
        );
    }

    #[test]
    fn presents_the_entries_and_the_ratio() {
        let data = SevenzipCore.view(&fixture()).unwrap();

        let lines = SevenzipPresentation.present(&data);

        assert!(lines[0].starts_with("7-Zip: "));
        assert!(lines.iter().any(|line| line.contains("samples.csv")));
    }

    #[test]
    fn a_file_that_is_not_an_archive_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.7z");
        std::fs::write(&path, b"7z\xbc\xaf\x27\x1c and then nothing of the sort").unwrap();

        assert!(SevenzipCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
