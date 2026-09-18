//! Popping a pane out into its own window, and docking it back (#617).
//!
//! `shared_selection_in_the_window.rs` already proves the three panes of
//! one window agree on one shared selection. This proves the same join
//! survives a pane moving into a window of its own: a real
//! [`gui::PaneWindows`] registry, built with `gui::wire_callbacks` and
//! `gui::wire_pop_out` - the same functions `main` calls - against one
//! shared `App` and one shared service, per CLAUDE.md rule 14.

use gui::app::{App, Pane};
use gui::{MainWindow, PaneWindows, sync_ui};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, Model};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

mod common;
use common::ensure_service;

/// 0 Folders, 1 Contents, 2 File - matching `pop-out-requested`'s index and
/// `lib.rs`'s own (private) `pane_index`.
fn pane_index(pane: Pane) -> i32 {
    match pane {
        Pane::Folders => 0,
        Pane::Contents => 1,
        Pane::File => 2,
    }
}

/// A scratch directory of this test's own: a child folder holding a file of
/// its own, so navigating into it and selecting that file are each visible
/// in a different pane.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("repos-explorer-pop-out")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).expect("a scratch directory");
    std::fs::write(dir.join("demo.txt"), "root file\n").expect("a scratch file");
    std::fs::write(dir.join("sub").join("nested.txt"), "nested file\n")
        .expect("a nested scratch file");
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

/// A strong handle to the window holding `pane` right now, gotten and
/// dropped out of `windows`'s borrow in one statement - so the caller can
/// invoke a callback on it without still holding that borrow, which a
/// pop-out or dock callback re-enters (#617).
fn pane_handle(windows: &Rc<RefCell<PaneWindows>>, pane: Pane) -> MainWindow {
    windows
        .borrow()
        .window_for(pane)
        .as_weak()
        .upgrade()
        .expect("the window has not been dropped yet")
}

/// The same, for the main window itself - `main_handle(&windows)` rather
/// than `windows.borrow().main()`, for the same reason [`pane_handle`] does.
fn main_handle(windows: &Rc<RefCell<PaneWindows>>) -> MainWindow {
    windows
        .borrow()
        .main()
        .as_weak()
        .upgrade()
        .expect("the window has not been dropped yet")
}

thread_local! {
    /// `init_no_event_loop` sets the testing backend for the calling
    /// thread; a second call on the same thread panics rather than being
    /// a no-op, and this file - unlike every other window test - opens
    /// more than one window (and so calls `windows_at` more than once)
    /// per `#[test]`.
    static TESTING_BACKEND_READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A shown main window on a real `App` rooted at `root`, registered as the
/// only entry of a fresh [`PaneWindows`], wired exactly as `main` wires it.
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
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let windows = Rc::new(RefCell::new(PaneWindows::new(ui)));
    gui::wire_pop_out(windows.borrow().main(), None, &windows, &app);
    pump(&windows, &app);
    (windows, app)
}

#[test]
fn each_pane_can_be_popped_out_and_docked_back() {
    for pane in [Pane::Folders, Pane::Contents, Pane::File] {
        let (windows, app) = windows_at(&scratch(&format!("{pane:?}")));
        let index = pane_index(pane);

        // Nothing is out yet: the main window holds all three.
        assert!(!windows.borrow().is_popped_out(pane));
        {
            let borrowed = windows.borrow();
            assert!(std::ptr::eq(borrowed.window_for(pane), borrowed.main()));
        }

        pane_handle(&windows, pane).invoke_pop_out_requested(index);
        pump(&windows, &app);

        // A new window holds the pane; the main window no longer does.
        assert!(
            windows.borrow().is_popped_out(pane),
            "{pane:?} should be popped out"
        );
        {
            let borrowed = windows.borrow();
            let popped = borrowed.window_for(pane);
            assert!(
                !std::ptr::eq(popped, borrowed.main()),
                "the popped-out window is a different window from the main one"
            );
            assert!(!popped.get_is_main_window());
            match pane {
                Pane::Folders => {
                    assert!(popped.get_show_folders_pane());
                    assert!(!borrowed.main().get_show_folders_pane());
                }
                Pane::Contents => {
                    assert!(popped.get_show_contents_pane());
                    assert!(!borrowed.main().get_show_contents_pane());
                }
                Pane::File => {
                    assert!(popped.get_show_file_pane());
                    assert!(!borrowed.main().get_show_file_pane());
                }
            }
            assert!(
                popped.get_window_title().starts_with("Repos Explorer - "),
                "the popped-out window is titled for its pane: {}",
                popped.get_window_title()
            );
        }

        // Selecting in the popped-out window updates the other panes: a
        // folder click there is a real navigation `App` and every window
        // agree happened, the same join `shared_selection_in_the_window.rs`
        // proves for one window (#614).
        if pane == Pane::Folders {
            windows
                .borrow()
                .window_for(Pane::Folders)
                .invoke_folder_row_clicked(1, 999.0);
            pump(&windows, &app);
            assert_eq!(app.borrow().selection().folder, 1);
            // The main window no longer draws the Folders pane, but it is
            // still handed the same state every other window is (#617
            // point 3): the timer syncs every open window.
            assert_eq!(windows.borrow().main().get_folder_selected(), 1);
        }

        // And vice versa: selecting in a window that still holds another
        // pane reaches the popped-out one too.
        if pane != Pane::Folders {
            windows.borrow().main().invoke_folder_row_clicked(1, 999.0);
            pump(&windows, &app);
            assert_eq!(app.borrow().selection().folder, 1);
            assert_eq!(
                windows.borrow().window_for(pane).get_folder_selected(),
                1,
                "the popped-out {pane:?} window is handed the same state too"
            );
        }

        // Docking closes the window and puts the pane back.
        pane_handle(&windows, pane).invoke_dock_requested(index);
        pump(&windows, &app);
        assert!(!windows.borrow().is_popped_out(pane));
        {
            let borrowed = windows.borrow();
            match pane {
                Pane::Folders => assert!(borrowed.main().get_show_folders_pane()),
                Pane::Contents => assert!(borrowed.main().get_show_contents_pane()),
                Pane::File => assert!(borrowed.main().get_show_file_pane()),
            }
        }
    }
}

#[test]
fn two_panes_popped_out_at_once_still_share_the_selection() {
    let (windows, app) = windows_at(&scratch("two-at-once"));

    pane_handle(&windows, Pane::Contents).invoke_pop_out_requested(pane_index(Pane::Contents));
    pump(&windows, &app);
    pane_handle(&windows, Pane::File).invoke_pop_out_requested(pane_index(Pane::File));
    pump(&windows, &app);

    assert!(windows.borrow().is_popped_out(Pane::Contents));
    assert!(windows.borrow().is_popped_out(Pane::File));
    // Only Folders is left in the main window now.
    assert!(windows.borrow().main().get_show_folders_pane());
    assert!(!windows.borrow().main().get_show_contents_pane());
    assert!(!windows.borrow().main().get_show_file_pane());

    // Selecting the child folder in the main window is a real navigation:
    // the popped-out Contents window has to show what is actually in it.
    windows.borrow().main().invoke_folder_row_clicked(1, 999.0);
    pump(&windows, &app);
    assert_eq!(app.borrow().selection().folder, 1);
    {
        let borrowed = windows.borrow();
        let contents = borrowed.window_for(Pane::Contents);
        let rows = contents.get_content_rows();
        assert_eq!(rows.row_count(), 1, "the child folder holds one file");
        assert_eq!(rows.row_data(0).expect("a row").name, "nested.txt");
    }

    // Selecting that file in the popped-out Contents window has to reach
    // the popped-out File window too.
    windows
        .borrow()
        .window_for(Pane::Contents)
        .invoke_content_row_clicked(0);
    pump(&windows, &app);
    {
        let selection = app.borrow().selection();
        assert_eq!(selection.content, 0);
        assert_eq!(selection.focus, Pane::Contents);
    }
    {
        let borrowed = windows.borrow();
        let file_window = borrowed.window_for(Pane::File);
        assert!(
            file_window.get_file_text().contains("nested file"),
            "the popped-out File window shows the file selected in the popped-out Contents window: {}",
            file_window.get_file_text()
        );
    }
}

/// Closing a popped-out window - a real `CloseRequested` event, the one a
/// platform's own decoration or Alt+F4 sends - docks its pane back into the
/// main window, exactly as its own dock button does (#618).
#[test]
fn closing_a_popped_out_window_docks_its_pane_back() {
    let (windows, app) = windows_at(&scratch("close-popped-docks"));
    pane_handle(&windows, Pane::Contents).invoke_pop_out_requested(pane_index(Pane::Contents));
    pump(&windows, &app);
    assert!(windows.borrow().is_popped_out(Pane::Contents));

    pane_handle(&windows, Pane::Contents)
        .window()
        .dispatch_event(WindowEvent::CloseRequested);
    pump(&windows, &app);

    assert!(!windows.borrow().is_popped_out(Pane::Contents));
    assert!(windows.borrow().main().get_show_contents_pane());
}

/// Popping every pane out leaves the main window holding none of them: the
/// empty-main-window message and Dock All button take over, and Dock All
/// returns all three panes and closes their windows (#618).
#[test]
fn popping_out_every_pane_lets_dock_all_return_them() {
    let (windows, app) = windows_at(&scratch("dock-all"));
    for pane in [Pane::Folders, Pane::Contents, Pane::File] {
        pane_handle(&windows, pane).invoke_pop_out_requested(pane_index(pane));
        pump(&windows, &app);
    }

    {
        // The condition the markup shows the empty-main-window message and
        // Dock All button under: every pane is out, so none of it is drawn.
        let borrowed = windows.borrow();
        assert!(!borrowed.main().get_show_folders_pane());
        assert!(!borrowed.main().get_show_contents_pane());
        assert!(!borrowed.main().get_show_file_pane());
        assert!(borrowed.is_popped_out(Pane::Folders));
        assert!(borrowed.is_popped_out(Pane::Contents));
        assert!(borrowed.is_popped_out(Pane::File));
    }

    main_handle(&windows).invoke_dock_all_requested();
    pump(&windows, &app);

    let borrowed = windows.borrow();
    assert!(!borrowed.is_popped_out(Pane::Folders));
    assert!(!borrowed.is_popped_out(Pane::Contents));
    assert!(!borrowed.is_popped_out(Pane::File));
    assert!(borrowed.main().get_show_folders_pane());
    assert!(borrowed.main().get_show_contents_pane());
    assert!(borrowed.main().get_show_file_pane());
}

/// Closing the main window while a pane is popped out closes only the main
/// window: the popped-out one keeps running and still responds to
/// selection. Closing that last remaining window then leaves nothing
/// visible, which is what ends the event loop (GUIDANCE.md §2.6's "the
/// application exits with its last window") (#618).
#[test]
fn closing_the_main_window_leaves_a_popped_out_one_running_and_the_last_close_ends_it() {
    let (windows, app) = windows_at(&scratch("close-main"));
    pane_handle(&windows, Pane::Contents).invoke_pop_out_requested(pane_index(Pane::Contents));
    pump(&windows, &app);

    main_handle(&windows)
        .window()
        .dispatch_event(WindowEvent::CloseRequested);
    pump(&windows, &app);

    assert!(!windows.borrow().main().window().is_visible());
    assert!(
        windows
            .borrow()
            .window_for(Pane::Contents)
            .window()
            .is_visible(),
        "the popped-out window is still there, and the application with it"
    );

    // Still responds to a real selection, made in the popped-out window.
    pane_handle(&windows, Pane::Contents).invoke_folder_row_clicked(1, 999.0);
    pump(&windows, &app);
    assert_eq!(app.borrow().selection().folder, 1);
    assert_eq!(
        windows
            .borrow()
            .window_for(Pane::Contents)
            .get_folder_selected(),
        1
    );

    // Closing that popped-out window docks its pane back into the
    // already-closed main window, leaving no window visible at all.
    pane_handle(&windows, Pane::Contents)
        .window()
        .dispatch_event(WindowEvent::CloseRequested);
    pump(&windows, &app);
    assert!(!windows.borrow().is_popped_out(Pane::Contents));
    assert!(!windows.borrow().main().window().is_visible());
}

/// "View > Show Main Window", reachable from a popped-out window's own menu,
/// brings a closed main window back (#618).
#[test]
fn show_main_window_brings_a_closed_main_window_back() {
    let (windows, app) = windows_at(&scratch("show-main"));
    pane_handle(&windows, Pane::Contents).invoke_pop_out_requested(pane_index(Pane::Contents));
    pump(&windows, &app);

    main_handle(&windows)
        .window()
        .dispatch_event(WindowEvent::CloseRequested);
    pump(&windows, &app);
    assert!(!windows.borrow().main().window().is_visible());

    pane_handle(&windows, Pane::Contents).invoke_show_main_window_requested();
    pump(&windows, &app);
    assert!(windows.borrow().main().window().is_visible());
}
