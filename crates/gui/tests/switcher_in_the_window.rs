//! The Go to Repository switcher (#590), driven end to end through a real
//! window on a real [`App`], against real folders in a scratch directory.
//!
//! `switcher.rs`'s own unit tests know the matcher scores a word-start run
//! above a contiguous one and a contiguous one above a scattered one, and
//! `app`'s own unit tests know what opening the switcher, typing into it
//! and confirming a match do to `App`'s state - neither has ever met a
//! window. This is the seam CLAUDE.md rule 14 asks for: Ctrl+P dispatched
//! as a real key chord, through the same `wire_callbacks` `main` uses,
//! landing on the repository a reader typed towards.

use gui::app::App;
use gui::{MainWindow, sync_ui};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model as _};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

mod common;
use common::ensure_service;

/// A folder the directory plugin reads as a working copy: `HEAD` is all it
/// looks at for a branch. No `git` runs (rule 8), and nothing here is a
/// real repository anybody works in.
fn checkout(dir: &Path, name: &str) {
    let git = dir.join(name).join(".git");
    std::fs::create_dir_all(&git).expect("a checkout's git directory");
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");
}

/// A Repos Directory of this test's own: three checkouts (one a
/// word-start match for "tsc", one a scattered match, one no match at
/// all), a plain folder, and a file, so the switcher's "working copies
/// only" rule (#590's acceptance check 2) has something to exclude.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("repos-explorer-switcher-{name}"));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    checkout(&directory, "TankSwarmCode");
    checkout(&directory, "other-repo");
    checkout(&directory, "unrelated");
    std::fs::create_dir_all(directory.join("plain-folder")).expect("a non-repository folder");
    std::fs::write(directory.join("notes.txt"), "hello\n").expect("a file");
    directory
}

/// Ticks the application until `done`, the way the window's timer does.
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

/// A shown window, wired to an application listing `directory`, with the
/// root's own listing already in - the wiring is `gui::wire_callbacks`,
/// the same `main` calls, per rule 14.
fn window_on(directory: &Path) -> (MainWindow, Rc<RefCell<App>>) {
    ensure_service();
    let app = Rc::new(RefCell::new(App::new(directory.to_path_buf())));
    let ui = MainWindow::new().expect("the window should build");
    gui::wire_callbacks(&ui, &app);
    sync_ui(&ui, &app.borrow());
    ui.show().expect("the window should show");
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    pump(&ui, &app, "a listing of the root", |app| {
        !app.content_rows().is_empty()
    });
    (ui, app)
}

/// A typed character, as a keyboard sends one.
fn press_text(ui: &MainWindow, text: &str) {
    let text = slint::SharedString::from(text);
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

/// Presses and releases a named key with no printable text of its own.
fn press_key(ui: &MainWindow, key: Key) {
    let text = slint::SharedString::from(char::from(key).to_string());
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

/// Ctrl+P, dispatched as the chord a keyboard actually reports: the
/// modifier down, the letter, then both released in reverse.
fn press_ctrl_p(ui: &MainWindow) {
    let window = ui.window();
    let control = slint::SharedString::from(char::from(Key::Control).to_string());
    window.dispatch_event(WindowEvent::KeyPressed {
        text: control.clone(),
    });
    window.dispatch_event(WindowEvent::KeyPressed { text: "p".into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: "p".into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: control });
}

#[test]
fn ctrl_p_finds_and_goes_to_a_repository_by_a_fuzzy_typed_name() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = scratch("goes-to-a-repository");
    let (ui, app) = window_on(&directory);

    assert!(!ui.get_switcher_open(), "closed until Ctrl+P");
    press_ctrl_p(&ui);
    assert!(ui.get_switcher_open(), "Ctrl+P opens it");

    // Every result is a working copy: the plain folder and the file are
    // both left out (#590's acceptance check 2).
    let opening_names: Vec<String> = ui
        .get_switcher_rows()
        .iter()
        .map(|row| row.name.to_string())
        .collect();
    assert!(!opening_names.contains(&"plain-folder".to_owned()));
    assert!(!opening_names.contains(&"notes.txt".to_owned()));
    assert_eq!(opening_names.len(), 3, "{opening_names:?}");

    press_text(&ui, "t");
    press_text(&ui, "s");
    press_text(&ui, "c");

    let rows = ui.get_switcher_rows();
    let first = rows.row_data(0).expect("a first match");
    assert_eq!(
        first.name, "TankSwarmCode",
        "a word-start run outranks the other two checkouts"
    );
    assert!(first.selected, "the first match is highlighted");

    press_key(&ui, Key::Return);

    assert!(!ui.get_switcher_open(), "Return closes it");
    pump(&ui, &app, "the reselected listing", |app| {
        app.content_rows()
            .get(app.content_selected())
            .is_some_and(|row| row.name.trim_end_matches('/') == "TankSwarmCode")
    });
    let app = app.borrow();
    assert_eq!(
        app.folder_selected(),
        0,
        "its parent, the Repos Directory, is selected in Folders"
    );
}

#[test]
fn escape_closes_the_switcher_and_types_no_key_text_into_the_listing() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = scratch("escape-closes-it");
    let (ui, _app) = window_on(&directory);

    press_ctrl_p(&ui);
    assert!(ui.get_switcher_open());
    press_text(&ui, "t");

    press_key(&ui, Key::Escape);

    assert!(!ui.get_switcher_open(), "Escape closes it");
    assert_eq!(
        ui.get_switcher_query(),
        "",
        "closing drops the typed query rather than leaving it for next time"
    );
}
