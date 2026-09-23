//! Streaming directory listings (#717): the first screenful is answered as
//! soon as it is read, rather than only once the whole directory has been
//! walked, so a Repos Directory holding hundreds of entries on a slow or
//! network-mounted drive draws before the last one is read.
//!
//! GUIDANCE.md §3.3: "Directory listing streams; the first screen renders
//! before the walk finishes." Modelled on [`crate::all_repositories`]'s
//! background scan and poll, with one difference: a directory's contents
//! are current information, not a session-long survey, so a fresh
//! [`protocol::Request::ListDirectory`] for a path whose read already
//! finished starts reading again rather than repeating what it found last
//! time.

use crate::{directory_entry_from, sort_directory_entries};
use protocol::DirectoryEntry;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;

/// Entries read on the calling thread before a poll of a freshly started
/// read hands the rest to a background thread - enough for a full screen on
/// any front end, small enough that an ordinary folder still answers in one
/// round trip.
const FIRST_BATCH: usize = 200;

/// One directory read's state: what it has found so far, whether it has
/// finished, and - if it stopped early having found something - why.
struct Scan {
    path: PathBuf,
    entries: Vec<DirectoryEntry>,
    done: bool,
    error: Option<String>,
}

/// The most recent read, whatever path it was asked about. One at a time,
/// matching [`crate::all_repositories`]'s single slot: correct for the one
/// Contents pane a real session has open at once.
static SCAN: OnceLock<Mutex<Option<Scan>>> = OnceLock::new();

/// What the read of `path` has found so far - entries sorted by name,
/// whether the read has finished, and the reason if it stopped early having
/// found something.
///
/// Starts a fresh read if none is under way for `path`, or `refresh` asks
/// for one - the same contract as [`crate::all_repositories::poll`]. A
/// front end continuing to watch a read already under way polls with
/// `refresh: false`, so the answer that finally reports `done` is not
/// itself mistaken for a reason to start over; a fresh navigation, or a
/// reload after an operation changed what `path` holds, asks with
/// `refresh: true`, because a directory's contents are expected to be
/// current, not cached for a session the way the All Repositories view
/// deliberately is. Reads the first [`FIRST_BATCH`] entries on whichever
/// thread calls this before returning, so an ordinary folder still answers
/// in one round trip; only a longer read continues on a thread of its own.
///
/// # Errors
/// Returns an error if `path` itself cannot be opened as a directory. An
/// error partway through reading entries is instead recorded on the read
/// and returned alongside the entries already found, in the third element
/// of the answer.
pub(crate) fn poll(
    path: &Path,
    refresh: bool,
) -> io::Result<(Vec<DirectoryEntry>, bool, Option<String>)> {
    let cell = SCAN.get_or_init(|| Mutex::new(None));
    let mut guard = cell
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let start_new = refresh || guard.as_ref().is_none_or(|scan| scan.path != path);
    if start_new {
        let mut read_dir = fs::read_dir(path)?;
        let mut scan = Scan {
            path: path.to_path_buf(),
            entries: Vec::new(),
            done: false,
            error: None,
        };
        read_first_batch(&mut read_dir, &mut scan);
        let finished = scan.done;
        *guard = Some(scan);
        if !finished {
            spawn_scan(path.to_path_buf(), read_dir);
        }
    }
    Ok(guard.as_ref().map_or((Vec::new(), false, None), |scan| {
        let mut entries = scan.entries.clone();
        sort_directory_entries(&mut entries);
        (entries, scan.done, scan.error.clone())
    }))
}

/// Reads up to [`FIRST_BATCH`] entries from `read_dir` into `scan`, marking
/// it done if the directory has no more, or an entry cannot be read.
fn read_first_batch(read_dir: &mut fs::ReadDir, scan: &mut Scan) {
    for _ in 0..FIRST_BATCH {
        match read_dir.next() {
            Some(Ok(entry)) => match directory_entry_from(&entry) {
                Ok(entry) => scan.entries.push(entry),
                Err(err) => {
                    scan.error = Some(err.to_string());
                    scan.done = true;
                    return;
                }
            },
            Some(Err(err)) => {
                scan.error = Some(err.to_string());
                scan.done = true;
                return;
            }
            None => {
                scan.done = true;
                return;
            }
        }
    }
}

/// Continues a read past its first batch on its own thread, publishing each
/// further entry as it is found so a poll never waits for the rest.
fn spawn_scan(path: PathBuf, read_dir: fs::ReadDir) {
    thread::spawn(move || {
        for entry in read_dir {
            match entry.and_then(|entry| directory_entry_from(&entry)) {
                Ok(entry) => publish(&path, entry),
                Err(err) => {
                    mark_done(&path, Some(err.to_string()));
                    return;
                }
            }
        }
        mark_done(&path, None);
    });
}

/// Records one found entry against the read for `path`, if that is still
/// the one being asked about - a stale read, superseded by a fresh one for
/// the same or a different path, has nowhere left to publish to and simply
/// finishes unread.
fn publish(path: &Path, entry: DirectoryEntry) {
    let cell = SCAN.get_or_init(|| Mutex::new(None));
    let mut guard = cell
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(scan) = guard.as_mut()
        && scan.path == path
    {
        scan.entries.push(entry);
    }
}

/// Marks the read for `path` finished, with `error` if it stopped early, if
/// it is still the current one.
fn mark_done(path: &Path, error: Option<String>) {
    let cell = SCAN.get_or_init(|| Mutex::new(None));
    let mut guard = cell
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(scan) = guard.as_mut()
        && scan.path == path
    {
        scan.done = true;
        scan.error = error;
    }
}

/// [`SCAN`] is one slot for the whole process. Every test that can reach
/// [`poll`] - here and the few in `lib.rs` that send a
/// [`protocol::Request::ListDirectory`] - takes this first, the same
/// discipline `lib.rs`'s own `journal_to_themselves` uses for the shared
/// undo journal: two tests racing for one slot would otherwise let an
/// unrelated small directory's read stomp a large one's still in progress.
#[cfg(test)]
static SERIAL: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) fn serially() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::{poll, serially};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A fresh, uniquely named scratch directory.
    fn scratch() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rse-directory-scan-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Polls `path` until the read finishes, or panics after a generous
    /// number of tries - none of this module's fixtures are large enough
    /// for a hang here to be anything but a defect. The first poll asks for
    /// `refresh_first`; every poll after that asks `false`, continuing
    /// whichever read the first one started rather than restarting it -
    /// restarting on every poll is exactly the bug (#717) that once made
    /// this loop spin forever, since a read that had just finished looked,
    /// to the very next poll, indistinguishable from one never begun.
    fn poll_to_completion(path: &Path, refresh_first: bool) -> Vec<String> {
        let mut refresh = refresh_first;
        for _ in 0..1000 {
            let (entries, done, error) = poll(path, refresh).unwrap();
            refresh = false;
            if done {
                assert!(error.is_none(), "unexpected failure: {error:?}");
                return entries.into_iter().map(|entry| entry.name).collect();
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the read of {} never finished", path.display());
    }

    #[test]
    fn a_small_directory_answers_done_on_the_first_poll() {
        let _serial = serially();
        let root = scratch();
        fs::write(root.join("a.txt"), "").unwrap();
        fs::write(root.join("b.txt"), "").unwrap();

        let (entries, done, error) = poll(&root, true).unwrap();

        assert!(
            done,
            "a directory well under FIRST_BATCH should finish in one read"
        );
        assert!(error.is_none());
        assert_eq!(
            entries
                .into_iter()
                .map(|entry| entry.name)
                .collect::<Vec<_>>(),
            vec!["a.txt".to_owned(), "b.txt".to_owned()]
        );
    }

    #[test]
    fn a_large_directory_answers_partial_before_it_finishes() {
        let _serial = serially();
        let root = scratch();
        for index in 0..(super::FIRST_BATCH + 50) {
            fs::write(root.join(format!("file-{index:04}.txt")), "").unwrap();
        }

        let (first_entries, first_done, first_error) = poll(&root, true).unwrap();

        assert!(
            !first_done,
            "a directory over FIRST_BATCH must not finish in the first poll"
        );
        assert!(first_error.is_none());
        assert_eq!(first_entries.len(), super::FIRST_BATCH);

        // Continuing the same read - never another `refresh: true` - is
        // what carries it the rest of the way.
        let names = poll_to_completion(&root, false);
        assert_eq!(names.len(), super::FIRST_BATCH + 50);
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "the finished read must be in name order");
    }

    #[test]
    fn a_finished_read_is_cached_until_refresh_asks_for_another() {
        let _serial = serially();
        let root = scratch();
        fs::write(root.join("one.txt"), "").unwrap();
        assert_eq!(poll_to_completion(&root, true), vec!["one.txt".to_owned()]);

        // A file that appears after the read finished is not found by an
        // ordinary poll: the cached result from the first read is what
        // keeps being shown, the same rule `all_repositories::poll` follows
        // for its own cache.
        fs::write(root.join("two.txt"), "").unwrap();
        let (cached, done, _) = poll(&root, false).unwrap();
        assert!(done, "the earlier read's own completion still holds");
        assert_eq!(
            cached
                .into_iter()
                .map(|entry| entry.name)
                .collect::<Vec<_>>(),
            vec!["one.txt".to_owned()],
            "an ordinary poll must not have started reading again"
        );

        // Only `refresh: true` picks up the new file.
        let mut names = poll_to_completion(&root, true);
        names.sort();
        assert_eq!(names, vec!["one.txt".to_owned(), "two.txt".to_owned()]);
    }

    #[test]
    fn a_missing_directory_is_an_error_rather_than_an_empty_read() {
        let _serial = serially();
        let root = scratch();
        let missing = root.join("does-not-exist");

        assert!(poll(&missing, true).is_err());
    }
}
