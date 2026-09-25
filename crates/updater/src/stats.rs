//! The time series behind the site's "Project stats" trend charts
//! (GUIDANCE.md §4.4): each pipeline run appends one entry rather than
//! overwriting the last, so the history survives, and a measure a run
//! could not gather is recorded as a gap - `null` - never coerced to a
//! misleading `0`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One run's measurements. Every field but [`StatsEntry::generated_at`]
/// is optional: a run that could not gather one leaves it `None`, which
/// [`serde_json`] renders as `null`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct StatsEntry {
    /// The date this entry was generated, `"YYYY-MM-DD"`.
    pub generated_at: String,
    /// Total commits on the default branch.
    pub commits: Option<u64>,
    /// Distinct contributors across the default branch's history.
    pub contributors: Option<u64>,
    /// Open issues labelled `work-order`.
    pub work_orders_open: Option<u64>,
    /// Closed issues labelled `work-order`.
    pub work_orders_closed: Option<u64>,
    /// Published releases.
    pub releases: Option<u64>,
    /// Average days between the most recent releases.
    pub release_cadence_days: Option<f64>,
    /// Fraction, `0.0` to `1.0`, of recent required continuous
    /// integration (CI) runs on the default branch that succeeded.
    pub ci_pass_rate: Option<f64>,
    /// Average duration of recent CI runs, in seconds.
    pub ci_duration_seconds: Option<f64>,
    /// File-type plugin crates.
    pub plugin_count: Option<u64>,
    /// Formats those plugins recognise, per `PLUGINS.md`'s Built table -
    /// distinct from [`StatsEntry::plugin_count`] because a folder
    /// plugin's crate is not one of that table's rows.
    pub formats_supported: Option<u64>,
    /// Total published binary bytes, keyed by platform.
    pub binary_size_bytes: Option<BTreeMap<String, u64>>,
}

/// Parses `raw` as a measure: absent or blank means a gap - the run
/// could not gather it - and text that will not parse as `T` is treated
/// the same way, rather than failing the whole run over one bad number.
pub fn parse_measure<T: std::str::FromStr>(raw: Option<&str>) -> Option<T> {
    raw.map(str::trim)
        .filter(|text| !text.is_empty())
        .and_then(|text| text.parse().ok())
}

/// Parses `history_json` as a series of entries, treating anything that
/// will not parse - a first run with nothing published yet, or a
/// response body that is not JSON at all - as an empty series rather
/// than failing the run.
#[must_use]
pub fn parse_history(history_json: &str) -> Vec<StatsEntry> {
    serde_json::from_str(history_json).unwrap_or_default()
}

/// Appends `entry` to `history`: the whole of the append behaviour a
/// series needs to keep every past point rather than being overwritten
/// by the next run.
#[must_use]
pub fn append(mut history: Vec<StatsEntry>, entry: StatsEntry) -> Vec<StatsEntry> {
    history.push(entry);
    history
}

#[cfg(test)]
mod tests {
    use super::{StatsEntry, append, parse_history, parse_measure};

    fn entry(generated_at: &str, commits: Option<u64>) -> StatsEntry {
        StatsEntry {
            generated_at: generated_at.to_owned(),
            commits,
            ..StatsEntry::default()
        }
    }

    #[test]
    fn parse_measure_reads_a_present_value() {
        assert_eq!(parse_measure::<u64>(Some("42")), Some(42));
        assert_eq!(parse_measure::<f64>(Some("0.75")), Some(0.75));
    }

    #[test]
    fn parse_measure_is_a_gap_when_absent() {
        assert_eq!(parse_measure::<u64>(None), None);
    }

    #[test]
    fn parse_measure_is_a_gap_when_blank() {
        assert_eq!(parse_measure::<u64>(Some("")), None);
        assert_eq!(parse_measure::<u64>(Some("   ")), None);
    }

    #[test]
    fn parse_measure_is_a_gap_rather_than_an_error_when_unparsable() {
        // A malformed number from the gathering script must not fail the
        // whole run: it is recorded the same way an absent one is.
        assert_eq!(parse_measure::<u64>(Some("not-a-number")), None);
    }

    #[test]
    fn parse_history_is_empty_for_a_first_run_with_nothing_published_yet() {
        assert_eq!(parse_history(""), Vec::new());
    }

    #[test]
    fn parse_history_is_empty_for_a_body_that_is_not_json() {
        assert_eq!(parse_history("<html>not found</html>"), Vec::new());
    }

    #[test]
    fn parse_history_reads_back_what_was_written() {
        let entries = vec![entry("2026-09-01", Some(10)), entry("2026-09-02", Some(12))];
        let json = serde_json::to_string(&entries).unwrap();
        assert_eq!(parse_history(&json), entries);
    }

    #[test]
    fn append_keeps_every_past_entry_rather_than_replacing_them() {
        let first_run = append(Vec::new(), entry("2026-09-01", Some(10)));
        assert_eq!(first_run, vec![entry("2026-09-01", Some(10))]);

        let second_run = append(first_run.clone(), entry("2026-09-02", Some(12)));

        assert_eq!(second_run.len(), 2, "the first run's entry must survive");
        assert_eq!(second_run[0], first_run[0], "the past entry is unchanged");
        assert_eq!(second_run[1], entry("2026-09-02", Some(12)));
    }

    #[test]
    fn a_missing_measure_serializes_as_a_gap_not_a_zero() {
        let entry = entry("2026-09-01", None);
        let json = serde_json::to_string(&entry).unwrap();

        assert!(json.contains("\"commits\":null"), "{json}");
        assert!(!json.contains("\"commits\":0"), "{json}");
    }

    #[test]
    fn every_measure_named_by_guidance_is_present_in_the_emitted_entry() {
        let json = serde_json::to_string(&StatsEntry::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let object = value.as_object().unwrap();

        for key in [
            "commits",
            "contributors",
            "work_orders_open",
            "work_orders_closed",
            "releases",
            "release_cadence_days",
            "ci_pass_rate",
            "ci_duration_seconds",
            "plugin_count",
            "formats_supported",
            "binary_size_bytes",
        ] {
            assert!(object.contains_key(key), "missing {key} in {json}");
        }
    }
}
