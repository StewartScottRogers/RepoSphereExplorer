//! The File pane's width floor (#577), and the splitters that decide it,
//! driven through a real window on a real [`App`].
//!
//! `gui::lib`'s own unit tests prove `fit_pane_widths` does the arithmetic
//! right; nothing there proves the window actually applies it, or that the
//! splitter it names is the one a reader's double-click lands on. This file
//! is that seam - CLAUDE.md rule 14.

use gui::app::App;
use gui::{MainWindow, fit_pane_widths_to_window, sync_ui};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

mod common;
use common::ensure_service;

/// An empty directory of this test's own under the platform's temporary
/// directory.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("rse-pane-widths").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// A shown window on a real `App` rooted at `dir`, wired as `main` wires it.
fn window_on(dir: &Path) -> (MainWindow, Rc<RefCell<App>>) {
    ensure_service();
    let app = Rc::new(RefCell::new(App::new(dir.to_path_buf())));
    let ui = MainWindow::new().expect("the window should build");
    sync_ui(&ui, &app.borrow());
    gui::wire_callbacks(&ui, &app);
    ui.show().expect("the window should show");
    (ui, app)
}

/// Ticks the application until `done`, the way the window's timer does.
fn pump(ui: &MainWindow, app: &Rc<RefCell<App>>, done: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        {
            let mut app = app.borrow_mut();
            app.tick();
            sync_ui(ui, &app);
            if done(&app) {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the service never produced the listing"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The File pane, as it is actually drawn.
fn file_pane(ui: &MainWindow) -> ElementHandle {
    ElementHandle::find_by_element_id(ui, "MainWindow::file-pane")
        .next()
        .expect("the window has a File pane")
}

/// The splitter to the right of the Folders pane.
fn folders_splitter(ui: &MainWindow) -> ElementHandle {
    ElementHandle::find_by_element_id(ui, "MainWindow::folders-splitter")
        .next()
        .expect("the window draws the Folders splitter")
}

/// Double-clicks the middle of `target`, the way a pointer does.
fn double_click(ui: &MainWindow, target: &ElementHandle) {
    let at = target.absolute_position();
    let size = target.size();
    let position = LogicalPosition::new(at.x + size.width / 2.0, at.y + size.height / 2.0);
    let window = ui.window();
    window.dispatch_event(WindowEvent::PointerMoved { position });
    for _ in 0..2 {
        window.dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        window.dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
    }
}

/// Widths restored from `gui.json` at startup - or left over from a window
/// that has since been made narrower - are corrected against the window
/// that is actually open, not trusted as they are.
#[test]
fn restored_widths_too_wide_for_the_window_are_corrected() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = scratch("restore");
    let (ui, _app) = window_on(&dir);
    let window_width = ui
        .window()
        .size()
        .to_logical(ui.window().scale_factor())
        .width;
    assert!(
        (window_width - 1000.0).abs() < 20.0,
        "this check is against `MainWindow`'s own 1000px `preferred-width`; \
         it opened at {window_width}"
    );
    // The pair the work order's own acceptance check names: on their own
    // they leave the File pane exactly its 280px minimum, and a window
    // restored a pixel narrower than that would break it.
    ui.set_folders_width(350.0);
    ui.set_contents_width(470.0);

    fit_pane_widths_to_window(&ui);

    let pane = file_pane(&ui);
    assert!(
        pane.size().width >= 280.0,
        "the File pane is {}px wide, below its 280px minimum",
        pane.size().width
    );
}

/// Double-clicking the splitter to the right of Folders fits the pane to
/// its widest row, within its limits.
#[test]
fn double_clicking_the_folders_splitter_fits_the_pane_to_its_widest_row() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = scratch("double-click");
    std::fs::create_dir_all(dir.join("a-very-long-repository-folder-name"))
        .expect("a scratch folder");
    let (ui, app) = window_on(&dir);
    pump(&ui, &app, |app| app.folder_rows().len() > 1);
    // Room for Folders to grow into - the way a reader chasing a long name
    // would first narrow Contents themselves.
    ui.set_contents_width(220.0);
    let before = ui.get_folders_width();

    double_click(&ui, &folders_splitter(&ui));

    let after = ui.get_folders_width();
    assert!(
        after > before,
        "double-clicking the splitter should widen a pane too narrow for \
         its longest name; it stayed at {before}"
    );
    let window_width = ui
        .window()
        .size()
        .to_logical(ui.window().scale_factor())
        .width;
    assert!(
        after <= window_width - 220.0 - 280.0 - 10.0 + 0.5,
        "the fit should stop within Folders' own limit, leaving Contents \
         and the File pane their minimums; got {after}"
    );
}
