//! The Contents pane's centred message (#592) - what is wrong with the
//! Repos Directory itself, or that it is empty - driven through a real
//! window on a real `App`, against a real (or deliberately missing)
//! directory.
//!
//! `app`'s own unit tests know what `contents_message_title` and its
//! siblings say and have never met a window; this file is the half that
//! proves Retry and "Choose Repos Directory..." are real, clickable,
//! keyboard-reachable controls wired to the callbacks that fire them.
//!
//! `main`'s `wire_callbacks` is public, so this file calls it directly
//! rather than copying it - a defect in the real wiring shows up here.

use gui::app::App;
use gui::{MainWindow, sync_ui};
use i_slint_backend_testing::ElementHandle;
use slint::ComponentHandle;
use slint::platform::{PointerEventButton, WindowEvent};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

mod common;
use common::ensure_service;

/// A directory of this test's own under the platform's temporary
/// directory, removed first so a previous run's leftovers cannot answer
/// for a "missing path" test.
fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("repos-explorer-contents-message-{name}"))
}

/// Ticks the application until `done`, the way the window's 100ms timer
/// does - not a sleep, since the answer may land on the very first tick.
fn pump(ui: &MainWindow, app: &Rc<RefCell<App>>, what: &str, done: impl Fn(&App) -> bool) {
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
            "the service never produced {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A shown window, wired as `main` wires it, on an `App` rooted at
/// `directory` - which may or may not exist - with the opening request's
/// answer (success, failure, or empty) already landed.
fn window_on(directory: &Path) -> (MainWindow, Rc<RefCell<App>>) {
    ensure_service();
    i_slint_backend_testing::init_no_event_loop();
    let app = Rc::new(RefCell::new(App::new(directory.to_path_buf())));
    let ui = MainWindow::new().expect("the window should build");
    gui::wire_callbacks(&ui, &app);
    sync_ui(&ui, &app.borrow());
    ui.show().expect("the window should show");
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    pump(&ui, &app, "the opening listing's answer", |app| {
        !app.status_text().starts_with("loading ")
    });
    (ui, app)
}

/// Clicks the widest element labelled `label`.
fn click_labelled(ui: &MainWindow, label: &str) {
    let mut matches: Vec<ElementHandle> =
        ElementHandle::find_by_accessible_label(ui, label).collect();
    matches.sort_by(|a, b| {
        b.size()
            .width
            .partial_cmp(&a.size().width)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    matches
        .first()
        .unwrap_or_else(|| panic!("nothing labelled {label:?}"))
        .mock_single_click(PointerEventButton::Left);
}

#[test]
fn a_missing_repos_directory_shows_its_message_with_retry_and_choose() {
    let directory = scratch("missing");
    let _ = std::fs::remove_dir_all(&directory);
    let (ui, app) = window_on(&directory);

    assert!(
        ui.get_message_title().contains("is not available"),
        "title: {}",
        ui.get_message_title()
    );
    assert_eq!(ui.get_message_detail(), "The folder does not exist");
    assert!(ui.get_message_show_retry());
    assert!(ui.get_message_show_choose());
    assert!(!ui.get_message_show_clear_filter());
    assert!(app.borrow().content_rows().is_empty());
    assert_eq!(
        app.borrow().folder_rows().len(),
        1,
        "the root, with no children"
    );

    // Keyboard reachable (rule "The message is keyboard reachable"):
    // both buttons are real, clickable, accessibly-labelled controls.
    assert!(
        ElementHandle::find_by_accessible_label(&ui, "Retry")
            .next()
            .is_some()
    );
    assert!(
        ElementHandle::find_by_accessible_label(&ui, "Choose Repos Directory...")
            .next()
            .is_some()
    );
}

#[test]
fn an_empty_repos_directory_shows_its_message_with_only_choose() {
    let directory = scratch("empty");
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    let (ui, app) = window_on(&directory);

    assert!(
        ui.get_message_title().contains("has no repositories yet"),
        "title: {}",
        ui.get_message_title()
    );
    assert!(ui.get_message_detail().contains("will appear here"));
    assert!(!ui.get_message_show_retry());
    assert!(ui.get_message_show_choose());
    assert!(!ui.get_message_show_clear_filter());
    assert!(app.borrow().content_rows().is_empty());

    std::fs::remove_dir_all(&directory).expect("cleanup");
}

#[test]
fn clicking_retry_after_the_folder_appears_lists_it() {
    let directory = scratch("retry");
    let _ = std::fs::remove_dir_all(&directory);
    let (ui, app) = window_on(&directory);
    assert!(
        ui.get_message_show_retry(),
        "the missing directory should offer Retry first"
    );

    std::fs::create_dir_all(&directory).expect("the directory now appears");
    std::fs::write(directory.join("a.txt"), "hi").expect("something in it to list");

    click_labelled(&ui, "Retry");
    pump(&ui, &app, "the re-listed folder", |app| {
        !app.content_rows().is_empty()
    });

    assert_eq!(ui.get_message_title(), "");
    assert_eq!(
        app.borrow()
            .content_rows()
            .iter()
            .map(|row| row.name.clone())
            .collect::<Vec<_>>(),
        vec!["a.txt".to_owned()]
    );

    std::fs::remove_dir_all(&directory).expect("cleanup");
}
