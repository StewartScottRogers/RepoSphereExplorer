//! The application icon, at the two places the graphical front end meets it.
//!
//! `icon`'s own tests know that the committed `.ico` is six sizes of the
//! committed drawing. What they cannot say is whether either file reaches
//! this crate: the build script names the icon by a relative path nothing
//! else checks, and the markup names the drawing by another. Both are joins
//! that a rename would break silently - on Linux and macOS the build script
//! does nothing at all, so a missing icon would not even fail the build.

/// What `build.rs` hands the resource compiler, spelled the same way.
const EMBEDDED: &str = "../../assets/RepoSphereExplorer.ico";

#[test]
fn the_build_script_embeds_the_committed_icon() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(EMBEDDED);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|err| panic!("build.rs points at {}: {err}", path.display()));
    assert_eq!(
        bytes,
        icon::COMMITTED,
        "the icon build.rs embeds is not the committed one"
    );
}

#[test]
fn the_window_is_given_the_icon_drawing() {
    // The headless backend, as every other window test uses: a continuous
    // integration runner has no display, and asking for a real one there
    // fails before the markup is ever read.
    i_slint_backend_testing::init_no_event_loop();
    let ui = gui::MainWindow::new().expect("the window should build");
    let size = ui.get_window_icon().size();
    assert_eq!(
        (size.width, size.height),
        (256, 256),
        "the window icon should be the drawing, at its own size"
    );
}

/// The picture Explorer, the taskbar, Alt+Tab and a shortcut read: an icon
/// group naming six images, each of them an entry of the committed `.ico`.
///
/// Windows only, because only a Windows binary has a resource directory. The
/// binary read here is the one this test run built.
#[cfg(windows)]
#[test]
fn the_built_binary_carries_the_icon() {
    let bytes = std::fs::read(env!("CARGO_BIN_EXE_RepoSphereExplorerGui"))
        .expect("the graphical binary this test run built");
    let carried = icon::icon_resources(&bytes).expect("the binary has a resource directory");
    assert_eq!(
        carried.groups, 1,
        "one icon group, so Windows has one picture to choose a size from"
    );
    let committed: Vec<&[u8]> = icon::entries(icon::COMMITTED)
        .expect("the committed icon is an icon file")
        .iter()
        .map(|entry| entry.payload)
        .collect();
    assert_eq!(
        carried.images, committed,
        "the binary's icon images are not the committed icon's"
    );
}
