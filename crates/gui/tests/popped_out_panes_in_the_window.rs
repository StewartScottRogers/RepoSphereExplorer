//! Use-case tests (#729): a popped-out pane does real work, not only the
//! pop-out and dock lifecycle `pop_out_windows.rs` and `pin_the_pane.rs`
//! already prove.
//!
//! Forty-odd files in this crate test one interaction or one hand-off from a
//! fresh window; `reading_and_editing_journey_in_the_window.rs` is the first
//! to carry a task through a single window end to end. Nothing before this
//! file sorts, right-clicks, renames, types towards a row, or edits and
//! saves inside a *popped-out* one, and nothing undoes a file operation
//! across the seam between two windows on one `App` - exactly where
//! CLAUDE.md rule 14 says a fault would hide.
//!
//! Full stack throughout: a real [`gui::PaneWindows`] registry, built with
//! `gui::wire_callbacks` and `gui::wire_pop_out` - the same functions `main`
//! calls - against one shared `App`, a real service on the private socket
//! `common::ensure_service` provides, and real files in a scratch directory.
//! Every step is a dispatched pointer or keyboard event or a markup
//! callback; nothing is measured that was not drawn.

use gui::app::{App, Pane};
use gui::{MainWindow, PaneWindows, sync_pinned_window, sync_ui};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model as _};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

mod common;
use common::ensure_service;

/// Row height in `app.slint`'s panes, so a click can be aimed at a row.
const ROW_HEIGHT: f32 = 20.0;

/// Context menu width in `app.slint`'s Contents pane, so a menu search does
/// not also match the command bar's same-labelled button.
const MENU_WIDTH: f32 = 150.0;

/// The tab strip's height in `app.slint`, for aiming at the middle of a
/// strip whose position and width are measured, never assumed.
const TAB_HEIGHT: f32 = 24.0;

/// This suite shares one service, and with it one undo step, so its tests
/// run one at a time - the guard `operations_in_the_window.rs` and
/// `reading_and_editing_journey_in_the_window.rs` both take.
static SERIAL: Mutex<()> = Mutex::new(());

/// Takes the shared lock, tolerating a previous test having panicked while
/// holding it - a poisoned lock would otherwise turn one failure into many.
fn serially() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// 0 Folders, 1 Contents, 2 File - matching `pop-out-requested`'s index and
/// `lib.rs`'s own (private) `pane_index`.
fn pane_index(pane: Pane) -> i32 {
    match pane {
        Pane::Folders => 0,
        Pane::Contents => 1,
        Pane::File => 2,
    }
}

/// An empty directory of this test's own under the platform's temporary
/// directory. Nothing in this file ever touches a path outside it.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("rse-popped-out-journeys")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// Writes a file into `dir`.
fn file(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).expect("a scratch file");
}

// ---------------------------------------------------------------------
// The harness, in the shape `pop_out_windows.rs` and `pin_the_pane.rs` use.
// ---------------------------------------------------------------------

/// Whether `status` is one of the transient lines an in-flight request puts
/// up, which is how [`pump`] knows the application is still working.
fn still_working(status: &str) -> bool {
    status.starts_with("loading ")
        || matches!(
            status,
            "working..." | "deleting..." | "undoing..." | "saving..."
        )
}

/// Ticks the application and syncs every open window, the way `main`'s
/// 100ms timer does - a pinned window from its own snapshot, every other
/// window from the shared selection, exactly as `main.rs` chooses between
/// them (see `pin_the_pane.rs`'s own `pump`).
fn pump(windows: &Rc<RefCell<PaneWindows>>, app: &Rc<RefCell<App>>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut quiet = 0u32;
    loop {
        assert!(Instant::now() < deadline, "the application never settled");
        let (busy, status) = {
            let mut app = app.borrow_mut();
            app.tick();
            let windows = windows.borrow();
            for (ui, pin) in windows.windows_with_pin() {
                match pin {
                    Some(id) if app.is_pinned(id) => sync_pinned_window(ui, &app, id),
                    _ => sync_ui(ui, &app),
                }
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

/// A shown main window on a real `App` rooted at `root`, registered as the
/// only entry of a fresh [`PaneWindows`], wired exactly as `main` wires it.
fn windows_at(root: &Path) -> (Rc<RefCell<PaneWindows>>, Rc<RefCell<App>>) {
    ensure_service();
    i_slint_backend_testing::init_no_event_loop();
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

/// A strong handle to the window holding `pane` right now, gotten and
/// dropped out of `windows`'s borrow in one statement - so the caller can
/// invoke a callback on it without still holding that borrow, which a
/// pop-out, dock or pin callback re-enters (#617).
fn pane_handle(windows: &Rc<RefCell<PaneWindows>>, pane: Pane) -> MainWindow {
    windows
        .borrow()
        .window_for(pane)
        .as_weak()
        .upgrade()
        .expect("the window has not been dropped yet")
}

/// The same, for the main window itself.
fn main_handle(windows: &Rc<RefCell<PaneWindows>>) -> MainWindow {
    windows
        .borrow()
        .main()
        .as_weak()
        .upgrade()
        .expect("the window has not been dropped yet")
}

// ---------------------------------------------------------------------
// Keyboard and pointer helpers, in the shape `operations_in_the_window.rs`
// and `context_menu.rs` use.
// ---------------------------------------------------------------------

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

/// Clears a pre-filled prompt by backspacing over every character of it.
fn clear_prompt(ui: &MainWindow, characters: usize) {
    for _ in 0..characters {
        press_key(ui, Key::Backspace);
    }
}

/// The contents pane's click area in `ui`, wherever that window draws it.
fn click_area(ui: &MainWindow) -> ElementHandle {
    ElementHandle::find_by_element_id(ui, "ContentsPane::click-area")
        .next()
        .expect("the contents pane has a click area")
}

/// Clicks the contents pane `rows_down` rows below its first row, with
/// `button` and no modifiers.
fn click_row_button(ui: &MainWindow, rows_down: f32, button: PointerEventButton) {
    let pane = click_area(ui);
    let origin = pane.absolute_position();
    let position = LogicalPosition::new(
        origin.x + 20.0,
        origin.y + rows_down.mul_add(ROW_HEIGHT, ROW_HEIGHT / 2.0),
    );
    let window = ui.window();
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

/// Left-clicks the contents pane `rows_down` rows below its first row.
fn click_row(ui: &MainWindow, rows_down: f32) {
    click_row_button(ui, rows_down, PointerEventButton::Left);
}

/// Right-clicks the contents pane `rows_down` rows below its first row,
/// which selects that row and opens its context menu, the way
/// `contents_pane.slint`'s own right-click handling does.
fn right_click(ui: &MainWindow, rows_down: f32) {
    click_row_button(ui, rows_down, PointerEventButton::Right);
}

/// The open menu's items labelled `label` in `ui`, ignoring same-named
/// elements elsewhere in the window such as the command bar's buttons.
fn menu_items(ui: &MainWindow, label: &str) -> Vec<ElementHandle> {
    ElementHandle::find_by_accessible_label(ui, label)
        .filter(|item| (item.size().width - MENU_WIDTH).abs() < f32::EPSILON)
        .collect()
}

/// Clicks the open menu's item labelled `label`.
fn choose(ui: &MainWindow, label: &str) {
    let items = menu_items(ui, label);
    assert_eq!(
        items.len(),
        1,
        "exactly one open menu item should be labelled {label:?}"
    );
    items[0].mock_single_click(PointerEventButton::Left);
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

/// The size column the contents pane draws for the row named `name`.
fn size_of(ui: &MainWindow, name: &str) -> String {
    ui.get_content_rows()
        .iter()
        .find(|row| row.name.trim_end_matches('/') == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing: {:?}", listing(ui)))
        .size
        .to_string()
}

/// The tab labels the File pane is drawing.
fn tabs(ui: &MainWindow) -> Vec<String> {
    ui.get_file_tabs()
        .iter()
        .map(|label| label.to_string())
        .collect()
}

/// The tab strip, measured rather than assumed - the tabs share whatever
/// width the File pane has, and a constant here would let a test click a
/// tab that is off the edge of the pane and pass while the reader could not
/// reach it.
fn strip(ui: &MainWindow) -> ElementHandle {
    ElementHandle::find_by_element_id(ui, "FilePane::tab-strip")
        .next()
        .expect("a strip should be drawn")
}

/// Clicks the middle of tab `index`, as it is actually drawn.
fn click_tab(ui: &MainWindow, index: usize) {
    let strip = strip(ui);
    let count = u16::try_from(tabs(ui).len()).expect("a handful of tabs");
    let width = strip.size().width / f32::from(count);
    let index = u16::try_from(index).expect("a handful of tabs");
    let origin = strip.absolute_position();
    let position = LogicalPosition::new(
        origin.x + f32::from(index).mul_add(width, width / 2.0),
        origin.y + TAB_HEIGHT / 2.0,
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

// ---------------------------------------------------------------------
// 1. A popped-out Contents pane is a Contents pane.
// ---------------------------------------------------------------------

/// A scratch directory of three files whose sizes disagree with their name
/// order, so sorting by size really does reorder the popped-out listing
/// rather than leaving it as name order already had it.
fn build_sortable_directory(name: &str) -> PathBuf {
    let dir = scratch(name);
    for (file_name, size) in [("alpha.txt", 300), ("bravo.txt", 200), ("charlie.txt", 100)] {
        std::fs::write(dir.join(file_name), "x".repeat(size)).expect("a scratch file");
    }
    dir
}

/// Sorts the popped-out Contents pane by size (column 1) and checks the
/// smallest file leads.
fn sort_by_size(popped: &MainWindow) {
    popped.invoke_content_sort_requested(1);
    assert_eq!(
        listing(popped),
        vec![
            "charlie.txt".to_owned(),
            "bravo.txt".to_owned(),
            "alpha.txt".to_owned(),
        ],
        "sorting by size should put the smallest file first"
    );
}

/// Selects the smallest file, then types towards bravo - the mirror of
/// `contents_in_the_window.rs`'s type-ahead test, run inside the popped-out
/// window instead of the main one.
fn type_towards_bravo(popped: &MainWindow) {
    click_row(popped, row_of(popped, "charlie.txt"));
    assert_eq!(selected(popped), vec!["charlie.txt".to_owned()]);
    press(popped, "b");
    assert_eq!(
        selected(popped),
        vec!["bravo.txt".to_owned()],
        "a typed letter should jump the popped-out listing to bravo.txt"
    );
}

/// Opens bravo's row menu and renames it to gamma.txt.
fn rename_bravo_from_the_row_menu(
    popped: &MainWindow,
    app: &Rc<RefCell<App>>,
    windows: &Rc<RefCell<PaneWindows>>,
) {
    right_click(popped, row_of(popped, "bravo.txt"));
    choose(popped, "Rename");
    assert_eq!(
        popped.get_content_prompt_text().as_str(),
        "Rename to:  bravo.txt",
        "the row menu's Rename should offer the row's own name"
    );
    clear_prompt(popped, "bravo.txt".len());
    type_text(popped, "gamma.txt");
    press_key(popped, Key::Return);
    pump(windows, app);
}

/// A popped-out Contents pane sorts, types towards a row, opens the row
/// menu, and renames a file - and the main window, which no longer draws
/// the Contents pane at all, is still handed the renamed listing.
#[test]
fn a_popped_out_contents_pane_sorts_types_towards_and_renames_a_row() {
    let _serial = serially();
    let dir = build_sortable_directory("sort-type-ahead-rename");
    let (windows, app) = windows_at(&dir);

    pane_handle(&windows, Pane::Contents).invoke_pop_out_requested(pane_index(Pane::Contents));
    pump(&windows, &app);
    let popped = pane_handle(&windows, Pane::Contents);

    sort_by_size(&popped);
    type_towards_bravo(&popped);
    rename_bravo_from_the_row_menu(&popped, &app, &windows);

    assert!(dir.join("gamma.txt").is_file(), "the file was renamed");
    assert!(!dir.join("bravo.txt").exists(), "the old name is gone");
    assert_eq!(
        selected(&popped),
        vec!["gamma.txt".to_owned()],
        "the renamed file should stay selected in the popped-out window"
    );

    let main = main_handle(&windows);
    assert!(
        !main.get_show_contents_pane(),
        "Contents is still popped out"
    );
    assert!(
        listing(&main).contains(&"gamma.txt".to_owned()),
        "the main window's own listing should agree, even though it is not \
         drawing the Contents pane right now: {:?}",
        listing(&main)
    );
    assert!(!listing(&main).contains(&"bravo.txt".to_owned()));
}

// ---------------------------------------------------------------------
// 2. A popped-out File pane edits and saves.
// ---------------------------------------------------------------------

/// Selects notes.txt in the main window's Contents pane - the pane the File
/// pop-out below leaves behind - and checks the popped-out File pane picked
/// it up. Returns the row's size as drawn now, so a save's effect on it can
/// be measured rather than assumed.
fn select_notes_from_main_and_check_the_popped_out_preview(
    main: &MainWindow,
    popped: &MainWindow,
    windows: &Rc<RefCell<PaneWindows>>,
    app: &Rc<RefCell<App>>,
) -> String {
    click_row(main, row_of(main, "notes.txt"));
    pump(windows, app);
    assert_eq!(
        shown(popped),
        "first line",
        "the popped-out File pane should show the file selected in the main \
         window's Contents pane: {}",
        shown(popped)
    );
    size_of(main, "notes.txt")
}

/// Opens the editor in the popped-out File window, types at the caret
/// without clicking first - the seam CLAUDE.md rule 14 names by name - and
/// saves.
fn type_and_save_in_the_popped_out_window(
    popped: &MainWindow,
    windows: &Rc<RefCell<PaneWindows>>,
    app: &Rc<RefCell<App>>,
) {
    click_tab(popped, tabs(popped).len() - 1);
    assert!(
        popped.get_editing_file(),
        "the last tab should open the editor"
    );
    popped
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: "X".into() });
    popped
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: "X".into() });
    assert_eq!(
        app.borrow().edit_text(),
        "Xfirst line\n",
        "the keystroke should have reached the editor without a click first"
    );
    popped.invoke_save_requested();
    pump(windows, app);
}

/// A popped-out File pane opens the editor, types at the caret and saves -
/// the bytes reach disk, the main window's own Contents row shows the new
/// size, and the popped-out preview re-reads what was written.
#[test]
fn a_popped_out_file_pane_edits_and_saves() {
    let _serial = serially();
    let dir = scratch("edit-and-save");
    file(&dir, "notes.txt", "first line\n");
    let (windows, app) = windows_at(&dir);

    pane_handle(&windows, Pane::File).invoke_pop_out_requested(pane_index(Pane::File));
    pump(&windows, &app);
    let main = main_handle(&windows);
    let popped = pane_handle(&windows, Pane::File);

    let size_before =
        select_notes_from_main_and_check_the_popped_out_preview(&main, &popped, &windows, &app);
    type_and_save_in_the_popped_out_window(&popped, &windows, &app);

    assert_eq!(
        std::fs::read_to_string(dir.join("notes.txt")).expect("notes.txt is readable"),
        "Xfirst line\n",
        "the edit should have reached the file"
    );
    assert_ne!(
        size_of(&main, "notes.txt"),
        size_before,
        "the main window's own Contents row should reflect the saved file"
    );
    assert_eq!(
        selected(&main),
        vec!["notes.txt".to_owned()],
        "saving should not move the reader off the file they saved"
    );
    assert!(!popped.get_editing_file(), "saving should close the editor");
    assert_eq!(
        shown(&popped),
        "Xfirst line",
        "the popped-out preview should re-read the file rather than keep \
         showing what was open in the editor"
    );
}

// ---------------------------------------------------------------------
// 3. An operation started in a popped-out window is undone from either.
// ---------------------------------------------------------------------

/// Renames `from` to `to` through the popped-out (or main) window `ui`'s own
/// F2 prompt.
fn rename_via_f2(
    ui: &MainWindow,
    from: &str,
    to: &str,
    windows: &Rc<RefCell<PaneWindows>>,
    app: &Rc<RefCell<App>>,
) {
    click_row(ui, row_of(ui, from));
    press_key(ui, Key::F2);
    clear_prompt(ui, from.len());
    type_text(ui, to);
    press_key(ui, Key::Return);
    pump(windows, app);
}

/// Undo reverses a rename made in a popped-out Contents window when the
/// keyboard shortcut is pressed on the main window instead.
#[test]
fn undo_pressed_in_the_main_window_reverses_a_popped_out_windows_rename() {
    let _serial = serially();
    let dir = scratch("undo-from-main");
    file(&dir, "one.txt", "1");
    let (windows, app) = windows_at(&dir);

    pane_handle(&windows, Pane::Contents).invoke_pop_out_requested(pane_index(Pane::Contents));
    pump(&windows, &app);
    let popped = pane_handle(&windows, Pane::Contents);
    rename_via_f2(&popped, "one.txt", "uno.txt", &windows, &app);
    assert!(dir.join("uno.txt").is_file(), "the rename happened");

    let main = main_handle(&windows);
    press_with(&main, "z", &[Key::Control]);
    pump(&windows, &app);

    assert!(dir.join("one.txt").is_file(), "undo put the name back");
    assert!(!dir.join("uno.txt").exists(), "the new name is gone");
    assert!(
        listing(&popped).contains(&"one.txt".to_owned()),
        "the popped-out window's own listing should have been reloaded too: {:?}",
        listing(&popped)
    );
}

/// The other way round: undo reverses a rename made in the main window's own
/// Contents pane when the keyboard shortcut is pressed on a popped-out
/// window instead.
#[test]
fn undo_pressed_in_a_popped_out_window_reverses_the_main_windows_rename() {
    let _serial = serially();
    let dir = scratch("undo-from-popped-out");
    file(&dir, "two.txt", "2");
    let (windows, app) = windows_at(&dir);

    // File pops out, leaving the Contents pane in the main window - the
    // reverse pairing from the test above, so both directions are covered
    // by a real window that actually draws the pane being acted on.
    pane_handle(&windows, Pane::File).invoke_pop_out_requested(pane_index(Pane::File));
    pump(&windows, &app);
    let main = main_handle(&windows);
    rename_via_f2(&main, "two.txt", "dos.txt", &windows, &app);
    assert!(dir.join("dos.txt").is_file(), "the rename happened");

    let popped = pane_handle(&windows, Pane::File);
    press_with(&popped, "z", &[Key::Control]);
    pump(&windows, &app);

    assert!(dir.join("two.txt").is_file(), "undo put the name back");
    assert!(!dir.join("dos.txt").exists(), "the new name is gone");
    assert!(
        listing(&main).contains(&"two.txt".to_owned()),
        "the main window's own listing should have been reloaded too: {:?}",
        listing(&main)
    );
}

// ---------------------------------------------------------------------
// 4. The selection stays shared throughout, and a pinned window still does
//    not follow (#619, #620).
// ---------------------------------------------------------------------

/// Pins the already popped-out File window on `name`.
fn pin_on(popped: &MainWindow, windows: &Rc<RefCell<PaneWindows>>, app: &Rc<RefCell<App>>) {
    assert!(!popped.get_pinned(), "not pinned yet");
    popped.invoke_pin_requested();
    pump(windows, app);
    assert!(popped.get_pinned(), "pinning should have taken");
}

/// With the File pane pinned on a.txt and the Contents pane also popped out,
/// moving the shared selection in the popped-out Contents window reaches the
/// main window's own hidden state too, but leaves the pinned File window
/// alone - and unpinning it catches it straight back up.
#[test]
fn the_shared_selection_reaches_every_popped_out_window_but_a_pinned_one_does_not_follow() {
    let _serial = serially();
    let dir = scratch("pin-and-selection");
    file(&dir, "a.txt", "file a\n");
    file(&dir, "b.txt", "file b\n");
    let (windows, app) = windows_at(&dir);
    let main = main_handle(&windows);

    click_row(&main, row_of(&main, "a.txt"));
    pump(&windows, &app);
    pane_handle(&windows, Pane::File).invoke_pop_out_requested(pane_index(Pane::File));
    pump(&windows, &app);
    let pinned = pane_handle(&windows, Pane::File);
    pin_on(&pinned, &windows, &app);
    assert!(shown(&pinned).contains("file a"), "pinned on a.txt");

    pane_handle(&windows, Pane::Contents).invoke_pop_out_requested(pane_index(Pane::Contents));
    pump(&windows, &app);
    let contents = pane_handle(&windows, Pane::Contents);
    click_row(&contents, row_of(&contents, "b.txt"));
    pump(&windows, &app);

    assert_eq!(
        selected(&contents),
        vec!["b.txt".to_owned()],
        "the popped-out Contents window shows the new selection"
    );
    assert_eq!(
        selected(&main),
        vec!["b.txt".to_owned()],
        "the main window, though it draws neither Contents nor File pane \
         right now, is still handed the same shared selection"
    );
    assert!(
        shown(&pinned).contains("file a"),
        "the pinned window keeps showing a.txt while the shared selection \
         moves on: {}",
        shown(&pinned)
    );

    pinned.invoke_unpin_requested();
    pump(&windows, &app);
    assert!(!pinned.get_pinned());
    assert!(
        shown(&pinned).contains("file b"),
        "unpinned, it now shows the shared selection too: {}",
        shown(&pinned)
    );
}
