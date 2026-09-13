//! `NuGet` package file type plugin: core and presentation halves.
//!
//! A `NuGet` package is a zip built to the Open Packaging Conventions, so
//! it carries a `[Content_Types].xml` and a `_rels/.rels` alongside the
//! `.nuspec` that actually describes it. The interesting part is not the
//! metadata but the shape: which target frameworks it ships assemblies
//! for, which dependencies each of those needs, and whether installing it
//! changes how the consuming project builds.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::io::Read as _;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
///
/// `snupkg` is the symbols package: the same container, holding the
/// debugging symbols for the assemblies its sibling ships.
pub const EXTENSIONS: &[&str] = &["nupkg", "snupkg"];

/// The four bytes a zip's first local header opens with.
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// The entry every Open Packaging Conventions container carries.
const CONTENT_TYPES: &[u8] = b"[Content_Types].xml";

/// What every package's own description is named, by extension.
const NUSPEC: &str = ".nuspec";

/// One dependency, as a `.nuspec` names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    /// The package it needs.
    pub id: String,
    /// The versions of it that will do, in `NuGet`'s range notation.
    pub version: String,
}

/// The dependencies one target framework needs.
///
/// Grouping is the whole point: the same package can need different
/// things depending on what it is being installed into.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyGroup {
    /// The framework, as the `.nuspec` spells it.
    pub target_framework: String,
    /// What that framework needs.
    pub dependencies: Vec<Dependency>,
}

/// View data produced by [`NugetCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NugetView {
    /// The package identifier.
    pub id: String,
    /// Its version.
    pub version: String,
    /// The display title, when it differs from the identifier.
    pub title: Option<String>,
    /// Who wrote it.
    pub authors: Option<String>,
    /// Who publishes it.
    pub owners: Option<String>,
    /// What it is for.
    pub description: Option<String>,
    /// What changed in this version.
    pub release_notes: Option<String>,
    /// Where it lives.
    pub project_url: Option<String>,
    /// The licence, and whether it is named as an expression or as a
    /// file inside the package.
    pub licence: Option<String>,
    /// The copyright line.
    pub copyright: Option<String>,
    /// The tags a gallery searches on.
    pub tags: Vec<String>,
    /// The source repository, as kind and address.
    pub repository: Option<String>,
    /// Whether installing it puts a licence in front of somebody.
    pub requires_licence_acceptance: bool,
    /// The dependencies, grouped by the framework that needs them.
    pub dependency_groups: Vec<DependencyGroup>,
    /// The frameworks `lib/` ships assemblies for.
    pub target_frameworks: Vec<String>,
    /// Those assemblies, by path.
    pub assemblies: Vec<String>,
    /// Anything under `build/`, which changes how the consuming project
    /// builds the moment the package is installed.
    pub build_files: Vec<String>,
    /// Anything under `tools/`, which is a script the package runs.
    pub tool_files: Vec<String>,
    /// How many entries the zip holds.
    pub entries: usize,
}

/// Whether `haystack` holds `needle` anywhere as a contiguous byte run.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Whether `prefix` opens like a `NuGet` package.
///
/// A package is a zip, so the signature settles nothing. The two markers
/// together do: `[Content_Types].xml` says this is an Open Packaging
/// Conventions container, which a plain zip is not, and the `.nuspec`
/// says it is this kind of one rather than a Word document.
fn looks_like_it(prefix: &[u8]) -> bool {
    prefix.starts_with(ZIP_MAGIC)
        && contains_bytes(prefix, CONTENT_TYPES)
        && contains_bytes(prefix, NUSPEC.as_bytes())
}

/// Every `<name ...>...</name>` or `<name ... />` in `xml`, as the whole
/// element including its tags.
///
/// Naive about an element of the same name nested inside itself, which
/// nothing in a `.nuspec` does: `group` holds `dependency`, never
/// another `group`.
fn elements<'xml>(xml: &'xml str, name: &str) -> Vec<&'xml str> {
    let open = format!("<{name}");
    let close = format!("</{name}>");
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find(&open) {
        let after = &rest[at + open.len()..];
        // `<dependencies>` must not match a search for `<dependency`.
        if !after.starts_with([' ', '>', '/', '\t', '\r', '\n']) {
            rest = &rest[at + open.len()..];
            continue;
        }
        let Some(head_end) = after.find('>') else {
            break;
        };
        if after[..head_end].ends_with('/') {
            found.push(&rest[at..=at + open.len() + head_end]);
            rest = &after[head_end + 1..];
            continue;
        }
        let body = &after[head_end + 1..];
        let Some(end) = body.find(&close) else { break };
        found.push(&rest[at..at + open.len() + head_end + 1 + end + close.len()]);
        rest = &body[end + close.len()..];
    }
    found
}

/// The text an element holds, between its opening and closing tags.
fn text_in(element: &str) -> &str {
    let Some(start) = element.find('>') else {
        return "";
    };
    let body = &element[start + 1..];
    body.rfind('<').map_or(body, |end| &body[..end]).trim()
}

/// The value of the attribute `name` on an element's opening tag.
fn attribute<'element>(element: &'element str, name: &str) -> Option<&'element str> {
    let head = &element[..element.find('>').unwrap_or(element.len())];
    let wanted = format!("{name}=\"");
    let at = head.find(&wanted)? + wanted.len();
    let rest = &head[at..];
    Some(&rest[..rest.find('"')?])
}

/// The text of the first `<name>` element, if there is one and it is not
/// empty.
fn field(xml: &str, name: &str) -> Option<String> {
    elements(xml, name)
        .first()
        .map(|element| text_in(element).to_owned())
        .filter(|text| !text.is_empty())
}

/// The dependency groups a `.nuspec` declares.
///
/// A `.nuspec` may list dependencies either grouped by framework or as a
/// flat list that applies to everything; the flat form is reported as one
/// group with no framework, because that is what it means.
fn dependency_groups_in(nuspec: &str) -> Vec<DependencyGroup> {
    let Some(block) = elements(nuspec, "dependencies").into_iter().next() else {
        return Vec::new();
    };
    let read = |source: &str| -> Vec<Dependency> {
        elements(source, "dependency")
            .iter()
            .filter_map(|element| {
                Some(Dependency {
                    id: attribute(element, "id")?.to_owned(),
                    version: attribute(element, "version").unwrap_or("any").to_owned(),
                })
            })
            .collect()
    };
    let groups = elements(block, "group");
    if groups.is_empty() {
        let flat = read(block);
        if flat.is_empty() {
            return Vec::new();
        }
        return vec![DependencyGroup {
            target_framework: "any framework".to_owned(),
            dependencies: flat,
        }];
    }
    groups
        .iter()
        .map(|group| DependencyGroup {
            target_framework: attribute(group, "targetFramework")
                .unwrap_or("any framework")
                .to_owned(),
            dependencies: read(group),
        })
        .collect()
}

/// Everything [`NugetView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<NugetView> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let names: Vec<String> = archive.file_names().map(ToOwned::to_owned).collect();

    let Some(nuspec_name) = names
        .iter()
        .find(|name| name.ends_with(NUSPEC) && !name.contains('/'))
        .cloned()
    else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no .nuspec at the root, so this is a zip and not a NuGet package",
        ));
    };
    let mut nuspec = String::new();
    archive
        .by_name(&nuspec_name)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
        .read_to_string(&mut nuspec)?;

    let licence = elements(&nuspec, "license").first().map(|element| {
        let kind = attribute(element, "type").unwrap_or("expression");
        let said = text_in(element);
        if kind == "file" {
            format!("{said}, a file inside the package")
        } else {
            said.to_owned()
        }
    });
    let repository = elements(&nuspec, "repository").first().and_then(|element| {
        let url = attribute(element, "url")?;
        Some(match attribute(element, "type") {
            Some(kind) => format!("{url} ({kind})"),
            None => url.to_owned(),
        })
    });

    let under = |directory: &str| -> Vec<String> {
        names
            .iter()
            .filter(|name| name.starts_with(directory) && !name.ends_with('/'))
            .cloned()
            .collect()
    };
    let assemblies = under("lib/");
    let mut target_frameworks: Vec<String> = Vec::new();
    for name in &assemblies {
        if let Some(rest) = name.strip_prefix("lib/")
            && let Some((framework, _)) = rest.split_once('/')
            && !target_frameworks.iter().any(|had| had == framework)
        {
            target_frameworks.push(framework.to_owned());
        }
    }

    Ok(NugetView {
        id: field(&nuspec, "id").unwrap_or_default(),
        version: field(&nuspec, "version").unwrap_or_default(),
        title: field(&nuspec, "title"),
        authors: field(&nuspec, "authors"),
        owners: field(&nuspec, "owners"),
        description: field(&nuspec, "description"),
        release_notes: field(&nuspec, "releaseNotes"),
        project_url: field(&nuspec, "projectUrl"),
        licence,
        copyright: field(&nuspec, "copyright"),
        tags: field(&nuspec, "tags")
            .map(|tags| tags.split_whitespace().map(ToOwned::to_owned).collect())
            .unwrap_or_default(),
        repository,
        requires_licence_acceptance: field(&nuspec, "requireLicenseAcceptance")
            .is_some_and(|said| said.eq_ignore_ascii_case("true")),
        dependency_groups: dependency_groups_in(&nuspec),
        target_frameworks,
        assemblies,
        build_files: under("build/"),
        tool_files: under("tools/"),
        entries: names.len(),
    })
}

/// The `NuGet` package plugin's core half.
#[derive(Debug, Default)]
pub struct NugetCore;

impl PluginCore for NugetCore {
    fn name(&self) -> &'static str {
        "nuget"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A package is a zip, which `archive` recognises. This is the
        // narrower reading of the same bytes (D13).
        &["archive"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_it(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The `NuGet` package plugin's presentation half.
#[derive(Debug, Default)]
pub struct NugetPresentation;

impl PluginPresentation for NugetPresentation {
    fn name(&self) -> &'static str {
        "nuget"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "NPKG",
            tint: 0x0000_4880,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: NugetView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "NuGet package {} {}, {} entry(ies)",
            view.id, view.version, view.entries
        )];
        if let Some(title) = &view.title
            && title != &view.id
        {
            lines.push(title.clone());
        }
        if let Some(description) = &view.description {
            lines.push(description.clone());
        }
        if let Some(authors) = &view.authors {
            lines.push(format!("By {authors}"));
        }
        if let Some(owners) = &view.owners {
            lines.push(format!("Published by {owners}"));
        }
        if let Some(licence) = &view.licence {
            lines.push(format!("Licence {licence}"));
        }
        lines.push(if view.requires_licence_acceptance {
            "Installing it puts the licence in front of somebody first.".to_owned()
        } else {
            "Installs without asking anybody to accept a licence.".to_owned()
        });
        if let Some(url) = &view.project_url {
            lines.push(url.clone());
        }
        if let Some(repository) = &view.repository {
            lines.push(format!("Source {repository}"));
        }
        if let Some(copyright) = &view.copyright {
            lines.push(copyright.clone());
        }
        if let Some(notes) = &view.release_notes {
            lines.push(format!("This version: {notes}"));
        }
        if !view.tags.is_empty() {
            lines.push(format!("Tags: {}", view.tags.join(", ")));
        }
        if view.target_frameworks.is_empty() {
            lines.push("Ships no assemblies of its own.".to_owned());
        } else {
            lines.push(format!(
                "Ships assemblies for {}",
                view.target_frameworks.join(", ")
            ));
            for assembly in &view.assemblies {
                lines.push(format!("  {assembly}"));
            }
        }
        for group in &view.dependency_groups {
            if group.dependencies.is_empty() {
                lines.push(format!("{} needs nothing else", group.target_framework));
                continue;
            }
            lines.push(format!("{} needs:", group.target_framework));
            for dependency in &group.dependencies {
                lines.push(format!("  {} {}", dependency.id, dependency.version));
            }
        }
        if !view.build_files.is_empty() {
            lines.push("Changes how the consuming project builds:".to_owned());
            for file in &view.build_files {
                lines.push(format!("  {file}"));
            }
        }
        if !view.tool_files.is_empty() {
            lines.push("Runs on install:".to_owned());
            for file in &view.tool_files {
                lines.push(format!("  {file}"));
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NugetCore, NugetPresentation, NugetView, attribute, dependency_groups_in, elements,
        looks_like_it, text_in,
    };
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/nuget/CsvStats.1.0.3.nupkg")
    }

    fn view_of() -> NugetView {
        serde_json::from_value(NugetCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&NugetCore),
            PluginPresentation::extensions(&NugetPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn sniffs_a_packaging_conventions_zip_holding_a_nuspec() {
        assert!(looks_like_it(
            b"PK\x03\x04..[Content_Types].xml..CsvStats.nuspec.."
        ));
        assert!(
            !looks_like_it(b"PK\x03\x04..[Content_Types].xml..word/document.xml.."),
            "an Open Packaging Conventions container is not necessarily this one"
        );
        assert!(
            !looks_like_it(b"PK\x03\x04..CsvStats.nuspec.."),
            "a .nuspec loose in a zip is not a package"
        );
        assert!(!looks_like_it(b""));
    }

    #[test]
    fn it_says_it_specialises_the_archive_reading() {
        assert_eq!(NugetCore.specialises(), &["archive"]);
    }

    #[test]
    fn an_element_search_does_not_match_a_longer_name() {
        let xml = "<dependencies><dependency id=\"a\" version=\"1\" /></dependencies>";

        let found = elements(xml, "dependency");

        assert_eq!(found.len(), 1, "`<dependencies>` is not a `<dependency>`");
        assert_eq!(attribute(found[0], "id"), Some("a"));
        assert_eq!(text_in("<x>  hello  </x>"), "hello");
    }

    #[test]
    fn a_flat_dependency_list_reads_as_one_group_for_any_framework() {
        let nuspec = "<dependencies><dependency id=\"a\" version=\"1\" /></dependencies>";

        let groups = dependency_groups_in(nuspec);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].target_framework, "any framework");
        assert_eq!(groups[0].dependencies[0].id, "a");
    }

    #[test]
    fn reads_the_identity_and_the_prose() {
        let view = view_of();

        assert_eq!(view.id, "CsvStats");
        assert_eq!(view.version, "1.0.3");
        assert_eq!(view.title.as_deref(), Some("CsvStats"));
        assert_eq!(view.authors.as_deref(), Some("The floor"));
        assert_eq!(view.owners.as_deref(), Some("The floor"));
        assert_eq!(
            view.description.as_deref(),
            Some("Summary statistics for a column of readings.")
        );
        assert!(view.release_notes.is_some());
        assert_eq!(view.copyright.as_deref(), Some("Copyright the floor"));
        assert_eq!(view.tags, vec!["csv", "statistics", "readings"]);
    }

    #[test]
    fn reads_the_licence_the_urls_and_the_acceptance_flag() {
        let view = view_of();

        assert_eq!(view.licence.as_deref(), Some("MIT"));
        assert_eq!(
            view.project_url.as_deref(),
            Some("https://example.com/floor/csvstats")
        );
        assert_eq!(
            view.repository.as_deref(),
            Some("https://example.com/floor/csvstats.git (git)")
        );
        assert!(!view.requires_licence_acceptance);
    }

    #[test]
    fn keeps_each_target_framework_dependencies_apart() {
        let view = view_of();

        assert_eq!(view.dependency_groups.len(), 2);
        assert_eq!(view.dependency_groups[0].target_framework, "net8.0");
        assert_eq!(view.dependency_groups[0].dependencies.len(), 1);
        assert_eq!(
            view.dependency_groups[1].target_framework,
            ".NETStandard2.0"
        );
        assert_eq!(
            view.dependency_groups[1]
                .dependencies
                .iter()
                .map(|one| one.id.as_str())
                .collect::<Vec<&str>>(),
            vec!["System.Text.Json", "System.Memory"],
            "the older framework needs a package the newer one has built in"
        );
        assert_eq!(view.dependency_groups[1].dependencies[1].version, "4.5.5");
    }

    #[test]
    fn reads_the_assemblies_the_build_files_and_the_tools() {
        let view = view_of();

        assert_eq!(view.target_frameworks, vec!["net8.0", "netstandard2.0"]);
        assert!(
            view.assemblies
                .iter()
                .any(|one| one == "lib/net8.0/CsvStats.dll")
        );
        assert_eq!(view.build_files, vec!["build/CsvStats.targets"]);
        assert_eq!(view.tool_files, vec!["tools/install.ps1"]);
        assert!(view.entries >= 9);
    }

    #[test]
    fn presents_the_groups_and_what_installing_it_changes() {
        let data = NugetCore.view(&fixture()).unwrap();

        let lines = NugetPresentation.present(&data);

        assert!(lines[0].starts_with("NuGet package CsvStats 1.0.3"));
        assert!(lines.iter().any(|line| line.contains("net8.0 needs:")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains(".NETStandard2.0 needs:"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Changes how the consuming project builds"))
        );
        assert!(lines.iter().any(|line| line.contains("Runs on install")));
    }

    #[test]
    fn a_zip_that_is_not_a_package_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really-a.nupkg");
        std::fs::write(&path, b"PK\x03\x04 and then nothing of the sort").unwrap();

        assert!(NugetCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
