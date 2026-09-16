//! Compiles the Slint UI markup into generated Rust types, and on Windows
//! embeds the application icon as a binary resource.

fn main() {
    // Debug info is what lets `tests/context_menu.rs` locate elements through
    // Slint's testing API. It is only emitted for debug builds - which is
    // every build `cargo test` makes - so the release binary keeps none of it.
    let debug_info = std::env::var("PROFILE").is_ok_and(|profile| profile == "debug");
    let config = slint_build::CompilerConfiguration::new().with_debug_info(debug_info);
    slint_build::compile_with_config("ui/app.slint", config).unwrap();
    embed_icon();
}

/// Puts `assets/RepoSphereExplorer.ico` into the binary's resource directory,
/// which is where Explorer, the taskbar, Alt+Tab and a shortcut look for an
/// application's icon.
#[cfg(windows)]
fn embed_icon() {
    println!("cargo:rerun-if-changed=../../assets/RepoSphereExplorer.ico");
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../assets/RepoSphereExplorer.ico");
    // Version information the same resource carries, and what Task Manager
    // and Settings read. Without these the binary would name itself `gui`,
    // after the crate.
    resource.set("ProductName", "Repos Explorer");
    resource.set("FileDescription", "Repos Explorer");
    resource
        .compile()
        .expect("the icon should compile into the binary");
}

/// Only a Windows binary has a resource directory to put an icon in; the
/// graphical front end's window icon comes from the markup on every system.
#[cfg(not(windows))]
fn embed_icon() {}
