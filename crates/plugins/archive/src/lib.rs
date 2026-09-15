//! Archive file type plugin: core and presentation halves.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
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
/// per GUIDANCE.md §3.
///
/// Returns the paths the extraction brought into being - the shallowest
/// path above each entry that did not exist beforehand - so an undo can
/// remove exactly those. Without it the service could only remember the
/// destination as a whole, and extracting into a folder the reader
/// already had meant Ctrl+Z sent that folder, and everything they had put
/// in it, to the recycle bin.
///
/// A file the archive overwrites is not among them: it existed before, so
/// removing it would destroy the reader's name for it, and its old
/// contents are not kept anywhere to put back.
///
/// # Errors
/// Returns an error if the archive cannot be read or an entry cannot be
/// written under `destination`.
pub fn extract(archive_path: &Path, destination: &Path) -> io::Result<Vec<PathBuf>> {
    // Open and validate the archive before creating the destination, so
    // extracting something that is not an archive leaves no empty directory
    // behind.
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    // Measured before anything is written: afterwards every path exists and
    // there is no telling which were already there.
    let created = paths_extraction_creates(&mut archive, destination);
    std::fs::create_dir_all(destination)?;
    archive
        .extract(destination)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    Ok(created)
}

/// Whether anything at all is at `path`. A dangling symlink counts, so it is
/// never mistaken for room and later removed.
fn occupied(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// The shallowest not-yet-existing path above each entry in `archive`, as it
/// would land under `destination`.
fn paths_extraction_creates<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    destination: &Path,
) -> Vec<PathBuf> {
    if !occupied(destination) {
        // Everything lands inside a folder this extraction makes, including
        // any missing parents `create_dir_all` makes on the way to it.
        let mut top = destination.to_path_buf();
        while let Some(parent) = top.parent() {
            if parent.as_os_str().is_empty() || occupied(parent) {
                break;
            }
            top = parent.to_path_buf();
        }
        return vec![top];
    }
    let mut created: Vec<PathBuf> = Vec::new();
    for index in 0..archive.len() {
        // An entry whose name would escape the destination is one `extract`
        // refuses to write, so it creates nothing to record.
        let Some(relative) = archive
            .by_index_raw(index)
            .ok()
            .and_then(|entry| entry.enclosed_name())
        else {
            continue;
        };
        let mut path = destination.to_path_buf();
        for component in relative.components() {
            path.push(component);
            if !occupied(&path) {
                if !created.iter().any(|done| path.starts_with(done)) {
                    created.push(path.clone());
                }
                break;
            }
        }
    }
    created
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

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::ArchiveCore),
            plugin_api::PluginPresentation::extensions(&crate::ArchivePresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    /// A destination that did not exist is what the extraction made, so it
    /// is the one path reported - and a missing parent above it is reported
    /// in its place, because `create_dir_all` made that too.
    #[test]
    fn a_new_destination_is_reported_as_the_one_thing_made() {
        let root = unique_temp_file("created-new-destination");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let archive_path = root.join("source.zip");
        write_test_zip(&archive_path);

        let created = super::extract(&archive_path, &root.join("out")).unwrap();
        assert_eq!(created, vec![root.join("out")]);

        let created = super::extract(&archive_path, &root.join("missing").join("out")).unwrap();
        assert_eq!(
            created,
            vec![root.join("missing")],
            "the parent was made on the way, so it is what an undo has to remove"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Into a folder that already exists, only the entries that were not
    /// there are reported - never the folder, and never what was in it.
    #[test]
    fn an_existing_destination_reports_only_what_the_archive_added() {
        let root = unique_temp_file("created-existing-destination");
        let _ = std::fs::remove_dir_all(&root);
        let destination = root.join("out");
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(destination.join("keepsake.txt"), "mine").unwrap();
        let archive_path = root.join("source.zip");
        write_test_zip(&archive_path);

        let mut created = super::extract(&archive_path, &destination).unwrap();
        created.sort();

        assert_eq!(
            created,
            vec![destination.join("hello.txt"), destination.join("nested")],
            "a folder the archive brings in is reported once, not once per file in it"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A new file inside a folder that was already there is reported as the
    /// file, not the folder.
    #[test]
    fn a_new_file_in_an_existing_folder_is_reported_without_its_folder() {
        let root = unique_temp_file("created-inside-existing-folder");
        let _ = std::fs::remove_dir_all(&root);
        let destination = root.join("out");
        std::fs::create_dir_all(destination.join("nested")).unwrap();
        std::fs::write(destination.join("nested").join("mine.txt"), "mine").unwrap();
        let archive_path = root.join("source.zip");
        write_test_zip(&archive_path);

        let mut created = super::extract(&archive_path, &destination).unwrap();
        created.sort();

        assert_eq!(
            created,
            vec![
                destination.join("hello.txt"),
                destination.join("nested").join("world.txt"),
            ]
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A file the archive overwrites existed before, so it is not reported:
    /// removing it on undo would destroy the reader's name for it, and its
    /// old contents are not kept anywhere to put back.
    #[test]
    fn a_file_the_archive_overwrites_is_not_reported_as_made() {
        let root = unique_temp_file("created-collision");
        let _ = std::fs::remove_dir_all(&root);
        let destination = root.join("out");
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(destination.join("hello.txt"), "already here").unwrap();
        let archive_path = root.join("source.zip");
        write_test_zip(&archive_path);

        let created = super::extract(&archive_path, &destination).unwrap();

        assert!(
            !created.contains(&destination.join("hello.txt")),
            "a file that was already there was not made by this extraction: {created:?}"
        );
        assert_eq!(created, vec![destination.join("nested")]);

        std::fs::remove_dir_all(&root).unwrap();
    }
}
