//! Rewrites `assets/RepoSphereExplorer.ico` from
//! `assets/RepoSphereExplorer.svg`.
//!
//! `cargo run -p icon`. Nothing runs it during a build: the icon is a
//! committed asset, because a resource compiler needs a file on disk and a
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
    Ok(())
}
