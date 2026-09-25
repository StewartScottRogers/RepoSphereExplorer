//! Appends one run's measurements to the published development-statistics
//! history, run the way `pages.yml` runs it: gathering the raw numbers -
//! from `git`, the GitHub command-line tool `gh`, and the release's own
//! dist files - is the workflow's job, and everything here does is decide
//! what a missing one means and keep every past entry (GUIDANCE.md §4.4).
//!
//! Usage: `gather_stats <previous-history-json-path> <output-path>`
//!
//! `<previous-history-json-path>` is read as a [`updater::stats::StatsEntry`]
//! series - an empty one if the file is absent or is not valid JSON, which
//! is this repository's own first run - and one entry built from the
//! `STATS_*` environment variables below is appended before the result is
//! written to `<output-path>`.
//!
//! Every `STATS_*` variable but `STATS_GENERATED_AT` is optional: unset or
//! blank means this run could not gather that measure, recorded as a gap
//! rather than a zero.
//!
//! - `STATS_GENERATED_AT` - the date, `"YYYY-MM-DD"`.
//! - `STATS_COMMITS`, `STATS_CONTRIBUTORS`, `STATS_RELEASES`,
//!   `STATS_PLUGIN_COUNT`, `STATS_FORMATS_SUPPORTED`,
//!   `STATS_WORK_ORDERS_OPEN`, `STATS_WORK_ORDERS_CLOSED` - whole numbers.
//! - `STATS_RELEASE_CADENCE_DAYS`, `STATS_CI_PASS_RATE`,
//!   `STATS_CI_DURATION_SECONDS` - decimal numbers.
//! - `STATS_BINARY_SIZE_WINDOWS`, `STATS_BINARY_SIZE_MACOS`,
//!   `STATS_BINARY_SIZE_LINUX` - whole numbers of bytes, gathered
//!   separately per platform.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::Path;
use updater::stats::{StatsEntry, append, parse_history, parse_measure};

fn env_var(name: &str) -> Option<String> {
    env::var(name).ok()
}

fn measure<T: std::str::FromStr>(name: &str) -> Option<T> {
    parse_measure(env_var(name).as_deref())
}

fn binary_size_bytes() -> Option<BTreeMap<String, u64>> {
    let mut sizes = BTreeMap::new();
    for platform in ["windows", "macos", "linux"] {
        let variable = format!("STATS_BINARY_SIZE_{}", platform.to_uppercase());
        if let Some(size) = measure::<u64>(&variable) {
            sizes.insert(platform.to_owned(), size);
        }
    }
    if sizes.is_empty() { None } else { Some(sizes) }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let [previous_path, output_path] = args.as_slice() else {
        eprintln!("usage: gather_stats <previous-history-json-path> <output-path>");
        std::process::exit(2);
    };

    let previous = fs::read_to_string(previous_path).unwrap_or_default();
    let history = parse_history(&previous);

    let entry = StatsEntry {
        generated_at: env_var("STATS_GENERATED_AT").unwrap_or_default(),
        commits: measure("STATS_COMMITS"),
        contributors: measure("STATS_CONTRIBUTORS"),
        work_orders_open: measure("STATS_WORK_ORDERS_OPEN"),
        work_orders_closed: measure("STATS_WORK_ORDERS_CLOSED"),
        releases: measure("STATS_RELEASES"),
        release_cadence_days: measure("STATS_RELEASE_CADENCE_DAYS"),
        ci_pass_rate: measure("STATS_CI_PASS_RATE"),
        ci_duration_seconds: measure("STATS_CI_DURATION_SECONDS"),
        plugin_count: measure("STATS_PLUGIN_COUNT"),
        formats_supported: measure("STATS_FORMATS_SUPPORTED"),
        binary_size_bytes: binary_size_bytes(),
    };

    let updated = append(history, entry);
    let json = serde_json::to_string_pretty(&updated).expect("serialize stats history");
    if let Some(dir) = Path::new(output_path)
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
    {
        fs::create_dir_all(dir).unwrap_or_else(|err| panic!("create {}: {err}", dir.display()));
    }
    fs::write(output_path, json).unwrap_or_else(|err| panic!("write {output_path}: {err}"));
    println!("wrote {} entries to {output_path}", updated.len());
}
