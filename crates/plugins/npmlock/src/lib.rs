//! npm lock file file type plugin: core and presentation halves.
//!
//! Registered before `json`, which would otherwise claim it on the
//! extension. `lockfileVersion` is a key no other JSON document carries.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One locked package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    /// The path within `node_modules`, or the empty string for the root.
    pub path: String,
    /// The exact version locked.
    pub version: String,
    /// Whether it is needed only to develop, not to run.
    pub dev: bool,
    /// Whether the entry carries an integrity hash.
    pub integrity: bool,
}

/// View data produced by [`NpmlockCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpmlockView {
    /// The lock file version: 1, 2 or 3.
    pub format: u64,
    /// The root package's name.
    pub name: Option<String>,
    /// The root package's version.
    pub version: Option<String>,
    /// Every locked package below the root.
    pub packages: Vec<Package>,
    /// How many are development-only.
    pub dev_count: usize,
    /// The registries the resolved addresses point at.
    pub registries: Vec<String>,
    /// The scopes used, as `@scope`.
    pub scopes: Vec<String>,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The host of a resolved address, if it has one.
fn host_of(resolved: &str) -> Option<String> {
    let rest = resolved.split("://").nth(1)?;
    rest.split('/').next().map(str::to_owned)
}

/// Everything [`NpmlockView`] holds, read from `text`.
fn parse(text: &str) -> NpmlockView {
    let mut view = NpmlockView {
        format: 0,
        name: None,
        version: None,
        packages: Vec::new(),
        dev_count: 0,
        registries: Vec::new(),
        scopes: Vec::new(),
        truncated: false,
    };
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return view;
    };
    let string =
        |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);

    view.format = root
        .get("lockfileVersion")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    view.name = string(&root, "name");
    view.version = string(&root, "version");

    let Some(packages) = root.get("packages").and_then(Value::as_object) else {
        return view;
    };
    for (path, entry) in packages {
        if path.is_empty() {
            // The root entry restates the manifest; its name and version
            // are already taken from the top level.
            continue;
        }
        let dev = entry.get("dev").and_then(Value::as_bool).unwrap_or(false);
        if dev {
            view.dev_count += 1;
        }
        if let Some(resolved) = entry.get("resolved").and_then(Value::as_str)
            && let Some(host) = host_of(resolved)
            && !view.registries.contains(&host)
        {
            view.registries.push(host);
        }
        if let Some(scope) = path
            .rsplit("node_modules/")
            .next()
            .and_then(|name| name.split('/').next())
            .filter(|segment| segment.starts_with('@'))
            && !view.scopes.contains(&scope.to_owned())
        {
            view.scopes.push(scope.to_owned());
        }
        view.packages.push(Package {
            path: path.clone(),
            version: string(entry, "version").unwrap_or_default(),
            dev,
            integrity: entry.get("integrity").is_some(),
        });
    }
    view
}

/// Whether `text` looks like an npm lock file.
fn looks_like_it(text: &str) -> bool {
    // Cheap first: the key has to be in the prefix at all.
    if !text.contains("\"lockfileVersion\"") {
        return false;
    }
    // Then honestly: it has to be a top-level key of an object.
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|root| root.get("lockfileVersion").and_then(Value::as_u64))
        .is_some()
}

/// The npm lock file plugin's core half.
#[derive(Debug, Default)]
pub struct NpmlockCore;

impl PluginCore for NpmlockCore {
    fn name(&self) -> &'static str {
        "npmlock"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A lock file is JSON, and `json` owns the extension. Without
        // this the extension hint hands it over regardless of order.
        &["json"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        // A lock file is machine-written and long; what a reader wants is
        // the summary, not the ten thousand lines. So no `content` here.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The npm lock file plugin's presentation half.
#[derive(Debug, Default)]
pub struct NpmlockPresentation;

impl PluginPresentation for NpmlockPresentation {
    fn name(&self) -> &'static str {
        "npmlock"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "NPM",
            tint: 0x00cb_3837,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: NpmlockView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("npm lock file version {}", view.format));
        if let (Some(name), Some(version)) = (&view.name, &view.version) {
            lines.push(format!("Root package: {name} {version}"));
        }
        lines.push(format!(
            "{} locked package(s), {} of them development-only",
            view.packages.len(),
            view.dev_count
        ));
        let unhashed = view
            .packages
            .iter()
            .filter(|package| !package.integrity)
            .count();
        if unhashed > 0 {
            lines.push(format!(
                "{unhashed} without an integrity hash, which nothing can verify"
            ));
        }
        if !view.registries.is_empty() {
            lines.push(format!("Registries: {}", view.registries.join(", ")));
        }
        if !view.scopes.is_empty() {
            lines.push(format!("Scopes: {}", view.scopes.join(", ")));
        }
        for package in &view.packages {
            let dev = if package.dev { "  (dev)" } else { "" };
            lines.push(format!("  {} {}{dev}", package.path, package.version));
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{NpmlockCore, NpmlockPresentation, NpmlockView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const LOCK: &str = r#"{
      "name": "@example/store",
      "version": "3.1.0",
      "lockfileVersion": 3,
      "packages": {
        "": { "name": "@example/store", "version": "3.1.0" },
        "node_modules/eslint": {
          "version": "9.27.0",
          "resolved": "https://registry.npmjs.org/eslint/-/eslint-9.27.0.tgz",
          "integrity": "sha512-abc",
          "dev": true
        },
        "node_modules/@types/node": {
          "version": "22.15.21",
          "resolved": "https://registry.npmjs.org/@types/node/-/node-22.15.21.tgz",
          "dev": true
        },
        "node_modules/left-pad": { "version": "1.3.0" }
      }
    }"#;

    #[test]
    fn sniffs_a_top_level_lockfile_version() {
        assert!(NpmlockCore.sniff(LOCK.as_bytes()));
    }

    #[test]
    fn does_not_claim_json_that_merely_mentions_the_word() {
        assert!(!NpmlockCore.sniff(br#"{"note": "the lockfileVersion key is missing"}"#));
        assert!(!NpmlockCore.sniff(br#"{"name": "a"}"#));
        assert!(!NpmlockCore.sniff(b""));
    }

    #[test]
    fn reads_the_root_and_skips_its_restated_entry() {
        let view = parse(LOCK);

        assert_eq!(view.format, 3);
        assert_eq!(view.name.as_deref(), Some("@example/store"));
        assert_eq!(
            view.packages.len(),
            3,
            "the empty-path root is not a package"
        );
    }

    #[test]
    fn counts_development_only_packages() {
        let view = parse(LOCK);

        assert_eq!(view.dev_count, 2);
    }

    #[test]
    fn notices_a_package_with_no_integrity_hash() {
        let view = parse(LOCK);

        assert!(view.packages.iter().any(|package| !package.integrity));
        assert!(view.packages.iter().any(|package| package.integrity));
    }

    #[test]
    fn reads_registries_and_scopes() {
        let view = parse(LOCK);

        assert_eq!(view.registries, vec!["registry.npmjs.org".to_owned()]);
        assert_eq!(view.scopes, vec!["@types".to_owned()]);
    }

    #[test]
    fn presents_the_version_first() {
        let data = serde_json::to_value(parse(LOCK)).unwrap();

        let lines = NpmlockPresentation.present(&data);

        assert_eq!(lines[0], "npm lock file version 3");
        assert!(lines.iter().any(|line| line.contains("development-only")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/npmlock/package-lock.json");

        let data = NpmlockCore.view(&path).unwrap();
        let view: NpmlockView = serde_json::from_value(data).unwrap();

        assert_eq!(view.format, 3);
        assert!(view.name.is_some() && view.version.is_some());
        assert!(view.packages.len() >= 5);
        assert!(view.dev_count >= 2);
        assert!(!view.registries.is_empty());
        assert!(!view.scopes.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::NpmlockCore),
            plugin_api::PluginPresentation::extensions(&crate::NpmlockPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
