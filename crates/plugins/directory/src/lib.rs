//! Directory-as-file type plugin: core and presentation halves.
//!
//! Unlike the other plugins, this one is never reached by content-based
//! sniffing (a directory has no bytes to read a prefix from). `service`
//! special-cases directories and dispatches to it directly by name before
//! attempting `sniff`; [`DirectoryCore::sniff`] always returns `false` and
//! exists only to satisfy the trait.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// View data produced by [`DirectoryCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryView {
    /// Number of immediate entries in the directory.
    pub entry_count: u64,
    /// Combined size in bytes of immediate entries whose size is known
    /// (subdirectories are not recursed into).
    pub total_size: u64,
}

/// The directory-as-file plugin's core half.
#[derive(Debug, Default)]
pub struct DirectoryCore;

impl PluginCore for DirectoryCore {
    fn name(&self) -> &'static str {
        "directory"
    }

    fn sniff(&self, _prefix: &[u8]) -> bool {
        false
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let mut entry_count = 0u64;
        let mut total_size = 0u64;
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            entry_count += 1;
            // Only files, not subdirectories: a directory's own metadata
            // size is a filesystem-block-size artifact (e.g. ~4096 bytes on
            // Linux ext4, but 0 on Windows NTFS), not meaningful content
            // size, and summing it would make this platform-dependent.
            if entry.file_type().is_ok_and(|file_type| file_type.is_file())
                && let Ok(metadata) = entry.metadata()
            {
                total_size += metadata.len();
            }
        }
        let view = DirectoryView {
            entry_count,
            total_size,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The directory-as-file plugin's presentation half.
#[derive(Debug, Default)]
pub struct DirectoryPresentation;

impl PluginPresentation for DirectoryPresentation {
    fn name(&self) -> &'static str {
        "directory"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        match serde_json::from_value::<DirectoryView>(data.clone()) {
            Ok(view) => vec![
                format!("{} entries", view.entry_count),
                format!("{} bytes total", view.total_size),
            ],
            Err(err) => vec![format!("could not read view data: {err}")],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DirectoryCore, DirectoryPresentation, DirectoryView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rse-plugin-dir-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn sniff_always_returns_false() {
        assert!(!DirectoryCore.sniff(b""));
        assert!(!DirectoryCore.sniff(b"anything"));
    }

    #[test]
    fn views_a_real_directory() {
        let dir = unique_temp_dir("view");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"12345").unwrap();
        std::fs::write(dir.join("b.txt"), b"1234567890").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();

        let data = DirectoryCore.view(&dir).unwrap();
        let view: DirectoryView = serde_json::from_value(data).unwrap();

        assert_eq!(view.entry_count, 3);
        assert_eq!(view.total_size, 15);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn presents_entry_count_and_total_size() {
        let data = serde_json::to_value(DirectoryView {
            entry_count: 4,
            total_size: 1024,
        })
        .unwrap();

        let lines = DirectoryPresentation.present(&data);

        assert_eq!(lines, vec!["4 entries", "1024 bytes total"]);
    }
}
