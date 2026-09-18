//! The All Repositories view (#591): working copies found up to three
//! folder levels below the Repos Directory, scanned in a background thread
//! and cached for the session.
//!
//! GUIDANCE.md §2.5 settles the shape of this: additive to the ordinary
//! listing rather than a replacement for it, a constant depth rather than a
//! setting, and never a look inside a working copy it has already found -
//! a submodule is that working copy's own business.

use crate::{relative_to, repos};
use protocol::AllRepositoryEntry;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;

/// Folder levels below the root the scan looks at. A constant, not a
/// setting.
const MAX_DEPTH: usize = 3;

/// Folder names the scan never descends into, alongside hidden folders and
/// a working copy's own contents.
const EXCLUDED: [&str; 4] = ["node_modules", "target", "bin", "obj"];

/// One background scan's state: what it was asked to look below, what it
/// has found so far, and whether it has finished.
struct Scan {
    root: PathBuf,
    entries: Vec<AllRepositoryEntry>,
    done: bool,
}

/// The most recent scan, whatever root it was asked about. One scan at a
/// time: a second `root` (or a refresh) replaces it rather than running
/// beside it.
static SCAN: OnceLock<Mutex<Option<Scan>>> = OnceLock::new();

/// What the background scan below `root` has found so far, starting one if
/// none is under way or cached for `root`, or `refresh` asks for a fresh
/// one.
#[must_use]
pub(crate) fn poll(root: &Path, refresh: bool) -> (Vec<AllRepositoryEntry>, bool) {
    let cell = SCAN.get_or_init(|| Mutex::new(None));
    let mut guard = cell
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let start_new = refresh || guard.as_ref().is_none_or(|scan| scan.root != root);
    if start_new {
        *guard = Some(Scan {
            root: root.to_path_buf(),
            entries: Vec::new(),
            done: false,
        });
        spawn_scan(root.to_path_buf());
    }
    guard.as_ref().map_or((Vec::new(), false), |scan| {
        (scan.entries.clone(), scan.done)
    })
}

/// Runs one scan of `root` on its own thread, publishing what it finds as it
/// goes so a poll never waits for the whole tree.
fn spawn_scan(root: PathBuf) {
    thread::spawn(move || {
        walk(&root, &root, 0, &mut |entry| publish(&root, entry));
        mark_done(&root);
    });
}

/// Records one found entry against the scan for `root`, if that is still
/// the scan being asked about - a stale scan, superseded by a refresh or a
/// different root, has nowhere left to publish to and simply finishes
/// unread.
fn publish(root: &Path, entry: AllRepositoryEntry) {
    let cell = SCAN.get_or_init(|| Mutex::new(None));
    let mut guard = cell
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(scan) = guard.as_mut()
        && scan.root == root
    {
        scan.entries.push(entry);
    }
}

/// Marks the scan for `root` finished, if it is still the current one.
fn mark_done(root: &Path) {
    let cell = SCAN.get_or_init(|| Mutex::new(None));
    let mut guard = cell
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(scan) = guard.as_mut()
        && scan.root == root
    {
        scan.done = true;
    }
}

/// Walks `dir`, `depth` folder levels below `root`, calling `out` with every
/// working copy found at up to [`MAX_DEPTH`] levels below `root`.
///
/// Does not follow a symbolic link or a junction - `DirEntry::file_type`
/// reports what the entry itself is, never what it points to, so a link
/// is skipped as "not a directory" and a cycle through one cannot occur.
/// Does not enter a working copy once found, `node_modules`, `target`,
/// `bin`, `obj`, or a hidden folder. An unreadable folder is skipped, not
/// the end of the walk.
fn walk(root: &Path, dir: &Path, depth: usize, out: &mut impl FnMut(AllRepositoryEntry)) {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<_> = read_dir.flatten().collect();
    children.sort_by_key(std::fs::DirEntry::file_name);
    for entry in children {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || EXCLUDED.contains(&name.as_str()) {
            continue;
        }
        let path = entry.path();
        if let Some(repository) = repos::describe(&path) {
            out(AllRepositoryEntry {
                name,
                location: relative_to(root, dir),
                repository,
            });
            continue;
        }
        if depth + 1 < MAX_DEPTH {
            walk(root, &path, depth + 1, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AllRepositoryEntry, poll};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, MutexGuard};

    /// [`super::SCAN`] is one slot for the whole process, holding whichever
    /// root was most recently asked about - correct for the one Repos
    /// Directory a real session has open (D9), but two of these tests
    /// running at once would each keep invalidating the other's scan. Every
    /// test takes this first, so they run one at a time within this file;
    /// other test binaries, and other files' tests in this one, still run
    /// alongside them.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serially() -> MutexGuard<'static, ()> {
        SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A fresh, uniquely named scratch directory.
    fn scratch() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rse-all-repositories-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A fake checkout at `root/name`, `name` possibly a nested path: a
    /// `.git` directory with the two files a clone has, which is all the
    /// working copy marker needs. No `git` runs.
    fn checkout(root: &Path, name: &str) -> PathBuf {
        let dir = root.join(name);
        let git = dir.join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(
            git.join("config"),
            "[remote \"origin\"]\n\turl = https://github.com/owner/repo.git\n",
        )
        .unwrap();
        dir
    }

    /// Polls `root` (never a refresh) until the scan finishes, or panics
    /// after a generous number of tries - the scan is a handful of small
    /// directories, not a real workload, so a hang here is a defect.
    fn scan_to_completion(root: &Path) -> Vec<AllRepositoryEntry> {
        for _ in 0..1000 {
            let (entries, done) = poll(root, false);
            if done {
                return entries;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the scan below {} never finished", root.display());
    }

    #[test]
    fn finds_repositories_at_depths_one_two_and_three_but_not_four() {
        let _serial = serially();
        let root = scratch();
        checkout(&root, "level1");
        checkout(&root, "a/level2");
        checkout(&root, "a/b/level3");
        checkout(&root, "a/b/c/level4");

        let names: Vec<String> = scan_to_completion(&root)
            .into_iter()
            .map(|entry| entry.name)
            .collect();

        assert!(names.contains(&"level1".to_owned()));
        assert!(names.contains(&"level2".to_owned()));
        assert!(names.contains(&"level3".to_owned()));
        assert!(
            !names.contains(&"level4".to_owned()),
            "level4 sits four folders below the root: {names:?}"
        );
    }

    #[test]
    fn a_locations_column_holds_the_parent_path_relative_to_the_root() {
        let _serial = serially();
        let root = scratch();
        checkout(&root, "github/owner/project");

        let entries = scan_to_completion(&root);

        let found = entries
            .iter()
            .find(|entry| entry.name == "project")
            .expect("the nested checkout should be found");
        assert_eq!(found.location, "github/owner");
    }

    #[test]
    fn nothing_inside_a_working_copy_is_returned() {
        let _serial = serially();
        let root = scratch();
        let inner = checkout(&root, "outer");
        checkout(&inner, "nested");

        let names: Vec<String> = scan_to_completion(&root)
            .into_iter()
            .map(|entry| entry.name)
            .collect();

        assert_eq!(names, vec!["outer".to_owned()]);
    }

    #[test]
    fn an_excluded_or_hidden_folder_is_not_descended_into() {
        let _serial = serially();
        let root = scratch();
        checkout(&root, "node_modules/some-package");
        checkout(&root, "target/debug-checkout");
        checkout(&root, ".hidden/checkout");

        let entries = scan_to_completion(&root);

        assert!(
            entries.is_empty(),
            "nothing here should be found: {entries:?}"
        );
    }

    #[test]
    fn a_symbolic_link_loop_does_not_hang() {
        let _serial = serially();
        let root = scratch();
        fs::create_dir_all(root.join("real")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&root, root.join("real").join("loop")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&root, root.join("real").join("loop")).unwrap();

        let entries = scan_to_completion(&root);

        assert!(entries.is_empty());
    }

    #[test]
    fn when_every_direct_child_is_a_working_copy_the_set_matches_the_ordinary_listing() {
        let _serial = serially();
        let root = scratch();
        checkout(&root, "one");
        checkout(&root, "two");

        let names: Vec<String> = scan_to_completion(&root)
            .into_iter()
            .map(|entry| entry.name)
            .collect();

        assert_eq!(names, vec!["one".to_owned(), "two".to_owned()]);
    }

    #[test]
    fn the_scan_is_cached_until_refresh_asks_for_another() {
        let _serial = serially();
        let root = scratch();
        checkout(&root, "one");
        assert_eq!(
            scan_to_completion(&root)
                .into_iter()
                .map(|entry| entry.name)
                .collect::<Vec<_>>(),
            vec!["one".to_owned()]
        );

        // A working copy that appears after the scan finished is not found
        // by an ordinary poll: the cached result from the first scan is
        // what a session keeps showing.
        checkout(&root, "two");
        let (cached, done) = poll(&root, false);
        assert!(done, "the earlier scan's own completion still holds");
        assert_eq!(
            cached
                .into_iter()
                .map(|entry| entry.name)
                .collect::<Vec<_>>(),
            vec!["one".to_owned()],
            "an ordinary poll must not have started scanning again"
        );

        // F5 (Refresh) asks with refresh: true, and only then does the new
        // checkout show up.
        let _ = poll(&root, true);
        let names: Vec<String> = scan_to_completion(&root)
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, vec!["one".to_owned(), "two".to_owned()]);
    }
}
