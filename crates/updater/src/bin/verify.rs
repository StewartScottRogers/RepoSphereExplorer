//! Verifies downloaded release files against a signed update manifest, for
//! the install scripts in `scripts/`.
//!
//! Usage: `verify <manifest.json> <file>...`
//!
//! Each file is matched to its manifest entry by name and held to exactly
//! the check an update is held to - [`updater::verify_bytes`]: its SHA-256
//! digest must be the published one, and the Ed25519 signature over that
//! digest must verify against the public key compiled into this binary.
//! Exits non-zero, naming every file that failed, if any one does.
//!
//! A binary rather than script code because neither Windows PowerShell 5.1
//! nor the `openssl` that macOS ships can check an Ed25519 signature, and a
//! hand-written one in a shell script would be a second, weaker scheme.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [manifest_path, files @ ..] = args.as_slice() else {
        eprintln!("usage: verify <manifest.json> <file>...");
        return ExitCode::from(2);
    };
    if files.is_empty() {
        eprintln!("usage: verify <manifest.json> <file>...");
        return ExitCode::from(2);
    }

    let manifest: updater::Manifest = match std::fs::read(manifest_path)
        .map_err(|err| err.to_string())
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|err| err.to_string()))
    {
        Ok(manifest) => manifest,
        Err(err) => {
            eprintln!("refusing: could not read the manifest {manifest_path}: {err}");
            return ExitCode::FAILURE;
        }
    };

    let mut failed = false;
    for file in files {
        match check(&manifest, Path::new(file)) {
            Ok(()) => println!("verified {file}"),
            Err(reason) => {
                eprintln!("refusing {file}: {reason}");
                failed = true;
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn check(manifest: &updater::Manifest, path: &Path) -> Result<(), String> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let asset = manifest.find_file(&name).ok_or_else(|| {
        format!(
            "release {} publishes no file named {name}",
            manifest.version
        )
    })?;
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    updater::verify_bytes(asset, &bytes).map_err(|err| err.to_string())
}
