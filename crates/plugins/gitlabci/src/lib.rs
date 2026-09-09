//! `GitLab` CI configuration file type plugin: core and presentation halves.
//!
//! A specialisation of YAML: a `stages:` list, or jobs carrying a
//! `script:` key, is a pipeline. GitHub's workflows use `jobs:` and `on:`
//! and are claimed by their own plugin first.

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

/// One job in the pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    /// Its key at the top level.
    pub name: String,
    /// The stage it belongs to, or `test` when it does not say.
    pub stage: String,
    /// The image it runs in.
    pub image: Option<String>,
    /// How many script lines it has.
    pub script_lines: usize,
    /// Whether it declares `rules:` or the older `only:`/`except:`.
    pub conditional: bool,
    /// Whether it declares artifacts.
    pub artifacts: bool,
    /// Whether it is a template, which starts with a dot and never runs
    /// on its own.
    pub template: bool,
}

/// View data produced by [`GitlabciCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitlabciView {
    /// The stages, in the order they run.
    pub stages: Vec<String>,
    /// Every job, in file order.
    pub jobs: Vec<Job>,
    /// The images used, each once.
    pub images: Vec<String>,
    /// The files it includes.
    pub includes: Vec<String>,
    /// Jobs whose stage is not in the `stages:` list, which never run.
    pub orphan_stages: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The keys `GitLab` reserves, which are configuration rather than jobs.
const RESERVED: &[&str] = &[
    "stages",
    "variables",
    "default",
    "include",
    "workflow",
    "image",
    "before_script",
    "after_script",
    "cache",
    "services",
];

/// The stage names, whether written in flow style or as a block.
fn stages_of(lines: &[&str], from: usize, line: &str) -> Vec<String> {
    if let Some(flow) = value_of(line) {
        return flow
            .trim_matches(['[', ']'])
            .split(',')
            .map(|one| one.trim().trim_matches(['"', '\'']).to_owned())
            .filter(|one| !one.is_empty())
            .collect();
    }
    lines
        .iter()
        .skip(from + 1)
        .take_while(|candidate| candidate.trim().is_empty() || indent(candidate) > 0)
        .filter_map(|candidate| {
            candidate
                .trim_start()
                .strip_prefix("- ")
                .map(|one| one.trim().trim_matches(['"', '\'']).to_owned())
        })
        .collect()
}

/// The files named under an `include:` block, in whichever of its four
/// forms they were written.
fn includes_under(lines: &[&str], from: usize) -> Vec<String> {
    let mut found = Vec::new();
    for candidate in lines.iter().skip(from + 1) {
        if candidate.trim().is_empty() {
            continue;
        }
        if indent(candidate) == 0 {
            break;
        }
        let trimmed = candidate.trim_start();
        for prefix in ["- local:", "- project:", "- remote:", "- template:", "- "] {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                let value = rest.trim().trim_matches(['"', '\'']).to_owned();
                if !value.is_empty() && !value.ends_with(':') {
                    found.push(value);
                }
                break;
            }
        }
    }
    found
}

/// Everything [`GitlabciView`] holds, read from `text`.
fn parse(text: &str) -> GitlabciView {
    let lines: Vec<&str> = text.lines().collect();
    let mut view = GitlabciView {
        stages: Vec::new(),
        jobs: Vec::new(),
        images: Vec::new(),
        includes: Vec::new(),
        orphan_stages: Vec::new(),
        content: String::new(),
        truncated: false,
    };

    for (index, line) in lines.iter().enumerate() {
        if indent(line) != 0 {
            continue;
        }
        let Some(key) = key_of(line) else { continue };

        if key == "stages" {
            view.stages = stages_of(&lines, index, line);
            continue;
        }
        if key == "include" {
            view.includes = includes_under(&lines, index);
            continue;
        }
        if RESERVED.contains(&key) {
            continue;
        }

        // Anything else at the top level with a block under it is a job.
        let base = indent(line);
        let body: Vec<&&str> = lines
            .iter()
            .skip(index + 1)
            .take_while(|candidate| candidate.trim().is_empty() || indent(candidate) > base)
            .collect();
        if body.is_empty() {
            continue;
        }
        let has = |wanted: &str| {
            body.iter()
                .any(|candidate| key_of(candidate) == Some(wanted))
        };
        // A leading dot is GitLab's own marker for a hidden key, and a
        // hidden key with a block under it is a template - whether or not
        // it carries a `script` of its own. `.cargo` here holds an image, a
        // cache and a `before_script`, and is still a template.
        let template = key.starts_with('.');
        if !template && !has("script") && !has("extends") && !has("trigger") {
            continue;
        }

        let image = body
            .iter()
            .find(|candidate| key_of(candidate) == Some("image"))
            .and_then(|candidate| value_of(candidate));
        if let Some(image) = &image
            && !view.images.contains(image)
        {
            view.images.push(image.clone());
        }

        let script_lines = body
            .iter()
            .filter(|candidate| candidate.trim_start().starts_with("- "))
            .count();
        let stage = body
            .iter()
            .find(|candidate| key_of(candidate) == Some("stage"))
            .and_then(|candidate| value_of(candidate))
            .unwrap_or_else(|| "test".to_owned());

        view.jobs.push(Job {
            name: key.to_owned(),
            stage,
            image,
            script_lines,
            conditional: has("rules") || has("only") || has("except"),
            artifacts: has("artifacts"),
            template,
        });
    }

    for job in &view.jobs {
        if job.template || view.stages.is_empty() {
            continue;
        }
        if !view.stages.contains(&job.stage) && !view.orphan_stages.contains(&job.name) {
            view.orphan_stages.push(job.name.clone());
        }
    }
    view
}

/// Whether `text` is a `GitLab` pipeline.
fn looks_like_it(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let top = |wanted: &str| {
        lines
            .iter()
            .any(|line| indent(line) == 0 && key_of(line) == Some(wanted))
    };
    // GitHub's workflows are claimed by their own plugin; `on:` and
    // `jobs:` together are theirs, not this one's.
    if top("on") && top("jobs") {
        return false;
    }
    let scripts = lines
        .iter()
        .filter(|line| key_of(line) == Some("script"))
        .count();
    (top("stages") && scripts >= 1) || scripts >= 2
}

/// The `GitLab` CI configuration plugin's core half.
#[derive(Debug, Default)]
pub struct GitlabciCore;

impl PluginCore for GitlabciCore {
    fn name(&self) -> &'static str {
        "gitlabci"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A specialisation of YAML, which owns the extension (D13).
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

/// The `GitLab` CI configuration plugin's presentation half.
#[derive(Debug, Default)]
pub struct GitlabciPresentation;

impl PluginPresentation for GitlabciPresentation {
    fn name(&self) -> &'static str {
        "gitlabci"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "GLCI",
            tint: 0x00fc_6d26,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: GitlabciView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.stages.is_empty() {
            lines.push(format!("Stages: {}", view.stages.join(" -> ")));
        }
        lines.push(format!("{} job(s):", view.jobs.len()));
        for job in &view.jobs {
            let mut notes = Vec::new();
            if job.template {
                notes.push("template".to_owned());
            }
            if job.conditional {
                notes.push("conditional".to_owned());
            }
            if job.artifacts {
                notes.push("artifacts".to_owned());
            }
            if let Some(image) = &job.image {
                notes.push(format!("in {image}"));
            }
            let suffix = if notes.is_empty() {
                String::new()
            } else {
                format!("  ({})", notes.join(", "))
            };
            lines.push(format!(
                "  {}  stage {}, {} script line(s){suffix}",
                job.name, job.stage, job.script_lines
            ));
        }
        if !view.images.is_empty() {
            lines.push(format!("Images: {}", view.images.join(", ")));
        }
        if !view.includes.is_empty() {
            lines.push(format!("Includes: {}", view.includes.join(", ")));
        }
        if !view.orphan_stages.is_empty() {
            lines.push(
                "These jobs name a stage the pipeline does not declare, so they never run:"
                    .to_owned(),
            );
            for job in &view.orphan_stages {
                lines.push(format!("  {job}"));
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
    use super::{GitlabciCore, GitlabciPresentation, GitlabciView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const PIPELINE: &str = "stages: [build, test, deploy]\n\
        include:\n  - local: /ci/shared.yml\n\
        .base:\n  image: rust:1.87\n  script:\n    - cargo --version\n\
        build:\n  stage: build\n  image: rust:1.87\n  script:\n\
        \x20   - cargo build --release\n  artifacts:\n    paths: [target/release]\n\
        test:\n  stage: test\n  script:\n    - cargo test\n    - cargo clippy\n\
        \x20 rules:\n    - if: $CI_COMMIT_BRANCH\n\
        stray:\n  stage: nowhere\n  script:\n    - echo hello\n";

    #[test]
    fn sniffs_a_pipeline() {
        assert!(GitlabciCore.sniff(PIPELINE.as_bytes()));
    }

    #[test]
    fn leaves_a_github_workflow_to_its_own_plugin() {
        assert!(
            !GitlabciCore.sniff(
                b"on:\n  push:\njobs:\n  build:\n    runs-on: ubuntu-latest\n    script: x\n"
            )
        );
        assert!(!GitlabciCore.sniff(b"name: a\nversion: 1\n"));
        assert!(!GitlabciCore.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_yaml() {
        assert_eq!(GitlabciCore.specialises(), &["yaml"]);
    }

    #[test]
    fn reads_the_stages_in_order() {
        let view = parse(PIPELINE);

        assert_eq!(
            view.stages,
            vec!["build".to_owned(), "test".to_owned(), "deploy".to_owned()]
        );
    }

    #[test]
    fn reserved_keys_are_not_jobs_but_templates_are() {
        let view = parse(PIPELINE);

        let names: Vec<&str> = view.jobs.iter().map(|job| job.name.as_str()).collect();
        assert!(
            !names.contains(&"stages"),
            "a reserved key is configuration"
        );
        assert!(!names.contains(&"include"));
        assert!(
            names.contains(&".base"),
            "a template is a job that never runs alone"
        );
        assert!(
            view.jobs
                .iter()
                .find(|job| job.name == ".base")
                .unwrap()
                .template
        );
    }

    #[test]
    fn reads_what_each_job_declares() {
        let view = parse(PIPELINE);

        let build = view.jobs.iter().find(|job| job.name == "build").unwrap();
        assert_eq!(build.stage, "build");
        assert!(build.artifacts);
        assert_eq!(build.image.as_deref(), Some("rust:1.87"));

        let test = view.jobs.iter().find(|job| job.name == "test").unwrap();
        assert!(test.conditional);
        assert!(test.script_lines >= 2);
    }

    #[test]
    fn a_job_naming_a_stage_the_pipeline_does_not_declare_never_runs() {
        let view = parse(PIPELINE);

        assert_eq!(view.orphan_stages, vec!["stray".to_owned()]);
    }

    #[test]
    fn presents_the_stages_and_the_orphans() {
        let data = serde_json::to_value(parse(PIPELINE)).unwrap();

        let lines = GitlabciPresentation.present(&data);

        assert_eq!(lines[0], "Stages: build -> test -> deploy");
        assert!(lines.iter().any(|line| line.contains("never run")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/gitlabci/.gitlab-ci.yml");

        let data = GitlabciCore.view(&path).unwrap();
        let view: GitlabciView = serde_json::from_value(data).unwrap();

        assert!(view.stages.len() >= 3);
        assert!(view.jobs.len() >= 4);
        assert!(view.jobs.iter().any(|job| job.template));
        assert!(view.jobs.iter().any(|job| job.conditional));
        assert!(view.jobs.iter().any(|job| job.artifacts));
        assert!(!view.images.is_empty());
        assert!(!view.includes.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::GitlabciCore),
            plugin_api::PluginPresentation::extensions(&crate::GitlabciPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
