//! Archive file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of entries listed in the view; archives with more are
/// truncated, matching §2.1's parse limits.
const MAX_ENTRIES: usize = 200;

/// One entry in an archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveEntry {
    /// The entry's path within the archive.
    pub name: String,
    /// Uncompressed size in bytes.
    pub size: u64,
}

/// View data produced by [`ArchiveCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveView {
    /// Total number of entries in the archive.
    pub entry_count: usize,
    /// The first [`MAX_ENTRIES`] entries.
    pub entries: Vec<ArchiveEntry>,
}

/// The archive plugin's core half. Recognises ZIP archives.
#[derive(Debug, Default)]
pub struct ArchiveCore;

impl PluginCore for ArchiveCore {
    fn name(&self) -> &'static str {
        "archive"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        prefix.starts_with(b"PK\x03\x04")
            || prefix.starts_with(b"PK\x05\x06")
            || prefix.starts_with(b"PK\x07\x08")
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let file = std::fs::File::open(path)?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let entry_count = archive.len();
        let mut entries = Vec::with_capacity(entry_count.min(MAX_ENTRIES));
        for index in 0..entry_count.min(MAX_ENTRIES) {
            let entry = archive
                .by_index(index)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            entries.push(ArchiveEntry {
                name: entry.name().to_owned(),
                size: entry.size(),
            });
        }
        let view = ArchiveView {
            entry_count,
            entries,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// Extracts every entry in the archive at `archive_path` into
/// `destination`, creating it if needed. The operation this plugin offers,
/// per GUIDANCE.md §3.
///
/// # Errors
/// Returns an error if the archive cannot be read or an entry cannot be
/// written under `destination`.
pub fn extract(archive_path: &Path, destination: &Path) -> io::Result<()> {
    std::fs::create_dir_all(destination)?;
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    archive
        .extract(destination)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

/// The archive plugin's presentation half.
#[derive(Debug, Default)]
pub struct ArchivePresentation;

impl PluginPresentation for ArchivePresentation {
    fn name(&self) -> &'static str {
        "archive"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: ArchiveView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!("{} entries", view.entry_count)];
        lines.extend(
            view.entries
                .iter()
                .map(|entry| format!("{} ({} bytes)", entry.name, entry.size)),
        );
        if view.entry_count > view.entries.len() {
            lines.push(format!(
                "... {} more entries not shown",
                view.entry_count - view.entries.len()
            ));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{ArchiveCore, ArchivePresentation, ArchiveView};
    use plugin_api::{PluginCore, PluginPresentation};
    use std::io::Write;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-archive-test-{}-{name}",
            std::process::id()
        ))
    }

    fn write_test_zip(path: &std::path::Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("hello.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"hello, archive").unwrap();
        writer
            .start_file("nested/world.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"world").unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn sniffs_the_zip_local_file_header_magic() {
        assert!(ArchiveCore.sniff(b"PK\x03\x04rest of header"));
        assert!(!ArchiveCore.sniff(b"not a zip"));
    }

    #[test]
    fn views_a_real_zip_archive() {
        let path = unique_temp_file("test.zip");
        write_test_zip(&path);

        let data = ArchiveCore.view(&path).unwrap();
        let view: ArchiveView = serde_json::from_value(data).unwrap();

        assert_eq!(view.entry_count, 2);
        assert_eq!(view.entries[0].name, "hello.txt");
        assert_eq!(view.entries[0].size, 14);
        assert_eq!(view.entries[1].name, "nested/world.txt");

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_entry_count_and_names() {
        let data = serde_json::to_value(ArchiveView {
            entry_count: 1,
            entries: vec![super::ArchiveEntry {
                name: "a.txt".to_owned(),
                size: 5,
            }],
        })
        .unwrap();

        let lines = ArchivePresentation.present(&data);

        assert_eq!(lines, vec!["1 entries", "a.txt (5 bytes)"]);
    }

    #[test]
    fn extracts_a_real_zip_archive_to_a_destination() {
        let archive_path = unique_temp_file("extract-source.zip");
        write_test_zip(&archive_path);
        let destination = unique_temp_file("extract-destination");

        super::extract(&archive_path, &destination).unwrap();

        assert_eq!(
            std::fs::read_to_string(destination.join("hello.txt")).unwrap(),
            "hello, archive"
        );
        assert_eq!(
            std::fs::read_to_string(destination.join("nested").join("world.txt")).unwrap(),
            "world"
        );

        std::fs::remove_file(&archive_path).unwrap();
        std::fs::remove_dir_all(&destination).unwrap();
    }
}
