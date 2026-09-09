//! A bounded, cancellable directory indexer: walks a tree on a worker
//! thread, reports progress as it goes, and stops the moment it is asked
//! to - the shape a lot of this application's own work takes.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How deep the walk may go before it refuses to descend further. A
/// symlink loop is the reason this exists.
pub const MAX_DEPTH: usize = 64;

/// How many entries are gathered before a progress report is sent.
const REPORT_EVERY: usize = 256;

/// What the indexer found for one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Path relative to the root the walk started from.
    pub relative: PathBuf,
    /// Size in bytes, or `None` for a directory.
    pub size: Option<u64>,
    /// Lowercase extension, without the dot.
    pub extension: Option<String>,
}

/// A running total, sent to whoever asked for the index.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    /// Files seen so far.
    pub files: usize,
    /// Directories entered so far.
    pub directories: usize,
    /// Bytes accounted for so far.
    pub bytes: u64,
}

/// Why a walk stopped early.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Halted {
    /// The caller cancelled it.
    Cancelled,
    /// The depth limit was reached.
    TooDeep { at: PathBuf, depth: usize },
    /// The filesystem refused a directory.
    Unreadable { at: PathBuf, reason: String },
}

impl fmt::Display for Halted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => write!(formatter, "cancelled"),
            Self::TooDeep { at, depth } => {
                write!(formatter, "{} is deeper than {depth} levels", at.display())
            }
            Self::Unreadable { at, reason } => {
                write!(formatter, "could not read {}: {reason}", at.display())
            }
        }
    }
}

impl std::error::Error for Halted {}

/// Anything that can be told how a walk is going.
pub trait Observer: Send {
    /// Called every [`REPORT_EVERY`] entries, and once at the end.
    fn progress(&self, progress: Progress);

    /// Called once, when the walk stops for a reason other than finishing.
    fn halted(&self, reason: &Halted) {
        let _ = reason;
    }
}

/// An observer that keeps the last report, for a caller that only wants
/// the total.
#[derive(Debug, Default)]
pub struct LastProgress {
    seen: Mutex<Progress>,
}

impl LastProgress {
    /// The most recent report, or the default if there has been none.
    #[must_use]
    pub fn get(&self) -> Progress {
        self.seen.lock().map(|seen| *seen).unwrap_or_default()
    }
}

impl Observer for LastProgress {
    fn progress(&self, progress: Progress) {
        if let Ok(mut seen) = self.seen.lock() {
            *seen = progress;
        }
    }
}

/// A handle the caller keeps to stop a walk in flight.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// Asks the walk to stop at the next entry.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether the walk has been asked to stop.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// The index itself: every entry, plus a count per extension.
#[derive(Debug, Default)]
pub struct Index {
    entries: Vec<Entry>,
    by_extension: BTreeMap<String, usize>,
    elapsed: Duration,
}

impl Index {
    /// Every entry, in the order the walk found them.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// How many files carry each extension, largest first.
    #[must_use]
    pub fn ranked_extensions(&self) -> Vec<(&str, usize)> {
        let mut ranked: Vec<(&str, usize)> = self
            .by_extension
            .iter()
            .map(|(extension, count)| (extension.as_str(), *count))
            .collect();
        ranked.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(right.0)));
        ranked
    }

    /// Total size of every file in the index.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.entries.iter().filter_map(|entry| entry.size).sum()
    }

    /// How long the walk took.
    #[must_use]
    pub const fn elapsed(&self) -> Duration {
        self.elapsed
    }

    fn push(&mut self, entry: Entry) {
        if let Some(extension) = entry.extension.clone() {
            *self.by_extension.entry(extension).or_insert(0) += 1;
        }
        self.entries.push(entry);
    }
}

/// Walks `root`, reporting to `observer`, until it finishes or `cancel`
/// says otherwise.
///
/// # Errors
/// Returns [`Halted`] when the walk stopped for any reason other than
/// running out of entries.
pub fn index(
    root: &Path,
    cancel: &Cancel,
    observer: &dyn Observer,
) -> Result<Index, (Index, Halted)> {
    let started = Instant::now();
    let mut index = Index::default();
    let mut progress = Progress::default();
    let mut pending = vec![(root.to_path_buf(), 0usize)];

    while let Some((directory, depth)) = pending.pop() {
        if cancel.is_cancelled() {
            index.elapsed = started.elapsed();
            let reason = Halted::Cancelled;
            observer.halted(&reason);
            return Err((index, reason));
        }

        if depth > MAX_DEPTH {
            index.elapsed = started.elapsed();
            let reason = Halted::TooDeep {
                at: directory,
                depth: MAX_DEPTH,
            };
            observer.halted(&reason);
            return Err((index, reason));
        }

        let listing = match std::fs::read_dir(&directory) {
            Ok(listing) => listing,
            Err(err) => {
                index.elapsed = started.elapsed();
                let reason = Halted::Unreadable {
                    at: directory,
                    reason: err.to_string(),
                };
                observer.halted(&reason);
                return Err((index, reason));
            }
        };

        progress.directories += 1;

        for candidate in listing.flatten() {
            let path = candidate.path();
            let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            let is_dir = candidate.file_type().is_ok_and(|kind| kind.is_dir());

            if is_dir {
                pending.push((path, depth + 1));
                index.push(Entry {
                    relative,
                    size: None,
                    extension: None,
                });
                continue;
            }

            let size = candidate.metadata().map(|meta| meta.len()).unwrap_or(0);
            progress.files += 1;
            progress.bytes += size;

            index.push(Entry {
                extension: relative
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map(str::to_lowercase),
                relative,
                size: Some(size),
            });

            if progress.files % REPORT_EVERY == 0 {
                observer.progress(progress);
            }
        }
    }

    observer.progress(progress);
    index.elapsed = started.elapsed();
    Ok(index)
}

/// Runs [`index`] on a worker thread, handing back the receiver and the
/// cancel handle so the caller keeps its own thread free.
#[must_use]
pub fn spawn(root: PathBuf) -> (Receiver<Result<Index, (Index, Halted)>>, Cancel) {
    let (sender, receiver): (Sender<_>, Receiver<_>) = channel();
    let cancel = Cancel::default();
    let handle = cancel.clone();

    std::thread::spawn(move || {
        let observer = LastProgress::default();
        let _ = sender.send(index(&root, &handle, &observer));
    });

    (receiver, cancel)
}

fn main() {
    let root = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);

    let observer = LastProgress::default();
    let cancel = Cancel::default();

    match index(&root, &cancel, &observer) {
        Ok(index) => {
            println!(
                "{} entries, {} bytes, {:?}",
                index.entries().len(),
                index.total_bytes(),
                index.elapsed()
            );
            for (extension, count) in index.ranked_extensions().into_iter().take(8) {
                println!("  {extension:<10} {count:>6}");
            }
        }
        Err((partial, reason)) => {
            eprintln!("stopped after {} entries: {reason}", partial.entries().len());
        }
    }
}
