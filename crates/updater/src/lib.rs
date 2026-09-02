//! In-app updater: fetches, signature-verifies, and atomically applies
//! release binaries.
//!
//! Automatic relaunch after an update, and a rollback path if the new
//! binary fails to start, are deferred: this covers §4.2's non-negotiable
//! part (nothing is applied unless it verifies against the embedded public
//! key) and stages the replacement atomically, but does not yet supervise
//! the *next* launch to confirm it succeeded.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io;
use std::io::Read as _;
use std::path::Path;

/// The public key this build trusts. The matching private key is held only
/// as a GitHub Actions secret and never appears in this repository;
/// `cargo run -p updater --bin keygen` generates a new pair when rotation
/// is needed.
pub const PUBLIC_KEY: [u8; 32] = [
    0x32, 0x68, 0xae, 0x9c, 0x1a, 0xdd, 0xd9, 0x23, 0x7c, 0x4a, 0xeb, 0x29, 0x18, 0xf2, 0xb5, 0xd5,
    0x33, 0xfc, 0x23, 0x5b, 0x87, 0x12, 0x33, 0x76, 0x47, 0xcf, 0x79, 0x36, 0x5b, 0x4f, 0xea, 0xe9,
];

/// The stable, versioned URL this build checks for updates.
pub const MANIFEST_URL: &str =
    "https://stewartscottrogers.github.io/RepoSphereExplorer/latest.json";

/// This build's target triple, matching one of `release.yml`'s matrix
/// entries. `"unknown"` on a target the release workflow doesn't publish.
#[must_use]
pub const fn current_target() -> &'static str {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else {
        "unknown"
    }
}

/// Checks for and applies an update to `binary_name`, using this build's
/// own target, version, and running executable path.
///
/// Every crate in this workspace shares one `[workspace.package] version`,
/// so `updater`'s own compiled-in version is also the calling binary's.
///
/// # Errors
/// See [`check_and_update`].
pub fn self_update(binary_name: &str) -> Result<Outcome, UpdateError> {
    let exe_path = std::env::current_exe()?;
    check_and_update(
        binary_name,
        current_target(),
        env!("CARGO_PKG_VERSION"),
        MANIFEST_URL,
        &exe_path,
    )
}

/// A published release: its version and the signed assets available for
/// each `(binary, target)` pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// The release version, e.g. `"0.3.0"`.
    pub version: String,
    /// Every binary published for this release, across all targets.
    pub targets: Vec<TargetAsset>,
}

/// One binary published for one target triple.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetAsset {
    /// Binary name, e.g. `"tui"`.
    pub binary: String,
    /// Rust target triple, e.g. `"x86_64-pc-windows-msvc"`.
    pub target: String,
    /// Download URL for the binary.
    pub url: String,
    /// Lowercase hex-encoded SHA-256 of the binary's bytes.
    pub sha256: String,
    /// Lowercase hex-encoded Ed25519 signature over the raw SHA-256 digest.
    pub signature: String,
}

impl Manifest {
    /// Finds the asset for `binary` on `target`, if this release publishes
    /// one.
    #[must_use]
    pub fn find(&self, binary: &str, target: &str) -> Option<&TargetAsset> {
        self.targets
            .iter()
            .find(|asset| asset.binary == binary && asset.target == target)
    }
}

/// What [`check_and_update`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The running binary is already at the manifest's version.
    UpToDate {
        /// The current (and latest) version.
        version: String,
    },
    /// The running binary was replaced with a newer, verified one.
    Updated {
        /// The version that was running before the update.
        from: String,
        /// The version now installed.
        to: String,
    },
}

/// Everything that can go wrong while checking for or applying an update.
#[derive(Debug)]
pub enum UpdateError {
    /// The manifest could not be fetched or parsed.
    Manifest(String),
    /// No published asset matches this binary and target.
    NoMatchingAsset {
        /// The binary name that was searched for.
        binary: String,
        /// The target triple that was searched for.
        target: String,
    },
    /// The downloaded bytes did not match the manifest's declared hash.
    HashMismatch,
    /// The signature over the downloaded bytes' hash did not verify.
    SignatureInvalid,
    /// Reading, writing, or renaming a file failed.
    Io(io::Error),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateError::Manifest(message) => {
                write!(f, "could not read the update manifest: {message}")
            }
            UpdateError::NoMatchingAsset { binary, target } => {
                write!(f, "no published build of `{binary}` for target `{target}`")
            }
            UpdateError::HashMismatch => {
                write!(f, "downloaded bytes did not match the published hash")
            }
            UpdateError::SignatureInvalid => write!(f, "signature verification failed"),
            UpdateError::Io(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for UpdateError {}

impl From<io::Error> for UpdateError {
    fn from(err: io::Error) -> Self {
        UpdateError::Io(err)
    }
}

/// Hex-encodes the SHA-256 digest of `data`.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    hex_encode(&digest)
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

/// Verifies that `signature_hex` is a valid Ed25519 signature over
/// `digest`, made by the embedded [`PUBLIC_KEY`].
#[must_use]
pub fn verify_digest(digest: &[u8], signature_hex: &str) -> bool {
    let Ok(verifying_key) = VerifyingKey::from_bytes(&PUBLIC_KEY) else {
        return false;
    };
    let Some(signature_bytes) = hex_decode(signature_hex) else {
        return false;
    };
    let Ok(signature) = Signature::try_from(signature_bytes.as_slice()) else {
        return false;
    };
    verifying_key.verify(digest, &signature).is_ok()
}

/// Fetches and parses the manifest at `url`.
///
/// # Errors
/// Returns an error if the manifest cannot be fetched or is not valid JSON.
pub fn fetch_manifest(url: &str) -> Result<Manifest, UpdateError> {
    let mut response = ureq::get(url)
        .call()
        .map_err(|err| UpdateError::Manifest(err.to_string()))?;
    response
        .body_mut()
        .read_json::<Manifest>()
        .map_err(|err| UpdateError::Manifest(err.to_string()))
}

/// Downloads `asset`'s binary, then checks its hash and signature before
/// returning its bytes.
///
/// # Errors
/// Returns an error if the download fails, the hash does not match, or the
/// signature does not verify.
pub fn download_and_verify(asset: &TargetAsset) -> Result<Vec<u8>, UpdateError> {
    let mut response = ureq::get(&asset.url)
        .call()
        .map_err(|err| UpdateError::Manifest(err.to_string()))?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .read_to_end(&mut bytes)
        .map_err(UpdateError::Io)?;

    let digest = Sha256::digest(&bytes);
    if hex_encode(&digest) != asset.sha256.to_lowercase() {
        return Err(UpdateError::HashMismatch);
    }
    if !verify_digest(&digest, &asset.signature) {
        return Err(UpdateError::SignatureInvalid);
    }
    Ok(bytes)
}

/// Replaces the file at `target_path` with `bytes`, atomically: writes to a
/// temporary file in the same directory (so the rename is same-filesystem),
/// marks it executable on Unix, then renames it over `target_path`.
///
/// # Errors
/// Returns an error if writing the temporary file, setting its permissions,
/// or renaming it fails.
pub fn apply_atomic(bytes: &[u8], target_path: &Path) -> io::Result<()> {
    let dir = target_path.parent().unwrap_or_else(|| Path::new("."));
    let temp_path = dir.join(format!(
        ".{}.update",
        target_path.file_name().map_or_else(
            || "binary".into(),
            |name| name.to_string_lossy().into_owned()
        )
    ));
    fs::write(&temp_path, bytes)?;
    set_executable(&temp_path)?;
    fs::rename(&temp_path, target_path)?;
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() | 0o111);
    fs::set_permissions(path, perms)
}

// Kept as io::Result<()> (rather than dropping the return type) so it has
// the same signature as the Unix version above, both callable via `?`.
#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn set_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Checks `manifest_url` for a release newer than `current_version` for
/// `(binary, target)`, and if one exists, downloads, verifies, and applies
/// it to `exe_path`.
///
/// # Errors
/// Returns an error if the manifest cannot be fetched, no asset matches, or
/// the downloaded bytes fail verification or cannot be applied.
pub fn check_and_update(
    binary: &str,
    target: &str,
    current_version: &str,
    manifest_url: &str,
    exe_path: &Path,
) -> Result<Outcome, UpdateError> {
    let manifest = fetch_manifest(manifest_url)?;
    if manifest.version == current_version {
        return Ok(Outcome::UpToDate {
            version: manifest.version,
        });
    }
    let asset = manifest
        .find(binary, target)
        .ok_or_else(|| UpdateError::NoMatchingAsset {
            binary: binary.to_owned(),
            target: target.to_owned(),
        })?;
    let bytes = download_and_verify(asset)?;
    apply_atomic(&bytes, exe_path)?;
    Ok(Outcome::Updated {
        from: current_version.to_owned(),
        to: manifest.version,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        Manifest, PUBLIC_KEY, apply_atomic, hex_decode, hex_encode, sha256_hex, verify_digest,
    };
    use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
    use sha2::{Digest, Sha256};

    #[test]
    fn embedded_public_key_is_a_valid_ed25519_point() {
        assert!(VerifyingKey::from_bytes(&PUBLIC_KEY).is_ok());
    }

    #[test]
    fn hex_round_trips() {
        let bytes = [0x00, 0x0f, 0xab, 0xff];
        let encoded = hex_encode(&bytes);
        assert_eq!(encoded, "000fabff");
        assert_eq!(hex_decode(&encoded).unwrap(), bytes);
    }

    #[test]
    fn sha256_hex_is_64_lowercase_hex_characters() {
        let digest = sha256_hex(b"some bytes");
        assert_eq!(digest.len(), 64);
        assert!(
            digest
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        // Deterministic: hashing the same bytes again gives the same digest.
        assert_eq!(digest, sha256_hex(b"some bytes"));
    }

    #[test]
    fn rejects_a_signature_from_the_wrong_key() {
        let signing_key = SigningKey::generate(&mut rand::rng());
        let digest = Sha256::digest(b"some binary bytes");
        let signature = signing_key.sign(&digest);
        let wrong_signature_hex = hex_encode(&signature.to_bytes());

        // verify_digest checks against the embedded PUBLIC_KEY, which this
        // signature was not made with, so it must be rejected.
        assert!(!verify_digest(&digest, &wrong_signature_hex));
    }

    #[test]
    fn manifest_finds_the_matching_asset() {
        let manifest: Manifest = serde_json::from_str(
            r#"{
                "version": "0.3.0",
                "targets": [
                    {"binary": "tui", "target": "x86_64-pc-windows-msvc", "url": "https://example/tui.exe", "sha256": "aa", "signature": "bb"},
                    {"binary": "gui", "target": "x86_64-pc-windows-msvc", "url": "https://example/gui.exe", "sha256": "cc", "signature": "dd"}
                ]
            }"#,
        )
        .unwrap();

        let found = manifest.find("gui", "x86_64-pc-windows-msvc").unwrap();
        assert_eq!(found.url, "https://example/gui.exe");
        assert!(manifest.find("gui", "aarch64-apple-darwin").is_none());
    }

    #[test]
    fn apply_atomic_replaces_the_target_file_in_place() {
        let dir = std::env::temp_dir().join(format!("rse-updater-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("binary");
        std::fs::write(&target, b"old").unwrap();

        apply_atomic(b"new", &target).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
