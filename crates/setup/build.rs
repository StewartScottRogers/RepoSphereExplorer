//! Compiles the setup window's Slint markup, and on Windows embeds the
//! application icon as a binary resource.

fn main() {
    slint_build::compile("ui/setup.slint").unwrap();
    embed_icon();
}

/// Puts `assets/RepoSphereExplorer.ico` into the binary's resource
/// directory. The setup program is the first file a reader downloads, so it
/// is the first one that should not look like a generic program.
#[cfg(windows)]
fn embed_icon() {
    println!("cargo:rerun-if-changed=../../assets/RepoSphereExplorer.ico");
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../assets/RepoSphereExplorer.ico");
    // Version information the same resource carries, and what Task Manager
    // and Settings read. Without these the binary would name itself `setup`,
    // after the crate.
    resource.set("ProductName", "Repos Explorer");
    resource.set("FileDescription", "Repos Explorer Setup");
    resource
        .compile()
        .expect("the icon should compile into the binary");
}

/// Only a Windows binary has a resource directory to put an icon in.
#[cfg(not(windows))]
fn embed_icon() {}
