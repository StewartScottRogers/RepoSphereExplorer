//! On Windows, embeds the application icon as a binary resource. Everywhere
//! else this does nothing.

fn main() {
    embed_icon();
}

/// Puts `assets/RepoSphereExplorer.ico` into the binary's resource directory,
/// which is where Explorer, the taskbar, Alt+Tab and a shortcut look for an
/// application's icon. The terminal front end draws no window of its own, but
/// its file on disk is still something a person meets.
#[cfg(windows)]
fn embed_icon() {
    println!("cargo:rerun-if-changed=../../assets/RepoSphereExplorer.ico");
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../assets/RepoSphereExplorer.ico");
    // Version information the same resource carries, and what Task Manager
    // and Settings read. Without these the binary would name itself `tui`,
    // after the crate.
    resource.set("ProductName", "Repos Explorer");
    resource.set("FileDescription", "Repos Explorer (terminal)");
    resource
        .compile()
        .expect("the icon should compile into the binary");
}

/// Only a Windows binary has a resource directory to put an icon in.
#[cfg(not(windows))]
fn embed_icon() {}
