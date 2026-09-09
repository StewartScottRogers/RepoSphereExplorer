//! GitHub Actions workflow file type plugin: core and presentation halves.
//!
//! A specialisation of YAML: `on:` alongside `jobs:` whose entries carry
//! `runs-on` or `steps` is a workflow and nothing else.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// How deep `line` is indented, in spaces.
fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The key of a `key:` or `key: value` line, at any depth.
fn key_of(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') || trimmed.starts_with('-') {
        return None;
    }
    let (key, rest) = trimmed.split_once(':')?;
    if !(rest.is_empty() || rest.starts_with(' ')) {
        return None;
    }
    let key = key.trim();
    if key.is_empty() || key.contains(' ') {
        return None;
    }
    Some(key)
}

/// The value of a `key: value` line, unquoted, or `None` when the line
/// only opens a block.
fn value_of(line: &str) -> Option<String> {
    let (_, rest) = line.trim_start().split_once(':')?;
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    Some(rest.trim_matches(['"', '\'']).to_owned())
}

/// The value of the first `key:` at indentation `depth` or deeper, within
/// the block starting at `from`.
fn nested_value(lines: &[&str], from: usize, wanted: &str) -> Option<String> {
    let base = indent(lines.get(from)?);
    for line in lines.iter().skip(from + 1) {
        if line.trim().is_empty() {
            continue;
        }
        if indent(line) <= base {
            break;
        }
        if key_of(line) == Some(wanted) {
            return value_of(line);
        }
    }
    None
}

/// The keys directly one level inside the block that starts at `from`.
fn children_of(lines: &[&str], from: usize) -> Vec<String> {
    let base = indent(lines[from]);
    let mut depth: Option<usize> = None;
    let mut found = Vec::new();
    for line in lines.iter().skip(from + 1) {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let here = indent(line);
        if here <= base {
            break;
        }
        let depth = *depth.get_or_insert(here);
        if here == depth
            && let Some(key) = key_of(line)
        {
            found.push(key.to_owned());
        }
    }
    found
}

/// One job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    /// Its key in the `jobs:` mapping.
    pub id: String,
    /// The runner it asks for.
    pub runs_on: Option<String>,
    /// The jobs it waits for.
    pub needs: Vec<String>,
    /// A reusable workflow it calls instead of running steps.
    pub uses: Option<String>,
    /// How many steps it has.
    pub steps: usize,
}

/// View data produced by [`GithubactionsCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubactionsView {
    /// The workflow's name.
    pub name: Option<String>,
    /// The events that trigger it.
    pub events: Vec<String>,
    /// The jobs, in file order.
    pub jobs: Vec<Job>,
    /// The actions used, as `owner/repo@ref`, each once.
    pub actions: Vec<String>,
    /// Actions pinned to a branch or a major tag rather than a commit,
    /// which is what makes a supply chain mutable under you.
    pub unpinned_actions: Vec<String>,
    /// The permissions the workflow grants.
    pub permissions: Vec<String>,
    /// The secrets it reads, by name.
    pub secrets: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Every `secrets.NAME` in `text`.
fn secrets_in(text: &str, into: &mut Vec<String>) {
    let mut rest = text;
    while let Some(at) = rest.find("secrets.") {
        rest = &rest[at + "secrets.".len()..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() && !into.contains(&name) {
            into.push(name);
        }
    }
}

/// Everything [`GithubactionsView`] holds, read from `text`.
fn parse(text: &str) -> GithubactionsView {
    let lines: Vec<&str> = text.lines().collect();
    let mut view = GithubactionsView {
        name: None,
        events: Vec::new(),
        jobs: Vec::new(),
        actions: Vec::new(),
        unpinned_actions: Vec::new(),
        permissions: Vec::new(),
        secrets: Vec::new(),
        content: String::new(),
        truncated: false,
    };

    for (index, line) in lines.iter().enumerate() {
        if indent(line) == 0 {
            match key_of(line) {
                Some("name") => view.name = value_of(line),
                Some("on") => {
                    // `on: push` and `on:` with a block beneath it.
                    if let Some(single) = value_of(line) {
                        view.events.push(single);
                    } else {
                        view.events = children_of(&lines, index);
                    }
                }
                Some("permissions") => view.permissions = children_of(&lines, index),
                Some("jobs") => {
                    for job in children_of(&lines, index) {
                        let at = lines
                            .iter()
                            .enumerate()
                            .skip(index + 1)
                            .find(|(_, candidate)| key_of(candidate) == Some(job.as_str()))
                            .map(|(at, _)| at);
                        let Some(at) = at else { continue };
                        let base = indent(lines[at]);
                        let steps = lines
                            .iter()
                            .skip(at + 1)
                            .take_while(|candidate| {
                                candidate.trim().is_empty() || indent(candidate) > base
                            })
                            .filter(|candidate| {
                                let trimmed = candidate.trim_start();
                                trimmed.starts_with("- name:")
                                    || trimmed.starts_with("- uses:")
                                    || trimmed.starts_with("- run:")
                            })
                            .count();
                        view.jobs.push(Job {
                            id: job,
                            runs_on: nested_value(&lines, at, "runs-on"),
                            needs: nested_value(&lines, at, "needs")
                                .map(|needs| {
                                    needs
                                        .trim_matches(['[', ']'])
                                        .split(',')
                                        .map(|one| one.trim().to_owned())
                                        .filter(|one| !one.is_empty())
                                        .collect()
                                })
                                .unwrap_or_default(),
                            uses: nested_value(&lines, at, "uses"),
                            steps,
                        });
                    }
                }
                _ => {}
            }
        }

        if let Some(rest) = line.trim_start().strip_prefix("- uses:") {
            let action = rest.trim().trim_matches(['"', '\'']).to_owned();
            if !action.is_empty() && !view.actions.contains(&action) {
                // A commit identifier is forty hexadecimal characters; a
                // branch or a major tag is anything else, and can move.
                let pinned = action.rsplit('@').next().is_some_and(|reference| {
                    reference.len() == 40 && reference.chars().all(|c| c.is_ascii_hexdigit())
                });
                if !pinned && action.contains('@') {
                    view.unpinned_actions.push(action.clone());
                }
                view.actions.push(action);
            }
        }
        secrets_in(line, &mut view.secrets);
    }
    view
}

/// Whether `text` is a GitHub Actions workflow.
fn looks_like_it(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let top = |wanted: &str| {
        lines
            .iter()
            .any(|line| indent(line) == 0 && key_of(line) == Some(wanted))
    };
    if !(top("on") && top("jobs")) {
        return false;
    }
    // `on:` and `jobs:` together is close; a `runs-on`, `steps` or `uses`
    // beneath them settles it.
    lines.iter().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("runs-on:")
            || trimmed.starts_with("steps:")
            || trimmed.starts_with("- uses:")
    })
}

/// The GitHub Actions workflow plugin's core half.
#[derive(Debug, Default)]
pub struct GithubactionsCore;

impl PluginCore for GithubactionsCore {
    fn name(&self) -> &'static str {
        "githubactions"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A specialisation of YAML, which owns the extension. Without this
        // the extension hint hands the file over whatever the order (D13).
        &["yaml"]
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

/// The GitHub Actions workflow plugin's presentation half.
#[derive(Debug, Default)]
pub struct GithubactionsPresentation;

impl PluginPresentation for GithubactionsPresentation {
    fn name(&self) -> &'static str {
        "githubactions"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "GHA",
            tint: 0x002b_3137,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: GithubactionsView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(name) = &view.name {
            lines.push(format!("Workflow: {name}"));
        }
        if !view.events.is_empty() {
            lines.push(format!("Runs on: {}", view.events.join(", ")));
        }
        if !view.permissions.is_empty() {
            lines.push(format!("Permissions: {}", view.permissions.join(", ")));
        }
        lines.push(format!("{} job(s):", view.jobs.len()));
        for job in &view.jobs {
            let runner = job.runs_on.as_deref().unwrap_or("no runner named");
            let after = if job.needs.is_empty() {
                String::new()
            } else {
                format!(", after {}", job.needs.join(" and "))
            };
            if let Some(uses) = &job.uses {
                lines.push(format!("  {}  calls {uses}{after}", job.id));
            } else {
                lines.push(format!(
                    "  {}  on {runner}, {} step(s){after}",
                    job.id, job.steps
                ));
            }
        }
        if !view.actions.is_empty() {
            lines.push(format!("Actions ({}):", view.actions.len()));
            for action in &view.actions {
                lines.push(format!("  {action}"));
            }
        }
        if !view.unpinned_actions.is_empty() {
            lines.push(
                "Pinned to a moving reference, so what runs can change under you:".to_owned(),
            );
            for action in &view.unpinned_actions {
                lines.push(format!("  {action}"));
            }
        }
        if !view.secrets.is_empty() {
            lines.push(format!("Secrets read: {}", view.secrets.join(", ")));
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{GithubactionsCore, GithubactionsPresentation, GithubactionsView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const WORKFLOW: &str = "name: CI\n\
        on:\n  push:\n    branches: [main]\n  pull_request:\n  workflow_dispatch:\n\
        permissions:\n  contents: read\n  pull-requests: write\n\
        jobs:\n\
        \x20 build:\n    runs-on: ubuntu-latest\n    steps:\n\
        \x20     - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683\n\
        \x20     - name: Test\n        run: cargo test\n\
        \x20 deploy:\n    runs-on: ubuntu-latest\n    needs: [build]\n    steps:\n\
        \x20     - uses: actions/deploy-pages@v4\n\
        \x20       env:\n          TOKEN: ${{ secrets.DEPLOY_TOKEN }}\n";

    #[test]
    fn sniffs_a_workflow() {
        assert!(GithubactionsCore.sniff(WORKFLOW.as_bytes()));
    }

    #[test]
    fn does_not_claim_yaml_that_merely_has_those_keys() {
        assert!(!GithubactionsCore.sniff(b"on: monday\njobs: many\n"));
        assert!(!GithubactionsCore.sniff(b"name: a\nversion: 1\n"));
        assert!(!GithubactionsCore.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_yaml() {
        assert_eq!(GithubactionsCore.specialises(), &["yaml"]);
    }

    #[test]
    fn reads_the_name_events_and_permissions() {
        let view = parse(WORKFLOW);

        assert_eq!(view.name.as_deref(), Some("CI"));
        assert_eq!(
            view.events,
            vec![
                "push".to_owned(),
                "pull_request".to_owned(),
                "workflow_dispatch".to_owned()
            ]
        );
        assert_eq!(
            view.permissions,
            vec!["contents".to_owned(), "pull-requests".to_owned()]
        );
    }

    #[test]
    fn reads_each_job_with_its_runner_steps_and_dependencies() {
        let view = parse(WORKFLOW);

        assert_eq!(view.jobs.len(), 2);
        assert_eq!(view.jobs[0].id, "build");
        assert_eq!(view.jobs[0].runs_on.as_deref(), Some("ubuntu-latest"));
        assert_eq!(view.jobs[0].steps, 2);
        assert_eq!(view.jobs[1].needs, vec!["build".to_owned()]);
    }

    #[test]
    fn tells_a_commit_pin_from_a_moving_tag() {
        let view = parse(WORKFLOW);

        assert_eq!(view.actions.len(), 2);
        assert_eq!(
            view.unpinned_actions,
            vec!["actions/deploy-pages@v4".to_owned()],
            "a forty-character commit is pinned; `@v4` can move under you"
        );
    }

    #[test]
    fn names_the_secrets_it_reads() {
        let view = parse(WORKFLOW);

        assert_eq!(view.secrets, vec!["DEPLOY_TOKEN".to_owned()]);
    }

    #[test]
    fn presents_the_moving_pins_with_the_reason() {
        let data = serde_json::to_value(parse(WORKFLOW)).unwrap();

        let lines = GithubactionsPresentation.present(&data);

        assert_eq!(lines[0], "Workflow: CI");
        assert!(lines.iter().any(|line| line.contains("change under you")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/githubactions/release.yml");

        let data = GithubactionsCore.view(&path).unwrap();
        let view: GithubactionsView = serde_json::from_value(data).unwrap();

        assert!(view.name.is_some());
        assert!(view.events.len() >= 2);
        assert!(view.jobs.len() >= 3);
        assert!(view.jobs.iter().any(|job| !job.needs.is_empty()));
        assert!(view.jobs.iter().any(|job| job.uses.is_some()));
        assert!(view.actions.len() >= 3);
        assert!(!view.unpinned_actions.is_empty());
        assert!(!view.permissions.is_empty());
        assert!(!view.secrets.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::GithubactionsCore),
            plugin_api::PluginPresentation::extensions(&crate::GithubactionsPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
