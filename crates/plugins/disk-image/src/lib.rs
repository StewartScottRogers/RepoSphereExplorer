//! Disk image file type plugin: core and presentation halves.
//!
//! Covers `.iso` (ISO 9660) only. `.dmg` (Apple's UDIF format) was ruled
//! out for this plugin: its only fixed magic is a `koly` trailer at the
//! *end* of the file, and `PluginCore::sniff` only ever sees a bounded
//! prefix from the *start* — no sniff window can see it. See PLUGINS.md's
//! Rejected table.
//!
//! ISO 9660's own magic isn't near the start of the file either: the
//! `CD001` standard identifier lives inside the Primary Volume Descriptor,
//! at sector 16 (byte offset 32768) rather than sector 0. That's why this
//! plugin needed `crates/service`'s `SNIFF_PREFIX_LEN` raised to cover it,
//! a project-wide change no earlier plugin's sniff needed.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io::{self, Read as _, Seek as _, SeekFrom};
use std::path::Path;

/// Sector size fixed by the ISO 9660 spec.
const SECTOR_SIZE: u64 = 2048;

/// The system area occupies the first 16 sectors; the Primary Volume
/// Descriptor is the first sector after it.
const PRIMARY_VOLUME_DESCRIPTOR_OFFSET: u64 = 16 * SECTOR_SIZE;

/// Offset, relative to the descriptor's start, of its 5-byte standard
/// identifier field.
const STANDARD_IDENTIFIER_OFFSET: usize = 1;

/// The fixed standard identifier every ISO 9660 volume descriptor carries.
const STANDARD_IDENTIFIER: &[u8] = b"CD001";

/// Offset and length, relative to the descriptor's start, of the Volume
/// Identifier field (`d`-characters, space-padded).
const VOLUME_IDENTIFIER_RANGE: std::ops::Range<usize> = 40..72;

/// Offset of the Volume Space Size field (little-endian `u32`): the number
/// of logical blocks the volume occupies.
const VOLUME_SPACE_SIZE_OFFSET: usize = 80;

/// Offset of the Logical Block Size field (little-endian `u16`).
const LOGICAL_BLOCK_SIZE_OFFSET: usize = 128;

/// Number of descriptor bytes this plugin reads: enough to cover the
/// Logical Block Size field, the last one it uses.
const DESCRIPTOR_READ_LEN: u64 = 132;

/// Whether `prefix` carries the `CD001` standard identifier at ISO 9660's
/// fixed Primary Volume Descriptor offset.
fn looks_like_iso9660(prefix: &[u8]) -> bool {
    let start = usize::try_from(PRIMARY_VOLUME_DESCRIPTOR_OFFSET).unwrap_or(usize::MAX)
        + STANDARD_IDENTIFIER_OFFSET;
    prefix
        .get(start..start + STANDARD_IDENTIFIER.len())
        .is_some_and(|bytes| bytes == STANDARD_IDENTIFIER)
}

/// Reads the Volume Descriptor sector's leading [`DESCRIPTOR_READ_LEN`]
/// bytes from `path`.
fn read_descriptor(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(PRIMARY_VOLUME_DESCRIPTOR_OFFSET))?;
    let mut buf = Vec::new();
    file.take(DESCRIPTOR_READ_LEN).read_to_end(&mut buf)?;
    if buf.len() < usize::try_from(DESCRIPTOR_READ_LEN).unwrap_or(0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "truncated ISO 9660 volume descriptor",
        ));
    }
    Ok(buf)
}

/// Decodes the space-padded Volume Identifier field, if it carries any
/// non-space text.
fn read_volume_identifier(descriptor: &[u8]) -> Option<String> {
    let raw = descriptor.get(VOLUME_IDENTIFIER_RANGE)?;
    let text = String::from_utf8_lossy(raw).trim_end().to_owned();
    (!text.is_empty()).then_some(text)
}

/// View data produced by [`DiskImageCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskImageView {
    /// The detected disk image format, currently always `"ISO 9660"`.
    pub format: String,
    /// The volume's label, from the Primary Volume Descriptor, if set.
    pub volume_identifier: Option<String>,
    /// Logical block size, in bytes.
    pub logical_block_size: u16,
    /// Number of logical blocks the volume occupies.
    pub block_count: u32,
    /// Volume size in bytes (`logical_block_size * block_count`).
    pub volume_size: u64,
    /// Size of the file on disk, in bytes.
    pub file_size: u64,
}

/// The disk image plugin's core half.
#[derive(Debug, Default)]
pub struct DiskImageCore;

impl PluginCore for DiskImageCore {
    fn name(&self) -> &'static str {
        "disk-image"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_iso9660(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let file_size = std::fs::metadata(path)?.len();
        let descriptor = read_descriptor(path)?;
        let logical_block_size = u16::from_le_bytes(
            descriptor[LOGICAL_BLOCK_SIZE_OFFSET..][..2]
                .try_into()
                .unwrap(),
        );
        let block_count = u32::from_le_bytes(
            descriptor[VOLUME_SPACE_SIZE_OFFSET..][..4]
                .try_into()
                .unwrap(),
        );
        let view = DiskImageView {
            format: "ISO 9660".to_owned(),
            volume_identifier: read_volume_identifier(&descriptor),
            logical_block_size,
            block_count,
            volume_size: u64::from(logical_block_size) * u64::from(block_count),
            file_size,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The disk image plugin's presentation half.
#[derive(Debug, Default)]
pub struct DiskImagePresentation;

impl PluginPresentation for DiskImagePresentation {
    fn name(&self) -> &'static str {
        "disk-image"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: DiskImageView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!("{} disk image", view.format)];
        if let Some(volume_identifier) = &view.volume_identifier {
            lines.push(volume_identifier.clone());
        }
        lines.push(format!(
            "{} blocks x {} bytes = {} bytes",
            view.block_count, view.logical_block_size, view.volume_size
        ));
        lines.push(format!("{} bytes on disk", view.file_size));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DiskImageCore, DiskImagePresentation, DiskImageView, LOGICAL_BLOCK_SIZE_OFFSET,
        PRIMARY_VOLUME_DESCRIPTOR_OFFSET, STANDARD_IDENTIFIER_OFFSET, VOLUME_IDENTIFIER_RANGE,
        VOLUME_SPACE_SIZE_OFFSET,
    };
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-disk-image-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// Builds a minimal file with a real ISO 9660 Primary Volume Descriptor
    /// at the correct offset: a `CD001` identifier, a volume label, a
    /// logical block size, and a volume space size (block count).
    fn write_test_iso(path: &std::path::Path, label: &str, block_count: u32) {
        let start = usize::try_from(PRIMARY_VOLUME_DESCRIPTOR_OFFSET).unwrap();
        let mut bytes = vec![0u8; start + 140];
        bytes[start] = 1; // type code: primary volume descriptor
        bytes[start + STANDARD_IDENTIFIER_OFFSET..start + STANDARD_IDENTIFIER_OFFSET + 5]
            .copy_from_slice(b"CD001");
        bytes[start + 6] = 1; // version

        let label_field =
            &mut bytes[start + VOLUME_IDENTIFIER_RANGE.start..start + VOLUME_IDENTIFIER_RANGE.end];
        label_field.fill(b' ');
        label_field[..label.len()].copy_from_slice(label.as_bytes());

        bytes[start + VOLUME_SPACE_SIZE_OFFSET..start + VOLUME_SPACE_SIZE_OFFSET + 4]
            .copy_from_slice(&block_count.to_le_bytes());
        bytes[start + LOGICAL_BLOCK_SIZE_OFFSET..start + LOGICAL_BLOCK_SIZE_OFFSET + 2]
            .copy_from_slice(&2048u16.to_le_bytes());

        std::fs::write(path, bytes).unwrap();
    }

    fn read_prefix(path: &std::path::Path) -> Vec<u8> {
        std::fs::read(path).unwrap()
    }

    #[test]
    fn sniffs_a_real_primary_volume_descriptor() {
        let path = unique_temp_file("sniff.iso");
        write_test_iso(&path, "MY_VOLUME", 100);

        assert!(DiskImageCore.sniff(&read_prefix(&path)));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn does_not_sniff_a_short_or_unrelated_file() {
        assert!(!DiskImageCore.sniff(b"not a disk image"));
        assert!(!DiskImageCore.sniff(&[0u8; 100]));
    }

    #[test]
    fn views_a_real_iso_and_reads_its_volume_descriptor() {
        let path = unique_temp_file("test.iso");
        write_test_iso(&path, "MY_VOLUME", 100);

        let data = DiskImageCore.view(&path).unwrap();
        let view: DiskImageView = serde_json::from_value(data).unwrap();

        assert_eq!(view.format, "ISO 9660");
        assert_eq!(view.volume_identifier.as_deref(), Some("MY_VOLUME"));
        assert_eq!(view.logical_block_size, 2048);
        assert_eq!(view.block_count, 100);
        assert_eq!(view.volume_size, 204_800);
        assert!(view.file_size > 0);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_an_iso_with_a_blank_volume_label() {
        let path = unique_temp_file("blank.iso");
        write_test_iso(&path, "", 10);

        let data = DiskImageCore.view(&path).unwrap();
        let view: DiskImageView = serde_json::from_value(data).unwrap();

        assert_eq!(view.volume_identifier, None);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_volume_metadata() {
        let data = serde_json::to_value(DiskImageView {
            format: "ISO 9660".to_owned(),
            volume_identifier: Some("MY_VOLUME".to_owned()),
            logical_block_size: 2048,
            block_count: 100,
            volume_size: 204_800,
            file_size: 204_800,
        })
        .unwrap();

        let lines = DiskImagePresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "ISO 9660 disk image",
                "MY_VOLUME",
                "100 blocks x 2048 bytes = 204800 bytes",
                "204800 bytes on disk",
            ]
        );
    }
}
