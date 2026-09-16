//! Builds `Repos Explorer.app` from a folder of built executables.
//!
//! Usage: `macos-bundle <binaries-dir> <icon.icns> <bundle-path>`
//!
//! `release.yml` runs it on the macOS runner, over
//! `target/aarch64-apple-darwin/release` and `assets/RepoSphereExplorer.icns`,
//! before `hdiutil` puts the result in the disk image.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [binaries, icon, bundle] = args.as_slice() else {
        eprintln!("usage: macos-bundle <binaries-dir> <icon.icns> <bundle-path>");
        return ExitCode::from(2);
    };
    let bundle = Path::new(bundle);
    match macos_bundle::lay_out(
        bundle,
        Path::new(binaries),
        Path::new(icon),
        env!("CARGO_PKG_VERSION"),
    ) {
        Ok(()) => {
            println!(
                "built {} for version {}",
                bundle.display(),
                env!("CARGO_PKG_VERSION")
            );
            ExitCode::SUCCESS
        }
        Err(reason) => {
            eprintln!("macos-bundle: {reason}");
            ExitCode::FAILURE
        }
    }
}
