//! The file operations, driven end to end through a real window on a real
//! [`App`], against real files in a scratch directory.
//!
//! Every other suite in this crate tests one half alone: `commands.rs`
//! builds the window with no `App` behind it and asks only whether a
//! control reaches its callback, and the `app` module's own unit tests
//! drive `App` with no window in front of it. Neither half can see a
//! defect that lives between them - a prompt drawn over the wrong row, a
//! key that reaches an operation the prompt on screen was not offering.
//! That seam is what this file measures.
//!
//! Each test dispatches real `slint::platform::WindowEvent`s (or real
//! pointer events through Slint's own hit testing) at a shown window, and
//! then looks at the files on disk and at the properties the window is
//! drawing from.
//!
//! The wiring below mirrors `main.rs`'s `wire_callbacks`, which is private
//! to the binary and so cannot be called from an integration test. Only
//! the callbacks these tests exercise are wired; a defect in `main.rs`'s
//! own wiring would therefore be invisible here, and `commands.rs` does
//! not cover it either.

use gui::app::App;
use gui::{MainWindow, sync_ui};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard, Once};
use std::time::{Duration, Instant};

/// Row height in `app.slint`'s contents pane, so a click can be aimed at a
/// row.
const ROW_HEIGHT: f32 = 20.0;

/// Tests in this file share one service, and with it one undo step, so they
/// run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

/// Takes the shared lock, tolerating a previous test having panicked while
/// holding it - a poisoned lock would otherwise turn one failure into many.
fn serially() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Makes sure something is answering on the service's local socket. A
/// service the developer already has running answers; otherwise one is run
/// on a background thread of this test process.
fn ensure_service() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let name = protocol::socket_name().expect("the platform has a socket name");
        if let Ok(listener) = service::bind(name) {
            std::thread::spawn(move || {
                let _ = service::run(&listener);
            });
        }
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        use interprocess::local_socket::traits::Stream as _;
        let name = protocol::socket_name().expect("the platform has a socket name");
        if interprocess::local_socket::Stream::connect(name).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("no service answered on the local socket");
}

/// An empty directory of this test's own under the platform's temporary
/// directory. Nothing in these tests ever touches a path outside it.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("rse-operations").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// Writes a file into `dir`.
fn file(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).expect("a scratch file");
}

// ---------------------------------------------------------------------
// Wiring, mirroring `main.rs`.
// ---------------------------------------------------------------------

macro_rules! on_plain {
    ($ui:expr, $app:expr, $setter:ident, $method:ident) => {{
        let app = Rc::clone($app);
        let weak = $ui.as_weak();
        $ui.$setter(move || {
            let mut app = app.borrow_mut();
            app.$method();
            if let Some(ui) = weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }};
}

macro_rules! on_index {
    ($ui:expr, $app:expr, $setter:ident, $method:ident) => {{
        let app = Rc::clone($app);
        let weak = $ui.as_weak();
        $ui.$setter(move |i| {
            let mut app = app.borrow_mut();
            app.$method(usize::try_from(i).unwrap_or(usize::MAX));
            if let Some(ui) = weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }};
}

macro_rules! on_delta {
    ($ui:expr, $app:expr, $setter:ident, $method:ident) => {{
        let app = Rc::clone($app);
        let weak = $ui.as_weak();
        $ui.$setter(move |delta| {
            let mut app = app.borrow_mut();
            app.$method(delta);
            if let Some(ui) = weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }};
}

/// The row and selection callbacks.
fn wire_rows(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    on_index!(ui, app, on_content_row_clicked, select_content);
    on_index!(ui, app, on_content_row_ctrl_clicked, toggle_content);
    on_index!(ui, app, on_content_row_shift_clicked, extend_selection_to);
    on_index!(ui, app, on_content_row_double_clicked, open_content);
    on_delta!(ui, app, on_selection_moved, move_selection);
    on_delta!(ui, app, on_pane_cycled, cycle_focus);
    {
        let app = Rc::clone(app);
        let weak = ui.as_weak();
        ui.on_content_rows_marqueed(move |from, to| {
            let mut app = app.borrow_mut();
            app.select_range(
                usize::try_from(from).unwrap_or(usize::MAX),
                usize::try_from(to).unwrap_or(usize::MAX),
            );
            if let Some(ui) = weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
    {
        let app = Rc::clone(app);
        let weak = ui.as_weak();
        ui.on_edge_requested(move |last| {
            let mut app = app.borrow_mut();
            app.select_edge(last != 0);
            if let Some(ui) = weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
}

/// The command and operation callbacks these tests reach.
///
/// `repos-root-requested` is deliberately left unwired: it would write the
/// developer's own configured Repos Directory, and nothing here asks for it.
fn wire_commands(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    on_plain!(ui, app, on_cancel_requested, cancel_pending);
    on_plain!(ui, app, on_delete_requested, request_delete);
    on_plain!(ui, app, on_content_delete_requested, request_delete);
    on_plain!(ui, app, on_return_pressed, handle_return);
    on_plain!(ui, app, on_backspace_pressed, backspace);
    on_plain!(ui, app, on_parent_requested, navigate_to_parent);
    on_plain!(ui, app, on_content_rename_requested, request_rename);
    on_plain!(ui, app, on_content_copy_requested, request_copy);
    on_plain!(ui, app, on_content_extract_requested, request_extract);
    on_plain!(ui, app, on_new_folder_requested, request_new_folder);
    on_plain!(ui, app, on_new_file_requested, request_new_file);
    on_plain!(ui, app, on_clipboard_copy_requested, copy_to_clipboard);
    on_plain!(ui, app, on_clipboard_cut_requested, cut_to_clipboard);
    on_plain!(ui, app, on_clipboard_paste_requested, paste_from_clipboard);
    on_plain!(ui, app, on_select_all_requested, select_all);
    on_plain!(ui, app, on_undo_requested, undo);
    on_plain!(ui, app, on_refresh_requested, refresh);
    {
        let app = Rc::clone(app);
        let weak = ui.as_weak();
        ui.on_key_text(move |text| {
            let mut app = app.borrow_mut();
            app.handle_key_text(&text);
            if let Some(ui) = weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
    {
        let app = Rc::clone(app);
        let weak = ui.as_weak();
        ui.on_content_open_requested(move || {
            let mut app = app.borrow_mut();
            let selected = app.content_selected();
            app.open_content(selected);
            if let Some(ui) = weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
}

// ---------------------------------------------------------------------
// The harness.
// ---------------------------------------------------------------------

/// A shown window on a real `App` rooted at `root`, wired as `main` wires
/// it, with the opening listing already loaded.
fn window_at(root: &Path) -> (MainWindow, Rc<RefCell<App>>) {
    ensure_service();
    i_slint_backend_testing::init_no_event_loop();
    let app = Rc::new(RefCell::new(App::new(root.to_path_buf())));
    let ui = MainWindow::new().expect("the window should build");
    sync_ui(&ui, &app.borrow());
    wire_rows(&ui, &app);
    wire_commands(&ui, &app);
    ui.show().expect("the window should show");
    pump(&ui, &app);
    (ui, app)
}

/// Whether `status` is one of the transient lines an in-flight request puts
/// up, which is how [`pump`] knows the application is still working.
fn still_working(status: &str) -> bool {
    status.starts_with("loading ") || matches!(status, "working..." | "deleting..." | "undoing...")
}

/// Ticks the application the way the window's 100ms timer does, until every
/// in-flight request has landed. Not a sleep: the same `tick` and `sync_ui`
/// pair `main` runs, just as fast as the results arrive.
fn pump(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut quiet = 0u32;
    while Instant::now() < deadline {
        {
            let mut app = app.borrow_mut();
            app.tick();
            sync_ui(ui, &app);
        }
        if still_working(&ui.get_status_text()) {
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

/// Presses a named key such as `Key::F2`.
fn press_key(ui: &MainWindow, key: Key) {
    press(ui, &char::from(key).to_string());
}

/// Types `text` one character at a time, the way a keyboard delivers it.
fn type_text(ui: &MainWindow, text: &str) {
    for c in text.chars() {
        press(ui, &c.to_string());
    }
}

/// Clicks the contents pane `rows_down` rows below its first row, with
/// `modifiers` held.
fn click_row_with(ui: &MainWindow, rows_down: f32, modifiers: &[Key]) {
    let pane = ElementHandle::find_by_element_id(ui, "ContentsPane::click-area")
        .next()
        .expect("the contents pane has a click area");
    let origin = pane.absolute_position();
    let position = LogicalPosition::new(
        origin.x + 20.0,
        origin.y + rows_down.mul_add(ROW_HEIGHT, ROW_HEIGHT / 2.0),
    );
    let window = ui.window();
    for modifier in modifiers {
        window.dispatch_event(WindowEvent::KeyPressed {
            text: char::from(*modifier).into(),
        });
    }
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    for modifier in modifiers.iter().rev() {
        window.dispatch_event(WindowEvent::KeyReleased {
            text: char::from(*modifier).into(),
        });
    }
}

/// Clicks the contents pane `rows_down` rows below its first row.
fn click_row(ui: &MainWindow, rows_down: f32) {
    click_row_with(ui, rows_down, &[]);
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

/// The row index of `name` in the drawn listing.
fn row_of(ui: &MainWindow, name: &str) -> f32 {
    let index = listing(ui)
        .iter()
        .position(|drawn| drawn == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing: {:?}", listing(ui)));
    let index = u16::try_from(index).expect("a small listing");
    f32::from(index)
}

/// Clears a pre-filled prompt by backspacing over every character of it.
fn clear_prompt(ui: &MainWindow, characters: usize) {
    for _ in 0..characters {
        press_key(ui, Key::Backspace);
    }
}

// ---------------------------------------------------------------------
// Rename.
// ---------------------------------------------------------------------

#[test]
fn f2_renames_the_file_the_prompt_names() {
    let _serial = serially();
    let dir = scratch("rename-f2");
    file(&dir, "alpha.txt", "a");
    file(&dir, "beta.txt", "b");
    let (ui, app) = window_at(&dir);

    click_row(&ui, row_of(&ui, "alpha.txt"));
    press_key(&ui, Key::F2);

    assert_eq!(
        ui.get_content_prompt_text().as_str(),
        "Rename to:  alpha.txt",
        "F2 should offer the selected file's own name"
    );
    assert!(
        ui.get_content_prompt_editable(),
        "a rename prompt takes typing"
    );

    clear_prompt(&ui, "alpha.txt".len());
    type_text(&ui, "gamma.txt");
    press_key(&ui, Key::Return);
    pump(&ui, &app);

    assert!(dir.join("gamma.txt").is_file(), "the file was renamed");
    assert!(!dir.join("alpha.txt").exists(), "the old name is gone");
    assert_eq!(
        listing(&ui),
        vec!["beta.txt".to_owned(), "gamma.txt".to_owned()],
        "the listing shows the new name"
    );
    assert_eq!(
        selected(&ui),
        vec!["gamma.txt".to_owned()],
        "the renamed file stays selected"
    );
}

#[test]
fn escape_abandons_a_rename_and_leaves_the_file_alone() {
    let _serial = serially();
    let dir = scratch("rename-escape");
    file(&dir, "alpha.txt", "a");
    let (ui, app) = window_at(&dir);

    click_row(&ui, 0.0);
    press_key(&ui, Key::F2);
    clear_prompt(&ui, "alpha.txt".len());
    type_text(&ui, "wrong.txt");
    press_key(&ui, Key::Escape);
    pump(&ui, &app);

    assert!(dir.join("alpha.txt").is_file(), "the file kept its name");
    assert!(!dir.join("wrong.txt").exists(), "nothing was renamed");
    assert_eq!(
        ui.get_content_prompt_row(),
        -1,
        "the prompt is gone after Escape"
    );
}

#[test]
fn a_rename_prompt_stays_on_the_row_it_names() {
    let _serial = serially();
    let dir = scratch("rename-prompt-row");
    file(&dir, "alpha.txt", "a");
    file(&dir, "beta.txt", "b");
    file(&dir, "gamma.txt", "g");
    let (ui, app) = window_at(&dir);

    let alpha_row = row_of(&ui, "alpha.txt");
    let gamma_row = row_of(&ui, "gamma.txt");
    click_row(&ui, alpha_row);
    press_key(&ui, Key::F2);
    assert_eq!(ui.get_content_prompt_row(), 0, "the prompt opens on alpha");

    // The prompt is an overlay on one row, not a modal dialog: every other
    // row is still clickable underneath it.
    click_row(&ui, gamma_row);
    pump(&ui, &app);
    let drawn_over = ui.get_content_prompt_row();

    // Still the same prompt, still renaming alpha.txt...
    assert_eq!(
        ui.get_content_prompt_text().as_str(),
        "Rename to:  alpha.txt",
        "the prompt still names alpha.txt"
    );
    // ...and Enter proves it.
    clear_prompt(&ui, "alpha.txt".len());
    type_text(&ui, "renamed.txt");
    press_key(&ui, Key::Return);
    pump(&ui, &app);
    assert!(dir.join("renamed.txt").is_file(), "alpha.txt was renamed");
    assert!(dir.join("gamma.txt").is_file(), "gamma.txt was not touched");

    assert_eq!(
        drawn_over, 0,
        "the prompt that renames alpha.txt (row 0) followed the click onto \
         gamma.txt's row: the reader types a new name over gamma.txt and \
         alpha.txt is what gets renamed"
    );
}

// ---------------------------------------------------------------------
// New File and New Folder.
// ---------------------------------------------------------------------

#[test]
fn ctrl_n_creates_a_file_and_offers_its_name_for_retyping() {
    let _serial = serially();
    let dir = scratch("new-file");
    file(&dir, "zeta.txt", "z");
    let (ui, app) = window_at(&dir);

    press_with(&ui, "n", &[Key::Control]);
    pump(&ui, &app);

    assert!(dir.join("New file").is_file(), "Ctrl+N created a file");
    assert!(
        listing(&ui).contains(&"New file".to_owned()),
        "the listing shows it: {:?}",
        listing(&ui)
    );
    assert_eq!(
        ui.get_content_prompt_text().as_str(),
        "Rename to:  New file",
        "creating drops straight into rename"
    );
    assert_eq!(
        ui.get_content_prompt_row(),
        i32::try_from(
            listing(&ui)
                .iter()
                .position(|name| name == "New file")
                .expect("the new file is listed")
        )
        .expect("a small listing"),
        "the rename prompt sits on the row it renames"
    );
}

#[test]
fn ctrl_shift_n_creates_a_folder() {
    let _serial = serially();
    let dir = scratch("new-folder");
    let (ui, app) = window_at(&dir);

    press_with(&ui, "N", &[Key::Control, Key::Shift]);
    pump(&ui, &app);

    assert!(
        dir.join("New folder").is_dir(),
        "Ctrl+Shift+N made a folder"
    );
    assert_eq!(
        ui.get_content_prompt_text().as_str(),
        "Rename to:  New folder",
        "creating drops straight into rename"
    );

    clear_prompt(&ui, "New folder".len());
    type_text(&ui, "notes");
    press_key(&ui, Key::Return);
    pump(&ui, &app);

    assert!(dir.join("notes").is_dir(), "the retyped name took effect");
    assert!(!dir.join("New folder").exists(), "the default name is gone");
}

// ---------------------------------------------------------------------
// Delete.
// ---------------------------------------------------------------------

#[test]
fn deleting_a_multiple_selection_names_and_removes_the_same_files() {
    let _serial = serially();
    let dir = scratch("delete-many");
    file(&dir, "one.txt", "1");
    file(&dir, "two.txt", "2");
    file(&dir, "three.txt", "3");
    let (ui, app) = window_at(&dir);

    click_row(&ui, row_of(&ui, "one.txt"));
    click_row_with(&ui, row_of(&ui, "three.txt"), &[Key::Control]);
    assert_eq!(
        selected(&ui).len(),
        2,
        "Ctrl+click should have built a two-row selection, got {:?}",
        selected(&ui)
    );

    press_key(&ui, Key::Delete);

    assert_eq!(
        ui.get_content_prompt_text().as_str(),
        "Delete 2 items?  (y / n)",
        "the prompt counts what is selected"
    );

    press(&ui, "y");
    pump(&ui, &app);

    assert!(!dir.join("one.txt").exists(), "one.txt was deleted");
    assert!(!dir.join("three.txt").exists(), "three.txt was deleted");
    assert!(
        dir.join("two.txt").is_file(),
        "two.txt was never selected and must survive"
    );
}

#[test]
fn a_delete_confirmation_stays_on_the_row_it_names() {
    let _serial = serially();
    let dir = scratch("delete-prompt-row");
    file(&dir, "keep.txt", "k");
    file(&dir, "target.txt", "t");
    let (ui, app) = window_at(&dir);

    let keep_row = row_of(&ui, "keep.txt");
    let target_row = row_of(&ui, "target.txt");
    click_row(&ui, target_row);
    press_key(&ui, Key::Delete);
    assert_eq!(
        ui.get_content_prompt_text().as_str(),
        "Delete target.txt?  (y / n)"
    );
    assert_eq!(ui.get_content_prompt_row(), 1, "the prompt opens on target");

    click_row(&ui, keep_row);
    pump(&ui, &app);
    let drawn_over = ui.get_content_prompt_row();

    // The y goes to whatever the prompt is still waiting on, not to the row
    // the prompt is now sitting on.
    press(&ui, "y");
    pump(&ui, &app);
    assert!(!dir.join("target.txt").exists(), "target.txt was deleted");
    assert!(dir.join("keep.txt").is_file(), "keep.txt survived");

    assert_eq!(
        drawn_over, 1,
        "the confirmation that names target.txt (row 1) followed the click \
         onto keep.txt's row 0: the reader sees \"Delete target.txt?\" drawn \
         over keep.txt and presses y"
    );
}

#[test]
fn delete_while_typing_a_new_name_does_not_arm_a_deletion() {
    let _serial = serially();
    let dir = scratch("delete-during-rename");
    file(&dir, "precious.txt", "p");
    let (ui, app) = window_at(&dir);

    click_row(&ui, 0.0);
    press_key(&ui, Key::F2);
    // Clearing a pre-filled field with Delete rather than Backspace is an
    // ordinary reflex, and the prompt draws a caret inviting exactly that.
    press_key(&ui, Key::Delete);
    let prompt_after_delete = ui.get_content_prompt_text().to_string();

    // As far as the user is concerned they are still typing a name, and the
    // next letter of it happens to be a y.
    press(&ui, "y");
    pump(&ui, &app);

    assert!(
        dir.join("precious.txt").is_file(),
        "pressing Delete and then typing a name deleted the file being \
         renamed; after Delete the prompt read {prompt_after_delete:?}"
    );
}

#[test]
fn escape_declines_a_delete_confirmation() {
    let _serial = serially();
    let dir = scratch("delete-escape");
    file(&dir, "alpha.txt", "a");
    let (ui, app) = window_at(&dir);

    click_row(&ui, 0.0);
    press_key(&ui, Key::Delete);
    press_key(&ui, Key::Escape);
    pump(&ui, &app);

    assert!(
        dir.join("alpha.txt").is_file(),
        "Escape declined the delete"
    );
    assert_eq!(ui.get_content_prompt_row(), -1, "the prompt is gone");

    // And the y that would have confirmed it is now only a type-ahead.
    press(&ui, "y");
    pump(&ui, &app);
    assert!(
        dir.join("alpha.txt").is_file(),
        "a declined confirmation must not still be armed"
    );
}

// ---------------------------------------------------------------------
// Copy, Cut and Paste across two directories.
// ---------------------------------------------------------------------

/// Opens the folder named `name` in the contents pane, by selecting its row
/// and pressing Return.
fn open_folder(ui: &MainWindow, app: &Rc<RefCell<App>>, name: &str) {
    click_row(ui, row_of(ui, name));
    press_key(ui, Key::Return);
    pump(ui, app);
}

#[test]
fn copy_and_paste_moves_a_file_into_another_directory() {
    let _serial = serially();
    let dir = scratch("copy-paste");
    std::fs::create_dir(dir.join("source")).expect("a source directory");
    std::fs::create_dir(dir.join("target")).expect("a target directory");
    file(&dir.join("source"), "note.txt", "hello");
    let (ui, app) = window_at(&dir);

    open_folder(&ui, &app, "source");
    click_row(&ui, row_of(&ui, "note.txt"));
    press_with(&ui, "c", &[Key::Control]);

    assert_eq!(
        ui.get_status_text().as_str(),
        "note.txt copied",
        "the status bar says what went on the clipboard"
    );
    assert!(ui.get_can_paste(), "Paste is now available");

    press_with(&ui, &char::from(Key::UpArrow).to_string(), &[Key::Alt]);
    pump(&ui, &app);
    open_folder(&ui, &app, "target");
    press_with(&ui, "v", &[Key::Control]);
    pump(&ui, &app);

    assert!(
        dir.join("target").join("note.txt").is_file(),
        "the copy landed in target"
    );
    assert!(
        dir.join("source").join("note.txt").is_file(),
        "a copy leaves the original where it was"
    );
    assert_eq!(
        selected(&ui),
        vec!["note.txt".to_owned()],
        "the pasted file is selected in the folder it landed in"
    );
}

#[test]
fn cut_and_paste_moves_a_file_and_spends_the_clipboard() {
    let _serial = serially();
    let dir = scratch("cut-paste");
    std::fs::create_dir(dir.join("source")).expect("a source directory");
    std::fs::create_dir(dir.join("target")).expect("a target directory");
    file(&dir.join("source"), "note.txt", "hello");
    let (ui, app) = window_at(&dir);

    open_folder(&ui, &app, "source");
    click_row(&ui, row_of(&ui, "note.txt"));
    press_with(&ui, "x", &[Key::Control]);
    assert_eq!(ui.get_status_text().as_str(), "note.txt cut");

    press_with(&ui, &char::from(Key::UpArrow).to_string(), &[Key::Alt]);
    pump(&ui, &app);
    open_folder(&ui, &app, "target");
    press_with(&ui, "v", &[Key::Control]);
    pump(&ui, &app);

    assert!(
        dir.join("target").join("note.txt").is_file(),
        "the file moved into target"
    );
    assert!(
        !dir.join("source").join("note.txt").exists(),
        "a cut leaves nothing behind"
    );
    assert!(
        !ui.get_can_paste(),
        "a cut is spent by the paste that moved it"
    );
}

// Ignored, not deleted: the defect is real and this test is the proof of
// it, but the fix is not a front-end change. `Request::Copy` moves one
// file, so a paste of several is several requests, and the service keeps
// one undo step (D6) - three requests would leave Ctrl+Z putting back one
// file of three and reporting success, which is a worse lie than the one
// being fixed. #504 settles whether D6 counts operations or requests, and
// removes this attribute.
#[ignore = "see #504: needs a multi-step undo before a multi-file paste is honest"]
#[test]
fn copying_a_multiple_selection_takes_every_file_it_says_it_took() {
    let _serial = serially();
    let dir = scratch("copy-many");
    std::fs::create_dir(dir.join("target")).expect("a target directory");
    file(&dir, "one.txt", "1");
    file(&dir, "two.txt", "2");
    let (ui, app) = window_at(&dir);

    click_row(&ui, row_of(&ui, "one.txt"));
    click_row_with(&ui, row_of(&ui, "two.txt"), &[Key::Control]);
    assert_eq!(selected(&ui).len(), 2, "two rows are selected");

    press_with(&ui, "c", &[Key::Control]);
    let said = ui.get_status_text().to_string();

    open_folder(&ui, &app, "target");
    press_with(&ui, "v", &[Key::Control]);
    pump(&ui, &app);

    assert!(
        dir.join("target").join("one.txt").is_file()
            && dir.join("target").join("two.txt").is_file(),
        "Ctrl+C with two rows selected copied only one of them; the status \
         bar said {said:?} while the pane showed two rows highlighted"
    );
}

// ---------------------------------------------------------------------
// Undo.
// ---------------------------------------------------------------------

#[test]
fn undo_reverses_a_rename() {
    let _serial = serially();
    let dir = scratch("undo-rename");
    file(&dir, "alpha.txt", "a");
    let (ui, app) = window_at(&dir);

    click_row(&ui, 0.0);
    press_key(&ui, Key::F2);
    clear_prompt(&ui, "alpha.txt".len());
    type_text(&ui, "omega.txt");
    press_key(&ui, Key::Return);
    pump(&ui, &app);
    assert!(dir.join("omega.txt").is_file(), "the rename happened");

    press_with(&ui, "z", &[Key::Control]);
    pump(&ui, &app);

    assert!(dir.join("alpha.txt").is_file(), "undo put the name back");
    assert!(!dir.join("omega.txt").exists(), "the new name is gone");
    assert_eq!(
        listing(&ui),
        vec!["alpha.txt".to_owned()],
        "the listing was reloaded after the undo"
    );
}

#[test]
fn undo_removes_a_file_that_was_just_created() {
    let _serial = serially();
    let dir = scratch("undo-create");
    let (ui, app) = window_at(&dir);

    press_with(&ui, "n", &[Key::Control]);
    pump(&ui, &app);
    assert!(dir.join("New file").is_file(), "the file was created");

    // Ctrl+N leaves a rename prompt open, and Ctrl+Z is ignored while one
    // is: Escape first, the way the user would.
    press_key(&ui, Key::Escape);
    press_with(&ui, "z", &[Key::Control]);
    pump(&ui, &app);

    assert!(
        !dir.join("New file").exists(),
        "undo should remove what Ctrl+N created"
    );
}

#[test]
fn undo_removes_a_pasted_copy() {
    let _serial = serially();
    let dir = scratch("undo-paste");
    std::fs::create_dir(dir.join("target")).expect("a target directory");
    file(&dir, "note.txt", "hello");
    let (ui, app) = window_at(&dir);

    click_row(&ui, row_of(&ui, "note.txt"));
    press_with(&ui, "c", &[Key::Control]);
    open_folder(&ui, &app, "target");
    press_with(&ui, "v", &[Key::Control]);
    pump(&ui, &app);
    assert!(dir.join("target").join("note.txt").is_file(), "pasted");

    press_with(&ui, "z", &[Key::Control]);
    pump(&ui, &app);

    assert!(
        !dir.join("target").join("note.txt").exists(),
        "undo should take the pasted copy back out"
    );
    assert!(dir.join("note.txt").is_file(), "the original is untouched");
}

#[test]
fn a_rename_onto_a_name_already_in_use_is_refused_out_loud() {
    let _serial = serially();
    let dir = scratch("rename-collision");
    file(&dir, "alpha.txt", "a");
    file(&dir, "beta.txt", "b");
    let (ui, app) = window_at(&dir);

    click_row(&ui, row_of(&ui, "alpha.txt"));
    press_key(&ui, Key::F2);
    clear_prompt(&ui, "alpha.txt".len());
    type_text(&ui, "beta.txt");
    press_key(&ui, Key::Return);
    pump(&ui, &app);

    assert_eq!(
        std::fs::read_to_string(dir.join("beta.txt")).expect("beta.txt is readable"),
        "b",
        "the rename must not have written over beta.txt"
    );
    assert!(dir.join("alpha.txt").is_file(), "alpha.txt is still there");
    assert!(
        ui.get_status_text().contains("already exists"),
        "the status bar should say why nothing happened; it said {:?}",
        ui.get_status_text()
    );
}

#[test]
fn undo_with_nothing_to_undo_says_so_rather_than_nothing() {
    let _serial = serially();
    let dir = scratch("undo-nothing");
    file(&dir, "alpha.txt", "a");
    let (ui, app) = window_at(&dir);

    // A delete is the one operation the service records no undo step for,
    // so it leaves the application with nothing to reverse.
    click_row(&ui, 0.0);
    press_key(&ui, Key::Delete);
    press(&ui, "y");
    pump(&ui, &app);
    assert!(!dir.join("alpha.txt").exists(), "the delete happened");

    press_with(&ui, "z", &[Key::Control]);
    pump(&ui, &app);

    assert!(
        ui.get_status_text().contains("nothing to undo"),
        "the status bar should say the undo did nothing, not go quiet; it \
         said {:?}",
        ui.get_status_text()
    );
}
