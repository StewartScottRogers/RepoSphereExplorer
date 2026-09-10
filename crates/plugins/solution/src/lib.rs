//! Visual Studio solution file type plugin: core and presentation halves.
//!
//! A solution file lists what a Visual Studio workspace is made of. This
//! reads the format version, every project with its kind and path, the
//! solution folders, the build configurations, and the projects a build
//! passes over because they are selected but not built.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["sln"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One entry from the solution's `Project(...)` lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolutionProject {
    /// The name shown in the solution tree.
    pub name: String,
    /// Its path relative to the solution, or its own name when it is a
    /// solution folder, which has no path of its own.
    pub path: String,
    /// The type identifier, which says what kind of project it is.
    pub type_id: String,
    /// That identifier in words, when it is one we know.
    pub kind: String,
    /// The project's own identifier, which the configuration block refers
    /// to it by.
    pub id: String,
    /// The solution configurations this project actually builds in.
    pub builds_in: Vec<String>,
}

/// View data produced by [`SolutionCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolutionView {
    /// The solution file format version from the banner.
    pub format_version: String,
    /// The Visual Studio version that last wrote it, when it says.
    pub visual_studio_version: Option<String>,
    /// Every project, solution folders included.
    pub projects: Vec<SolutionProject>,
    /// The names of the solution folders, which hold no code.
    pub folders: Vec<String>,
    /// The solution-wide build configurations.
    pub configurations: Vec<String>,
    /// Project and configuration pairs that are selected but not built -
    /// the setting that makes a build quietly skip a project.
    pub selected_but_not_built: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The type identifier a solution folder always carries.
const FOLDER_TYPE: &str = "{2150E333-8FDC-42A3-9474-1A3956D46DE8}";

/// The project type identifiers worth naming, upper-cased.
const KINDS: &[(&str, &str)] = &[
    ("{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}", "C#"),
    ("{9A19103F-16F7-4668-BE54-9A1E7A4F7556}", "C#, SDK-style"),
    ("{F184B08F-C81C-45F6-A57F-5ABD9991F28F}", "Visual Basic"),
    (
        "{778DAE3C-4631-46EA-AA77-85C1314464D9}",
        "Visual Basic, SDK-style",
    ),
    ("{8BC9CEB8-8B4A-11D0-8D11-00A0C91BC942}", "C++"),
    ("{F2A71F9B-5D33-465A-A702-920D77279786}", "F#"),
    ("{6EC3EE1D-3C4E-46DD-8F32-0CC8E7565705}", "F#, SDK-style"),
    ("{A9ACE9BB-CECE-4E62-9AA4-C7E7C5BD2124}", "database"),
    ("{E24C65DC-7377-472B-9ABA-BC803B73C61A}", "web site"),
    (FOLDER_TYPE, "solution folder"),
];

/// What `type_id` means, or the identifier itself when we do not know.
fn kind_of(type_id: &str) -> String {
    let upper = type_id.to_ascii_uppercase();
    KINDS
        .iter()
        .find(|(identifier, _)| *identifier == upper)
        .map_or_else(
            || "unrecognised type".to_owned(),
            |(_, kind)| (*kind).to_owned(),
        )
}

/// The three quoted fields of a `Project(...) = ...` line.
///
/// The line is `Project("{type}") = "name", "path", "{id}"`, so the type
/// identifier is the first quoted run and the rest follow it in order.
fn quoted_fields(line: &str) -> Vec<String> {
    line.split('"')
        .skip(1)
        .step_by(2)
        .map(str::trim)
        .map(str::to_owned)
        .collect()
}

/// Reads one `Project(...)` line into `view`.
fn read_project(line: &str, view: &mut SolutionView) {
    let fields = quoted_fields(line);
    let [type_id, name, path, id] = fields.as_slice() else {
        return;
    };
    if type_id.to_ascii_uppercase() == FOLDER_TYPE {
        view.folders.push(name.clone());
    }
    view.projects.push(SolutionProject {
        name: name.clone(),
        path: path.clone(),
        type_id: type_id.clone(),
        kind: kind_of(type_id),
        id: id.clone(),
        builds_in: Vec::new(),
    });
}

/// Reads one line of the `ProjectConfigurationPlatforms` section.
///
/// Each project gets two lines per configuration: `ActiveCfg` says which
/// configuration it uses, `Build.0` says it is built at all. A project
/// with the first and not the second is in the solution and skipped by
/// the build, which is a thing a reader has to go looking for otherwise.
fn read_project_configuration(line: &str, selected: &mut Vec<String>, built: &mut Vec<String>) {
    let Some((left, _)) = line.trim().split_once('=') else {
        return;
    };
    let left = left.trim();
    let Some((id, rest)) = left.split_once('.') else {
        return;
    };
    // `Build.0` carries a dot of its own, so the *last* dot is not the one
    // that ends the configuration name.
    if let Some(configuration) = rest.strip_suffix(".ActiveCfg") {
        selected.push(format!("{id}|{configuration}"));
    } else if let Some((configuration, _)) = rest.split_once(".Build.") {
        built.push(format!("{id}|{configuration}"));
    }
}

/// Everything [`SolutionView`] holds, read from `text`.
fn parse(text: &str) -> SolutionView {
    let mut view = SolutionView {
        format_version: "unstated".to_owned(),
        visual_studio_version: None,
        projects: Vec::new(),
        folders: Vec::new(),
        configurations: Vec::new(),
        selected_but_not_built: Vec::new(),
        truncated: false,
    };
    let mut section = String::new();
    let mut selected: Vec<String> = Vec::new();
    let mut built: Vec<String> = Vec::new();

    for line in text.lines() {
        // Visual Studio writes a byte order mark, and `trim` leaves it
        // where it is: it is not whitespace.
        let trimmed = line.trim().trim_start_matches('\u{feff}');
        if let Some(version) =
            trimmed.strip_prefix("Microsoft Visual Studio Solution File, Format Version ")
        {
            version.trim().clone_into(&mut view.format_version);
        } else if let Some(version) = trimmed.strip_prefix("VisualStudioVersion = ") {
            view.visual_studio_version = Some(version.trim().to_owned());
        } else if trimmed.starts_with("Project(") {
            read_project(trimmed, &mut view);
        } else if let Some(rest) = trimmed.strip_prefix("GlobalSection(") {
            rest.split(')')
                .next()
                .unwrap_or_default()
                .clone_into(&mut section);
        } else if trimmed == "EndGlobalSection" {
            section.clear();
        } else if section == "SolutionConfigurationPlatforms" {
            if let Some((configuration, _)) = trimmed.split_once('=') {
                view.configurations.push(configuration.trim().to_owned());
            }
        } else if section == "ProjectConfigurationPlatforms" {
            read_project_configuration(trimmed, &mut selected, &mut built);
        }
    }

    for pair in &selected {
        if built.contains(pair) {
            continue;
        }
        let Some((id, configuration)) = pair.split_once('|') else {
            continue;
        };
        let name = view
            .projects
            .iter()
            .find(|project| project.id.eq_ignore_ascii_case(id))
            .map_or(id, |project| project.name.as_str());
        view.selected_but_not_built
            .push(format!("{name} in {configuration}"));
    }
    for pair in &built {
        let Some((id, configuration)) = pair.split_once('|') else {
            continue;
        };
        if let Some(project) = view
            .projects
            .iter_mut()
            .find(|project| project.id.eq_ignore_ascii_case(id))
        {
            project.builds_in.push(configuration.to_owned());
        }
    }
    view
}

/// Whether `text` is a Visual Studio solution.
fn looks_like_it(text: &str) -> bool {
    // The banner is the whole of it: nothing else opens this way, and a
    // solution without it will not open in Visual Studio either.
    text.lines().take(3).any(|line| {
        line.trim_start_matches('\u{feff}')
            .starts_with("Microsoft Visual Studio Solution File, Format Version ")
    })
}

/// The Visual Studio solution plugin's core half.
#[derive(Debug, Default)]
pub struct SolutionCore;

impl PluginCore for SolutionCore {
    fn name(&self) -> &'static str {
        "solution"
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
        // The projects and the configuration block are the whole of
        // what a solution says, and both are on the view already.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Visual Studio solution plugin's presentation half.
#[derive(Debug, Default)]
pub struct SolutionPresentation;

impl PluginPresentation for SolutionPresentation {
    fn name(&self) -> &'static str {
        "solution"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "SLN",
            tint: 0x0068_217a,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: SolutionView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "Solution file format version {}",
            view.format_version
        ));
        if let Some(version) = &view.visual_studio_version {
            lines.push(format!("Written by Visual Studio {version}"));
        }
        if !view.configurations.is_empty() {
            lines.push(format!(
                "Configurations: {}",
                view.configurations.join(", ")
            ));
        }
        lines.push(format!("{} project(s):", view.projects.len()));
        for project in &view.projects {
            lines.push(format!("  {} ({})", project.name, project.kind));
            if project.path != project.name {
                lines.push(format!("      {}", project.path));
            }
            if !project.builds_in.is_empty() {
                lines.push(format!("      builds in {}", project.builds_in.join(", ")));
            }
        }
        if !view.folders.is_empty() {
            lines.push(format!(
                "Solution folders, which hold no code: {}",
                view.folders.join(", ")
            ));
        }
        if !view.selected_but_not_built.is_empty() {
            lines.push("Selected but not built, so a solution build passes over".to_owned());
            lines.push("them without saying so:".to_owned());
            for entry in &view.selected_but_not_built {
                lines.push(format!("  {entry}"));
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
    use super::{SolutionCore, SolutionPresentation, SolutionView, parse, quoted_fields};
    use plugin_api::{PluginCore, PluginPresentation};

    const SOLUTION: &str = concat!(
        "Microsoft Visual Studio Solution File, Format Version 12.00\n",
        "# Visual Studio Version 17\n",
        "VisualStudioVersion = 17.9.34728.123\n",
        "Project(\"{9A19103F-16F7-4668-BE54-9A1E7A4F7556}\") = \"App\", ",
        "\"src\\App\\App.csproj\", \"{11111111-1111-1111-1111-111111111111}\"\n",
        "EndProject\n",
        "Project(\"{2150E333-8FDC-42A3-9474-1A3956D46DE8}\") = \"Solution Items\", ",
        "\"Solution Items\", \"{33333333-3333-3333-3333-333333333333}\"\n",
        "EndProject\n",
        "Global\n",
        "\tGlobalSection(SolutionConfigurationPlatforms) = preSolution\n",
        "\t\tDebug|Any CPU = Debug|Any CPU\n",
        "\t\tRelease|Any CPU = Release|Any CPU\n",
        "\tEndGlobalSection\n",
        "\tGlobalSection(ProjectConfigurationPlatforms) = postSolution\n",
        "\t\t{11111111-1111-1111-1111-111111111111}.Debug|Any CPU.ActiveCfg = Debug|Any CPU\n",
        "\t\t{11111111-1111-1111-1111-111111111111}.Debug|Any CPU.Build.0 = Debug|Any CPU\n",
        "\t\t{11111111-1111-1111-1111-111111111111}.Release|Any CPU.ActiveCfg = Release|Any CPU\n",
        "\tEndGlobalSection\n",
        "EndGlobal\n",
    );

    #[test]
    fn sniffs_a_solution() {
        assert!(SolutionCore.sniff(SOLUTION.as_bytes()));
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_banner() {
        // Visual Studio writes one, and every real solution file has it.
        let with_mark = format!("\u{feff}{SOLUTION}");

        assert!(SolutionCore.sniff(with_mark.as_bytes()));
    }

    #[test]
    fn does_not_claim_a_document_that_mentions_the_banner_further_down() {
        let prose = format!("# Notes\n\n\n\n{SOLUTION}");

        assert!(!SolutionCore.sniff(prose.as_bytes()));
        assert!(!SolutionCore.sniff(b""));
    }

    #[test]
    fn splits_the_project_line_into_its_four_fields() {
        assert_eq!(
            quoted_fields("Project(\"{A}\") = \"Name\", \"a\\b.csproj\", \"{B}\""),
            vec![
                "{A}".to_owned(),
                "Name".to_owned(),
                "a\\b.csproj".to_owned(),
                "{B}".to_owned()
            ]
        );
    }

    #[test]
    fn names_the_project_kind_and_marks_the_solution_folder() {
        let view = parse(SOLUTION);

        assert_eq!(view.format_version, "12.00");
        assert_eq!(
            view.visual_studio_version.as_deref(),
            Some("17.9.34728.123")
        );
        assert_eq!(view.projects.len(), 2);
        assert_eq!(view.projects[0].kind, "C#, SDK-style");
        assert_eq!(view.folders, vec!["Solution Items".to_owned()]);
    }

    #[test]
    fn a_selected_configuration_is_not_a_built_one() {
        let view = parse(SOLUTION);

        assert_eq!(view.configurations.len(), 2);
        assert_eq!(view.projects[0].builds_in, vec!["Debug|Any CPU".to_owned()]);
        assert_eq!(
            view.selected_but_not_built,
            vec!["App in Release|Any CPU".to_owned()],
            "an ActiveCfg with no Build.0 means the build passes the project over"
        );
    }

    #[test]
    fn presents_the_skipped_build_with_its_reason() {
        let data = serde_json::to_value(parse(SOLUTION)).unwrap();

        let lines = SolutionPresentation.present(&data);

        assert_eq!(lines[0], "Solution file format version 12.00");
        assert!(lines.iter().any(|line| line.contains("without saying so")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("App in Release|Any CPU"))
        );
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/solution/Workspace.sln");

        let data = SolutionCore.view(&path).unwrap();
        let view: SolutionView = serde_json::from_value(data).unwrap();

        assert_eq!(view.format_version, "12.00");
        assert!(view.visual_studio_version.is_some());
        assert!(view.projects.len() >= 3);
        assert!(!view.folders.is_empty());
        assert!(view.configurations.len() >= 2);
        assert!(view.projects.iter().any(|p| !p.builds_in.is_empty()));
        assert!(!view.selected_but_not_built.is_empty());
        assert!(view.projects.iter().any(|p| p.kind == "C#, SDK-style"));
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::SolutionCore),
            plugin_api::PluginPresentation::extensions(&crate::SolutionPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
