//! The `package_manifests` binary, run the way `pages.yml` runs it: a dry
//! run against a dist directory, so a broken manifest fails the build rather
//! than a reader's `winget install` or `scoop install`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rse-package-manifests-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A dist directory holding the two files this release publishes that the
/// generator reads: the Windows installer and the macOS disk image.
fn dist_dir(name: &str) -> PathBuf {
    let dir = scratch(name);
    std::fs::write(dir.join("ReposExplorerSetup.exe"), b"the windows installer").unwrap();
    std::fs::write(dir.join("ReposExplorer.dmg"), b"the macos disk image").unwrap();
    dir
}

fn run(version: &str, repo: &str, tag: &str, dist: &Path, output: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_package_manifests"))
        .args([version, repo, tag])
        .arg(dist)
        .arg(output)
        .output()
        .unwrap()
}

#[test]
fn a_release_produces_a_valid_manifest_for_every_channel() {
    let dist = dist_dir("full-release");
    let output = scratch("full-release-out");

    let result = run("0.9.0", "owner/repo", "v0.9.0", &dist, &output);

    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let winget = std::fs::read_to_string(
        output
            .join("winget")
            .join("StewartScottRogers.RepoSphereExplorer.yaml"),
    )
    .unwrap();
    assert!(winget.contains("PackageVersion: 0.9.0"), "{winget}");
    assert!(
        winget.contains(
            "InstallerUrl: https://github.com/owner/repo/releases/download/v0.9.0/ReposExplorerSetup.exe"
        ),
        "{winget}"
    );

    let scoop =
        std::fs::read_to_string(output.join("scoop").join("repo-sphere-explorer.json")).unwrap();
    assert!(scoop.contains("\"version\": \"0.9.0\""), "{scoop}");

    let homebrew =
        std::fs::read_to_string(output.join("homebrew").join("repo-sphere-explorer.rb")).unwrap();
    assert!(homebrew.contains("version \"0.9.0\""), "{homebrew}");
    assert!(
        homebrew.contains(
            "url \"https://github.com/owner/repo/releases/download/v0.9.0/ReposExplorer.dmg\""
        ),
        "{homebrew}"
    );

    std::fs::remove_dir_all(&dist).unwrap();
    std::fs::remove_dir_all(&output).unwrap();
}

#[test]
fn a_different_tag_produces_manifests_naming_that_tags_own_assets() {
    let dist = dist_dir("other-tag");
    let output = scratch("other-tag-out");

    let result = run("1.2.3", "owner/repo", "v1.2.3", &dist, &output);

    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let winget = std::fs::read_to_string(
        output
            .join("winget")
            .join("StewartScottRogers.RepoSphereExplorer.yaml"),
    )
    .unwrap();
    assert!(winget.contains("PackageVersion: 1.2.3"), "{winget}");
    assert!(winget.contains("/v1.2.3/"), "{winget}");
    assert!(!winget.contains("0.9.0"), "{winget}");

    std::fs::remove_dir_all(&dist).unwrap();
    std::fs::remove_dir_all(&output).unwrap();
}

#[test]
fn a_dist_directory_missing_the_windows_installer_fails_the_build() {
    let dist = scratch("missing-installer");
    std::fs::write(dist.join("ReposExplorer.dmg"), b"the macos disk image").unwrap();
    let output = scratch("missing-installer-out");

    let result = run("0.9.0", "owner/repo", "v0.9.0", &dist, &output);

    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("ReposExplorerSetup.exe"),
        "stderr: {stderr}"
    );
    assert!(
        !output.join("winget").exists(),
        "nothing must be published on failure"
    );

    std::fs::remove_dir_all(&dist).unwrap();
    std::fs::remove_dir_all(&output).unwrap();
}

#[test]
fn a_dist_directory_missing_the_macos_disk_image_fails_the_build() {
    let dist = scratch("missing-disk-image");
    std::fs::write(
        dist.join("ReposExplorerSetup.exe"),
        b"the windows installer",
    )
    .unwrap();
    let output = scratch("missing-disk-image-out");

    let result = run("0.9.0", "owner/repo", "v0.9.0", &dist, &output);

    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("ReposExplorer.dmg"), "stderr: {stderr}");

    std::fs::remove_dir_all(&dist).unwrap();
    std::fs::remove_dir_all(&output).unwrap();
}
