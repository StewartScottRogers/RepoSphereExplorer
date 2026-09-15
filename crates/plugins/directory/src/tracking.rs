//! How a checkout's branch stands against the remote-tracking branch it
//! follows: how many commits are not pushed, how many are not pulled.
//!
//! This compares the local branch with the remote-tracking reference
//! **already on disk**, as the last fetch left it. It never contacts the
//! remote and never runs a source control command (D10, rule 8), which is
//! why [`Tracking::last_fetch`] travels with every answer: "0 behind" is
//! only as true as the fetch it was measured against.
//!
//! Like the working-tree status, this is for the selected repository only
//! (GUIDANCE.md 3.5), and the walk is capped so a click never walks a huge
//! history.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How many commits a side is counted to before the walk stops and the
/// count reads as "at least" rather than exact.
const CAP: u32 = 10_000;

/// Where a checked-out branch stands against its upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tracking {
    /// The branch checked out, without `refs/heads/`.
    pub branch: String,
    /// What the branch is compared with, and how that came out.
    pub upstream: Upstream,
    /// When the clone last fetched - the modification time of `FETCH_HEAD`
    /// - or `None` when it never has.
    pub last_fetch: Option<SystemTime>,
}

/// The upstream of a branch, and how the branch compares with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Upstream {
    /// The branch has no `branch.<name>.remote` and `branch.<name>.merge`.
    None,
    /// An upstream is configured, but its remote-tracking reference is not
    /// on disk - the remote was added and never fetched.
    NotFetched {
        /// The upstream's short name, such as `origin/main`.
        name: String,
    },
    /// Both commits are known but the history between them could not be
    /// read - a missing or damaged object. Nothing is guessed.
    Unreadable {
        /// The upstream's short name, such as `origin/main`.
        name: String,
    },
    /// The history was walked. Up to date is both counts `Exact(0)`.
    Compared {
        /// The upstream's short name, such as `origin/main`.
        name: String,
        /// Commits on the branch that the upstream does not have.
        ahead: Count,
        /// Commits on the upstream that the branch does not have.
        behind: Count,
    },
}

/// A number of commits, and whether the walk finished counting them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Count {
    /// Exactly this many.
    Exact(u32),
    /// At least this many: a side passed the cap and the walk stopped
    /// there, before either side was finished. The side that passed the cap
    /// holds the cap itself.
    AtLeast(u32),
}

/// How the branch checked out at `path` stands against its upstream.
///
/// `None` when there is no branch to compare: `path` is not a working copy,
/// its head is detached, or its branch has no commits yet. Everything else,
/// including an upstream that cannot be compared, is a [`Tracking`].
#[must_use]
pub fn tracking(path: &Path) -> Option<Tracking> {
    tracking_capped(path, CAP)
}

/// [`tracking`], with the cap given - so a test can prove the cap on a
/// short history.
fn tracking_capped(path: &Path, cap: u32) -> Option<Tracking> {
    let git_dir = git_dir_of(path)?;
    let common_dir = common_dir_of(&git_dir);
    let refs = if common_dir == git_dir {
        gix_ref::file::Store::at(git_dir.clone(), gix_hash::Kind::Sha1)
    } else {
        gix_ref::file::Store::for_linked_worktree(
            git_dir.clone(),
            common_dir.clone(),
            gix_hash::Kind::Sha1,
        )
    };

    let head = refs.try_find_loose("HEAD").ok()??;
    let gix_ref::Target::Symbolic(head_target) = head.target else {
        return None;
    };
    let branch = head_target
        .as_bstr()
        .to_string()
        .strip_prefix("refs/heads/")?
        .to_owned();
    let local = match refs.try_find(head_target.as_ref()) {
        Ok(None) => return None,
        Ok(Some(reference)) => object_of(reference),
        Err(_) => None,
    };

    let last_fetch = last_fetch(&git_dir, &common_dir);
    let config = gix_config::File::from_path_no_includes(
        common_dir.join("config"),
        gix_config::Source::Local,
    )
    .ok()?;
    let Some((name, upstream_ref)) = upstream_of(&config, &branch) else {
        return Some(Tracking {
            branch,
            upstream: Upstream::None,
            last_fetch,
        });
    };

    let upstream = match refs.try_find(upstream_ref.as_str()) {
        Ok(None) => Upstream::NotFetched { name },
        Ok(Some(reference)) => match (local, object_of(reference)) {
            (Some(local), Some(remote)) => match counts(&common_dir, local, remote, cap) {
                Some((ahead, behind)) => Upstream::Compared {
                    name,
                    ahead,
                    behind,
                },
                None => Upstream::Unreadable { name },
            },
            _ => Upstream::Unreadable { name },
        },
        Err(_) => Upstream::Unreadable { name },
    };
    Some(Tracking {
        branch,
        upstream,
        last_fetch,
    })
}

/// The commit a branch reference names, or `None` when it names another
/// reference instead - which a branch or remote-tracking branch does not.
fn object_of(reference: gix_ref::Reference) -> Option<gix_hash::ObjectId> {
    reference.target.try_into_id().ok()
}

/// The upstream's short name and full reference, from
/// `branch.<name>.remote` and `branch.<name>.merge`.
///
/// A remote of `.` is the clone itself: the branch tracks another local
/// branch, which `git branch --track` sets up, and is compared with that.
fn upstream_of(config: &gix_config::File, branch: &str) -> Option<(String, String)> {
    let remote = config.string_by("branch", Some(branch.into()), "remote")?;
    let merge = config.string_by("branch", Some(branch.into()), "merge")?;
    let merge = merge.to_string();
    let merged_branch = merge.strip_prefix("refs/heads/").unwrap_or(&merge);
    if remote == "." {
        return Some((
            merged_branch.to_owned(),
            format!("refs/heads/{merged_branch}"),
        ));
    }
    Some((
        format!("{remote}/{merged_branch}"),
        format!("refs/remotes/{remote}/{merged_branch}"),
    ))
}

/// When the clone last fetched.
///
/// `git fetch` writes `FETCH_HEAD` into the git directory of the checkout
/// it ran in, but updates the remote-tracking references every checkout of
/// the clone shares. So a linked worktree's comparison is as fresh as the
/// latest fetch anywhere in the clone: the newer of its own and the
/// clone's.
fn last_fetch(git_dir: &Path, common_dir: &Path) -> Option<SystemTime> {
    [git_dir, common_dir]
        .into_iter()
        .filter_map(|dir| {
            std::fs::metadata(dir.join("FETCH_HEAD"))
                .ok()?
                .modified()
                .ok()
        })
        .max()
}

/// Commit bits painted while walking: reached from the branch, reached
/// from the upstream. The same two bits shifted up record what a commit
/// was last counted as and passed on to its parents.
const LOCAL: u8 = 1;
const REMOTE: u8 = 2;
const BOTH: u8 = LOCAL | REMOTE;
const PASSED_SHIFT: u8 = 2;

/// Commits reachable from `local` but not `remote` (ahead), and from
/// `remote` but not `local` (behind), each counted to at most `cap`.
///
/// This is the merge-base paint `git` itself uses: walk both tips newest
/// first, marking each commit with the side or sides that reach it, and
/// stop once every commit still waiting is reached by both - the shared
/// history below the merge base is never walked. A commit is counted when
/// it is taken off the queue, and recounted - with the ancestors counted
/// alongside it - if the other side reaches it later, which commits made
/// in the same second, as a rebase makes them, can do.
///
/// When a side passes `cap` the walk stops there and both counts read as
/// "at least": what was counted is on one side only, but the unfinished
/// side may have more.
///
/// Newest first means by committer time, as `git` does without a
/// commit-graph file; like `git`, a history whose clocks ran backwards can
/// be miscounted by the commits the skew reorders.
///
/// `None` when a commit on the way cannot be read - a missing or damaged
/// object, or a shallow clone's boundary - since a partial walk would be a
/// guessed number.
fn counts(
    common_dir: &Path,
    local: gix_hash::ObjectId,
    remote: gix_hash::ObjectId,
    cap: u32,
) -> Option<(Count, Count)> {
    if local == remote {
        return Some((Count::Exact(0), Count::Exact(0)));
    }
    let objects = gix_odb::at(common_dir.join("objects"), gix_hash::Kind::Sha1).ok()?;
    let mut graph = gix_revwalk::Graph::<gix_revwalk::graph::Commit<u8>>::new(objects, None);
    let mut queue = gix_revwalk::PriorityQueue::new();
    for (tip, side) in [(local, LOCAL), (remote, REMOTE)] {
        let commit = graph
            .get_or_insert_full_commit(tip, |commit| commit.data |= side)
            .ok()??;
        queue.insert(commit.commit_time, tip);
    }

    let (mut ahead, mut behind) = (0u32, 0u32);
    let mut stopped = false;
    // Work remains while a waiting commit is reached by one side only, or
    // was counted for one side and has since been reached by the other -
    // that correction must carry down to the ancestors counted with it.
    // A commit reached by both and never counted is shared history.
    while queue.iter_unordered().any(|id| {
        graph.get(id).is_some_and(|commit| {
            let sides = commit.data & BOTH;
            let passed = commit.data >> PASSED_SHIFT;
            sides != BOTH || (passed != 0 && passed != sides)
        })
    }) {
        let Some(id) = queue.pop_value() else { break };
        let commit = graph.get_mut(&id)?;
        let sides = commit.data & BOTH;
        let passed = commit.data >> PASSED_SHIFT;
        if sides == passed {
            continue;
        }
        commit.data = sides | (sides << PASSED_SHIFT);
        match passed {
            LOCAL => ahead -= 1,
            REMOTE => behind -= 1,
            _ => {}
        }
        match sides {
            LOCAL => ahead += 1,
            REMOTE => behind += 1,
            _ => {}
        }
        if ahead > cap || behind > cap {
            stopped = true;
            break;
        }
        for parent in commit.parents.clone() {
            graph
                .get_or_insert_full_commit(parent, |parent_commit| {
                    if parent_commit.data & sides != sides {
                        parent_commit.data |= sides;
                        queue.insert(parent_commit.commit_time, parent);
                    }
                })
                .ok()??;
        }
    }

    let count = |counted: u32| {
        if stopped {
            Count::AtLeast(counted.min(cap))
        } else {
            Count::Exact(counted)
        }
    };
    Some((count(ahead), count(behind)))
}

/// The directory holding the checkout's own files, following a worktree or
/// submodule marker to wherever it points.
///
/// A copy of the private helper in `repository.rs`, kept here so that this
/// module and the working-copy description could be changed side by side
/// (#534 and #537) without editing the same file; fold the two together
/// once both have landed.
fn git_dir_of(path: &Path) -> Option<PathBuf> {
    let marker = path.join(".git");
    if marker.is_dir() {
        return Some(marker);
    }
    let text = std::fs::read_to_string(&marker).ok()?;
    let target = text.lines().find_map(|line| line.strip_prefix("gitdir:"))?;
    let resolved = path.join(target.trim());
    resolved.is_dir().then_some(resolved)
}

/// The directory every checkout of a clone shares: the `config`, the
/// objects, and the remote-tracking references. A linked worktree names it
/// in a `commondir` file (#513); anything else is its own. A copy of the
/// helper in `repository.rs` - see [`git_dir_of`].
fn common_dir_of(git_dir: &Path) -> PathBuf {
    let Ok(text) = std::fs::read_to_string(git_dir.join("commondir")) else {
        return git_dir.to_path_buf();
    };
    let target = text.trim();
    if target.is_empty() {
        return git_dir.to_path_buf();
    }
    let resolved = git_dir.join(target);
    if resolved.is_dir() {
        resolved
    } else {
        git_dir.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::{Count, Tracking, Upstream, tracking, tracking_capped};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// A fresh scratch directory under the system temporary directory.
    fn scratch(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rse-tracking-{}-{}-{name}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Runs `git` in `dir`, which must be inside the temporary directory:
    /// rule 8 says no source control command ever runs on somebody's
    /// working copy, and a test building fixtures is where a stray path
    /// would do it. The identity is given on the command line so the
    /// machine's own configuration does not matter. Each call is dated a
    /// second after the last, as commits made by hand would be. Returns
    /// what it printed.
    fn git(dir: &Path, arguments: &[&str]) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CLOCK: AtomicU64 = AtomicU64::new(1_700_000_000);
        git_at(dir, arguments, CLOCK.fetch_add(1, Ordering::Relaxed))
    }

    /// [`git`], with any commit it makes dated `seconds` after the epoch.
    fn git_at(dir: &Path, arguments: &[&str], seconds: u64) -> String {
        let date = format!("@{seconds} +0000");
        assert!(
            dir.starts_with(std::env::temp_dir()),
            "refusing to run git outside the temporary directory: {}",
            dir.display()
        );
        let output = Command::new("git")
            .args([
                "-c",
                "user.name=Repos Explorer Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(arguments)
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .current_dir(dir)
            .output()
            .expect("git should be on PATH");
        assert!(
            output.status.success(),
            "git {arguments:?} failed in {}: {}",
            dir.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    /// `count` empty commits on whatever `dir` has checked out.
    fn commit(dir: &Path, count: usize) {
        for _ in 0..count {
            git(dir, &["commit", "--quiet", "--allow-empty", "-m", "change"]);
        }
    }

    /// A bare remote with one commit on `main`, and a clone of it that has
    /// fetched - so the clone is up to date with `origin/main`.
    fn remote_and_clone(root: &Path) -> (PathBuf, PathBuf) {
        let seed = root.join("seed");
        std::fs::create_dir_all(&seed).unwrap();
        git(&seed, &["init", "--quiet", "."]);
        commit(&seed, 1);
        git(root, &["clone", "--quiet", "--bare", "seed", "remote.git"]);
        git(root, &["clone", "--quiet", "remote.git", "clone"]);
        let clone = root.join("clone");
        git(&clone, &["fetch", "--quiet"]);
        (seed, clone)
    }

    /// Commits `count` times on the remote's `main`, by pushing from the
    /// seed checkout, and fetches them into `clone`.
    fn advance_remote(seed: &Path, clone: &Path, count: usize) {
        commit(seed, count);
        git(seed, &["push", "--quiet", "../remote.git", "main"]);
        git(clone, &["fetch", "--quiet"]);
    }

    fn compared(tracking: Option<Tracking>) -> (Count, Count) {
        match tracking.expect("a branch to compare").upstream {
            Upstream::Compared {
                name,
                ahead,
                behind,
            } => {
                assert_eq!(name, "origin/main");
                (ahead, behind)
            }
            other => panic!("expected a comparison, got {other:?}"),
        }
    }

    #[test]
    fn a_freshly_fetched_clone_is_up_to_date_as_of_that_fetch() {
        let root = scratch("up-to-date");
        let (_, clone) = remote_and_clone(&root);
        let found = tracking(&clone).expect("a branch");
        assert_eq!(found.branch, "main");
        assert!(found.last_fetch.is_some(), "the fetch wrote FETCH_HEAD");
        assert_eq!(compared(Some(found)), (Count::Exact(0), Count::Exact(0)));
    }

    #[test]
    fn unpushed_commits_are_ahead() {
        let root = scratch("ahead");
        let (_, clone) = remote_and_clone(&root);
        commit(&clone, 2);
        assert_eq!(
            compared(tracking(&clone)),
            (Count::Exact(2), Count::Exact(0))
        );
    }

    #[test]
    fn fetched_but_unpulled_commits_are_behind() {
        let root = scratch("behind");
        let (seed, clone) = remote_and_clone(&root);
        advance_remote(&seed, &clone, 5);
        assert_eq!(
            compared(tracking(&clone)),
            (Count::Exact(0), Count::Exact(5))
        );
    }

    #[test]
    fn a_branch_can_be_ahead_and_behind_at_once() {
        let root = scratch("diverged");
        let (seed, clone) = remote_and_clone(&root);
        commit(&clone, 2);
        advance_remote(&seed, &clone, 5);
        assert_eq!(
            compared(tracking(&clone)),
            (Count::Exact(2), Count::Exact(5))
        );
    }

    #[test]
    fn a_merge_already_pulled_in_is_not_counted_as_behind() {
        let root = scratch("merged");
        let (seed, clone) = remote_and_clone(&root);
        commit(&clone, 1);
        advance_remote(&seed, &clone, 3);
        git(&clone, &["merge", "--quiet", "--no-edit", "origin/main"]);
        // The merge commit and the local commit are not on the remote;
        // the remote's three are now reachable from the branch.
        assert_eq!(
            compared(tracking(&clone)),
            (Count::Exact(2), Count::Exact(0))
        );
    }

    #[test]
    fn commits_made_in_the_same_second_are_still_counted_exactly() {
        let root = scratch("same-second");
        let seed = root.join("seed");
        std::fs::create_dir_all(&seed).unwrap();
        let moment = 1_600_000_000;
        git_at(&seed, &["init", "--quiet", "."], moment);
        for _ in 0..3 {
            git_at(
                &seed,
                &["commit", "--quiet", "--allow-empty", "-m", "base"],
                moment,
            );
        }
        git(&root, &["clone", "--quiet", "--bare", "seed", "remote.git"]);
        git(&root, &["clone", "--quiet", "remote.git", "clone"]);
        let clone = root.join("clone");
        for _ in 0..2 {
            git_at(
                &clone,
                &["commit", "--quiet", "--allow-empty", "-m", "mine"],
                moment,
            );
        }
        for _ in 0..5 {
            git_at(
                &seed,
                &["commit", "--quiet", "--allow-empty", "-m", "theirs"],
                moment,
            );
        }
        git(&seed, &["push", "--quiet", "../remote.git", "main"]);
        git(&clone, &["fetch", "--quiet"]);
        assert_eq!(
            compared(tracking(&clone)),
            (Count::Exact(2), Count::Exact(5))
        );
    }

    #[test]
    fn a_branch_tracking_a_local_branch_is_compared_with_it() {
        let root = scratch("local-upstream");
        let (_, clone) = remote_and_clone(&root);
        git(
            &clone,
            &["checkout", "--quiet", "--track", "-b", "topic", "main"],
        );
        commit(&clone, 3);
        let found = tracking(&clone).expect("a branch");
        assert_eq!(
            found.upstream,
            Upstream::Compared {
                name: "main".to_owned(),
                ahead: Count::Exact(3),
                behind: Count::Exact(0),
            }
        );
    }

    #[test]
    fn a_branch_with_no_upstream_says_so() {
        let root = scratch("no-upstream");
        let (_, clone) = remote_and_clone(&root);
        git(&clone, &["checkout", "--quiet", "-b", "topic"]);
        let found = tracking(&clone).expect("a branch");
        assert_eq!(found.branch, "topic");
        assert_eq!(found.upstream, Upstream::None);
    }

    #[test]
    fn an_upstream_never_fetched_is_not_counted() {
        let root = scratch("not-fetched");
        let checkout = root.join("checkout");
        std::fs::create_dir_all(&checkout).unwrap();
        git(&checkout, &["init", "--quiet", "."]);
        commit(&checkout, 1);
        git(&checkout, &["remote", "add", "origin", "../nowhere.git"]);
        git(&checkout, &["config", "branch.main.remote", "origin"]);
        git(
            &checkout,
            &["config", "branch.main.merge", "refs/heads/main"],
        );
        let found = tracking(&checkout).expect("a branch");
        assert_eq!(
            found.upstream,
            Upstream::NotFetched {
                name: "origin/main".to_owned()
            }
        );
        assert_eq!(found.last_fetch, None, "never fetched");
    }

    #[test]
    fn a_detached_head_has_nothing_to_compare() {
        let root = scratch("detached");
        let (_, clone) = remote_and_clone(&root);
        git(&clone, &["checkout", "--quiet", "--detach"]);
        assert_eq!(tracking(&clone), None);
    }

    #[test]
    fn a_checkout_with_no_commits_has_nothing_to_compare() {
        let root = scratch("unborn");
        git(&root, &["init", "--quiet", "."]);
        assert_eq!(tracking(&root), None);
    }

    #[test]
    fn a_plain_folder_has_nothing_to_compare() {
        let root = scratch("plain");
        assert_eq!(tracking(&root), None);
    }

    #[test]
    fn a_missing_object_is_unreadable_rather_than_a_guess() {
        let root = scratch("unreadable");
        let (seed, clone) = remote_and_clone(&root);
        advance_remote(&seed, &clone, 2);
        // A fetch this small leaves loose objects; remove the fetched tip.
        let tip = git(&clone, &["rev-parse", "origin/main"]);
        let tip = tip.trim();
        let object = clone.join(".git/objects").join(&tip[..2]).join(&tip[2..]);
        std::fs::remove_file(&object).expect("the fetched commit is a loose object");
        assert_eq!(
            tracking(&clone).expect("a branch").upstream,
            Upstream::Unreadable {
                name: "origin/main".to_owned()
            }
        );
    }

    #[test]
    fn a_linked_worktree_compares_through_the_clone_it_shares() {
        let root = scratch("worktree");
        let (seed, clone) = remote_and_clone(&root);
        advance_remote(&seed, &clone, 3);
        git(
            &clone,
            &["branch", "--quiet", "--track", "feature", "origin/main"],
        );
        git(
            &clone,
            &["worktree", "add", "--quiet", "../linked", "feature"],
        );
        let linked = root.join("linked");
        commit(&linked, 1);
        let found = tracking(&linked).expect("a branch");
        assert_eq!(found.branch, "feature");
        assert!(found.last_fetch.is_some(), "the clone's fetch counts");
        assert_eq!(compared(Some(found)), (Count::Exact(1), Count::Exact(0)));
    }

    #[test]
    fn packed_objects_and_references_are_read() {
        let root = scratch("packed");
        let (seed, clone) = remote_and_clone(&root);
        commit(&clone, 2);
        advance_remote(&seed, &clone, 4);
        git(&clone, &["gc", "--quiet", "--prune=now"]);
        assert!(
            !clone.join(".git/refs/remotes/origin/main").exists(),
            "gc packs the remote-tracking reference"
        );
        assert_eq!(
            compared(tracking(&clone)),
            (Count::Exact(2), Count::Exact(4))
        );
    }

    #[test]
    fn the_walk_stops_at_the_cap() {
        let root = scratch("cap");
        let (seed, clone) = remote_and_clone(&root);
        commit(&clone, 2);
        advance_remote(&seed, &clone, 8);
        let found = tracking_capped(&clone, 5).expect("a branch");
        let Upstream::Compared { ahead, behind, .. } = found.upstream else {
            panic!("expected a comparison, got {:?}", found.upstream);
        };
        assert_eq!(behind, Count::AtLeast(5));
        assert!(
            matches!(ahead, Count::AtLeast(n) if n <= 2),
            "the other side is unfinished, so not exact: {ahead:?}"
        );
        // Under the cap, the same history counts exactly.
        assert_eq!(
            compared(tracking_capped(&clone, 8)),
            (Count::Exact(2), Count::Exact(8))
        );
    }

    #[test]
    fn unrelated_histories_stop_at_the_cap_on_both_sides() {
        let root = scratch("unrelated");
        let (_, clone) = remote_and_clone(&root);
        git(&clone, &["checkout", "--quiet", "--orphan", "fresh"]);
        commit(&clone, 7);
        git(
            &clone,
            &["branch", "--quiet", "--set-upstream-to", "origin/main"],
        );
        let Upstream::Compared { ahead, behind, .. } =
            tracking_capped(&clone, 3).expect("a branch").upstream
        else {
            panic!("expected a comparison");
        };
        assert_eq!(ahead, Count::AtLeast(3));
        assert!(matches!(behind, Count::AtLeast(_)));
    }
}
