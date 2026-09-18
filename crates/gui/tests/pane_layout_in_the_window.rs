//! Restoring which panes were popped out, and where, between launches
//! (#620).
//!
//! `pop_out_windows.rs` already proves a pane popping out and docking back
//! within one run, and `pin_the_pane.rs` proves a pinned pop-out's own
//! life. This proves the layout survives to the next run: `gui`'s own
//! `restore_pane_layout` and `PaneWindows::pane_layout`, driven against a
//! real `PaneWindows` registry built with `gui::wire_callbacks` and
//! `gui::wire_pop_out` - the same functions `main` calls - against one
//! shared `App` and one shared service, per CLAUDE.md rule 14.

use gui::app::{App, Pane};
use gui::settings::{PaneLayout, WindowGeometry};
use gui::{MainWindow, PaneWindows, sync_ui};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

mod common;
use common::ensure_service;

/// A scratch directory of this test's own.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("repos-explorer-pane-layout")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    std::fs::write(dir.join("demo.txt"), "root file\n").expect("a scratch file");
    dir
}

/// Whether `status` is one of the transient lines an in-flight request puts
/// up, which is how [`pump`] knows the application is still working.
fn still_working(status: &str) -> bool {
    status.starts_with("loading ") || status == "working..."
}

/// Ticks the application and syncs every open window, the way `main`'s
/// 100ms timer does, until every in-flight request has landed.
fn pump(windows: &Rc<RefCell<PaneWindows>>, app: &Rc<RefCell<App>>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut quiet = 0u32;
    loop {
        assert!(Instant::now() < deadline, "the application never settled");
        let (busy, status) = {
            let mut app = app.borrow_mut();
            app.tick();
            let windows = windows.borrow();
            for ui in windows.windows() {
                sync_ui(ui, &app);
            }
            (app.is_busy(), windows.main().get_status_text().to_string())
        };
        if busy || still_working(&status) {
            quiet = 0;
        } else {
            quiet += 1;
            if quiet >= 8 {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

thread_local! {
    /// `init_no_event_loop` sets the testing backend for the calling
    /// thread; a second call on the same thread panics rather than being
    /// a no-op, and this file - like `pop_out_windows.rs` - opens more
    /// than one window (and so calls `windows_at` more than once) per
    /// `#[test]`.
    static TESTING_BACKEND_READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A shown main window on a real `App` rooted at `root`, registered as the
/// only entry of a fresh [`PaneWindows`], wired exactly as `main` wires it -
/// with nothing popped out or restored yet.
fn windows_at(root: &Path) -> (Rc<RefCell<PaneWindows>>, Rc<RefCell<App>>) {
    ensure_service();
    TESTING_BACKEND_READY.with(|ready| {
        if !ready.get() {
            i_slint_backend_testing::init_no_event_loop();
            ready.set(true);
        }
    });
    let app = Rc::new(RefCell::new(App::new(root.to_path_buf())));
    let ui = MainWindow::new().expect("the window should build");
    gui::wire_callbacks(&ui, &app);
    sync_ui(&ui, &app.borrow());
    ui.show().expect("the window should show");
    let windows = Rc::new(RefCell::new(PaneWindows::new(ui)));
    gui::wire_pop_out(windows.borrow().main(), None, &windows, &app);
    pump(&windows, &app);
    (windows, app)
}

/// A strong handle to the window holding `pane` right now, gotten and
/// dropped out of `windows`'s borrow in one statement - so the caller can
/// invoke a callback on it without still holding that borrow, which a
/// pop-out or pin callback re-enters.
fn pane_handle(windows: &Rc<RefCell<PaneWindows>>, pane: Pane) -> MainWindow {
    windows
        .borrow()
        .window_for(pane)
        .as_weak()
        .upgrade()
        .expect("the window has not been dropped yet")
}

/// A remembered window at `x`, with a fixed size, docked nowhere.
const fn geometry(x: f32) -> WindowGeometry {
    WindowGeometry {
        x,
        y: 80.0,
        width: 500.0,
        height: 400.0,
        maximized: false,
    }
}

/// #620's acceptance check 3: restoring a layout naming Folders opens it
/// popped out, and does not restore a location - the listing is still at
/// the Repos Directory (D7), because a `PaneLayout` holds no path at all.
#[test]
fn a_remembered_layout_opens_its_pane_popped_out_at_the_repos_directory() {
    let root = scratch("restore-folders");
    let (windows, app) = windows_at(&root);

    let layout = PaneLayout {
        folders: Some(geometry(50.0)),
        contents: None,
        file: None,
    };
    gui::restore_pane_layout(layout, &windows, &app);
    pump(&windows, &app);

    assert!(
        windows.borrow().is_popped_out(Pane::Folders),
        "Folders should have opened popped out"
    );
    assert!(!windows.borrow().is_popped_out(Pane::Contents));
    assert!(!windows.borrow().is_popped_out(Pane::File));
    assert!(
        !pane_handle(&windows, Pane::Folders).get_is_main_window(),
        "the popped-out window is not the main one"
    );
    assert_eq!(
        app.borrow().current_path(),
        root.to_string_lossy(),
        "D7: restoring the layout must not restore a location too"
    );
}

/// #620's acceptance check 2: a window a pin ever opened is never part of
/// the saved layout, even while it is still open and pinned - and what is
/// saved holds no path, folder or file name either way.
#[test]
fn a_pinned_window_at_exit_is_excluded_from_the_saved_layout_and_it_holds_no_path() {
    let root = scratch("pin-excluded");
    let (windows, app) = windows_at(&root);

    pane_handle(&windows, Pane::File).invoke_pop_out_requested(2);
    pump(&windows, &app);
    let popped = pane_handle(&windows, Pane::File);
    popped.invoke_pin_requested();
    pump(&windows, &app);
    assert!(popped.get_pinned(), "pinning should have taken");

    let layout = windows.borrow().pane_layout();
    assert_eq!(
        layout.file, None,
        "a pinned window is not part of the pop-out layout (requirement 3)"
    );
    assert_eq!(layout.folders, None);
    assert_eq!(layout.contents, None);

    let written = format!("{layout:?}");
    assert!(
        !written.contains('/') && !written.contains('\\'),
        "the saved layout must hold no path: {written}"
    );
}
