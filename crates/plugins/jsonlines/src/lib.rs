//! JSON Lines file type plugin: core and presentation halves.
//!
//! One complete JavaScript Object Notation (JSON) value per line, with
//! no enclosing array - the shape a log or an export takes when it is
//! written a record at a time. This reads how many records there are,
//! the keys across them, the first few in full, the keys only some
//! records carry, and the lines that are no record at all.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["jsonl", "ndjson", "jsonlines"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`JsonlinesCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonlinesView {
    /// How many records were read.
    pub records: usize,
    /// Every key seen, with how many records carry it.
    pub keys: Vec<String>,
    /// The keys every record has.
    pub shared_keys: Vec<String>,
    /// Keys only some records carry, which is what makes a file of these
    /// awkward to load into anything expecting a table.
    pub ragged_keys: Vec<String>,
    /// The first few records, flattened to one line each.
    pub first_records: Vec<String>,
    /// Lines that are not a JSON value at all.
    pub unreadable_lines: Vec<usize>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`], in which case
    /// the counts describe what was read rather than what is there.
    pub truncated: bool,
}

/// How many records are shown in full.
const SHOWN: usize = 5;

/// Everything [`JsonlinesView`] holds, read from `text`.
fn parse(text: &str) -> JsonlinesView {
    let mut view = JsonlinesView {
        records: 0,
        keys: Vec::new(),
        shared_keys: Vec::new(),
        ragged_keys: Vec::new(),
        first_records: Vec::new(),
        unreadable_lines: Vec::new(),
        truncated: false,
    };
    // Each key with the number of records carrying it, in first-seen
    // order - which reads better than an alphabetical list.
    let mut counts: Vec<(String, usize)> = Vec::new();

    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            view.unreadable_lines.push(number + 1);
            continue;
        };
        view.records += 1;
        if view.first_records.len() < SHOWN {
            view.first_records.push(summarise(&value));
        }
        if let Some(object) = value.as_object() {
            for key in object.keys() {
                match counts.iter_mut().find(|(seen, _)| seen == key) {
                    Some((_, count)) => *count += 1,
                    None => counts.push((key.clone(), 1)),
                }
            }
        }
    }

    for (key, count) in &counts {
        view.keys.push(format!("{key} ({count})"));
        if *count == view.records {
            view.shared_keys.push(key.clone());
        } else {
            view.ragged_keys
                .push(format!("{key}, in {count} of {}", view.records));
        }
    }
    view
}

/// One record, flattened onto a line short enough to read.
fn summarise(value: &Value) -> String {
    let said = value.to_string();
    if said.chars().count() <= 120 {
        return said;
    }
    let cut: String = said.chars().take(117).collect();
    format!("{cut}...")
}

/// Whether `text` is JSON Lines.
fn looks_like_it(text: &str) -> bool {
    let mut records = 0usize;
    let mut broken = 0usize;
    for line in text.lines().take(50) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if serde_json::from_str::<Value>(line).is_ok() {
            records += 1;
        } else {
            broken += 1;
        }
    }
    // A single line is just JSON, and `json` should have it.
    //
    // A file with one line cut off is still this format - a writer killed
    // mid-record leaves exactly that, and reporting it is what
    // `unreadable_lines` is for. So one bad line is allowed, and one more
    // per twenty good ones; a file where most lines fail is something
    // else entirely.
    records >= 2 && broken <= 1 + records / 20
}

/// The JSON Lines plugin's core half.
#[derive(Debug, Default)]
pub struct JsonlinesCore;

impl PluginCore for JsonlinesCore {
    fn name(&self) -> &'static str {
        "jsonlines"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A file of these is not JSON - a strict reader rejects it at the
        // second line - but a single record is, and `json` will claim a
        // one-line file. Saying so settles which reading wins (D13).
        &["json"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        // A file of these is long by nature; the counts and the first
        // few records are what a reader came for.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The JSON Lines plugin's presentation half.
#[derive(Debug, Default)]
pub struct JsonlinesPresentation;

impl PluginPresentation for JsonlinesPresentation {
    fn name(&self) -> &'static str {
        "jsonlines"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "JSNL",
            tint: 0x0069_9b3f,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: JsonlinesView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "{} record(s){}",
            view.records,
            if view.truncated { ", so far" } else { "" }
        ));
        if !view.shared_keys.is_empty() {
            lines.push(format!("Every record has: {}", view.shared_keys.join(", ")));
        }
        if !view.first_records.is_empty() {
            lines.push("First records:".to_owned());
            for record in &view.first_records {
                lines.push(format!("  {record}"));
            }
        }
        if !view.keys.is_empty() {
            lines.push(format!("Keys seen: {}", view.keys.join(", ")));
        }
        if !view.ragged_keys.is_empty() {
            lines.push("Only some records carry these, so anything loading the".to_owned());
            lines.push("file as a table has to decide what to do about the rest:".to_owned());
            for key in &view.ragged_keys {
                lines.push(format!("  {key}"));
            }
        }
        if !view.unreadable_lines.is_empty() {
            lines.push("Not a JSON value, so these lines are no record at all:".to_owned());
            for number in &view.unreadable_lines {
                lines.push(format!("  line {number}"));
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
    use super::{JsonlinesCore, JsonlinesPresentation, JsonlinesView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const RECORDS: &str = concat!(
        r#"{"id":1,"name":"alpha","value":1.5}"#,
        "\n",
        r#"{"id":2,"name":"beta","value":2.5}"#,
        "\n",
        r#"{"id":3,"name":"gamma","value":3.5,"note":"an extra"}"#,
        "\n",
    );

    #[test]
    fn sniffs_a_file_of_records() {
        assert!(JsonlinesCore.sniff(RECORDS.as_bytes()));
    }

    #[test]
    fn does_not_claim_json_spread_over_several_lines() {
        assert!(!JsonlinesCore.sniff(b"{\n  \"id\": 1\n}\n"));
        assert!(
            !JsonlinesCore.sniff(br#"{"id":1}"#),
            "one line is just JSON, and `json` should have it"
        );
        assert!(!JsonlinesCore.sniff(b""));
    }

    #[test]
    fn a_record_cut_off_part_way_does_not_disqualify_the_file() {
        let interrupted = concat!(
            r#"{"id":1}"#,
            "
",
            r#"{"id":2}"#,
            "
",
            r#"{"id":3,"nam"#,
            "
",
            r#"{"id":4}"#,
            "
",
        );

        assert!(
            JsonlinesCore.sniff(interrupted.as_bytes()),
            "a writer killed mid-record leaves exactly this, and reporting it is the point"
        );
        assert!(
            !JsonlinesCore.sniff(
                b"one
two
three
four
"
            ),
            "a file where every line fails is something else entirely"
        );
    }

    #[test]
    fn it_says_it_specialises_json() {
        assert_eq!(JsonlinesCore.specialises(), &["json"]);
    }

    #[test]
    fn counts_the_records_and_the_keys() {
        let view = parse(RECORDS);

        assert_eq!(view.records, 3);
        assert_eq!(
            view.shared_keys,
            vec!["id".to_owned(), "name".to_owned(), "value".to_owned()]
        );
        assert_eq!(view.first_records.len(), 3);
    }

    #[test]
    fn names_the_key_only_some_records_carry() {
        let view = parse(RECORDS);

        assert_eq!(view.ragged_keys, vec!["note, in 1 of 3".to_owned()]);
    }

    #[test]
    fn a_line_that_is_not_a_value_is_reported_rather_than_counted() {
        let view = parse(concat!(
            r#"{"id":1}"#,
            "\nnot json at all\n",
            r#"{"id":2}"#,
            "\n",
        ));

        assert_eq!(view.records, 2);
        assert_eq!(view.unreadable_lines, vec![2]);
    }

    #[test]
    fn a_long_record_is_cut_rather_than_wrapped() {
        let long = format!(r#"{{"note":"{}"}}"#, "x".repeat(400));
        let view = parse(&format!("{long}\n{long}\n"));

        assert!(view.first_records[0].ends_with("..."));
        assert!(view.first_records[0].chars().count() <= 120);
    }

    #[test]
    fn presents_the_ragged_key_with_its_consequence() {
        let data = serde_json::to_value(parse(RECORDS)).unwrap();

        let lines = JsonlinesPresentation.present(&data);

        assert_eq!(lines[0], "3 record(s)");
        assert!(lines.iter().any(|line| line.contains("as a table")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/jsonlines/readings.jsonl");

        let data = JsonlinesCore.view(&path).unwrap();
        let view: JsonlinesView = serde_json::from_value(data).unwrap();

        assert!(view.records >= 200);
        assert!(view.keys.len() >= 4);
        assert!(!view.shared_keys.is_empty());
        assert!(!view.ragged_keys.is_empty());
        assert_eq!(view.first_records.len(), 5);
    }

    #[test]
    fn the_damaged_fixture_proves_the_unreadable_line() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/jsonlines/interrupted.jsonl");

        let data = JsonlinesCore.view(&path).unwrap();
        let view: JsonlinesView = serde_json::from_value(data).unwrap();

        assert!(!view.unreadable_lines.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::JsonlinesCore),
            plugin_api::PluginPresentation::extensions(&crate::JsonlinesPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
