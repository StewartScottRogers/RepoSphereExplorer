//! `MSBuild` project file type plugin: core and presentation halves.
//!
//! A specialisation of XML: a `<Project>` root with an `Sdk` attribute,
//! or `<PropertyGroup>` and `<ItemGroup>` children, which nothing else
//! writes. It claims the project extensions itself, so `xml` never sees
//! them.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["csproj", "vbproj", "fsproj", "props", "targets"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One referenced package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageReference {
    /// The package identifier.
    pub name: String,
    /// The version asked for, or `None` when central package management
    /// supplies it.
    pub version: Option<String>,
}

/// View data produced by [`MsbuildCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MsbuildView {
    /// The software development kit the project is built with.
    pub sdk: Option<String>,
    /// The frameworks it targets, whether written singly or as a list.
    pub target_frameworks: Vec<String>,
    /// What it produces: `Library` unless it says otherwise.
    pub output_type: String,
    /// The properties set, as `name = value`.
    pub properties: Vec<String>,
    /// The packages it references.
    pub packages: Vec<PackageReference>,
    /// The other projects it references, by path.
    pub project_references: Vec<String>,
    /// The `.props` and `.targets` files it imports.
    pub imports: Vec<String>,
    /// Packages with no version here, which central package management
    /// has to supply.
    pub unversioned: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The value of `wanted` on the tag starting at `from`.
fn attribute(text: &str, from: usize, wanted: &str) -> Option<String> {
    let tag_end = text.get(from..)?.find('>')? + from;
    let tag = text.get(from..tag_end)?;
    let needle = format!("{wanted}=\"");
    let at = tag.find(&needle)? + needle.len();
    let end = tag.get(at..)?.find('"')? + at;
    Some(tag.get(at..end)?.to_owned())
}

/// Where each `<name ...>` tag starts, ignoring longer names that merely
/// begin the same way.
fn tags(text: &str, name: &str) -> Vec<usize> {
    let open = format!("<{name}");
    let mut found = Vec::new();
    let mut from = 0usize;
    while let Some(at) = text[from..].find(&open) {
        let start = from + at;
        let after = text[start + open.len()..].chars().next();
        if after.is_none_or(|c| c.is_whitespace() || c == '>' || c == '/') {
            found.push(start);
        }
        from = start + open.len();
    }
    found
}

/// The text of every `<name>...</name>`, in order.
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

/// Everything [`MsbuildView`] holds, read from `text`.
fn parse(text: &str) -> MsbuildView {
    let sdk = tags(text, "Project")
        .first()
        .and_then(|at| attribute(text, *at, "Sdk"));

    let mut target_frameworks: Vec<String> = elements(text, "TargetFrameworks")
        .into_iter()
        .flat_map(|list| {
            list.split(';')
                .map(|one| one.trim().to_owned())
                .filter(|one| !one.is_empty())
                .collect::<Vec<_>>()
        })
        .collect();
    target_frameworks.extend(elements(text, "TargetFramework"));

    // A property is any element inside a PropertyGroup. Read by name
    // rather than by position: the groups may be conditional and nested.
    let mut properties = Vec::new();
    for start in tags(text, "PropertyGroup") {
        let Some(end) = text[start..].find("</PropertyGroup>") else {
            continue;
        };
        for line in text[start..start + end].lines() {
            let trimmed = line.trim();
            let Some(name) = trimmed.strip_prefix('<').and_then(|r| r.split('>').next()) else {
                continue;
            };
            if name.starts_with('/') || name.starts_with('!') || name.contains(' ') {
                continue;
            }
            if let Some(value) = elements(trimmed, name).first() {
                properties.push(format!("{name} = {value}"));
            }
        }
    }

    let mut packages = Vec::new();
    let mut unversioned = Vec::new();
    for at in tags(text, "PackageReference") {
        let Some(name) = attribute(text, at, "Include") else {
            continue;
        };
        let version = attribute(text, at, "Version");
        if version.is_none() {
            unversioned.push(name.clone());
        }
        packages.push(PackageReference { name, version });
    }

    let project_references = tags(text, "ProjectReference")
        .into_iter()
        .filter_map(|at| attribute(text, at, "Include"))
        .collect();
    let imports = tags(text, "Import")
        .into_iter()
        .filter_map(|at| attribute(text, at, "Project"))
        .collect();

    MsbuildView {
        sdk,
        target_frameworks,
        output_type: elements(text, "OutputType")
            .first()
            .cloned()
            .unwrap_or_else(|| "Library".to_owned()),
        properties,
        packages,
        project_references,
        imports,
        unversioned,
        content: String::new(),
        truncated: false,
    }
}

/// Whether `text` is an `MSBuild` project.
fn looks_like_it(text: &str) -> bool {
    if !text.contains("<Project") {
        return false;
    }
    tags(text, "Project")
        .first()
        .and_then(|at| attribute(text, *at, "Sdk"))
        .is_some()
        || (text.contains("<PropertyGroup") && text.contains("<ItemGroup"))
        || text.contains("ToolsVersion=")
}

/// The `MSBuild` project plugin's core half.
#[derive(Debug, Default)]
pub struct MsbuildCore;

impl PluginCore for MsbuildCore {
    fn name(&self) -> &'static str {
        "msbuild"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
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

/// The `MSBuild` project plugin's presentation half.
#[derive(Debug, Default)]
pub struct MsbuildPresentation;

impl PluginPresentation for MsbuildPresentation {
    fn name(&self) -> &'static str {
        "msbuild"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "PROJ",
            tint: 0x0068_217a,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: MsbuildView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(sdk) = &view.sdk {
            lines.push(format!("Software development kit: {sdk}"));
        }
        lines.push(format!("Produces: {}", view.output_type));
        if !view.target_frameworks.is_empty() {
            lines.push(format!("Targets: {}", view.target_frameworks.join(", ")));
        }
        if !view.packages.is_empty() {
            lines.push(format!("Packages ({}):", view.packages.len()));
            for package in &view.packages {
                let version = package
                    .version
                    .as_deref()
                    .unwrap_or("version managed centrally");
                lines.push(format!("  {} {version}", package.name));
            }
        }
        if !view.unversioned.is_empty() {
            lines.push(format!(
                "No version here, so central package management supplies it: {}",
                view.unversioned.join(", ")
            ));
        }
        if !view.project_references.is_empty() {
            lines.push(format!(
                "Project references ({}):",
                view.project_references.len()
            ));
            for reference in &view.project_references {
                lines.push(format!("  {reference}"));
            }
        }
        if !view.imports.is_empty() {
            lines.push(format!("Imports: {}", view.imports.join(", ")));
        }
        if !view.properties.is_empty() {
            lines.push(format!("Properties ({}):", view.properties.len()));
            for property in &view.properties {
                lines.push(format!("  {property}"));
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
    use super::{MsbuildCore, MsbuildPresentation, MsbuildView, parse, tags};
    use plugin_api::{PluginCore, PluginPresentation};

    const PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFrameworks>net9.0;net8.0</TargetFrameworks>
    <OutputType>Exe</OutputType>
    <Nullable>enable</Nullable>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="xunit" Version="2.9.3" />
    <PackageReference Include="Serilog" />
  </ItemGroup>
  <ItemGroup>
    <ProjectReference Include="..\..\src\Warehouse.Inventory\Warehouse.Inventory.csproj" />
  </ItemGroup>
  <Import Project="../Directory.Build.props" />
</Project>
"#;

    #[test]
    fn sniffs_an_sdk_project_or_property_and_item_groups() {
        assert!(MsbuildCore.sniff(PROJECT.as_bytes()));
        assert!(
            MsbuildCore
                .sniff(b"<Project ToolsVersion=\"4.0\"><PropertyGroup/><ItemGroup/></Project>")
        );
    }

    #[test]
    fn does_not_claim_ordinary_xml() {
        assert!(!MsbuildCore.sniff(b"<catalog><book id=\"1\"/></catalog>"));
        assert!(!MsbuildCore.sniff(b""));
    }

    #[test]
    fn a_longer_tag_name_is_not_a_shorter_one() {
        // `<ProjectReference>` must not be read as `<Project>`, which is
        // what a plain substring search would do.
        assert_eq!(tags(PROJECT, "Project").len(), 1);
        assert_eq!(tags(PROJECT, "ProjectReference").len(), 1);
    }

    #[test]
    fn reads_the_kit_the_output_and_a_multi_targeted_framework_list() {
        let view = parse(PROJECT);

        assert_eq!(view.sdk.as_deref(), Some("Microsoft.NET.Sdk"));
        assert_eq!(view.output_type, "Exe");
        assert_eq!(
            view.target_frameworks,
            vec!["net9.0".to_owned(), "net8.0".to_owned()]
        );
    }

    #[test]
    fn a_package_with_no_version_is_reported_rather_than_shown_blank() {
        let view = parse(PROJECT);

        assert_eq!(view.packages.len(), 2);
        assert_eq!(view.packages[0].version.as_deref(), Some("2.9.3"));
        assert_eq!(view.unversioned, vec!["Serilog".to_owned()]);
    }

    #[test]
    fn reads_project_references_and_imports() {
        let view = parse(PROJECT);

        assert_eq!(view.project_references.len(), 1);
        assert!(view.project_references[0].ends_with("Warehouse.Inventory.csproj"));
        assert_eq!(view.imports, vec!["../Directory.Build.props".to_owned()]);
    }

    #[test]
    fn the_default_output_type_is_a_library() {
        let view = parse(r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup/></Project>"#);

        assert_eq!(view.output_type, "Library");
    }

    #[test]
    fn presents_the_kit_first() {
        let data = serde_json::to_value(parse(PROJECT)).unwrap();

        let lines = MsbuildPresentation.present(&data);

        assert_eq!(lines[0], "Software development kit: Microsoft.NET.Sdk");
        assert!(lines.iter().any(|line| line.contains("managed centrally")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/msbuild/Warehouse.Inventory.Tests.csproj");

        let data = MsbuildCore.view(&path).unwrap();
        let view: MsbuildView = serde_json::from_value(data).unwrap();

        assert!(view.sdk.is_some());
        assert!(view.target_frameworks.len() >= 2);
        assert!(view.packages.len() >= 3);
        assert!(!view.unversioned.is_empty());
        assert!(!view.project_references.is_empty());
        assert!(!view.imports.is_empty());
        assert!(view.properties.len() >= 3);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::MsbuildCore),
            plugin_api::PluginPresentation::extensions(&crate::MsbuildPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
