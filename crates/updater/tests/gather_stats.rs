//! The `gather_stats` binary, run the way `pages.yml` runs it: a previous
//! history file (or none, for this repository's first run) plus one run's
//! `STATS_*` environment variables in, an appended history file out.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rse-gather-stats-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(previous: &Path, output: &Path, vars: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gather_stats"));
    command.env_clear().args([previous, output]);
    for (key, value) in vars {
        command.env(key, value);
    }
    command.output().unwrap()
}

#[test]
fn a_first_run_with_no_published_history_starts_a_series_of_one() {
    let dir = scratch("first-run");
    let previous = dir.join("history.json");
    let output = dir.join("out.json");

    let result = run(
        &previous,
        &output,
        &[
            ("STATS_GENERATED_AT", "2026-09-01"),
            ("STATS_COMMITS", "100"),
            ("STATS_CONTRIBUTORS", "5"),
        ],
    );

    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let history: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
    let entries = history.as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["generated_at"], "2026-09-01");
    assert_eq!(entries[0]["commits"], 100);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_second_run_appends_to_the_series_rather_than_replacing_it() {
    let dir = scratch("second-run");
    let first_output = dir.join("run1.json");
    let missing_previous = dir.join("no-such-file.json");

    let first = run(
        &missing_previous,
        &first_output,
        &[
            ("STATS_GENERATED_AT", "2026-09-01"),
            ("STATS_COMMITS", "100"),
        ],
    );
    assert!(first.status.success());

    let second_output = dir.join("run2.json");
    let second = run(
        &first_output,
        &second_output,
        &[
            ("STATS_GENERATED_AT", "2026-09-02"),
            ("STATS_COMMITS", "110"),
        ],
    );
    assert!(
        second.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );

    let history: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&second_output).unwrap()).unwrap();
    let entries = history.as_array().unwrap();

    assert_eq!(entries.len(), 2, "the first run's entry must survive");
    assert_eq!(entries[0]["generated_at"], "2026-09-01");
    assert_eq!(entries[0]["commits"], 100, "the past entry is unchanged");
    assert_eq!(entries[1]["generated_at"], "2026-09-02");
    assert_eq!(entries[1]["commits"], 110);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_measure_the_run_could_not_gather_is_a_gap_not_a_zero() {
    let dir = scratch("missing-measure");
    let previous = dir.join("history.json");
    let output = dir.join("out.json");

    // STATS_CI_PASS_RATE is deliberately left unset: this run could not
    // gather it, which must not be indistinguishable from a 0% pass rate.
    let result = run(
        &previous,
        &output,
        &[
            ("STATS_GENERATED_AT", "2026-09-01"),
            ("STATS_COMMITS", "100"),
        ],
    );
    assert!(result.status.success());

    let raw = std::fs::read_to_string(&output).unwrap();
    let history: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(
        history[0]["ci_pass_rate"].is_null(),
        "an ungathered measure must be null, not 0: {raw}"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn every_measure_guidance_names_is_present_in_the_emitted_entry() {
    let dir = scratch("all-measures-present");
    let previous = dir.join("history.json");
    let output = dir.join("out.json");

    let result = run(&previous, &output, &[("STATS_GENERATED_AT", "2026-09-01")]);
    assert!(result.status.success());

    let history: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
    let entry = history[0].as_object().unwrap();
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
        assert!(entry.contains_key(key), "missing {key} in {history}");
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn binary_sizes_are_gathered_per_platform() {
    let dir = scratch("binary-sizes");
    let previous = dir.join("history.json");
    let output = dir.join("out.json");

    let result = run(
        &previous,
        &output,
        &[
            ("STATS_GENERATED_AT", "2026-09-01"),
            ("STATS_BINARY_SIZE_WINDOWS", "1000"),
            ("STATS_BINARY_SIZE_LINUX", "900"),
        ],
    );
    assert!(result.status.success());

    let history: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
    let sizes: BTreeMap<String, u64> =
        serde_json::from_value(history[0]["binary_size_bytes"].clone()).unwrap();
    assert_eq!(sizes.get("windows"), Some(&1000));
    assert_eq!(sizes.get("linux"), Some(&900));
    assert_eq!(sizes.get("macos"), None, "an ungathered platform is absent");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_previous_history_file_that_is_not_valid_json_is_treated_as_a_first_run() {
    let dir = scratch("corrupt-previous");
    let previous = dir.join("history.json");
    std::fs::write(&previous, "<html>404</html>").unwrap();
    let output = dir.join("out.json");

    let result = run(&previous, &output, &[("STATS_GENERATED_AT", "2026-09-01")]);

    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let history: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
    assert_eq!(history.as_array().unwrap().len(), 1);

    std::fs::remove_dir_all(&dir).unwrap();
}
