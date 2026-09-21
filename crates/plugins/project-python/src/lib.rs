//! Python project folder plugin: core and presentation halves.
//!
//! A Python checkout announces itself through whichever manifest its age
//! and tooling wrote: `pyproject.toml` under
//! [PEP 621](https://peps.python.org/pep-0621/), the older `setup.cfg`
//! `setuptools` dialect, a bare `requirements.txt`, or a `setup.py` with
//! none of the above beside it. This reads whichever of those a folder
//! holds, in that order, to say what kind of project it is: its
//! distribution name and version, the build backend, the Python version
//! it demands, how many dependencies it declares, and the entry points it
//! installs - and separately, whichever dependency manager's lock file
//! and whichever virtual environment sit beside the manifest, and whether
//! the project is tested with `tox`.
//!
//! Per decision D10 this reads and never drives. Nothing here runs `pip`,
//! `poetry`, `pdm`, `uv` or `python` itself, and nothing here writes to
//! the folder.

use plugin_api::{FolderCore, FolderPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// The files that make a folder a Python project, checked for recognition
/// in this order and, when more than one is present, in the same order
/// for which manifest supplies the facts below - `pyproject.toml` is the
/// richest of the four, and a bare `requirements.txt` the poorest.
const MANIFESTS: &[&str] = &[
    "pyproject.toml",
    "setup.cfg",
    "requirements.txt",
    "setup.py",
];

/// The largest manifest this will read. A Python manifest is kilobytes; a
/// file past this is not one, and a folder plugin runs while somebody is
/// waiting for a pane to draw.
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

/// The directory names a virtual environment is conventionally created
/// under, checked in this order. Presence alone is not enough - a folder
/// can hold a directory by either name that is not one - so this also
/// requires the `pyvenv.cfg` marker file every tool in the ecosystem
/// writes inside a real one.
const VIRTUAL_ENV_DIRECTORIES: &[&str] = &[".venv", "venv"];

/// The marker file inside a virtual environment directory that proves it
/// is one, rather than an unrelated folder that happens to share a name.
const VIRTUAL_ENV_MARKER: &str = "pyvenv.cfg";

/// The file that says a project is tested with `tox`.
const TOX_CONFIGURATION: &str = "tox.ini";

/// How many entry points are named before the rest are only counted.
const MAX_ENTRY_POINTS_SHOWN: usize = 8;

/// Which tool a project's lock file names, in the order this checks for
/// one. A project with more than one lock file is not one this plugin has
/// seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyManager {
    /// `poetry.lock`.
    Poetry,
    /// `pdm.lock`.
    Pdm,
    /// `uv.lock`.
    Uv,
    /// `Pipfile.lock`.
    Pipenv,
}

impl DependencyManager {
    /// The lock file that names this dependency manager.
    const fn lock_file(self) -> &'static str {
        match self {
            Self::Poetry => "poetry.lock",
            Self::Pdm => "pdm.lock",
            Self::Uv => "uv.lock",
            Self::Pipenv => "Pipfile.lock",
        }
    }

    /// A short label for a reader.
    const fn label(self) -> &'static str {
        match self {
            Self::Poetry => "Poetry",
            Self::Pdm => "PDM",
            Self::Uv => "uv",
            Self::Pipenv => "Pipenv",
        }
    }
}

/// Every dependency manager this plugin knows a lock file for, in the
/// order they are checked.
const DEPENDENCY_MANAGERS: &[DependencyManager] = &[
    DependencyManager::Poetry,
    DependencyManager::Pdm,
    DependencyManager::Uv,
    DependencyManager::Pipenv,
];

/// One entry point the project installs: the command a reader runs it by,
/// and what it calls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryPoint {
    /// The command name.
    pub name: String,
    /// The `module:function` it runs.
    pub target: String,
}

/// The facts a manifest supplies, whichever of the four this reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ManifestFacts {
    name: Option<String>,
    version: Option<String>,
    build_backend: Option<String>,
    requires_python: Option<String>,
    dependencies: usize,
    entry_points: Vec<EntryPoint>,
}

/// View data produced by [`PythonProjectCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PythonProjectView {
    /// The distribution name, when the manifest names one.
    pub name: Option<String>,
    /// Its version, when the manifest states one.
    pub version: Option<String>,
    /// The build backend named in `[build-system]`. `setup.cfg` and
    /// `requirements.txt` have no such table, so this is `None` for
    /// either.
    pub build_backend: Option<String>,
    /// The Python version the project demands.
    pub requires_python: Option<String>,
    /// The dependency manager the lock file beside the manifest names,
    /// when there is one this plugin recognises.
    pub dependency_manager: Option<DependencyManager>,
    /// How many dependencies the manifest declares.
    pub dependencies: usize,
    /// Whether a virtual environment is present in the checkout.
    pub virtual_env: bool,
    /// The entry points the project installs.
    pub entry_points: Vec<EntryPoint>,
    /// Whether the project is tested with `tox`.
    pub tox: bool,
}

/// The core half: recognises the folder and reads its manifest.
pub struct PythonProjectCore;

/// The presentation half: turns the manifest into lines.
pub struct PythonProjectPresentation;

/// The string a `[project]` (or `[build-system]`) table's field holds, if
/// it is a string.
fn string_field(table: Option<&toml::Table>, key: &str) -> Option<String> {
    table?.get(key)?.as_str().map(str::to_owned)
}

/// How many entries the array at `key` holds, and none if there is no
/// such array.
fn array_len(table: Option<&toml::Table>, key: &str) -> usize {
    table
        .and_then(|table| table.get(key))
        .and_then(toml::Value::as_array)
        .map_or(0, Vec::len)
}

/// Reads `[project.scripts]` as name/target pairs, in the order
/// `toml` hands them back.
fn scripts_of(project: Option<&toml::Table>) -> Vec<EntryPoint> {
    project
        .and_then(|project| project.get("scripts"))
        .and_then(toml::Value::as_table)
        .map(|scripts| {
            scripts
                .iter()
                .filter_map(|(name, target)| {
                    Some(EntryPoint {
                        name: name.clone(),
                        target: target.as_str()?.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Reads a `pyproject.toml` at `text` into the facts it declares. `None`
/// if it does not parse as TOML at all - a truncated read can cut a
/// multi-line array in the middle, and the reader is meant to see that a
/// manifest could not be read rather than one missing everything past the
/// cut.
fn pyproject_facts(text: &str) -> Option<ManifestFacts> {
    let root: toml::Table = toml::from_str(text).ok()?;
    let build_system = root.get("build-system").and_then(toml::Value::as_table);
    let project = root.get("project").and_then(toml::Value::as_table);

    Some(ManifestFacts {
        name: string_field(project, "name"),
        version: string_field(project, "version"),
        build_backend: string_field(build_system, "build-backend"),
        requires_python: string_field(project, "requires-python"),
        dependencies: array_len(project, "dependencies"),
        entry_points: scripts_of(project),
    })
}

/// One `[section]` of a `setup.cfg`-style INI file: its name, and its
/// `key = value` entries in file order, each value split into the lines
/// `configparser`'s indented-continuation syntax joins - an inline value
/// after the `=` counts as the first of them.
struct CfgSection {
    name: String,
    entries: Vec<(String, Vec<String>)>,
}

impl CfgSection {
    /// The first value line of `key`, if the section has it.
    fn first(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .and_then(|(_, values)| values.first())
            .map(String::as_str)
    }

    /// All the value lines of `key`, or empty if the section lacks it.
    fn list(&self, key: &str) -> &[String] {
        self.entries
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map_or(&[], |(_, values)| values.as_slice())
    }
}

/// `text`'s `[section]`s, in file order, reading `configparser`'s
/// indented-continuation syntax: a line indented past its key continues
/// that key's value as another list item, which is how `setup.cfg` writes
/// `install_requires` and `entry_points`.
fn parse_cfg_sections(text: &str) -> Vec<CfgSection> {
    let mut sections = Vec::new();
    let mut current: Option<CfgSection> = None;
    let mut current_key: Option<usize> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if indented {
            if let (Some(section), Some(index)) = (current.as_mut(), current_key) {
                section.entries[index].1.push(trimmed.to_owned());
            }
            continue;
        }
        if let Some(name) = trimmed
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            if let Some(section) = current.take() {
                sections.push(section);
            }
            current = Some(CfgSection {
                name: name.trim().to_owned(),
                entries: Vec::new(),
            });
            current_key = None;
            continue;
        }
        current_key = None;
        if let Some((key, value)) = trimmed.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            if key.is_empty() {
                continue;
            }
            if let Some(section) = current.as_mut() {
                let values = if value.is_empty() {
                    Vec::new()
                } else {
                    vec![value.to_owned()]
                };
                section.entries.push((key.to_owned(), values));
                current_key = Some(section.entries.len() - 1);
            }
        }
    }
    if let Some(section) = current {
        sections.push(section);
    }
    sections
}

/// The named section, if `sections` has one.
fn cfg_section<'a>(sections: &'a [CfgSection], name: &str) -> Option<&'a CfgSection> {
    sections
        .iter()
        .find(|section| section.name.eq_ignore_ascii_case(name))
}

/// Reads a `setup.cfg` at `text` into the facts it declares. `setup.cfg`
/// has no `[build-system]` table - a project of that era is built by
/// `setuptools` implicitly - so `build_backend` is always `None`.
fn setup_cfg_facts(text: &str) -> ManifestFacts {
    let sections = parse_cfg_sections(text);
    let metadata = cfg_section(&sections, "metadata");
    let options = cfg_section(&sections, "options");
    let entry_points = cfg_section(&sections, "options.entry_points")
        .map_or(&[][..], |section| section.list("console_scripts"))
        .iter()
        .filter_map(|line| {
            let (name, target) = line.split_once('=')?;
            Some(EntryPoint {
                name: name.trim().to_owned(),
                target: target.trim().to_owned(),
            })
        })
        .collect();

    ManifestFacts {
        name: metadata
            .and_then(|metadata| metadata.first("name"))
            .map(str::to_owned),
        version: metadata
            .and_then(|metadata| metadata.first("version"))
            .map(str::to_owned),
        build_backend: None,
        requires_python: options
            .and_then(|options| options.first("python_requires"))
            .map(str::to_owned),
        dependencies: options.map_or(0, |options| options.list("install_requires").len()),
        entry_points,
    }
}

/// Reads a `requirements.txt` at `text` into the one fact it can carry:
/// how many dependencies it pins. A line is a dependency unless it is
/// blank, a comment, or an option flag (`-r other.txt`, `-e .`) rather
/// than a package.
fn requirements_txt_facts(text: &str) -> ManifestFacts {
    let dependencies = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with('-'))
        .count();
    ManifestFacts {
        dependencies,
        ..ManifestFacts::default()
    }
}

/// Reads whichever manifest [`MANIFESTS`] finds first in `path`, and the
/// name it was read as. `setup.py` alone carries none of these facts
/// statically - reading what it declares would mean running it, which
/// decision D10 rules out - so it recognises the folder without adding to
/// what is reported.
fn manifest_facts(path: &Path) -> io::Result<ManifestFacts> {
    for manifest in MANIFESTS {
        let manifest_path = path.join(manifest);
        let Ok(metadata) = std::fs::metadata(&manifest_path) else {
            continue;
        };
        if *manifest == "setup.py" {
            return Ok(ManifestFacts::default());
        }
        if metadata.len() > MAX_MANIFEST_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{manifest} is larger than a manifest can reasonably be"),
            ));
        }
        let text = std::fs::read_to_string(&manifest_path)?;
        return match *manifest {
            "pyproject.toml" => pyproject_facts(&text).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "pyproject.toml is not valid TOML",
                )
            }),
            "requirements.txt" => Ok(requirements_txt_facts(&text)),
            _ => Ok(setup_cfg_facts(&text)),
        };
    }
    Ok(ManifestFacts::default())
}

/// Which dependency manager's lock file sits beside `path`, checked in
/// [`DEPENDENCY_MANAGERS`] order.
fn dependency_manager_at(path: &Path) -> Option<DependencyManager> {
    DEPENDENCY_MANAGERS
        .iter()
        .copied()
        .find(|manager| path.join(manager.lock_file()).is_file())
}

/// Whether a virtual environment sits in the checkout: one of
/// [`VIRTUAL_ENV_DIRECTORIES`], proven by the marker file every tool in
/// the ecosystem writes inside a real one.
fn virtual_env_at(path: &Path) -> bool {
    VIRTUAL_ENV_DIRECTORIES
        .iter()
        .any(|directory| path.join(directory).join(VIRTUAL_ENV_MARKER).is_file())
}

impl FolderCore for PythonProjectCore {
    fn name(&self) -> &'static str {
        "project-python"
    }

    fn sniff(&self, entries: &[&str]) -> bool {
        MANIFESTS.iter().any(|manifest| entries.contains(manifest))
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let facts = manifest_facts(path)?;

        let view = PythonProjectView {
            name: facts.name,
            version: facts.version,
            build_backend: facts.build_backend,
            requires_python: facts.requires_python,
            dependency_manager: dependency_manager_at(path),
            dependencies: facts.dependencies,
            virtual_env: virtual_env_at(path),
            entry_points: facts.entry_points,
            tox: path.join(TOX_CONFIGURATION).is_file(),
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The dependency manager line, when a lock file names one.
fn dependency_manager_line(view: &PythonProjectView) -> Option<String> {
    let manager = view.dependency_manager?;
    Some(format!("Dependency manager: {}", manager.label()))
}

impl FolderPresentation for PythonProjectPresentation {
    fn name(&self) -> &'static str {
        "project-python"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let Ok(view) = serde_json::from_value::<PythonProjectView>(data.clone()) else {
            return vec!["Python project: unreadable manifest".to_owned()];
        };

        let mut lines = vec!["Python project".to_owned()];

        if let Some(name) = &view.name {
            let line = match &view.version {
                Some(version) => format!("Package: {name} {version}"),
                None => format!("Package: {name}"),
            };
            lines.push(line);
        }

        if let Some(backend) = &view.build_backend {
            lines.push(format!("Build backend: {backend}"));
        }

        if let Some(requires_python) = &view.requires_python {
            lines.push(format!("Python: {requires_python}"));
        }

        if let Some(line) = dependency_manager_line(&view) {
            lines.push(line);
        }

        if view.dependencies > 0 {
            lines.push(format!("Dependencies: {}", view.dependencies));
        }

        lines.push(format!(
            "Virtual environment: {}",
            if view.virtual_env {
                "present"
            } else {
                "not present"
            }
        ));

        if view.tox {
            lines.push("Tested with tox".to_owned());
        }

        if !view.entry_points.is_empty() {
            lines.push(format!("Entry points: {}", view.entry_points.len()));
            for entry in view.entry_points.iter().take(MAX_ENTRY_POINTS_SHOWN) {
                lines.push(format!("  {}: {}", entry.name, entry.target));
            }
            let hidden = view
                .entry_points
                .len()
                .saturating_sub(MAX_ENTRY_POINTS_SHOWN);
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
        DependencyManager, EntryPoint, MAX_ENTRY_POINTS_SHOWN, PythonProjectCore,
        PythonProjectPresentation, PythonProjectView,
    };
    use plugin_api::{FolderCore, FolderPresentation};
    use std::path::{Path, PathBuf};

    /// A folder under the temporary directory, holding `files` beside one
    /// another.
    fn folder_with(label: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rse-project-python-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, contents) in files {
            std::fs::write(dir.join(name), contents).unwrap();
        }
        dir
    }

    fn view_of(dir: &Path) -> PythonProjectView {
        let data = PythonProjectCore.view(dir).unwrap();
        serde_json::from_value(data).unwrap()
    }

    #[test]
    fn a_folder_with_any_of_the_four_manifests_is_recognised() {
        assert!(PythonProjectCore.sniff(&["src", "pyproject.toml", "README.md"]));
        assert!(PythonProjectCore.sniff(&["setup.py"]));
        assert!(PythonProjectCore.sniff(&["setup.cfg"]));
        assert!(PythonProjectCore.sniff(&["requirements.txt"]));
    }

    #[test]
    fn a_folder_without_one_is_not() {
        assert!(!PythonProjectCore.sniff(&["src", "Cargo.toml"]));
        assert!(!PythonProjectCore.sniff(&[]));
        // A lock file is not a manifest: a folder can hold one without
        // being a project root.
        assert!(!PythonProjectCore.sniff(&["poetry.lock"]));
    }

    #[test]
    fn a_pyproject_toml_reports_name_version_backend_python_and_dependencies() {
        let dir = folder_with(
            "pyproject",
            &[(
                "pyproject.toml",
                r#"
                [build-system]
                requires = ["hatchling"]
                build-backend = "hatchling.build"

                [project]
                name = "widgets"
                version = "0.4.1"
                requires-python = ">=3.11"
                dependencies = ["click>=8", "requests>=2"]

                [project.scripts]
                widgets = "widgets.cli:main"
                "#,
            )],
        );
        std::fs::write(dir.join("poetry.lock"), "").unwrap();

        let view = view_of(&dir);

        assert_eq!(view.name.as_deref(), Some("widgets"));
        assert_eq!(view.version.as_deref(), Some("0.4.1"));
        assert_eq!(view.build_backend.as_deref(), Some("hatchling.build"));
        assert_eq!(view.requires_python.as_deref(), Some(">=3.11"));
        assert_eq!(view.dependencies, 2);
        assert_eq!(view.dependency_manager, Some(DependencyManager::Poetry));
        assert!(!view.virtual_env);
        assert!(!view.tox);
        assert_eq!(
            view.entry_points,
            vec![EntryPoint {
                name: "widgets".to_owned(),
                target: "widgets.cli:main".to_owned(),
            }]
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_setup_cfg_reports_name_version_python_dependencies_and_entry_points() {
        let dir = folder_with(
            "setup-cfg",
            &[(
                "setup.cfg",
                "[metadata]\n\
                 name = widgets\n\
                 version = 0.2.0\n\
                 \n\
                 [options]\n\
                 python_requires = >=3.9\n\
                 install_requires =\n\
                 \tclick>=8\n\
                 \trequests>=2\n\
                 \tpyyaml\n\
                 \n\
                 [options.entry_points]\n\
                 console_scripts =\n\
                 \twidgets = widgets.cli:main\n",
            )],
        );

        let view = view_of(&dir);

        assert_eq!(view.name.as_deref(), Some("widgets"));
        assert_eq!(view.version.as_deref(), Some("0.2.0"));
        assert!(view.build_backend.is_none());
        assert_eq!(view.requires_python.as_deref(), Some(">=3.9"));
        assert_eq!(view.dependencies, 3);
        assert_eq!(
            view.entry_points,
            vec![EntryPoint {
                name: "widgets".to_owned(),
                target: "widgets.cli:main".to_owned(),
            }]
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_bare_requirements_txt_reports_only_a_dependency_count() {
        let dir = folder_with(
            "requirements",
            &[(
                "requirements.txt",
                "# comment\nclick>=8\nrequests>=2\n\n-e .\n",
            )],
        );

        let view = view_of(&dir);

        assert!(view.name.is_none());
        assert_eq!(view.dependencies, 2);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_bare_setup_py_is_recognised_but_carries_no_facts() {
        let dir = folder_with(
            "setup-py",
            &[("setup.py", "from setuptools import setup\n")],
        );

        let view = view_of(&dir);

        assert!(view.name.is_none());
        assert_eq!(view.dependencies, 0);
        assert!(view.entry_points.is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_pyvenv_cfg_marker_says_a_virtual_environment_is_present() {
        let dir = folder_with(
            "venv",
            &[("pyproject.toml", "[project]\nname = \"widgets\"\n")],
        );
        std::fs::create_dir_all(dir.join(".venv")).unwrap();
        std::fs::write(dir.join(".venv").join("pyvenv.cfg"), "home = /usr\n").unwrap();

        assert!(view_of(&dir).virtual_env);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_venv_directory_without_the_marker_is_not_one() {
        let dir = folder_with(
            "fake-venv",
            &[("pyproject.toml", "[project]\nname = \"widgets\"\n")],
        );
        std::fs::create_dir_all(dir.join("venv")).unwrap();

        assert!(!view_of(&dir).virtual_env);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_tox_ini_says_the_project_is_tested_with_tox() {
        let dir = folder_with(
            "tox",
            &[
                ("pyproject.toml", "[project]\nname = \"widgets\"\n"),
                ("tox.ini", "[tox]\nenvlist = py311\n"),
            ],
        );

        assert!(view_of(&dir).tox);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_manifest_that_is_not_toml_is_an_error_rather_than_a_guess() {
        let dir = folder_with(
            "broken",
            &[("pyproject.toml", "this is not TOML at all {{{")],
        );

        let refused = PythonProjectCore.view(&dir);

        assert!(refused.is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn presents_a_package_as_lines_a_reader_can_use() {
        let data = PythonProjectCore
            .view(&folder_with(
                "present",
                &[(
                    "pyproject.toml",
                    r#"
                    [build-system]
                    build-backend = "hatchling.build"

                    [project]
                    name = "widgets"
                    version = "0.4.1"
                    requires-python = ">=3.11"
                    dependencies = ["click>=8"]

                    [project.scripts]
                    widgets = "widgets.cli:main"
                    "#,
                )],
            ))
            .unwrap();

        let lines = PythonProjectPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "Python project",
                "Package: widgets 0.4.1",
                "Build backend: hatchling.build",
                "Python: >=3.11",
                "Dependencies: 1",
                "Virtual environment: not present",
                "Entry points: 1",
                "  widgets: widgets.cli:main",
            ]
        );
    }

    #[test]
    fn presents_a_long_entry_point_list_without_running_off_the_pane() {
        let entry_points: Vec<EntryPoint> = (0..MAX_ENTRY_POINTS_SHOWN + 3)
            .map(|index| EntryPoint {
                name: format!("tool-{index}"),
                target: "widgets.cli:main".to_owned(),
            })
            .collect();
        let data = serde_json::to_value(PythonProjectView {
            name: None,
            version: None,
            build_backend: None,
            requires_python: None,
            dependency_manager: None,
            dependencies: 0,
            virtual_env: false,
            entry_points: entry_points.clone(),
            tox: false,
        })
        .unwrap();

        let lines = PythonProjectPresentation.present(&data);

        let header = lines
            .iter()
            .position(|line| line == &format!("Entry points: {}", entry_points.len()))
            .unwrap();
        assert_eq!(lines[header + MAX_ENTRY_POINTS_SHOWN + 1], "  and 3 more");
    }

    #[test]
    fn both_halves_answer_to_the_same_name() {
        assert_eq!(
            FolderCore::name(&PythonProjectCore),
            FolderPresentation::name(&PythonProjectPresentation),
            "the service names the plugin that produced a view, and the front end looks the \
             presentation half up by that name"
        );
    }

    #[test]
    fn the_repository_fixture_reads_as_the_project_it_declares() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/project-python");

        let view = view_of(&dir);

        assert_eq!(view.name.as_deref(), Some("widgets"));
        assert_eq!(view.version.as_deref(), Some("0.4.1"));
        assert_eq!(view.build_backend.as_deref(), Some("hatchling.build"));
        assert_eq!(view.requires_python.as_deref(), Some(">=3.11"));
        assert_eq!(view.dependencies, 2);
        assert_eq!(view.dependency_manager, Some(DependencyManager::Poetry));
        assert_eq!(
            view.entry_points,
            vec![EntryPoint {
                name: "widgets".to_owned(),
                target: "widgets.cli:main".to_owned(),
            }]
        );
        assert!(
            !view.virtual_env,
            "the fixture ships without a virtual environment"
        );
        assert!(!view.tox, "the fixture ships without a tox.ini");
    }
}
