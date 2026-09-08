//! What a directory is, as a source control working copy.
//!
//! This lives with the directory plugin because a working copy *is* a
//! directory, and GUIDANCE.md 2.4 makes the File pane the plugin's to fill:
//! the pane says which provider a checkout came from and which branch is
//! out because this module read it. `service` calls the same function when
//! it builds a listing, so a row and its preview cannot disagree.
//!
//! Per decision D10 this describes and never drives. Nothing here runs a
//! source control command, and nothing here writes to a working copy.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The largest `.git/config` this will read. Git's own configuration is
/// small, and this only wants the remote's address out of it.
const MAX_GIT_CONFIG_BYTES: u64 = 1024 * 1024;

/// What the application knows about a source control working directory,
/// read from the checkout itself rather than guessed from its name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repository {
    /// The host the checkout came from - `github.com`, `gitlab.com`,
    /// `bitbucket.org`, `dev.azure.com`, or whatever else its remote names.
    /// `None` for a checkout with no remote configured.
    pub provider: Option<String>,
    /// The branch checked out, or `None` when the working copy is not on a
    /// branch - a detached head, mid-rebase, or a clone with no commits.
    pub branch: Option<String>,
    /// The address the checkout tracks, as written in its own configuration.
    pub remote: Option<String>,
    /// Whether the working tree has uncommitted changes. Always `None` for
    /// now: answering it needs a walk of the work tree, deferred with the
    /// operations that will need the same walk (decision D10).
    pub dirty: Option<bool>,
}

/// What `path` is as a working copy, or `None` if it is not one.
///
/// A working copy is recognised by its `.git` marker and nothing else -
/// never by its name, and never by asking a source control tool. The marker
/// is a directory in an ordinary clone and a file in a worktree or
/// submodule, and both count.
#[must_use]
pub fn describe(path: &Path) -> Option<Repository> {
    let marker = path.join(".git");
    let git_dir = if marker.is_dir() {
        marker
    } else if marker.is_file() {
        worktree_git_dir(&marker)?
    } else {
        return None;
    };

    let remote = remote_url(&git_dir);
    Some(Repository {
        provider: remote.as_deref().and_then(provider_of),
        branch: branch_at(&git_dir),
        remote,
        // Deferred with the operations that need the same walk of the work
        // tree; see decision D10.
        dirty: None,
    })
}

/// The real git directory a worktree or submodule's `.git` file points at.
fn worktree_git_dir(marker: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(marker).ok()?;
    let target = text.lines().find_map(|line| line.strip_prefix("gitdir:"))?;
    let target = PathBuf::from(target.trim());
    let resolved = if target.is_absolute() {
        target
    } else {
        marker.parent()?.join(target)
    };
    resolved.is_dir().then_some(resolved)
}

/// The branch checked out, read from `HEAD`.
///
/// `HEAD` holds `ref: refs/heads/<branch>` on a branch, and a raw commit
/// identifier when the head is detached - which is not a branch, and is
/// reported as none rather than as a name nobody would recognise.
fn branch_at(git_dir: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let reference = head.trim().strip_prefix("ref:")?.trim();
    let branch = reference.strip_prefix("refs/heads/").unwrap_or(reference);
    (!branch.is_empty()).then(|| branch.to_owned())
}

/// The address of the remote the checkout tracks, from its own `config`.
///
/// Prefers `origin`, which is what a clone writes, and otherwise takes the
/// first remote it finds - a checkout renamed to `upstream` still came from
/// somewhere.
fn remote_url(git_dir: &Path) -> Option<String> {
    let config = git_dir.join("config");
    if std::fs::metadata(&config).ok()?.len() > MAX_GIT_CONFIG_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(config).ok()?;

    let mut section: Option<String> = None;
    let mut first: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = header
                .strip_prefix("remote ")
                .map(|name| name.trim_matches('"').to_owned());
            continue;
        }
        let Some(name) = section.as_deref() else {
            continue;
        };
        let Some(url) = line.strip_prefix("url") else {
            continue;
        };
        let Some(url) = url.trim_start().strip_prefix('=') else {
            continue;
        };
        let url = url.trim().to_owned();
        if name == "origin" {
            return Some(url);
        }
        if first.is_none() {
            first = Some(url);
        }
    }
    first
}

/// The host in a remote's address: `github.com`, `gitlab.com`,
/// `bitbucket.org`, `dev.azure.com`, a self-hosted name, whatever it is.
///
/// Handles the two shapes a remote is written in - a uniform resource
/// locator (`https://github.com/owner/name.git`) and the secure shell short
/// form (`git@github.com:owner/name.git`) - and gives up rather than
/// guessing on anything else, including a purely local path.
fn provider_of(remote: &str) -> Option<String> {
    let remote = remote.trim();
    if remote.is_empty() {
        return None;
    }

    if let Some((_scheme, rest)) = remote.split_once("://") {
        let authority = rest.split(['/', '?', '#']).next()?;
        // A password would sit between the credentials and the host, and
        // neither belongs in a listing.
        let host = authority.rsplit('@').next()?;
        let host = host.split(':').next()?;
        return (!host.is_empty()).then(|| host.to_ascii_lowercase());
    }

    if let Some((credentials, rest)) = remote.split_once('@')
        && !credentials.contains('/')
    {
        let host = rest.split([':', '/']).next()?;
        return (!host.is_empty()).then(|| host.to_ascii_lowercase());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{describe, provider_of};
    use std::path::Path;

    /// A directory holding a `.git` directory with the files a clone has.
    fn write_checkout(dir: &Path, head: &str, config: &str) {
        let git = dir.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), head).unwrap();
        std::fs::write(git.join("config"), config).unwrap();
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rse-repository-{}-{}-{name}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_checkout_reports_its_provider_branch_and_remote() {
        let dir = temp_dir("clone");
        write_checkout(
            &dir,
            "ref: refs/heads/main\n",
            "[core]\n\trepositoryformatversion = 0\n\
             [remote \"origin\"]\n\turl = https://github.com/owner/name.git\n\
             \tfetch = +refs/heads/*:refs/remotes/origin/*\n",
        );

        let found = describe(&dir).expect("a directory with a .git marker is a working copy");

        assert_eq!(found.provider.as_deref(), Some("github.com"));
        assert_eq!(found.branch.as_deref(), Some("main"));
        assert_eq!(
            found.remote.as_deref(),
            Some("https://github.com/owner/name.git")
        );
        assert_eq!(found.dirty, None, "status is deferred with the operations");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_plain_folder_is_not_a_working_copy() {
        let dir = temp_dir("plain");
        std::fs::write(dir.join("notes.txt"), b"no checkout here").unwrap();

        assert!(
            describe(&dir).is_none(),
            "a folder without a .git marker must stay a folder"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_detached_head_names_no_branch() {
        let dir = temp_dir("detached");
        write_checkout(
            &dir,
            "9f0c2b6e6f2a4f3e9a1d77c0b2f1a0049f0c2b6e\n",
            "[remote \"origin\"]\n\turl = git@gitlab.com:group/project.git\n",
        );

        let found = describe(&dir).expect("still a working copy");

        assert_eq!(found.branch, None, "a detached head is not on a branch");
        assert_eq!(found.provider.as_deref(), Some("gitlab.com"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_checkout_with_no_remote_still_reports_its_branch() {
        let dir = temp_dir("local-only");
        write_checkout(&dir, "ref: refs/heads/trunk\n", "[core]\n\tbare = false\n");

        let found = describe(&dir).expect("still a working copy");

        assert_eq!(found.branch.as_deref(), Some("trunk"));
        assert_eq!(found.provider, None);
        assert_eq!(found.remote, None);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_remote_other_than_origin_is_used_when_that_is_all_there_is() {
        let dir = temp_dir("upstream");
        write_checkout(
            &dir,
            "ref: refs/heads/main\n",
            "[remote \"upstream\"]\n\turl = https://dev.azure.com/org/project/_git/repo\n",
        );

        let found = describe(&dir).expect("still a working copy");

        assert_eq!(found.provider.as_deref(), Some("dev.azure.com"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn origin_wins_when_a_checkout_has_several_remotes() {
        let dir = temp_dir("several");
        write_checkout(
            &dir,
            "ref: refs/heads/main\n",
            "[remote \"upstream\"]\n\turl = https://gitlab.com/group/fork.git\n\
             [remote \"origin\"]\n\turl = https://bitbucket.org/team/name.git\n",
        );

        let found = describe(&dir).expect("still a working copy");

        assert_eq!(found.provider.as_deref(), Some("bitbucket.org"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_worktree_marker_file_is_followed_to_the_real_git_directory() {
        let dir = temp_dir("worktree-parent");
        let real = dir.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("HEAD"), "ref: refs/heads/feature\n").unwrap();
        std::fs::write(
            real.join("config"),
            "[remote \"origin\"]\n\turl = git@github.com:owner/name.git\n",
        )
        .unwrap();

        let checkout = dir.join("checkout");
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(
            checkout.join(".git"),
            format!("gitdir: {}\n", real.display()),
        )
        .unwrap();

        let found = describe(&checkout).expect("a worktree is a working copy too");

        assert_eq!(found.branch.as_deref(), Some("feature"));
        assert_eq!(found.provider.as_deref(), Some("github.com"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_provider_is_read_from_either_shape_of_remote_address() {
        assert_eq!(
            provider_of("https://github.com/owner/name.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            provider_of("git@github.com:owner/name.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            provider_of("ssh://git@git.example.co.uk:2222/owner/name.git").as_deref(),
            Some("git.example.co.uk"),
            "a port is not part of the host"
        );
        assert_eq!(
            provider_of("https://user:secret@gitlab.example.com/g/p.git").as_deref(),
            Some("gitlab.example.com"),
            "credentials do not belong in a listing"
        );
        assert_eq!(
            provider_of("/srv/git/name.git"),
            None,
            "a local path came from no provider"
        );
        assert_eq!(provider_of(""), None);
    }
}
