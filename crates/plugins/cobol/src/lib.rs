//! COBOL file type plugin: core and presentation halves.
//!
//! A COBOL program is four divisions in a fixed order. This reads the
//! program identifier, the divisions and sections, the file control
//! entries, the working storage items with their levels and pictures,
//! the paragraphs of the procedure division - and the paragraphs nothing
//! performs, which run only if the one above them falls through.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["cbl", "cob", "cpy", "cobol"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One data item from a data division.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataItem {
    /// Its level number, which is how COBOL spells nesting.
    pub level: String,
    /// Its name, or `FILLER` when it has none.
    pub name: String,
    /// Its `PICTURE` clause, which is its type.
    pub picture: Option<String>,
    /// Its `VALUE`, when it is given one.
    pub value: Option<String>,
}

/// View data produced by [`CobolCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CobolView {
    /// The `PROGRAM-ID`.
    pub program: Option<String>,
    /// The divisions the program has, in order.
    pub divisions: Vec<String>,
    /// The sections, each with the division it is in.
    pub sections: Vec<String>,
    /// The files named by `SELECT`, with what each is assigned to.
    pub files: Vec<String>,
    /// The items of working storage, with their levels and pictures.
    pub working_storage: Vec<DataItem>,
    /// The paragraphs of the procedure division, in order.
    pub paragraphs: Vec<String>,
    /// Paragraphs nothing performs and no other paragraph falls into,
    /// which are reachable only by accident.
    pub never_performed: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Statements that are a single word, and so look on the page exactly
/// like a paragraph name written on a line of its own.
const ONE_WORD_STATEMENTS: &[&str] = &["EXIT", "CONTINUE", "GOBACK", "STOP"];

/// The four divisions, in the order COBOL requires them.
const DIVISIONS: &[&str] = &["IDENTIFICATION", "ENVIRONMENT", "DATA", "PROCEDURE"];

/// A source line with its sequence area and indicator handled.
///
/// Fixed-format COBOL puts a six-column sequence number first and an
/// indicator in column seven, where a `*` marks a comment. Free-format
/// does neither. Both are in use, so both are read.
fn source_of(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('*') || trimmed.starts_with('/') {
        return None;
    }
    // A fixed-format line is at least seven columns of margin; if the
    // first six are digits or blank and the seventh is an indicator,
    // treat it as fixed.
    if line.len() > 7 {
        let (area, rest) = line.split_at(7);
        let sequence = &area[..6];
        let indicator = area.as_bytes()[6];
        if sequence
            .chars()
            .all(|letter| letter.is_ascii_digit() || letter == ' ')
            && (indicator == b' ' || indicator == b'-')
        {
            return Some(rest);
        }
        if sequence.chars().all(|letter| letter.is_ascii_digit()) && indicator == b'*' {
            return None;
        }
    }
    Some(line)
}

/// The value of `keyword` on `line`, up to the full stop.
fn clause(line: &str, keyword: &str) -> Option<String> {
    let upper = line.to_ascii_uppercase();
    let at = upper.find(keyword)?;
    let rest = line[at + keyword.len()..].trim_start();
    let rest = rest.strip_prefix("IS ").unwrap_or(rest);
    // A quoted literal may hold a full stop of its own, and that one does
    // not end the statement: `ASSIGN TO "samples.dat"` is one value.
    if let Some(quote) = rest
        .chars()
        .next()
        .filter(|mark| *mark == '"' || *mark == '\'')
    {
        let end = rest[1..].find(quote)?;
        return Some(rest[..=end + 1].to_owned());
    }
    let end = rest.find('.').unwrap_or(rest.len());
    let value = rest[..end]
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim()
        .to_owned();
    (!value.is_empty()).then_some(value)
}

/// The data item `line` declares, if it declares one.
fn data_item(line: &str) -> Option<DataItem> {
    let trimmed = line.trim();
    let mut words = trimmed.split_whitespace();
    let level = words.next()?;
    // A level is one or two digits: 01 to 49, plus 66, 77 and 88.
    if level.len() > 2 || !level.chars().all(|letter| letter.is_ascii_digit()) {
        return None;
    }
    let name = words.next()?.trim_end_matches('.').to_owned();
    Some(DataItem {
        level: level.to_owned(),
        name,
        picture: clause(trimmed, "PICTURE ").or_else(|| clause(trimmed, "PIC ")),
        value: clause(trimmed, "VALUE "),
    })
}

/// Everything [`CobolView`] holds, read from `text`.
fn parse(text: &str) -> CobolView {
    let mut view = CobolView {
        program: None,
        divisions: Vec::new(),
        sections: Vec::new(),
        files: Vec::new(),
        working_storage: Vec::new(),
        paragraphs: Vec::new(),
        never_performed: Vec::new(),
        truncated: false,
    };
    let mut division = String::new();
    let mut section = String::new();
    let mut performed: Vec<String> = Vec::new();

    for raw in text.lines() {
        let Some(source) = source_of(raw) else {
            continue;
        };
        let line = source.trim();
        if line.is_empty() {
            continue;
        }
        let upper = line.to_ascii_uppercase();

        if let Some(name) = DIVISIONS
            .iter()
            .find(|name| upper.starts_with(&format!("{name} DIVISION")))
        {
            (*name).clone_into(&mut division);
            section.clear();
            view.divisions.push((*name).to_owned());
            continue;
        }
        if let Some(at) = upper.find(" SECTION") {
            line[..at].trim().clone_into(&mut section);
            view.sections.push(format!("{section} ({division})"));
            continue;
        }
        if upper.starts_with("PROGRAM-ID") {
            view.program = clause(line, "PROGRAM-ID.")
                .or_else(|| clause(line, "PROGRAM-ID"))
                .map(|name| name.trim_end_matches('.').to_owned());
            continue;
        }
        if upper.starts_with("SELECT ") {
            let name = line[7..].split_whitespace().next().unwrap_or("").to_owned();
            let assigned = clause(line, "ASSIGN TO ").unwrap_or_else(|| "unstated".to_owned());
            view.files.push(format!("{name} -> {assigned}"));
            continue;
        }
        if let Some(at) = upper.find("PERFORM ")
            && let Some(name) = line[at + "PERFORM ".len()..].split_whitespace().next()
        {
            performed.push(name.trim_end_matches('.').to_ascii_uppercase());
        }

        if division == "DATA" && section.eq_ignore_ascii_case("WORKING-STORAGE") {
            if let Some(item) = data_item(line) {
                view.working_storage.push(item);
            }
            continue;
        }
        if division == "PROCEDURE" && line.ends_with('.') && !line.contains(' ') && line.len() > 1 {
            // A paragraph name is a word alone on a line with a full stop -
            // and so, written out, is every scope terminator and a handful
            // of one-word statements. Reading those as paragraphs invented
            // five of the nine this repository's own fixture appeared to
            // have, and then reported two of them as unreachable. Found by
            // running the application, not by a test.
            let name = line.trim_end_matches('.');
            let upper = name.to_ascii_uppercase();
            if !upper.starts_with("END-") && !ONE_WORD_STATEMENTS.contains(&upper.as_str()) {
                view.paragraphs.push(name.to_owned());
            }
        }
    }

    // The first paragraph is entered by falling into it, so it is never
    // "never performed"; every other one has to be reached somehow.
    view.never_performed = view
        .paragraphs
        .iter()
        .skip(1)
        .filter(|name| !performed.contains(&name.to_ascii_uppercase()))
        .cloned()
        .collect();
    view
}

/// Whether `text` is COBOL.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    // The divisions are the give-away, and no other format in this
    // registry writes them. Two of them, and it is COBOL.
    view.divisions.len() >= 2 || (view.program.is_some() && !view.divisions.is_empty())
}

/// The COBOL plugin's core half.
#[derive(Debug, Default)]
pub struct CobolCore;

impl PluginCore for CobolCore {
    fn name(&self) -> &'static str {
        "cobol"
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
        // The divisions and their contents are the whole of the
        // structure, and each is on the view already.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The COBOL plugin's presentation half.
#[derive(Debug, Default)]
pub struct CobolPresentation;

impl PluginPresentation for CobolPresentation {
    fn name(&self) -> &'static str {
        "cobol"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "COB",
            tint: 0x0000_5c8a,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: CobolView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "PROGRAM-ID {}",
            view.program.as_deref().unwrap_or("(unstated)")
        ));
        if !view.divisions.is_empty() {
            lines.push(format!("Divisions: {}", view.divisions.join(", ")));
        }
        if !view.sections.is_empty() {
            lines.push(format!("Sections: {}", view.sections.join(", ")));
        }
        if !view.files.is_empty() {
            lines.push(format!("{} file(s):", view.files.len()));
            for file in &view.files {
                lines.push(format!("  {file}"));
            }
        }
        if !view.working_storage.is_empty() {
            lines.push(format!(
                "{} working storage item(s):",
                view.working_storage.len()
            ));
            for item in &view.working_storage {
                let picture = item
                    .picture
                    .as_ref()
                    .map_or_else(|| "a group".to_owned(), |said| format!("PIC {said}"));
                let value = item
                    .value
                    .as_ref()
                    .map_or_else(String::new, |said| format!(" = {said}"));
                lines.push(format!("  {} {} - {picture}{value}", item.level, item.name));
            }
        }
        if !view.paragraphs.is_empty() {
            lines.push(format!("{} paragraph(s):", view.paragraphs.len()));
            for name in &view.paragraphs {
                lines.push(format!("  {name}"));
            }
        }
        if !view.never_performed.is_empty() {
            lines.push("Nothing performs these, and the paragraph above each".to_owned());
            lines.push("would have to fall through for them to run at all:".to_owned());
            for name in &view.never_performed {
                lines.push(format!("  {name}"));
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
    use super::{CobolCore, CobolPresentation, CobolView, data_item, parse, source_of};
    use plugin_api::{PluginCore, PluginPresentation};

    const PROGRAM: &str = concat!(
        "       IDENTIFICATION DIVISION.\n",
        "       PROGRAM-ID. CSVSTATS.\n",
        "      * This line is a comment and holds no PROGRAM-ID.\n",
        "       ENVIRONMENT DIVISION.\n",
        "       INPUT-OUTPUT SECTION.\n",
        "       FILE-CONTROL.\n",
        "           SELECT SAMPLE-FILE ASSIGN TO \"samples.dat\"\n",
        "               ORGANIZATION IS LINE SEQUENTIAL.\n",
        "       DATA DIVISION.\n",
        "       WORKING-STORAGE SECTION.\n",
        "       01  WS-TOTALS.\n",
        "           05  WS-COUNT      PIC 9(4) VALUE ZERO.\n",
        "           05  WS-SUM        PIC S9(9)V99 VALUE ZERO.\n",
        "       77  WS-MEAN           PIC S9(5)V99.\n",
        "       PROCEDURE DIVISION.\n",
        "       MAIN-PARAGRAPH.\n",
        "           PERFORM READ-SAMPLES\n",
        "           PERFORM REPORT-RESULTS\n",
        "           STOP RUN.\n",
        "       READ-SAMPLES.\n",
        "           MOVE ZERO TO WS-COUNT.\n",
        "       REPORT-RESULTS.\n",
        "           DISPLAY WS-MEAN.\n",
        "       ORPHANED-PARAGRAPH.\n",
        "           DISPLAY \"nothing reaches this\".\n",
    );

    #[test]
    fn sniffs_a_program() {
        assert!(CobolCore.sniff(PROGRAM.as_bytes()));
    }

    #[test]
    fn does_not_claim_prose_that_uses_the_word_division() {
        assert!(!CobolCore.sniff(b"The data division met on Tuesday.\n"));
        assert!(!CobolCore.sniff(b""));
    }

    #[test]
    fn an_indicator_star_marks_a_comment() {
        assert_eq!(source_of("      * a comment"), None);
        assert!(source_of("       DISPLAY \"x\".").is_some());
        assert_eq!(
            parse(PROGRAM).program.as_deref(),
            Some("CSVSTATS"),
            "the comment also holds the words PROGRAM-ID and must not win"
        );
    }

    #[test]
    fn reads_the_divisions_and_sections_in_order() {
        let view = parse(PROGRAM);

        assert_eq!(
            view.divisions,
            vec![
                "IDENTIFICATION".to_owned(),
                "ENVIRONMENT".to_owned(),
                "DATA".to_owned(),
                "PROCEDURE".to_owned()
            ]
        );
        assert!(
            view.sections
                .iter()
                .any(|s| s.contains("WORKING-STORAGE (DATA)"))
        );
    }

    #[test]
    fn reads_the_file_control_entry() {
        let view = parse(PROGRAM);

        assert_eq!(
            view.files,
            vec!["SAMPLE-FILE -> \"samples.dat\"".to_owned()]
        );
    }

    #[test]
    fn a_level_number_is_one_or_two_digits() {
        assert_eq!(data_item("05  WS-COUNT PIC 9(4).").unwrap().level, "05");
        assert_eq!(
            data_item("77  WS-MEAN PIC S9(5)V99.").unwrap().name,
            "WS-MEAN"
        );
        assert!(
            data_item("12345 NOT-A-LEVEL PIC X.").is_none(),
            "a five digit word is a sequence number, not a level"
        );
    }

    #[test]
    fn reads_the_pictures_and_the_values() {
        let view = parse(PROGRAM);

        assert_eq!(view.working_storage.len(), 4);
        let group = &view.working_storage[0];
        assert_eq!(group.level, "01");
        assert!(
            group.picture.is_none(),
            "a group item has no picture of its own"
        );
        let count = &view.working_storage[1];
        assert_eq!(count.picture.as_deref(), Some("9(4)"));
        assert_eq!(count.value.as_deref(), Some("ZERO"));
    }

    #[test]
    fn a_scope_terminator_is_not_a_paragraph() {
        let view = parse(concat!(
            "       IDENTIFICATION DIVISION.\n",
            "       PROGRAM-ID. T.\n",
            "       PROCEDURE DIVISION.\n",
            "       MAIN-PARAGRAPH.\n",
            "           READ SAMPLE-FILE\n",
            "               AT END\n",
            "                   MOVE \"Y\" TO WS-EOF\n",
            "           END-READ.\n",
            "           IF WS-COUNT > ZERO\n",
            "               DISPLAY WS-COUNT\n",
            "           END-IF.\n",
            "           EXIT.\n",
        ));

        assert_eq!(
            view.paragraphs,
            vec!["MAIN-PARAGRAPH".to_owned()],
            "END-READ, END-IF and EXIT are statements on a line of their own"
        );
        assert!(
            view.never_performed.is_empty(),
            "and so none of them can be an unreachable paragraph either"
        );
    }

    #[test]
    fn names_the_paragraph_nothing_reaches() {
        let view = parse(PROGRAM);

        assert_eq!(view.paragraphs.len(), 4);
        assert_eq!(
            view.never_performed,
            vec!["ORPHANED-PARAGRAPH".to_owned()],
            "the first paragraph is entered by falling into it, so it is not orphaned"
        );
    }

    #[test]
    fn presents_the_orphan_with_its_reason() {
        let data = serde_json::to_value(parse(PROGRAM)).unwrap();

        let lines = CobolPresentation.present(&data);

        assert_eq!(lines[0], "PROGRAM-ID CSVSTATS");
        assert!(lines.iter().any(|line| line.contains("fall through")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/cobol/csvstats.cbl");

        let data = CobolCore.view(&path).unwrap();
        let view: CobolView = serde_json::from_value(data).unwrap();

        assert!(view.program.is_some());
        assert_eq!(view.divisions.len(), 4);
        assert!(view.sections.len() >= 2);
        assert!(!view.files.is_empty());
        assert!(view.working_storage.len() >= 6);
        assert!(view.working_storage.iter().any(|i| i.picture.is_some()));
        assert!(view.working_storage.iter().any(|i| i.value.is_some()));
        assert!(view.working_storage.iter().any(|i| i.picture.is_none()));
        assert!(view.paragraphs.len() >= 4);
        assert!(!view.never_performed.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::CobolCore),
            plugin_api::PluginPresentation::extensions(&crate::CobolPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
