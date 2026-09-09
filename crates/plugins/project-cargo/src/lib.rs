//! Cargo project folder plugin: core and presentation halves.
//!
//! A folder holding a `Cargo.toml` is a Rust project, and this reads that
//! manifest to say which kind: a workspace, a package, or - as this
//! repository's own root is - both at once.
//!
//! Per decision D10 this reads and never drives. Nothing here runs
//! `cargo`, and nothing here writes to the folder. A `Cargo.toml` states
//! what the author declared, which is the question a reader standing in
//! the folder is asking; resolving it into what would actually build
//! needs the registry, the lock file and the network, and is a different
//! job from describing a folder.
//!
//! Values a member manifest inherits from its workspace (`version.workspace
//! = true`) are reported as inherited rather than as missing. Every crate
//! in this repository is written that way, so a plugin that read them as
//! absent would describe its own home wrongly.

use plugin_api::{FolderCore, FolderPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// The file that makes a folder a Cargo project.
const MANIFEST: &str = "Cargo.toml";

/// The largest manifest this will read. A `Cargo.toml` is kilobytes; a
/// file past this is not one, and a folder plugin runs while somebody is
/// waiting for a pane to draw.
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

/// How many workspace members are named before the rest are counted.
const MAX_MEMBERS_SHOWN: usize = 8;

/// A value a member manifest takes from its workspace instead of stating.
const INHERITED: &str = "inherited from the workspace";

/// What a `Cargo.toml` declares its folder to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectKind {
    /// A `[workspace]` and nothing else: the root of a set of crates,
    /// which is not itself one.
    Workspace,
    /// A `[package]`: one crate.
    Package,
    /// Both, which is what a repository with a leading crate and members
    /// beside it looks like.
    WorkspaceAndPackage,
    /// Neither, which a `Cargo.toml` should not be - but a folder can hold
    /// a file by that name that declares something else entirely, and
    /// saying so is better than guessing.
    Neither,
}

/// What the `[package]` section declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageInfo {
    /// The crate's name.
    pub name: String,
    /// Its version, or [`INHERITED`] where the manifest defers to the
    /// workspace.
    pub version: String,
    /// The Rust edition it is written against, or [`INHERITED`].
    pub edition: String,
    /// The oldest Rust it declares support for, when it declares one.
    pub rust_version: Option<String>,
    /// Its one-line description, when it has one.
    pub description: Option<String>,
}

/// View data produced by [`CargoProjectCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CargoProjectView {
    /// What the manifest declares the folder to be.
    pub kind: ProjectKind,
    /// The `[package]` section, when there is one.
    pub package: Option<PackageInfo>,
    /// The workspace members as the manifest names them, globs and all -
    /// expanding `crates/plugins/*` would mean walking the tree, which is
    /// more than describing a folder should cost.
    pub members: Vec<String>,
    /// How many entries `[dependencies]` holds.
    pub dependencies: usize,
    /// How many entries `[dev-dependencies]` holds.
    pub dev_dependencies: usize,
    /// How many entries `[build-dependencies]` holds.
    pub build_dependencies: usize,
}

/// The core half: recognises the folder and reads its manifest.
pub struct CargoProjectCore;

/// The presentation half: turns the manifest into lines.
pub struct CargoProjectPresentation;

/// The string a manifest field holds, resolving the `{ workspace = true }`
/// form into [`INHERITED`].
///
/// A number is accepted too: `version = 1` is legal TOML and a manifest in
/// the wild will eventually hold one.
fn field(table: &toml::Table, key: &str) -> Option<String> {
    match table.get(key)? {
        toml::Value::String(text) => Some(text.clone()),
        toml::Value::Integer(number) => Some(number.to_string()),
        toml::Value::Table(inherited)
            if inherited.get("workspace") == Some(&toml::Value::Boolean(true)) =>
        {
            Some(INHERITED.to_owned())
        }
        _ => None,
    }
}

/// How many entries the table at `key` holds, and none if there is no such
/// table.
fn count(manifest: &toml::Table, key: &str) -> usize {
    manifest
        .get(key)
        .and_then(toml::Value::as_table)
        .map_or(0, toml::Table::len)
}

/// Reads the `[package]` section, if the manifest has one worth reporting.
///
/// A `[package]` with no name is not a package: Cargo would refuse it, and
/// a pane saying "Package:" with nothing after it tells a reader less than
/// saying nothing.
fn package_of(manifest: &toml::Table) -> Option<PackageInfo> {
    let package = manifest.get("package")?.as_table()?;
    Some(PackageInfo {
        name: field(package, "name")?,
        version: field(package, "version").unwrap_or_else(|| "unstated".to_owned()),
        edition: field(package, "edition").unwrap_or_else(|| "2015".to_owned()),
        rust_version: field(package, "rust-version"),
        description: field(package, "description"),
    })
}

/// Reads `workspace.members`, which is a list of paths and glob patterns.
fn members_of(manifest: &toml::Table) -> Vec<String> {
    manifest
        .get("workspace")
        .and_then(toml::Value::as_table)
        .and_then(|workspace| workspace.get("members"))
        .and_then(toml::Value::as_array)
        .map(|members| {
            members
                .iter()
                .filter_map(|member| member.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

impl FolderCore for CargoProjectCore {
    fn name(&self) -> &'static str {
        "project-cargo"
    }

    fn sniff(&self, entries: &[&str]) -> bool {
        entries.contains(&MANIFEST)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let manifest_path = path.join(MANIFEST);
        if std::fs::metadata(&manifest_path)?.len() > MAX_MANIFEST_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{MANIFEST} is larger than a manifest can reasonably be"),
            ));
        }
        let text = std::fs::read_to_string(&manifest_path)?;
        let manifest: toml::Table = text
            .parse()
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, format!("{err}")))?;

        let package = package_of(&manifest);
        let has_workspace = manifest.contains_key("workspace");
        let kind = match (has_workspace, package.is_some()) {
            (true, true) => ProjectKind::WorkspaceAndPackage,
            (true, false) => ProjectKind::Workspace,
            (false, true) => ProjectKind::Package,
            (false, false) => ProjectKind::Neither,
        };

        let view = CargoProjectView {
            kind,
            package,
            members: members_of(&manifest),
            dependencies: count(&manifest, "dependencies"),
            dev_dependencies: count(&manifest, "dev-dependencies"),
            build_dependencies: count(&manifest, "build-dependencies"),
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The headline for a project of this kind.
const fn headline(kind: ProjectKind) -> &'static str {
    match kind {
        ProjectKind::Workspace => "Cargo workspace",
        ProjectKind::Package => "Cargo package",
        ProjectKind::WorkspaceAndPackage => "Cargo workspace, and a package itself",
        ProjectKind::Neither => "Cargo manifest, declaring neither a package nor a workspace",
    }
}

/// `n dependencies`, counting the three tables separately because a reader
/// asking "what does this pull in" means the first of them.
fn dependency_line(view: &CargoProjectView) -> Option<String> {
    let mut parts = Vec::new();
    if view.dependencies > 0 {
        parts.push(format!("{}", view.dependencies));
    }
    if view.dev_dependencies > 0 {
        parts.push(format!("{} for development", view.dev_dependencies));
    }
    if view.build_dependencies > 0 {
        parts.push(format!("{} for the build script", view.build_dependencies));
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!("Dependencies: {}", parts.join(", ")))
}

impl FolderPresentation for CargoProjectPresentation {
    fn name(&self) -> &'static str {
        "project-cargo"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let Ok(view) = serde_json::from_value::<CargoProjectView>(data.clone()) else {
            return vec!["Cargo project: unreadable manifest".to_owned()];
        };

        let mut lines = vec![headline(view.kind).to_owned()];

        if let Some(package) = &view.package {
            lines.push(if package.version == INHERITED {
                format!("Package: {} (version {INHERITED})", package.name)
            } else {
                format!("Package: {} {}", package.name, package.version)
            });
            if let Some(description) = &package.description {
                lines.push(description.clone());
            }
            lines.push(if package.edition == INHERITED {
                format!("Edition: {INHERITED}")
            } else {
                format!("Edition: {}", package.edition)
            });
            if let Some(rust_version) = &package.rust_version {
                lines.push(format!("Rust: {rust_version} or newer"));
            }
        }

        if let Some(line) = dependency_line(&view) {
            lines.push(line);
        }

        if !view.members.is_empty() {
            lines.push(format!("Members: {}", view.members.len()));
            for member in view.members.iter().take(MAX_MEMBERS_SHOWN) {
                lines.push(format!("  {member}"));
            }
            let hidden = view.members.len().saturating_sub(MAX_MEMBERS_SHOWN);
            if hidden > 0 {
                lines.push(format!("  and {hidden} more"));
            }
        }

        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CargoProjectCore, CargoProjectPresentation, CargoProjectView, INHERITED, MAX_MEMBERS_SHOWN,
        ProjectKind,
    };
    use plugin_api::{FolderCore, FolderPresentation};
    use std::path::{Path, PathBuf};

    /// A folder under the temporary directory, holding `manifest` as its
    /// `Cargo.toml`.
    fn folder_with(label: &str, manifest: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rse-project-cargo-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Cargo.toml"), manifest).unwrap();
        dir
    }

    fn view_of(dir: &Path) -> CargoProjectView {
        let data = CargoProjectCore.view(dir).unwrap();
        serde_json::from_value(data).unwrap()
    }

    #[test]
    fn a_folder_with_a_manifest_is_recognised() {
        assert!(CargoProjectCore.sniff(&["src", "Cargo.toml", "README.md"]));
    }

    #[test]
    fn a_folder_without_one_is_not() {
        assert!(!CargoProjectCore.sniff(&["src", "package.json"]));
        assert!(!CargoProjectCore.sniff(&[]));
        // The lock file is not the manifest: a folder can hold one without
        // being a project root, and Cargo itself goes by `Cargo.toml`.
        assert!(!CargoProjectCore.sniff(&["Cargo.lock"]));
    }

    #[test]
    fn a_package_reports_its_name_version_and_edition() {
        let dir = folder_with(
            "package",
            r#"
[package]
name = "ringbuffer"
version = "0.4.1"
edition = "2021"
rust-version = "1.82"
description = "A bounded queue."

[dependencies]
serde = "1"
thiserror = "2"

[dev-dependencies]
proptest = "1"
"#,
        );

        let view = view_of(&dir);

        assert_eq!(view.kind, ProjectKind::Package);
        let package = view.package.as_ref().unwrap();
        assert_eq!(package.name, "ringbuffer");
        assert_eq!(package.version, "0.4.1");
        assert_eq!(package.edition, "2021");
        assert_eq!(package.rust_version.as_deref(), Some("1.82"));
        assert_eq!(package.description.as_deref(), Some("A bounded queue."));
        assert_eq!(view.dependencies, 2);
        assert_eq!(view.dev_dependencies, 1);
        assert_eq!(view.build_dependencies, 0);
        assert!(view.members.is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_workspace_reports_its_members() {
        let dir = folder_with(
            "workspace",
            r#"
[workspace]
resolver = "3"
members = ["crates/one", "crates/two", "crates/plugins/*"]
"#,
        );

        let view = view_of(&dir);

        assert_eq!(view.kind, ProjectKind::Workspace);
        assert!(view.package.is_none());
        assert_eq!(
            view.members,
            vec![
                "crates/one".to_owned(),
                "crates/two".to_owned(),
                "crates/plugins/*".to_owned(),
            ],
            "a glob is reported as written: expanding it means walking the tree"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_manifest_that_is_both_says_so() {
        let dir = folder_with(
            "both",
            r#"
[workspace]
members = ["crates/one"]

[package]
name = "leading-crate"
version = "1.0.0"
edition = "2024"
"#,
        );

        let view = view_of(&dir);

        assert_eq!(view.kind, ProjectKind::WorkspaceAndPackage);
        assert_eq!(view.package.unwrap().name, "leading-crate");
        assert_eq!(view.members.len(), 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_inherited_value_reads_as_inherited_rather_than_missing() {
        // Every crate in this repository is written this way, so a plugin
        // that read these as absent would describe its own home wrongly.
        let dir = folder_with(
            "inherited",
            r#"
[package]
name = "plugin-toml"
version.workspace = true
edition.workspace = true
"#,
        );

        let view = view_of(&dir);
        let package = view.package.unwrap();

        assert_eq!(package.version, INHERITED);
        assert_eq!(package.edition, INHERITED);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_manifest_that_is_not_one_is_an_error_rather_than_a_guess() {
        let dir = folder_with("broken", "this is not TOML at all [[[");

        let refused = CargoProjectCore.view(&dir);

        assert!(refused.is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn presents_a_package_as_lines_a_reader_can_use() {
        let data = CargoProjectCore
            .view(&folder_with(
                "present",
                r#"
[package]
name = "ringbuffer"
version = "0.4.1"
edition = "2021"
rust-version = "1.82"
description = "A bounded queue."

[dependencies]
serde = "1"

[dev-dependencies]
proptest = "1"
"#,
            ))
            .unwrap();

        let lines = CargoProjectPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "Cargo package",
                "Package: ringbuffer 0.4.1",
                "A bounded queue.",
                "Edition: 2021",
                "Rust: 1.82 or newer",
                "Dependencies: 1, 1 for development",
            ]
        );
    }

    #[test]
    fn presents_a_long_member_list_without_running_off_the_pane() {
        let members: Vec<String> = (0..MAX_MEMBERS_SHOWN + 4)
            .map(|index| format!("crates/member-{index}"))
            .collect();
        let data = serde_json::to_value(CargoProjectView {
            kind: ProjectKind::Workspace,
            package: None,
            members: members.clone(),
            dependencies: 0,
            dev_dependencies: 0,
            build_dependencies: 0,
        })
        .unwrap();

        let lines = CargoProjectPresentation.present(&data);

        assert_eq!(lines[0], "Cargo workspace");
        assert_eq!(lines[1], format!("Members: {}", members.len()));
        assert_eq!(lines.last().unwrap(), "  and 4 more");
        assert_eq!(lines.len(), 2 + MAX_MEMBERS_SHOWN + 1);
    }

    #[test]
    fn both_halves_answer_to_the_same_name() {
        assert_eq!(
            FolderCore::name(&CargoProjectCore),
            FolderPresentation::name(&CargoProjectPresentation),
            "the service names the plugin that produced a view, and the front end looks the \
             presentation half up by that name"
        );
    }
    #[test]
    fn the_repository_fixture_reads_as_the_project_it_declares() {
        // `samples/project-cargo/` is a real crate - a manifest naming a
        // package with no source would be a manifest for something that
        // cannot build, and this sample set holds working files.
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/project-cargo");

        let data = CargoProjectCore.view(&dir).unwrap();
        let view: CargoProjectView = serde_json::from_value(data).unwrap();

        assert_eq!(view.kind, ProjectKind::WorkspaceAndPackage);
        let package = view.package.as_ref().unwrap();
        assert_eq!(package.name, "instrument-log");
        assert_eq!(package.version, "2.3.0");
        assert_eq!(package.edition, "2024");
        assert_eq!(package.rust_version.as_deref(), Some("1.85"));
        assert_eq!(
            package.description.as_deref(),
            Some("Reads and summarises instrument run logs.")
        );
        assert_eq!(
            view.members,
            vec![
                "crates/reading".to_owned(),
                "crates/writing".to_owned(),
                "crates/plugins/*".to_owned(),
            ]
        );
        assert_eq!(view.dependencies, 3);
        assert_eq!(view.dev_dependencies, 2);
        assert_eq!(view.build_dependencies, 1);
    }
}
