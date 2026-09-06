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

        targets.push(TargetAsset {
            binary,
            target,
            url: format!("https://github.com/{repo}/releases/download/{tag}/{filename}"),
            sha256: hex_encode(&digest),
            signature: hex_encode(&signature.to_bytes()),
        });
    }

    let manifest = Manifest {
        version: version.clone(),
        targets,
    };
    let json = serde_json::to_string_pretty(&manifest).expect("serialize manifest");
    fs::write(Path::new(output_path), json).expect("write manifest");
    println!("wrote {output_path}");
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
