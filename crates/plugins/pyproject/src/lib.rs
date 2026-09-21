//! Python project manifest (`pyproject.toml`, `setup.cfg`) file type
//! plugin: core and presentation halves.
//!
//! Two files declare the same thing in two eras' syntax: a modern project
//! writes `pyproject.toml`, TOML under [PEP
//! 621](https://peps.python.org/pep-0621/); an older one writes
//! `setup.cfg`, the `setuptools` INI dialect it replaced. Both name a
//! distribution, a version and its dependencies, so one plugin reads
//! either shape into the same view.

use plugin_api::{Icon, PluginCore, PluginPresentation, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;
use syntax::{Language, Quote};

/// The lowercase extensions this type claims, without their dot.
///
/// Deliberately empty: `pyproject.toml` is TOML and `toml` owns that
/// extension, `setup.cfg` is INI and `ini` owns `cfg`. This plugin reads
/// the object shape each carries instead, and `specialises` settles it
/// against both.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One `name: value` pair, used for entry points.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The command name.
    pub name: String,
    /// What it runs: `module:function`.
    pub value: String,
}

/// One optional-dependency extra.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extra {
    /// The extra's name, asked for as `package[name]`.
    pub name: String,
    /// The dependencies it adds.
    pub dependencies: Vec<String>,
}

/// View data produced by [`PyprojectCore::view`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PyprojectView {
    /// Whether the file parsed as one of the two shapes at all. When
    /// `false` every other field is empty, and the presentation half says
    /// so instead of showing a manifest with nothing in it.
    pub valid: bool,
    /// Which file this was read as: `"pyproject.toml"` or `"setup.cfg"`.
    pub format: String,
    /// The build backend named in `[build-system]`. `setup.cfg` has no
    /// such table; a project of that era is built by `setuptools`
    /// implicitly, so this is `None` for one.
    pub build_backend: Option<String>,
    /// The packages `[build-system]` needs to build the project at all.
    pub build_requires: Vec<String>,
    /// The distribution name.
    pub name: Option<String>,
    /// The distribution version.
    pub version: Option<String>,
    /// The Python versions this project supports.
    pub requires_python: Option<String>,
    /// Dependencies needed to run the project.
    pub dependencies: Vec<String>,
    /// The optional-dependency extras, by name.
    pub optional_dependencies: Vec<Extra>,
    /// The command-line executables it installs.
    pub console_scripts: Vec<Entry>,
    /// The graphical executables it installs.
    pub gui_scripts: Vec<Entry>,
    /// The declared licence: an SPDX expression, or the text of one
    /// named in a table.
    pub license: Option<String>,
    /// The declared authors.
    pub authors: Vec<String>,
    /// The tools the repository configures in-tree, read off `[tool.*]`.
    /// `setup.cfg` has no equivalent table, so this is empty for one.
    pub tools: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Whether `text` has a top-level `[project]` or `[build-system]` table -
/// the two tables [PEP 621](https://peps.python.org/pep-0621/) and
/// PEP 517 added, and the shape no other format in this project's
/// `samples/` carries.
fn looks_like_pyproject_toml(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .any(|line| line == "[project]" || line == "[build-system]")
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
    fn list(&self, key: &str) -> Vec<String> {
        self.entries
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, values)| values.clone())
            .unwrap_or_default()
    }
}

/// `text`'s `[section]`s, in file order, reading `configparser`'s
/// indented-continuation syntax: a line indented past its key continues
/// that key's value as another list item, which is how `setup.cfg` writes
/// `install_requires`, `extras_require` and `entry_points`.
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
fn section<'a>(sections: &'a [CfgSection], name: &str) -> Option<&'a CfgSection> {
    sections
        .iter()
        .find(|section| section.name.eq_ignore_ascii_case(name))
}

/// Whether `text` is a `setup.cfg`: it has a `[metadata]` section naming a
/// `name`. `poetry.lock` also has a bare `[metadata]` table, but never a
/// `name` inside it - only `lock-version`, `python-versions` and
/// `content-hash` - so requiring one is what tells the two apart.
fn looks_like_setup_cfg(text: &str) -> bool {
    section(&parse_cfg_sections(text), "metadata")
        .is_some_and(|metadata| metadata.first("name").is_some())
}

/// `value` as a string: itself, or its compact JSON rendering otherwise.
fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// The strings in a JSON array, or empty if `value` is not one.
fn strings_of(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// `object[key]`'s entries as `name: value` pairs, or empty when the key
/// is absent or not an object.
fn entries_of(object: &serde_json::Map<String, Value>, key: &str) -> Vec<Entry> {
    object
        .get(key)
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(name, value)| Entry {
                    name: name.clone(),
                    value: value_to_string(value),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The declared licence: an SPDX expression string, or the `text` a
/// licence table names.
///
/// [PEP 639](https://peps.python.org/pep-0639/) also lets a table name a
/// `file` instead, which carries the licence text itself rather than
/// naming it - read out as where to find it, since there is no text here
/// to show.
fn license_of(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Object(table) => table
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                table
                    .get("file")
                    .and_then(Value::as_str)
                    .map(|file| format!("declared in {file}"))
            }),
        _ => None,
    }
}

/// One entry of `[project.authors]`: a bare string, or a table naming a
/// `name`, an `email`, or both.
fn author_of(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Object(table) => {
            let name = table.get("name").and_then(Value::as_str);
            let email = table.get("email").and_then(Value::as_str);
            match (name, email) {
                (Some(name), Some(email)) => Some(format!("{name} <{email}>")),
                (Some(name), None) => Some(name.to_owned()),
                (None, Some(email)) => Some(email.to_owned()),
                (None, None) => None,
            }
        }
        _ => None,
    }
}

/// Everything [`PyprojectView`] holds, read from a `pyproject.toml`'s
/// `text`. `None` if it does not parse as TOML at all - a truncated read
/// can cut a multi-line array in the middle, and the reader is meant to
/// say so rather than show a manifest missing everything past the cut.
fn parse_pyproject_toml(text: &str) -> Option<PyprojectView> {
    let root: Value = toml::from_str::<toml::Value>(text)
        .ok()
        .and_then(|value| serde_json::to_value(value).ok())?;
    let object = root.as_object()?;

    let build_system = object.get("build-system").and_then(Value::as_object);
    let project = object.get("project").and_then(Value::as_object);
    let string = |table: Option<&serde_json::Map<String, Value>>, key: &str| {
        table
            .and_then(|table| table.get(key))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };

    let optional_dependencies = project
        .and_then(|project| project.get("optional-dependencies"))
        .and_then(Value::as_object)
        .map(|extras| {
            extras
                .iter()
                .map(|(name, value)| Extra {
                    name: name.clone(),
                    dependencies: strings_of(Some(value)),
                })
                .collect()
        })
        .unwrap_or_default();

    let empty = serde_json::Map::new();
    let project_table = project.unwrap_or(&empty);

    Some(PyprojectView {
        valid: true,
        format: "pyproject.toml".to_owned(),
        build_backend: string(build_system, "build-backend"),
        build_requires: strings_of(build_system.and_then(|table| table.get("requires"))),
        name: string(project, "name"),
        version: string(project, "version"),
        requires_python: string(project, "requires-python"),
        dependencies: strings_of(project.and_then(|table| table.get("dependencies"))),
        optional_dependencies,
        console_scripts: entries_of(project_table, "scripts"),
        gui_scripts: entries_of(project_table, "gui-scripts"),
        license: project
            .and_then(|table| table.get("license"))
            .and_then(license_of),
        authors: project
            .and_then(|table| table.get("authors"))
            .and_then(Value::as_array)
            .map(|authors| authors.iter().filter_map(author_of).collect())
            .unwrap_or_default(),
        tools: object
            .get("tool")
            .and_then(Value::as_object)
            .map(|tools| tools.keys().cloned().collect())
            .unwrap_or_default(),
        truncated: false,
    })
}

/// One `name = module:function` line of an `[options.entry_points]`
/// section.
fn entry_of(line: &str) -> Option<Entry> {
    let (name, value) = line.split_once('=')?;
    Some(Entry {
        name: name.trim().to_owned(),
        value: value.trim().to_owned(),
    })
}

/// Everything [`PyprojectView`] holds, read from a `setup.cfg`'s `text`.
fn parse_setup_cfg(text: &str) -> PyprojectView {
    let sections = parse_cfg_sections(text);
    let metadata = section(&sections, "metadata");
    let options = section(&sections, "options");
    let entry_points = section(&sections, "options.entry_points");

    let authors = metadata
        .and_then(|metadata| metadata.first("author"))
        .map(|author| {
            author
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    PyprojectView {
        valid: true,
        format: "setup.cfg".to_owned(),
        build_backend: None,
        build_requires: Vec::new(),
        name: metadata
            .and_then(|metadata| metadata.first("name"))
            .map(str::to_owned),
        version: metadata
            .and_then(|metadata| metadata.first("version"))
            .map(str::to_owned),
        requires_python: options
            .and_then(|options| options.first("python_requires"))
            .map(str::to_owned),
        dependencies: options
            .map(|options| options.list("install_requires"))
            .unwrap_or_default(),
        optional_dependencies: section(&sections, "options.extras_require")
            .map(|extras| {
                extras
                    .entries
                    .iter()
                    .map(|(name, dependencies)| Extra {
                        name: name.clone(),
                        dependencies: dependencies.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        console_scripts: entry_points
            .map(|entry_points| {
                entry_points
                    .list("console_scripts")
                    .iter()
                    .filter_map(|line| entry_of(line))
                    .collect()
            })
            .unwrap_or_default(),
        gui_scripts: entry_points
            .map(|entry_points| {
                entry_points
                    .list("gui_scripts")
                    .iter()
                    .filter_map(|line| entry_of(line))
                    .collect()
            })
            .unwrap_or_default(),
        license: metadata
            .and_then(|metadata| metadata.first("license"))
            .map(str::to_owned),
        authors,
        tools: Vec::new(),
        truncated: false,
    }
}

/// Everything [`PyprojectView`] holds, read from `text` as whichever of
/// the two shapes it is.
fn parse(text: &str) -> PyprojectView {
    if looks_like_pyproject_toml(text) {
        parse_pyproject_toml(text).unwrap_or_else(|| PyprojectView {
            format: "pyproject.toml".to_owned(),
            ..PyprojectView::default()
        })
    } else if looks_like_setup_cfg(text) {
        parse_setup_cfg(text)
    } else {
        PyprojectView::default()
    }
}

/// The Python project manifest plugin's core half.
#[derive(Debug, Default)]
pub struct PyprojectCore;

/// How this language is coloured, for the shared tokeniser. GUIDANCE.md
/// §3.6: the plugin describes its own format, the pane paints what it is
/// told.
const PYPROJECT: Language = Language {
    line_comment: &["#", ";"],
    block_comment: &[],
    quotes: &[Quote::simple('"'), Quote::simple('\'')],
    keywords: &["false", "true"],
    types: &[],
    calls: false,
    ignore_case: false,
};

impl PluginCore for PyprojectCore {
    fn name(&self) -> &'static str {
        "pyproject"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // `pyproject.toml` is TOML, `setup.cfg` is INI, and both `toml`
        // and `ini` recognise a bare `[section]` header - `ini`'s sniff
        // returns true on the first one it sees. Naming both is what
        // settles either shape here instead.
        &["toml", "ini"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix)
            .is_ok_and(|text| looks_like_pyproject_toml(text) || looks_like_setup_cfg(text))
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Python project manifest plugin's presentation half.
#[derive(Debug, Default)]
pub struct PyprojectPresentation;

/// `entries` as indented `name value` lines, under `heading` - or nothing
/// when there are none.
fn section_lines(lines: &mut Vec<String>, heading: &str, entries: &[String]) {
    if entries.is_empty() {
        return;
    }
    lines.push(format!("{heading}:"));
    for entry in entries {
        lines.push(format!("  {entry}"));
    }
}

impl PluginPresentation for PyprojectPresentation {
    fn classify(&self, text: &str) -> Vec<Span> {
        syntax::classify(text, &PYPROJECT)
    }

    fn name(&self) -> &'static str {
        "pyproject"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "PROJ",
            tint: 0x0030_6998,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: PyprojectView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        if !view.valid {
            return vec!["not a valid Python project manifest: could not parse it".to_owned()];
        }
        let mut lines = Vec::new();
        let identity = match (&view.name, &view.version) {
            (Some(name), Some(version)) => format!("{name} {version} ({})", view.format),
            (Some(name), None) => format!("{name} ({})", view.format),
            (None, Some(version)) => format!("(unnamed) {version} ({})", view.format),
            (None, None) => format!("(unnamed project) ({})", view.format),
        };
        lines.push(identity);
        if let Some(backend) = &view.build_backend {
            lines.push(format!("build backend: {backend}"));
        }
        section_lines(&mut lines, "build requires", &view.build_requires);
        if let Some(requires_python) = &view.requires_python {
            lines.push(format!("requires-python: {requires_python}"));
        }
        if let Some(license) = &view.license {
            lines.push(format!("licence: {license}"));
        }
        section_lines(&mut lines, "authors", &view.authors);
        section_lines(&mut lines, "dependencies", &view.dependencies);
        for extra in &view.optional_dependencies {
            lines.push(format!("optional dependencies [{}]:", extra.name));
            for dependency in &extra.dependencies {
                lines.push(format!("  {dependency}"));
            }
        }
        if !view.console_scripts.is_empty() {
            lines.push("console scripts:".to_owned());
            for entry in &view.console_scripts {
                lines.push(format!("  {} = {}", entry.name, entry.value));
            }
        }
        if !view.gui_scripts.is_empty() {
            lines.push("gui scripts:".to_owned());
            for entry in &view.gui_scripts {
                lines.push(format!("  {} = {}", entry.name, entry.value));
            }
        }
        section_lines(&mut lines, "tools configured in-tree", &view.tools);
        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Entry, Extra, PyprojectCore, PyprojectPresentation, PyprojectView,
        looks_like_pyproject_toml, looks_like_setup_cfg, parse,
    };
    use plugin_api::{PluginCore, PluginPresentation};

    const PYPROJECT_TOML: &str = r#"
[build-system]
requires = ["hatchling>=1.27", "hatch-vcs"]
build-backend = "hatchling.build"

[project]
name = "widgets"
version = "3.1.0"
requires-python = ">=3.11"
license = { text = "MIT" }
authors = [
    { name = "Ada Lovelace", email = "ada@example.com" },
    { name = "Grace Hopper" },
]
dependencies = ["click>=8.1", "requests>=2.31"]

[project.optional-dependencies]
dev = ["pytest>=8.3", "ruff>=0.11"]
docs = ["sphinx>=7.4"]

[project.scripts]
widgets = "widgets.cli:main"

[project.gui-scripts]
widgets-gui = "widgets.gui:main"

[tool.ruff]
line-length = 88

[tool.mypy]
strict = true

[tool.pytest.ini_options]
testpaths = ["tests"]
"#;

    const SETUP_CFG: &str = r"
[metadata]
name = legacy-widgets
version = 1.4.2
author = Ada Lovelace, Grace Hopper
license = MIT

[options]
python_requires = >=3.9
install_requires =
    click>=8.0
    requests>=2.25

[options.extras_require]
dev =
    pytest
    ruff
docs =
    sphinx

[options.entry_points]
console_scripts =
    legacy-widgets = legacy_widgets.cli:main
gui_scripts =
    legacy-widgets-gui = legacy_widgets.gui:main
";

    #[test]
    fn sniffs_pyproject_toml_by_its_project_or_build_system_table() {
        assert!(PyprojectCore.sniff(PYPROJECT_TOML.as_bytes()));
        assert!(PyprojectCore.sniff(b"[project]\nname = \"a\"\n"));
        assert!(PyprojectCore.sniff(b"[build-system]\nrequires = []\n"));
    }

    #[test]
    fn sniffs_setup_cfg_by_a_named_metadata_section() {
        assert!(PyprojectCore.sniff(SETUP_CFG.as_bytes()));
        assert!(PyprojectCore.sniff(b"[metadata]\nname = a\n"));
    }

    #[test]
    fn does_not_claim_a_metadata_table_naming_nothing() {
        // samples/pythonlock/poetry.lock has a bare `[metadata]` table with
        // no `name` in it - only `lock-version`, `python-versions` and
        // `content-hash`.
        assert!(!PyprojectCore.sniff(
            b"[metadata]\nlock-version = \"2.0\"\npython-versions = \"^3.9\"\ncontent-hash = \"abc\"\n"
        ));
        assert!(!PyprojectCore.sniff(b"[other]\nkey = 1\n"));
        assert!(!PyprojectCore.sniff(b""));
    }

    #[test]
    fn recognises_the_two_shapes_by_line() {
        assert!(looks_like_pyproject_toml("[project]\nname = \"a\"\n"));
        assert!(looks_like_pyproject_toml("[build-system]\n"));
        assert!(!looks_like_pyproject_toml("[tool.ruff]\n"));
        assert!(looks_like_setup_cfg("[metadata]\nname = a\n"));
        assert!(!looks_like_setup_cfg("[metadata]\nversion = 1\n"));
    }

    #[test]
    fn it_says_it_specialises_toml_and_ini() {
        assert_eq!(PyprojectCore.specialises(), &["toml", "ini"]);
        assert!(PluginCore::extensions(&PyprojectCore).is_empty());
    }

    #[test]
    fn reads_the_build_system_and_identity_from_pyproject_toml() {
        let view = parse(PYPROJECT_TOML);

        assert!(view.valid);
        assert_eq!(view.format, "pyproject.toml");
        assert_eq!(view.build_backend.as_deref(), Some("hatchling.build"));
        assert_eq!(
            view.build_requires,
            vec!["hatchling>=1.27".to_owned(), "hatch-vcs".to_owned()]
        );
        assert_eq!(view.name.as_deref(), Some("widgets"));
        assert_eq!(view.version.as_deref(), Some("3.1.0"));
        assert_eq!(view.requires_python.as_deref(), Some(">=3.11"));
        assert_eq!(view.license.as_deref(), Some("MIT"));
    }

    #[test]
    fn reads_dependencies_and_extras_from_pyproject_toml() {
        let view = parse(PYPROJECT_TOML);

        assert_eq!(
            view.dependencies,
            vec!["click>=8.1".to_owned(), "requests>=2.31".to_owned()]
        );
        assert_eq!(view.optional_dependencies.len(), 2);
        assert!(view.optional_dependencies.contains(&Extra {
            name: "dev".to_owned(),
            dependencies: vec!["pytest>=8.3".to_owned(), "ruff>=0.11".to_owned()],
        }));
    }

    #[test]
    fn reads_authors_by_name_and_email_or_name_alone() {
        let view = parse(PYPROJECT_TOML);

        assert_eq!(
            view.authors,
            vec![
                "Ada Lovelace <ada@example.com>".to_owned(),
                "Grace Hopper".to_owned(),
            ]
        );
    }

    #[test]
    fn reads_console_and_gui_scripts_from_pyproject_toml() {
        let view = parse(PYPROJECT_TOML);

        assert_eq!(
            view.console_scripts,
            vec![Entry {
                name: "widgets".to_owned(),
                value: "widgets.cli:main".to_owned(),
            }]
        );
        assert_eq!(
            view.gui_scripts,
            vec![Entry {
                name: "widgets-gui".to_owned(),
                value: "widgets.gui:main".to_owned(),
            }]
        );
    }

    #[test]
    fn reads_the_tools_configured_in_tree() {
        let view = parse(PYPROJECT_TOML);

        assert_eq!(
            view.tools,
            vec!["mypy".to_owned(), "pytest".to_owned(), "ruff".to_owned()]
        );
    }

    #[test]
    fn reads_the_older_shape_from_setup_cfg() {
        let view = parse(SETUP_CFG);

        assert!(view.valid);
        assert_eq!(view.format, "setup.cfg");
        assert_eq!(view.name.as_deref(), Some("legacy-widgets"));
        assert_eq!(view.version.as_deref(), Some("1.4.2"));
        assert_eq!(view.requires_python.as_deref(), Some(">=3.9"));
        assert_eq!(view.license.as_deref(), Some("MIT"));
        assert_eq!(
            view.authors,
            vec!["Ada Lovelace".to_owned(), "Grace Hopper".to_owned()]
        );
        assert!(view.build_backend.is_none());
        assert!(view.tools.is_empty());
    }

    #[test]
    fn reads_the_multi_line_lists_setup_cfg_writes() {
        let view = parse(SETUP_CFG);

        assert_eq!(
            view.dependencies,
            vec!["click>=8.0".to_owned(), "requests>=2.25".to_owned()]
        );
        assert_eq!(view.optional_dependencies.len(), 2);
        assert!(view.optional_dependencies.contains(&Extra {
            name: "dev".to_owned(),
            dependencies: vec!["pytest".to_owned(), "ruff".to_owned()],
        }));
        assert_eq!(
            view.console_scripts,
            vec![Entry {
                name: "legacy-widgets".to_owned(),
                value: "legacy_widgets.cli:main".to_owned(),
            }]
        );
        assert_eq!(
            view.gui_scripts,
            vec![Entry {
                name: "legacy-widgets-gui".to_owned(),
                value: "legacy_widgets.gui:main".to_owned(),
            }]
        );
    }

    #[test]
    fn a_malformed_pyproject_toml_is_refused_without_panicking() {
        let view = parse("[project]\nname = \"a\ndependencies = [\"unterminated");

        assert!(!view.valid);
        assert_eq!(view.format, "pyproject.toml");

        let data = serde_json::to_value(&view).unwrap();
        let lines = PyprojectPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["not a valid Python project manifest: could not parse it"]
        );
    }

    #[test]
    fn neither_shape_is_an_error_it_is_just_not_valid() {
        let view = parse("just some ordinary text\nwith no structure at all\n");

        assert!(!view.valid);
        assert_eq!(view.format, "");
    }

    #[test]
    fn presents_the_identity_line_first() {
        let data = serde_json::to_value(parse(PYPROJECT_TOML)).unwrap();

        let lines = PyprojectPresentation.present(&data);

        assert_eq!(lines[0], "widgets 3.1.0 (pyproject.toml)");
        assert!(
            lines
                .iter()
                .any(|line| line == "build backend: hatchling.build")
        );
    }

    #[test]
    fn the_repository_fixtures_together_fill_every_field() {
        let toml_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/pyproject/pyproject.toml");
        let cfg_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/pyproject/setup.cfg");

        let toml_view: PyprojectView =
            serde_json::from_value(PyprojectCore.view(&toml_path).unwrap()).unwrap();
        let cfg_view: PyprojectView =
            serde_json::from_value(PyprojectCore.view(&cfg_path).unwrap()).unwrap();

        assert!(toml_view.valid);
        assert!(toml_view.name.is_some());
        assert!(toml_view.version.is_some());
        assert!(toml_view.build_backend.is_some());
        assert!(!toml_view.build_requires.is_empty());
        assert!(toml_view.requires_python.is_some());
        assert!(!toml_view.dependencies.is_empty());
        assert!(!toml_view.optional_dependencies.is_empty());
        assert!(!toml_view.console_scripts.is_empty());
        assert!(!toml_view.gui_scripts.is_empty());
        assert!(toml_view.license.is_some());
        assert!(!toml_view.authors.is_empty());
        assert!(!toml_view.tools.is_empty());

        assert!(cfg_view.valid);
        assert!(cfg_view.name.is_some());
        assert!(cfg_view.version.is_some());
        assert!(cfg_view.requires_python.is_some());
        assert!(!cfg_view.dependencies.is_empty());
        assert!(!cfg_view.optional_dependencies.is_empty());
        assert!(!cfg_view.console_scripts.is_empty());
        assert!(!cfg_view.gui_scripts.is_empty());
        assert!(cfg_view.license.is_some());
        assert!(!cfg_view.authors.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::PyprojectCore),
            plugin_api::PluginPresentation::extensions(&crate::PyprojectPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
