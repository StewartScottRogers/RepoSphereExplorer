//! Rewrites `assets/RepoSphereExplorer.ico` and
//! `assets/RepoSphereExplorer.icns` from `assets/RepoSphereExplorer.svg`.
//!
//! `cargo run -p icon`. Nothing runs it during a build: the icons are
//! committed assets, because a resource compiler needs a file on disk, the
//! macOS bundle is laid out on a runner that has no rasteriser of ours, and a
//! release should carry the same picture the repository shows.

use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let drawing = fs::read_to_string(assets.join("RepoSphereExplorer.svg"))?;

    let bytes = icon::ico(&drawing)?;
    let written = assets.join("RepoSphereExplorer.ico");
    fs::write(&written, &bytes)?;
    println!(
        "wrote {} sizes, {} bytes, to {}",
        icon::SIZES.len(),
        bytes.len(),
        written.display()
    );

    let bytes = icon::icns(&drawing)?;
    let written = assets.join("RepoSphereExplorer.icns");
    fs::write(&written, &bytes)?;
    println!(
        "wrote {} types, {} bytes, to {}",
        icon::ICNS_ENTRIES.len(),
        bytes.len(),
        written.display()
    );

    Ok(())
}
