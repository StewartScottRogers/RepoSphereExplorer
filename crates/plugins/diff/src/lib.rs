//! Unified diff file type plugin: core and presentation halves.
//!
//! A patch is the most repository-resident file there is, and until now it
//! opened as plain text: a wall of pluses and minuses with no summary of
//! what it touches.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["diff", "patch"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 256 * 1024;

/// One hunk's header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    /// The first line of the range in the old file.
    pub old_start: usize,
    /// How many lines of the old file it covers.
    pub old_lines: usize,
    /// The first line of the range in the new file.
    pub new_start: usize,
    /// How many lines of the new file it covers.
    pub new_lines: usize,
}

/// One file the patch touches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    /// The path as the patch names it, with any `a/` or `b/` prefix
    /// stripped.
    pub path: String,
    /// Where it moved from, when the patch renames it.
    pub renamed_from: Option<String>,
    /// Whether the patch reports it as binary rather than as lines.
    pub binary: bool,
    /// The hunks against this file.
    pub hunks: Vec<Hunk>,
    /// Lines added.
    pub added: usize,
    /// Lines removed.
    pub removed: usize,
}

/// View data produced by [`DiffCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffView {
    /// Every file the patch touches, in order.
    pub files: Vec<FileChange>,
    /// Whether it carries git's own headers rather than bare `---`/`+++`.
    pub git_format: bool,
    /// The commit subject from a `git format-patch` header, when present.
    pub subject: Option<String>,
    /// Total lines added across every file.
    pub added: usize,
    /// Total lines removed across every file.
    pub removed: usize,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Strips git's `a/` or `b/` prefix from a path in a header.
fn strip_prefix(path: &str) -> String {
    let path = path.split('\t').next().unwrap_or(path).trim();
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
        .to_owned()
}

/// The four numbers in an `@@ -1,4 +1,6 @@` header.
fn hunk_header(line: &str) -> Option<Hunk> {
    let inner = line.strip_prefix("@@ ")?;
    let inner = inner.split(" @@").next()?;
    let (old, new) = inner.split_once(' ')?;
    let range = |text: &str, sigil: char| -> Option<(usize, usize)> {
        let text = text.strip_prefix(sigil)?;
        match text.split_once(',') {
            Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
            // A range with no comma covers exactly one line.
            None => Some((text.parse().ok()?, 1)),
        }
    };
    let (old_start, old_lines) = range(old, '-')?;
    let (new_start, new_lines) = range(new, '+')?;
    Some(Hunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
    })
}

/// A fresh, empty record for `path`.
fn opening(path: String) -> FileChange {
    FileChange {
        path,
        renamed_from: None,
        binary: false,
        hunks: Vec::new(),
        added: 0,
        removed: 0,
    }
}

/// What one header line means, applied to the file currently being read.
///
/// Split out of [`parse`] along a real seam: this decides what a header
/// says, and `parse` walks the patch. Returns `true` when the line was a
/// header and needs no further handling.
fn apply_header(line: &str, current: &mut Option<FileChange>, view: &mut DiffView) -> bool {
    if let Some(rest) = line.strip_prefix("rename from ") {
        if let Some(file) = current.as_mut() {
            file.renamed_from = Some(strip_prefix(rest));
        }
        return true;
    }
    if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
        if let Some(file) = current.as_mut() {
            file.binary = true;
        }
        return true;
    }
    if let Some(rest) = line.strip_prefix("+++ ") {
        let path = strip_prefix(rest);
        if path != "/dev/null" {
            match current.as_mut() {
                Some(file) if file.path.is_empty() => file.path = path,
                Some(_) => {}
                None => *current = Some(opening(path)),
            }
        }
        return true;
    }
    if line.starts_with("--- ") {
        // A bare unified diff opens a file here rather than at `diff
        // --git`; the `+++` line that follows fills the path in.
        if !view.git_format && current.is_none() {
            *current = Some(opening(String::new()));
        }
        return true;
    }
    if let Some(hunk) = hunk_header(line) {
        if let Some(file) = current.as_mut() {
            file.hunks.push(hunk);
        }
        return true;
    }
    false
}

/// Everything [`DiffView`] holds, read from `text`.
fn parse(text: &str) -> DiffView {
    let mut view = DiffView {
        files: Vec::new(),
        git_format: false,
        subject: None,
        added: 0,
        removed: 0,
        content: String::new(),
        truncated: false,
    };
    let mut current: Option<FileChange> = None;

    let close = |current: &mut Option<FileChange>, view: &mut DiffView| {
        if let Some(file) = current.take() {
            view.added += file.added;
            view.removed += file.removed;
            view.files.push(file);
        }
    };

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Subject: ") {
            view.subject = Some(
                rest.strip_prefix("[PATCH] ")
                    .unwrap_or(rest)
                    .trim()
                    .to_owned(),
            );
            continue;
        }
        if let Some(rest) = line.strip_prefix("diff --git ") {
            close(&mut current, &mut view);
            view.git_format = true;
            let path = rest
                .split_whitespace()
                .next_back()
                .map_or_else(String::new, strip_prefix);
            current = Some(opening(path));
            continue;
        }
        if apply_header(line, &mut current, &mut view) {
            continue;
        }
        if let Some(file) = current.as_mut() {
            if line.starts_with('+') && !line.starts_with("+++") {
                file.added += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                file.removed += 1;
            }
        }
    }
    close(&mut current, &mut view);
    view
}

/// Whether `prefix` looks like a patch.
fn looks_like_diff(prefix: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(prefix) else {
        return false;
    };
    let mut minus = false;
    let mut plus = false;
    for line in text.lines() {
        if line.starts_with("diff --git ") || hunk_header(line).is_some() {
            return true;
        }
        if line.starts_with("--- ") {
            minus = true;
        }
        if line.starts_with("+++ ") && minus {
            plus = true;
        }
    }
    minus && plus
}

/// The diff plugin's core half.
#[derive(Debug, Default)]
pub struct DiffCore;

impl PluginCore for DiffCore {
    fn name(&self) -> &'static str {
        "diff"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_diff(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let mut view = parse(&content);
        view.content = content;
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The diff plugin's presentation half.
#[derive(Debug, Default)]
pub struct DiffPresentation;

impl PluginPresentation for DiffPresentation {
    fn name(&self) -> &'static str {
        "diff"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "DIFF",
            tint: 0x0028_a745,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: DiffView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();

        if let Some(subject) = &view.subject {
            lines.push(format!("Subject: {subject}"));
        }
        lines.push(format!(
            "{} file(s) changed, {} insertion(s), {} deletion(s){}",
            view.files.len(),
            view.added,
            view.removed,
            if view.git_format { ", git format" } else { "" }
        ));

        for file in &view.files {
            let mut label = file.path.clone();
            if let Some(from) = &file.renamed_from {
                label = format!("{from} -> {label}");
            }
            if file.binary {
                lines.push(format!("  {label}  (binary)"));
                continue;
            }
            lines.push(format!(
                "  {label}  +{} -{}  in {} hunk(s)",
                file.added,
                file.removed,
                file.hunks.len()
            ));
            for hunk in &file.hunks {
                lines.push(format!(
                    "    @@ -{},{} +{},{} @@",
                    hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
                ));
            }
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{DiffCore, DiffPresentation, DiffView, hunk_header, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_a_git_header_a_hunk_or_a_pair_of_file_headers() {
        assert!(DiffCore.sniff(b"diff --git a/x b/x\n"));
        assert!(DiffCore.sniff(b"@@ -1,4 +1,6 @@\n"));
        assert!(DiffCore.sniff(b"--- a/x\n+++ b/x\n"));
    }

    #[test]
    fn does_not_claim_prose_that_merely_starts_with_a_dash() {
        assert!(!DiffCore.sniff(b"--- a section break ---\n"));
        assert!(!DiffCore.sniff(b"- a list item\n+ another\n"));
        assert!(!DiffCore.sniff(b""));
    }

    #[test]
    fn reads_a_hunk_range_with_and_without_a_count() {
        assert_eq!(hunk_header("@@ -1,4 +1,6 @@").unwrap().old_lines, 4);
        // No comma means exactly one line, not zero.
        assert_eq!(hunk_header("@@ -7 +7 @@").unwrap().old_lines, 1);
        assert!(hunk_header("@@ nonsense @@").is_none());
    }

    #[test]
    fn counts_additions_and_removals_per_file() {
        let view = parse(
            "diff --git a/one.txt b/one.txt\n--- a/one.txt\n+++ b/one.txt\n\
             @@ -1,2 +1,3 @@\n context\n-gone\n+new\n+also new\n",
        );

        assert_eq!(view.files.len(), 1);
        assert_eq!(view.files[0].path, "one.txt");
        assert_eq!(view.files[0].added, 2);
        assert_eq!(view.files[0].removed, 1);
        assert_eq!(view.added, 2);
    }

    #[test]
    fn a_rename_records_where_the_file_came_from() {
        let view = parse(
            "diff --git a/old.txt b/new.txt\nsimilarity index 100%\n\
             rename from a/old.txt\nrename to b/new.txt\n",
        );

        assert_eq!(view.files[0].path, "new.txt");
        assert_eq!(view.files[0].renamed_from.as_deref(), Some("old.txt"));
    }

    #[test]
    fn a_binary_file_is_marked_rather_than_counted() {
        let view = parse(
            "diff --git a/logo.png b/logo.png\nBinary files a/logo.png and b/logo.png differ\n",
        );

        assert!(view.files[0].binary);
        assert_eq!(view.files[0].added, 0);
    }

    #[test]
    fn reads_the_subject_of_a_format_patch() {
        let view = parse("From abc\nSubject: [PATCH] Make it work\n\ndiff --git a/x b/x\n");

        assert_eq!(view.subject.as_deref(), Some("Make it work"));
        assert!(view.git_format);
    }

    #[test]
    fn presents_a_summary_before_the_detail() {
        let data = serde_json::to_value(parse(
            "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1,2 @@\n a\n+b\n",
        ))
        .unwrap();

        let lines = DiffPresentation.present(&data);

        assert!(lines[0].starts_with("1 file(s) changed"));
        assert!(lines.iter().any(|line| line.contains("in 1 hunk(s)")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/diff/add-the-plugin.patch");

        let data = DiffCore.view(&path).unwrap();
        let view: DiffView = serde_json::from_value(data).unwrap();

        assert!(view.git_format);
        assert!(view.subject.is_some());
        assert!(view.files.len() >= 3);
        assert!(view.files.iter().any(|file| file.binary));
        assert!(view.files.iter().any(|file| file.renamed_from.is_some()));
        assert!(view.files.iter().any(|file| file.hunks.len() >= 2));
        assert!(view.added > 0 && view.removed > 0);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::DiffCore),
            plugin_api::PluginPresentation::extensions(&crate::DiffPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
