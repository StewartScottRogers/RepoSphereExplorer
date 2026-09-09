//! Cargo lock file file type plugin: core and presentation halves.
//!
//! Registered before `toml`, which would otherwise claim it: a lock file
//! is TOML, and `Cargo.lock` has no extension the tiebreak can use. The
//! `[[package]]` tables under a leading `version = ` key are the marker.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["lock"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// Where a locked package came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// A registry, which is crates.io unless the manifest says otherwise.
    Registry,
    /// A git remote, pinned to a revision.
    Git,
    /// A path on this machine, which is what a workspace member and a
    /// local override both look like.
    Path,
    /// No `source` key at all, which is how a workspace's own members
    /// appear.
    Workspace,
}

/// One locked package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    /// The crate name.
    pub name: String,
    /// The exact version locked.
    pub version: String,
    /// Where it comes from.
    pub source: Source,
    /// Its `source` string, when it has one.
    pub source_text: Option<String>,
    /// How many dependencies it declares.
    pub dependencies: usize,
}

/// View data produced by [`CargolockCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CargolockView {
    /// The lock file format version, from the leading `version = ` key.
    pub format: Option<u32>,
    /// Every locked package, in file order.
    pub packages: Vec<Package>,
    /// The packages with no source, which are this workspace's own.
    pub workspace_members: Vec<String>,
    /// The distinct source addresses, so a reader can see at a glance
    /// whether anything comes from somewhere unexpected.
    pub sources: Vec<String>,
    /// Packages locked at more than one version at once.
    pub duplicated: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The unquoted value of a `key = "value"` line.
fn value_of<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.trim().strip_prefix(key)?.trim_start();
    let rest = rest.strip_prefix('=')?.trim();
    Some(rest.trim_matches('"'))
}

/// Records one finished `[[package]]` on `view`, and clears the fields
/// the walk was accumulating into.
///
/// Lifted out of [`parse`], which clippy counts at more lines than it
/// allows. The seam is real: this decides what one package was, and
/// `parse` decides where one ends and the next begins.
fn record(
    name: &mut Option<String>,
    version: &mut Option<String>,
    source: &mut Option<String>,
    dependencies: &mut usize,
    view: &mut CargolockView,
) {
    let (Some(package), Some(at)) = (name.take(), version.take()) else {
        source.take();
        *dependencies = 0;
        return;
    };
    let source_text = source.take();
    let kind = match source_text.as_deref() {
        None => Source::Workspace,
        Some(text) if text.starts_with("git+") => Source::Git,
        Some(text) if text.starts_with("path+") => Source::Path,
        Some(_) => Source::Registry,
    };
    if kind == Source::Workspace {
        view.workspace_members.push(package.clone());
    }
    if let Some(text) = &source_text
        && !view.sources.contains(text)
    {
        view.sources.push(text.clone());
    }
    if view.packages.iter().any(|other| other.name == package)
        && !view.duplicated.contains(&package)
    {
        view.duplicated.push(package.clone());
    }
    view.packages.push(Package {
        name: package,
        version: at,
        source: kind,
        source_text,
        dependencies: *dependencies,
    });
    *dependencies = 0;
}

/// Everything [`CargolockView`] holds, read from `text`.
fn parse(text: &str) -> CargolockView {
    let mut view = CargolockView {
        format: None,
        packages: Vec::new(),
        workspace_members: Vec::new(),
        sources: Vec::new(),
        duplicated: Vec::new(),
        content: String::new(),
        truncated: false,
    };

    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    let mut source: Option<String> = None;
    let mut dependencies = 0usize;
    let mut in_dependencies = false;
    let mut started = false;

    for raw in text.lines() {
        let line = raw.trim();

        if line == "[[package]]" {
            if started {
                record(
                    &mut name,
                    &mut version,
                    &mut source,
                    &mut dependencies,
                    &mut view,
                );
            }
            started = true;
            in_dependencies = false;
            continue;
        }
        if !started {
            if let Some(format) = value_of(line, "version") {
                view.format = format.parse().ok();
            }
            continue;
        }

        if line.starts_with("dependencies") {
            in_dependencies = true;
            // `dependencies = []` is empty and closes at once.
            if line.contains(']') {
                in_dependencies = false;
            }
            continue;
        }
        if in_dependencies {
            if line.starts_with(']') {
                in_dependencies = false;
            } else if !line.is_empty() {
                dependencies += 1;
            }
            continue;
        }

        if let Some(found) = value_of(line, "name") {
            name = Some(found.to_owned());
        } else if let Some(found) = value_of(line, "version") {
            version = Some(found.to_owned());
        } else if let Some(found) = value_of(line, "source") {
            source = Some(found.to_owned());
        }
    }
    if started {
        record(
            &mut name,
            &mut version,
            &mut source,
            &mut dependencies,
            &mut view,
        );
    }
    view
}

/// Whether `text` looks like a Cargo lock file.
fn looks_like_it(text: &str) -> bool {
    let mut packages = 0usize;
    let mut generated = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("# This file is automatically @generated by Cargo") {
            generated = true;
        }
        if trimmed == "[[package]]" {
            packages += 1;
        }
    }
    generated || packages >= 2
}

/// The Cargo lock file plugin's core half.
#[derive(Debug, Default)]
pub struct CargolockCore;

impl PluginCore for CargolockCore {
    fn name(&self) -> &'static str {
        "cargolock"
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

/// The Cargo lock file plugin's presentation half.
#[derive(Debug, Default)]
pub struct CargolockPresentation;

impl PluginPresentation for CargolockPresentation {
    fn name(&self) -> &'static str {
        "cargolock"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "LOCK",
            tint: 0x00b7_410e,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: CargolockView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(format) = view.format {
            lines.push(format!("Lock file format version {format}"));
        }
        lines.push(format!("{} locked package(s)", view.packages.len()));

        let count = |wanted: Source| {
            view.packages
                .iter()
                .filter(|package| package.source == wanted)
                .count()
        };
        lines.push(format!(
            "  {} from a registry, {} from git, {} from a path, {} from this workspace",
            count(Source::Registry),
            count(Source::Git),
            count(Source::Path),
            count(Source::Workspace)
        ));

        for package in &view.packages {
            lines.push(format!("  {} {}", package.name, package.version));
        }
        if !view.workspace_members.is_empty() {
            lines.push(format!(
                "This workspace's own: {}",
                view.workspace_members.join(", ")
            ));
        }
        if !view.sources.is_empty() {
            lines.push(format!("Sources: {}", view.sources.join(", ")));
        }
        if !view.duplicated.is_empty() {
            lines.push(format!(
                "Locked at more than one version at once: {}",
                view.duplicated.join(", ")
            ));
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{CargolockCore, CargolockPresentation, CargolockView, Source, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_the_generated_banner_or_several_package_tables() {
        assert!(
            CargolockCore
                .sniff(b"# This file is automatically @generated by Cargo.\nversion = 4\n")
        );
        assert!(CargolockCore.sniff(b"[[package]]\nname = \"a\"\n[[package]]\nname = \"b\"\n"));
    }

    #[test]
    fn does_not_claim_an_ordinary_manifest() {
        assert!(!CargolockCore.sniff(b"[package]\nname = \"a\"\nversion = \"0.1.0\"\n"));
        assert!(!CargolockCore.sniff(b""));
    }

    #[test]
    fn reads_the_format_version_from_above_the_first_package() {
        let view = parse("version = 4\n\n[[package]]\nname = \"a\"\nversion = \"1.0.0\"\n");

        assert_eq!(view.format, Some(4));
        // The package's own version must not be mistaken for the format's.
        assert_eq!(view.packages[0].version, "1.0.0");
    }

    #[test]
    fn tells_a_registry_from_a_git_remote_from_a_path() {
        let view = parse(
            "version = 4\n\n[[package]]\nname = \"reg\"\nversion = \"1.0.0\"\n\
             source = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n\
             [[package]]\nname = \"rem\"\nversion = \"2.0.0\"\n\
             source = \"git+https://example.com/a?rev=abc#abc\"\n\n\
             [[package]]\nname = \"loc\"\nversion = \"3.0.0\"\nsource = \"path+file:///tmp/a\"\n\n\
             [[package]]\nname = \"mine\"\nversion = \"0.6.0\"\n",
        );

        assert_eq!(view.packages[0].source, Source::Registry);
        assert_eq!(view.packages[1].source, Source::Git);
        assert_eq!(view.packages[2].source, Source::Path);
        assert_eq!(view.packages[3].source, Source::Workspace);
        assert_eq!(view.workspace_members, vec!["mine".to_owned()]);
    }

    #[test]
    fn counts_a_packages_dependencies() {
        let view = parse(
            "[[package]]\nname = \"a\"\nversion = \"1.0.0\"\n\
             dependencies = [\n \"b\",\n \"c\",\n]\n",
        );

        assert_eq!(view.packages[0].dependencies, 2);
    }

    #[test]
    fn an_empty_dependency_list_counts_none() {
        let view = parse("[[package]]\nname = \"a\"\nversion = \"1.0.0\"\ndependencies = []\n");

        assert_eq!(view.packages[0].dependencies, 0);
    }

    #[test]
    fn reports_a_package_locked_at_two_versions() {
        let view = parse(
            "[[package]]\nname = \"a\"\nversion = \"1.0.0\"\n\n\
             [[package]]\nname = \"a\"\nversion = \"2.0.0\"\n",
        );

        assert_eq!(view.duplicated, vec!["a".to_owned()]);
    }

    #[test]
    fn presents_where_the_packages_came_from() {
        let data = serde_json::to_value(parse(
            "version = 4\n[[package]]\nname = \"a\"\nversion = \"1.0.0\"\n",
        ))
        .unwrap();

        let lines = CargolockPresentation.present(&data);

        assert_eq!(lines[0], "Lock file format version 4");
        assert!(lines.iter().any(|line| line.contains("from a registry")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/cargolock/Cargo.lock");

        let data = CargolockCore.view(&path).unwrap();
        let view: CargolockView = serde_json::from_value(data).unwrap();

        assert_eq!(view.format, Some(4));
        assert!(view.packages.len() >= 6);
        assert!(!view.workspace_members.is_empty());
        assert!(view.sources.len() >= 2);
        assert!(!view.duplicated.is_empty());
        assert!(view.packages.iter().any(|p| p.dependencies > 0));
        assert!(view.packages.iter().any(|p| p.source == Source::Git));
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::CargolockCore),
            plugin_api::PluginPresentation::extensions(&crate::CargolockPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
