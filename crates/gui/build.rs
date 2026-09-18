//! Compiles the Slint UI markup into generated Rust types, and on Windows
//! embeds the application icon as a binary resource.

fn main() {
    // Debug info is what lets `tests/context_menu.rs` locate elements through
    // Slint's testing API. It is only emitted for debug builds - which is
    // every build `cargo test` makes - so the release binary keeps none of it.
    let debug_info = std::env::var("PROFILE").is_ok_and(|profile| profile == "debug");
    // On a thread with a stack of its own, because Slint's compiler walks
    // the markup recursively and `ui/app.slint` is now deep enough to
    // overflow the megabyte a Windows build script's main thread gets:
    // every build on Windows failed with STATUS_STACK_OVERFLOW while the
    // Linux runners, which get eight megabytes, saw nothing wrong.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            // The configuration is built here rather than passed in: it
            // holds reference-counted callbacks and cannot cross threads.
            let config = slint_build::CompilerConfiguration::new().with_debug_info(debug_info);
            slint_build::compile_with_config("ui/app.slint", config).unwrap();
        })
        .expect("a thread to compile the markup on")
        .join()
        .expect("the markup should compile");
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
