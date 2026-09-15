//! Builds and signs the update manifest (`latest.json`) for a release.
//!
//! Usage: `sign_release <version> <owner/repo> <tag> <dist-dir> <output-path>`
//! Reads the hex-encoded signing key from the `UPDATER_SIGNING_KEY`
//! environment variable. Scans `<dist-dir>` for files named
//! `<binary>-<target-triple>[.exe]`, matching `release.yml`'s packaging
//! convention, and signs each one.

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
    "tui",
    "gui",
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

    let targets = collect_targets(Path::new(dist_dir), repo, tag, &signing_key);

    let manifest = Manifest {
        version: version.clone(),
        targets,
    };
    let json = serde_json::to_string_pretty(&manifest).expect("serialize manifest");
    fs::write(Path::new(output_path), json).expect("write manifest");
    println!("wrote {output_path}");
}

/// Scans `dist_dir` and signs every recognised file, aliasing the graphical
/// and terminal applications under both their current and their v0.6.0
/// manifest names ([`alias_names`]) so an installed old build can still find
/// an update to ask for by its old name.
fn collect_targets(
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
        let url = format!("https://github.com/{repo}/releases/download/{tag}/{filename}");
        let sha256 = hex_encode(&digest);
        let signature = hex_encode(&signature.to_bytes());

        for name in alias_names(&binary) {
            targets.push(TargetAsset {
                binary: name,
                target: target.clone(),
                url: url.clone(),
                sha256: sha256.clone(),
                signature: signature.clone(),
            });
        }
    }
    targets
}

/// The manifest names a signed file should be published under. The
/// graphical and terminal applications are published under both their
/// current name and the `gui` / `tui` names a v0.6.0 install still asks
/// `--self-update` for, so re-signing either one's dist file publishes an
/// update the old install can find (issue #549). Every other binary is
/// published under its own name only.
fn alias_names(binary: &str) -> Vec<String> {
    match binary {
        "RepoSphereExplorerGui" | "gui" => {
            vec!["RepoSphereExplorerGui".to_owned(), "gui".to_owned()]
        }
        "RepoSphereExplorerTui" | "tui" => {
            vec!["RepoSphereExplorerTui".to_owned(), "tui".to_owned()]
        }
        other => vec![other.to_owned()],
    }
}

fn parse_filename(filename: &str) -> Option<(String, String)> {
    for binary in KNOWN_BINARIES {
        if let Some(rest) = filename
            .strip_prefix(binary)
            .and_then(|r| r.strip_prefix('-'))
        {
            let target = rest.strip_suffix(".exe").unwrap_or(rest);
            return Some(((*binary).to_owned(), target.to_owned()));
        }
    }
    None
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
    use super::{
        KNOWN_BINARIES, alias_names, collect_targets, hex_decode, hex_encode, parse_filename,
    };
    use ed25519_dalek::SigningKey;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn test_signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    /// A scratch directory under the OS temp dir, unique per call so tests
    /// running in parallel do not see each other's fixture files.
    fn temp_dist_dir(files: &[(&str, &[u8])]) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sign_release_test_{}_{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dist dir");
        for (name, bytes) in files {
            fs::write(dir.join(name), bytes).expect("write fixture file");
        }
        dir
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

    #[test]
    fn alias_names_publishes_gui_and_tui_under_both_their_names() {
        // A v0.6.0 install still asks `--self-update` for "gui" / "tui"
        // (issue #549): every other binary keeps its one published name.
        assert_eq!(
            alias_names("RepoSphereExplorerGui"),
            vec!["RepoSphereExplorerGui", "gui"]
        );
        assert_eq!(
            alias_names("RepoSphereExplorerTui"),
            vec!["RepoSphereExplorerTui", "tui"]
        );
        assert_eq!(alias_names("gui"), vec!["RepoSphereExplorerGui", "gui"]);
        assert_eq!(alias_names("tui"), vec!["RepoSphereExplorerTui", "tui"]);
        assert_eq!(alias_names("service"), vec!["service"]);
    }

    #[test]
    fn collect_targets_signs_a_current_style_gui_file_under_both_names() {
        let dist_dir = temp_dist_dir(&[(
            "RepoSphereExplorerGui-x86_64-pc-windows-msvc.exe",
            b"gui bytes",
        )]);

        let targets = collect_targets(
            &dist_dir,
            "StewartScottRogers/RepoSphereExplorer",
            "v0.7.0",
            &test_signing_key(),
        );
        fs::remove_dir_all(&dist_dir).expect("clean up temp dist dir");

        let new_name = targets
            .iter()
            .find(|asset| asset.binary == "RepoSphereExplorerGui")
            .expect("published under the current name");
        let old_name = targets
            .iter()
            .find(|asset| asset.binary == "gui")
            .expect("published under the v0.6.0 name");

        assert_eq!(targets.len(), 2, "only these two entries for one file");
        assert_eq!(new_name.target, "x86_64-pc-windows-msvc");
        assert_eq!(old_name.target, new_name.target);
        assert_eq!(old_name.url, new_name.url);
        assert_eq!(old_name.sha256, new_name.sha256);
        assert_eq!(old_name.signature, new_name.signature);
    }

    #[test]
    fn collect_targets_signs_a_v0_6_0_style_gui_file_under_both_names() {
        let dist_dir = temp_dist_dir(&[("gui-x86_64-pc-windows-msvc.exe", b"gui bytes")]);

        let targets = collect_targets(
            &dist_dir,
            "StewartScottRogers/RepoSphereExplorer",
            "v0.6.0",
            &test_signing_key(),
        );
        fs::remove_dir_all(&dist_dir).expect("clean up temp dist dir");

        let new_name = targets
            .iter()
            .find(|asset| asset.binary == "RepoSphereExplorerGui")
            .expect("re-signing publishes the current name too");
        let old_name = targets
            .iter()
            .find(|asset| asset.binary == "gui")
            .expect("the v0.6.0 file's own name is kept");

        assert_eq!(targets.len(), 2, "only these two entries for one file");
        assert_eq!(old_name.url, new_name.url);
        assert_eq!(old_name.sha256, new_name.sha256);
        assert_eq!(old_name.signature, new_name.signature);
    }

    #[test]
    fn manifest_find_locates_the_old_gui_name_in_a_manifest_built_this_way() {
        let dist_dir = temp_dist_dir(&[("gui-x86_64-pc-windows-msvc.exe", b"gui bytes")]);

        let targets = collect_targets(
            &dist_dir,
            "StewartScottRogers/RepoSphereExplorer",
            "v0.6.0",
            &test_signing_key(),
        );
        fs::remove_dir_all(&dist_dir).expect("clean up temp dist dir");

        let manifest = super::Manifest {
            version: "0.6.0".to_owned(),
            targets,
        };
        let json = serde_json::to_string(&manifest).expect("serialize manifest");
        let manifest: updater::Manifest =
            serde_json::from_str(&json).expect("deserialize as the updater's own manifest type");

        let asset = manifest
            .find("gui", "x86_64-pc-windows-msvc")
            .expect("a v0.6.0 install's self_update(\"gui\") must still find an asset");
        assert!(asset.url.ends_with("gui-x86_64-pc-windows-msvc.exe"));
    }
}
