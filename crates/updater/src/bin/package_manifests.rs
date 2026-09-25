//! Builds install manifests for winget, Scoop and a Homebrew cask from a
//! release's dist files, so a version cannot ship to one channel and not
//! another (GUIDANCE.md §4.2).
//!
//! Usage: `package_manifests <version> <owner/repo> <tag> <dist-dir> <output-dir>`
//! Reads `ReposExplorerSetup.exe` (winget, Scoop) and `ReposExplorer.dmg`
//! (the Homebrew cask) from `<dist-dir>`, matching `release.yml`'s packaging,
//! and writes one manifest per channel under `<output-dir>`. Each manifest is
//! also validated before it is written, so a manifest this binary cannot
//! stand behind is never published.
//!
//! `cargo install` needs no generated manifest: every crate that makes sense
//! as an installed binary already carries the `description`, `license` and
//! `repository` metadata `cargo install --git` reads: see the README's
//! "Package managers" section.

use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::Path;

/// The Windows installer `release.yml` publishes with no target suffix,
/// because there is one Windows build.
const WINDOWS_INSTALLER: &str = "ReposExplorerSetup.exe";
/// The macOS disk image `release.yml` publishes.
const MACOS_DISK_IMAGE: &str = "ReposExplorer.dmg";

const WINGET_PACKAGE_IDENTIFIER: &str = "StewartScottRogers.RepoSphereExplorer";
const HOMEPAGE: &str = "https://github.com/StewartScottRogers/RepoSphereExplorer";
const SHORT_DESCRIPTION: &str = "A cross-platform Repos Explorer for local working directories.";

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let [version, repo, tag, dist_dir, output_dir] = args.as_slice() else {
        eprintln!("usage: package_manifests <version> <owner/repo> <tag> <dist-dir> <output-dir>");
        std::process::exit(2);
    };
    let dist_dir = Path::new(dist_dir);
    let output_dir = Path::new(output_dir);

    let installer_sha256 = hash_dist_file(dist_dir, WINDOWS_INSTALLER);
    let disk_image_sha256 = hash_dist_file(dist_dir, MACOS_DISK_IMAGE);
    let installer_url = asset_url(repo, tag, WINDOWS_INSTALLER);
    let disk_image_url = asset_url(repo, tag, MACOS_DISK_IMAGE);

    let winget = winget_manifest(version, &installer_url, &installer_sha256);
    validate_winget(&winget, version, &installer_url).expect("generated winget manifest");
    write(
        &output_dir
            .join("winget")
            .join(format!("{WINGET_PACKAGE_IDENTIFIER}.yaml")),
        &winget,
    );

    let scoop = scoop_manifest(version, &installer_url, &installer_sha256);
    validate_scoop(&scoop, version, &installer_url).expect("generated Scoop manifest");
    write(
        &output_dir.join("scoop").join("repo-sphere-explorer.json"),
        &scoop,
    );

    let homebrew = homebrew_cask(version, &disk_image_url, &disk_image_sha256);
    validate_homebrew(&homebrew, version, &disk_image_url).expect("generated Homebrew cask");
    write(
        &output_dir.join("homebrew").join("repo-sphere-explorer.rb"),
        &homebrew,
    );

    println!("wrote package manifests to {}", output_dir.display());
}

fn hash_dist_file(dist_dir: &Path, filename: &str) -> String {
    let path = dist_dir.join(filename);
    let bytes = fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    updater::sha256_hex(&bytes)
}

fn asset_url(repo: &str, tag: &str, filename: &str) -> String {
    format!("https://github.com/{repo}/releases/download/{tag}/{filename}")
}

fn write(path: &Path, contents: &str) {
    let dir = path.parent().expect("manifest path has a parent directory");
    fs::create_dir_all(dir).unwrap_or_else(|err| panic!("create {}: {err}", dir.display()));
    fs::write(path, contents).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
}

// ---- winget: a singleton manifest, installed locally with `winget install
// --manifest`. Not submitted to the winget-pkgs community repository, which
// is a separate, externally-owned project this generator does not touch. ----

fn winget_manifest(version: &str, installer_url: &str, sha256: &str) -> String {
    [
        format!("PackageIdentifier: {WINGET_PACKAGE_IDENTIFIER}"),
        format!("PackageVersion: {version}"),
        "PackageName: Repos Explorer".to_owned(),
        "Publisher: StewartScottRogers".to_owned(),
        "License: MIT".to_owned(),
        format!("ShortDescription: {SHORT_DESCRIPTION}"),
        "Installers:".to_owned(),
        "  - Architecture: x64".to_owned(),
        "    InstallerType: exe".to_owned(),
        format!("    InstallerUrl: {installer_url}"),
        format!("    InstallerSha256: {}", sha256.to_uppercase()),
        "    InstallerSwitches:".to_owned(),
        "      Silent: --quiet".to_owned(),
        "ManifestType: singleton".to_owned(),
        "ManifestVersion: 1.6.0".to_owned(),
        String::new(),
    ]
    .join("\n")
}

/// Checks that `yaml` carries every field a reader's `winget install
/// --manifest` needs, with the version and installer this release built.
fn validate_winget(yaml: &str, version: &str, installer_url: &str) -> Result<(), String> {
    let required = [
        format!("PackageIdentifier: {WINGET_PACKAGE_IDENTIFIER}"),
        format!("PackageVersion: {version}"),
        format!("InstallerUrl: {installer_url}"),
        "InstallerSha256: ".to_owned(),
        "ManifestType: singleton".to_owned(),
    ];
    for line in required {
        if !yaml.contains(&line) {
            return Err(format!("winget manifest is missing {line:?}"));
        }
    }
    Ok(())
}

// ---- Scoop: a manifest installed directly, with `scoop install <url>`,
// rather than through a bucket this repository would have to maintain a
// second copy of. ----

#[derive(Serialize, Deserialize)]
struct ScoopManifest {
    version: String,
    description: String,
    homepage: String,
    license: String,
    url: String,
    hash: String,
    installer: ScoopHook,
    uninstaller: ScoopHook,
}

#[derive(Serialize, Deserialize)]
struct ScoopHook {
    args: Vec<String>,
}

fn scoop_manifest(version: &str, installer_url: &str, sha256: &str) -> String {
    let manifest = ScoopManifest {
        version: version.to_owned(),
        description: SHORT_DESCRIPTION.to_owned(),
        homepage: HOMEPAGE.to_owned(),
        license: "MIT".to_owned(),
        url: installer_url.to_owned(),
        hash: format!("sha256:{sha256}"),
        installer: ScoopHook {
            args: vec![
                "--quiet".to_owned(),
                "--prefix".to_owned(),
                "$dir".to_owned(),
            ],
        },
        uninstaller: ScoopHook {
            args: vec![
                "--uninstall".to_owned(),
                "--prefix".to_owned(),
                "$dir".to_owned(),
            ],
        },
    };
    serde_json::to_string_pretty(&manifest).expect("serialize Scoop manifest") + "\n"
}

/// Checks that `json` parses back as a [`ScoopManifest`] naming the version
/// and installer this release built - a genuine round trip through the
/// structure a malformed manifest could not survive.
fn validate_scoop(json: &str, version: &str, installer_url: &str) -> Result<(), String> {
    let parsed: ScoopManifest = serde_json::from_str(json)
        .map_err(|err| format!("Scoop manifest does not parse: {err}"))?;
    if parsed.version != version {
        return Err(format!(
            "Scoop manifest version {:?} does not match release version {version:?}",
            parsed.version
        ));
    }
    if parsed.url != installer_url {
        return Err(format!(
            "Scoop manifest url {:?} does not match the release's installer",
            parsed.url
        ));
    }
    Ok(())
}

// ---- Homebrew: a cask installed directly, with `brew install --cask
// <url-or-path>`, rather than through a tap this repository would have to
// maintain a second copy of. ----

fn homebrew_cask(version: &str, disk_image_url: &str, sha256: &str) -> String {
    [
        "cask \"repo-sphere-explorer\" do".to_owned(),
        format!("  version \"{version}\""),
        format!("  sha256 \"{sha256}\""),
        String::new(),
        format!("  url \"{disk_image_url}\""),
        "  name \"Repos Explorer\"".to_owned(),
        format!("  desc \"{}\"", SHORT_DESCRIPTION.trim_end_matches('.')),
        format!("  homepage \"{HOMEPAGE}\""),
        String::new(),
        "  app \"Repos Explorer.app\"".to_owned(),
        "end".to_owned(),
        String::new(),
    ]
    .join("\n")
}

/// Checks that `ruby` carries the cask's required stanzas, with the version
/// and disk image this release built. Line-based rather than a Ruby parse:
/// this generator's own output is the only Ruby it ever has to read back.
fn validate_homebrew(ruby: &str, version: &str, disk_image_url: &str) -> Result<(), String> {
    let required = [
        "cask \"repo-sphere-explorer\" do".to_owned(),
        format!("  version \"{version}\""),
        format!("  url \"{disk_image_url}\""),
        "  sha256 \"".to_owned(),
        "  app \"Repos Explorer.app\"".to_owned(),
        "end".to_owned(),
    ];
    for line in required {
        if !ruby.contains(&line) {
            return Err(format!("Homebrew cask is missing {line:?}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        homebrew_cask, scoop_manifest, validate_homebrew, validate_scoop, validate_winget,
        winget_manifest,
    };

    const VERSION: &str = "0.9.0";
    const INSTALLER_URL: &str =
        "https://github.com/owner/repo/releases/download/v0.9.0/ReposExplorerSetup.exe";
    const DISK_IMAGE_URL: &str =
        "https://github.com/owner/repo/releases/download/v0.9.0/ReposExplorer.dmg";
    const SHA256: &str = "aabbccdd";

    #[test]
    fn winget_manifest_carries_the_releases_own_version_and_installer() {
        let yaml = winget_manifest(VERSION, INSTALLER_URL, SHA256);
        assert!(validate_winget(&yaml, VERSION, INSTALLER_URL).is_ok());
        assert!(yaml.contains("InstallerSha256: AABBCCDD"), "{yaml}");
    }

    #[test]
    fn winget_validation_rejects_a_manifest_naming_a_different_version() {
        let yaml = winget_manifest("0.8.0", INSTALLER_URL, SHA256);
        assert!(validate_winget(&yaml, VERSION, INSTALLER_URL).is_err());
    }

    #[test]
    fn winget_validation_rejects_text_that_is_not_a_manifest_at_all() {
        assert!(validate_winget("not a manifest", VERSION, INSTALLER_URL).is_err());
    }

    #[test]
    fn scoop_manifest_round_trips_with_the_releases_own_version_and_url() {
        let json = scoop_manifest(VERSION, INSTALLER_URL, SHA256);
        assert!(validate_scoop(&json, VERSION, INSTALLER_URL).is_ok());
        assert!(json.contains(&format!("sha256:{SHA256}")), "{json}");
    }

    #[test]
    fn scoop_validation_rejects_a_manifest_naming_a_different_version() {
        let json = scoop_manifest("0.8.0", INSTALLER_URL, SHA256);
        assert!(validate_scoop(&json, VERSION, INSTALLER_URL).is_err());
    }

    #[test]
    fn scoop_validation_rejects_malformed_json() {
        assert!(validate_scoop("{ not json", VERSION, INSTALLER_URL).is_err());
    }

    #[test]
    fn homebrew_cask_carries_the_releases_own_version_and_disk_image() {
        let ruby = homebrew_cask(VERSION, DISK_IMAGE_URL, SHA256);
        assert!(validate_homebrew(&ruby, VERSION, DISK_IMAGE_URL).is_ok());
        assert!(ruby.contains(&format!("sha256 \"{SHA256}\"")), "{ruby}");
    }

    #[test]
    fn homebrew_validation_rejects_a_cask_naming_a_different_version() {
        let ruby = homebrew_cask("0.8.0", DISK_IMAGE_URL, SHA256);
        assert!(validate_homebrew(&ruby, VERSION, DISK_IMAGE_URL).is_err());
    }

    #[test]
    fn homebrew_validation_rejects_prose_that_is_not_a_cask() {
        assert!(validate_homebrew("not a cask", VERSION, DISK_IMAGE_URL).is_err());
    }

    #[test]
    fn a_different_release_produces_manifests_with_no_trace_of_the_old_one() {
        let old = winget_manifest("0.8.0", INSTALLER_URL, SHA256);
        let new = winget_manifest(VERSION, INSTALLER_URL, SHA256);
        assert!(!new.contains("0.8.0"), "{new}");
        assert_ne!(old, new);
    }
}
