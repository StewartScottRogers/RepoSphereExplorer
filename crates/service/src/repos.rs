//! The Repos Directory, and what the application knows about the working
//! copies inside it.
//!
//! Two jobs, both of them reading rather than driving (decision D10):
//!
//! 1. Where the workspace is - the configured roots, stored per machine,
//!    with exactly one marked active (D9). The front ends ask for this at
//!    startup instead of remembering where anybody was last time (D7).
//! 2. What each folder in it is - whether it is a source control working
//!    copy, which host it came from, and which branch is checked out. All
//!    of it read out of the checkout's own files; nothing here runs a
//!    source control command, and nothing here writes to a working copy.

use protocol::{ReposRoot, RepositoryInfo};
use std::io;
use std::path::{Path, PathBuf};

/// The largest configuration file this will read. A settings file is a few
/// hundred bytes; anything past this is not one.
const MAX_CONFIG_BYTES: u64 = 64 * 1024;

/// `<data-local-dir>/RepoSphereExplorer/repos.json`, or `None` where the
/// platform reports no such directory.
///
/// The directory keeps the old product name deliberately: it holds live
/// user data, and renaming it without a migration would orphan the
/// settings and the journal beside it. Listed as a deferred rename in
/// `README.md`.
fn config_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("RepoSphereExplorer").join("repos.json"))
}

/// The Repos Directory this platform suggests when nothing is configured:
/// `Z:\repos` on Windows, `~/repos` everywhere else.
///
/// Suggested, never imposed - the first run offers it and takes whatever
/// the user gives instead.
#[must_use]
pub fn default_root() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"Z:\repos")
    } else {
        dirs::home_dir().map_or_else(|| PathBuf::from("repos"), |home| home.join("repos"))
    }
}

/// Every configured root, in the order they were added.
///
/// An empty list is what a first run looks like, and is not an error: the
/// file is missing, unreadable or malformed, and the caller offers
/// [`default_root`] instead.
#[must_use]
pub fn roots() -> Vec<ReposRoot> {
    let Some(path) = config_path() else {
        return Vec::new();
    };
    let Ok(metadata) = std::fs::metadata(&path) else {
        return Vec::new();
    };
    if metadata.len() > MAX_CONFIG_BYTES {
        return Vec::new();
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<ReposRoot>>(&text).unwrap_or_default()
}

/// The root the application opens at, or `None` when nothing is configured.
///
/// Exactly one root is active (D9). A stored list that somehow holds
/// several active entries yields the first, rather than refusing to open.
#[must_use]
pub fn active_root() -> Option<PathBuf> {
    roots()
        .into_iter()
        .find(|root| root.active)
        .map(|root| PathBuf::from(root.path))
}

/// `stored` with `wanted` active and everything else not, adding `wanted` if
/// the list does not already hold it.
///
/// Split from the writing so the rule it enforces - exactly one root is
/// active (decision D9) - is testable without touching the machine's real
/// settings.
#[must_use]
fn with_active(mut stored: Vec<ReposRoot>, wanted: &str) -> Vec<ReposRoot> {
    for root in &mut stored {
        root.active = root.path == wanted;
    }
    if !stored.iter().any(|root| root.path == wanted) {
        stored.push(ReposRoot {
            path: wanted.to_owned(),
            active: true,
        });
    }
    stored
}

/// Makes `path` the active root, adding it to the stored list if it is not
/// already there, and writes the list back.
///
/// # Errors
/// Returns an error if `path` is not a directory, or if the configuration
/// cannot be written.
pub fn set_active_root(path: &Path) -> io::Result<()> {
    if !path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not a directory", path.display()),
        ));
    }

    let stored = with_active(roots(), &path.to_string_lossy());

    let Some(config) = config_path() else {
        return Err(io::Error::other(
            "this platform reports no directory to store settings in",
        ));
    };
    if let Some(parent) = config.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(&stored)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    std::fs::write(&config, text)
}

/// What `path` is as a working copy, or `None` if it is not one.
///
/// The reading itself belongs to the directory plugin - a working copy is a
/// directory, and the File pane is the plugin's to fill - so a listing and a
/// preview cannot disagree about the same folder.
#[must_use]
pub fn describe(path: &Path) -> Option<RepositoryInfo> {
    plugin_directory::repository::describe(path).map(|found| RepositoryInfo {
        provider: found.provider,
        branch: found.branch,
        remote: found.remote,
        dirty: found.dirty,
    })
}

#[cfg(test)]
mod tests {
    use super::{ReposRoot, default_root, set_active_root, with_active};

    #[test]
    fn the_default_root_is_the_platform_convention() {
        let root = default_root();
        if cfg!(windows) {
            assert_eq!(root, std::path::PathBuf::from(r"Z:\repos"));
        } else {
            assert!(
                root.ends_with("repos"),
                "elsewhere the default sits under the home directory: {}",
                root.display()
            );
        }
    }

    #[test]
    fn making_a_root_active_adds_it_when_it_is_new() {
        let stored = with_active(Vec::new(), "/home/ada/repos");

        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].path, "/home/ada/repos");
        assert!(stored[0].active);
    }

    #[test]
    fn exactly_one_root_is_active_afterwards() {
        let stored = with_active(
            vec![
                ReposRoot {
                    path: "/one".to_owned(),
                    active: true,
                },
                ReposRoot {
                    path: "/two".to_owned(),
                    active: false,
                },
            ],
            "/two",
        );

        assert_eq!(stored.iter().filter(|root| root.active).count(), 1);
        assert!(
            stored
                .iter()
                .find(|root| root.path == "/two")
                .unwrap()
                .active
        );
        assert!(
            !stored
                .iter()
                .find(|root| root.path == "/one")
                .unwrap()
                .active
        );
    }

    #[test]
    fn a_root_already_stored_is_not_added_twice() {
        let stored = with_active(
            vec![ReposRoot {
                path: "/repos".to_owned(),
                active: false,
            }],
            "/repos",
        );

        assert_eq!(stored.len(), 1, "switching back is not a new root");
        assert!(stored[0].active);
    }

    #[test]
    fn the_other_roots_are_kept_so_several_can_come_later() {
        // Decision D9: one active at a time, stored as a list, so supporting
        // several later is a user interface change and not a migration.
        let stored = with_active(
            vec![
                ReposRoot {
                    path: "/work".to_owned(),
                    active: true,
                },
                ReposRoot {
                    path: "/personal".to_owned(),
                    active: false,
                },
            ],
            "/personal",
        );

        assert_eq!(stored.len(), 2);
    }

    #[test]
    fn set_active_root_refuses_a_path_that_is_not_a_directory() {
        let file = std::env::temp_dir().join(format!("rse-not-a-dir-{}", std::process::id()));
        std::fs::write(&file, b"a file, not a workspace").unwrap();

        let refused = set_active_root(&file);

        assert!(refused.is_err(), "a file is not a Repos Directory");
        std::fs::remove_file(&file).unwrap();
    }
}
