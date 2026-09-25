//! Caches [`Response::FileView`]s by path, modification time and size, and
//! tracks which path each is being parsed for so a [`protocol::Request::Cancel`]
//! has something to stop (#718).
//!
//! GUIDANCE.md §3.3: "Thumbnails and parses are cancellable, and cached by
//! (path, mtime, size)." A thumbnail is not a separate thing to cache: every
//! plugin that produces one puts it inside the same view its `view()` call
//! returns (see `plugin-api`'s `thumbnail` module), so caching a view caches
//! its thumbnail along with it.
//!
//! Keying by modification time and size, rather than just path, is what
//! makes touching a file invalidate its entry without any explicit
//! invalidation step: a changed file has a different key, so a request for
//! it misses the cache and is answered fresh; the stale entry under the old
//! key simply ages out under [`CAPACITY`] like any other.

use protocol::Response;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// What a view is cached under: the same path answers differently once its
/// modification time or size has changed, so either makes for a different
/// entry.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    path: PathBuf,
    modified: Option<u64>,
    size: u64,
}

/// Builds the key `path` would be cached under, from its current metadata.
///
/// # Errors
/// Returns an error if `path` cannot be stat'd - the same failure
/// [`crate::view_file_uncapped`] would meet reading it.
pub(crate) fn key_for(path: &Path) -> std::io::Result<Key> {
    let metadata = std::fs::metadata(path)
        .map_err(|err| std::io::Error::new(err.kind(), format!("{}: {err}", path.display())))?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs());
    Ok(Key {
        path: path.to_path_buf(),
        modified,
        size: metadata.len(),
    })
}

/// The most views held at once. Chosen as comfortably more than a reader
/// holds warm attention on in one sitting - a folder pane rarely shows more
/// than a couple of hundred rows at a time - while staying small enough
/// that the whole cache is a handful of megabytes even if every entry held
/// a full-size thumbnail.
const CAPACITY: usize = 200;

/// Entries in least-recently-used order: [`Cache::get`] moves a hit to the
/// back, and [`Cache::put`] evicts from the front once [`CAPACITY`] is
/// passed. A `Vec` rather than a real LRU structure because `CAPACITY` is
/// small enough that the linear scan and shift this costs are not worth a
/// second data structure to track order.
struct Cache {
    entries: Vec<(Key, Response)>,
}

static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

fn cache() -> &'static Mutex<Cache> {
    CACHE.get_or_init(|| {
        Mutex::new(Cache {
            entries: Vec::new(),
        })
    })
}

/// The view cached under `key`, if there is one - moved to the
/// most-recently-used end first.
pub(crate) fn lookup(key: &Key) -> Option<Response> {
    let mut guard = cache().lock().unwrap_or_else(PoisonError::into_inner);
    let at = guard.entries.iter().position(|(k, _)| k == key)?;
    let (key, response) = guard.entries.remove(at);
    guard.entries.push((key, response.clone()));
    Some(response)
}

/// Caches `response` under `key`, evicting the least-recently-used entry
/// first if this would pass [`CAPACITY`].
pub(crate) fn store(key: Key, response: Response) {
    let mut guard = cache().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(at) = guard.entries.iter().position(|(k, _)| *k == key) {
        guard.entries.remove(at);
    }
    guard.entries.push((key, response));
    while guard.entries.len() > CAPACITY {
        guard.entries.remove(0);
    }
}

/// Discards any cached view of `path`, whatever modification time or size
/// it was cached under.
///
/// [`Key`] already makes an edit that changes a file's modification time or
/// size invalidate itself, without this - but a rewrite that leaves both
/// unchanged (same length, within the same reported second) would not, and
/// an editor saving over a file it just showed is exactly the case where
/// that would matter. Called from `write_atomically`, the one place a
/// file's bytes are actually replaced, so an ordinary save and undoing one
/// are both covered by writing this once.
pub(crate) fn forget(path: &Path) {
    let mut guard = cache().lock().unwrap_or_else(PoisonError::into_inner);
    guard.entries.retain(|(key, _)| key.path != path);
}

/// One path's most recent parse: the flag a [`protocol::Request::Cancel`]
/// for that path sets, and the generation that tells a superseded
/// registration it is no longer the one to remove.
struct InFlight {
    generation: u64,
    cancelled: Arc<AtomicBool>,
}

static IN_FLIGHT: OnceLock<Mutex<HashMap<PathBuf, InFlight>>> = OnceLock::new();

fn in_flight() -> &'static Mutex<HashMap<PathBuf, InFlight>> {
    IN_FLIGHT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Hands out [`InFlight::generation`]. Never reused within a run, so a
/// registration this module has already forgotten can never be mistaken
/// for the current one - the same discipline `directory_scan`'s own
/// generation counter keeps.
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Registers `path` as about to be parsed, returning a handle whose
/// [`Registration::cancelled`] the parse should poll instead of blocking on
/// its result alone.
///
/// Registering a path a request is already registered for replaces that
/// earlier registration: only the most recent parse of a path is the one a
/// [`cancel`] for that path can reach, which is correct because a front end
/// only ever has one outstanding request per path (a new selection cancels
/// the last one itself before asking for the next - see `App::load_file_view`
/// in both front ends).
pub(crate) fn begin(path: &Path) -> Registration {
    let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut guard = in_flight().lock().unwrap_or_else(PoisonError::into_inner);
    guard.insert(
        path.to_path_buf(),
        InFlight {
            generation,
            cancelled: Arc::clone(&cancelled),
        },
    );
    Registration {
        path: path.to_path_buf(),
        generation,
        cancelled,
    }
}

/// Sets the cancelled flag for whichever parse is currently registered for
/// `path`, if any is. A path nothing is registered for - the parse already
/// finished, or nothing asked for it - is not an error: cancelling is a
/// best-effort courtesy to work still under way, not a command that must
/// find a target.
pub(crate) fn cancel(path: &Path) {
    let guard = in_flight().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(entry) = guard.get(path) {
        entry.cancelled.store(true, Ordering::SeqCst);
    }
}

/// A parse's registration in [`IN_FLIGHT`], live for as long as the parse
/// is waiting to answer. Removes itself on drop - covering every return
/// path a parse can take, cancelled or not - so a finished parse cannot be
/// cancelled after the fact and a path is never left pointing at a
/// registration nothing will ever poll again.
pub(crate) struct Registration {
    path: PathBuf,
    generation: u64,
    cancelled: Arc<AtomicBool>,
}

impl Registration {
    /// Whether a [`cancel`] for this registration's path has been asked for
    /// since it began.
    pub(crate) fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut guard = in_flight().lock().unwrap_or_else(PoisonError::into_inner);
        if guard
            .get(&self.path)
            .is_some_and(|entry| entry.generation == self.generation)
        {
            guard.remove(&self.path);
        }
    }
}

/// [`CACHE`] is one slot for the whole process, so a test asserting on
/// exactly what it holds - not merely on the presence or absence of the one
/// key it stored, but on the entry surviving eviction until it is checked -
/// races every other test that stores into the same cache while `cargo
/// test` runs them in parallel. The same discipline `directory_scan`'s own
/// `serially` keeps for its single scan slot.
#[cfg(test)]
static SERIAL: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) fn serially() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Empties [`CACHE`], so a test asserting on exact occupancy or eviction
/// order starts from nothing rather than whatever earlier tests, run in the
/// same process, happened to leave behind.
#[cfg(test)]
fn clear() {
    cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entries
        .clear();
}

#[cfg(test)]
mod tests {
    use super::{Key, serially, store};
    use protocol::Response;
    use std::path::PathBuf;

    fn key(name: &str, modified: u64, size: u64) -> Key {
        Key {
            path: PathBuf::from(name),
            modified: Some(modified),
            size,
        }
    }

    fn view(content: &str) -> Response {
        Response::FileView {
            plugin: "text".to_owned(),
            data: serde_json::json!({ "content": content }),
            also: Vec::new(),
        }
    }

    #[test]
    fn a_stored_view_is_returned_by_its_exact_key() {
        let _serial = serially();
        let k = key("/a", 1, 2);
        store(k.clone(), view("hello"));

        assert_eq!(super::lookup(&k), Some(view("hello")));
    }

    #[test]
    fn a_different_modification_time_is_a_different_entry() {
        let _serial = serially();
        let stale = key("/a", 1, 2);
        let fresh = key("/a", 2, 2);
        store(stale, view("old"));

        assert_eq!(
            super::lookup(&fresh),
            None,
            "a key naming a different modification time must not see the earlier entry"
        );
    }

    #[test]
    fn the_cache_stays_within_its_bound_under_many_files() {
        let _serial = serially();
        super::clear();
        for index in 0..(super::CAPACITY + 50) {
            store(
                key(&format!("/many/{index}"), index as u64, index as u64),
                view("x"),
            );
        }

        let guard = super::cache().lock().unwrap();
        assert!(
            guard.entries.len() <= super::CAPACITY,
            "the cache grew past its bound: {} entries",
            guard.entries.len()
        );
    }

    #[test]
    fn the_least_recently_used_entry_is_evicted_first() {
        let _serial = serially();
        super::clear();
        let oldest = key("/oldest", 1, 1);
        let newest = key("/newest", 1, 1);
        store(oldest.clone(), view("oldest"));
        // Fills the cache to exactly its bound without evicting anything
        // yet: one slot for `oldest`, the rest for the fillers.
        for index in 0..(super::CAPACITY - 1) {
            store(key(&format!("/filler/{index}"), 1, 1), view("filler"));
        }
        // Touching `oldest` again moves it to the most-recently-used end,
        // so the next insertion evicts the filler that was never touched
        // instead.
        assert_eq!(super::lookup(&oldest), Some(view("oldest")));
        store(newest.clone(), view("newest"));

        assert_eq!(super::lookup(&oldest), Some(view("oldest")));
        assert_eq!(super::lookup(&newest), Some(view("newest")));
    }

    #[test]
    fn cancelling_a_path_nothing_is_registered_for_does_nothing() {
        // No panic, no error: see `cancel`'s own doc comment.
        super::cancel(&PathBuf::from("/nobody/is/parsing/this"));
    }

    #[test]
    fn a_registration_is_forgotten_once_dropped() {
        let path = PathBuf::from("/dropped");
        let registration = super::begin(&path);
        drop(registration);

        // Nothing is registered any more, so cancelling is a no-op rather
        // than reaching a stale handle - proven indirectly, since `cancel`
        // never panics either way; what this guards is `Registration::drop`
        // itself running without one.
        super::cancel(&path);
    }

    #[test]
    fn a_later_registration_for_the_same_path_is_not_removed_by_the_earlier_one_s_drop() {
        let path = PathBuf::from("/superseded");
        let first = super::begin(&path);
        let second = super::begin(&path);
        drop(first);

        assert!(
            !second.cancelled(),
            "dropping the superseded registration must not touch the current one"
        );
        super::cancel(&path);
        assert!(
            second.cancelled(),
            "cancelling the path must still reach the current registration"
        );
    }
}
