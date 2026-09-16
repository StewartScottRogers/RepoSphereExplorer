//! Rewrites what is drawn from `assets/RepoSphereExplorer.svg`.
//!
//! `cargo run -p icon` rewrites `assets/RepoSphereExplorer.ico`, which the two
//! Windows binaries embed, `assets/RepoSphereExplorer.icns`, which the macOS
//! application bundle carries, and the generated icon section of
//! `scripts/install.sh`, which brings the free desktop's themed icon to a
//! machine that has nothing but that script. Nothing runs any of it during a
//! build: a resource compiler needs a file on disk, the bundle is laid out on
//! a runner with no rasteriser of ours, a reader downloads the script on its
//! own, and a release should carry the same picture the repository shows.
//!
//! `cargo run -p icon -- --icons <dir>` writes the hicolor icon theme's tree
//! under `<dir>` instead - `<size>x<size>/apps/<name>.png` for each size and
//! `scalable/apps/<name>.svg` - which is what `scripts/appimage.sh` puts
//! inside the `AppImage`.

use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let drawing = fs::read_to_string(root.join("assets/RepoSphereExplorer.svg"))?;

    let mut arguments = std::env::args().skip(1);
    match arguments.next().as_deref() {
        None => rewrite_committed(&root, &drawing),
        Some("--icons") => {
            let into = arguments.next().ok_or("--icons needs a directory")?;
            write_theme(Path::new(&into), &drawing)
        }
        Some(other) => Err(format!("unknown argument: {other}").into()),
    }
}

/// Rewrites the three committed things the drawing produces.
fn rewrite_committed(root: &Path, drawing: &str) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = icon::ico(drawing)?;
    let written = root.join("assets/RepoSphereExplorer.ico");
    fs::write(&written, &bytes)?;
    println!(
        "wrote {} sizes, {} bytes, to {}",
        icon::SIZES.len(),
        bytes.len(),
        written.display()
    );

    let bytes = icon::icns(drawing)?;
    let written = root.join("assets/RepoSphereExplorer.icns");
    fs::write(&written, &bytes)?;
    println!(
        "wrote {} types, {} bytes, to {}",
        icon::ICNS_ENTRIES.len(),
        bytes.len(),
        written.display()
    );

    let script_path = root.join("scripts/install.sh");
    let script = fs::read_to_string(&script_path)?;
    let updated = icon::script_with_icons(&script, &icon::install_script_icons(drawing)?)?;
    if updated == script {
        println!("{} already carries this drawing", script_path.display());
    } else {
        fs::write(&script_path, &updated)?;
        println!("rewrote the icon section of {}", script_path.display());
    }
    Ok(())
}

/// Writes the hicolor icon theme's directories under `into`.
fn write_theme(into: &Path, drawing: &str) -> Result<(), Box<dyn std::error::Error>> {
    for size in icon::SIZES {
        let directory = into.join(format!("{size}x{size}")).join("apps");
        fs::create_dir_all(&directory)?;
        fs::write(
            directory.join(format!("{}.png", icon::THEMED_NAME)),
            icon::png(drawing, size)?,
        )?;
    }
    let scalable = into.join("scalable").join("apps");
    fs::create_dir_all(&scalable)?;
    fs::write(scalable.join(format!("{}.svg", icon::THEMED_NAME)), drawing)?;
    println!(
        "wrote {} sizes and the drawing to {}",
        icon::SIZES.len(),
        into.display()
    );
    Ok(())
}
