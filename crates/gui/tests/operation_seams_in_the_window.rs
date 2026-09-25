//! An operation, and everything it touches (#726).
//!
//! The graphical front end has forty-odd test files and, until #725's
//! flagship journey, not one of them followed a reader's whole journey:
//! every other suite was one interaction or one hand-off from a fresh
//! scratch directory and a fresh window. That leaves the seams between
//! features uncovered, which CLAUDE.md rule 14 says is where the faults
//! live. This file crosses three of the seams the #726 inventory names,
//! each a use-case test of its own, plus a fourth added alongside them
//! for the same reason:
//!
//! - An operation and the Folders tree: `operations_in_the_window.rs`
//!   asserts disk and Contents rows; `folders_in_the_window.rs` never
//!   runs an operation. Nothing creates or deletes a folder and then
//!   looks at the tree.
//! - An operation and the File pane, or an open editor: renaming or
//!   deleting the file currently previewed - or open in the editor - was
//!   untested outside a pinned pop-out.
//! - Undo and the interface after it: the undo tests in
//!   `operations_in_the_window.rs` check disk and status text, never
//!   where the selection, the scroll offset or the File pane land.
//!
//! Full stack throughout: a real Repos Directory of real files on disk, a
//! real service on the private socket `common::ensure_service` provides,
//! and a real `MainWindow` joined to a real `App` by `gui::wire_callbacks`,
//! the function `main` calls, never a copy of it. Every step is a
//! dispatched pointer or keyboard event or a markup callback, the way a
//! reader reaches the application; nothing is measured that was not
//! drawn.

use gui::app::App;
use gui::{MainWindow, sync_ui};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model as _};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

mod common;
use common::ensure_service;

/// Row height in `app.slint`'s contents pane, so a click can be aimed at a
/// row and a scroll offset can be turned into a row index.
const ROW_HEIGHT: f32 = 20.0;

/// The Slint testing backend is process-wide, and these tests share one
/// service and with it one undo step, so they run one at a time - the
/// guard `operations_in_the_window.rs` takes.
static SERIAL: Mutex<()> = Mutex::new(());

/// Takes the shared lock, tolerating a previous test having panicked while
/// holding it - a poisoned lock would otherwise turn one failure into many.
fn serially() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// An empty directory of this test's own under the platform's temporary
/// directory. Nothing in these tests ever touches a path outside it.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("rse-operation-seams").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// Writes a file into `dir`.
fn file(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).expect("a scratch file");
}

// ---------------------------------------------------------------------
// The harness, in the shape `operations_in_the_window.rs` uses.
// ---------------------------------------------------------------------

/// A shown window on a real `App` rooted at `root`, wired by the crate's
/// own wiring - the same call `main` makes - with the opening listing
/// already settled.
fn window_at(root: &Path) -> (MainWindow, Rc<RefCell<App>>) {
    ensure_service();
    i_slint_backend_testing::init_no_event_loop();
    let app = Rc::new(RefCell::new(App::new(root.to_path_buf())));
    let ui = MainWindow::new().expect("the window should build");
    sync_ui(&ui, &app.borrow());
    gui::wire_callbacks(&ui, &app);
    ui.show().expect("the window should show");
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    settle(&ui, &app);
    (ui, app)
}

/// Whether `status` is one of the transient lines an in-flight request puts
/// up, which is how [`settle`] knows the application is still working.
fn still_working(status: &str) -> bool {
    status.starts_with("loading ")
        || matches!(
            status,
            "working..." | "deleting..." | "undoing..." | "saving..."
        )
}

/// Ticks the application the way the window's 100ms timer does, until every
/// in-flight request has landed and stayed landed. Not a sleep: the same
/// `tick` and `sync_ui` pair `main` runs, just as fast as the results
/// arrive.
fn settle(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut quiet = 0u32;
    while Instant::now() < deadline {
        {
            let mut app = app.borrow_mut();
            app.tick();
            sync_ui(ui, &app);
        }
        if still_working(&ui.get_status_text()) || app.borrow().is_busy() {
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

/// Presses `text` as a key with `modifiers` held. Slint tracks modifier
/// state from the modifier key's own press, so holding one means pressing
/// and releasing it around the key itself.
fn press_with(ui: &MainWindow, text: &str, modifiers: &[Key]) {
    let window = ui.window();
    for modifier in modifiers {
        window.dispatch_event(WindowEvent::KeyPressed {
            text: char::from(*modifier).into(),
        });
    }
    window.dispatch_event(WindowEvent::KeyPressed { text: text.into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: text.into() });
    for modifier in modifiers.iter().rev() {
        window.dispatch_event(WindowEvent::KeyReleased {
            text: char::from(*modifier).into(),
        });
    }
}

/// Presses `text` as a key with nothing held.
fn press(ui: &MainWindow, text: &str) {
    press_with(ui, text, &[]);
}

/// Presses a named key such as `Key::F2` with nothing held.
fn press_key(ui: &MainWindow, key: Key) {
    press(ui, &char::from(key).to_string());
}

/// Types `text` one character at a time, the way a keyboard delivers it.
fn type_text(ui: &MainWindow, text: &str) {
    for c in text.chars() {
        press(ui, &c.to_string());
    }
}

/// Clears a pre-filled prompt by backspacing over every character of it.
fn clear_prompt(ui: &MainWindow, characters: usize) {
    for _ in 0..characters {
        press_key(ui, Key::Backspace);
    }
}

/// Clicks the contents pane `rows_down` rows below its first row.
fn click_row(ui: &MainWindow, rows_down: f32) {
    let origin =
        i_slint_backend_testing::ElementHandle::find_by_element_id(ui, "ContentsPane::click-area")
            .next()
            .expect("the contents pane has a click area")
            .absolute_position();
    let position = LogicalPosition::new(
        origin.x + 20.0,
        origin.y + rows_down.mul_add(ROW_HEIGHT, ROW_HEIGHT / 2.0),
    );
    let window = ui.window();
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

/// The names the contents pane is drawing, in the order it draws them. A
/// directory carries a trailing separator, which is stripped here so a test
/// can name a folder the way the filesystem does.
fn listing(ui: &MainWindow) -> Vec<String> {
    ui.get_content_rows()
        .iter()
        .map(|row| row.name.trim_end_matches('/').to_owned())
        .collect()
}

/// The names the contents pane is drawing as selected.
fn selected(ui: &MainWindow) -> Vec<String> {
    ui.get_content_rows()
        .iter()
        .filter(|row| row.selected)
        .map(|row| row.name.trim_end_matches('/').to_owned())
        .collect()
}

/// The row index of `name` in the drawn listing, as a `u16` both `row_of`
/// (for pixel arithmetic) and [`content_row_index`] (for the row's own
/// callback) convert from, rather than each rounding a wider integer
/// through a cast of their own.
fn content_row(ui: &MainWindow, name: &str) -> u16 {
    let index = listing(ui)
        .iter()
        .position(|drawn| drawn == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing: {:?}", listing(ui)));
    u16::try_from(index).expect("a small listing")
}

/// The row index of `name` in the drawn listing, as pixels-per-row
/// arithmetic wants it.
fn row_of(ui: &MainWindow, name: &str) -> f32 {
    f32::from(content_row(ui, name))
}

/// The row index of `name` in the drawn listing, as
/// `content-row-clicked`'s own callback wants it - for a row past the
/// bottom of an unscrolled window, which a pixel click cannot reach.
fn content_row_index(ui: &MainWindow, name: &str) -> i32 {
    i32::from(content_row(ui, name))
}

/// What the File pane has on screen, whichever of its two text surfaces is
/// drawing it: the coloured one when the plugin described a language, and
/// the plain one otherwise. A reader cannot tell them apart, so neither
/// does this test.
fn shown(ui: &MainWindow) -> String {
    let coloured = ui
        .get_file_lines()
        .iter()
        .map(|line| {
            line.iter()
                .map(|run| run.text.to_string())
                .collect::<Vec<_>>()
                .concat()
        })
        .collect::<Vec<_>>()
        .join("\n");
    if coloured.is_empty() {
        ui.get_file_text().to_string()
    } else {
        coloured
    }
}

/// The tab labels the File pane is drawing.
fn tabs(ui: &MainWindow) -> Vec<String> {
    ui.get_file_tabs()
        .iter()
        .map(|label| label.to_string())
        .collect()
}

/// Clicks the middle of tab `index`, as it is actually drawn - measured
/// rather than assumed, the way `reading_and_editing_journey_in_the_window.rs`
/// does.
fn click_tab(ui: &MainWindow, index: usize) {
    let strip =
        i_slint_backend_testing::ElementHandle::find_by_element_id(ui, "FilePane::tab-strip")
            .next()
            .expect("a strip should be drawn");
    let count = u16::try_from(tabs(ui).len()).expect("a handful of tabs");
    let width = strip.size().width / f32::from(count);
    let index = u16::try_from(index).expect("a handful of tabs");
    let origin = strip.absolute_position();
    let position = LogicalPosition::new(
        origin.x + f32::from(index).mul_add(width, width / 2.0),
        origin.y + 12.0,
    );
    let window = ui.window();
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

/// The names the Folders tree is currently drawing, at whatever depth,
/// through the application rather than the pane's own visual rows - this
/// file is asking whether an operation reaches the tree's data, not
/// re-measuring the pane's own hit testing, which `folders_in_the_window.rs`
/// already does.
fn tree_names(app: &Rc<RefCell<App>>) -> Vec<String> {
    app.borrow()
        .folder_rows()
        .iter()
        .map(|row| row.name.clone())
        .collect()
}

// ---------------------------------------------------------------------
// 1. A new folder reaches the tree.
// ---------------------------------------------------------------------

/// From the Repos Directory: make a folder through the application, and it
/// appears in the Folders tree as well as the listing. Delete it, and it
/// leaves both.
#[test]
fn a_new_folder_reaches_the_tree_and_leaving_it_leaves_both() {
    let _serial = serially();
    let dir = scratch("new-folder-reaches-tree");
    file(&dir, "keep.txt", "k");
    let (ui, app) = window_at(&dir);
    assert!(
        !tree_names(&app).contains(&"docs".to_owned()),
        "docs should not exist yet"
    );

    // Ctrl+Shift+N makes the folder and drops straight into rename
    // (`operations_in_the_window.rs`'s own `ctrl_shift_n_creates_a_folder`).
    press_with(&ui, "N", &[Key::Control, Key::Shift]);
    settle(&ui, &app);
    clear_prompt(&ui, "New folder".len());
    type_text(&ui, "docs");
    press_key(&ui, Key::Return);
    settle(&ui, &app);

    assert!(dir.join("docs").is_dir(), "the folder is on disk");
    assert!(
        listing(&ui).contains(&"docs".to_owned()),
        "and in the listing: {:?}",
        listing(&ui)
    );
    assert!(
        tree_names(&app).contains(&"docs".to_owned()),
        "and in the Folders tree, which reads the same node the listing \
         just reloaded: {:?}",
        tree_names(&app)
    );

    // Delete it, and both halves let go of it together.
    click_row(&ui, row_of(&ui, "docs"));
    press_key(&ui, Key::Delete);
    press(&ui, "y");
    settle(&ui, &app);

    assert!(!dir.join("docs").exists(), "the folder is gone from disk");
    assert!(
        !listing(&ui).contains(&"docs".to_owned()),
        "and from the listing: {:?}",
        listing(&ui)
    );
    assert!(
        !tree_names(&app).contains(&"docs".to_owned()),
        "and from the Folders tree: {:?}",
        tree_names(&app)
    );
}

// ---------------------------------------------------------------------
// 2. Renaming the file being previewed.
// ---------------------------------------------------------------------

/// Select a file so the File pane shows it, rename it through the
/// application, and the pane follows it under its new name rather than
/// showing a file that is gone.
#[test]
fn renaming_the_previewed_file_keeps_the_pane_on_it() {
    let _serial = serially();
    let dir = scratch("rename-previewed");
    // `aardvark.txt` sorts ahead of both the old and the new name, so a
    // reload that dropped the selection to row 0 rather than following the
    // rename would land here instead - and this test would not notice the
    // difference with only one file in the folder.
    file(&dir, "aardvark.txt", "aardvark\n");
    file(&dir, "notes.txt", "hello there\n");
    let (ui, app) = window_at(&dir);

    click_row(&ui, row_of(&ui, "notes.txt"));
    settle(&ui, &app);
    assert!(
        shown(&ui).contains("hello there"),
        "the pane should be showing notes.txt before the rename: {:?}",
        shown(&ui)
    );

    press_key(&ui, Key::F2);
    clear_prompt(&ui, "notes.txt".len());
    type_text(&ui, "memo.txt");
    press_key(&ui, Key::Return);
    settle(&ui, &app);

    assert!(dir.join("memo.txt").is_file(), "the file was renamed");
    assert!(!dir.join("notes.txt").exists(), "the old name is gone");
    assert_eq!(
        selected(&ui),
        vec!["memo.txt".to_owned()],
        "the renamed file stays selected"
    );
    assert!(
        shown(&ui).contains("hello there"),
        "the File pane should have followed the rename rather than showing \
         a file that no longer exists: {:?}",
        shown(&ui)
    );
}

// ---------------------------------------------------------------------
// 3. Deleting the file open in the editor.
// ---------------------------------------------------------------------

/// With unsaved changes, the application refuses to lose the edit, the way
/// #619 settles it for a pinned window; with none, the editor closes and
/// says why. Reached the way a reader actually reaches it: the Delete key
/// is swallowed by the editor's own keyboard (`editor_keyboard.rs`'s
/// `the_contents_panes_shortcuts_stay_out_of_the_editors_way`), so this
/// drives the row menu's mouse-only route (`content-delete-requested`)
/// instead, which needs no keyboard the editor could ever hold.
#[test]
fn deleting_the_file_open_in_the_editor_refuses_or_closes_it() {
    let _serial = serially();
    let dir = scratch("delete-open-in-editor");
    file(&dir, "demo.rs", "fn main() {}\n");
    let (ui, app) = window_at(&dir);

    click_row(&ui, row_of(&ui, "demo.rs"));
    settle(&ui, &app);
    click_tab(&ui, tabs(&ui).len() - 1);
    assert!(ui.get_editing_file(), "the last tab should open the editor");
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: "X".into() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text: "X".into() });
    assert!(
        ui.get_edit_text().contains('X'),
        "the keystroke should have reached the editor: {:?}",
        ui.get_edit_text()
    );

    // Unsaved: refused, synchronously, before anything asks the service.
    ui.invoke_content_delete_requested();
    assert!(
        ui.get_status_text().contains("unsaved changes"),
        "the status bar should refuse and say why; it read {:?}",
        ui.get_status_text()
    );
    assert!(
        ui.get_editing_file(),
        "the editor must still be open: nothing may be lost"
    );
    assert!(
        dir.join("demo.rs").is_file(),
        "and the file must still exist"
    );

    // Discard the edit for real, the way a reader would (Escape reaches the
    // window, per `editor_keyboard.rs`), and reopen the editor clean.
    press_key(&ui, Key::Escape);
    assert!(!ui.get_editing_file(), "Escape discarded the edit");
    click_tab(&ui, tabs(&ui).len() - 1);
    assert!(ui.get_editing_file(), "the editor is open again");
    assert!(
        !ui.get_edit_modified(),
        "a fresh reopen carries no unsaved changes"
    );

    // Unmodified: carried out, and the editor closes because of it.
    ui.invoke_content_delete_requested();
    assert!(
        ui.get_status_text().contains("was deleted"),
        "the status bar should say why the editor closed; it read {:?}",
        ui.get_status_text()
    );
    assert!(
        !ui.get_editing_file(),
        "the editor should have closed along with the file it was open on"
    );
    settle(&ui, &app);

    assert!(!dir.join("demo.rs").exists(), "the file was deleted");
    assert!(
        !listing(&ui).contains(&"demo.rs".to_owned()),
        "and dropped from the listing: {:?}",
        listing(&ui)
    );
}

// ---------------------------------------------------------------------
// 4. Undo puts the interface back, not only the disk.
// ---------------------------------------------------------------------

/// After undoing a paste, the restored listing is accurate, the selection
/// is on something that exists, the pane is scrolled back to show it, and
/// the File pane shows what is now selected rather than something stale.
#[test]
fn undo_of_a_paste_puts_the_interface_back_too() {
    let _serial = serially();
    let dir = scratch("undo-puts-the-interface-back");
    // Sixty files sort ahead of the pasted copy below, the way
    // `contents_in_the_window.rs` uses sixty to guarantee the pane has to
    // scroll - one file would let everything fit and prove nothing.
    for index in 0..60 {
        file(
            &dir,
            &format!("aa-{index:02}.txt"),
            &format!("content of aa-{index:02}.txt"),
        );
    }
    file(&dir, "note.txt", "content of note.txt");
    let (ui, app) = window_at(&dir);

    // `note.txt` sorts after every `aa-*` file, at a row past the bottom
    // of the window - `click_row` assumes an unscrolled pane and would
    // click outside it, so the selection is made through the row's own
    // callback instead (`operations_in_the_window.rs`'s batch-undo test
    // does the same for the row past what a click can reach unscrolled).
    ui.invoke_content_row_clicked(content_row_index(&ui, "note.txt"));
    press_with(&ui, "c", &[Key::Control]);
    press_with(&ui, "v", &[Key::Control]);
    settle(&ui, &app);

    assert!(
        dir.join("note (2).txt").is_file(),
        "the paste landed on disk"
    );
    assert_eq!(
        selected(&ui),
        vec!["note (2).txt".to_owned()],
        "and is selected, sorting after every aa-* file - so bringing it \
         on screen has to scroll"
    );
    assert!(
        ui.get_content_scroll_y() < -ROW_HEIGHT,
        "the pane should have scrolled down to it; it is at {}",
        ui.get_content_scroll_y()
    );

    press_with(&ui, "z", &[Key::Control]);
    settle(&ui, &app);

    assert!(
        !dir.join("note (2).txt").exists(),
        "undo should have taken the pasted copy back out"
    );
    assert!(
        !listing(&ui).contains(&"note (2).txt".to_owned()),
        "and dropped it from the listing: {:?}",
        listing(&ui)
    );

    let after = selected(&ui);
    assert_eq!(
        after.len(),
        1,
        "undo should leave exactly one row selected, not none and not the \
         row the pasted copy used to occupy: {after:?}"
    );
    assert!(
        listing(&ui).contains(&after[0]),
        "the selection has to be on a row that still exists: {after:?} vs \
         {:?}",
        listing(&ui)
    );

    let index = ui
        .get_content_rows()
        .iter()
        .position(|row| row.selected)
        .expect("a selected row is drawn");
    let top = f32::from(u16::try_from(index).expect("a small listing")) * ROW_HEIGHT;
    let scroll = ui.get_content_scroll_y();
    let viewport = ui.get_content_viewport_height();
    assert!(
        top + scroll >= 0.0 && top + scroll < viewport,
        "the selected row (index {index}) should be scrolled into view; \
         scroll is {scroll}, viewport is {viewport}"
    );

    assert!(
        shown(&ui).contains(&format!("content of {}", after[0])),
        "the File pane should show what is now selected, not something \
         stale left over from before the undo: {:?}",
        shown(&ui)
    );
}
