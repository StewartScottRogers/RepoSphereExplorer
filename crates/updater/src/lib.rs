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

    /// Finds the asset published under `file_name`: the last segment of its
    /// download address, which is the name a release asset carries.
    ///
    /// For the installer's `verify`, which is handed files that have
    /// already been downloaded and has only their names to go on.
    #[must_use]
    pub fn find_file(&self, file_name: &str) -> Option<&TargetAsset> {
        self.targets
            .iter()
            .find(|asset| asset.url.rsplit('/').next() == Some(file_name))
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
    let bytes = download(&asset.url)?;
    verify_bytes(asset, &bytes)?;
    Ok(bytes)
}

/// Downloads `url`, without checking anything about what comes back.
///
/// Its caller verifies: an update through [`download_and_verify`], a fresh
/// install through the setup program, which reports a file the manifest
/// disowns in its own words.
///
/// # Errors
/// Returns an error if the request fails or the body cannot be read.
pub fn download(url: &str) -> Result<Vec<u8>, UpdateError> {
    let mut response = ureq::get(url)
        .call()
        .map_err(|err| UpdateError::Manifest(err.to_string()))?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .read_to_end(&mut bytes)
        .map_err(UpdateError::Io)?;
    Ok(bytes)
}

/// Checks `bytes` against `asset`: their SHA-256 digest must be the one the
/// manifest publishes, and the manifest's signature over that digest must
/// verify against the embedded [`PUBLIC_KEY`].
///
/// The one check both an update and a fresh install are held to.
///
/// # Errors
/// Returns [`UpdateError::HashMismatch`] when the digest differs, and
/// [`UpdateError::SignatureInvalid`] when it matches but the signature does
/// not verify.
pub fn verify_bytes(asset: &TargetAsset, bytes: &[u8]) -> Result<(), UpdateError> {
    let digest = Sha256::digest(bytes);
    if hex_encode(&digest) != asset.sha256.to_lowercase() {
        return Err(UpdateError::HashMismatch);
    }
    if !verify_digest(&digest, &asset.signature) {
        return Err(UpdateError::SignatureInvalid);
    }
    Ok(())
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
    let result = rename_with_retry(&temp_path, target_path);
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

/// Retries `fs::rename` a few times with a short backoff before giving up.
///
/// A freshly-written executable is a common target for antivirus real-time
/// scanning, which briefly holds an exclusive lock on Windows; renaming
/// over it right after `fs::write` can hit a transient "Access is denied"
/// that clears within milliseconds. Observed in practice self-updating a
/// real install, not a hypothetical.
fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    const ATTEMPTS: u32 = 5;
    const DELAY: std::time::Duration = std::time::Duration::from_millis(100);
    let mut last_err = None;
    for attempt in 0..ATTEMPTS {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(err) => {
                last_err = Some(err);
                if attempt + 1 < ATTEMPTS {
                    std::thread::sleep(DELAY);
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| io::Error::other("rename failed with no recorded error")))
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

/// A release version split into its numbers and whatever follows them.
///
/// `None` when the numbers cannot be read at all, which is the answer that
/// makes [`is_newer`] refuse rather than guess.
fn split_version(text: &str) -> Option<(Vec<u64>, Option<String>)> {
    let text = text.trim();
    let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
    let (core, pre) = match text.find(['-', '+']) {
        Some(at) => (&text[..at], Some(text[at + 1..].to_owned())),
        None => (text, None),
    };
    if core.is_empty() {
        return None;
    }
    let mut numbers = Vec::new();
    for part in core.split('.') {
        numbers.push(part.parse::<u64>().ok()?);
    }
    Some((numbers, pre))
}

/// Whether `candidate` names a release later than `current`.
///
/// The dot-separated numbers are compared left to right and a missing
/// component counts as zero, so `1.10.0` is later than `1.9.0` and `0.6` is
/// the same release as `0.6.0`. A leading `v` is ignored on either side. A
/// version carrying a pre-release suffix is earlier than the same numbers
/// without one, the way Cargo reads it.
///
/// When either side cannot be read as numbers, the answer is `false`.
/// Refusing to act on a version nobody can order is the safe direction for
/// something that replaces the running executable.
///
/// There was no comparison here at all before: the gate was
/// `manifest.version == current_version`, so a manifest naming an *older*
/// release was not equal and was therefore fetched, verified and installed
/// over a newer build - and any cosmetic difference, `v0.6.0` against
/// `0.6.0` or a trailing newline, never compared equal and so re-installed
/// the same release on every launch, forever.
#[must_use]
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (Some((theirs, their_pre)), Some((ours, our_pre))) =
        (split_version(candidate), split_version(current))
    else {
        return false;
    };
    for index in 0..theirs.len().max(ours.len()) {
        let theirs = theirs.get(index).copied().unwrap_or(0);
        let ours = ours.get(index).copied().unwrap_or(0);
        if theirs != ours {
            return theirs > ours;
        }
    }
    match (their_pre, our_pre) {
        (None, Some(_)) => true,
        (Some(_) | None, None) => false,
        (Some(theirs), Some(ours)) => theirs > ours,
    }
}

/// What [`check_and_update`] should do about a manifest it has fetched.
#[derive(Debug)]
pub enum Decision<'a> {
    /// Nothing to install: this build is the same release or a later one.
    UpToDate,
    /// Install this asset.
    Install(&'a TargetAsset),
}

/// Decides what to do about `manifest`, without touching the network or
/// the filesystem.
///
/// Separated out so the decision can be tested at all. Everything here used
/// to sit inside [`check_and_update`], on the far side of an HTTP call, so
/// the version gate and every asset-selection path were unreachable from a
/// test.
///
/// # Errors
/// Returns [`UpdateError::NoMatchingAsset`] when the release is newer but
/// carries nothing for this binary and target.
pub fn decide<'a>(
    manifest: &'a Manifest,
    binary: &str,
    target: &str,
    current_version: &str,
) -> Result<Decision<'a>, UpdateError> {
    if !is_newer(&manifest.version, current_version) {
        return Ok(Decision::UpToDate);
    }
    manifest
        .find(binary, target)
        .map(Decision::Install)
        .ok_or_else(|| UpdateError::NoMatchingAsset {
            binary: binary.to_owned(),
            target: target.to_owned(),
        })
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
    let asset = match decide(&manifest, binary, target, current_version)? {
        Decision::UpToDate => {
            // The version this build is running, not the one the manifest
            // named: with an ordering rather than an equality those differ
            // whenever the manifest has gone backwards, and what "up to
            // date" means is the release you are on.
            return Ok(Outcome::UpToDate {
                version: current_version.to_owned(),
            });
        }
        Decision::Install(asset) => asset,
    };
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
        Decision, Manifest, Outcome, PUBLIC_KEY, TargetAsset, UpdateError, apply_atomic,
        check_and_update, current_target, decide, fetch_manifest, hex_decode, hex_encode, is_newer,
        sha256_hex, verify_bytes, verify_digest,
    };
    use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
    use sha2::{Digest, Sha256};
    use std::io;
    use std::path::PathBuf;

    /// A URL whose scheme `ureq` rejects while parsing, so calling it opens
    /// no socket and resolves no name. Lets the offline error paths be
    /// exercised without touching the network.
    const UNREACHABLE_URL: &str = "ftp://example.invalid/latest.json";

    /// A scratch directory of this test's own, so tests running on separate
    /// threads of one process cannot collide.
    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rse-updater-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn asset(binary: &str, target: &str) -> TargetAsset {
        TargetAsset {
            binary: binary.to_owned(),
            target: target.to_owned(),
            url: format!("https://example.invalid/{binary}-{target}"),
            sha256: "aa".to_owned(),
            signature: "bb".to_owned(),
        }
    }

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

    // ---- hex encoding and decoding ----

    #[test]
    fn hex_round_trips_every_byte_value() {
        let all_bytes: Vec<u8> = (0..=u8::MAX).collect();
        let encoded = hex_encode(&all_bytes);
        assert_eq!(encoded.len(), 512);
        assert!(encoded.starts_with("000102"), "encoded as {encoded}");
        assert!(encoded.ends_with("fdfeff"), "encoded as {encoded}");
        assert_eq!(hex_decode(&encoded).unwrap(), all_bytes);
    }

    #[test]
    fn hex_decode_rejects_an_odd_number_of_characters() {
        // A truncated signature must not silently decode to a shorter one.
        assert!(hex_decode("abc").is_none());
        assert!(hex_decode("a").is_none());
    }

    #[test]
    fn hex_decode_rejects_non_hexadecimal_characters() {
        assert!(hex_decode("zz").is_none());
        assert!(hex_decode("00zz").is_none());
        assert!(hex_decode(" f").is_none());
        assert!(hex_decode("-f").is_none());
    }

    #[test]
    fn hex_decode_declines_multibyte_characters_without_panicking() {
        // Two euro signs are six bytes, an even length, but slicing them in
        // pairs lands mid-character. Indexing rather than `str::get` would
        // panic here instead of declining.
        assert!(hex_decode("\u{20ac}\u{20ac}").is_none());
        assert!(hex_decode("aa\u{20ac}\u{20ac}").is_none());
    }

    #[test]
    fn hex_decode_accepts_uppercase_hexadecimal() {
        // `download_and_verify` lowercases the published hash before
        // comparing, so a manifest may legitimately carry uppercase hex.
        assert_eq!(hex_decode("AABBCC").unwrap(), [0xaa, 0xbb, 0xcc]);
    }

    #[test]
    fn hex_decode_wrongly_accepts_a_plus_sign_as_a_hexadecimal_digit() {
        // `u8::from_str_radix` accepts a leading `+`, so `hex_decode` is not
        // strictly hexadecimal: "+f" decodes as 0x0f. Recorded rather than
        // fixed - it cannot forge a signature, because the decoded bytes
        // still have to verify, but the parser is laxer than its name says.
        assert_eq!(hex_decode("+f").unwrap(), [0x0f]);
        assert_eq!(hex_decode("+1+2").unwrap(), [0x01, 0x02]);
    }

    // ---- signature verification ----

    #[test]
    fn verify_digest_rejects_an_empty_signature() {
        let digest = Sha256::digest(b"some binary bytes");
        assert!(!verify_digest(&digest, ""));
    }

    #[test]
    fn verify_digest_rejects_an_all_zero_signature() {
        // A zeroed field in a hand-written manifest must not pass as
        // "signed".
        let digest = Sha256::digest(b"some binary bytes");
        assert!(!verify_digest(&digest, &"0".repeat(128)));
    }

    #[test]
    fn verify_digest_rejects_a_signature_that_is_not_sixty_four_bytes() {
        let signing_key = SigningKey::generate(&mut rand::rng());
        let digest = Sha256::digest(b"some binary bytes");
        let full = hex_encode(&signing_key.sign(&digest).to_bytes());

        assert!(!verify_digest(&digest, &full[..126]), "too short");
        assert!(!verify_digest(&digest, &format!("{full}0000")), "too long");
    }

    #[test]
    fn verify_digest_rejects_signature_text_that_is_not_hexadecimal() {
        let digest = Sha256::digest(b"some binary bytes");
        assert!(!verify_digest(&digest, "not hex at all"), "prose");
        assert!(!verify_digest(&digest, &"z".repeat(128)), "non-hex digits");
        assert!(!verify_digest(&digest, &"a".repeat(127)), "odd length");
    }

    #[test]
    fn verify_digest_rejects_a_wrong_key_signature_whatever_the_digest() {
        // The existing wrong-key test uses one digest; a verifier that
        // happened to ignore the message would still be caught here.
        let signing_key = SigningKey::generate(&mut rand::rng());
        for payload in [b"".as_slice(), b"a".as_slice(), b"a longer body".as_slice()] {
            let digest = Sha256::digest(payload);
            let signature = hex_encode(&signing_key.sign(&digest).to_bytes());
            assert!(
                !verify_digest(&digest, &signature),
                "a signature by a key other than PUBLIC_KEY must never verify"
            );
        }
    }

    // ---- manifest parsing ----
    //
    // Note on the version gate, recorded here because no test can assert an
    // absence: `check_and_update` decides with `manifest.version ==
    // current_version` and nothing else. There is no ordering comparison
    // anywhere in the crate, so a manifest that names an older version, or
    // the same version written differently ("v0.6.0" against "0.6.0"), is
    // treated as an update and applied on every launch. Making that
    // decidable in a test needs a seam; see the report accompanying these
    // tests.

    #[test]
    fn manifest_parsing_rejects_a_missing_version() {
        let result = serde_json::from_str::<Manifest>(r#"{"targets": []}"#);
        assert!(result.is_err(), "a manifest without a version is unusable");
    }

    #[test]
    fn manifest_parsing_rejects_a_missing_targets_list() {
        let result = serde_json::from_str::<Manifest>(r#"{"version": "0.6.0"}"#);
        assert!(result.is_err(), "a manifest without assets is unusable");
    }

    #[test]
    fn manifest_parsing_rejects_a_version_that_is_not_a_string() {
        let result = serde_json::from_str::<Manifest>(r#"{"version": 6, "targets": []}"#);
        assert!(result.is_err(), "a numeric version must not be coerced");
    }

    #[test]
    fn manifest_parsing_rejects_an_asset_missing_its_signature() {
        // An unsigned asset must fail at parse time rather than reaching
        // `download_and_verify` with an empty signature.
        let result = serde_json::from_str::<Manifest>(
            r#"{"version": "0.6.0", "targets": [
                {"binary": "gui", "target": "x86_64-pc-windows-msvc",
                 "url": "https://example.invalid/gui.exe", "sha256": "aa"}
            ]}"#,
        );
        assert!(result.is_err(), "an asset without a signature is unusable");
    }

    #[test]
    fn manifest_parsing_rejects_an_asset_missing_its_hash() {
        let result = serde_json::from_str::<Manifest>(
            r#"{"version": "0.6.0", "targets": [
                {"binary": "gui", "target": "x86_64-pc-windows-msvc",
                 "url": "https://example.invalid/gui.exe", "signature": "bb"}
            ]}"#,
        );
        assert!(result.is_err(), "an asset without a hash is unusable");
    }

    #[test]
    fn manifest_parsing_accepts_fields_it_does_not_know_about() {
        // A later release may add fields; an older build must still update.
        let manifest: Manifest = serde_json::from_str(
            r#"{
                "version": "0.7.0",
                "notes": "https://example.invalid/changelog",
                "targets": [
                    {"binary": "gui", "target": "x86_64-pc-windows-msvc",
                     "url": "https://example.invalid/gui.exe", "sha256": "aa",
                     "signature": "bb", "size": 1234}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(manifest.version, "0.7.0");
        assert!(manifest.find("gui", "x86_64-pc-windows-msvc").is_some());
    }

    #[test]
    fn manifest_round_trips_through_json() {
        let manifest = Manifest {
            version: "0.6.0".to_owned(),
            targets: vec![asset("gui", "x86_64-pc-windows-msvc")],
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let parsed: Manifest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.version, manifest.version);
        assert_eq!(parsed.targets.len(), 1);
        let round_tripped = parsed.find("gui", "x86_64-pc-windows-msvc").unwrap();
        assert_eq!(round_tripped.url, manifest.targets[0].url);
        assert_eq!(round_tripped.sha256, manifest.targets[0].sha256);
        assert_eq!(round_tripped.signature, manifest.targets[0].signature);
    }

    // ---- asset selection ----

    #[test]
    fn a_manifest_with_no_assets_matches_nothing() {
        let manifest = Manifest {
            version: "0.6.0".to_owned(),
            targets: Vec::new(),
        };
        assert!(manifest.find("gui", current_target()).is_none());
    }

    #[test]
    fn manifest_find_declines_when_only_another_architecture_is_published() {
        // The failure this guards against is picking an asset "close
        // enough" and installing a binary for the wrong machine.
        let manifest = Manifest {
            version: "0.6.0".to_owned(),
            targets: vec![
                asset("gui", "aarch64-apple-darwin"),
                asset("gui", "x86_64-unknown-linux-gnu"),
            ],
        };
        assert!(manifest.find("gui", "x86_64-pc-windows-msvc").is_none());
    }

    #[test]
    fn manifest_find_declines_when_the_target_matches_but_the_binary_does_not() {
        let manifest = Manifest {
            version: "0.6.0".to_owned(),
            targets: vec![asset("gui", "x86_64-pc-windows-msvc")],
        };
        assert!(manifest.find("service", "x86_64-pc-windows-msvc").is_none());
    }

    #[test]
    fn manifest_find_picks_the_one_asset_matching_both_binary_and_target() {
        let manifest = Manifest {
            version: "0.6.0".to_owned(),
            targets: vec![
                asset("gui", "aarch64-apple-darwin"),
                asset("service", "x86_64-pc-windows-msvc"),
                asset("gui", "x86_64-pc-windows-msvc"),
                asset("gui", "x86_64-unknown-linux-gnu"),
            ],
        };
        let found = manifest.find("gui", "x86_64-pc-windows-msvc").unwrap();
        assert_eq!(found.binary, "gui");
        assert_eq!(found.target, "x86_64-pc-windows-msvc");
    }

    #[test]
    fn manifest_find_matches_the_target_triple_exactly() {
        // Triples are compared as written, so a differently-cased entry is
        // declined rather than accepted as equivalent.
        let manifest = Manifest {
            version: "0.6.0".to_owned(),
            targets: vec![asset("gui", "X86_64-PC-Windows-MSVC")],
        };
        assert!(manifest.find("gui", "x86_64-pc-windows-msvc").is_none());
    }

    #[test]
    fn find_file_matches_the_last_segment_of_the_download_address() {
        let manifest = Manifest {
            version: "0.7.0".to_owned(),
            targets: vec![
                asset("service", "x86_64-unknown-linux-gnu"),
                asset("service", "x86_64-pc-windows-msvc"),
            ],
        };
        let found = manifest
            .find_file("service-x86_64-pc-windows-msvc")
            .unwrap();
        assert_eq!(found.target, "x86_64-pc-windows-msvc");
        assert!(
            manifest.find_file("service").is_none(),
            "a prefix is not a name"
        );
        assert!(manifest.find_file("example.invalid").is_none());
    }

    // ---- verifying bytes, the check an install shares with an update ----

    #[test]
    fn verify_bytes_refuses_bytes_whose_digest_is_not_the_published_one() {
        let mut published = asset("service", current_target());
        published.sha256 = sha256_hex(b"the published build");

        let err = verify_bytes(&published, b"the published build, tampered").unwrap_err();

        assert!(matches!(err, UpdateError::HashMismatch), "was {err:?}");
    }

    #[test]
    fn verify_bytes_refuses_a_matching_digest_signed_by_another_key() {
        // Rewriting the manifest's digest to match tampered bytes must not
        // be enough: the signature is what nobody but the release can make.
        let signing_key = SigningKey::generate(&mut rand::rng());
        let bytes = b"a build somebody else made";
        let digest = Sha256::digest(bytes);
        let mut forged = asset("service", current_target());
        forged.sha256 = hex_encode(&digest);
        forged.signature = hex_encode(&signing_key.sign(&digest).to_bytes());

        let err = verify_bytes(&forged, bytes).unwrap_err();

        assert!(matches!(err, UpdateError::SignatureInvalid), "was {err:?}");
    }

    // ---- this build's target ----

    #[test]
    fn current_target_agrees_with_the_host_operating_system_and_architecture() {
        let expected = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "x86_64") => "x86_64-pc-windows-msvc",
            ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
            ("macos", "aarch64") => "aarch64-apple-darwin",
            _ => "unknown",
        };
        assert_eq!(current_target(), expected);
    }

    // ---- outcomes and errors ----

    #[test]
    fn outcome_distinguishes_an_upgrade_from_a_downgrade_and_from_standing_still() {
        let up_to_date = Outcome::UpToDate {
            version: "0.6.0".to_owned(),
        };
        let upgrade = Outcome::Updated {
            from: "0.5.0".to_owned(),
            to: "0.6.0".to_owned(),
        };
        let downgrade = Outcome::Updated {
            from: "0.6.0".to_owned(),
            to: "0.5.0".to_owned(),
        };

        assert_ne!(upgrade, downgrade, "the direction of an update matters");
        assert_ne!(upgrade, up_to_date);
        assert_ne!(
            up_to_date,
            Outcome::UpToDate {
                version: "0.5.0".to_owned()
            }
        );
        assert_eq!(
            upgrade,
            Outcome::Updated {
                from: "0.5.0".to_owned(),
                to: "0.6.0".to_owned()
            }
        );
    }

    #[test]
    fn no_matching_asset_names_the_binary_and_the_target_it_looked_for() {
        let message = UpdateError::NoMatchingAsset {
            binary: "gui".to_owned(),
            target: "aarch64-apple-darwin".to_owned(),
        }
        .to_string();
        assert!(message.contains("gui"), "message was {message}");
        assert!(
            message.contains("aarch64-apple-darwin"),
            "message was {message}"
        );
    }

    #[test]
    fn every_update_error_renders_its_own_message() {
        let messages: Vec<String> = vec![
            UpdateError::Manifest("bad json".to_owned()).to_string(),
            UpdateError::NoMatchingAsset {
                binary: "gui".to_owned(),
                target: "aarch64-apple-darwin".to_owned(),
            }
            .to_string(),
            UpdateError::HashMismatch.to_string(),
            UpdateError::SignatureInvalid.to_string(),
            UpdateError::Io(io::Error::new(io::ErrorKind::PermissionDenied, "denied")).to_string(),
        ];

        for message in &messages {
            assert!(!message.is_empty(), "every variant needs a message");
        }
        let mut distinct = messages.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            messages.len(),
            "each failure must be distinguishable in a log: {messages:?}"
        );
        assert!(messages[0].contains("bad json"), "the cause is carried");
        assert!(messages[4].contains("denied"), "the cause is carried");
    }

    #[test]
    fn an_io_error_converts_into_the_io_variant_keeping_its_cause() {
        let converted = UpdateError::from(io::Error::new(io::ErrorKind::NotFound, "no such file"));
        assert!(matches!(converted, UpdateError::Io(_)));
        assert!(converted.to_string().contains("no such file"));
    }

    // ---- applying the replacement ----

    #[test]
    fn apply_atomic_creates_the_target_when_no_binary_is_installed_yet() {
        let dir = scratch_dir("absent-target");
        let target = dir.join("binary");

        apply_atomic(b"new", &target).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn apply_atomic_replaces_a_longer_binary_without_leaving_a_tail_behind() {
        // A rename gives this for free; an in-place write that forgot to
        // truncate would leave the old binary's tail appended, which is the
        // sort of half-written executable that will not start.
        let dir = scratch_dir("truncation");
        let target = dir.join("binary");
        std::fs::write(&target, vec![b'o'; 4096]).unwrap();

        apply_atomic(b"new", &target).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(std::fs::metadata(&target).unwrap().len(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn apply_atomic_leaves_no_staging_file_beside_the_binary_on_success() {
        let dir = scratch_dir("no-leftovers");
        let target = dir.join("binary");
        std::fs::write(&target, b"old").unwrap();

        apply_atomic(b"new", &target).unwrap();

        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries.len(), 1, "left behind: {entries:?}");
        assert_eq!(entries[0], "binary");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn apply_atomic_fails_when_the_destination_directory_does_not_exist() {
        let dir = scratch_dir("missing-parent");
        let absent = dir.join("no-such-directory");
        let target = absent.join("binary");

        let err = apply_atomic(b"new", &target).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound, "error was {err}");
        assert!(!absent.exists(), "it must not create the directory itself");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn apply_atomic_keeps_the_existing_install_and_clears_up_when_the_rename_fails() {
        // A directory where the binary should be is the portable way to
        // make the rename fail after the replacement has been staged. What
        // matters is that the failure leaves the user with what they had,
        // and with no orphaned staging file.
        let dir = scratch_dir("rename-fails");
        let occupied = dir.join("binary");
        std::fs::create_dir(&occupied).unwrap();
        std::fs::write(occupied.join("inside"), b"precious").unwrap();

        let err = apply_atomic(b"new", &occupied).unwrap_err();

        assert!(occupied.is_dir(), "the original must survive: {err}");
        assert_eq!(std::fs::read(occupied.join("inside")).unwrap(), b"precious");
        assert!(
            !dir.join(".binary.update").exists(),
            "the staged file must be removed when the rename fails"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn apply_atomic_installs_empty_bytes_without_complaint() {
        // Recorded, not asserted as desirable: nothing between the download
        // and the rename rejects a zero-byte payload, so a release that
        // published an empty (but correctly signed) asset would leave the
        // user with a file that cannot be executed. The guard would belong
        // in `download_and_verify`.
        let dir = scratch_dir("empty-payload");
        let target = dir.join("binary");
        std::fs::write(&target, b"a working build").unwrap();

        apply_atomic(b"", &target).unwrap();

        assert_eq!(std::fs::metadata(&target).unwrap().len(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn apply_atomic_marks_the_installed_binary_executable() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = scratch_dir("executable-bit");
        let target = dir.join("binary");

        apply_atomic(b"new", &target).unwrap();

        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o111,
            0o111,
            "an installed binary nobody may execute is no binary at all: {mode:o}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ---- the offline paths of the check itself ----

    #[test]
    fn fetch_manifest_reports_an_unusable_url_as_a_manifest_error() {
        let err = fetch_manifest(UNREACHABLE_URL).unwrap_err();
        assert!(matches!(err, UpdateError::Manifest(_)), "was {err:?}");
        assert!(
            err.to_string().starts_with("could not read the update"),
            "message was {err}"
        );
    }

    #[test]
    fn check_and_update_leaves_the_binary_alone_when_the_manifest_cannot_be_read() {
        // The commonest real condition - no connectivity - must end with
        // the user's install exactly as it was, and nothing staged next to
        // it.
        let dir = scratch_dir("manifest-unreadable");
        let exe = dir.join("binary");
        std::fs::write(&exe, b"the running build").unwrap();

        let err = check_and_update(
            "gui",
            "x86_64-pc-windows-msvc",
            "0.6.0",
            UNREACHABLE_URL,
            &exe,
        )
        .unwrap_err();

        assert!(matches!(err, UpdateError::Manifest(_)), "was {err:?}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"the running build");
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "nothing may be staged beside the binary"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ---- version ordering, which did not exist before ------------------

    /// The case that made this worth fixing: a manifest naming an older
    /// release was not *equal*, so it was fetched, verified and installed
    /// over a newer build.
    #[test]
    fn a_release_older_than_this_build_is_not_newer() {
        assert!(!is_newer("0.5.0", "0.6.0"));
        assert!(!is_newer("0.6.0", "0.6.0"));
        assert!(is_newer("0.7.0", "0.6.0"));
    }

    /// The other half: any cosmetic difference was unequal, so the same
    /// release re-installed itself on every launch, forever.
    #[test]
    fn a_version_spelt_differently_is_still_the_same_release() {
        assert!(!is_newer("v0.6.0", "0.6.0"));
        assert!(!is_newer("0.6.0", "v0.6.0"));
        assert!(!is_newer("0.6.0\n", "0.6.0"));
        assert!(!is_newer(" 0.6.0 ", "0.6.0"));
        assert!(!is_newer("0.6", "0.6.0"));
        assert!(!is_newer("0.6.0", "0.6"));
    }

    /// Numbers, not text. `1.10.0` sorts before `1.9.0` as a string.
    #[test]
    fn versions_are_ordered_as_numbers_rather_than_as_text() {
        assert!(is_newer("1.10.0", "1.9.0"));
        assert!(!is_newer("1.9.0", "1.10.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("2.0.0", "1.99.99"));
    }

    /// A component missing from either side counts as zero, so a longer
    /// spelling of the same release is not an upgrade.
    #[test]
    fn a_missing_component_counts_as_zero() {
        assert!(!is_newer("1", "1.0.0"));
        assert!(!is_newer("1.0.0", "1"));
        assert!(is_newer("1.0.1", "1"));
        assert!(!is_newer("1", "1.0.1"));
    }

    /// A pre-release is earlier than the release it leads to, and a
    /// release is later than its own pre-release - so a build running
    /// `0.7.0-rc1` upgrades to `0.7.0` and not back again.
    #[test]
    fn a_pre_release_is_earlier_than_the_release_it_leads_to() {
        assert!(is_newer("0.7.0", "0.7.0-rc1"));
        assert!(!is_newer("0.7.0-rc1", "0.7.0"));
        assert!(is_newer("0.7.0-rc2", "0.7.0-rc1"));
        assert!(is_newer("0.7.0-rc1", "0.6.9"));
    }

    /// A version nobody can order is not acted on. For something that
    /// replaces the running executable, refusing is the safe direction.
    #[test]
    fn a_version_that_cannot_be_read_is_never_treated_as_newer() {
        assert!(!is_newer("latest", "0.6.0"));
        assert!(!is_newer("0.6.0", "latest"));
        assert!(!is_newer("", "0.6.0"));
        assert!(!is_newer("0.6.0", ""));
        assert!(!is_newer("0.x.0", "0.6.0"));
        assert!(!is_newer("v", "0.6.0"));
    }

    // ---- the decision, now reachable without a network ------------------

    fn manifest_of(version: &str) -> Manifest {
        Manifest {
            version: version.to_owned(),
            targets: vec![TargetAsset {
                binary: "gui".to_owned(),
                target: current_target().to_owned(),
                url: "https://example.invalid/gui".to_owned(),
                sha256: "00".repeat(32),
                signature: "00".repeat(64),
            }],
        }
    }

    #[test]
    fn a_manifest_that_has_gone_backwards_installs_nothing() {
        let manifest = manifest_of("0.5.0");

        let decision = decide(&manifest, "gui", current_target(), "0.6.0");

        assert!(
            matches!(decision, Ok(Decision::UpToDate)),
            "an older release must not be installed over a newer build; got {decision:?}"
        );
    }

    #[test]
    fn a_manifest_naming_this_very_release_installs_nothing() {
        let manifest = manifest_of("0.6.0");

        assert!(matches!(
            decide(&manifest, "gui", current_target(), "v0.6.0"),
            Ok(Decision::UpToDate)
        ));
    }

    #[test]
    fn a_newer_manifest_offers_the_asset_for_this_binary_and_target() {
        let manifest = manifest_of("0.7.0");

        let decision = decide(&manifest, "gui", current_target(), "0.6.0");

        match decision {
            Ok(Decision::Install(asset)) => {
                assert_eq!(asset.binary, "gui");
                assert_eq!(asset.target, current_target());
            }
            other => panic!("a newer release should be installed; got {other:?}"),
        }
    }

    #[test]
    fn a_newer_manifest_with_nothing_for_this_build_says_so() {
        let manifest = manifest_of("0.7.0");

        let decision = decide(&manifest, "tui", current_target(), "0.6.0");

        assert!(
            matches!(decision, Err(UpdateError::NoMatchingAsset { .. })),
            "a release that carries nothing for this binary has to say so \
             rather than install something else; got {decision:?}"
        );
    }

    /// A release with nothing for this build is only an error when it is
    /// newer - an older one is settled before the assets are looked at.
    #[test]
    fn an_older_manifest_with_nothing_for_this_build_is_not_an_error() {
        let manifest = manifest_of("0.5.0");

        assert!(matches!(
            decide(&manifest, "tui", current_target(), "0.6.0"),
            Ok(Decision::UpToDate)
        ));
    }
}
