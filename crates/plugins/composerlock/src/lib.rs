//! Composer lock file file type plugin: core and presentation halves.
//!
//! A `composer.lock` is JSON, so the JSON plugin recognises it and
//! would show a reader a tree. What they want instead is what was
//! resolved: the packages, which of them are only needed while working
//! on the project, where each came from, and the platform requirements
//! - which are not packages at all and cannot be installed, only met.
//!
//! The content hash is the other half: it is what tells Composer
//! whether the `composer.json` beside it has changed since.

use plugin_api::{Icon, PluginCore, PluginPresentation, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;
use syntax::{Language, Quote};

/// The lowercase extensions this type claims, without their dot.
///
/// Deliberately empty. The file is named `composer.lock`, and `lock`
/// belongs to the Cargo lock plugin; the content decides this one.
pub const EXTENSIONS: &[&str] = &[];

/// How many packages are listed before the rest are only counted.
const SHOWN: usize = 64;

/// One resolved package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    /// Its vendor and name.
    pub name: String,
    /// The version resolved.
    pub version: String,
    /// Where it came from.
    pub source: Option<String>,
    /// What kind of thing it is: a library, an application, a plugin.
    pub kind: Option<String>,
    /// Whether it is only needed while working on the project.
    pub development: bool,
}

/// View data produced by [`ComposerlockCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposerlockView {
    /// The hash of the `composer.json` this was resolved from.
    pub content_hash: Option<String>,
    /// The packages, production first.
    pub packages: Vec<Package>,
    /// How many there are in all.
    pub package_count: usize,
    /// How many of those are development-only.
    pub development_count: usize,
    /// What the machine itself has to provide: PHP and its extensions.
    pub platform: Vec<String>,
    /// The same, for working on the project.
    pub platform_development: Vec<String>,
    /// How stable a release has to be before Composer will take it.
    pub minimum_stability: Option<String>,
    /// Whether a stable release is preferred when one exists.
    pub prefer_stable: Option<bool>,
    /// The plugin interface version it was written against.
    pub plugin_api_version: Option<String>,
}

/// Whether `text` reads like a Composer lock file.
fn looks_like_it(text: &str) -> bool {
    text.trim_start().starts_with('{')
        && text.contains("\"content-hash\"")
        && (text.contains("\"packages\"") || text.contains("\"_readme\""))
}

/// The packages in one array, marked development or not.
fn packages_in(value: Option<&Value>, development: bool) -> Vec<Package> {
    value
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    Some(Package {
                        name: entry.get("name")?.as_str()?.to_owned(),
                        version: entry
                            .get("version")
                            .and_then(Value::as_str)
                            .unwrap_or("?")
                            .to_owned(),
                        source: entry
                            .get("source")
                            .and_then(|source| source.get("url"))
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        kind: entry
                            .get("type")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        development,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The `name: constraint` pairs of a requirement object.
fn requirements_in(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_object)
        .map(|entries| {
            entries
                .iter()
                .map(|(name, constraint)| format!("{name} {}", constraint.as_str().unwrap_or("*")))
                .collect()
        })
        .unwrap_or_default()
}

/// Everything [`ComposerlockView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<ComposerlockView> {
    let source = std::fs::read_to_string(path)?;
    if !looks_like_it(&source) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a Composer lock file",
        ));
    }
    let document: Value = serde_json::from_str(&source)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    let mut packages = packages_in(document.get("packages"), false);
    let development = packages_in(document.get("packages-dev"), true);
    let development_count = development.len();
    packages.extend(development);
    let package_count = packages.len();
    packages.truncate(SHOWN);

    Ok(ComposerlockView {
        content_hash: document
            .get("content-hash")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        packages,
        package_count,
        development_count,
        platform: requirements_in(document.get("platform")),
        platform_development: requirements_in(document.get("platform-dev")),
        minimum_stability: document
            .get("minimum-stability")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        prefer_stable: document.get("prefer-stable").and_then(Value::as_bool),
        plugin_api_version: document
            .get("plugin-api-version")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

/// The Composer lock file plugin's core half.
#[derive(Debug, Default)]
pub struct ComposerlockCore;

/// How this language is coloured, for the shared tokeniser. GUIDANCE.md
/// §3.6: the plugin describes its own format, the pane paints what it is
/// told.
const COMPOSERLOCK: Language = Language {
    line_comment: &[],
    block_comment: &[],
    quotes: &[Quote::simple('"'), Quote::simple('\'')],
    keywords: &["false", "null", "true"],
    types: &[],
    calls: false,
    ignore_case: false,
};

impl PluginCore for ComposerlockCore {
    fn name(&self) -> &'static str {
        "composerlock"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A lock file is JSON, which `json` recognises. This is the
        // narrower reading of the same bytes (D13).
        &["json"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Composer lock file plugin's presentation half.
#[derive(Debug, Default)]
pub struct ComposerlockPresentation;

impl PluginPresentation for ComposerlockPresentation {
    fn classify(&self, text: &str) -> Vec<Span> {
        syntax::classify(text, &COMPOSERLOCK)
    }

    fn name(&self) -> &'static str {
        "composerlock"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "LOCK",
            tint: 0x0059_5a9e,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: ComposerlockView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let production = view.package_count - view.development_count;
        let mut lines = vec![format!(
            "Composer lock: {production} package(s) installed in production, \
             {} more for working on it",
            view.development_count
        )];
        if let Some(hash) = &view.content_hash {
            lines.push(format!("Resolved from a composer.json hashing to {hash}"));
        }
        match (&view.minimum_stability, view.prefer_stable) {
            (Some(stability), Some(true)) => lines.push(format!(
                "Takes nothing below {stability}, and prefers a stable release"
            )),
            (Some(stability), _) => lines.push(format!("Takes nothing below {stability}")),
            (None, _) => {}
        }
        if let Some(version) = &view.plugin_api_version {
            lines.push(format!("Plugin interface {version}"));
        }
        if view.platform.is_empty() {
            lines.push("Asks nothing of the machine itself.".to_owned());
        } else {
            lines.push("The machine itself has to provide:".to_owned());
            for requirement in &view.platform {
                lines.push(format!("  {requirement}"));
            }
        }
        if !view.platform_development.is_empty() {
            lines.push("and, to work on it:".to_owned());
            for requirement in &view.platform_development {
                lines.push(format!("  {requirement}"));
            }
        }
        lines.push("Packages:".to_owned());
        for package in &view.packages {
            let development = if package.development { " [dev]" } else { "" };
            lines.push(format!(
                "  {} {}{development}",
                package.name, package.version
            ));
            if let Some(kind) = &package.kind
                && kind != "library"
            {
                lines.push(format!("      {kind}, not a library"));
            }
        }
        if view.package_count > view.packages.len() {
            lines.push(format!(
                "  ... and {} more",
                view.package_count - view.packages.len()
            ));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{ComposerlockCore, ComposerlockPresentation, ComposerlockView, looks_like_it};
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/composerlock/composer.lock")
    }

    fn view_of() -> ComposerlockView {
        serde_json::from_value(ComposerlockCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&ComposerlockCore),
            PluginPresentation::extensions(&ComposerlockPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn it_says_it_specialises_the_json_reading() {
        assert_eq!(ComposerlockCore.specialises(), &["json"]);
        assert!(PluginCore::extensions(&ComposerlockCore).is_empty());
    }

    #[test]
    fn recognises_the_content_hash_rather_than_the_json() {
        assert!(looks_like_it(
            "{\"content-hash\": \"abc\", \"packages\": []}"
        ));
        assert!(
            !looks_like_it("{\"name\": \"a\", \"packages\": []}"),
            "any JSON with a packages array is not this"
        );
        assert!(!looks_like_it(""));
    }

    #[test]
    fn keeps_production_and_development_packages_apart() {
        let view = view_of();

        assert_eq!(view.package_count, 5);
        assert_eq!(view.development_count, 2);
        let production: Vec<&str> = view
            .packages
            .iter()
            .filter(|one| !one.development)
            .map(|one| one.name.as_str())
            .collect();
        assert_eq!(
            production,
            vec!["monolog/monolog", "psr/log", "league/csv"],
            "and they come first"
        );
        assert!(
            view.packages
                .iter()
                .filter(|one| one.development)
                .all(|one| one.name.contains('/'))
        );
    }

    #[test]
    fn reads_each_package_with_its_version_source_and_kind() {
        let view = view_of();

        let monolog = &view.packages[0];
        assert_eq!(monolog.version, "3.7.0");
        assert_eq!(
            monolog.source.as_deref(),
            Some("https://github.com/Seldaek/monolog.git")
        );
        assert_eq!(monolog.kind.as_deref(), Some("library"));

        let fixer = view
            .packages
            .iter()
            .find(|one| one.name == "friendsofphp/php-cs-fixer")
            .expect("the application");
        assert_eq!(fixer.kind.as_deref(), Some("application"));
        assert!(fixer.development);
    }

    #[test]
    fn reads_the_platform_requirements_which_are_not_packages() {
        let view = view_of();

        assert_eq!(view.platform.len(), 3);
        assert!(view.platform.iter().any(|one| one == "php >=8.1"));
        assert!(view.platform.iter().any(|one| one == "ext-json *"));
        assert_eq!(view.platform_development, vec!["ext-xdebug *"]);
    }

    #[test]
    fn reads_the_hash_and_the_stability() {
        let view = view_of();

        assert_eq!(
            view.content_hash.as_deref(),
            Some("9c1f4b2a7d6e5038c1b2a3d4e5f60718")
        );
        assert_eq!(view.minimum_stability.as_deref(), Some("stable"));
        assert_eq!(view.prefer_stable, Some(true));
        assert_eq!(view.plugin_api_version.as_deref(), Some("2.6.0"));
    }

    #[test]
    fn presents_what_is_installed_where() {
        let data = ComposerlockCore.view(&fixture()).unwrap();

        let lines = ComposerlockPresentation.present(&data);

        assert!(lines[0].starts_with("Composer lock: 3 package(s) installed in production"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("The machine itself has to provide"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("phpunit/phpunit 11.3.1 [dev]"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("application, not a library"))
        );
    }

    #[test]
    fn a_file_that_is_not_a_lock_file_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really-composer.lock");
        std::fs::write(&path, b"{\"nothing\": \"of the sort\"}").unwrap();

        assert!(ComposerlockCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
