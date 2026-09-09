//! Helm chart metadata file type plugin: core and presentation halves.
//!
//! A specialisation of YAML. `apiVersion`, `name` and `version` together,
//! with no `kind`, is a chart: a Kubernetes manifest carries `kind` and is
//! claimed by its own plugin first.

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

/// One chart this one depends on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    /// The dependency's chart name.
    pub name: String,
    /// The version range it asks for.
    pub version: Option<String>,
    /// Where it comes from.
    pub repository: Option<String>,
    /// The value that has to be true for it to be installed at all.
    pub condition: Option<String>,
}

/// View data produced by [`HelmchartCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelmchartView {
    /// The chart's name.
    pub name: Option<String>,
    /// The chart's own version.
    pub version: Option<String>,
    /// The version of the application it installs, which moves separately.
    pub app_version: Option<String>,
    /// `application` or `library`.
    pub kind: Option<String>,
    /// The chart API version: `v1` or `v2`.
    pub api_version: Option<String>,
    /// Its one-line description.
    pub description: Option<String>,
    /// The charts it depends on.
    pub dependencies: Vec<Dependency>,
    /// Its keywords.
    pub keywords: Vec<String>,
    /// Its maintainers, by name.
    pub maintainers: Vec<String>,
    /// Whether it is marked deprecated.
    pub deprecated: bool,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The `- item` entries directly inside the block starting at `from`.
fn list_under(lines: &[&str], from: usize) -> Vec<String> {
    let base = indent(lines[from]);
    let mut found = Vec::new();
    for line in lines.iter().skip(from + 1) {
        if line.trim().is_empty() {
            continue;
        }
        if indent(line) <= base {
            break;
        }
        if let Some(rest) = line.trim_start().strip_prefix("- ") {
            found.push(rest.trim().trim_matches(['"', '\'']).to_owned());
        }
    }
    found
}

/// The `- name:` entries directly inside the `maintainers:` block.
fn maintainers_under(lines: &[&str], from: usize) -> Vec<String> {
    let base = indent(lines[from]);
    let mut found = Vec::new();
    for candidate in lines.iter().skip(from + 1) {
        if candidate.trim().is_empty() {
            continue;
        }
        if indent(candidate) <= base {
            break;
        }
        if let Some(rest) = candidate.trim_start().strip_prefix("- name:") {
            found.push(rest.trim().trim_matches(['"', '\'']).to_owned());
        }
    }
    found
}

/// The entries of the `dependencies:` block, each opened by `- name:`.
fn dependencies_under(lines: &[&str], from: usize) -> Vec<Dependency> {
    let base = indent(lines[from]);
    let mut found = Vec::new();
    let mut current: Option<Dependency> = None;
    for candidate in lines.iter().skip(from + 1) {
        if candidate.trim().is_empty() {
            continue;
        }
        if indent(candidate) <= base {
            break;
        }
        if let Some(rest) = candidate.trim_start().strip_prefix("- name:") {
            if let Some(done) = current.take() {
                found.push(done);
            }
            current = Some(Dependency {
                name: rest.trim().trim_matches(['"', '\'']).to_owned(),
                version: None,
                repository: None,
                condition: None,
            });
            continue;
        }
        let Some(entry) = current.as_mut() else {
            continue;
        };
        match key_of(candidate) {
            Some("version") => entry.version = value_of(candidate),
            Some("repository") => entry.repository = value_of(candidate),
            Some("condition") => entry.condition = value_of(candidate),
            _ => {}
        }
    }
    if let Some(done) = current {
        found.push(done);
    }
    found
}

/// Everything [`HelmchartView`] holds, read from `text`.
fn parse(text: &str) -> HelmchartView {
    let lines: Vec<&str> = text.lines().collect();
    let mut view = HelmchartView {
        name: None,
        version: None,
        app_version: None,
        kind: None,
        api_version: None,
        description: None,
        dependencies: Vec::new(),
        keywords: Vec::new(),
        maintainers: Vec::new(),
        deprecated: false,
        content: String::new(),
        truncated: false,
    };

    for (index, line) in lines.iter().enumerate() {
        if indent(line) != 0 {
            continue;
        }
        match key_of(line) {
            Some("name") => view.name = value_of(line),
            Some("version") => view.version = value_of(line),
            Some("appVersion") => view.app_version = value_of(line),
            Some("type") => view.kind = value_of(line),
            Some("apiVersion") => view.api_version = value_of(line),
            Some("description") => view.description = value_of(line),
            Some("deprecated") => {
                view.deprecated = value_of(line).as_deref() == Some("true");
            }
            Some("keywords") => {
                view.keywords = value_of(line).map_or_else(
                    || list_under(&lines, index),
                    |flow| {
                        flow.trim_matches(['[', ']'])
                            .split(',')
                            .map(|one| one.trim().trim_matches(['"', '\'']).to_owned())
                            .filter(|one| !one.is_empty())
                            .collect()
                    },
                );
            }
            Some("maintainers") => view.maintainers = maintainers_under(&lines, index),
            Some("dependencies") => view.dependencies = dependencies_under(&lines, index),
            _ => {}
        }
    }
    view
}

/// Whether `text` is a Helm chart's metadata.
fn looks_like_it(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let top = |wanted: &str| {
        lines
            .iter()
            .any(|line| indent(line) == 0 && key_of(line) == Some(wanted))
    };
    // `kind` would make it a Kubernetes manifest, which has its own plugin.
    top("apiVersion") && top("name") && top("version") && !top("kind")
}

/// The Helm chart metadata plugin's core half.
#[derive(Debug, Default)]
pub struct HelmchartCore;

impl PluginCore for HelmchartCore {
    fn name(&self) -> &'static str {
        "helmchart"
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

/// The Helm chart metadata plugin's presentation half.
#[derive(Debug, Default)]
pub struct HelmchartPresentation;

impl PluginPresentation for HelmchartPresentation {
    fn name(&self) -> &'static str {
        "helmchart"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "HELM",
            tint: 0x000f_1689,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: HelmchartView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(name) = &view.name {
            let version = view.version.as_deref().unwrap_or("no version");
            lines.push(format!("Chart: {name} {version}"));
        }
        if let Some(app) = &view.app_version {
            lines.push(format!("Installs application version {app}"));
        }
        if view.deprecated {
            lines.push("DEPRECATED: this chart is marked deprecated.".to_owned());
        }
        if let Some(description) = &view.description {
            lines.push(description.clone());
        }
        if let (Some(api), Some(kind)) = (&view.api_version, &view.kind) {
            lines.push(format!("Chart API {api}, type {kind}"));
        } else if let Some(api) = &view.api_version {
            lines.push(format!("Chart API {api}"));
        }
        if !view.dependencies.is_empty() {
            lines.push(format!("Dependencies ({}):", view.dependencies.len()));
            for dependency in &view.dependencies {
                let version = dependency.version.as_deref().unwrap_or("any version");
                let repository = dependency
                    .repository
                    .as_ref()
                    .map_or_else(String::new, |from| format!(" from {from}"));
                lines.push(format!("  {} {version}{repository}", dependency.name));
                if let Some(condition) = &dependency.condition {
                    lines.push(format!("      only when {condition}"));
                }
            }
        }
        if !view.keywords.is_empty() {
            lines.push(format!("Keywords: {}", view.keywords.join(", ")));
        }
        if !view.maintainers.is_empty() {
            lines.push(format!("Maintainers: {}", view.maintainers.join(", ")));
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{HelmchartCore, HelmchartPresentation, HelmchartView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const CHART: &str = "apiVersion: v2\nname: repo-sphere\nversion: 1.4.0\n\
        appVersion: \"0.6.0\"\ntype: application\n\
        description: A front door to your development workspace\n\
        keywords: [explorer, repositories]\n\
        maintainers:\n  - name: Example Engineering\n    email: eng@example.com\n\
        dependencies:\n  - name: postgresql\n    version: \"15.5.x\"\n\
        \x20   repository: https://charts.bitnami.com/bitnami\n    condition: postgresql.enabled\n\
        \x20 - name: redis\n    version: \"19.x\"\n\
        \x20   repository: https://charts.bitnami.com/bitnami\n";

    #[test]
    fn sniffs_a_chart() {
        assert!(HelmchartCore.sniff(CHART.as_bytes()));
    }

    #[test]
    fn a_manifest_with_a_kind_is_not_a_chart() {
        // Kubernetes has its own plugin, and a `kind` is how you tell.
        assert!(!HelmchartCore.sniff(b"apiVersion: v1\nkind: Service\nname: a\nversion: 1\n"));
        assert!(!HelmchartCore.sniff(b"name: a\nversion: 1\n"));
        assert!(!HelmchartCore.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_yaml() {
        assert_eq!(HelmchartCore.specialises(), &["yaml"]);
    }

    #[test]
    fn the_chart_version_and_the_application_version_are_different_things() {
        let view = parse(CHART);

        assert_eq!(view.version.as_deref(), Some("1.4.0"));
        assert_eq!(view.app_version.as_deref(), Some("0.6.0"));
    }

    #[test]
    fn reads_each_dependency_with_its_condition() {
        let view = parse(CHART);

        assert_eq!(view.dependencies.len(), 2);
        assert_eq!(view.dependencies[0].name, "postgresql");
        assert_eq!(
            view.dependencies[0].condition.as_deref(),
            Some("postgresql.enabled")
        );
        assert!(view.dependencies[1].condition.is_none());
    }

    #[test]
    fn reads_keywords_in_flow_style_and_maintainers_in_block_style() {
        let view = parse(CHART);

        assert_eq!(
            view.keywords,
            vec!["explorer".to_owned(), "repositories".to_owned()]
        );
        assert_eq!(view.maintainers, vec!["Example Engineering".to_owned()]);
    }

    #[test]
    fn a_deprecated_chart_says_so_first() {
        let data = serde_json::to_value(parse(
            "apiVersion: v2\nname: old\nversion: 1.0.0\ndeprecated: true\n",
        ))
        .unwrap();

        let lines = HelmchartPresentation.present(&data);

        assert!(lines.iter().any(|line| line.starts_with("DEPRECATED")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/helmchart/Chart.yaml");

        let data = HelmchartCore.view(&path).unwrap();
        let view: HelmchartView = serde_json::from_value(data).unwrap();

        assert!(view.name.is_some() && view.version.is_some());
        assert!(view.app_version.is_some());
        assert!(view.kind.is_some() && view.api_version.is_some());
        assert!(view.description.is_some());
        assert!(view.dependencies.len() >= 2);
        assert!(view.dependencies.iter().any(|d| d.condition.is_some()));
        assert!(!view.keywords.is_empty());
        assert!(!view.maintainers.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::HelmchartCore),
            plugin_api::PluginPresentation::extensions(&crate::HelmchartPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
