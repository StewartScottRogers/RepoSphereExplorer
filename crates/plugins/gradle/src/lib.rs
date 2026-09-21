//! Gradle build and settings file type plugin: core and presentation
//! halves.
//!
//! A build script (`build.gradle`, `build.gradle.kts`) is Groovy or
//! Kotlin source with a narrower shape: a `plugins {` block, a
//! `dependencies {` block, or a legacy `apply plugin:` line. A settings
//! script (`settings.gradle`, `settings.gradle.kts`) names the root
//! project and the modules it includes, which is the shape of a
//! multi-module repository - which Gradle plugins apply is what tells a
//! reader whether a checkout is an Android application, a Spring service
//! or a plain library.

use plugin_api::{Icon, PluginCore, PluginPresentation, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;
use syntax::{Language, Quote};

/// The lowercase extensions this type claims, without their dot. A build
/// script's extension - `gradle` or `kts` - already belongs to `groovy`
/// or `kotlin`; this plugin claims none of its own and wins by
/// specialising both instead (D13).
pub const EXTENSIONS: &[&str] = &[];

/// How much of a script is read. Real ones are a few dozen lines; this is
/// generous headroom rather than an expectation of reaching it.
const READ_CAP: usize = 1024 * 1024;

/// One plugin applied by a build script's `plugins {}` block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedPlugin {
    /// The plugin's id, e.g. `java-library` or `org.springframework.boot`.
    pub id: String,
    /// The version pinned alongside it, when the block states one.
    pub version: Option<String>,
}

/// One dependency declared inside a `dependencies {}` block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    /// The configuration it is declared under: `implementation`, `api`,
    /// `testImplementation` and the rest.
    pub configuration: String,
    /// Its coordinates, or a project path for a module dependency.
    pub coordinates: String,
}

/// View data produced by [`GradleCore::view`]: a build script or a
/// settings script, which hold different things and are never both at
/// once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum GradleView {
    /// A `build.gradle` or `build.gradle.kts`.
    Build {
        /// The plugins applied, with their versions where stated.
        plugins: Vec<AppliedPlugin>,
        /// The `group` coordinate.
        group: Option<String>,
        /// The `version` coordinate.
        version: Option<String>,
        /// The Java or Kotlin toolchain target, read off
        /// `JavaLanguageVersion.of(...)` or `jvmToolchain(...)`.
        toolchain: Option<String>,
        /// The dependencies, grouped by the configuration each is
        /// declared under.
        dependencies: Vec<Dependency>,
        /// The repositories declared, by the name of the call that adds
        /// each one (`mavenCentral`, `google`, a custom `maven` block).
        repositories: Vec<String>,
        /// The file's content, decoded as UTF-8 (lossily, if necessary).
        content: String,
        /// Whether the file was longer than this reads.
        truncated: bool,
    },
    /// A `settings.gradle` or `settings.gradle.kts`.
    Settings {
        /// The name from `rootProject.name`, when the script sets one.
        root_project_name: Option<String>,
        /// Every module named in an `include(...)` call.
        modules: Vec<String>,
        /// The file's content, decoded as UTF-8 (lossily, if necessary).
        content: String,
        /// Whether the file was longer than this reads.
        truncated: bool,
    },
}

/// Every `'single'`- or `"double"`-quoted string in `line`, in order and
/// unquoted.
fn quoted_strings(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        if ch == '\'' || ch == '"' {
            let quote = ch;
            let mut value = String::new();
            for next in chars.by_ref() {
                if next == quote {
                    break;
                }
                value.push(next);
            }
            found.push(value);
        }
    }
    found
}

/// The leading run of identifier characters in `line`, e.g. `implementation`
/// out of both `implementation 'x'` (Groovy) and `implementation("x")`
/// (Kotlin), which put nothing or a `(` right after the word.
fn leading_identifier(line: &str) -> Option<&str> {
    let end = line.find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))?;
    (end > 0).then(|| &line[..end])
}

/// The value of a top-level `key = "value"` or Groovy's bare `key "value"`
/// assignment, when `line` is one. `None` when `line` starts with a longer
/// identifier that merely begins with `key` (`groupId` is not `group`).
fn assignment_value(line: &str, key: &str) -> Option<String> {
    let rest = line.strip_prefix(key)?;
    if rest
        .chars()
        .next()
        .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
    {
        return None;
    }
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest).trim_start();
    quoted_strings(rest).into_iter().next()
}

/// The digits between `marker` and the `)` that follows it, e.g. `21` out
/// of `JavaLanguageVersion.of(21)`.
fn number_after(text: &str, marker: &str) -> Option<String> {
    let at = text.find(marker)? + marker.len();
    let rest = text.get(at..)?;
    let end = rest.find(')')?;
    let digits = rest[..end].trim();
    (!digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit())).then(|| digits.to_owned())
}

/// The lines of `text` that sit outside every `{ ... }` block, brace depth
/// tracked line by line: a `key {` line opens at depth zero and so is
/// still reported, but nothing between it and the matching `}` is.
fn top_level_lines(text: &str) -> Vec<&str> {
    let mut depth = 0i32;
    let mut lines = Vec::new();
    for raw in text.lines() {
        if depth == 0 {
            lines.push(raw);
        }
        for ch in raw.chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
        }
    }
    lines
}

/// The text inside the first top-level `name { ... }` block, brace-matched
/// so a block nested inside it (`java { toolchain { ... } }`) does not
/// close the outer one early. `None` when `name` never opens, or opens and
/// never closes.
fn block<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let marker = format!("{name} {{");
    let at = text.find(&marker)?;
    let start = at + marker.len();
    let mut depth = 1i32;
    for (offset, ch) in text[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..start + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Every `id ...` or `id(...)` entry in a `plugins {}` block's contents.
fn parse_plugins(block: &str) -> Vec<AppliedPlugin> {
    let mut plugins = Vec::new();
    for raw in block.lines() {
        let line = raw.trim();
        let strings = quoted_strings(line);
        let Some(id) = strings.first() else {
            continue;
        };
        let version = line.contains("version").then(|| strings.get(1)).flatten();
        plugins.push(AppliedPlugin {
            id: id.clone(),
            version: version.cloned(),
        });
    }
    plugins
}

/// Every dependency declaration in a `dependencies {}` block's contents.
fn parse_dependencies(block: &str) -> Vec<Dependency> {
    let mut dependencies = Vec::new();
    for raw in top_level_lines(block) {
        let line = raw.trim();
        let (Some(configuration), Some(coordinates)) = (
            leading_identifier(line),
            quoted_strings(line).into_iter().next(),
        ) else {
            continue;
        };
        dependencies.push(Dependency {
            configuration: configuration.to_owned(),
            coordinates,
        });
    }
    dependencies
}

/// Every repository declaration in a `repositories {}` block's contents,
/// named by the call that adds it (`mavenCentral`, `google`, a `maven`
/// block with its own `url`).
fn parse_repositories(block: &str) -> Vec<String> {
    top_level_lines(block)
        .into_iter()
        .filter_map(|raw| leading_identifier(raw.trim()))
        .map(str::to_owned)
        .collect()
}

/// Every module named by an `include(...)` or Groovy `include ...` call.
fn parse_modules(text: &str) -> Vec<String> {
    let mut modules = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("include") else {
            continue;
        };
        if rest
            .chars()
            .next()
            .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
        {
            continue;
        }
        modules.extend(quoted_strings(line));
    }
    modules
}

/// Whether every `{` in `text` is matched by a `}`, so a block truncated
/// mid-file is caught here rather than read half-parsed.
fn braces_balanced(text: &str) -> bool {
    let mut depth = 0i32;
    for ch in text.chars() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// Whether `text` reads like a Gradle build or settings script: a
/// `plugins {`, `dependencies {`, `apply plugin:` or `rootProject.name`
/// marker, with every block it opens closed again.
fn looks_like_it(text: &str) -> bool {
    braces_balanced(text)
        && (text.contains("plugins {")
            || text.contains("dependencies {")
            || text.contains("apply plugin:")
            || text.contains("rootProject.name"))
}

/// Whether `text` is a settings script rather than a build script.
fn is_settings_script(text: &str) -> bool {
    text.contains("rootProject.name")
}

/// Everything [`GradleView`] holds, read from `text`.
fn parse(text: &str, truncated: bool) -> GradleView {
    if is_settings_script(text) {
        let root_project_name = top_level_lines(text)
            .into_iter()
            .find(|line| line.contains("rootProject.name"))
            .and_then(|line| quoted_strings(line).into_iter().next());
        GradleView::Settings {
            root_project_name,
            modules: parse_modules(text),
            content: text.to_owned(),
            truncated,
        }
    } else {
        let toolchain = number_after(text, "JavaLanguageVersion.of(")
            .or_else(|| number_after(text, "jvmToolchain("))
            .map(|version| format!("Java {version}"));
        GradleView::Build {
            plugins: block(text, "plugins")
                .map(parse_plugins)
                .unwrap_or_default(),
            group: top_level_lines(text)
                .into_iter()
                .find_map(|line| assignment_value(line.trim(), "group")),
            version: top_level_lines(text)
                .into_iter()
                .find_map(|line| assignment_value(line.trim(), "version")),
            toolchain,
            dependencies: block(text, "dependencies")
                .map(parse_dependencies)
                .unwrap_or_default(),
            repositories: block(text, "repositories")
                .map(parse_repositories)
                .unwrap_or_default(),
            content: text.to_owned(),
            truncated,
        }
    }
}

/// Everything [`GradleView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<GradleView> {
    let bytes = std::fs::read(path)?;
    let truncated = bytes.len() > READ_CAP;
    let slice = &bytes[..bytes.len().min(READ_CAP)];
    let content = String::from_utf8_lossy(slice).into_owned();
    if !looks_like_it(&content) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a Gradle build or settings script",
        ));
    }
    Ok(parse(&content, truncated))
}

/// The Gradle plugin's core half.
#[derive(Debug, Default)]
pub struct GradleCore;

/// How this format is coloured, for the shared tokeniser. GUIDANCE.md §3.6:
/// the plugin describes its own format, the pane paints what it is told.
const GRADLE: Language = Language {
    line_comment: &["//"],
    block_comment: &[("/*", "*/")],
    quotes: &[Quote::simple('"'), Quote::simple('\'')],
    keywords: &[
        "plugins",
        "id",
        "version",
        "apply",
        "group",
        "dependencies",
        "implementation",
        "api",
        "testImplementation",
        "compileOnly",
        "runtimeOnly",
        "annotationProcessor",
        "repositories",
        "mavenCentral",
        "mavenLocal",
        "google",
        "rootProject",
        "include",
        "project",
        "toolchain",
        "java",
        "kotlin",
    ],
    types: &[],
    calls: false,
    ignore_case: false,
};

impl PluginCore for GradleCore {
    fn name(&self) -> &'static str {
        "gradle"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A build script is Groovy or Kotlin source, and either way it is
        // text; naming all three means the narrowing in `service` picks
        // this plugin over whichever of those also recognised the same
        // bytes, regardless of which one that happened to be (D13).
        &["text", "groovy", "kotlin"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Gradle plugin's presentation half.
#[derive(Debug, Default)]
pub struct GradlePresentation;

impl PluginPresentation for GradlePresentation {
    fn classify(&self, text: &str) -> Vec<Span> {
        syntax::classify(text, &GRADLE)
    }

    fn name(&self) -> &'static str {
        "gradle"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "GDL",
            tint: 0x0002_303a,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: GradleView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        match view {
            GradleView::Settings {
                root_project_name,
                modules,
                truncated,
                ..
            } => {
                let mut lines = vec![format!(
                    "Settings: {}",
                    root_project_name.as_deref().unwrap_or("(unnamed)")
                )];
                if !modules.is_empty() {
                    lines.push(format!("Modules ({}):", modules.len()));
                    for module in &modules {
                        lines.push(format!("  {module}"));
                    }
                }
                if truncated {
                    lines.push("Longer than this reads; what follows is the start.".to_owned());
                }
                lines
            }
            GradleView::Build {
                plugins,
                group,
                version,
                toolchain,
                dependencies,
                repositories,
                truncated,
                ..
            } => {
                let mut lines = Vec::new();
                let coordinates = match (&group, &version) {
                    (Some(group), Some(version)) => format!("{group}:{version}"),
                    (Some(group), None) => group.clone(),
                    (None, Some(version)) => version.clone(),
                    (None, None) => "(no coordinates declared)".to_owned(),
                };
                lines.push(format!("Build script: {coordinates}"));
                if let Some(toolchain) = &toolchain {
                    lines.push(format!("Toolchain: {toolchain}"));
                }
                if !plugins.is_empty() {
                    lines.push("Plugins:".to_owned());
                    for plugin in &plugins {
                        let version = plugin
                            .version
                            .as_deref()
                            .map(|version| format!(" {version}"))
                            .unwrap_or_default();
                        lines.push(format!("  {}{version}", plugin.id));
                    }
                }
                if !dependencies.is_empty() {
                    lines.push("Dependencies:".to_owned());
                    for dependency in &dependencies {
                        lines.push(format!(
                            "  [{}] {}",
                            dependency.configuration, dependency.coordinates
                        ));
                    }
                }
                if !repositories.is_empty() {
                    lines.push(format!("Repositories: {}", repositories.join(", ")));
                }
                if truncated {
                    lines.push("Longer than this reads; what follows is the start.".to_owned());
                }
                lines
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GradleCore, GradlePresentation, GradleView, looks_like_it};
    use plugin_api::{PluginCore, PluginPresentation};
    use std::path::{Path, PathBuf};

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../../samples/gradle/{name}"))
    }

    fn view_of(name: &str) -> GradleView {
        serde_json::from_value(GradleCore.view(&fixture(name)).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&GradleCore),
            PluginPresentation::extensions(&GradlePresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn it_specialises_text_groovy_and_kotlin() {
        assert_eq!(GradleCore.specialises(), &["text", "groovy", "kotlin"]);
    }

    #[test]
    fn sniffs_each_of_the_recognising_markers() {
        assert!(looks_like_it("plugins {\n    id 'java'\n}\n"));
        assert!(looks_like_it(
            "dependencies {\n    implementation 'x:y:1'\n}\n"
        ));
        assert!(looks_like_it("apply plugin: 'java'\n"));
        assert!(looks_like_it("rootProject.name = \"pipeline\"\n"));
    }

    #[test]
    fn does_not_sniff_plain_groovy_or_kotlin() {
        assert!(!looks_like_it("def greet() {\n    println 'hi'\n}\n"));
        assert!(!looks_like_it("fun main() {\n    println(\"hi\")\n}\n"));
        assert!(!looks_like_it(""));
    }

    #[test]
    fn a_block_left_open_at_the_end_is_not_a_complete_file() {
        assert!(!looks_like_it("plugins {\n    id 'java'\n"));
    }

    #[test]
    fn a_file_that_is_not_a_gradle_script_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really-build.gradle");
        std::fs::write(&path, b"nothing of the sort at all").unwrap();

        assert!(GradleCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_settings_fixture_names_the_root_and_its_modules() {
        let view = view_of("settings.gradle.kts");

        let GradleView::Settings {
            root_project_name,
            modules,
            truncated,
            ..
        } = view
        else {
            panic!("settings.gradle.kts parsed as a build script");
        };
        assert_eq!(root_project_name.as_deref(), Some("pipeline"));
        assert_eq!(modules, vec!["core".to_owned(), "worker".to_owned()]);
        assert!(!truncated);
    }

    #[test]
    fn the_root_build_fixture_fills_every_build_field() {
        let view = view_of("build.gradle.kts");

        let GradleView::Build {
            plugins,
            group,
            version,
            toolchain,
            repositories,
            truncated,
            ..
        } = view
        else {
            panic!("build.gradle.kts parsed as a settings script");
        };
        assert!(plugins.iter().any(|plugin| plugin.version.is_none()));
        assert!(plugins.iter().any(|plugin| plugin.version.is_some()));
        assert_eq!(group.as_deref(), Some("com.example.pipeline"));
        assert_eq!(version.as_deref(), Some("1.4.0"));
        assert_eq!(toolchain.as_deref(), Some("Java 21"));
        assert!(repositories.len() >= 2);
        assert!(!truncated);
    }

    #[test]
    fn the_module_build_fixture_groups_dependencies_by_configuration() {
        let view = view_of("core/build.gradle");

        let GradleView::Build { dependencies, .. } = view else {
            panic!("core/build.gradle parsed as a settings script");
        };
        assert!(
            dependencies
                .iter()
                .any(|dependency| dependency.configuration == "implementation")
        );
        assert!(
            dependencies
                .iter()
                .any(|dependency| dependency.configuration == "api")
        );
        assert!(
            dependencies
                .iter()
                .any(|dependency| dependency.configuration == "testImplementation")
        );
    }

    #[test]
    fn presents_the_settings_script() {
        let data = GradleCore.view(&fixture("settings.gradle.kts")).unwrap();

        let lines = GradlePresentation.present(&data);

        assert!(lines[0].contains("pipeline"));
        assert!(lines.iter().any(|line| line.contains("core")));
    }

    #[test]
    fn presents_the_build_script() {
        let data = GradleCore.view(&fixture("build.gradle.kts")).unwrap();

        let lines = GradlePresentation.present(&data);

        assert!(lines[0].contains("com.example.pipeline"));
        assert!(lines.iter().any(|line| line.contains("Toolchain")));
        assert!(lines.iter().any(|line| line.contains("Plugins")));
    }
}
