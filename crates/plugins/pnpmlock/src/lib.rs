//! pnpm lock file file type plugin: core and presentation halves.
//!
//! A pnpm lock file resolves a whole workspace at once. This reads its
//! format version, each project and how much of its dependency list is
//! development-only, every locked package, the peer resolutions, and the
//! packages served through a local patch.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One workspace project the lock file resolves for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Importer {
    /// Its path relative to the workspace root; `.` is the root itself.
    pub path: String,
    /// How many dependencies it declares.
    pub dependencies: usize,
    /// How many of its dependencies are development-only.
    pub development: usize,
}

/// View data produced by [`PnpmlockCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PnpmlockView {
    /// The lock file's own format version.
    pub format: String,
    /// Every workspace project it resolves for.
    pub importers: Vec<Importer>,
    /// Every locked package, as `name@version`.
    pub packages: Vec<String>,
    /// The packages reached only through a development dependency.
    pub development_only: Vec<String>,
    /// Packages that resolve a peer dependency, and what they resolve it
    /// against - the part of a lock file that changes under you when a
    /// sibling's version moves.
    pub peer_resolutions: Vec<String>,
    /// Packages served through a local patch.
    pub patched: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// How many spaces `line` opens with.
fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The key a mapping line names, without its trailing colon or quotes.
fn key_of(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
        return None;
    }
    let (key, _) = trimmed.split_once(':')?;
    Some(key.trim().trim_matches('"').trim_matches('\''))
}

/// The scalar a `key: value` line carries, if it carries one.
fn scalar_of(line: &str) -> Option<&str> {
    let (_, value) = line.trim().split_once(':')?;
    let value = value.trim().trim_matches('"').trim_matches('\'');
    (!value.is_empty()).then_some(value)
}

/// Reads the `importers:` block, returning where it ended.
fn read_importers(
    lines: &[&str],
    from: usize,
    view: &mut PnpmlockView,
    plain: &mut Vec<String>,
) -> usize {
    let mut at = from;
    let mut development = false;
    while at < lines.len() {
        let line = lines[at];
        if !line.trim().is_empty() && indent(line) == 0 {
            break;
        }
        match indent(line) {
            2 => {
                if let Some(path) = key_of(line) {
                    view.importers.push(Importer {
                        path: path.to_owned(),
                        dependencies: 0,
                        development: 0,
                    });
                }
                development = false;
            }
            4 => development = key_of(line) == Some("devDependencies"),
            6 => {
                if let (Some(name), Some(importer)) = (key_of(line), view.importers.last_mut()) {
                    importer.dependencies += 1;
                    if development {
                        importer.development += 1;
                        view.development_only.push(name.to_owned());
                    } else {
                        plain.push(name.to_owned());
                    }
                }
            }
            _ => {}
        }
        at += 1;
    }
    at
}

/// Reads the `packages:` or `snapshots:` block, returning where it ended.
fn read_packages(lines: &[&str], from: usize, view: &mut PnpmlockView) -> usize {
    let mut at = from;
    let mut package = String::new();
    let mut in_peers = false;
    while at < lines.len() {
        let line = lines[at];
        if !line.trim().is_empty() && indent(line) == 0 {
            break;
        }
        if indent(line) == 2 {
            if let Some(key) = key_of(line) {
                key.clone_into(&mut package);
                if !view.packages.contains(&package) {
                    view.packages.push(package.clone());
                }
            }
            in_peers = false;
        } else if indent(line) == 4 {
            in_peers = key_of(line) == Some("peerDependencies");
        } else if in_peers
            && indent(line) == 6
            && let (Some(peer), Some(against)) = (key_of(line), scalar_of(line))
        {
            view.peer_resolutions
                .push(format!("{package} needs {peer} {against}"));
        }
        at += 1;
    }
    at
}

/// Reads the `patchedDependencies:` block, returning where it ended.
fn read_patched(lines: &[&str], from: usize, view: &mut PnpmlockView) -> usize {
    let mut at = from;
    while at < lines.len() {
        let line = lines[at];
        if !line.trim().is_empty() && indent(line) == 0 {
            break;
        }
        if indent(line) == 2
            && let Some(key) = key_of(line)
        {
            view.patched.push(key.to_owned());
        }
        at += 1;
    }
    at
}

/// Everything [`PnpmlockView`] holds, read from `text`.
fn parse(text: &str) -> PnpmlockView {
    let mut view = PnpmlockView {
        format: "unstated".to_owned(),
        importers: Vec::new(),
        packages: Vec::new(),
        development_only: Vec::new(),
        peer_resolutions: Vec::new(),
        patched: Vec::new(),
        truncated: false,
    };
    let lines: Vec<&str> = text.lines().collect();
    // Names some project declares as a plain dependency, which is what
    // makes "development-only" mean anything.
    let mut plain: Vec<String> = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        let line = lines[at];
        if line.trim().is_empty() || indent(line) > 0 {
            at += 1;
            continue;
        }
        at += 1;
        match key_of(line) {
            Some("lockfileVersion") => {
                if let Some(version) = scalar_of(line) {
                    version.clone_into(&mut view.format);
                }
            }
            Some("importers") => at = read_importers(&lines, at, &mut view, &mut plain),
            Some("packages" | "snapshots") => at = read_packages(&lines, at, &mut view),
            Some("patchedDependencies") => at = read_patched(&lines, at, &mut view),
            _ => {}
        }
    }
    // A package one project builds with and another ships is not
    // development-only, whatever the first project called it.
    view.development_only.retain(|name| !plain.contains(name));
    view.development_only.sort_unstable();
    view.development_only.dedup();
    view
}

/// Whether `text` is a pnpm lock file.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    // The version alone is not enough: npm writes `lockfileVersion` too,
    // and so does a note about lock files. It has to have resolved
    // something under pnpm's own headings.
    view.format != "unstated" && (!view.importers.is_empty() || !view.packages.is_empty())
}

/// The pnpm lock file plugin's core half.
#[derive(Debug, Default)]
pub struct PnpmlockCore;

impl PluginCore for PnpmlockCore {
    fn name(&self) -> &'static str {
        "pnpmlock"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A pnpm lock file is YAML, and `yaml` owns the extension. Without
        // this the extension hint hands it over regardless of order (D13).
        &["yaml"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        // A lock file is machine-written and long; what a reader wants
        // is the summary, not the ten thousand lines.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The pnpm lock file plugin's presentation half.
#[derive(Debug, Default)]
pub struct PnpmlockPresentation;

impl PluginPresentation for PnpmlockPresentation {
    fn name(&self) -> &'static str {
        "pnpmlock"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "PNPM",
            tint: 0x00f9_ad00,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: PnpmlockView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("pnpm lock file version {}", view.format));
        lines.push(format!("{} workspace project(s):", view.importers.len()));
        for importer in &view.importers {
            lines.push(format!(
                "  {} - {} dependency(ies), {} development-only",
                importer.path, importer.dependencies, importer.development
            ));
        }
        lines.push(format!("{} locked package(s):", view.packages.len()));
        for package in &view.packages {
            lines.push(format!("  {package}"));
        }
        if !view.patched.is_empty() {
            lines.push("Served through a local patch, so the code that runs is not".to_owned());
            lines.push("what the registry publishes:".to_owned());
            for name in &view.patched {
                lines.push(format!("  {name}"));
            }
        }
        if !view.peer_resolutions.is_empty() {
            lines.push("Peer dependencies, which resolve against a sibling and so".to_owned());
            lines.push("move when that sibling does:".to_owned());
            for resolution in &view.peer_resolutions {
                lines.push(format!("  {resolution}"));
            }
        }
        if !view.development_only.is_empty() {
            lines.push(format!(
                "Development-only: {}",
                view.development_only.join(", ")
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
    use super::{PnpmlockCore, PnpmlockPresentation, PnpmlockView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const LOCK: &str = concat!(
        "lockfileVersion: '9.0'\n",
        "\n",
        "patchedDependencies:\n",
        "  left-pad@1.3.0:\n",
        "    hash: aaaa\n",
        "\n",
        "importers:\n",
        "\n",
        "  .:\n",
        "    dependencies:\n",
        "      react:\n",
        "        specifier: ^18.2.0\n",
        "        version: 18.2.0\n",
        "    devDependencies:\n",
        "      typescript:\n",
        "        specifier: ^5.4.0\n",
        "        version: 5.4.5\n",
        "\n",
        "  packages/ui:\n",
        "    dependencies:\n",
        "      '@scope/tool':\n",
        "        specifier: ^2.0.0\n",
        "        version: 2.1.0\n",
        "\n",
        "packages:\n",
        "\n",
        "  react@18.2.0:\n",
        "    resolution: {integrity: sha512-aaaa}\n",
        "\n",
        "  react-dom@18.2.0:\n",
        "    resolution: {integrity: sha512-bbbb}\n",
        "    peerDependencies:\n",
        "      react: ^18.2.0\n",
    );

    const LOCK_WITH_SCOPE: &str = concat!(
        "lockfileVersion: '9.0'\n",
        "\n",
        "packages:\n",
        "\n",
        "  '@scope/tool@2.1.0':\n",
        "    resolution: {integrity: sha512-aaaa}\n",
    );

    #[test]
    fn sniffs_a_lock_file() {
        assert!(PnpmlockCore.sniff(LOCK.as_bytes()));
    }

    #[test]
    fn does_not_claim_an_npm_lock_file() {
        // npm writes `lockfileVersion` too, and it is not YAML.
        assert!(
            !PnpmlockCore
                .sniff(br#"{"name":"app","lockfileVersion":3,"packages":{"":{"name":"app"}}}"#)
        );
        assert!(!PnpmlockCore.sniff(b"lockfileVersion: '9.0'\n"));
        assert!(!PnpmlockCore.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_yaml() {
        assert_eq!(PnpmlockCore.specialises(), &["yaml"]);
    }

    #[test]
    fn counts_each_project_separately() {
        let view = parse(LOCK);

        assert_eq!(view.format, "9.0");
        assert_eq!(view.importers.len(), 2);
        assert_eq!(view.importers[0].path, ".");
        assert_eq!(view.importers[0].dependencies, 2);
        assert_eq!(view.importers[0].development, 1);
        assert_eq!(view.importers[1].path, "packages/ui");
        assert_eq!(view.importers[1].development, 0);
    }

    #[test]
    fn a_development_block_does_not_leak_into_the_next_project() {
        let view = parse(LOCK);

        assert_eq!(
            view.development_only,
            vec!["typescript".to_owned()],
            "`@scope/tool` is a plain dependency of the second project"
        );
    }

    #[test]
    fn a_scoped_package_key_is_kept_whole() {
        let view = parse(LOCK_WITH_SCOPE);

        assert_eq!(
            view.packages,
            vec!["@scope/tool@2.1.0".to_owned()],
            "the scope, the name and the version are one key"
        );
    }

    #[test]
    fn reads_the_peer_resolutions_and_the_patches() {
        let view = parse(LOCK);

        assert_eq!(view.packages.len(), 2);
        assert_eq!(
            view.peer_resolutions,
            vec!["react-dom@18.2.0 needs react ^18.2.0".to_owned()]
        );
        assert_eq!(view.patched, vec!["left-pad@1.3.0".to_owned()]);
    }

    #[test]
    fn presents_the_peer_warning_with_its_reason() {
        let data = serde_json::to_value(parse(LOCK)).unwrap();

        let lines = PnpmlockPresentation.present(&data);

        assert_eq!(lines[0], "pnpm lock file version 9.0");
        assert!(
            lines
                .iter()
                .any(|line| line.contains("move when that sibling does"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("what the registry publishes"))
        );
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/pnpmlock/pnpm-lock.yaml");

        let data = PnpmlockCore.view(&path).unwrap();
        let view: PnpmlockView = serde_json::from_value(data).unwrap();

        assert_eq!(view.format, "9.0");
        assert!(view.importers.len() >= 2);
        assert!(view.importers.iter().any(|i| i.development > 0));
        assert!(view.packages.len() >= 4);
        assert!(!view.development_only.is_empty());
        assert!(!view.peer_resolutions.is_empty());
        assert!(!view.patched.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::PnpmlockCore),
            plugin_api::PluginPresentation::extensions(&crate::PnpmlockPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
