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
use std::time::SystemTime;

/// The largest `.git/config` this will read. Git's own configuration is
/// small, and this only wants the remote's address out of it.
const MAX_GIT_CONFIG_BYTES: u64 = 1024 * 1024;

/// What kind of working copy a [`Repository`] describes: an ordinary clone,
/// a linked worktree of one, or a submodule pinned by one (#587).
///
/// Detection only (D10): each variant's path comes from a marker already
/// being read to answer `describe`, never from running `git`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    /// An ordinary checkout: `.git` is its own directory.
    #[default]
    Clone,
    /// A linked worktree (`git worktree add`), sharing a clone's git
    /// directory. `clone` is that clone's working directory, taken from the
    /// worktree's `commondir` and the `gitdir` file that led to it -
    /// present whether or not it still exists, so a reader can be told when
    /// it is gone (`clone_exists`).
    Worktree {
        /// The clone's working directory.
        clone: PathBuf,
        /// Whether `clone` still exists.
        clone_exists: bool,
    },
    /// A submodule, pinned by an outer working copy. `outer` is that outer
    /// copy's own directory, from the `.git/modules/<name>` location the
    /// submodule's `.git` file points at.
    Submodule {
        /// The outer working copy's directory.
        outer: PathBuf,
    },
}

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
    /// Whether this is an ordinary clone, a linked worktree, or a submodule
    /// (#587).
    #[serde(default)]
    pub kind: Kind,
    /// What the working tree looks like against what was last staged.
    /// `None` when nothing asked - a listing does not, since the answer
    /// costs a pass over every tracked file - or when the checkout's index
    /// could not be read.
    pub status: Option<crate::status::WorkingTree>,
    /// How the branch stands against its upstream, as the last fetch left
    /// it. `None` when nothing asked - like `status`, it is for the selected
    /// repository only - or when there is no branch to compare.
    #[serde(default)]
    pub tracking: Option<crate::tracking::Tracking>,
    /// When the checkout was last worked in - the latest modification time
    /// among its `index`, `HEAD` and `logs/HEAD`, which change on checkout,
    /// commit, staging and branch switches, unlike the folder's own
    /// modification time (#588). Falls back to the folder's own time when
    /// none of the three exist.
    #[serde(default)]
    pub last_activity: Option<SystemTime>,
}

/// What `path` is as a working copy, or `None` if it is not one.
///
/// A working copy is recognised by its `.git` marker and nothing else -
/// never by its name, and never by asking a source control tool. The marker
/// is a directory in an ordinary clone and a file in a worktree or
/// submodule, and both count.
///
/// This is the cheap half: the marker, the remote, the branch, all of it a
/// few small file reads. It is what a *listing* can afford for every row.
/// For the working tree, which costs a pass over the tracked files, see
/// [`describe_with_status`].
#[must_use]
pub fn describe(path: &Path) -> Option<Repository> {
    let git_dir = git_dir_of(path)?;
    let remote = remote_url(&common_dir_of(&git_dir));
    Some(Repository {
        provider: remote.as_deref().and_then(provider_of),
        branch: branch_at(&git_dir),
        remote,
        kind: kind_of(path, &git_dir),
        // The listing asks about forty folders at once and can afford none
        // of this; `describe_with_status` answers it for the one the reader
        // selected.
        status: None,
        tracking: None,
        last_activity: last_activity_of(&git_dir, path),
    })
}

/// When the checkout at `git_dir` was last worked in: the latest of `index`,
/// `HEAD` and `logs/HEAD`'s own modification times, a stat of three files
/// affordable for every row of a listing (CLAUDE.md rule 9) - never a
/// directory read. A worktree's `git_dir` is already its own, per
/// [`git_dir_of`], so this reads what changed in *this* checkout, not the
/// clone it shares a config with. Falls back to `path`'s own folder time
/// when none of the three files exist.
fn last_activity_of(git_dir: &Path, path: &Path) -> Option<SystemTime> {
    ["index", "HEAD", "logs/HEAD"]
        .into_iter()
        .filter_map(|name| std::fs::metadata(git_dir.join(name)).ok()?.modified().ok())
        .max()
        .or_else(|| std::fs::metadata(path).ok()?.modified().ok())
}

/// Whether `path` is an ordinary clone, a linked worktree, or a submodule.
///
/// The marker tells clone from the other two: a directory is a clone's own.
/// Among the other two, only a worktree's own git directory carries a
/// `commondir` file (#587); a submodule's carries a real `config` of its
/// own instead, as [`common_dir_of`]'s own documentation explains.
fn kind_of(path: &Path, git_dir: &Path) -> Kind {
    if path.join(".git").is_dir() {
        return Kind::Clone;
    }
    // Real git writes `gitdir:`/`commondir` targets with `..` segments
    // rather than resolving them, and the OS follows those fine for a
    // file read - but a search for the `.git` path component needs them
    // resolved first, or it finds the literal `..` sitting in front of it.
    let git_dir = normalize(git_dir);
    if let Some(clone_git_dir) = commondir_target(&git_dir) {
        let clone_git_dir = normalize(&clone_git_dir);
        let clone = clone_git_dir
            .parent()
            .map_or_else(|| clone_git_dir.clone(), Path::to_path_buf);
        return Kind::Worktree {
            clone_exists: clone.is_dir(),
            clone,
        };
    }
    match submodule_outer_of(&git_dir) {
        Some(outer) => Kind::Submodule { outer },
        None => Kind::Clone,
    }
}

/// The clone's git directory a worktree's `commondir` names, whether or not
/// it still exists - unlike [`common_dir_of`], which falls back to
/// `git_dir` itself so the remote can still be attempted at the worktree's
/// own directory. Kept separate so a clone that has been removed can still
/// be named, rather than looking like no `commondir` was ever there.
fn commondir_target(git_dir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(git_dir.join("commondir")).ok()?;
    let target = PathBuf::from(text.trim());
    if target.as_os_str().is_empty() {
        return None;
    }
    Some(if target.is_absolute() {
        target
    } else {
        git_dir.join(target)
    })
}

/// The outer working copy holding `git_dir`, from everything before the
/// `.git` component of `<outer>/.git/modules/<name>`. `None` when `git_dir`
/// has no `.git` component, which does not happen for a real submodule.
fn submodule_outer_of(git_dir: &Path) -> Option<PathBuf> {
    let components: Vec<_> = git_dir.components().collect();
    let index = components
        .iter()
        .position(|component| component.as_os_str() == std::ffi::OsStr::new(".git"))?;
    Some(components[..index].iter().collect())
}

/// `path` with `.` and `..` components resolved lexically, without touching
/// the filesystem - so a `commondir` written as `../..` still yields a real
/// directory to take the parent of, even one that no longer exists to
/// canonicalize.
fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                result.pop();
            }
            std::path::Component::CurDir => {}
            other => result.push(other),
        }
    }
    result
}

/// As [`describe`], and also whether the working tree has uncommitted
/// changes to the files it tracks.
///
/// For the *selected* repository only. A workspace of forty checkouts is
/// forty passes over forty sets of tracked files that nobody asked for; the
/// one a reader is looking at is a pass they did ask for.
#[must_use]
pub fn describe_with_status(path: &Path) -> Option<Repository> {
    let git_dir = git_dir_of(path)?;
    let mut found = describe(path)?;
    found.status = crate::status::working_tree(&git_dir, path);
    found.tracking = crate::tracking::tracking(path);
    Some(found)
}

/// The directory holding the checkout's own files, following a worktree or
/// submodule marker to wherever it points.
fn git_dir_of(path: &Path) -> Option<PathBuf> {
    let marker = path.join(".git");
    if marker.is_dir() {
        Some(marker)
    } else if marker.is_file() {
        worktree_git_dir(&marker)
    } else {
        None
    }
}

/// The directory holding what every checkout of a clone shares - the
/// `config`, and so the remote.
///
/// An ordinary clone's git directory is its own common directory. A linked
/// worktree's is not: `git worktree add` gives it `HEAD`, `index`, `logs`
/// and `refs` of its own and **no `config`**, leaving a `commondir` file
/// pointing at the clone's git directory beside it. Reading `config` from
/// the worktree's own directory therefore found nothing, so a worktree of a
/// GitHub clone reported "Remote: none configured" and lost its provider,
/// while the clone beside it named both.
///
/// A submodule is not the same case and must not be folded into it: its git
/// directory under `<outer>/.git/modules/<name>` carries a real `config` of
/// its own, which is already the right one to read.
fn common_dir_of(git_dir: &Path) -> PathBuf {
    let Ok(text) = std::fs::read_to_string(git_dir.join("commondir")) else {
        return git_dir.to_path_buf();
    };
    let target = PathBuf::from(text.trim());
    if target.as_os_str().is_empty() {
        return git_dir.to_path_buf();
    }
    // The path is relative to the git directory that named it, and git
    // writes it that way (`../..`) for a worktree inside its own clone.
    let resolved = if target.is_absolute() {
        target
    } else {
        git_dir.join(target)
    };
    if resolved.is_dir() {
        resolved
    } else {
        git_dir.to_path_buf()
    }
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

/// The web page for a repository, at `branch` when there is one.
///
/// Built from the remote the checkout already records, in either shape it
/// is written - `https://github.com/owner/name.git` or the secure shell
/// short form `git@github.com:owner/name.git` - and never by asking the
/// remote anything.
///
/// Three rules that are not negotiable, because the result is handed to a
/// browser:
///
/// - **Only `https` addresses are produced.** A remote written with any
///   other scheme - `file://`, a local path, something unrecognised - has
///   no web page this can vouch for, and gets `None`.
/// - **Credentials never travel.** `https://user:token@host/...` opens as
///   `https://host/...`.
/// - **The branch is percent-encoded**, since branch names carry `#`, `?`
///   and spaces that would otherwise end or reshape the address.
///
/// Each host wants its branch in its own place: GitHub `/tree/<branch>`,
/// GitLab `/-/tree/<branch>`, Bitbucket `/src/<branch>`, Azure DevOps
/// `?version=GB<branch>`. Any other host gets the repository's page.
#[must_use]
pub fn web_address(remote: &str, branch: Option<&str>) -> Option<String> {
    let (host, port, path) = web_parts(remote.trim())?;
    let path = path.trim_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');

    // Azure DevOps writes its secure shell remotes on a different host and
    // with a different path from its web pages.
    let (host, path) = if host == "ssh.dev.azure.com" {
        let rest = path.strip_prefix("v3/")?;
        let mut parts = rest.splitn(3, '/');
        let (organisation, project, repository) = (parts.next()?, parts.next()?, parts.next()?);
        (
            "dev.azure.com".to_owned(),
            format!("{organisation}/{project}/_git/{repository}"),
        )
    } else {
        (host, path.to_owned())
    };

    if host.is_empty() || path.is_empty() {
        return None;
    }
    let mut address = match port {
        Some(port) => format!("https://{host}:{port}/{path}"),
        None => format!("https://{host}/{path}"),
    };
    if let Some(branch) = branch.filter(|branch| !branch.is_empty()) {
        let branch = percent_encode_branch(branch);
        let place = match host.as_str() {
            "github.com" => "/tree/",
            "gitlab.com" => "/-/tree/",
            "bitbucket.org" => "/src/",
            "dev.azure.com" => "?version=GB",
            _ => "",
        };
        if !place.is_empty() {
            address.push_str(place);
            address.push_str(&branch);
        }
    }
    Some(address)
}

/// The host, a web port worth keeping, and the path, from a remote in
/// either of its two shapes. `None` for anything else.
fn web_parts(remote: &str) -> Option<(String, Option<u16>, &str)> {
    if let Some((scheme, rest)) = remote.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        if !matches!(scheme.as_str(), "https" | "http" | "ssh" | "git") {
            return None;
        }
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host_and_port = authority.rsplit('@').next()?;
        let (host, port) = match host_and_port.split_once(':') {
            Some((host, port)) => (host, port.parse::<u16>().ok()),
            None => (host_and_port, None),
        };
        // A secure shell or git-protocol port says nothing about where the
        // web pages are served, so only a web remote keeps its port.
        let port = port.filter(|_| matches!(scheme.as_str(), "https" | "http"));
        return Some((host.to_ascii_lowercase(), port, path));
    }

    // `user@host:path`, the secure shell short form. A colon after a slash,
    // or no `@` at all, is a local path and not this.
    let (credentials, rest) = remote.split_once('@')?;
    if credentials.contains('/') {
        return None;
    }
    let (host, path) = rest.split_once(':')?;
    if host.contains('/') {
        return None;
    }
    Some((host.to_ascii_lowercase(), None, path))
}

/// `branch` with everything but unreserved characters and `/`
/// percent-encoded, byte by byte, so a multi-byte character encodes as its
/// bytes.
fn percent_encode_branch(branch: &str) -> String {
    let mut encoded = String::with_capacity(branch.len());
    for byte in branch.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            encoded.push(char::from(byte));
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::{Kind, describe, describe_with_status, provider_of};
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
        assert_eq!(
            found.status, None,
            "a plain description costs a few reads; status is asked for separately"
        );

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
    ///
    /// The git directory here is given a `config`, which a real linked
    /// worktree's never has - this test is about the marker being
    /// followed, and nothing else. What git actually writes is covered by
    /// `a_worktrees_remote_comes_from_the_clone_it_shares`, and the
    /// difference is why a broken worktree remote survived this test.
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

    /// What `git worktree add` actually writes: the worktree's own git
    /// directory holds `HEAD`, `index`, `logs` and `refs` and **no
    /// `config`**, with a `commondir` file pointing at the clone's git
    /// directory beside it. The remote is shared, so it has to be read
    /// from there.
    ///
    /// Reading `config` from the worktree's own directory found nothing,
    /// so a worktree of a GitHub clone reported "Remote: none configured"
    /// and lost its provider while the clone beside it named both.
    #[test]
    fn a_worktrees_remote_comes_from_the_clone_it_shares() {
        let dir = temp_dir("worktree-commondir");

        // The clone's git directory, with the config every checkout shares.
        let clone_git = dir.join("clone").join(".git");
        std::fs::create_dir_all(&clone_git).unwrap();
        std::fs::write(clone_git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            clone_git.join("config"),
            "[remote \"origin\"]\n\turl = https://github.com/owner/name.git\n",
        )
        .unwrap();

        // The worktree's own git directory: no config, and a commondir
        // written relative to itself, the way git writes it.
        let worktree_git = clone_git.join("worktrees").join("side");
        std::fs::create_dir_all(&worktree_git).unwrap();
        std::fs::write(worktree_git.join("HEAD"), "ref: refs/heads/side\n").unwrap();
        std::fs::write(worktree_git.join("commondir"), "../..\n").unwrap();
        assert!(
            !worktree_git.join("config").exists(),
            "the fixture is only honest if it has no config of its own"
        );

        let checkout = dir.join("side");
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(
            checkout.join(".git"),
            format!("gitdir: {}\n", worktree_git.display()),
        )
        .unwrap();

        let found = describe(&checkout).expect("a worktree is a working copy");

        assert_eq!(
            found.branch.as_deref(),
            Some("side"),
            "the branch is the worktree's own, and always was"
        );
        assert_eq!(
            found.remote.as_deref(),
            Some("https://github.com/owner/name.git"),
            "the remote is the clone's, shared by every checkout of it"
        );
        assert_eq!(found.provider.as_deref(), Some("github.com"));
        assert_eq!(
            found.kind,
            Kind::Worktree {
                clone: dir.join("clone"),
                clone_exists: true,
            },
            "the clone beside it is right there"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A submodule is not the same case and must not be folded into it.
    /// Its git directory under `<outer>/.git/modules/<name>` carries a real
    /// `config` of its own, which is already the right one to read - so a
    /// fix for worktrees must leave it alone.
    #[test]
    fn a_submodule_still_reads_the_config_in_its_own_git_directory() {
        let dir = temp_dir("submodule-config");

        let module_git = dir.join("outer").join(".git").join("modules").join("inner");
        std::fs::create_dir_all(&module_git).unwrap();
        std::fs::write(module_git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            module_git.join("config"),
            "[remote \"origin\"]\n\turl = git@gitlab.com:group/inner.git\n",
        )
        .unwrap();

        let checkout = dir.join("outer").join("inner");
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(
            checkout.join(".git"),
            format!("gitdir: {}\n", module_git.display()),
        )
        .unwrap();

        let found = describe(&checkout).expect("a submodule is a working copy");

        assert_eq!(
            found.remote.as_deref(),
            Some("git@gitlab.com:group/inner.git"),
            "a submodule's own config is the one that names its remote"
        );
        assert_eq!(found.provider.as_deref(), Some("gitlab.com"));
        assert_eq!(
            found.kind,
            Kind::Submodule {
                outer: dir.join("outer"),
            }
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ---- kind (#587) -----------------------------------------------------

    /// Real `git submodule add` writes `gitdir:` with `..` segments rather
    /// than resolving them - `../../.git/modules/vendor/forge` from a
    /// submodule two levels below its outer copy - and the OS follows that
    /// fine for the file reads `describe` already does. Finding the outer
    /// copy needs the `.git` path component itself, and a search over the
    /// unresolved path found the literal `..` sitting in front of it,
    /// naming an outer copy of `clone/vendor/forge/../..` instead of
    /// `clone`.
    #[test]
    fn a_submodules_outer_copy_is_found_through_an_unresolved_gitdir() {
        let dir = temp_dir("submodule-dotdot");

        let module_git = dir
            .join("clone")
            .join(".git")
            .join("modules")
            .join("vendor")
            .join("forge");
        std::fs::create_dir_all(&module_git).unwrap();
        std::fs::write(module_git.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let checkout = dir.join("clone").join("vendor").join("forge");
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(
            checkout.join(".git"),
            "gitdir: ../../.git/modules/vendor/forge\n",
        )
        .unwrap();

        let found = describe(&checkout).expect("a submodule is a working copy");

        assert_eq!(
            found.kind,
            Kind::Submodule {
                outer: dir.join("clone"),
            }
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_ordinary_clone_is_kind_clone() {
        let dir = temp_dir("kind-clone");
        write_checkout(
            &dir,
            "ref: refs/heads/main\n",
            "[remote \"origin\"]\n\turl = https://github.com/owner/name.git\n",
        );

        let found = describe(&dir).expect("a directory .git marker is a clone");

        assert_eq!(found.kind, Kind::Clone);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A worktree whose clone has been removed still names where it was, so
    /// the File pane can say so rather than silently losing the fact.
    #[test]
    fn a_worktree_whose_clone_no_longer_exists_still_names_where_it_was() {
        let dir = temp_dir("worktree-orphan");

        // The worktree's own git directory - it still exists, and still
        // names the clone's, but the clone itself never does.
        let worktree_git = dir.join("worktree-git");
        std::fs::create_dir_all(&worktree_git).unwrap();
        std::fs::write(worktree_git.join("HEAD"), "ref: refs/heads/side\n").unwrap();
        let clone_git = dir.join("clone").join(".git");
        std::fs::write(
            worktree_git.join("commondir"),
            format!("{}\n", clone_git.display()),
        )
        .unwrap();
        assert!(
            !clone_git.exists(),
            "the fixture is only honest if the clone is really gone"
        );

        let checkout = dir.join("standalone");
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(
            checkout.join(".git"),
            format!("gitdir: {}\n", worktree_git.display()),
        )
        .unwrap();

        let found = describe(&checkout).expect("a worktree is a working copy even orphaned");

        assert_eq!(
            found.kind,
            Kind::Worktree {
                clone: dir.join("clone"),
                clone_exists: false,
            },
            "the clone's path survives even though it is gone"
        );

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

    #[test]
    fn asking_for_status_reads_the_working_tree_as_well() {
        let dir = temp_dir("with-status");
        write_checkout(
            &dir,
            "ref: refs/heads/main
",
            "[remote \"origin\"]
	url = https://github.com/owner/name.git
",
        );
        // An index listing one file that is not there: a deleted tracked
        // file, which is the cheapest change to stage for a test.
        let git = dir.join(".git");
        let mut index = b"DIRC".to_vec();
        index.extend_from_slice(&2u32.to_be_bytes());
        index.extend_from_slice(&1u32.to_be_bytes());
        let start = index.len();
        index.extend_from_slice(&[0u8; 40]);
        index.extend_from_slice(&[0u8; 20]);
        index.extend_from_slice(&5u16.to_be_bytes());
        index.extend_from_slice(b"a.txt");
        let written = index.len() - start;
        index.resize(start + written.div_ceil(8) * 8, 0);
        std::fs::write(git.join("index"), index).unwrap();

        let found = describe_with_status(&dir).expect("still a working copy");

        let status = found.status.expect("an index it could read");
        assert_eq!(status.changed, 1, "a tracked file that is not there");
        assert_eq!(found.provider.as_deref(), Some("github.com"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ---- last_activity (#588) -------------------------------------------

    /// Sets a file's modification time, so a test can control the order two
    /// files' mtimes come out in without depending on real elapsed time.
    fn set_mtime(path: &Path, seconds_from_now: i64) {
        let offset = std::time::Duration::from_secs(seconds_from_now.unsigned_abs());
        let at = if seconds_from_now >= 0 {
            std::time::SystemTime::now() + offset
        } else {
            std::time::SystemTime::now() - offset
        };
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }

    #[test]
    fn last_activity_is_the_latest_of_index_head_and_logs_head() {
        let dir = temp_dir("last-activity-latest");
        write_checkout(&dir, "ref: refs/heads/main\n", "[core]\n");
        let git = dir.join(".git");
        std::fs::write(git.join("index"), b"").unwrap();
        std::fs::create_dir_all(git.join("logs")).unwrap();
        std::fs::write(git.join("logs").join("HEAD"), b"").unwrap();

        set_mtime(&git.join("HEAD"), -300);
        set_mtime(&git.join("index"), -100);
        // The most recent of the three: a branch switch after the last
        // commit and after the last staging.
        set_mtime(&git.join("logs").join("HEAD"), -10);

        let found = describe(&dir).expect("a working copy");

        let expected = std::fs::metadata(git.join("logs").join("HEAD"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(
            found.last_activity,
            Some(expected),
            "logs/HEAD is the newest of the three"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_activity_files_are_skipped() {
        let dir = temp_dir("last-activity-missing");
        write_checkout(&dir, "ref: refs/heads/main\n", "[core]\n");
        let git = dir.join(".git");
        // No index and no logs/HEAD - only HEAD, which write_checkout wrote.

        let found = describe(&dir).expect("a working copy");

        let expected = std::fs::metadata(git.join("HEAD"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(
            found.last_activity,
            Some(expected),
            "HEAD is the only one of the three that exists"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_worktree_reads_its_own_git_directory_for_last_activity() {
        let dir = temp_dir("last-activity-worktree");

        let clone_git = dir.join("clone").join(".git");
        std::fs::create_dir_all(&clone_git).unwrap();
        std::fs::write(clone_git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(clone_git.join("config"), "[core]\n").unwrap();
        std::fs::write(clone_git.join("index"), b"").unwrap();
        // The clone's own activity is old - the worktree's is what matters.
        set_mtime(&clone_git.join("index"), -100_000);

        let worktree_git = clone_git.join("worktrees").join("side");
        std::fs::create_dir_all(&worktree_git).unwrap();
        std::fs::write(worktree_git.join("HEAD"), "ref: refs/heads/side\n").unwrap();
        std::fs::write(worktree_git.join("commondir"), "../..\n").unwrap();
        std::fs::write(worktree_git.join("index"), b"").unwrap();
        set_mtime(&worktree_git.join("HEAD"), -50);
        // The worktree's own newest file - older than the clone's own
        // (-100_000) is not the point; newer than the worktree's own HEAD
        // is, so the max among the worktree's own files is unambiguous.
        set_mtime(&worktree_git.join("index"), -5);

        let checkout = dir.join("side");
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(
            checkout.join(".git"),
            format!("gitdir: {}\n", worktree_git.display()),
        )
        .unwrap();

        let found = describe(&checkout).expect("a worktree is a working copy");

        let expected = std::fs::metadata(worktree_git.join("index"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(
            found.last_activity,
            Some(expected),
            "the worktree's own index, not the clone's"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_repository_with_none_of_the_three_falls_back_to_the_folders_time() {
        let dir = temp_dir("last-activity-fallback");
        // A `.git` directory with nothing in it that names an activity -
        // not even HEAD, which every real checkout has, so the fallback is
        // exercised honestly rather than by a HEAD that happens to exist.
        std::fs::create_dir_all(dir.join(".git")).unwrap();

        let found = describe(&dir).expect("a directory with a .git marker is a working copy");

        let expected = std::fs::metadata(&dir).unwrap().modified().unwrap();
        assert_eq!(
            found.last_activity,
            Some(expected),
            "falls back to the folder's own modification time"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ---- web_address (#534) --------------------------------------------

    #[test]
    fn a_github_remote_opens_at_its_branch_in_either_shape() {
        for remote in [
            "https://github.com/owner/name.git",
            "https://github.com/owner/name",
            "git@github.com:owner/name.git",
            "ssh://git@github.com/owner/name.git",
        ] {
            assert_eq!(
                super::web_address(remote, Some("main")).as_deref(),
                Some("https://github.com/owner/name/tree/main"),
                "{remote}"
            );
        }
    }

    #[test]
    fn each_host_is_given_its_branch_in_its_own_place() {
        let cases = [
            (
                "git@gitlab.com:group/sub/name.git",
                "https://gitlab.com/group/sub/name/-/tree/dev",
            ),
            (
                "https://bitbucket.org/team/name.git",
                "https://bitbucket.org/team/name/src/dev",
            ),
            (
                "https://org@dev.azure.com/org/project/_git/name",
                "https://dev.azure.com/org/project/_git/name?version=GBdev",
            ),
            (
                "git@ssh.dev.azure.com:v3/org/project/name",
                "https://dev.azure.com/org/project/_git/name?version=GBdev",
            ),
        ];
        for (remote, expected) in cases {
            assert_eq!(
                super::web_address(remote, Some("dev")).as_deref(),
                Some(expected),
                "{remote}"
            );
        }
    }

    /// A self-hosted server's page layout is unknown, so it opens at the
    /// repository rather than at a guessed branch path.
    #[test]
    fn an_unknown_host_opens_at_the_repository_page() {
        assert_eq!(
            super::web_address("git@git.example.com:team/name.git", Some("main")).as_deref(),
            Some("https://git.example.com/team/name")
        );
    }

    #[test]
    fn credentials_in_a_remote_never_reach_the_address() {
        let address = super::web_address(
            "https://someone:s3cret-token@github.com/owner/name.git",
            None,
        )
        .expect("an https remote has a page");

        assert_eq!(address, "https://github.com/owner/name");
        assert!(!address.contains("s3cret"));
        assert!(!address.contains("someone"));
    }

    /// Only ever `https`, and nothing at all for a remote that is not on a
    /// web host.
    #[test]
    fn a_remote_that_is_not_on_a_web_host_offers_nothing() {
        for remote in [
            "file:///srv/git/name.git",
            "/srv/git/name.git",
            "C:\\repos\\name",
            "../sibling",
            "javascript:alert(1)",
            "",
            "https://",
            "git@github.com:",
        ] {
            assert_eq!(super::web_address(remote, Some("main")), None, "{remote:?}");
        }
    }

    #[test]
    fn a_plain_http_remote_still_opens_over_https() {
        let address = super::web_address("http://git.example.com:8080/team/name", None);

        assert_eq!(
            address.as_deref(),
            Some("https://git.example.com:8080/team/name")
        );
    }

    /// A secure shell port says nothing about where the web pages are.
    #[test]
    fn a_secure_shell_port_is_not_carried_into_the_address() {
        assert_eq!(
            super::web_address("ssh://git@github.com:22/owner/name.git", None).as_deref(),
            Some("https://github.com/owner/name")
        );
    }

    #[test]
    fn a_detached_head_opens_the_repository_page() {
        assert_eq!(
            super::web_address("git@github.com:owner/name.git", None).as_deref(),
            Some("https://github.com/owner/name")
        );
    }

    /// `/` is how branches are grouped and stays readable; everything that
    /// would end or reshape the address is encoded.
    #[test]
    fn a_branch_is_encoded_so_it_cannot_reshape_the_address() {
        assert_eq!(
            super::web_address("git@github.com:owner/name.git", Some("feature/login")).as_deref(),
            Some("https://github.com/owner/name/tree/feature/login")
        );
        assert_eq!(
            super::web_address("git@github.com:owner/name.git", Some("fix#12 ?now&x")).as_deref(),
            Some("https://github.com/owner/name/tree/fix%2312%20%3Fnow%26x")
        );
        assert_eq!(
            super::web_address("git@github.com:owner/name.git", Some("caf\u{e9}")).as_deref(),
            Some("https://github.com/owner/name/tree/caf%C3%A9")
        );
    }
}
