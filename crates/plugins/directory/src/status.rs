//! Whether a working copy has uncommitted changes to the files it tracks.
//!
//! Read from the checkout's own index and the files beside it. Per decision
//! D10 this application describes source control and never drives it:
//! nothing here runs a command, and nothing here writes to a working copy.
//!
//! The index (`.git/index`) lists every tracked path with the size and
//! modification time recorded when it was last staged, and the identifier
//! of its content. That is enough to answer the question three ways:
//!
//! - the file is gone, or its size differs: it changed;
//! - its size and its recorded time both match: it did not;
//! - its size matches but its time does not: unknown from the index alone,
//!   so the content is hashed and compared. A touched file is not a
//!   changed file, and reporting one as the other would make the answer
//!   noise rather than information.
//!
//! Untracked files are not counted. Deciding whether one is ignored needs
//! the ignore rules, which is a larger job than this; the wording this
//! produces says "tracked" so nobody reads it as "clean".

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::fs;
use std::path::Path;

/// The largest index this will read. A checkout with a bigger one is
/// enormous, and an answer that took a second to produce is worse than one
/// that says it did not look.
const MAX_INDEX_BYTES: u64 = 32 * 1024 * 1024;

/// How many tracked files this will examine. Past this the answer is
/// partial, and says so.
const MAX_ENTRIES: usize = 20_000;

/// The largest file this will hash to settle a changed timestamp. Past this
/// the answer for that file is unknown, which makes the whole answer
/// partial rather than wrong.
const MAX_HASH_BYTES: u64 = 8 * 1024 * 1024;

/// What a checkout's tracked files look like against its index.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingTree {
    /// How many tracked files differ from what was last staged - modified
    /// or missing.
    pub changed: usize,
    /// How many tracked files were examined.
    pub examined: usize,
    /// Whether something stopped this looking at everything: an index past
    /// the cap, more entries than the cap, or a file too large to hash.
    /// A partial answer can say "at least this much changed", never "no
    /// changes".
    pub partial: bool,
}

impl WorkingTree {
    /// Whether anything tracked differs from what was last staged.
    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.changed > 0
    }

    /// A line for a reader, saying exactly what was counted.
    ///
    /// Never the word "clean" on its own: untracked files are not counted,
    /// and a checkout full of new files would wear it undeservedly.
    #[must_use]
    pub fn summary(&self) -> String {
        match (self.changed, self.partial) {
            (0, false) => "no uncommitted changes to tracked files".to_owned(),
            (0, true) => format!(
                "no uncommitted changes in the {} tracked files checked (not all of them)",
                self.examined
            ),
            (1, false) => "1 tracked file changed".to_owned(),
            (changed, false) => format!("{changed} tracked files changed"),
            (1, true) => "at least 1 tracked file changed".to_owned(),
            (changed, true) => format!("at least {changed} tracked files changed"),
        }
    }
}

/// One tracked file, as the index records it.
struct Entry {
    path: String,
    size: u32,
    mtime_seconds: u32,
    object_id: [u8; 20],
}

/// Compares `work_tree`'s tracked files with the index in `git_dir`.
///
/// `None` when there is no index to read, or it is not one this
/// understands - a fresh clone with nothing staged, or a format from the
/// future. Not knowing is reported as not knowing.
#[must_use]
pub fn working_tree(git_dir: &Path, work_tree: &Path) -> Option<WorkingTree> {
    let index = git_dir.join("index");
    if fs::metadata(&index).ok()?.len() > MAX_INDEX_BYTES {
        return Some(WorkingTree {
            changed: 0,
            examined: 0,
            partial: true,
        });
    }

    let bytes = fs::read(index).ok()?;
    let entries = parse_index(&bytes)?;

    let mut status = WorkingTree {
        changed: 0,
        examined: 0,
        partial: entries.len() > MAX_ENTRIES,
    };

    for entry in entries.iter().take(MAX_ENTRIES) {
        status.examined += 1;
        match compare(work_tree, entry) {
            Comparison::Same => {}
            Comparison::Changed => status.changed += 1,
            Comparison::Unknown => status.partial = true,
        }
    }

    Some(status)
}

/// What one tracked file turned out to be.
enum Comparison {
    Same,
    Changed,
    Unknown,
}

fn compare(work_tree: &Path, entry: &Entry) -> Comparison {
    let path = work_tree.join(&entry.path);
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        // Tracked and not there: deleted, which is a change.
        return Comparison::Changed;
    };

    if metadata.is_dir() {
        // A tracked path that is now a directory is a change, and not one
        // worth describing further here.
        return Comparison::Changed;
    }

    if metadata.len() != u64::from(entry.size) {
        return Comparison::Changed;
    }

    let unchanged_time = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .is_some_and(|since| since.as_secs() == u64::from(entry.mtime_seconds));
    if unchanged_time {
        return Comparison::Same;
    }

    // Same size, different timestamp: only the content settles it. A file
    // that was merely touched is not a changed file.
    if metadata.len() > MAX_HASH_BYTES {
        return Comparison::Unknown;
    }
    match fs::read(&path) {
        Ok(content) if blob_id(&content) == entry.object_id => Comparison::Same,
        Ok(_) => Comparison::Changed,
        Err(_) => Comparison::Unknown,
    }
}

/// The identifier a checkout stores for `content`: secure hash algorithm 1
/// (SHA-1) over the header `blob <length>\0` and then the bytes, which is
/// how the format defines a blob's name.
fn blob_id(content: &[u8]) -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", content.len()).as_bytes());
    hasher.update(content);
    hasher.finalize().into()
}

/// Reads the tracked paths out of an index file.
///
/// Versions 2 and 3 are read. Version 4 compresses path names against the
/// previous entry, which this does not unpack: an unrecognised version
/// yields `None`, and the caller reports not knowing rather than guessing.
fn parse_index(bytes: &[u8]) -> Option<Vec<Entry>> {
    if bytes.len() < 12 || &bytes[..4] != b"DIRC" {
        return None;
    }
    let version = u32::from_be_bytes(bytes[4..8].try_into().ok()?);
    if !matches!(version, 2 | 3) {
        return None;
    }
    let count = u32::from_be_bytes(bytes[8..12].try_into().ok()?) as usize;

    let mut entries = Vec::with_capacity(count.min(MAX_ENTRIES));
    let mut offset = 12usize;
    for _ in 0..count {
        // Each entry: ten four-byte fields, a twenty-byte object identifier,
        // two bytes of flags, then the path, padded to a multiple of eight.
        if offset + 62 > bytes.len() {
            return None;
        }
        let mtime_seconds = u32::from_be_bytes(bytes[offset + 8..offset + 12].try_into().ok()?);
        let size = u32::from_be_bytes(bytes[offset + 36..offset + 40].try_into().ok()?);
        let object_id: [u8; 20] = bytes[offset + 40..offset + 60].try_into().ok()?;
        let flags = u16::from_be_bytes(bytes[offset + 60..offset + 62].try_into().ok()?);

        let extended = flags & 0x4000 != 0;
        let name_start = offset + 62 + if extended { 2 } else { 0 };
        let name_len = usize::from(flags & 0x0FFF);
        if name_len == 0x0FFF {
            // A path longer than the flag field can hold is stored to the
            // next zero byte. Rare, and not worth guessing at.
            return None;
        }
        if name_start + name_len > bytes.len() {
            return None;
        }
        let path = String::from_utf8(bytes[name_start..name_start + name_len].to_vec()).ok()?;

        entries.push(Entry {
            path,
            size,
            mtime_seconds,
            object_id,
        });

        let entry_len = name_start - offset + name_len;
        offset += entry_len.div_ceil(8) * 8;
    }
    Some(entries)
}

#[cfg(test)]
mod tests {
    use super::{WorkingTree, blob_id, working_tree};
    use std::path::{Path, PathBuf};

    fn temp_dir(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rse-status-{}-{}-{name}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Writes an index holding `files`, each as the index would record it
    /// if the file on disk were exactly what was staged.
    fn write_index(git_dir: &Path, work_tree: &Path, files: &[&str]) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"DIRC");
        bytes.extend_from_slice(&2u32.to_be_bytes());
        bytes.extend_from_slice(&u32::try_from(files.len()).unwrap().to_be_bytes());

        for name in files {
            let path = work_tree.join(name);
            let content = std::fs::read(&path).unwrap_or_default();
            let metadata = std::fs::metadata(&path).unwrap();
            let mtime = u32::try_from(
                metadata
                    .modified()
                    .unwrap()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            )
            .expect("a fixture written now is inside the epoch seconds the index holds");

            let start = bytes.len();
            bytes.extend_from_slice(&0u32.to_be_bytes()); // ctime seconds
            bytes.extend_from_slice(&0u32.to_be_bytes()); // ctime nanoseconds
            bytes.extend_from_slice(&mtime.to_be_bytes());
            bytes.extend_from_slice(&0u32.to_be_bytes()); // mtime nanoseconds
            bytes.extend_from_slice(&0u32.to_be_bytes()); // device
            bytes.extend_from_slice(&0u32.to_be_bytes()); // inode
            bytes.extend_from_slice(&0o100_644u32.to_be_bytes());
            bytes.extend_from_slice(&0u32.to_be_bytes()); // user
            bytes.extend_from_slice(&0u32.to_be_bytes()); // group
            bytes.extend_from_slice(&u32::try_from(content.len()).unwrap().to_be_bytes());
            bytes.extend_from_slice(&blob_id(&content));
            bytes.extend_from_slice(&u16::try_from(name.len()).unwrap().to_be_bytes());
            bytes.extend_from_slice(name.as_bytes());

            let written = bytes.len() - start;
            bytes.resize(start + written.div_ceil(8) * 8, 0);
        }

        std::fs::create_dir_all(git_dir).unwrap();
        std::fs::write(git_dir.join("index"), bytes).unwrap();
    }

    /// A checkout whose files match its index exactly.
    fn checkout(name: &str, files: &[(&str, &str)]) -> (PathBuf, PathBuf) {
        let work_tree = temp_dir(name);
        for (path, content) in files {
            std::fs::write(work_tree.join(path), content).unwrap();
        }
        let git_dir = work_tree.join(".git");
        write_index(
            &git_dir,
            &work_tree,
            &files.iter().map(|(path, _)| *path).collect::<Vec<_>>(),
        );
        (git_dir, work_tree)
    }

    #[test]
    fn a_checkout_matching_its_index_has_no_tracked_changes() {
        let (git_dir, work_tree) = checkout("clean", &[("a.txt", "one"), ("b.txt", "two")]);

        let status = working_tree(&git_dir, &work_tree).expect("an index to read");

        assert_eq!(status.changed, 0);
        assert_eq!(status.examined, 2);
        assert!(!status.partial);
        assert!(!status.is_dirty());
        assert_eq!(status.summary(), "no uncommitted changes to tracked files");

        std::fs::remove_dir_all(&work_tree).unwrap();
    }

    #[test]
    fn a_modified_tracked_file_is_a_change() {
        let (git_dir, work_tree) = checkout("modified", &[("a.txt", "one"), ("b.txt", "two")]);
        std::fs::write(work_tree.join("a.txt"), "one, edited").unwrap();

        let status = working_tree(&git_dir, &work_tree).expect("an index to read");

        assert_eq!(status.changed, 1);
        assert!(status.is_dirty());
        assert_eq!(status.summary(), "1 tracked file changed");

        std::fs::remove_dir_all(&work_tree).unwrap();
    }

    #[test]
    fn a_deleted_tracked_file_is_a_change() {
        let (git_dir, work_tree) = checkout("deleted", &[("a.txt", "one"), ("b.txt", "two")]);
        std::fs::remove_file(work_tree.join("b.txt")).unwrap();

        let status = working_tree(&git_dir, &work_tree).expect("an index to read");

        assert_eq!(status.changed, 1);
        assert_eq!(status.summary(), "1 tracked file changed");

        std::fs::remove_dir_all(&work_tree).unwrap();
    }

    #[test]
    fn a_file_that_was_only_touched_is_not_a_change() {
        // The case the hash exists for: same contents, later timestamp,
        // which is what a checkout or a build leaves behind. Reporting it
        // as a change would make the answer noise.
        let (git_dir, work_tree) = checkout("touched", &[("a.txt", "one")]);
        let path = work_tree.join("a.txt");
        std::fs::write(&path, "one").unwrap();
        filetime_forward(&path);

        let status = working_tree(&git_dir, &work_tree).expect("an index to read");

        assert_eq!(status.changed, 0, "same contents, later timestamp");
        assert!(!status.partial);

        std::fs::remove_dir_all(&work_tree).unwrap();
    }

    #[test]
    fn a_file_changed_without_changing_size_is_still_a_change() {
        let (git_dir, work_tree) = checkout("same-size", &[("a.txt", "one")]);
        let path = work_tree.join("a.txt");
        std::fs::write(&path, "ONE").unwrap();
        filetime_forward(&path);

        let status = working_tree(&git_dir, &work_tree).expect("an index to read");

        assert_eq!(
            status.changed, 1,
            "the size matches, so only the content could tell"
        );

        std::fs::remove_dir_all(&work_tree).unwrap();
    }

    #[test]
    fn untracked_files_are_not_counted_and_the_wording_says_so() {
        let (git_dir, work_tree) = checkout("untracked", &[("a.txt", "one")]);
        std::fs::write(work_tree.join("new.txt"), "not staged").unwrap();

        let status = working_tree(&git_dir, &work_tree).expect("an index to read");

        assert_eq!(status.changed, 0);
        assert!(
            status.summary().contains("tracked"),
            "a checkout of new files must not read as clean: {}",
            status.summary()
        );

        std::fs::remove_dir_all(&work_tree).unwrap();
    }

    #[test]
    fn a_partial_answer_never_reads_as_no_changes() {
        let partial = WorkingTree {
            changed: 0,
            examined: 20_000,
            partial: true,
        };
        assert!(partial.summary().contains("not all of them"));

        let some = WorkingTree {
            changed: 3,
            examined: 20_000,
            partial: true,
        };
        assert_eq!(some.summary(), "at least 3 tracked files changed");
    }

    #[test]
    fn an_index_this_does_not_understand_reports_nothing() {
        let work_tree = temp_dir("unknown-version");
        let git_dir = work_tree.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        let mut bytes = b"DIRC".to_vec();
        bytes.extend_from_slice(&4u32.to_be_bytes()); // version 4: compressed paths
        bytes.extend_from_slice(&0u32.to_be_bytes());
        std::fs::write(git_dir.join("index"), bytes).unwrap();

        assert!(
            working_tree(&git_dir, &work_tree).is_none(),
            "not knowing is reported as not knowing"
        );

        std::fs::remove_dir_all(&work_tree).unwrap();
    }

    #[test]
    fn a_checkout_with_no_index_reports_nothing() {
        let work_tree = temp_dir("no-index");
        let git_dir = work_tree.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();

        assert!(working_tree(&git_dir, &work_tree).is_none());

        std::fs::remove_dir_all(&work_tree).unwrap();
    }

    /// Moves a file's modification time forward, so the index's recorded
    /// time no longer matches and the content has to settle it.
    fn filetime_forward(path: &Path) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(120);
        file.set_modified(later).unwrap();
    }
}
