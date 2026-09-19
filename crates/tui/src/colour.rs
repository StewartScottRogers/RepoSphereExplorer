//! Colouring the File pane's text the way its plugin classifies it (#644).
//!
//! `PluginPresentation::classify` returns spans over a plugin's own text;
//! this turns those spans into runs a line at a time, wraps them at the
//! pane's width without losing a run's class at the break, and gives each
//! [`Class`] one of the sixteen American National Standards Institute
//! (ANSI) colours a terminal is guaranteed to have (D16, GUIDANCE.md §2.2).
//! `coloured_lines` and `file_starts_in` are this front end's own copies of
//! the graphical front end's `colour_lines` and `file_starts_in_preview`
//! (`crates/gui/src/app.rs`) - the algorithm is a function of `&str` and
//! `Span` alone, and each front end already keeps its own small copies of
//! `present`/`facts`/`views`/`graphic` rather than sharing them.

use plugin_api::{Class, Span};
use ratatui::style::Color;

/// One run of a line: its text, with no newline in it, and what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) text: String,
    pub(crate) class: Class,
}

/// A line drawn as a single run in no class at all - what a line gets
/// when there is nothing to say about its syntax, so it reads exactly as
/// it did before this front end could colour anything.
pub(crate) fn plain_line(line: &str) -> Vec<Run> {
    vec![Run {
        text: line.to_owned(),
        class: Class::Plain,
    }]
}

/// A [`Class`] as one of the sixteen ANSI colours, chosen to echo the
/// graphical front end's own table (`crates/gui/ui/theme.slint`'s
/// `syntax-colour`) as closely as sixteen fixed colours allow: keyword is
/// blue, "the blue every editor uses"; type is cyan, the nearest ANSI
/// shade to that table's teal, near the keyword but not it; function is
/// yellow, warm, so a call stands out; a string (`Class::Text`) is red,
/// the nearest ANSI shade to that table's red-brown "oldest convention";
/// a number is green, quiet. That table spends two different greens on
/// number and comment, which sixteen named colours cannot; a comment
/// instead takes `DarkGray`, the dim, de-emphasised colour this front end
/// already draws a fact table's own dimmed row in (`app::facts_table`),
/// and punctuation - present but never the point - takes `Gray`, a step
/// brighter than a comment and a step dimmer than plain text.
///
/// `None` for `Plain`: there is nothing to say about it, so it is drawn in
/// the terminal's own foreground colour rather than one from this table.
pub(crate) const fn class_colour(class: Class) -> Option<Color> {
    match class {
        Class::Plain => None,
        Class::Keyword => Some(Color::Blue),
        Class::Type => Some(Color::Cyan),
        Class::Function => Some(Color::Yellow),
        Class::Text => Some(Color::Red),
        Class::Number => Some(Color::Green),
        Class::Comment => Some(Color::DarkGray),
        Class::Punctuation => Some(Color::Gray),
    }
}

/// Splits `text` into lines, each a list of [`Run`]s, using `spans`.
///
/// A span may cross a newline - a block comment usually does - so the
/// break is made here rather than asked of the classifier, which would
/// have to know how the pane lays text out to answer. The trailing
/// newline ends the last line rather than starting an empty one, so a
/// file ending in a newline does not draw a blank row that is not in it.
///
/// A span outside `text`'s bounds is skipped rather than trusted - the
/// same leniency the graphical front end's own copy has. What holds every
/// classifier to the contract instead is a test over `syntax::spans_cover`
/// and the whole sample catalogue (`tests/classification.rs`), the same
/// way the graphical front end's `classification.rs` does: a plugin that
/// breaks the contract fails that test, in both front ends, rather than
/// this drawing code guessing at a repair.
pub(crate) fn coloured_lines(text: &str, spans: &[Span]) -> Vec<Vec<Run>> {
    let mut lines: Vec<Vec<Run>> = Vec::new();
    let mut line: Vec<Run> = Vec::new();
    for span in spans {
        let Some(part) = text.get(span.start..span.start + span.len) else {
            continue;
        };
        let mut pieces = part.split('\n');
        if let Some(first) = pieces.next().filter(|first| !first.is_empty()) {
            line.push(Run {
                text: first.to_owned(),
                class: span.class,
            });
        }
        for piece in pieces {
            lines.push(std::mem::take(&mut line));
            if !piece.is_empty() {
                line.push(Run {
                    text: piece.to_owned(),
                    class: span.class,
                });
            }
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Where the file's own text begins inside `preview`'s lines, or `None`
/// when `preview` does not end with it.
///
/// Anchored at the end rather than searched for: a plugin's `present` is
/// its own summary followed by the file's content, so whatever else is
/// above, the file is the last lines - the graphical front end's own
/// `file_starts_in_preview` makes the same case at greater length.
pub(crate) fn file_starts_in(preview: &[String], text: &str) -> Option<usize> {
    let file: Vec<&str> = text.lines().collect();
    if file.is_empty() {
        return None;
    }
    let from = preview.len().checked_sub(file.len())?;
    (preview[from..] == file[..]).then_some(from)
}

/// Wraps one line's [`Run`]s at `width` characters, the same character
/// count `app::wrap_line` wraps a plain line at, carrying each
/// character's class across the break so a run split mid-word is still
/// that run's class on both halves.
///
/// Character count, not measured display width, matching `wrap_line`
/// exactly - the two have to agree on how many rows a line becomes, or a
/// scroll bound computed from one would not match what the other draws.
pub(crate) fn wrap_runs(line: &[Run], width: usize) -> Vec<Vec<Run>> {
    if width == 0 {
        return vec![line.to_vec()];
    }
    let chars: Vec<(char, Class)> = line
        .iter()
        .flat_map(|run| run.text.chars().map(|ch| (ch, run.class)))
        .collect();
    if chars.is_empty() {
        return vec![Vec::new()];
    }
    chars
        .chunks(width)
        .map(|chunk| {
            let mut runs: Vec<Run> = Vec::new();
            for &(ch, class) in chunk {
                match runs.last_mut() {
                    Some(run) if run.class == class => run.text.push(ch),
                    _ => runs.push(Run {
                        text: ch.to_string(),
                        class,
                    }),
                }
            }
            runs
        })
        .collect()
}

/// Whether the environment allows colour to be drawn at all, given
/// explicitly so a test can drive it without touching process-global
/// environment variables another test reads at the same time.
///
/// `NO_COLOR` (<https://no-color.org>, any value at all disables it) is
/// the first stop; D16 doubles it with a terminal that reports it
/// supports no colour, which `TERM=dumb` is the decades-old terminfo
/// entry for - no colour, no cursor movement, nothing but plain
/// characters - and every terminal that draws colour sets `TERM` to
/// something else.
pub(crate) fn colour_wanted(no_color: Option<&std::ffi::OsStr>, term: Option<&str>) -> bool {
    no_color.is_none() && term != Some("dumb")
}

/// [`colour_wanted`], read from this process's own environment.
pub(crate) fn terminal_colour_enabled() -> bool {
    colour_wanted(
        std::env::var_os("NO_COLOR").as_deref(),
        std::env::var("TERM").ok().as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        Class, Run, class_colour, colour_wanted, coloured_lines, file_starts_in, plain_line,
        wrap_runs,
    };
    use plugin_api::Span;

    #[test]
    fn every_class_but_plain_has_its_own_colour() {
        let classes = [
            Class::Keyword,
            Class::Type,
            Class::Function,
            Class::Text,
            Class::Number,
            Class::Comment,
            Class::Punctuation,
        ];
        let colours: Vec<_> = classes.iter().map(|class| class_colour(*class)).collect();
        assert!(colours.iter().all(Option::is_some));
        let mut unique = colours.clone();
        unique.sort_by_key(|colour| format!("{colour:?}"));
        unique.dedup();
        assert_eq!(
            unique.len(),
            colours.len(),
            "every class should read as its own colour: {colours:?}"
        );
        assert_eq!(class_colour(Class::Plain), None);
    }

    #[test]
    fn coloured_lines_splits_on_newlines_inside_a_span() {
        let text = "fn go() {\n// hi\n}";
        let spans = [
            Span::new(0, 9, Class::Keyword),
            Span::new(9, 1, Class::Plain),
            Span::new(10, 5, Class::Comment),
            Span::new(15, 1, Class::Plain),
            Span::new(16, 1, Class::Plain),
        ];
        let lines = coloured_lines(text, &spans);
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            vec![Run {
                text: "fn go() {".to_owned(),
                class: Class::Keyword
            }]
        );
        assert_eq!(
            lines[1],
            vec![Run {
                text: "// hi".to_owned(),
                class: Class::Comment
            }]
        );
    }

    #[test]
    fn a_span_outside_the_text_is_skipped_rather_than_trusted() {
        let text = "ab";
        let spans = [Span::new(0, 100, Class::Keyword)];
        assert_eq!(coloured_lines(text, &spans), Vec::<Vec<Run>>::new());
    }

    #[test]
    fn file_starts_in_finds_the_files_tail() {
        let preview = vec!["functions: go".to_owned(), "fn go() {}".to_owned()];
        assert_eq!(file_starts_in(&preview, "fn go() {}"), Some(1));
    }

    #[test]
    fn file_starts_in_is_none_when_the_preview_is_all_summary() {
        let preview = vec!["1 entry".to_owned()];
        assert_eq!(file_starts_in(&preview, "fn go() {}"), None);
    }

    #[test]
    fn wrap_runs_carries_a_class_across_the_break() {
        let line = vec![Run {
            text: "fnfn".to_owned(),
            class: Class::Keyword,
        }];
        let wrapped = wrap_runs(&line, 2);
        assert_eq!(
            wrapped,
            vec![
                vec![Run {
                    text: "fn".to_owned(),
                    class: Class::Keyword
                }],
                vec![Run {
                    text: "fn".to_owned(),
                    class: Class::Keyword
                }],
            ]
        );
    }

    #[test]
    fn wrap_runs_matches_plain_wrapping_in_row_count() {
        let plain = "abcdefgh";
        let line = plain_line(plain);
        let wrapped = wrap_runs(&line, 3);
        assert_eq!(wrapped.len(), plain.chars().count().div_ceil(3));
        let joined: String = wrapped
            .iter()
            .flat_map(|row| row.iter().map(|run| run.text.as_str()))
            .collect();
        assert_eq!(joined, plain);
    }

    #[test]
    fn wrap_runs_of_an_empty_line_is_one_empty_row() {
        assert_eq!(wrap_runs(&[], 10), vec![Vec::<Run>::new()]);
    }

    #[test]
    fn no_color_disables_colour_regardless_of_its_value() {
        assert!(!colour_wanted(
            Some(std::ffi::OsStr::new("")),
            Some("xterm")
        ));
        assert!(!colour_wanted(
            Some(std::ffi::OsStr::new("1")),
            Some("xterm")
        ));
    }

    #[test]
    fn a_dumb_terminal_disables_colour_even_without_no_color() {
        assert!(!colour_wanted(None, Some("dumb")));
    }

    #[test]
    fn an_ordinary_terminal_with_no_no_color_wants_colour() {
        assert!(colour_wanted(None, Some("xterm-256color")));
        assert!(colour_wanted(None, None));
    }
}
