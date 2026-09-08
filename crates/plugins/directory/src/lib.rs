//! Directory-as-file type plugin: core and presentation halves.
//!
//! Unlike the other plugins, this one is never reached by content-based
//! sniffing (a directory has no bytes to read a prefix from). `service`
//! special-cases directories and dispatches to it directly by name before
//! attempting `sniff`; [`DirectoryCore::sniff`] always returns `false` and
//! exists only to satisfy the trait.

pub mod repository;

use plugin_api::{Icon, PluginCore, PluginPresentation};
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
    /// What this directory is as a source control working copy, or `None`
    /// when it is an ordinary folder - which is not a lesser thing, just a
    /// different one (GUIDANCE.md 2.5).
    #[serde(default)]
    pub repository: Option<repository::Repository>,
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
            repository: repository::describe(path),
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// `"entry"` or `"entries"`, so a count of one does not read as "1 entries".
fn entries_noun(count: u64) -> &'static str {
    if count == 1 { "entry" } else { "entries" }
}

/// The directory-as-file plugin's presentation half.
#[derive(Debug, Default)]
pub struct DirectoryPresentation;

impl PluginPresentation for DirectoryPresentation {
    fn name(&self) -> &'static str {
        "directory"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "DIR",
            tint: 0x00dc_b67a,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        &[]
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view = match serde_json::from_value::<DirectoryView>(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };

        let mut lines = Vec::new();
        // A working copy leads with what it is: which provider it came from
        // and what is checked out. The folder facts follow, because they are
        // the less interesting half for a repository.
        if let Some(repository) = &view.repository {
            lines.push("Source control working copy".to_owned());
            if let Some(provider) = &repository.provider {
                lines.push(format!("Provider: {provider}"));
            }
            if let Some(branch) = &repository.branch {
                lines.push(format!("Branch: {branch}"));
            } else {
                lines.push("Branch: none checked out (detached head)".to_owned());
            }
            if let Some(remote) = &repository.remote {
                lines.push(format!("Remote: {remote}"));
            } else {
                lines.push("Remote: none configured".to_owned());
            }
            lines.push(String::new());
        }

        lines.push(format!(
            "{} {}",
            view.entry_count,
            entries_noun(view.entry_count)
        ));
        lines.push(format!("{} bytes total", view.total_size));
        lines
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
            repository: None,
        })
        .unwrap();

        let lines = DirectoryPresentation.present(&data);

        assert_eq!(lines, vec!["4 entries", "1024 bytes total"]);
    }

    #[test]
    fn presents_a_single_entry_in_the_singular() {
        let view = DirectoryView {
            entry_count: 1,
            total_size: 10,
            repository: None,
        };
        let data = serde_json::to_value(view).unwrap();

        let lines = DirectoryPresentation.present(&data);

        assert_eq!(lines, vec!["1 entry", "10 bytes total"]);
    }

    #[test]
    fn a_working_copy_leads_with_where_it_came_from() {
        let data = serde_json::to_value(DirectoryView {
            entry_count: 12,
            total_size: 4096,
            repository: Some(super::repository::Repository {
                provider: Some("github.com".to_owned()),
                branch: Some("main".to_owned()),
                remote: Some("https://github.com/owner/name.git".to_owned()),
                dirty: None,
            }),
        })
        .unwrap();

        let lines = DirectoryPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "Source control working copy",
                "Provider: github.com",
                "Branch: main",
                "Remote: https://github.com/owner/name.git",
                "",
                "12 entries",
                "4096 bytes total",
            ]
        );
    }

    #[test]
    fn a_working_copy_says_so_even_when_it_has_no_remote_or_branch() {
        let data = serde_json::to_value(DirectoryView {
            entry_count: 3,
            total_size: 90,
            repository: Some(super::repository::Repository::default()),
        })
        .unwrap();

        let lines = DirectoryPresentation.present(&data);

        assert!(lines.contains(&"Source control working copy".to_owned()));
        assert!(lines.contains(&"Branch: none checked out (detached head)".to_owned()));
        assert!(lines.contains(&"Remote: none configured".to_owned()));
    }

    #[test]
    fn an_ordinary_folder_says_nothing_about_source_control() {
        let data = serde_json::to_value(DirectoryView {
            entry_count: 2,
            total_size: 15,
            repository: None,
        })
        .unwrap();

        let lines = DirectoryPresentation.present(&data);

        assert_eq!(lines, vec!["2 entries", "15 bytes total"]);
    }
}
