//! Ignore file file type plugin: core and presentation halves.
//!
//! Bare glob patterns with no marker of their own, so the sniff leans on
//! shape: comments, negations, directory-only entries and anchored paths,
//! and nothing that looks like an assignment or a command.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &[
    "gitignore",
    "dockerignore",
    "npmignore",
    "eslintignore",
    "prettierignore",
];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pattern {
    /// The pattern as written, without its `!`.
    pub glob: String,
    /// Whether it re-includes rather than excludes.
    pub negated: bool,
    /// Whether it ends in `/`, and so matches directories only.
    pub directory_only: bool,
    /// Whether it is anchored to this file's own directory.
    pub anchored: bool,
}

/// View data produced by [`IgnorefileCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnorefileView {
    /// Every pattern, in file order. Later ones win.
    pub patterns: Vec<Pattern>,
    /// The negations, which re-include something an earlier line excluded.
    pub negations: Vec<String>,
    /// How many comment lines the file carries.
    pub comments: usize,
    /// Negations that can never take effect, because a parent directory is
    /// excluded and the walk never descends into it to reconsider.
    pub unreachable_negations: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Everything [`IgnorefileView`] holds, read from `text`.
fn parse(text: &str) -> IgnorefileView {
    let mut view = IgnorefileView {
        patterns: Vec::new(),
        negations: Vec::new(),
        comments: 0,
        unreachable_negations: Vec::new(),
        content: String::new(),
        truncated: false,
    };
    let mut excluded_directories: Vec<String> = Vec::new();

    for raw in text.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            view.comments += 1;
            continue;
        }
        let negated = trimmed.starts_with('!');
        let glob = trimmed.strip_prefix('!').unwrap_or(trimmed).to_owned();
        let directory_only = glob.ends_with('/');
        let anchored = glob.trim_end_matches('/').contains('/');

        if negated {
            view.negations.push(glob.clone());
            // A negation cannot resurrect anything under a directory that
            // an earlier line excluded: the walk never enters it. This is
            // the single most common mistake in these files.
            if excluded_directories
                .iter()
                .any(|directory| glob.starts_with(directory.as_str()))
            {
                view.unreachable_negations.push(glob.clone());
            }
        } else if directory_only {
            excluded_directories.push(glob.clone());
        }

        view.patterns.push(Pattern {
            glob,
            negated,
            directory_only,
            anchored,
        });
    }
    view
}

/// Whether `text` looks like an ignore file.
fn looks_like_it(text: &str) -> bool {
    let mut patterns = 0usize;
    let mut shaped = false;
    for raw in text.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Anything that reads as an assignment, a command or markup is not
        // a glob, and this must not claim the file it came from.
        if trimmed.contains('=')
            || trimmed.contains(": ")
            || trimmed.contains('{')
            || trimmed.contains('(')
            || trimmed.ends_with(';')
            || trimmed.contains(' ')
        {
            return false;
        }
        patterns += 1;
        if trimmed.starts_with('!') || trimmed.ends_with('/') || trimmed.contains('*') {
            shaped = true;
        }
    }
    patterns >= 3 && shaped
}

/// The Ignore file plugin's core half.
#[derive(Debug, Default)]
pub struct IgnorefileCore;

impl PluginCore for IgnorefileCore {
    fn name(&self) -> &'static str {
        "ignorefile"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
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

/// The Ignore file plugin's presentation half.
#[derive(Debug, Default)]
pub struct IgnorefilePresentation;

impl PluginPresentation for IgnorefilePresentation {
    fn name(&self) -> &'static str {
        "ignorefile"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "IGN",
            tint: 0x00f0_5033,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: IgnorefileView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "{} pattern(s), later ones winning",
            view.patterns.len()
        ));
        for pattern in &view.patterns {
            let mut notes = Vec::new();
            if pattern.negated {
                notes.push("re-includes");
            }
            if pattern.directory_only {
                notes.push("directories only");
            }
            if pattern.anchored {
                notes.push("anchored");
            }
            let suffix = if notes.is_empty() {
                String::new()
            } else {
                format!("  ({})", notes.join(", "))
            };
            let mark = if pattern.negated { "!" } else { " " };
            lines.push(format!("  {mark}{}{suffix}", pattern.glob));
        }
        lines.push(format!("Comments: {}", view.comments));
        if !view.unreachable_negations.is_empty() {
            lines.push(
                "These re-inclusions can never take effect: an excluded directory is".to_owned(),
            );
            lines.push("never descended into, so nothing inside it is reconsidered.".to_owned());
            for glob in &view.unreachable_negations {
                lines.push(format!("  !{glob}"));
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
    use super::{IgnorefileCore, IgnorefilePresentation, IgnorefileView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_globs_negations_and_directory_entries() {
        assert!(IgnorefileCore.sniff(b"target/\n*.log\n!keep.log\n"));
        assert!(IgnorefileCore.sniff(b"# build output\nnode_modules/\ndist/\n*.tsbuildinfo\n"));
    }

    #[test]
    fn does_not_claim_a_configuration_file_or_a_script() {
        assert!(!IgnorefileCore.sniff(b"port=8080\nhost=localhost\ndebug=true\n"));
        assert!(!IgnorefileCore.sniff(b"set -eu\ncd /tmp\nrm -rf x\n"));
        assert!(
            !IgnorefileCore.sniff(b"a\nb\nc\n"),
            "plain words are not globs"
        );
        assert!(!IgnorefileCore.sniff(b""));
    }

    #[test]
    fn reads_the_shape_of_each_pattern() {
        let view = parse("target/\n*.log\n!keep.log\n/only/here\n");

        assert_eq!(view.patterns.len(), 4);
        assert!(view.patterns[0].directory_only);
        assert!(!view.patterns[1].anchored);
        assert!(view.patterns[2].negated);
        assert!(view.patterns[3].anchored);
        assert_eq!(view.negations, vec!["keep.log".to_owned()]);
    }

    #[test]
    fn a_negation_under_an_excluded_directory_is_reported_as_unreachable() {
        // The single most common mistake in these files: the walk never
        // enters an excluded directory, so nothing inside it is
        // reconsidered and the re-inclusion silently does nothing.
        let view = parse("build/\n!build/keep.txt\n");

        assert_eq!(
            view.unreachable_negations,
            vec!["build/keep.txt".to_owned()]
        );
    }

    #[test]
    fn a_negation_outside_any_excluded_directory_is_fine() {
        let view = parse("*.log\n!important.log\n");

        assert!(view.unreachable_negations.is_empty());
    }

    #[test]
    fn presents_the_unreachable_negations_with_the_reason() {
        let data = serde_json::to_value(parse("build/\n!build/keep.txt\n")).unwrap();

        let lines = IgnorefilePresentation.present(&data);

        assert!(lines.iter().any(|line| line.contains("never take effect")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/ignorefile/build.gitignore");

        let data = IgnorefileCore.view(&path).unwrap();
        let view: IgnorefileView = serde_json::from_value(data).unwrap();

        assert!(view.patterns.len() >= 8);
        assert!(!view.negations.is_empty());
        assert!(view.patterns.iter().any(|p| p.directory_only));
        assert!(view.patterns.iter().any(|p| p.anchored));
        assert!(view.comments >= 2);
        assert!(!view.unreachable_negations.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::IgnorefileCore),
            plugin_api::PluginPresentation::extensions(&crate::IgnorefilePresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
