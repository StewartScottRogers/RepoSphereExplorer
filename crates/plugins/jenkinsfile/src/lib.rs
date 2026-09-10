//! Jenkinsfile file type plugin: core and presentation halves.
//!
//! A Jenkinsfile is the build, written down. This reads whether it is
//! declarative or scripted, the agent, every stage with its step count
//! and whether it is parallel or conditional, the parameters, the
//! environment names, the post conditions, and the stages with no steps.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One stage of the pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage {
    /// The name in `stage('...')`.
    pub name: String,
    /// How many steps its `steps` block holds.
    pub steps: usize,
    /// Whether it is a branch of a `parallel` block.
    pub parallel: bool,
    /// Whether a `when` block decides if it runs at all.
    pub conditional: bool,
    /// The agent it asks for, when it asks for one of its own.
    pub agent: Option<String>,
}

/// View data produced by [`JenkinsfileCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JenkinsfileView {
    /// `declarative` for a `pipeline` block, `scripted` for a `node` one.
    pub style: String,
    /// The agent the whole pipeline runs on.
    pub agent: Option<String>,
    /// Every stage, in the order the file declares them.
    pub stages: Vec<Stage>,
    /// The names set in the `environment` block. Names only: a value can
    /// be a secret, and a file pane is not the place to publish one.
    pub environment: Vec<String>,
    /// The build parameters declared.
    pub parameters: Vec<String>,
    /// The conditions the `post` block reacts to.
    pub post_conditions: Vec<String>,
    /// Stages whose `steps` block is empty, which run and do nothing.
    pub stages_without_steps: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The conditions a `post` block may name.
const POST_CONDITIONS: &[&str] = &[
    "always",
    "changed",
    "fixed",
    "regression",
    "aborted",
    "failure",
    "success",
    "unstable",
    "unsuccessful",
    "cleanup",
];

/// `line` up to its line comment.
fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(at) => &line[..at],
        None => line,
    }
}

/// The first single- or double-quoted run in `text`.
fn first_quoted(text: &str) -> Option<&str> {
    let open = text.find(['\'', '"'])?;
    let quote = text[open..].chars().next()?;
    let rest = &text[open + quote.len_utf8()..];
    let close = rest.find(quote)?;
    Some(&rest[..close])
}

/// What a line says before the brace it opens.
fn label_of(line: &str) -> &str {
    line.split('{').next().unwrap_or("").trim()
}

/// The state the walk carries from one line to the next.
#[derive(Default)]
struct Walk {
    /// The block labels currently open, with the depth each was opened at.
    stack: Vec<(String, usize)>,
    /// How many braces deep the walk is.
    depth: usize,
    /// The depth an `agent` block was opened at, while its body is being
    /// read - `agent` on its own line says nothing until the next one.
    awaiting_agent: Option<usize>,
}

impl Walk {
    /// The label of the block this line sits directly inside.
    fn parent(&self) -> &str {
        self.stack.last().map_or("", |(label, _)| label.as_str())
    }

    /// Whether any open block's label starts with `name`.
    fn inside(&self, name: &str) -> bool {
        self.stack.iter().any(|(label, _)| label.starts_with(name))
    }

    /// Applies `line`'s braces, pushing a frame when it opens one.
    fn advance(&mut self, line: &str, label: &str) {
        let opens = line.matches('{').count();
        let closes = line.matches('}').count();
        if opens > closes {
            self.depth += opens - closes;
            self.stack.push((label.to_owned(), self.depth));
        } else {
            self.depth = self.depth.saturating_sub(closes - opens);
            while self.stack.last().is_some_and(|(_, at)| *at > self.depth) {
                self.stack.pop();
            }
        }
    }
}

/// Reads a line that sits directly inside a named block.
fn read_block_member(walk: &Walk, line: &str, label: &str, view: &mut JenkinsfileView) {
    match walk.parent() {
        "environment" => {
            if let Some((name, _)) = line.split_once('=') {
                view.environment.push(name.trim().to_owned());
            }
        }
        "parameters" => {
            if let Some(name) = first_quoted(line) {
                let kind = line.split('(').next().unwrap_or("parameter").trim();
                view.parameters.push(format!("{name} ({kind})"));
            }
        }
        "post" => {
            if POST_CONDITIONS.contains(&label) {
                view.post_conditions.push(label.to_owned());
            }
        }
        parent if parent.starts_with("steps") => {
            if let Some(stage) = view.stages.last_mut() {
                stage.steps += 1;
            }
        }
        _ => {}
    }
}

/// Reads a line that opens or answers an `agent` declaration.
fn read_agent(walk: &mut Walk, line: &str, view: &mut JenkinsfileView) -> bool {
    if let Some(at) = walk.awaiting_agent
        && walk.depth == at
    {
        walk.awaiting_agent = None;
        let said = label_of(line);
        set_agent(walk, said.to_owned(), view);
        return true;
    }
    let Some(rest) = line.strip_prefix("agent") else {
        return false;
    };
    let rest = rest.trim();
    if rest.starts_with('{') && rest.len() == 1 {
        // `agent {` says nothing; the line after it does.
        walk.awaiting_agent = Some(walk.depth + 1);
        return false;
    }
    set_agent(walk, rest.trim_matches(['{', '}']).trim().to_owned(), view);
    false
}

/// Records `said` as the pipeline's agent, or the current stage's.
fn set_agent(walk: &Walk, said: String, view: &mut JenkinsfileView) {
    if walk.inside("stage(") {
        if let Some(stage) = view.stages.last_mut() {
            stage.agent = Some(said);
        }
    } else {
        view.agent = Some(said);
    }
}

/// Everything [`JenkinsfileView`] holds, read from `text`.
fn parse(text: &str) -> JenkinsfileView {
    let mut view = JenkinsfileView {
        style: "unrecognised".to_owned(),
        agent: None,
        stages: Vec::new(),
        environment: Vec::new(),
        parameters: Vec::new(),
        post_conditions: Vec::new(),
        stages_without_steps: Vec::new(),
        truncated: false,
    };
    let mut walk = Walk::default();
    // Stages that hold other stages. They have no steps of their own and
    // are not idle: their branches do the work.
    let mut parents: Vec<String> = Vec::new();

    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() || line.chars().all(|letter| "{}()".contains(letter)) {
            walk.advance(line, "");
            continue;
        }
        let label = label_of(line);

        if walk.stack.is_empty() {
            if label.starts_with("pipeline") {
                "declarative".clone_into(&mut view.style);
            } else if label.starts_with("node") {
                "scripted".clone_into(&mut view.style);
            }
        }
        if label.starts_with("stage(") || label.starts_with("stage (") {
            if let Some(enclosing) = walk
                .stack
                .iter()
                .rev()
                .find(|(open, _)| open.starts_with("stage("))
                .and_then(|(open, _)| first_quoted(open))
            {
                parents.push(enclosing.to_owned());
            }
            view.stages.push(Stage {
                name: first_quoted(label).unwrap_or("unnamed").to_owned(),
                steps: 0,
                parallel: walk.inside("parallel"),
                conditional: false,
                agent: None,
            });
        } else if label == "when"
            && let Some(stage) = view.stages.last_mut()
        {
            stage.conditional = true;
        }

        let answered_agent = read_agent(&mut walk, line, &mut view);
        if !answered_agent && !label.starts_with("stage(") {
            read_block_member(&walk, line, label, &mut view);
        }
        walk.advance(line, label);
    }

    view.stages_without_steps = view
        .stages
        .iter()
        .filter(|stage| stage.steps == 0 && !parents.contains(&stage.name))
        .map(|stage| stage.name.clone())
        .collect();
    view
}

/// Whether `text` is a Jenkinsfile.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    // A `pipeline` or `node` block on its own is Groovy that happens to
    // use those words. It has to have declared a stage as well.
    view.style != "unrecognised" && !view.stages.is_empty()
}

/// The Jenkinsfile plugin's core half.
#[derive(Debug, Default)]
pub struct JenkinsfileCore;

impl PluginCore for JenkinsfileCore {
    fn name(&self) -> &'static str {
        "jenkinsfile"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A Jenkinsfile is Groovy, and `groovy` recognises it. Saying so
        // is what settles the file on the narrower reading (D13).
        &["groovy"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        // The stages and their steps are what a reader came for, and
        // they are on the view already.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Jenkinsfile plugin's presentation half.
#[derive(Debug, Default)]
pub struct JenkinsfilePresentation;

impl PluginPresentation for JenkinsfilePresentation {
    fn name(&self) -> &'static str {
        "jenkinsfile"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "JNK",
            tint: 0x00d3_3833,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: JenkinsfileView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("{} pipeline", view.style));
        lines.push(format!(
            "Agent: {}",
            view.agent.as_deref().unwrap_or("not declared")
        ));
        if !view.parameters.is_empty() {
            lines.push(format!("Parameters: {}", view.parameters.join(", ")));
        }
        if !view.environment.is_empty() {
            lines.push(format!(
                "Environment (names only): {}",
                view.environment.join(", ")
            ));
        }
        lines.push(format!("{} stage(s):", view.stages.len()));
        for stage in &view.stages {
            let mut notes = Vec::new();
            if stage.parallel {
                notes.push("parallel".to_owned());
            }
            if stage.conditional {
                notes.push("conditional".to_owned());
            }
            if let Some(agent) = &stage.agent {
                notes.push(format!("agent {agent}"));
            }
            let notes = if notes.is_empty() {
                String::new()
            } else {
                format!(" [{}]", notes.join(", "))
            };
            lines.push(format!("  {} - {} step(s){notes}", stage.name, stage.steps));
        }
        if !view.post_conditions.is_empty() {
            lines.push(format!("Post: {}", view.post_conditions.join(", ")));
        }
        if !view.stages_without_steps.is_empty() {
            lines.push("No steps, so these stages run and do nothing:".to_owned());
            for name in &view.stages_without_steps {
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
    use super::{JenkinsfileCore, JenkinsfilePresentation, JenkinsfileView, first_quoted, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const PIPELINE: &str = concat!(
        "pipeline {\n",
        "    agent {\n",
        "        label 'linux'\n",
        "    }\n",
        "    parameters {\n",
        "        string(name: 'BRANCH', defaultValue: 'main')\n",
        "        booleanParam(name: 'PUBLISH', defaultValue: false)\n",
        "    }\n",
        "    environment {\n",
        "        REGISTRY = 'registry.example.com'\n",
        "        TOKEN = credentials('publish-token')\n",
        "    }\n",
        "    stages {\n",
        "        stage('Build') {\n",
        "            steps {\n",
        "                sh 'make'\n",
        "                sh 'make test'   // a second step\n",
        "            }\n",
        "        }\n",
        "        stage('Checks') {\n",
        "            parallel {\n",
        "                stage('Lint') {\n",
        "                    agent any\n",
        "                    when { branch 'main' }\n",
        "                    steps {\n",
        "                        sh 'make lint'\n",
        "                    }\n",
        "                }\n",
        "                stage('Audit') {\n",
        "                    steps {\n",
        "                    }\n",
        "                }\n",
        "            }\n",
        "        }\n",
        "    }\n",
        "    post {\n",
        "        always {\n",
        "            junit 'reports/*.xml'\n",
        "        }\n",
        "        failure {\n",
        "            mail to: 'team@example.com'\n",
        "        }\n",
        "    }\n",
        "}\n",
    );

    #[test]
    fn sniffs_a_declarative_pipeline() {
        assert!(JenkinsfileCore.sniff(PIPELINE.as_bytes()));
    }

    #[test]
    fn sniffs_a_scripted_pipeline() {
        assert!(
            JenkinsfileCore.sniff(b"node {\n    stage('Build') {\n        sh 'make'\n    }\n}\n")
        );
    }

    #[test]
    fn does_not_claim_groovy_that_merely_says_node() {
        assert!(!JenkinsfileCore.sniff(b"node {\n    println 'hello'\n}\n"));
        assert!(!JenkinsfileCore.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_groovy() {
        assert_eq!(JenkinsfileCore.specialises(), &["groovy"]);
    }

    #[test]
    fn a_bare_agent_takes_the_line_below_it() {
        let view = parse(PIPELINE);

        assert_eq!(view.style, "declarative");
        assert_eq!(
            view.agent.as_deref(),
            Some("label 'linux'"),
            "an `agent` block on its own says nothing; the line after it does"
        );
    }

    #[test]
    fn a_stage_agent_does_not_become_the_pipeline_agent() {
        let view = parse(PIPELINE);

        let lint = view
            .stages
            .iter()
            .find(|stage| stage.name == "Lint")
            .unwrap();
        assert_eq!(lint.agent.as_deref(), Some("any"));
        assert_eq!(view.agent.as_deref(), Some("label 'linux'"));
    }

    #[test]
    fn counts_the_steps_of_each_stage() {
        let view = parse(PIPELINE);

        assert_eq!(view.stages.len(), 4);
        assert_eq!(view.stages[0].name, "Build");
        assert_eq!(view.stages[0].steps, 2, "a trailing comment is not a step");
        assert!(!view.stages[0].parallel);
    }

    #[test]
    fn marks_the_parallel_branches_and_the_conditional_stage() {
        let view = parse(PIPELINE);

        let lint = view
            .stages
            .iter()
            .find(|stage| stage.name == "Lint")
            .unwrap();
        assert!(lint.parallel);
        assert!(lint.conditional);
        assert!(!view.stages[0].conditional);
    }

    #[test]
    fn names_the_stage_that_would_run_and_do_nothing() {
        let view = parse(PIPELINE);

        assert_eq!(
            view.stages_without_steps,
            vec!["Audit".to_owned()],
            "`Checks` has no steps either, but its parallel branches do the work"
        );
    }

    #[test]
    fn reads_the_parameters_the_environment_names_and_the_post_conditions() {
        let view = parse(PIPELINE);

        assert_eq!(
            view.parameters,
            vec![
                "BRANCH (string)".to_owned(),
                "PUBLISH (booleanParam)".to_owned()
            ]
        );
        assert_eq!(
            view.environment,
            vec!["REGISTRY".to_owned(), "TOKEN".to_owned()],
            "names only: a value can be a secret"
        );
        assert_eq!(
            view.post_conditions,
            vec!["always".to_owned(), "failure".to_owned()]
        );
    }

    #[test]
    fn reads_either_kind_of_quote() {
        assert_eq!(first_quoted("stage('Build')"), Some("Build"));
        assert_eq!(first_quoted("stage(\"Build\")"), Some("Build"));
        assert_eq!(first_quoted("stage(Build)"), None);
    }

    #[test]
    fn presents_the_empty_stage_with_its_reason() {
        let data = serde_json::to_value(parse(PIPELINE)).unwrap();

        let lines = JenkinsfilePresentation.present(&data);

        assert_eq!(lines[0], "declarative pipeline");
        assert!(lines.iter().any(|line| line.contains("run and do nothing")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("parallel, conditional"))
        );
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/jenkinsfile/Jenkinsfile");

        let data = JenkinsfileCore.view(&path).unwrap();
        let view: JenkinsfileView = serde_json::from_value(data).unwrap();

        assert_eq!(view.style, "declarative");
        assert!(view.agent.is_some());
        assert!(view.stages.len() >= 4);
        assert!(view.stages.iter().any(|stage| stage.parallel));
        assert!(view.stages.iter().any(|stage| stage.conditional));
        assert!(view.stages.iter().any(|stage| stage.agent.is_some()));
        assert!(
            view.stages_without_steps.is_empty(),
            "every stage in the main fixture does something"
        );
        assert!(view.parameters.len() >= 2);
        assert!(view.environment.len() >= 2);
        assert!(view.post_conditions.len() >= 2);
    }

    #[test]
    fn the_draft_fixture_proves_the_idle_stage() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/jenkinsfile/draft/Jenkinsfile");

        let data = JenkinsfileCore.view(&path).unwrap();
        let view: JenkinsfileView = serde_json::from_value(data).unwrap();

        assert_eq!(view.stages_without_steps, vec!["Release".to_owned()]);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::JenkinsfileCore),
            plugin_api::PluginPresentation::extensions(&crate::JenkinsfilePresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
