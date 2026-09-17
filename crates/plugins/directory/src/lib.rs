//! Directory-as-file type plugin: core and presentation halves.
//!
//! Unlike the other plugins, this one is never reached by content-based
//! sniffing (a directory has no bytes to read a prefix from). `service`
//! special-cases directories and dispatches to it directly by name before
//! attempting `sniff`; [`DirectoryCore::sniff`] always returns `false` and
//! exists only to satisfy the trait.

pub mod repository;
pub mod status;
pub mod tracking;

use plugin_api::{Fact, Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;
use std::time::SystemTime;

/// How far the age of a fetch has to be before it is too old to read the
/// comparison measured against it as current (#576).
const STALE_FETCH: std::time::Duration = std::time::Duration::from_hours(30 * 24);

/// How a branch stands against its upstream: `up to date with origin/main`,
/// `2 ahead, 5 behind origin/main`, `no upstream`.
///
/// Carries no word of when this was measured - that is [`tracking_summary`]
/// and the File pane's own "Last fetched" row's job - because nothing here
/// fetches, and a bare comparison is only ever true as of some fetch.
#[must_use]
pub fn tracking_comparison(tracking: &tracking::Tracking) -> String {
    use tracking::{Count, Upstream};

    let count = |count: &Count| match count {
        Count::Exact(n) => n.to_string(),
        Count::AtLeast(n) => format!("{n}+"),
    };
    match &tracking.upstream {
        Upstream::None => "no upstream".to_owned(),
        Upstream::NotFetched { name } => format!("{name} not fetched yet"),
        Upstream::Unreadable { name } => format!("cannot compare with {name}"),
        Upstream::Compared {
            name,
            ahead,
            behind,
        } => match (ahead, behind) {
            (Count::Exact(0), Count::Exact(0)) => format!("up to date with {name}"),
            (ahead, Count::Exact(0)) => format!("{} ahead of {name}", count(ahead)),
            (Count::Exact(0), behind) => format!("{} behind {name}", count(behind)),
            (ahead, behind) => format!("{} ahead, {} behind {name}", count(ahead), count(behind)),
        },
    }
}

/// How a branch stands against its upstream, for the Branch line:
/// `2 ahead, 5 behind origin/main (as of last fetch, 3 days ago)`.
///
/// Every count carries how old the fetch it was measured against is,
/// because nothing here fetches: "up to date" three weeks after the last
/// fetch is true about three weeks ago. `now` is a parameter so a test can
/// fix the clock.
#[must_use]
pub fn tracking_summary(tracking: &tracking::Tracking, now: SystemTime) -> String {
    let fetched = match tracking.last_fetch {
        Some(at) => format!(
            "as of last fetch, {}",
            age(now.duration_since(at).unwrap_or_default())
        ),
        None => "never fetched".to_owned(),
    };
    let comparison = tracking_comparison(tracking);
    if matches!(tracking.upstream, tracking::Upstream::Compared { .. }) {
        format!("{comparison} ({fetched})")
    } else {
        comparison
    }
}

/// A duration as a reader says it: `just now`, `5 minutes ago`,
/// `1 hour ago`, `3 days ago`.
fn age(elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs();
    let (amount, unit) = match seconds {
        0..60 => return "just now".to_owned(),
        60..3_600 => (seconds / 60, "minute"),
        3_600..86_400 => (seconds / 3_600, "hour"),
        _ => (seconds / 86_400, "day"),
    };
    let plural = if amount == 1 { "" } else { "s" };
    format!("{amount} {unit}{plural} ago")
}

/// View data produced by [`DirectoryCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryView {
    /// Number of immediate entries in the directory.
    pub entry_count: u64,
    /// Combined size in bytes of immediate entries whose size is known
    /// (subdirectories are not recursed into).
    pub total_size: u64,
    /// What this directory is as a source control working copy, or `None`
    /// when it is an ordinary folder - which is not a lesser thing, just a
    /// different one (GUIDANCE.md 2.5).
    #[serde(default)]
    pub repository: Option<repository::Repository>,
}

/// The directory-as-file plugin's core half.
#[derive(Debug, Default)]
pub struct DirectoryCore;

impl PluginCore for DirectoryCore {
    fn name(&self) -> &'static str {
        "directory"
    }

    fn sniff(&self, _prefix: &[u8]) -> bool {
        false
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let mut entry_count = 0u64;
        let mut total_size = 0u64;
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            entry_count += 1;
            // Only files, not subdirectories: a directory's own metadata
            // size is a filesystem-block-size artifact (e.g. ~4096 bytes on
            // Linux ext4, but 0 on Windows NTFS), not meaningful content
            // size, and summing it would make this platform-dependent.
            if entry.file_type().is_ok_and(|file_type| file_type.is_file())
                && let Ok(metadata) = entry.metadata()
            {
                total_size += metadata.len();
            }
        }
        let view = DirectoryView {
            entry_count,
            total_size,
            // The full description, status included: `view` runs for the
            // one directory a reader selected, which is the moment the
            // extra pass over its tracked files is worth making.
            repository: repository::describe_with_status(path),
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// `"entry"` or `"entries"`, so a count of one does not read as "1 entries".
fn entries_noun(count: u64) -> &'static str {
    if count == 1 { "entry" } else { "entries" }
}

/// A byte count the way the Contents pane's Size column formats one, e.g.
/// `17.0 KB` rather than `17357 bytes total` (#576).
#[allow(clippy::cast_precision_loss)]
fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = "B";
    for candidate in UNITS {
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
        unit = candidate;
    }
    if unit == "B" {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {unit}")
    }
}

/// The directory-as-file plugin's presentation half.
#[derive(Debug, Default)]
pub struct DirectoryPresentation;

impl PluginPresentation for DirectoryPresentation {
    fn name(&self) -> &'static str {
        "directory"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "DIR",
            tint: 0x00dc_b67a,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        &[]
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view = match serde_json::from_value::<DirectoryView>(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };

        let mut lines = Vec::new();
        // A working copy leads with what it is: which provider it came from
        // and what is checked out. The folder facts follow, because they are
        // the less interesting half for a repository.
        if let Some(repository) = &view.repository {
            lines.push("Source control working copy".to_owned());
            if let Some(provider) = &repository.provider {
                lines.push(format!("Provider: {provider}"));
            }
            if let Some(branch) = &repository.branch {
                lines.push(match &repository.tracking {
                    Some(tracking) => {
                        format!(
                            "Branch: {branch} - {}",
                            tracking_summary(tracking, SystemTime::now())
                        )
                    }
                    None => format!("Branch: {branch}"),
                });
            } else {
                lines.push("Branch: none checked out (detached head)".to_owned());
            }
            if let Some(remote) = &repository.remote {
                lines.push(format!("Remote: {remote}"));
            } else {
                lines.push("Remote: none configured".to_owned());
            }
            // Never the bare word "clean": untracked files are not counted,
            // and a checkout full of new files should not wear it.
            match &repository.status {
                Some(status) => lines.push(format!("Working tree: {}", status.summary())),
                None => lines.push("Working tree: could not read the index".to_owned()),
            }
            lines.push(String::new());
        }

        lines.push(format!(
            "{} {}",
            view.entry_count,
            entries_noun(view.entry_count)
        ));
        lines.push(format!("{} bytes total", view.total_size));
        lines
    }

    fn facts(&self, data: &serde_json::Value) -> Vec<Fact> {
        let Ok(view) = serde_json::from_value::<DirectoryView>(data.clone()) else {
            return Vec::new();
        };

        let mut facts = Vec::new();
        if let Some(repository) = &view.repository {
            if let Some(provider) = &repository.provider {
                facts.push(Fact::new("Provider", provider));
            }
            match &repository.branch {
                Some(branch) => facts.push(Fact::new("Branch", branch)),
                None => facts.push(Fact::new("Branch", "none checked out (detached head)")),
            }
            if let Some(tracking) = &repository.tracking {
                facts.push(Fact::new("Tracking", tracking_comparison(tracking)));
                let (value, stale) = match tracking.last_fetch {
                    Some(at) => {
                        let elapsed = SystemTime::now().duration_since(at).unwrap_or_default();
                        (age(elapsed), elapsed > STALE_FETCH)
                    }
                    None => ("never".to_owned(), true),
                };
                facts.push(Fact {
                    label: "Last fetched".to_owned(),
                    value,
                    dim: stale,
                });
            }
            facts.push(Fact::new(
                "Remote",
                repository.remote.as_deref().unwrap_or("none configured"),
            ));
            facts.push(Fact::new(
                "Working tree",
                repository.status.as_ref().map_or_else(
                    || "could not read the index".to_owned(),
                    status::WorkingTree::summary,
                ),
            ));
            // A blank row, separating the working copy's own facts from
            // the folder facts that follow - not repository facts, and so
            // drawn dim rather than sharing the table's full weight.
            facts.push(Fact::new("", ""));
        }
        facts.push(Fact {
            label: "Entries".to_owned(),
            value: view.entry_count.to_string(),
            dim: true,
        });
        facts.push(Fact {
            label: "Total size".to_owned(),
            value: format_size(view.total_size),
            dim: true,
        });
        facts
    }
}

#[cfg(test)]
mod tests {
    use super::{DirectoryCore, DirectoryPresentation, DirectoryView};
    use plugin_api::{Fact, PluginCore, PluginPresentation};

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rse-plugin-dir-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn sniff_always_returns_false() {
        assert!(!DirectoryCore.sniff(b""));
        assert!(!DirectoryCore.sniff(b"anything"));
    }

    #[test]
    fn views_a_real_directory() {
        let dir = unique_temp_dir("view");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"12345").unwrap();
        std::fs::write(dir.join("b.txt"), b"1234567890").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();

        let data = DirectoryCore.view(&dir).unwrap();
        let view: DirectoryView = serde_json::from_value(data).unwrap();

        assert_eq!(view.entry_count, 3);
        assert_eq!(view.total_size, 15);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn presents_entry_count_and_total_size() {
        let data = serde_json::to_value(DirectoryView {
            entry_count: 4,
            total_size: 1024,
            repository: None,
        })
        .unwrap();

        let lines = DirectoryPresentation.present(&data);

        assert_eq!(lines, vec!["4 entries", "1024 bytes total"]);
    }

    #[test]
    fn presents_a_single_entry_in_the_singular() {
        let view = DirectoryView {
            entry_count: 1,
            total_size: 10,
            repository: None,
        };
        let data = serde_json::to_value(view).unwrap();

        let lines = DirectoryPresentation.present(&data);

        assert_eq!(lines, vec!["1 entry", "10 bytes total"]);
    }

    #[test]
    fn a_working_copy_leads_with_where_it_came_from() {
        let data = serde_json::to_value(DirectoryView {
            entry_count: 12,
            total_size: 4096,
            repository: Some(super::repository::Repository {
                provider: Some("github.com".to_owned()),
                branch: Some("main".to_owned()),
                remote: Some("https://github.com/owner/name.git".to_owned()),
                tracking: None,
                status: None,
            }),
        })
        .unwrap();

        let lines = DirectoryPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "Source control working copy",
                "Provider: github.com",
                "Branch: main",
                "Remote: https://github.com/owner/name.git",
                "Working tree: could not read the index",
                "",
                "12 entries",
                "4096 bytes total",
            ]
        );
    }

    #[test]
    fn a_working_copy_says_so_even_when_it_has_no_remote_or_branch() {
        let data = serde_json::to_value(DirectoryView {
            entry_count: 3,
            total_size: 90,
            repository: Some(super::repository::Repository::default()),
        })
        .unwrap();

        let lines = DirectoryPresentation.present(&data);

        assert!(lines.contains(&"Source control working copy".to_owned()));
        assert!(lines.contains(&"Branch: none checked out (detached head)".to_owned()));
        assert!(lines.contains(&"Remote: none configured".to_owned()));
    }

    #[test]
    fn an_ordinary_folder_says_nothing_about_source_control() {
        let data = serde_json::to_value(DirectoryView {
            entry_count: 2,
            total_size: 15,
            repository: None,
        })
        .unwrap();

        let lines = DirectoryPresentation.present(&data);

        assert_eq!(lines, vec!["2 entries", "15 bytes total"]);
    }

    #[test]
    fn a_working_copy_reports_what_its_tracked_files_look_like() {
        let data = serde_json::to_value(DirectoryView {
            entry_count: 12,
            total_size: 4096,
            repository: Some(super::repository::Repository {
                provider: Some("github.com".to_owned()),
                branch: Some("main".to_owned()),
                remote: None,
                tracking: None,
                status: Some(super::status::WorkingTree {
                    changed: 2,
                    examined: 130,
                    partial: false,
                }),
            }),
        })
        .unwrap();

        let lines = DirectoryPresentation.present(&data);

        assert!(
            lines.contains(&"Working tree: 2 tracked files changed".to_owned()),
            "{lines:?}"
        );
    }

    #[test]
    fn a_clean_working_copy_never_reads_as_simply_clean() {
        let data = serde_json::to_value(DirectoryView {
            entry_count: 3,
            total_size: 90,
            repository: Some(super::repository::Repository {
                provider: None,
                branch: None,
                remote: None,
                tracking: None,
                status: Some(super::status::WorkingTree {
                    changed: 0,
                    examined: 40,
                    partial: false,
                }),
            }),
        })
        .unwrap();

        let lines = DirectoryPresentation.present(&data);

        // Untracked files are not counted, so the word "clean" would be a
        // claim this cannot make.
        assert!(
            lines.contains(&"Working tree: no uncommitted changes to tracked files".to_owned()),
            "{lines:?}"
        );
    }

    // ---- how the branch stands against its upstream (#537) -------------

    fn tracked(upstream: super::tracking::Upstream, fetched_ago: Option<u64>) -> String {
        let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        super::tracking_summary(
            &super::tracking::Tracking {
                branch: "main".to_owned(),
                upstream,
                last_fetch: fetched_ago.map(|ago| now - std::time::Duration::from_secs(ago)),
            },
            now,
        )
    }

    fn compared(
        ahead: super::tracking::Count,
        behind: super::tracking::Count,
    ) -> super::tracking::Upstream {
        super::tracking::Upstream::Compared {
            name: "origin/main".to_owned(),
            ahead,
            behind,
        }
    }

    #[test]
    fn each_tracking_state_reads_as_the_work_order_says() {
        use super::tracking::{Count::AtLeast, Count::Exact, Upstream};
        const DAY: u64 = 86_400;

        assert_eq!(
            tracked(compared(Exact(0), Exact(0)), Some(2 * 3_600)),
            "up to date with origin/main (as of last fetch, 2 hours ago)"
        );
        assert_eq!(
            tracked(compared(Exact(2), Exact(5)), Some(3 * DAY)),
            "2 ahead, 5 behind origin/main (as of last fetch, 3 days ago)"
        );
        assert_eq!(
            tracked(compared(Exact(1), Exact(0)), Some(60)),
            "1 ahead of origin/main (as of last fetch, 1 minute ago)"
        );
        assert_eq!(
            tracked(compared(Exact(0), Exact(7)), Some(5)),
            "7 behind origin/main (as of last fetch, just now)"
        );
        assert_eq!(
            tracked(compared(AtLeast(10_000), AtLeast(3)), None),
            "10000+ ahead, 3+ behind origin/main (never fetched)"
        );
        assert_eq!(tracked(Upstream::None, None), "no upstream");
        assert_eq!(
            tracked(
                Upstream::NotFetched {
                    name: "origin/main".to_owned()
                },
                None
            ),
            "origin/main not fetched yet"
        );
        assert_eq!(
            tracked(
                Upstream::Unreadable {
                    name: "origin/main".to_owned()
                },
                Some(DAY)
            ),
            "cannot compare with origin/main"
        );
    }

    #[test]
    fn a_fetch_stamped_in_the_future_reads_as_just_now() {
        // A clock moved back since the fetch must not panic or say
        // something negative.
        let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        let line = super::tracking_summary(
            &super::tracking::Tracking {
                branch: "main".to_owned(),
                upstream: compared(
                    super::tracking::Count::Exact(0),
                    super::tracking::Count::Exact(0),
                ),
                last_fetch: Some(now + std::time::Duration::from_secs(600)),
            },
            now,
        );
        assert_eq!(
            line,
            "up to date with origin/main (as of last fetch, just now)"
        );
    }

    // ---- the File pane's fact table (#576) ------------------------------

    fn view_with(repository: Option<super::repository::Repository>) -> serde_json::Value {
        serde_json::to_value(DirectoryView {
            entry_count: 12,
            total_size: 17357,
            repository,
        })
        .unwrap()
    }

    #[test]
    fn facts_for_a_checkout_with_an_upstream_ahead_and_behind() {
        let data = view_with(Some(super::repository::Repository {
            provider: Some("github.com".to_owned()),
            branch: Some("main".to_owned()),
            remote: Some("https://github.com/owner/name.git".to_owned()),
            tracking: Some(super::tracking::Tracking {
                branch: "main".to_owned(),
                upstream: compared(
                    super::tracking::Count::Exact(2),
                    super::tracking::Count::Exact(5),
                ),
                last_fetch: Some(
                    std::time::SystemTime::now() - std::time::Duration::from_hours(3 * 24),
                ),
            }),
            status: Some(super::status::WorkingTree {
                changed: 0,
                examined: 20,
                partial: false,
            }),
        }));

        let facts = DirectoryPresentation.facts(&data);

        assert_eq!(
            facts,
            vec![
                Fact::new("Provider", "github.com"),
                Fact::new("Branch", "main"),
                Fact::new("Tracking", "2 ahead, 5 behind origin/main"),
                Fact {
                    label: "Last fetched".to_owned(),
                    value: "3 days ago".to_owned(),
                    dim: false,
                },
                Fact::new("Remote", "https://github.com/owner/name.git"),
                Fact::new("Working tree", "no uncommitted changes to tracked files"),
                Fact::new("", ""),
                Fact {
                    label: "Entries".to_owned(),
                    value: "12".to_owned(),
                    dim: true,
                },
                Fact {
                    label: "Total size".to_owned(),
                    value: "17.0 KB".to_owned(),
                    dim: true,
                },
            ]
        );
    }

    #[test]
    fn facts_for_a_checkout_with_no_upstream_and_never_fetched() {
        let data = view_with(Some(super::repository::Repository {
            provider: None,
            branch: Some("trunk".to_owned()),
            remote: None,
            tracking: Some(super::tracking::Tracking {
                branch: "trunk".to_owned(),
                upstream: super::tracking::Upstream::None,
                last_fetch: None,
            }),
            status: None,
        }));

        let facts = DirectoryPresentation.facts(&data);

        assert!(
            !facts.iter().any(|fact| fact.label == "Provider"),
            "no remote means no provider row: {facts:?}"
        );
        assert!(facts.contains(&Fact::new("Branch", "trunk")));
        assert!(facts.contains(&Fact::new("Tracking", "no upstream")));
        assert!(
            facts.contains(&Fact {
                label: "Last fetched".to_owned(),
                value: "never".to_owned(),
                dim: true,
            }),
            "a fetch that never happened is at least as stale as an old one: {facts:?}"
        );
        assert!(facts.contains(&Fact::new("Remote", "none configured")));
        assert!(facts.contains(&Fact::new("Working tree", "could not read the index")));
    }

    #[test]
    fn facts_for_a_detached_head_names_no_branch_and_no_tracking() {
        let data = view_with(Some(super::repository::Repository {
            provider: Some("gitlab.com".to_owned()),
            branch: None,
            remote: Some("git@gitlab.com:group/project.git".to_owned()),
            tracking: None,
            status: None,
        }));

        let facts = DirectoryPresentation.facts(&data);

        assert!(facts.contains(&Fact::new("Branch", "none checked out (detached head)")));
        assert!(
            !facts.iter().any(|fact| fact.label == "Tracking"),
            "a detached head has no branch to compare with an upstream: {facts:?}"
        );
        assert!(
            !facts.iter().any(|fact| fact.label == "Last fetched"),
            "and so nothing to say when it was last fetched: {facts:?}"
        );
    }

    #[test]
    fn a_fetch_older_than_thirty_days_reads_as_stale() {
        let data = view_with(Some(super::repository::Repository {
            provider: None,
            branch: Some("main".to_owned()),
            remote: None,
            tracking: Some(super::tracking::Tracking {
                branch: "main".to_owned(),
                upstream: compared(
                    super::tracking::Count::Exact(0),
                    super::tracking::Count::Exact(0),
                ),
                last_fetch: Some(
                    std::time::SystemTime::now() - std::time::Duration::from_hours(40 * 24),
                ),
            }),
            status: None,
        }));

        let facts = DirectoryPresentation.facts(&data);

        let fetched = facts
            .iter()
            .find(|fact| fact.label == "Last fetched")
            .expect("a tracked branch has a last-fetched row");
        assert!(
            fetched.dim,
            "an old \"up to date\" should not be read as current: {facts:?}"
        );
    }

    #[test]
    fn a_plain_folders_facts_are_only_the_folder_ones() {
        let data = view_with(None);

        let facts = DirectoryPresentation.facts(&data);

        assert_eq!(
            facts,
            vec![
                Fact {
                    label: "Entries".to_owned(),
                    value: "12".to_owned(),
                    dim: true,
                },
                Fact {
                    label: "Total size".to_owned(),
                    value: "17.0 KB".to_owned(),
                    dim: true,
                },
            ]
        );
    }
}
