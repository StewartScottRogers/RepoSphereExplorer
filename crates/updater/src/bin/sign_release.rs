//! Builds and signs the update manifest (`latest.json`) for a release.
//!
//! Usage: `sign_release <version> <owner/repo> <tag> <dist-dir> <output-path>`
//! Reads the hex-encoded signing key from the `UPDATER_SIGNING_KEY`
//! environment variable. Scans `<dist-dir>` for files named
//! `<binary>-<target-triple>[.exe]`, matching `release.yml`'s packaging
//! convention, and signs each one.
//!
//! The graphical and terminal apps were published as `gui` and `tui` up to
//! v0.6.0, and those installs still ask the manifest for those names. Each
//! of their files is therefore listed under both its current name and its
//! old one, and v0.6.0's own `gui-*` / `tui-*` files are recognised too.

use ed25519_dalek::{Signer, SigningKey};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::Path;

const KNOWN_BINARIES: &[&str] = &[
    "repo_sphere_explorer",
    "service",
    "RepoSphereExplorerTui",
    "RepoSphereExplorerGui",
    "verify",
    // Not a binary: the icon the macOS application bundle carries, published
    // so that `scripts/install.sh` can build the same bundle the disk image
    // does and hold the icon to the same signature as everything else it
    // places.
    "AppIcon",
];

/// The suffixes a published file's name may carry after its target triple.
const EXTENSIONS: &[&str] = &[".exe", ".icns"];

/// `(current name, name v0.6.0 and earlier asked for)`. A file published
/// under either name is listed in the manifest under both.
const LEGACY_NAMES: &[(&str, &str)] = &[
    ("RepoSphereExplorerGui", "gui"),
    ("RepoSphereExplorerTui", "tui"),
];

#[derive(Serialize)]
struct Manifest {
    version: String,
    targets: Vec<TargetAsset>,
}

#[derive(Serialize)]
struct TargetAsset {
    binary: String,
    target: String,
    url: String,
    sha256: String,
    signature: String,
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let [version, repo, tag, dist_dir, output_path] = args.as_slice() else {
        eprintln!("usage: sign_release <version> <owner/repo> <tag> <dist-dir> <output-path>");
        std::process::exit(2);
    };

    let signing_key_hex = env::var("UPDATER_SIGNING_KEY").expect("UPDATER_SIGNING_KEY must be set");
    let secret_bytes: [u8; 32] = hex_decode(signing_key_hex.trim())
        .expect("UPDATER_SIGNING_KEY must be 64 hex characters")
        .try_into()
        .expect("UPDATER_SIGNING_KEY must decode to exactly 32 bytes");
    let signing_key = SigningKey::from_bytes(&secret_bytes);

    let manifest = Manifest {
        version: version.clone(),
        targets: sign_targets(Path::new(dist_dir), repo, tag, &signing_key),
    };
    let json = serde_json::to_string_pretty(&manifest).expect("serialize manifest");
    fs::write(Path::new(output_path), json).expect("write manifest");
    println!("wrote {output_path}");
}

/// Signs every recognised file in `dist_dir`, listing a graphical or
/// terminal app's file under its legacy name as well as its current one.
fn sign_targets(
    dist_dir: &Path,
    repo: &str,
    tag: &str,
    signing_key: &SigningKey,
) -> Vec<TargetAsset> {
    let mut targets = Vec::new();
    for entry in fs::read_dir(dist_dir).expect("read dist dir") {
        let path = entry.expect("dir entry").path();
        if !path.is_file() {
            continue;
        }
        let filename = path
            .file_name()
            .expect("file has a name")
            .to_string_lossy()
            .into_owned();
        let Some((binary, target)) = parse_filename(&filename) else {
            eprintln!("skipping unrecognised dist file: {filename}");
            continue;
        };

        let bytes = fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let digest = Sha256::digest(&bytes);
        let signature = signing_key.sign(&digest);

        let asset = TargetAsset {
            binary,
            target,
            url: format!("https://github.com/{repo}/releases/download/{tag}/{filename}"),
            sha256: hex_encode(&digest),
            signature: hex_encode(&signature.to_bytes()),
        };
        if let Some(&(_, legacy)) = LEGACY_NAMES
            .iter()
            .find(|(current, _)| *current == asset.binary)
        {
            targets.push(TargetAsset {
                binary: legacy.to_owned(),
                target: asset.target.clone(),
                url: asset.url.clone(),
                sha256: asset.sha256.clone(),
                signature: asset.signature.clone(),
            });
        }
        targets.push(asset);
    }
    targets
}

fn parse_filename(filename: &str) -> Option<(String, String)> {
    for binary in KNOWN_BINARIES {
        if let Some(rest) = filename
            .strip_prefix(binary)
            .and_then(|r| r.strip_prefix('-'))
        {
            return Some(((*binary).to_owned(), triple(rest).to_owned()));
        }
    }
    for (current, legacy) in LEGACY_NAMES {
        if let Some(rest) = filename
            .strip_prefix(legacy)
            .and_then(|r| r.strip_prefix('-'))
        {
            return Some(((*current).to_owned(), triple(rest).to_owned()));
        }
    }
    None
}

/// What is left of a published file's name once the binary name has been
/// taken off the front: the target triple, and whatever suffix the platform
/// puts after it.
fn triple(rest: &str) -> &str {
    EXTENSIONS
        .iter()
        .find_map(|extension| rest.strip_suffix(extension))
        .unwrap_or(rest)
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if text.len().is_multiple_of(2) {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
            .collect()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{KNOWN_BINARIES, hex_decode, hex_encode, parse_filename, sign_targets};
    use ed25519_dalek::SigningKey;
    use std::fs;
    use std::path::PathBuf;

    const TRIPLE: &str = "x86_64-pc-windows-msvc";

    /// A fresh dist directory holding one file per name in `files`.
    fn dist_dir(name: &str, files: &[&str]) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rse-sign-release-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for file in files {
            fs::write(dir.join(file), format!("bytes of {file}")).unwrap();
        }
        dir
    }

    /// Signs `files` with a throwaway test key and parses the result the way
    /// an installed app does.
    fn signed_manifest(name: &str, files: &[&str]) -> updater::Manifest {
        let dir = dist_dir(name, files);
        let key = SigningKey::from_bytes(&[7; 32]);
        let manifest = super::Manifest {
            version: "0.6.0".to_owned(),
            targets: sign_targets(&dir, "owner/repo", "v0.6.0", &key),
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let _ = fs::remove_dir_all(&dir);
        serde_json::from_str(&json).unwrap()
    }

    fn assert_listed_under_both_names(
        manifest: &updater::Manifest,
        current: &str,
        legacy: &str,
        filename: &str,
    ) {
        let new = manifest
            .find(current, TRIPLE)
            .unwrap_or_else(|| panic!("{filename} must be listed as {current}"));
        let old = manifest
            .find(legacy, TRIPLE)
            .unwrap_or_else(|| panic!("{filename} must be listed as {legacy}"));
        assert!(new.url.ends_with(&format!("/{filename}")), "{}", new.url);
        assert_eq!(old.url, new.url);
        assert_eq!(old.sha256, new.sha256);
        assert_eq!(old.signature, new.signature);
    }

    #[test]
    fn a_current_gui_file_is_listed_under_its_current_and_legacy_names() {
        let filename = "RepoSphereExplorerGui-x86_64-pc-windows-msvc.exe";
        let manifest = signed_manifest("current-gui", &[filename]);
        assert_eq!(manifest.targets.len(), 2);
        assert_listed_under_both_names(&manifest, "RepoSphereExplorerGui", "gui", filename);
    }

    #[test]
    fn a_v0_6_0_gui_file_is_listed_under_its_current_and_legacy_names() {
        let filename = "gui-x86_64-pc-windows-msvc.exe";
        let manifest = signed_manifest("legacy-gui", &[filename]);
        assert_eq!(manifest.targets.len(), 2);
        assert_listed_under_both_names(&manifest, "RepoSphereExplorerGui", "gui", filename);
    }

    #[test]
    fn terminal_app_files_are_listed_under_both_names_and_others_only_once() {
        let current = "RepoSphereExplorerTui-x86_64-pc-windows-msvc.exe";
        let manifest = signed_manifest(
            "tui-and-service",
            &[current, "service-x86_64-pc-windows-msvc.exe"],
        );
        assert_listed_under_both_names(&manifest, "RepoSphereExplorerTui", "tui", current);
        assert_eq!(
            manifest.targets.len(),
            3,
            "the service has no legacy name to list"
        );

        let legacy = "tui-x86_64-pc-windows-msvc.exe";
        let manifest = signed_manifest("legacy-tui", &[legacy]);
        assert_listed_under_both_names(&manifest, "RepoSphereExplorerTui", "tui", legacy);
    }

    #[test]
    fn the_legacy_entry_signature_verifies_against_the_file_its_url_names() {
        // The join a stranded install depends on: the `gui` entry's digest
        // and signature are over the very file it will download.
        let filename = "RepoSphereExplorerGui-x86_64-pc-windows-msvc.exe";
        let dir = dist_dir("verify", &[filename]);
        let key = SigningKey::from_bytes(&[9; 32]);
        let targets = sign_targets(&dir, "owner/repo", "v1", &key);
        let bytes = fs::read(dir.join(filename)).unwrap();
        let _ = fs::remove_dir_all(&dir);
        let legacy = targets.iter().find(|t| t.binary == "gui").unwrap();
        assert_eq!(legacy.sha256, updater::sha256_hex(&bytes));
        let digest = hex_decode(&legacy.sha256).unwrap();
        let signature: [u8; 64] = hex_decode(&legacy.signature).unwrap().try_into().unwrap();
        let signature = ed25519_dalek::Signature::from_bytes(&signature);
        assert!(
            key.verifying_key()
                .verify_strict(&digest, &signature)
                .is_ok()
        );
    }

    #[test]
    fn parse_filename_maps_a_v0_6_0_name_to_the_current_binary() {
        assert_eq!(
            parse_filename("gui-x86_64-pc-windows-msvc.exe"),
            Some(("RepoSphereExplorerGui".to_owned(), TRIPLE.to_owned()))
        );
        assert_eq!(
            parse_filename("tui-aarch64-apple-darwin"),
            Some((
                "RepoSphereExplorerTui".to_owned(),
                "aarch64-apple-darwin".to_owned()
            ))
        );
    }

    /// The macOS bundle's icon is published beside the binaries so the
    /// install script can place it under the same signature. Its name ends
    /// in `.icns`, and a triple with an extension left on it would send the
    /// script looking for a target nothing publishes.
    #[test]
    fn parse_filename_takes_the_icon_extension_off_the_triple() {
        assert_eq!(
            parse_filename("AppIcon-aarch64-apple-darwin.icns"),
            Some(("AppIcon".to_owned(), "aarch64-apple-darwin".to_owned()))
        );
    }

    #[test]
    fn the_bundle_icon_is_signed_like_everything_else() {
        let filename = "AppIcon-aarch64-apple-darwin.icns";
        let manifest = signed_manifest("bundle-icon", &[filename]);
        let icon = manifest
            .find("AppIcon", "aarch64-apple-darwin")
            .expect("the icon is listed for the macOS target");
        assert!(icon.url.ends_with(&format!("/{filename}")), "{}", icon.url);
        assert_eq!(
            manifest.targets.len(),
            1,
            "the icon has no legacy name to list"
        );
    }

    #[test]
    fn parse_filename_splits_a_known_binary_from_its_target_triple() {
        let (binary, target) = parse_filename("service-x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(binary, "service");
        assert_eq!(
            target, "x86_64-unknown-linux-gnu",
            "the triple's own hyphens must not be split on"
        );
    }

    #[test]
    fn parse_filename_strips_the_windows_executable_suffix_from_the_triple() {
        // The suffix has to go, or the manifest records a target no build
        // will ever ask for and the update silently never arrives.
        let (binary, target) =
            parse_filename("RepoSphereExplorerGui-x86_64-pc-windows-msvc.exe").unwrap();
        assert_eq!(binary, "RepoSphereExplorerGui");
        assert_eq!(target, "x86_64-pc-windows-msvc");
    }

    #[test]
    fn parse_filename_recognises_every_binary_the_release_publishes() {
        for binary in KNOWN_BINARIES {
            let filename = format!("{binary}-aarch64-apple-darwin");
            let (parsed, target) = parse_filename(&filename)
                .unwrap_or_else(|| panic!("{binary} must be recognised in {filename}"));
            assert_eq!(&parsed, binary);
            assert_eq!(target, "aarch64-apple-darwin");
        }
    }

    #[test]
    fn parse_filename_declines_a_name_that_is_not_a_published_binary() {
        assert!(parse_filename("some-other-tool-x86_64-pc-windows-msvc").is_none());
        assert!(parse_filename("latest.json").is_none());
        assert!(parse_filename("").is_none());
    }

    #[test]
    fn parse_filename_declines_a_binary_name_without_a_following_triple() {
        // Without the separating hyphen there is no target, and guessing
        // one would publish an asset against the wrong machine.
        assert!(parse_filename("service").is_none());
        assert!(parse_filename("service.exe").is_none());
        assert!(parse_filename("servicex86_64-pc-windows-msvc").is_none());
    }

    #[test]
    fn parse_filename_keeps_a_suffix_that_is_not_dot_exe_inside_the_triple() {
        // Recorded, not asserted as desirable: only `.exe` is stripped, so
        // a stray sidecar file in the dist directory is signed and
        // published under a target triple that does not exist. It is inert
        // - no build asks for that triple - but it bloats the manifest.
        let (binary, target) = parse_filename("service-x86_64-unknown-linux-gnu.sha256").unwrap();
        assert_eq!(binary, "service");
        assert_eq!(target, "x86_64-unknown-linux-gnu.sha256");
    }

    #[test]
    fn hex_decode_rejects_a_signing_key_that_is_not_hexadecimal_pairs() {
        // The signing key comes from an environment variable; a mistyped
        // one must fail loudly rather than decode to some other key.
        assert!(hex_decode(&"a".repeat(63)).is_none(), "odd length");
        assert!(hex_decode(&"z".repeat(64)).is_none(), "not hexadecimal");
        assert_eq!(hex_decode(&"ab".repeat(32)).unwrap().len(), 32);
    }

    #[test]
    fn hex_encode_round_trips_through_hex_decode() {
        let bytes: Vec<u8> = (0..=u8::MAX).collect();
        assert_eq!(hex_decode(&hex_encode(&bytes)).unwrap(), bytes);
    }
}
