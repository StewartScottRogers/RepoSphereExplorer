//! A bounded, cancellable directory indexer.
//!
//! The walk itself lives in [`indexer`], which gives a caller full control:
//! an observer for progress, a handle to stop the walk in flight, and a
//! partial index back when it does stop. This is the crate's front door,
//! for the caller who wants none of that and only wants the entries.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod indexer;

use std::path::Path;

pub use indexer::{Cancel, Entry, Halted, Index, MAX_DEPTH, Progress};

/// What went wrong while indexing.
#[derive(Debug)]
pub enum Error {
    /// The walk stopped before it ran out of entries, and this is what it
    /// had counted when it did.
    Halted {
        /// Why it stopped.
        reason: Halted,
        /// How far it had got.
        progress: Progress,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self::Halted { reason, progress } = self;
        write!(
            formatter,
            "{reason} after {} files in {} directories",
            progress.files, progress.directories
        )
    }
}

impl std::error::Error for Error {}

/// Walks `root` and returns every file under it.
///
/// Uncancellable and unobserved: use [`indexer::index`] directly for a
/// walk you need to watch or stop.
///
/// # Errors
/// Returns [`Error::Halted`] if the walk stopped before it ran out of
/// entries — a directory it could not read, or a tree deeper than
/// [`MAX_DEPTH`].
pub fn index(root: &Path) -> Result<Vec<Entry>, Error> {
    let cancel = Cancel::default();
    let observed = indexer::LastProgress::default();

    match indexer::index(root, &cancel, &observed) {
        Ok(found) => Ok(found.entries().to_vec()),
        Err((_partial, reason)) => Err(Error::Halted {
            reason,
            progress: observed.get(),
        }),
    }
}
