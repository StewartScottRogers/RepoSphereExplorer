//! The `verify` binary the install scripts call, run as they run it.
//!
//! Only refusals can be proven here: a file that verifies needs a signature
//! from the release signing key, which exists only as a GitHub Actions
//! secret. The accepting path is exercised by `distribution.yml`, against
//! a release's real signed manifest.

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rse-verify-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// Writes a manifest publishing one file, `name`, with the given digest and
/// signature, and returns its path.
fn manifest(dir: &Path, name: &str, sha256: &str, signature: &str) -> PathBuf {
    let path = dir.join("manifest.json");
    let json = format!(
        r#"{{"version": "0.7.0", "targets": [{{"binary": "service",
            "target": "x86_64-unknown-linux-gnu",
            "url": "https://example.invalid/releases/download/v0.7.0/{name}",
            "sha256": "{sha256}", "signature": "{signature}"}}]}}"#
    );
    std::fs::write(&path, json).unwrap();
    path
}

fn verify(manifest: &Path, file: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_verify"))
        .arg(manifest)
        .arg(file)
        .output()
        .unwrap()
}

#[test]
fn a_file_altered_after_signing_is_refused() {
    let dir = scratch("tampered");
    let original = b"the service as released";
    let digest = Sha256::digest(original);
    let manifest = manifest(
        &dir,
        "service-x86_64-unknown-linux-gnu",
        &hex(&digest),
        &"00".repeat(64),
    );
    let file = dir.join("service-x86_64-unknown-linux-gnu");
    std::fs::write(&file, b"the service as released, plus a payload").unwrap();

    let output = verify(&manifest, &file);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("did not match the published hash"),
        "stderr: {stderr}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_digest_rewritten_to_match_is_still_refused_on_its_signature() {
    let dir = scratch("resigned");
    let bytes = b"a service somebody else built";
    let digest = Sha256::digest(bytes);
    let other_key = SigningKey::generate(&mut rand::rng());
    let signature = hex(&other_key.sign(&digest).to_bytes());
    let manifest = manifest(
        &dir,
        "service-x86_64-unknown-linux-gnu",
        &hex(&digest),
        &signature,
    );
    let file = dir.join("service-x86_64-unknown-linux-gnu");
    std::fs::write(&file, bytes).unwrap();

    let output = verify(&manifest, &file);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("signature verification failed"),
        "stderr: {stderr}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_the_manifest_does_not_publish_is_refused() {
    let dir = scratch("unlisted");
    let manifest = manifest(&dir, "service-x86_64-unknown-linux-gnu", "aa", "bb");
    let file = dir.join("something-else");
    std::fs::write(&file, b"anything").unwrap();

    let output = verify(&manifest, &file);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("publishes no file named something-else"),
        "stderr: {stderr}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
