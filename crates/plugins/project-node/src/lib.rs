//! Node project folder plugin: core and presentation halves.
//!
//! A folder holding a `package.json` is a Node (Node.js JavaScript runtime)
//! project, and this reads that manifest to say what kind of one: its name
//! and version, whether it is private, which package manager its lock file
//! names, whether it has been installed, how many dependencies it
//! declares, the scripts a reader can run, the runtime it demands, and
//! whether it is a workspace root.
//!
//! Per decision D10 this reads and never drives. Nothing here runs `npm`,
//! `yarn`, `pnpm` or `bun`, and nothing here writes to the folder. A
//! `package.json` states what the author declared, which is the question a
//! reader standing in the folder is asking; resolving it into what would
//! actually install needs the registry and the lock file's own resolution,
//! and is a different job from describing a folder.

use plugin_api::{FolderCore, FolderPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// The file that makes a folder a Node project.
const MANIFEST: &str = "package.json";

/// The largest manifest this will read. A `package.json` is kilobytes; a
/// file past this is not one, and a folder plugin runs while somebody is
/// waiting for a pane to draw.
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

/// The directory a package manager installs into. Its presence, not its
/// contents, is what this reads - walking it would make a folder plugin
/// cost as much as the install it is describing.
const INSTALL_DIRECTORY: &str = "node_modules";

/// How many workspace members are named before the rest are counted.
const MAX_MEMBERS_SHOWN: usize = 8;

/// How many scripts are named before the rest are counted.
const MAX_SCRIPTS_SHOWN: usize = 8;

/// Which package manager a project's lock file names, in the order this
/// checks for one. A project with more than one lock file is not one this
/// plugin has seen, but npm's is checked first as the default `package.json`
/// itself assumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PackageManager {
    /// `package-lock.json`.
    Npm,
    /// `yarn.lock`.
    Yarn,
    /// `pnpm-lock.yaml`.
    Pnpm,
    /// `bun.lockb`.
    Bun,
}

impl PackageManager {
    /// The lock file that names this package manager.
    const fn lock_file(self) -> &'static str {
        match self {
            Self::Npm => "package-lock.json",
            Self::Yarn => "yarn.lock",
            Self::Pnpm => "pnpm-lock.yaml",
            Self::Bun => "bun.lockb",
        }
    }

    /// A short label for a reader.
    const fn label(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Yarn => "Yarn",
            Self::Pnpm => "pnpm",
            Self::Bun => "Bun",
        }
    }
}

/// Every package manager this plugin knows a lock file for, in the order
/// they are checked.
const PACKAGE_MANAGERS: &[PackageManager] = &[
    PackageManager::Npm,
    PackageManager::Yarn,
    PackageManager::Pnpm,
    PackageManager::Bun,
];

/// A `scripts` entry: the name a reader runs it by, and the command it
/// expands to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptEntry {
    /// The name passed to `npm run` (or its equivalent in another package
    /// manager).
    pub name: String,
    /// The command it runs.
    pub command: String,
}

/// What `workspaces` declares, when the manifest declares one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    /// The member paths and glob patterns as the manifest names them -
    /// expanding `packages/*` would mean walking the tree, which is more
    /// than describing a folder should cost.
    pub members: Vec<String>,
}

/// View data produced by [`NodeProjectCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeProjectView {
    /// The package's name, when the manifest names one.
    pub name: Option<String>,
    /// Its version, when the manifest states one.
    pub version: Option<String>,
    /// Whether the manifest marks the package private.
    pub private: bool,
    /// The package manager the lock file beside the manifest names, when
    /// there is one this plugin recognises.
    pub package_manager: Option<PackageManager>,
    /// Whether a `node_modules` directory sits beside the manifest.
    pub installed: bool,
    /// How many entries `dependencies` holds.
    pub dependencies: usize,
    /// How many entries `devDependencies` holds.
    pub dev_dependencies: usize,
    /// The scripts the manifest declares, in the order `scripts` names
    /// them.
    pub scripts: Vec<ScriptEntry>,
    /// The runtime the manifest demands, from `engines.node`.
    pub node_engine: Option<String>,
    /// The workspace this manifest is the root of, when it declares one.
    pub workspace: Option<WorkspaceInfo>,
}

/// The core half: recognises the folder and reads its manifest.
pub struct NodeProjectCore;

/// The presentation half: turns the manifest into lines.
pub struct NodeProjectPresentation;

/// The string a manifest field holds, when it is one.
fn string_field(object: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    object.get(key)?.as_str().map(str::to_owned)
}

/// How many entries the object at `key` holds, and none if there is no such
/// object.
fn count(manifest: &serde_json::Map<String, serde_json::Value>, key: &str) -> usize {
    manifest
        .get(key)
        .and_then(serde_json::Value::as_object)
        .map_or(0, serde_json::Map::len)
}

/// Reads `scripts` as name/command pairs, in the order `serde_json` hands
/// them back.
fn scripts_of(manifest: &serde_json::Map<String, serde_json::Value>) -> Vec<ScriptEntry> {
    manifest
        .get("scripts")
        .and_then(serde_json::Value::as_object)
        .map(|scripts| {
            scripts
                .iter()
                .filter_map(|(name, command)| {
                    Some(ScriptEntry {
                        name: name.clone(),
                        command: command.as_str()?.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Reads `engines.node`.
fn node_engine_of(manifest: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    string_field(manifest.get("engines")?.as_object()?, "node")
}

/// Reads `workspaces`, which is a list of paths and glob patterns, or an
/// object naming that list under `packages` (the form Lerna popularised
/// and npm also accepts).
fn workspace_of(manifest: &serde_json::Map<String, serde_json::Value>) -> Option<WorkspaceInfo> {
    let value = manifest.get("workspaces")?;
    let members = value
        .as_array()
        .or_else(|| value.as_object()?.get("packages")?.as_array())?;
    Some(WorkspaceInfo {
        members: members
            .iter()
            .filter_map(|member| member.as_str().map(str::to_owned))
            .collect(),
    })
}

/// Which package manager's lock file sits beside `path`, checked in
/// [`PACKAGE_MANAGERS`] order.
fn package_manager_at(path: &Path) -> Option<PackageManager> {
    PACKAGE_MANAGERS
        .iter()
        .copied()
        .find(|manager| path.join(manager.lock_file()).is_file())
}

impl FolderCore for NodeProjectCore {
    fn name(&self) -> &'static str {
        "project-node"
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
        let manifest: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&text)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, format!("{err}")))?;

        let view = NodeProjectView {
            name: string_field(&manifest, "name"),
            version: string_field(&manifest, "version"),
            private: manifest
                .get("private")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or_default(),
            package_manager: package_manager_at(path),
            installed: path.join(INSTALL_DIRECTORY).is_dir(),
            dependencies: count(&manifest, "dependencies"),
            dev_dependencies: count(&manifest, "devDependencies"),
            scripts: scripts_of(&manifest),
            node_engine: node_engine_of(&manifest),
            workspace: workspace_of(&manifest),
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// `n dependencies`, counting the two tables separately because a reader
/// asking "what does this pull in" means the first of them.
fn dependency_line(view: &NodeProjectView) -> Option<String> {
    let mut parts = Vec::new();
    if view.dependencies > 0 {
        parts.push(format!("{}", view.dependencies));
    }
    if view.dev_dependencies > 0 {
        parts.push(format!("{} for development", view.dev_dependencies));
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!("Dependencies: {}", parts.join(", ")))
}

/// The package manager line, when the lock file names one.
fn package_manager_line(view: &NodeProjectView) -> Option<String> {
    let manager = view.package_manager?;
    let state = if view.installed {
        "installed"
    } else {
        "not installed"
    };
    Some(format!("Package manager: {} ({state})", manager.label()))
}

impl FolderPresentation for NodeProjectPresentation {
    fn name(&self) -> &'static str {
        "project-node"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let Ok(view) = serde_json::from_value::<NodeProjectView>(data.clone()) else {
            return vec!["Node project: unreadable manifest".to_owned()];
        };

        let mut lines = vec![if view.workspace.is_some() {
            "Node workspace".to_owned()
        } else {
            "Node project".to_owned()
        }];

        if let Some(name) = &view.name {
            let mut line = match &view.version {
                Some(version) => format!("Package: {name} {version}"),
                None => format!("Package: {name}"),
            };
            if view.private {
                line.push_str(" (private)");
            }
            lines.push(line);
        } else if view.private {
            lines.push("Private package".to_owned());
        }

        if let Some(line) = package_manager_line(&view) {
            lines.push(line);
        }

        if let Some(line) = dependency_line(&view) {
            lines.push(line);
        }

        if let Some(node_engine) = &view.node_engine {
            lines.push(format!("Node: {node_engine}"));
        }

        if !view.scripts.is_empty() {
            lines.push(format!("Scripts: {}", view.scripts.len()));
            for script in view.scripts.iter().take(MAX_SCRIPTS_SHOWN) {
                lines.push(format!("  {}: {}", script.name, script.command));
            }
            let hidden = view.scripts.len().saturating_sub(MAX_SCRIPTS_SHOWN);
            if hidden > 0 {
                lines.push(format!("  and {hidden} more"));
            }
        }

        if let Some(workspace) = &view.workspace {
            lines.push(format!("Members: {}", workspace.members.len()));
            for member in workspace.members.iter().take(MAX_MEMBERS_SHOWN) {
                lines.push(format!("  {member}"));
            }
            let hidden = workspace.members.len().saturating_sub(MAX_MEMBERS_SHOWN);
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
        MAX_MEMBERS_SHOWN, MAX_SCRIPTS_SHOWN, NodeProjectCore, NodeProjectPresentation,
        NodeProjectView, PackageManager, ScriptEntry, WorkspaceInfo,
    };
    use plugin_api::{FolderCore, FolderPresentation};
    use std::path::{Path, PathBuf};

    /// A folder under the temporary directory, holding `manifest` as its
    /// `package.json`.
    fn folder_with(label: &str, manifest: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rse-project-node-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("package.json"), manifest).unwrap();
        dir
    }

    fn view_of(dir: &Path) -> NodeProjectView {
        let data = NodeProjectCore.view(dir).unwrap();
        serde_json::from_value(data).unwrap()
    }

    #[test]
    fn a_folder_with_a_manifest_is_recognised() {
        assert!(NodeProjectCore.sniff(&["src", "package.json", "README.md"]));
    }

    #[test]
    fn a_folder_without_one_is_not() {
        assert!(!NodeProjectCore.sniff(&["src", "Cargo.toml"]));
        assert!(!NodeProjectCore.sniff(&[]));
        // The lock file is not the manifest: a folder can hold one without
        // being a project root, and npm itself goes by `package.json`.
        assert!(!NodeProjectCore.sniff(&["package-lock.json"]));
    }

    #[test]
    fn a_package_reports_its_name_version_manager_and_dependencies() {
        let dir = folder_with(
            "package",
            r#"{
                "name": "widgets",
                "version": "0.4.1",
                "scripts": { "build": "tsc", "test": "node --test" },
                "engines": { "node": ">=20" },
                "dependencies": { "left-pad": "^1.0.0", "chalk": "^5.0.0" },
                "devDependencies": { "eslint": "^9.0.0" }
            }"#,
        );
        std::fs::write(dir.join("package-lock.json"), "{}").unwrap();

        let view = view_of(&dir);

        assert_eq!(view.name.as_deref(), Some("widgets"));
        assert_eq!(view.version.as_deref(), Some("0.4.1"));
        assert!(!view.private);
        assert_eq!(view.package_manager, Some(PackageManager::Npm));
        assert!(!view.installed);
        assert_eq!(view.dependencies, 2);
        assert_eq!(view.dev_dependencies, 1);
        assert_eq!(view.node_engine.as_deref(), Some(">=20"));
        assert_eq!(
            view.scripts,
            vec![
                ScriptEntry {
                    name: "build".to_owned(),
                    command: "tsc".to_owned(),
                },
                ScriptEntry {
                    name: "test".to_owned(),
                    command: "node --test".to_owned(),
                },
            ]
        );
        assert!(view.workspace.is_none());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_node_modules_directory_says_the_project_is_installed() {
        let dir = folder_with("installed", r#"{ "name": "widgets" }"#);
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();

        assert!(view_of(&dir).installed);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_private_workspace_root_reports_its_members() {
        let dir = folder_with(
            "workspace",
            r#"{
                "name": "monorepo",
                "private": true,
                "workspaces": ["packages/one", "packages/two", "packages/*"]
            }"#,
        );
        std::fs::write(dir.join("yarn.lock"), "").unwrap();

        let view = view_of(&dir);

        assert!(view.private);
        assert_eq!(view.package_manager, Some(PackageManager::Yarn));
        assert_eq!(
            view.workspace,
            Some(WorkspaceInfo {
                members: vec![
                    "packages/one".to_owned(),
                    "packages/two".to_owned(),
                    "packages/*".to_owned(),
                ],
            }),
            "a glob is reported as written: expanding it means walking the tree"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn workspaces_as_an_object_names_its_packages_list() {
        let dir = folder_with(
            "workspace-object",
            r#"{ "name": "monorepo", "workspaces": { "packages": ["apps/*"] } }"#,
        );

        let view = view_of(&dir);

        assert_eq!(
            view.workspace,
            Some(WorkspaceInfo {
                members: vec!["apps/*".to_owned()],
            })
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_lock_file_reports_no_package_manager() {
        let dir = folder_with("no-lock", r#"{ "name": "widgets" }"#);

        assert_eq!(view_of(&dir).package_manager, None);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_manifest_that_is_not_json_is_an_error_rather_than_a_guess() {
        let dir = folder_with("broken", "this is not JSON at all {{{");

        let refused = NodeProjectCore.view(&dir);

        assert!(refused.is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn presents_a_package_as_lines_a_reader_can_use() {
        let data = NodeProjectCore
            .view(&folder_with(
                "present",
                r#"{
                    "name": "widgets",
                    "version": "0.4.1",
                    "scripts": { "build": "tsc" },
                    "engines": { "node": ">=20" },
                    "dependencies": { "left-pad": "^1.0.0" },
                    "devDependencies": { "eslint": "^9.0.0" }
                }"#,
            ))
            .unwrap();

        let lines = NodeProjectPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "Node project",
                "Package: widgets 0.4.1",
                "Dependencies: 1, 1 for development",
                "Node: >=20",
                "Scripts: 1",
                "  build: tsc",
            ]
        );
    }

    #[test]
    fn presents_a_long_member_and_script_list_without_running_off_the_pane() {
        let members: Vec<String> = (0..MAX_MEMBERS_SHOWN + 4)
            .map(|index| format!("packages/member-{index}"))
            .collect();
        let scripts: Vec<ScriptEntry> = (0..MAX_SCRIPTS_SHOWN + 3)
            .map(|index| ScriptEntry {
                name: format!("script-{index}"),
                command: "run".to_owned(),
            })
            .collect();
        let data = serde_json::to_value(NodeProjectView {
            name: None,
            version: None,
            private: false,
            package_manager: None,
            installed: false,
            dependencies: 0,
            dev_dependencies: 0,
            scripts: scripts.clone(),
            node_engine: None,
            workspace: Some(WorkspaceInfo {
                members: members.clone(),
            }),
        })
        .unwrap();

        let lines = NodeProjectPresentation.present(&data);

        assert_eq!(lines[0], "Node workspace");
        let scripts_header = lines.iter().position(|line| line == "Scripts: 11").unwrap();
        assert_eq!(
            lines[scripts_header + MAX_SCRIPTS_SHOWN + 1],
            "  and 3 more"
        );
        let members_header = lines
            .iter()
            .position(|line| line == &format!("Members: {}", members.len()))
            .unwrap();
        assert_eq!(lines.last().unwrap(), "  and 4 more");
        assert_eq!(members_header, scripts_header + MAX_SCRIPTS_SHOWN + 2);
    }

    #[test]
    fn both_halves_answer_to_the_same_name() {
        assert_eq!(
            FolderCore::name(&NodeProjectCore),
            FolderPresentation::name(&NodeProjectPresentation),
            "the service names the plugin that produced a view, and the front end looks the \
             presentation half up by that name"
        );
    }

    #[test]
    fn the_repository_fixture_reads_as_the_project_it_declares() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/project-node");

        let view = view_of(&dir);

        assert_eq!(view.name.as_deref(), Some("@example/factory-floor"));
        assert_eq!(view.version.as_deref(), Some("1.4.0"));
        assert!(view.private);
        assert_eq!(view.package_manager, Some(PackageManager::Npm));
        assert!(!view.installed, "the fixture ships without node_modules");
        assert_eq!(view.dependencies, 2);
        assert_eq!(view.dev_dependencies, 2);
        assert_eq!(view.scripts.len(), 3);
        assert_eq!(view.node_engine.as_deref(), Some(">=20"));
        assert_eq!(
            view.workspace,
            Some(WorkspaceInfo {
                members: vec!["packages/*".to_owned()],
            })
        );
    }
}
