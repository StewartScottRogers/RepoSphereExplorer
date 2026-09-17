//! A working copy's README: its title and opening excerpt (#584).
//!
//! Read only for a folder [`super::DirectoryCore::view`] has already found
//! to be a working copy, so an ordinary folder's own README - if it has
//! one - is left alone: D12 stacks facts about what a folder *is*, and an
//! arbitrary folder is not what a README describes.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Candidate README file names, in the precedence order a folder is
/// checked against, matched case-insensitively.
const CANDIDATES: &[&str] = &[
    "README.md",
    "README.markdown",
    "README.rst",
    "README.txt",
    "README",
];

/// Largest README this reads an excerpt from. A larger file still gets its
/// Open link; there is just no excerpt below it.
const MAX_README_BYTES: u64 = 64 * 1024;

/// How many lines of opening text the excerpt draws from at most, counting
/// only the lines it actually keeps - a run of blank lines or badges costs
/// nothing against this.
const MAX_EXCERPT_LINES: usize = 20;

/// A working copy's README: its name, and the opening read from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadmeExcerpt {
    /// The README's real file name, cased as it sits on disk - what the
    /// File pane's Open README link selects in Contents.
    pub name: String,
    /// The file's first heading, when it has one before its second.
    pub title: Option<String>,
    /// The opening paragraphs, in order, with heading markers, bold
    /// markers, link syntax and badge images already removed. Empty when
    /// the file was larger than [`MAX_README_BYTES`], unreadable, or
    /// genuinely has nothing before its second heading.
    pub excerpt: Vec<String>,
}

/// The folder at `path`'s README, when it holds one at its top level.
#[must_use]
pub fn find(path: &Path) -> Option<ReadmeExcerpt> {
    let entries: Vec<String> = std::fs::read_dir(path)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    let refs: Vec<&str> = entries.iter().map(String::as_str).collect();
    let name = find_readme(&refs)?.to_owned();

    // A README past the read cap, or one that cannot be read at all, still
    // names itself - the Open README link works either way - it just
    // carries no title or excerpt below it (issue requirement 5).
    let (title, excerpt) = match std::fs::metadata(path.join(&name)) {
        Ok(metadata) if metadata.len() <= MAX_README_BYTES => {
            match std::fs::read_to_string(path.join(&name)) {
                Ok(text) => excerpt_of(&text),
                Err(_) => (None, Vec::new()),
            }
        }
        _ => (None, Vec::new()),
    };

    Some(ReadmeExcerpt {
        name,
        title,
        excerpt,
    })
}

/// The first candidate name `entries` holds, matched case-insensitively in
/// [`CANDIDATES`]'s own precedence order.
fn find_readme<'a>(entries: &[&'a str]) -> Option<&'a str> {
    CANDIDATES.iter().find_map(|candidate| {
        entries
            .iter()
            .copied()
            .find(|entry| entry.eq_ignore_ascii_case(candidate))
    })
}

/// If `chars[start]` opens a `[...](...)`  link or image reference, the
/// index of its closing `]` and the index just past its closing `)`.
/// Bracket-depth tracked rather than pattern-matched, the same shape as
/// the Markdown plugin's own `links_in` scanner, since a link's text may
/// itself hold brackets.
fn bracket_paren(chars: &[char], start: usize) -> Option<(usize, usize)> {
    let mut depth = 0usize;
    let mut index = start;
    let mut close = None;
    while index < chars.len() {
        match chars[index] {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(index);
                    break;
                }
            }
            _ => {}
        }
        index += 1;
    }
    let close = close?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let mut end = close + 2;
    while end < chars.len() && chars[end] != ')' {
        end += 1;
    }
    if end >= chars.len() {
        return None;
    }
    Some((close, end + 1))
}

/// `line` with every `![alt](target)` image reference - a badge included -
/// removed entirely: an image's alt text is not prose a reader asked for.
fn strip_images(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '!'
            && chars.get(index + 1) == Some(&'[')
            && let Some((_, end)) = bracket_paren(&chars, index + 1)
        {
            index = end;
            continue;
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

/// `line` with every `[text](target)` link reduced to its bracketed text.
fn strip_links(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '['
            && let Some((close, end)) = bracket_paren(&chars, index)
        {
            out.extend(&chars[index + 1..close]);
            index = end;
            continue;
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

/// Whether `line`, once every image reference is removed, has nothing left:
/// a line that was only badges and whitespace, worth dropping from the
/// excerpt entirely rather than emitting as a blank.
fn is_badge_line(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && trimmed.contains("![") && strip_images(trimmed).trim().is_empty()
}

/// `line` as plain text: images and badges removed, links reduced to their
/// text, bold markers removed, and internal whitespace collapsed to single
/// spaces so a run of removed markup does not leave a gap behind.
fn strip_inline(line: &str) -> String {
    let without_images = strip_images(line);
    let without_links = strip_links(&without_images);
    let without_bold = without_links.replace("**", "").replace("__", "");
    without_bold
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The heading text of an ATX heading (`## Text`), if `line` is one, with
/// its own markup already stripped. Requires the space `CommonMark`
/// requires, which is what keeps a `#!/bin/sh` line from reading as one.
fn atx_heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let hashes = trimmed.len() - trimmed.trim_start_matches('#').len();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.starts_with(' ') {
        return None;
    }
    let text = rest.trim().trim_end_matches('#').trim();
    if text.is_empty() {
        return None;
    }
    Some(strip_inline(text))
}

/// Appends `paragraph` to `excerpt` and clears it, when there is anything
/// in it.
///
/// A blank line between two blank lines should not add an empty entry.
fn flush_paragraph(paragraph: &mut String, excerpt: &mut Vec<String>) {
    if !paragraph.is_empty() {
        excerpt.push(std::mem::take(paragraph));
    }
}

/// The title and excerpt read from `text`.
///
/// The title is the first ATX heading, however deep. Setext headings
/// (`Title\n=====`) are deliberately not read: the only job past the
/// title is knowing where the *next* heading is, and one marker kept the
/// search unambiguous.
///
/// The excerpt is every paragraph between that heading and the next one
/// (or the end of the file, when there is no second heading), each
/// wrapped source line rejoined with a space so a hard-wrapped paragraph
/// reads as one sentence rather than several short ones. Counts only
/// against [`MAX_EXCERPT_LINES`] the source lines it actually keeps: a
/// badge line or a blank one costs nothing against the cap.
fn excerpt_of(text: &str) -> (Option<String>, Vec<String>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut title = None;
    let mut start = 0;
    for (index, line) in lines.iter().enumerate() {
        if let Some(heading) = atx_heading(line) {
            title = Some(heading);
            start = index + 1;
            break;
        }
    }

    let mut excerpt = Vec::new();
    let mut paragraph = String::new();
    let mut kept = 0usize;
    for line in &lines[start..] {
        if kept >= MAX_EXCERPT_LINES || atx_heading(line).is_some() {
            break;
        }
        if is_badge_line(line) {
            continue;
        }
        let stripped = strip_inline(line);
        if stripped.is_empty() {
            flush_paragraph(&mut paragraph, &mut excerpt);
            continue;
        }
        if !paragraph.is_empty() {
            paragraph.push(' ');
        }
        paragraph.push_str(&stripped);
        kept += 1;
    }
    flush_paragraph(&mut paragraph, &mut excerpt);
    (title, excerpt)
}

#[cfg(test)]
mod tests {
    use super::{
        CANDIDATES, MAX_EXCERPT_LINES, excerpt_of, find, find_readme, is_badge_line, strip_inline,
    };
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};

    fn unique_temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-directory-readme-{name}-{}",
            std::process::id()
        ))
    }

    // ---- finding the README (issue requirement 1) ----------------------

    #[test]
    fn finds_readme_md_first_when_more_than_one_candidate_is_present() {
        assert_eq!(
            find_readme(&["README.txt", "README.md", "src"]),
            Some("README.md")
        );
    }

    #[test]
    fn matches_case_insensitively() {
        assert_eq!(find_readme(&["readme.MD"]), Some("readme.MD"));
        assert_eq!(find_readme(&["Readme"]), Some("Readme"));
    }

    #[test]
    fn follows_the_stated_precedence_order() {
        assert_eq!(
            find_readme(&["README", "README.txt", "README.rst"]),
            Some("README.rst")
        );
        assert_eq!(find_readme(&["README", "README.txt"]), Some("README.txt"));
    }

    #[test]
    fn no_candidate_name_finds_nothing() {
        assert_eq!(find_readme(&["src", "Cargo.toml"]), None);
    }

    #[test]
    fn every_candidate_is_reachable() {
        for candidate in CANDIDATES {
            assert_eq!(find_readme(&[candidate]), Some(*candidate));
        }
    }

    // ---- the excerpt: title, second heading, line cap (requirement 2) --

    #[test]
    fn reads_the_first_heading_as_the_title() {
        let (title, _) = excerpt_of("# My Project\n\nAn opening line.\n");
        assert_eq!(title.as_deref(), Some("My Project"));
    }

    #[test]
    fn the_excerpt_stops_at_the_second_heading() {
        let (_, excerpt) = excerpt_of("# Title\n\nKept.\n\n## Next\n\nNot kept.\n");
        assert_eq!(excerpt, vec!["Kept.".to_owned()]);
    }

    #[test]
    fn the_excerpt_stops_at_the_line_cap() {
        let mut body = "# Title\n\n".to_owned();
        for index in 0..MAX_EXCERPT_LINES + 5 {
            writeln!(body, "line {index}").unwrap();
        }
        let (_, excerpt) = excerpt_of(&body);

        // Every source line here is `line <N>`, two tokens each, so the
        // numeric tokens kept name exactly which source lines survived.
        let kept: Vec<usize> = excerpt
            .join(" ")
            .split_whitespace()
            .filter_map(|token| token.parse().ok())
            .collect();
        assert_eq!(kept, (0..MAX_EXCERPT_LINES).collect::<Vec<_>>());
    }

    #[test]
    fn a_hard_wrapped_paragraph_rejoins_into_one_line() {
        let (_, excerpt) = excerpt_of("# Title\n\nFirst half\nsecond half.\n\nA new paragraph.\n");
        assert_eq!(
            excerpt,
            vec![
                "First half second half.".to_owned(),
                "A new paragraph.".to_owned(),
            ]
        );
    }

    #[test]
    fn no_heading_at_all_still_gives_an_excerpt_from_the_start() {
        let (title, excerpt) = excerpt_of("Just an opening line, no heading above it.\n");
        assert_eq!(title, None);
        assert_eq!(
            excerpt,
            vec!["Just an opening line, no heading above it.".to_owned()]
        );
    }

    // ---- markup stripped (requirement 2) --------------------------------

    #[test]
    fn strips_bold_markers() {
        assert_eq!(strip_inline("this is **bold** text"), "this is bold text");
        assert_eq!(strip_inline("this is __bold__ text"), "this is bold text");
    }

    #[test]
    fn reduces_a_link_to_its_text() {
        assert_eq!(
            strip_inline("see [the guide](guide.md) for more"),
            "see the guide for more"
        );
    }

    #[test]
    fn a_badge_line_is_dropped_entirely() {
        assert!(is_badge_line(
            "![build status](https://ci.example/badge.svg)"
        ));
        assert!(is_badge_line("![a](a.svg) ![b](b.svg)  "));
        assert!(!is_badge_line("some text ![inline](x.png) and more"));
        assert!(!is_badge_line(""));
    }

    #[test]
    fn badge_lines_never_reach_the_excerpt() {
        let (_, excerpt) =
            excerpt_of("# Title\n\n![build](https://ci.example/badge.svg)\n\nReal opening text.\n");
        assert_eq!(excerpt, vec!["Real opening text.".to_owned()]);
    }

    #[test]
    fn an_inline_image_is_removed_but_surrounding_text_survives() {
        let (_, excerpt) = excerpt_of("# Title\n\nSee ![a diagram](diagram.png) above.\n");
        assert_eq!(excerpt, vec!["See above.".to_owned()]);
    }

    // ---- a real, already-committed fixture (issue's samples/ check) ----

    #[test]
    fn strips_a_real_badge_and_link_from_the_markdown_fixture() {
        // `samples/markdown/guide.md` is a real, already-committed file
        // carrying a genuine badge-shaped image reference and an inline
        // link - this module's own doc comment names the reason this
        // stands in for a dedicated `samples/` fixture: recognising a
        // working copy needs a real `.git` entry, and `git add` refuses
        // to track a path with that name.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/markdown/guide.md");
        let text = std::fs::read_to_string(&path).expect("the markdown fixture should be readable");

        let badge_line = text
            .lines()
            .find(|line| line.trim_start().starts_with("!["))
            .expect("the fixture should carry a badge-shaped image line");
        assert!(is_badge_line(badge_line));

        let link_line = text
            .lines()
            .find(|line| line.contains("[the guidance]"))
            .expect("the fixture should carry a real inline link");
        let stripped = strip_inline(link_line);
        assert!(!stripped.contains('['), "{stripped}");
        assert!(!stripped.contains(']'), "{stripped}");
        assert!(stripped.contains("the guidance"), "{stripped}");
    }

    // ---- find() end to end ----------------------------------------------

    #[test]
    fn finds_a_readme_and_reads_its_opening() {
        let dir = unique_temp_dir("find");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("README.md"), "# ringbuffer\n\nA bounded queue.\n").unwrap();

        let found = find(&dir).unwrap();

        assert_eq!(found.name, "README.md");
        assert_eq!(found.title.as_deref(), Some("ringbuffer"));
        assert_eq!(found.excerpt, vec!["A bounded queue.".to_owned()]);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_oversized_readme_still_names_itself_with_no_excerpt() {
        let dir = unique_temp_dir("oversized");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("README.md"),
            "#".repeat(usize::try_from(super::MAX_README_BYTES).unwrap() + 1),
        )
        .unwrap();

        let found = find(&dir).unwrap();

        assert_eq!(found.name, "README.md");
        assert_eq!(found.title, None);
        assert!(found.excerpt.is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_folder_with_no_readme_is_none() {
        let dir = unique_temp_dir("no-readme");
        std::fs::create_dir_all(&dir).unwrap();

        assert!(find(&dir).is_none());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
