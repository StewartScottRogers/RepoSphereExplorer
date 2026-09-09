//! Maven POM file type plugin: core and presentation halves.
//!
//! A specialisation of XML: a `<project>` root in the Maven 4.0.0
//! namespace, or a `<modelVersion>` element, which nothing else writes.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// The text of the first `<name>...</name>` after `from`, if there is one
/// before `until`.
fn element(text: &str, name: &str, from: usize) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let at = text.get(from..)?.find(&open)? + from + open.len();
    let end = text.get(at..)?.find(&close)? + at;
    Some(text.get(at..end)?.trim().to_owned())
}

/// The text of every `<name>...</name>` in `text`, in order.
fn elements(text: &str, name: &str) -> Vec<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let mut found = Vec::new();
    let mut from = 0usize;
    while let Some(at) = text[from..].find(&open) {
        let start = from + at + open.len();
        let Some(end) = text[start..].find(&close) else {
            break;
        };
        found.push(text[start..start + end].trim().to_owned());
        from = start + end + close.len();
    }
    found
}

/// One dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    /// Its group.
    pub group: String,
    /// Its artefact.
    pub artifact: String,
    /// Its version, which may be a property reference or absent when a
    /// parent's dependency management supplies it.
    pub version: Option<String>,
    /// Its scope: `compile` unless it says otherwise.
    pub scope: String,
}

/// View data produced by [`MavenCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MavenView {
    /// The coordinates, as `group:artifact:version`.
    pub coordinates: String,
    /// What it builds: `jar` unless it says otherwise.
    pub packaging: String,
    /// The parent's coordinates, when it has one.
    pub parent: Option<String>,
    /// The modules of an aggregator.
    pub modules: Vec<String>,
    /// The dependencies.
    pub dependencies: Vec<Dependency>,
    /// The properties set, as `name = value`.
    pub properties: Vec<String>,
    /// The plugins configured, as `group:artifact`.
    pub plugins: Vec<String>,
    /// Dependencies with no version anywhere in this file, which means a
    /// parent or dependency management has to supply one.
    pub unversioned: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The text between `<dependencies>` and its close, if there is one.
fn block(text: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let at = text.find(&open)? + open.len();
    let end = text[at..].find(&close)? + at;
    Some(text[at..end].to_owned())
}

/// Reads the `<dependency>` entries out of a dependencies block.
fn dependencies_in(block: &str) -> Vec<Dependency> {
    let mut found = Vec::new();
    let mut from = 0usize;
    while let Some(at) = block[from..].find("<dependency>") {
        let start = from + at;
        let Some(end) = block[start..].find("</dependency>") else {
            break;
        };
        let entry = &block[start..start + end];
        found.push(Dependency {
            group: element(entry, "groupId", 0).unwrap_or_default(),
            artifact: element(entry, "artifactId", 0).unwrap_or_default(),
            version: element(entry, "version", 0),
            scope: element(entry, "scope", 0).unwrap_or_else(|| "compile".to_owned()),
        });
        from = start + end;
    }
    found
}

/// The slice of `text` holding the project's own direct children.
///
/// Bounded deliberately. A project that omits its group inherits the
/// parent's, so the reader has to look past `</parent>` - and the next
/// `<groupId>` after that is inside `<dependencies>`, which belongs to a
/// dependency and not to the project. Reading it as the project's own
/// named this fixture `org.junit.jupiter`.
fn project_header(text: &str) -> &str {
    let from = text
        .find("</parent>")
        .map_or(0, |at| at + "</parent>".len());
    let until = [
        "<dependencies>",
        "<dependencyManagement>",
        "<build>",
        "<modules>",
        "<profiles>",
    ]
    .iter()
    .filter_map(|marker| text.get(from..).and_then(|rest| rest.find(marker)))
    .min()
    .map_or(text.len(), |at| from + at);
    text.get(from..until).unwrap_or("")
}

/// Everything [`MavenView`] holds, read from `text`.
fn parse(text: &str) -> MavenView {
    let parent_block = block(text, "parent");
    let parent = parent_block.as_ref().map(|entry| {
        format!(
            "{}:{}:{}",
            element(entry, "groupId", 0).unwrap_or_default(),
            element(entry, "artifactId", 0).unwrap_or_default(),
            element(entry, "version", 0).unwrap_or_default()
        )
    });

    // The project's own coordinates are the first ones outside `<parent>`,
    // and a project inherits its group and version when it omits them.
    let header = project_header(text);
    let group = element(header, "groupId", 0)
        .or_else(|| parent_block.as_ref().and_then(|p| element(p, "groupId", 0)))
        .unwrap_or_default();
    let artifact = element(header, "artifactId", 0).unwrap_or_default();
    let version = element(header, "version", 0)
        .or_else(|| parent_block.as_ref().and_then(|p| element(p, "version", 0)))
        .unwrap_or_else(|| "inherited".to_owned());

    let dependencies = block(text, "dependencies")
        .as_deref()
        .map(dependencies_in)
        .unwrap_or_default();
    let unversioned = dependencies
        .iter()
        .filter(|dependency| dependency.version.is_none())
        .map(|dependency| format!("{}:{}", dependency.group, dependency.artifact))
        .collect();

    let properties = block(text, "properties")
        .map(|entry| {
            entry
                .lines()
                .filter_map(|line| {
                    let trimmed = line.trim();
                    let name = trimmed.strip_prefix('<')?.split('>').next()?;
                    if name.starts_with('/') || name.starts_with('!') {
                        return None;
                    }
                    element(trimmed, name, 0).map(|value| format!("{name} = {value}"))
                })
                .collect()
        })
        .unwrap_or_default();

    let mut plugins = Vec::new();
    if let Some(entry) = block(text, "build") {
        let mut from = 0usize;
        while let Some(at) = entry[from..].find("<plugin>") {
            let start = from + at;
            let Some(end) = entry[start..].find("</plugin>") else {
                break;
            };
            let one = &entry[start..start + end];
            let group =
                element(one, "groupId", 0).unwrap_or_else(|| "org.apache.maven.plugins".to_owned());
            if let Some(artifact) = element(one, "artifactId", 0) {
                plugins.push(format!("{group}:{artifact}"));
            }
            from = start + end;
        }
    }

    MavenView {
        coordinates: format!("{group}:{artifact}:{version}"),
        packaging: element(text, "packaging", 0).unwrap_or_else(|| "jar".to_owned()),
        parent,
        modules: elements(text, "module"),
        dependencies,
        properties,
        plugins,
        unversioned,
        content: String::new(),
        truncated: false,
    }
}

/// Whether `text` is a Maven POM.
fn looks_like_it(text: &str) -> bool {
    text.contains("<modelVersion>")
        || (text.contains("<project") && text.contains("maven.apache.org/POM"))
}

/// The Maven POM plugin's core half.
#[derive(Debug, Default)]
pub struct MavenCore;

impl PluginCore for MavenCore {
    fn name(&self) -> &'static str {
        "maven"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A specialisation of XML, which owns the extension (D13).
        &["xml"]
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

/// The Maven POM plugin's presentation half.
#[derive(Debug, Default)]
pub struct MavenPresentation;

impl PluginPresentation for MavenPresentation {
    fn name(&self) -> &'static str {
        "maven"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "POM",
            tint: 0x00c7_1a36,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: MavenView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("{} ({})", view.coordinates, view.packaging));
        if let Some(parent) = &view.parent {
            lines.push(format!("Inherits from: {parent}"));
        }
        if !view.modules.is_empty() {
            lines.push(format!("Modules ({}):", view.modules.len()));
            for module in &view.modules {
                lines.push(format!("  {module}"));
            }
        }
        if !view.dependencies.is_empty() {
            lines.push(format!("Dependencies ({}):", view.dependencies.len()));
            for dependency in &view.dependencies {
                let version = dependency.version.as_deref().unwrap_or("version inherited");
                lines.push(format!(
                    "  {}:{} {version}  [{}]",
                    dependency.group, dependency.artifact, dependency.scope
                ));
            }
        }
        if !view.unversioned.is_empty() {
            lines.push(
                "No version here, so a parent or dependency management supplies it:".to_owned(),
            );
            for name in &view.unversioned {
                lines.push(format!("  {name}"));
            }
        }
        if !view.properties.is_empty() {
            lines.push(format!("Properties: {}", view.properties.join(", ")));
        }
        if !view.plugins.is_empty() {
            lines.push(format!("Plugins: {}", view.plugins.join(", ")));
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{MavenCore, MavenPresentation, MavenView, element, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const POM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <parent>
    <groupId>com.example</groupId>
    <artifactId>platform</artifactId>
    <version>3.1.0</version>
  </parent>
  <artifactId>order-book</artifactId>
  <packaging>jar</packaging>
  <properties>
    <maven.compiler.release>21</maven.compiler.release>
    <junit.version>5.12.2</junit.version>
  </properties>
  <dependencies>
    <dependency>
      <groupId>org.junit.jupiter</groupId>
      <artifactId>junit-jupiter</artifactId>
      <version>${junit.version}</version>
      <scope>test</scope>
    </dependency>
    <dependency>
      <groupId>org.slf4j</groupId>
      <artifactId>slf4j-api</artifactId>
    </dependency>
  </dependencies>
  <build>
    <plugins>
      <plugin>
        <artifactId>maven-surefire-plugin</artifactId>
        <version>3.5.3</version>
      </plugin>
    </plugins>
  </build>
</project>
"#;

    #[test]
    fn sniffs_a_model_version_or_the_maven_namespace() {
        assert!(MavenCore.sniff(POM.as_bytes()));
        assert!(MavenCore.sniff(b"<project xmlns=\"http://maven.apache.org/POM/4.0.0\">"));
    }

    #[test]
    fn does_not_claim_ordinary_xml() {
        assert!(!MavenCore.sniff(b"<catalog><book id=\"1\"/></catalog>"));
        assert!(!MavenCore.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_xml() {
        assert_eq!(MavenCore.specialises(), &["xml"]);
    }

    #[test]
    fn a_project_inherits_the_group_and_version_it_does_not_state() {
        let view = parse(POM);

        assert_eq!(view.coordinates, "com.example:order-book:3.1.0");
        assert_eq!(
            view.parent.as_deref(),
            Some("com.example:platform:3.1.0"),
            "the parent's own coordinates stay separate"
        );
    }

    #[test]
    fn the_project_header_stops_before_the_dependencies() {
        // Looking past `</parent>` alone found `org.junit.jupiter` - the
        // first dependency's group - and called it the project's own.
        let header = super::project_header(POM);

        assert_eq!(
            element(header, "artifactId", 0).as_deref(),
            Some("order-book")
        );
        assert!(
            !header.contains("<dependencies>"),
            "the header stops before the dependencies, which is the whole point"
        );
        assert!(
            element(header, "groupId", 0).is_none(),
            "this project states no group of its own; it inherits one"
        );
        assert_eq!(element(POM, "artifactId", 0).as_deref(), Some("platform"));
    }

    #[test]
    fn reads_dependencies_with_their_scopes() {
        let view = parse(POM);

        assert_eq!(view.dependencies.len(), 2);
        assert_eq!(view.dependencies[0].scope, "test");
        assert_eq!(
            view.dependencies[1].scope, "compile",
            "the default when unstated"
        );
    }

    #[test]
    fn a_dependency_with_no_version_is_reported_rather_than_shown_blank() {
        let view = parse(POM);

        assert_eq!(view.unversioned, vec!["org.slf4j:slf4j-api".to_owned()]);
    }

    #[test]
    fn a_plugin_with_no_group_gets_mavens_own() {
        let view = parse(POM);

        assert_eq!(
            view.plugins,
            vec!["org.apache.maven.plugins:maven-surefire-plugin".to_owned()]
        );
    }

    #[test]
    fn presents_the_coordinates_first() {
        let data = serde_json::to_value(parse(POM)).unwrap();

        let lines = MavenPresentation.present(&data);

        assert_eq!(lines[0], "com.example:order-book:3.1.0 (jar)");
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/maven/pom.xml");

        let data = MavenCore.view(&path).unwrap();
        let view: MavenView = serde_json::from_value(data).unwrap();

        assert!(view.coordinates.contains(':'));
        assert_eq!(view.packaging, "pom");
        assert!(view.parent.is_some());
        assert!(view.modules.len() >= 2);
        assert!(view.dependencies.len() >= 3);
        assert!(!view.unversioned.is_empty());
        assert!(view.properties.len() >= 2);
        assert!(view.plugins.len() >= 2);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::MavenCore),
            plugin_api::PluginPresentation::extensions(&crate::MavenPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
