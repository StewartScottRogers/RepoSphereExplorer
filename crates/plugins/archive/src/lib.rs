//! Archive file type plugin: core and presentation halves.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
/// Only `zip`. This plugin sniffs `PK` and reads a zip archive; it has
/// never been able to read a tar, a gzip, a bzip2, an xz, a 7-Zip or a
/// rar, and an extension it cannot honour is a hint that points
/// nowhere - the hint chooses only between plugins that already
/// recognised the bytes (GUIDANCE.md section 3.3). The five formats
/// that now have plugins of their own take their extensions with them.
pub const EXTENSIONS: &[&str] = &["zip"];

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
    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

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
/// per GUIDANCE.md §3. Returns every path the extraction created there, so
/// a caller that already had a `destination` can undo precisely - removing
/// what extraction added, not what it found - instead of removing
/// `destination` whole.
///
/// # Errors
/// Returns an error if the archive cannot be read, an entry's path would
/// overwrite a file already at `destination`, or an entry cannot be
/// written.
pub fn extract(archive_path: &Path, destination: &Path) -> io::Result<Vec<PathBuf>> {
    // Open and validate the archive before creating the destination, so
    // extracting something that is not an archive leaves no empty directory
    // behind.
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    // Refuse a collision before writing anything. `ZipArchive::extract`
    // overwrites files that are already there, and an overwrite undo could
    // never put back is not a defensible way to merge into a destination a
    // reader already has something in.
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        if entry.is_file()
            && let Some(name) = entry.enclosed_name()
            && destination.join(name).is_file()
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "{} already exists in {}",
                    entry.name(),
                    destination.display()
                ),
            ));
        }
    }

    let destination_existed = destination.exists();
    let before: HashSet<PathBuf> = if destination_existed {
        paths_under(destination)?
    } else {
        HashSet::new()
    };

    std::fs::create_dir_all(destination)?;
    archive
        .extract(destination)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    if !destination_existed {
        return Ok(vec![destination.to_path_buf()]);
    }

    let mut created: Vec<PathBuf> = paths_under(destination)?
        .into_iter()
        .filter(|path| !before.contains(path))
        .collect();
    // Shallowest first: undo reverses this list, so the deepest paths -
    // the ones nested inside another path this same extraction created -
    // are removed before the directory that holds them.
    created.sort_by_key(|path| path.components().count());
    Ok(created)
}

/// Every file and directory somewhere under `root`, `root` itself excluded.
fn paths_under(root: &Path) -> io::Result<HashSet<PathBuf>> {
    let mut paths = HashSet::new();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                directories.push(path.clone());
            }
            paths.insert(path);
        }
    }
    Ok(paths)
}

/// `"entry"` or `"entries"`, so a count of one does not read as "1 entries".
fn entries_noun(count: usize) -> &'static str {
    if count == 1 { "entry" } else { "entries" }
}

/// The archive plugin's presentation half.
#[derive(Debug, Default)]
pub struct ArchivePresentation;

impl PluginPresentation for ArchivePresentation {
    fn name(&self) -> &'static str {
        "archive"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "ZIP",
            tint: 0x00b8_860b,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: ArchiveView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "{} {}",
            view.entry_count,
            entries_noun(view.entry_count)
        )];
        lines.extend(
            view.entries
                .iter()
                .map(|entry| format!("{} ({} bytes)", entry.name, entry.size)),
        );
        if view.entry_count > view.entries.len() {
            let remaining = view.entry_count - view.entries.len();
            lines.push(format!(
                "... {remaining} more {} not shown",
                entries_noun(remaining)
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
    fn a_failed_extract_leaves_no_destination_directory_behind() {
        let archive = unique_temp_file("not-an-archive.zip");
        let destination = unique_temp_file("not-an-archive-out");
        std::fs::write(&archive, b"plain text, not a zip").unwrap();

        let err = super::extract(&archive, &destination).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(!destination.exists());

        std::fs::remove_file(&archive).unwrap();
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

        assert_eq!(lines, vec!["1 entry", "a.txt (5 bytes)"]);
    }

    #[test]
    fn extracts_a_real_zip_archive_to_a_destination() {
        let archive_path = unique_temp_file("extract-source.zip");
        write_test_zip(&archive_path);
        let destination = unique_temp_file("extract-destination");

        let created = super::extract(&archive_path, &destination).unwrap();

        assert_eq!(
            std::fs::read_to_string(destination.join("hello.txt")).unwrap(),
            "hello, archive"
        );
        assert_eq!(
            std::fs::read_to_string(destination.join("nested").join("world.txt")).unwrap(),
            "world"
        );
        assert_eq!(
            created,
            vec![destination.clone()],
            "a destination that did not exist is one created path, not one per entry"
        );

        std::fs::remove_file(&archive_path).unwrap();
        std::fs::remove_dir_all(&destination).unwrap();
    }

    #[test]
    fn extracting_into_an_existing_destination_reports_only_what_it_added() {
        let archive_path = unique_temp_file("extract-merge-source.zip");
        write_test_zip(&archive_path);
        let destination = unique_temp_file("extract-merge-destination");
        std::fs::create_dir_all(&destination).unwrap();
        let already_there = destination.join("already-there.txt");
        std::fs::write(&already_there, "the reader's own notes").unwrap();

        let created = super::extract(&archive_path, &destination).unwrap();

        assert_eq!(
            std::fs::read_to_string(&already_there).unwrap(),
            "the reader's own notes",
            "extracting must not touch what was already in the destination"
        );
        let created: std::collections::HashSet<_> = created.into_iter().collect();
        assert!(created.contains(&destination.join("hello.txt")));
        assert!(created.contains(&destination.join("nested")));
        assert!(created.contains(&destination.join("nested").join("world.txt")));
        assert!(!created.contains(&destination));
        assert!(!created.contains(&already_there));

        std::fs::remove_file(&archive_path).unwrap();
        std::fs::remove_dir_all(&destination).unwrap();
    }

    #[test]
    fn extraction_refuses_an_entry_that_collides_with_an_existing_file() {
        let archive_path = unique_temp_file("extract-collision-source.zip");
        write_test_zip(&archive_path);
        let destination = unique_temp_file("extract-collision-destination");
        std::fs::create_dir_all(&destination).unwrap();
        let colliding = destination.join("hello.txt");
        std::fs::write(&colliding, "not what the archive has").unwrap();

        let err = super::extract(&archive_path, &destination).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(&colliding).unwrap(),
            "not what the archive has",
            "a refused extraction must not overwrite the file it collided with"
        );
        assert!(
            !destination.join("nested").exists(),
            "a refused extraction must not have written any other entry either"
        );

        std::fs::remove_file(&archive_path).unwrap();
        std::fs::remove_dir_all(&destination).unwrap();
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::ArchiveCore),
            plugin_api::PluginPresentation::extensions(&crate::ArchivePresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
