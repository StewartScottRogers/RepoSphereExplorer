//! One shared selection, driven from all three panes of a real window on a
//! real `App`.
//!
//! `App`'s own unit tests prove that an intent method such as
//! `select_folder` or `extend_selection_to` leaves the right
//! [`gui::app::Selection`] behind, with no window in front of it.
//! `editing_in_the_window.rs` and `file_pane_in_the_window.rs` each drive
//! one pane through a real window and check what that pane draws. The
//! join nobody has proved is the point of work order #614: that clicking
//! in the Folders pane, then the Contents pane, then the File pane, each
//! leaves `App::selection()` and every pane's own properties saying the
//! same thing - not only the pane just clicked.
//!
//! Built through `gui::wire_callbacks`, the same function `main` calls,
//! against a real service (CLAUDE.md rule 14), so a folder click is a
//! real navigation and a file click is a real preview, not a hand-planted
//! fixture standing in for one.

use gui::app::{App, Pane};
use gui::{MainWindow, sync_ui};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, Model};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

mod common;
use common::ensure_service;

/// A scratch directory of this test's own: one child folder and one Rust
/// file with real definitions in it, so its preview offers more than one
/// view to switch between.
fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join("repos-explorer-shared-selection");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).expect("a scratch directory");
    std::fs::write(dir.join("demo.rs"), "fn demo() {}\n").expect("a scratch file");
    dir
}

/// Whether `status` is one of the transient lines an in-flight request puts
/// up, which is how [`pump`] knows the application is still working.
fn still_working(status: &str) -> bool {
    status.starts_with("loading ") || status == "working..."
}

/// Ticks the application the way the window's 100ms timer does, until every
/// in-flight request has landed.
fn pump(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut quiet = 0u32;
    while Instant::now() < deadline {
        let busy = {
            let mut app = app.borrow_mut();
            app.tick();
            sync_ui(ui, &app);
            // A file preview is asked for without putting a line in the
            // status bar, so the status alone says "settled" while one is
            // still in flight - and the assertion below about how many
            // views a Rust file offers then reads the view before it. On a
            // loaded continuous integration runner that is the difference
            // between passing and failing, as it was for the other window
            // tests that already ask this.
            app.is_busy()
        };
        if busy || still_working(&ui.get_status_text()) {
            quiet = 0;
        } else {
            quiet += 1;
            if quiet >= 8 {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "the application never settled; status: {}",
        ui.get_status_text()
    );
}

/// A shown window on a real `App` rooted at `root`, wired as `main` wires
/// it, with the opening listing already loaded.
fn window_at(root: &Path) -> (MainWindow, Rc<RefCell<App>>) {
    ensure_service();
    i_slint_backend_testing::init_no_event_loop();
    let app = Rc::new(RefCell::new(App::new(root.to_path_buf())));
    let ui = MainWindow::new().expect("the window should build");
    gui::wire_callbacks(&ui, &app);
    sync_ui(&ui, &app.borrow());
    ui.show().expect("the window should show");
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    pump(&ui, &app);
    (ui, app)
}

/// The index of the content row named `name`, as the pane is drawing it.
fn content_row_index(ui: &MainWindow, name: &str) -> i32 {
    let rows = ui.get_content_rows();
    (0..rows.row_count())
        .find(|&i| rows.row_data(i).is_some_and(|row| row.name == name))
        .and_then(|i| i32::try_from(i).ok())
        .unwrap_or_else(|| panic!("no content row named {name}: {:?}", ui.get_content_rows()))
}

#[test]
fn each_pane_can_change_the_shared_selection_and_the_others_show_it() {
    let (ui, app) = window_at(&scratch());

    // The Folders pane: clicking the child folder selects it and gives
    // the tree focus. `App::selection()` and every relevant window
    // property have to say the same thing about which pane last moved.
    assert_eq!(
        app.borrow().folder_rows().len(),
        2,
        "the root and its one child"
    );
    ui.invoke_folder_row_clicked(1, 999.0);
    pump(&ui, &app);
    {
        let selection = app.borrow().selection();
        assert_eq!(selection.folder, 1, "the child folder is now selected");
        assert_eq!(selection.focus, Pane::Folders);
    }
    assert_eq!(ui.get_folder_selected(), 1, "the Folders pane agrees");
    assert_eq!(
        ui.get_focus_pane(),
        0,
        "the Contents and File panes' focus highlight agrees too"
    );

    // Back to the root, whose two rows the steps below act on.
    ui.invoke_folder_row_clicked(0, 999.0);
    pump(&ui, &app);

    // The Contents pane: a click and a shift-click build a two-row
    // selection, which the pane's own rows and `App::selection()` have to
    // agree on.
    let sub = content_row_index(&ui, "sub/");
    let demo = content_row_index(&ui, "demo.rs");
    ui.invoke_content_row_clicked(sub);
    pump(&ui, &app);
    ui.invoke_content_row_shift_clicked(demo);
    pump(&ui, &app);
    {
        let selection = app.borrow().selection();
        assert_eq!(
            selection.contents,
            [
                usize::try_from(sub).unwrap(),
                usize::try_from(demo).unwrap()
            ]
            .into_iter()
            .collect(),
            "both rows between the click and the shift-click are selected"
        );
        assert_eq!(selection.content, usize::try_from(demo).unwrap());
        assert_eq!(selection.focus, Pane::Contents);
    }
    assert_eq!(ui.get_content_selected(), demo, "the Contents pane agrees");
    assert_eq!(
        ui.get_focus_pane(),
        1,
        "and the focus highlight moved to it"
    );
    let rows = ui.get_content_rows();
    assert!(
        rows.row_data(usize::try_from(sub).unwrap())
            .expect("a drawn row")
            .selected,
        "the pane draws the row clicked first as selected"
    );
    assert!(
        rows.row_data(usize::try_from(demo).unwrap())
            .expect("a drawn row")
            .selected,
        "and the row shift-clicked too"
    );

    // The File pane: demo.rs's real preview offers more than one view,
    // which the strip and `App::selection()` have to agree is showing.
    assert!(
        app.borrow().file_views().len() > 1,
        "a Rust file with a real definition in it offers Preview and Text"
    );
    ui.invoke_file_view_selected(1);
    {
        let selection = app.borrow().selection();
        assert_eq!(selection.file_view_index, 1, "the Text view is now showing");
        // Switching views is not a Contents selection change: nothing
        // above should have moved.
        assert_eq!(
            selection.contents,
            [
                usize::try_from(sub).unwrap(),
                usize::try_from(demo).unwrap()
            ]
            .into_iter()
            .collect()
        );
        assert_eq!(selection.focus, Pane::Contents);
    }
    assert_eq!(ui.get_file_view_index(), 1, "the File pane agrees");
}
