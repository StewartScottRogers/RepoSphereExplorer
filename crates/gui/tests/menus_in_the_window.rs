//! The menu bar and the two context menus, driven through a real window on
//! a real [`App`], wired by the crate's own `wire_callbacks` - the same
//! function `main` calls.
//!
//! `commands.rs` and `context_menu.rs` already walk every menu item, but
//! they hang their own closures off the window's callbacks and have no
//! `App` behind them. They therefore prove that an item reaches *a*
//! callback and nothing about what the application then does: an item
//! pointed at the wrong method, an item offered where it cannot act, an
//! item that fires a pane command into a file somebody is typing, all look
//! identical to a watching closure. That seam is what this file measures,
//! per CLAUDE.md rule 14.
//!
//! Two rules this file holds itself to:
//!
//! * Where a test says an item is disabled, it *clicks* it and looks at
//!   what happened. Reading `accessible-enabled` back only proves the
//!   markup binds the property it binds; it says nothing about whether the
//!   press is actually refused.
//! * Items are located with [`ElementHandle`], by the label the reader
//!   sees, never by arithmetic from a hard-coded menu geometry. A constant
//!   that disagrees with the layout passes whatever it is pointed at.

use gui::app::App;
use gui::{MainWindow, sync_ui};
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

/// Row height in `app.slint`'s contents pane, so a pointer can be aimed at
/// a row. The one measurement that has no element to ask: a row is a `Text`
/// laid out by hand inside one overlay touch area, not an element of its
/// own.
const ROW_HEIGHT: f32 = 20.0;

/// These tests share one service, and with it one undo step, so they run
/// one at a time.
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
    let dir = std::env::temp_dir().join("rse-menus").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// Writes a file into `dir`.
fn file(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).expect("a scratch file");
}

// ---------------------------------------------------------------------
// The harness.
// ---------------------------------------------------------------------

/// A shown window on a real `App` rooted at `root`, wired the way `main`
/// wires it, with the opening listing already loaded.
fn window_at(root: &Path) -> (MainWindow, Rc<RefCell<App>>) {
    ensure_service();
    i_slint_backend_testing::init_no_event_loop();
    let app = Rc::new(RefCell::new(App::new(root.to_path_buf())));
    let ui = MainWindow::new().expect("the window should build");
    sync_ui(&ui, &app.borrow());
    gui::wire_callbacks(&ui, &app);
    ui.show().expect("the window should show");
    pump(&ui, &app);
    (ui, app)
}

/// Whether `status` is one of the transient lines an in-flight request puts
/// up, which is how [`pump`] knows the application is still working.
fn still_working(status: &str) -> bool {
    status.starts_with("loading ")
        || matches!(
            status,
            "working..." | "deleting..." | "undoing..." | "saving..."
        )
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
            // still in flight - and a test that then asked whether the file
            // can be edited read the answer for the row before it. On a
            // loaded continuous integration runner that is the difference
            // between passing and failing.
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

// ---------------------------------------------------------------------
// Keys and rows.
// ---------------------------------------------------------------------

/// Presses `text` as a key with `modifiers` held.
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

/// Presses a named key such as `Key::Delete`.
fn press_key(ui: &MainWindow, key: Key) {
    press_with(ui, &char::from(key).to_string(), &[]);
}

/// The contents pane's overlay touch area, which every row click goes
/// through.
fn contents_area(ui: &MainWindow) -> ElementHandle {
    ElementHandle::find_by_element_id(ui, "ContentsPane::click-area")
        .next()
        .expect("the contents pane has a click area")
}

/// Where in the window a point `rows_down` rows into the contents listing
/// falls.
fn row_point(ui: &MainWindow, rows_down: f32) -> LogicalPosition {
    let origin = contents_area(ui).absolute_position();
    LogicalPosition::new(
        origin.x + 20.0,
        origin.y + rows_down.mul_add(ROW_HEIGHT, ROW_HEIGHT / 2.0),
    )
}

/// Clicks the contents pane `rows_down` rows below its first row, with
/// `button` and `modifiers` held.
fn click_row_with(ui: &MainWindow, rows_down: f32, button: PointerEventButton, modifiers: &[Key]) {
    let position = row_point(ui, rows_down);
    let window = ui.window();
    for modifier in modifiers {
        window.dispatch_event(WindowEvent::KeyPressed {
            text: char::from(*modifier).into(),
        });
    }
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
    for modifier in modifiers.iter().rev() {
        window.dispatch_event(WindowEvent::KeyReleased {
            text: char::from(*modifier).into(),
        });
    }
}

/// Left-clicks a contents row.
fn click_row(ui: &MainWindow, rows_down: f32) {
    click_row_with(ui, rows_down, PointerEventButton::Left, &[]);
}

/// Control-clicks a contents row, which adds it to the selection or takes
/// it back out.
fn ctrl_click_row(ui: &MainWindow, rows_down: f32) {
    click_row_with(ui, rows_down, PointerEventButton::Left, &[Key::Control]);
}

/// Right-clicks the contents pane `rows_down` rows below its first row.
/// Past the last row that opens the empty-area menu; on one, the row menu.
fn right_click_row(ui: &MainWindow, rows_down: f32) {
    click_row_with(ui, rows_down, PointerEventButton::Right, &[]);
}

/// The folders pane's row rectangles, in the order they are drawn.
fn folder_rows(ui: &MainWindow) -> Vec<ElementHandle> {
    ElementHandle::find_by_element_id(ui, "Pane::tree-row").collect()
}

/// The row index of `name` in the folders tree, as drawn.
fn folder_row_of(ui: &MainWindow, name: &str) -> usize {
    folder_rows(ui)
        .iter()
        .position(|row| row.accessible_label().as_deref() == Some(name))
        .unwrap_or_else(|| panic!("{name} is not drawn in the folders tree"))
}

/// Right-clicks the folders tree's row `index`, the way a reader opens its
/// context menu (#581).
fn right_click_folder_row(ui: &MainWindow, index: usize) {
    let rows = folder_rows(ui);
    let handle = rows.get(index).expect("the tree draws that row");
    let at = handle.absolute_position();
    let size = handle.size();
    let position = LogicalPosition::new(at.x + 20.0, at.y + size.height / 2.0);
    let window = ui.window();
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Right,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Right,
    });
}

/// The names the contents pane is drawing, in the order it draws them.
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

/// The row index of `name` in the drawn listing, as a distance in rows.
fn row_of(ui: &MainWindow, name: &str) -> f32 {
    let index = listing(ui)
        .iter()
        .position(|drawn| drawn == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing: {:?}", listing(ui)));
    f32::from(u16::try_from(index).expect("a small listing"))
}

// ---------------------------------------------------------------------
// Menus.
// ---------------------------------------------------------------------

/// Every menu item currently on screen, in whichever menu is open. Found
/// by element type rather than by size or position, so a dropdown that
/// moves or changes width does not quietly stop being tested.
fn open_items(ui: &MainWindow) -> Vec<ElementHandle> {
    ElementHandle::find_by_element_type_name(ui, "ContextMenuItem").collect()
}

/// The labels of the open menu's items, top to bottom.
fn open_labels(ui: &MainWindow) -> Vec<String> {
    let mut items = open_items(ui);
    items.sort_by(|a, b| {
        a.absolute_position()
            .y
            .partial_cmp(&b.absolute_position().y)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    items
        .iter()
        .map(|item| {
            item.accessible_label()
                .map(|label| label.to_string())
                .unwrap_or_default()
        })
        .collect()
}

/// The open menu's item labelled `label`.
fn item(ui: &MainWindow, label: &str) -> ElementHandle {
    let mut found: Vec<ElementHandle> = open_items(ui)
        .into_iter()
        .filter(|item| item.accessible_label().is_some_and(|found| found == label))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "exactly one open menu item should be labelled {label:?}; the menu holds {:?}",
        open_labels(ui)
    );
    found.remove(0)
}

/// Clicks the open menu's item labelled `label`, by dispatching a real
/// press and release at it. A disabled item's touch area refuses the press
/// and it lands on whatever is behind the menu, which is what makes this a
/// measurement of the refusal rather than a reading of a property.
fn choose(ui: &MainWindow, label: &str) {
    item(ui, label).mock_single_click(PointerEventButton::Left);
}

/// Opens the menu-bar menu titled `title`. The titles are the topmost
/// elements in the window carrying their text, which is how one is told
/// from the command-bar button and the menu item that share its word.
fn open_menu(ui: &MainWindow, title: &str) {
    let mut titles: Vec<ElementHandle> =
        ElementHandle::find_by_accessible_label(ui, title).collect();
    titles.sort_by(|a, b| {
        a.absolute_position()
            .y
            .partial_cmp(&b.absolute_position().y)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    titles
        .first()
        .unwrap_or_else(|| panic!("the menu bar has no title {title:?}"))
        .mock_single_click(PointerEventButton::Left);
    assert!(
        !open_items(ui).is_empty(),
        "clicking the {title:?} title should have opened a menu"
    );
}

/// Opens `title` and chooses `label` from it.
fn from_menu(ui: &MainWindow, title: &str, label: &str) {
    open_menu(ui, title);
    choose(ui, label);
}

// ---------------------------------------------------------------------
// File: does each item do what its label says?
// ---------------------------------------------------------------------

#[test]
fn file_new_folder_creates_a_folder_and_offers_its_name() {
    let _serial = serially();
    let dir = scratch("file-new-folder");
    let (ui, app) = window_at(&dir);

    from_menu(&ui, "File", "New Folder");
    pump(&ui, &app);

    assert!(
        dir.join("New folder").is_dir(),
        "File > New Folder should have made a folder; the listing is {:?}",
        listing(&ui)
    );
    assert!(
        ui.get_content_prompt_text().starts_with("Rename to:"),
        "and dropped into rename so the name can be retyped; the prompt says {:?}",
        ui.get_content_prompt_text()
    );
}

#[test]
fn file_rename_opens_the_rename_prompt_for_the_selected_row() {
    let _serial = serially();
    let dir = scratch("file-rename");
    file(&dir, "notes.txt", "hello\n");
    let (ui, app) = window_at(&dir);
    click_row(&ui, row_of(&ui, "notes.txt"));
    pump(&ui, &app);

    from_menu(&ui, "File", "Rename");

    assert_eq!(
        ui.get_content_prompt_text(),
        "Rename to:  notes.txt",
        "File > Rename should open the rename box on the selected row"
    );
}

#[test]
fn file_delete_and_the_delete_key_arm_the_same_confirmation() {
    // The two are wired through different callbacks - `delete-requested`
    // from the menu, `delete-requested` from the key scope - so they are
    // free to drift. This asks whether they have.
    let _serial = serially();
    let dir = scratch("file-delete-pair");
    file(&dir, "doomed.txt", "x\n");
    let (ui, app) = window_at(&dir);
    click_row(&ui, row_of(&ui, "doomed.txt"));
    pump(&ui, &app);

    press_key(&ui, Key::Delete);
    let by_key = ui.get_content_prompt_text();
    let key_row = ui.get_content_prompt_row();
    press_key(&ui, Key::Escape);
    assert_eq!(ui.get_content_prompt_text(), "", "Escape clears the prompt");

    from_menu(&ui, "File", "Delete");

    assert_eq!(
        ui.get_content_prompt_text(),
        by_key,
        "File > Delete and the Delete key should arm the same confirmation"
    );
    assert_eq!(
        ui.get_content_prompt_row(),
        key_row,
        "and over the same row"
    );
    assert!(
        dir.join("doomed.txt").exists(),
        "neither deletes anything before it is confirmed"
    );
}

#[test]
fn file_open_on_a_file_does_something_or_says_why_not() {
    // Open was offered - not greyed - on whatever was selected. On a plain
    // file the reader got no navigation, no preview change and no word in
    // the status bar. An item that is enabled has to do something.
    //
    // Settled since: Open stays a navigation command and is drawn refused
    // on a file, which is the "says why not" half of this test's title.
    // Making it open the editor instead was tried and put back - the
    // pointer reaches Open by double-click, and two clicks in the same
    // place are easy to land by accident, so a file would have opened for
    // editing on a mis-click. A file is read in the File pane and edited
    // from its own Edit tab.
    let _serial = serially();
    let dir = scratch("file-open-file");
    file(&dir, "notes.txt", "hello\n");
    let (ui, app) = window_at(&dir);
    click_row(&ui, row_of(&ui, "notes.txt"));
    pump(&ui, &app);
    let before_path = ui.get_address_path().to_string();
    let before_status = ui.get_status_text().to_string();

    open_menu(&ui, "File");

    assert_eq!(
        item(&ui, "Open").accessible_enabled(),
        Some(false),
        "Open does nothing to a file, so it has to be drawn refused"
    );

    // Pressed directly rather than through `from_menu`, which opens the
    // menu itself and would toggle this one shut.
    item(&ui, "Open").mock_single_click(PointerEventButton::Left);
    pump(&ui, &app);

    assert_eq!(
        ui.get_address_path().to_string(),
        before_path,
        "and a refused Open navigates nowhere"
    );
    assert_eq!(
        ui.get_status_text().to_string(),
        before_status,
        "and says nothing"
    );
    assert!(
        !app.borrow().editing_file(),
        "and does not open the editor behind the reader's back"
    );
}

#[test]
fn file_save_refuses_when_no_file_is_open() {
    let _serial = serially();
    let dir = scratch("file-save-refused");
    file(&dir, "notes.txt", "hello\n");
    let (ui, app) = window_at(&dir);
    click_row(&ui, row_of(&ui, "notes.txt"));
    pump(&ui, &app);
    assert!(!ui.get_editing_file(), "nothing is open in the editor");

    open_menu(&ui, "File");
    let save = item(&ui, "Save");
    assert_eq!(
        save.accessible_enabled(),
        Some(false),
        "Save should be drawn greyed with no edit open"
    );
    save.mock_single_click(PointerEventButton::Left);
    pump(&ui, &app);

    assert_ne!(
        ui.get_status_text(),
        "saving...",
        "and the press should be refused, not merely drawn greyed"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("notes.txt")).expect("the file is still there"),
        "hello\n",
        "and nothing should have been written"
    );
}

#[test]
fn file_repos_directory_asks_where_the_repositories_are() {
    let _serial = serially();
    let dir = scratch("file-repos-root");
    let (ui, _app) = window_at(&dir);

    from_menu(&ui, "File", "Repos Directory...");

    assert!(
        ui.get_content_prompt_text()
            .starts_with("Where are your repositories?"),
        "File > Repos Directory... should open the Repos Directory prompt; \
         the pane says {:?}",
        ui.get_content_prompt_text()
    );
}

#[test]
fn help_about_names_the_application() {
    let _serial = serially();
    let dir = scratch("help-about");
    let (ui, _app) = window_at(&dir);

    from_menu(&ui, "Help", "About RepoSphereExplorer");

    assert!(
        ui.get_status_text().contains("Repos Explorer"),
        "Help > About should name the application; the status bar says {:?}",
        ui.get_status_text()
    );
}

// ---------------------------------------------------------------------
// Edit: the clipboard, and what it is enabled by.
// ---------------------------------------------------------------------

#[test]
fn edit_copy_puts_the_selected_file_on_the_clipboard() {
    let _serial = serially();
    let dir = scratch("edit-copy");
    file(
        &dir,
        "notes.txt",
        "hello
",
    );
    let (ui, app) = window_at(&dir);
    click_row(&ui, row_of(&ui, "notes.txt"));
    pump(&ui, &app);
    assert!(!ui.get_can_paste(), "the clipboard starts empty");

    from_menu(&ui, "Edit", "Copy");
    pump(&ui, &app);

    assert!(
        ui.get_can_paste(),
        "Edit > Copy should leave the file on the clipboard for a later paste"
    );
    assert_eq!(
        ui.get_status_text(),
        "notes.txt copied",
        "and say so, naming the file it took"
    );
    assert!(
        dir.join("notes.txt").is_file(),
        "and leave the file itself alone"
    );
}

#[test]
fn edit_cut_then_edit_paste_moves_the_file() {
    let _serial = serially();
    let dir = scratch("edit-cut-paste");
    std::fs::create_dir(dir.join("target")).expect("a target directory");
    file(
        &dir,
        "notes.txt",
        "hello
",
    );
    let (ui, app) = window_at(&dir);
    click_row(&ui, row_of(&ui, "notes.txt"));
    pump(&ui, &app);

    from_menu(&ui, "Edit", "Cut");
    pump(&ui, &app);
    assert!(
        ui.get_can_paste(),
        "Edit > Cut should leave the file on the clipboard; the status bar says {:?}",
        ui.get_status_text()
    );

    // Into the folder beside it, so the paste has somewhere unambiguous to
    // land.
    click_row(&ui, row_of(&ui, "target"));
    press_key(&ui, Key::Return);
    pump(&ui, &app);
    from_menu(&ui, "Edit", "Paste");
    pump(&ui, &app);

    assert!(
        dir.join("target").join("notes.txt").is_file(),
        "Edit > Cut then Edit > Paste should move the file into the folder pasted          into; the status bar says {:?} and the listing is {:?}",
        ui.get_status_text(),
        listing(&ui)
    );
    assert!(
        !dir.join("notes.txt").exists(),
        "and a cut takes the original with it"
    );
}

#[test]
fn edit_paste_refuses_with_an_empty_clipboard() {
    let _serial = serially();
    let dir = scratch("edit-paste-empty");
    file(&dir, "notes.txt", "hello\n");
    let (ui, app) = window_at(&dir);
    click_row(&ui, row_of(&ui, "notes.txt"));
    pump(&ui, &app);
    assert!(!ui.get_can_paste(), "the clipboard starts empty");

    open_menu(&ui, "Edit");
    let paste = item(&ui, "Paste");
    assert_eq!(
        paste.accessible_enabled(),
        Some(false),
        "Paste should be drawn greyed with an empty clipboard"
    );
    paste.mock_single_click(PointerEventButton::Left);
    pump(&ui, &app);

    assert_eq!(
        listing(&ui),
        vec!["notes.txt".to_owned()],
        "and the press should be refused, leaving the folder alone"
    );
}

#[test]
fn edit_cut_refuses_with_nothing_selected() {
    let _serial = serially();
    let dir = scratch("edit-cut-unselected");
    file(&dir, "notes.txt", "hello\n");
    let (ui, app) = window_at(&dir);
    // A control-click on the only selected row takes it back out, which is
    // how a reader arrives at a listing with nothing picked.
    click_row(&ui, 0.0);
    ctrl_click_row(&ui, 0.0);
    pump(&ui, &app);
    assert!(
        !ui.get_has_selection(),
        "control-clicking the only selected row leaves nothing selected"
    );

    open_menu(&ui, "Edit");
    let cut = item(&ui, "Cut");
    assert_eq!(
        cut.accessible_enabled(),
        Some(false),
        "Cut should be drawn greyed with nothing selected"
    );
    cut.mock_single_click(PointerEventButton::Left);
    pump(&ui, &app);

    assert!(
        !ui.get_can_paste(),
        "and the press should be refused, leaving the clipboard empty; \
         the status bar says {:?}",
        ui.get_status_text()
    );
}

#[test]
fn edit_select_all_selects_every_row() {
    let _serial = serially();
    let dir = scratch("edit-select-all");
    for name in ["a.txt", "b.txt", "c.txt"] {
        file(&dir, name, "x\n");
    }
    let (ui, app) = window_at(&dir);
    click_row(&ui, 0.0);
    pump(&ui, &app);

    from_menu(&ui, "Edit", "Select All");

    assert_eq!(
        selected(&ui).len(),
        listing(&ui).len(),
        "Edit > Select All should select the whole listing, which is {:?}",
        listing(&ui)
    );
}

// ---------------------------------------------------------------------
// View.
// ---------------------------------------------------------------------

#[test]
fn view_up_one_level_leaves_the_folder() {
    let _serial = serially();
    let dir = scratch("view-up");
    let inner = dir.join("inner");
    std::fs::create_dir_all(&inner).expect("a nested scratch directory");
    file(&inner, "notes.txt", "hello\n");
    let (ui, app) = window_at(&inner);

    from_menu(&ui, "View", "Up One Level");
    pump(&ui, &app);

    assert!(
        listing(&ui).iter().any(|name| name == "inner"),
        "View > Up One Level should show the parent, which holds `inner`; \
         the listing is {:?}",
        listing(&ui)
    );
}

#[test]
fn view_sort_by_size_puts_the_listing_in_size_order() {
    let _serial = serially();
    let dir = scratch("view-sort-size");
    file(&dir, "a-big.txt", &"x".repeat(300));
    file(&dir, "b-small.txt", "x");
    file(&dir, "c-middle.txt", &"x".repeat(60));
    let (ui, app) = window_at(&dir);

    from_menu(&ui, "View", "Sort by Size");
    pump(&ui, &app);

    assert_eq!(
        listing(&ui),
        vec![
            "b-small.txt".to_owned(),
            "c-middle.txt".to_owned(),
            "a-big.txt".to_owned(),
        ],
        "View > Sort by Size should order the listing by size, smallest first"
    );
}

#[test]
fn view_refresh_shows_a_file_that_appeared_behind_the_windows_back() {
    let _serial = serially();
    let dir = scratch("view-refresh");
    file(&dir, "first.txt", "x\n");
    let (ui, app) = window_at(&dir);
    assert_eq!(listing(&ui), vec!["first.txt".to_owned()]);

    file(&dir, "second.txt", "x\n");
    from_menu(&ui, "View", "Refresh");
    pump(&ui, &app);

    assert!(
        listing(&ui).iter().any(|name| name == "second.txt"),
        "View > Refresh should re-read the folder; the listing is {:?}",
        listing(&ui)
    );
}

// ---------------------------------------------------------------------
// The context menus.
// ---------------------------------------------------------------------

#[test]
fn the_empty_space_menu_offers_only_what_can_act_without_a_selection() {
    let _serial = serially();
    let dir = scratch("context-empty");
    file(&dir, "notes.txt", "hello\n");
    let (ui, app) = window_at(&dir);
    pump(&ui, &app);

    right_click_row(&ui, 6.0);

    assert_eq!(
        open_labels(&ui),
        vec!["New Folder".to_owned(), "New File".to_owned()],
        "right-clicking below the rows should offer only the two commands \
         that need no selection"
    );
}

#[test]
fn the_row_menu_offers_what_a_row_can_do() {
    let _serial = serially();
    let dir = scratch("context-row");
    file(&dir, "notes.txt", "hello\n");
    let (ui, app) = window_at(&dir);
    pump(&ui, &app);

    right_click_row(&ui, 0.0);

    assert_eq!(
        open_labels(&ui),
        vec![
            "Open".to_owned(),
            // Where the repository's page is hosted; refused on a row that
            // has none, which is why this one reads "on the web" (#534).
            "Open on the web".to_owned(),
            "Rename".to_owned(),
            // Two Copies, because there were always two things called
            // that. "Copy" is the clipboard, which is what the Ctrl+C
            // printed beside it means; "Copy to..." duplicates the file
            // here under a name you type. One item used to carry the
            // first's label and shortcut and do the second's work.
            "Copy".to_owned(),
            "Copy to...".to_owned(),
            "Delete".to_owned(),
            "Extract".to_owned(),
            // Handing the row to a program the user already has (#581),
            // greyed here since "notes.txt" is a file rather than a
            // folder.
            "Open terminal here".to_owned(),
            "Open in editor".to_owned(),
            "Copy path".to_owned(),
            "Copy remote address".to_owned(),
            "Show in Files".to_owned(),
        ],
        "the row menu should offer the row's own commands"
    );
    assert_eq!(
        selected(&ui),
        vec!["notes.txt".to_owned()],
        "and the right-click should have selected the row it landed on"
    );
}

#[test]
fn the_row_menus_copy_does_what_the_shortcut_beside_it_says() {
    // The item is labelled `Copy` and advertises `Ctrl+C`. Ctrl+C puts the
    // file on the clipboard for a later paste; if the menu item does
    // something else, the shortcut printed beside it is a lie.
    let _serial = serially();
    let dir = scratch("context-copy");
    file(&dir, "notes.txt", "hello\n");
    let (ui, app) = window_at(&dir);
    click_row(&ui, row_of(&ui, "notes.txt"));
    pump(&ui, &app);

    assert!(!ui.get_can_paste(), "the clipboard starts empty");

    right_click_row(&ui, row_of(&ui, "notes.txt"));
    choose(&ui, "Copy");
    pump(&ui, &app);

    assert!(
        ui.get_can_paste(),
        "the row menu's Copy advertises Ctrl+C beside it, but Ctrl+C puts the \
         file on the clipboard and this put nothing there; what it did instead \
         is show {:?} in the pane",
        ui.get_content_prompt_text()
    );
}

#[test]
fn control_c_is_the_shortcut_the_row_menu_advertises() {
    // The other half of the pair above: what the printed shortcut actually
    // does, so the comparison is against measured behaviour and not a guess.
    let _serial = serially();
    let dir = scratch("context-copy-shortcut");
    file(&dir, "notes.txt", "hello\n");
    let (ui, app) = window_at(&dir);
    click_row(&ui, row_of(&ui, "notes.txt"));
    pump(&ui, &app);

    press_with(&ui, "c", &[Key::Control]);

    assert!(
        ui.get_can_paste(),
        "Ctrl+C puts the selected file on the clipboard"
    );
    assert_eq!(
        ui.get_content_prompt_text(),
        "",
        "and asks the reader nothing"
    );
}

#[test]
fn the_row_menus_extract_refuses_on_something_that_is_not_an_archive() {
    let _serial = serially();
    let dir = scratch("context-extract-refused");
    file(&dir, "notes.txt", "hello\n");
    let (ui, app) = window_at(&dir);
    click_row(&ui, row_of(&ui, "notes.txt"));
    pump(&ui, &app);
    assert!(
        !ui.get_content_is_archive(),
        "a text file is not an archive"
    );

    right_click_row(&ui, row_of(&ui, "notes.txt"));
    let extract = item(&ui, "Extract");
    assert_eq!(
        extract.accessible_enabled(),
        Some(false),
        "Extract should be drawn greyed on something that is not an archive"
    );
    extract.mock_single_click(PointerEventButton::Left);
    pump(&ui, &app);

    assert_eq!(
        ui.get_content_prompt_text(),
        "",
        "and the press should be refused, not open the extract box"
    );
}

#[test]
fn pressing_a_greyed_menu_item_does_not_reach_the_listing_behind_it() {
    // The row menu is an ordinary element of the contents pane, drawn over
    // the same touch area that selects rows - and a disabled touch area is
    // transparent. So a press on a greyed item can fall straight through
    // the menu onto whatever row happens to be under it.
    let _serial = serially();
    let dir = scratch("context-greyed-fallthrough");
    for name in [
        "a.txt", "b.txt", "c.txt", "d.txt", "e.txt", "f.txt", "g.txt", "h.txt",
    ] {
        file(&dir, name, "x\n");
    }
    let (ui, app) = window_at(&dir);
    click_row(&ui, 0.0);
    pump(&ui, &app);
    assert_eq!(selected(&ui), vec!["a.txt".to_owned()]);

    right_click_row(&ui, 0.0);
    let extract = item(&ui, "Extract");
    assert_eq!(
        extract.accessible_enabled(),
        Some(false),
        "Extract is greyed on a text file"
    );
    extract.mock_single_click(PointerEventButton::Left);
    pump(&ui, &app);

    assert_eq!(
        selected(&ui),
        vec!["a.txt".to_owned()],
        "pressing a greyed menu item should do nothing at all; instead the press \
         went through the menu and landed on the listing behind it"
    );
}

// ---------------------------------------------------------------------
// A menu while a prompt is open.
// ---------------------------------------------------------------------

#[test]
fn a_menu_command_does_not_throw_away_an_open_delete_confirmation() {
    // `request_delete` refuses while another prompt is up, and says why:
    // a question on screen owns the keyboard. The menu can still reach the
    // other way round.
    let _serial = serially();
    let dir = scratch("menu-over-prompt");
    file(&dir, "doomed.txt", "x\n");
    let (ui, app) = window_at(&dir);
    click_row(&ui, row_of(&ui, "doomed.txt"));
    pump(&ui, &app);

    press_key(&ui, Key::Delete);
    let armed = ui.get_content_prompt_text().to_string();
    assert!(
        armed.starts_with("Delete "),
        "the confirmation is up: {armed:?}"
    );

    from_menu(&ui, "File", "Rename");

    assert_eq!(
        ui.get_content_prompt_text(),
        armed,
        "File > Rename replaced the delete confirmation the reader had not \
         answered yet; the question went without a yes or a no"
    );
}

// ---------------------------------------------------------------------
// A menu while the editor has a file open.
// ---------------------------------------------------------------------

/// A window with `notes.txt` open in the editor, opened the way a reader
/// opens it: select the row, then File > Edit.
fn editing(name: &str) -> (MainWindow, Rc<RefCell<App>>, PathBuf) {
    let dir = scratch(name);
    file(&dir, "notes.txt", "one\ntwo\n");
    file(&dir, "other.txt", "x\n");
    let (ui, app) = window_at(&dir);
    click_row(&ui, row_of(&ui, "notes.txt"));
    pump(&ui, &app);
    assert!(
        ui.get_can_edit(),
        "a text file is editable; the status bar says {:?}",
        ui.get_status_text()
    );
    from_menu(&ui, "File", "Edit");
    pump(&ui, &app);
    assert!(ui.get_editing_file(), "File > Edit should open the editor");
    assert_eq!(ui.get_focus_pane(), 2, "and give the File pane the focus");
    (ui, app, dir)
}

#[test]
fn file_edit_then_file_save_writes_the_file() {
    let _serial = serially();
    let (ui, app, dir) = editing("editor-save");

    assert!(ui.invoke_edit_key("X".into(), false, false));
    from_menu(&ui, "File", "Save");
    pump(&ui, &app);

    assert_eq!(
        std::fs::read_to_string(dir.join("notes.txt")).expect("the file is readable"),
        "Xone\ntwo\n",
        "File > Save should write what the editor is holding"
    );
}

#[test]
fn edit_undo_reaches_the_editor_while_a_file_is_open() {
    let _serial = serially();
    let (ui, app, _dir) = editing("editor-undo");

    assert!(ui.invoke_edit_key("X".into(), false, false));
    assert_eq!(app.borrow().edit_text(), "Xone\ntwo\n");

    from_menu(&ui, "Edit", "Undo");

    assert_eq!(
        app.borrow().edit_text(),
        "one\ntwo\n",
        "Edit > Undo should undo the typing, not reach past it into the filesystem"
    );
    assert_ne!(
        ui.get_status_text(),
        "undoing...",
        "and should not have asked the service to reverse an operation"
    );
}

#[test]
fn edit_cut_does_not_take_the_file_out_from_under_the_editor() {
    // The reader is typing. `Edit > Cut` in every Windows editor cuts the
    // selected text. Here the same item is still the Contents pane's, and
    // what it cuts is the file itself.
    let _serial = serially();
    let (ui, app, _dir) = editing("editor-cut");

    assert!(ui.invoke_edit_key("X".into(), false, false));
    from_menu(&ui, "Edit", "Cut");
    pump(&ui, &app);

    assert!(
        !ui.get_can_paste(),
        "Edit > Cut while a file is open in the editor put the file itself on the \
         clipboard, to be moved by the next paste; the status bar says {:?}",
        ui.get_status_text()
    );
}

#[test]
fn edit_select_all_does_not_take_the_keyboard_out_of_the_editor() {
    let _serial = serially();
    let (ui, app, _dir) = editing("editor-select-all");

    from_menu(&ui, "Edit", "Select All");
    pump(&ui, &app);

    assert_eq!(
        ui.get_focus_pane(),
        2,
        "Edit > Select All while a file is open selected every row in the Contents \
         pane and moved the keyboard there, out of the file being typed into; \
         the selection is now {:?}",
        selected(&ui)
    );
}

#[test]
fn file_delete_does_not_arm_a_question_the_editor_cannot_answer() {
    // With the editor focused, the window's key scope passes only Escape,
    // Ctrl+S, Ctrl+Z and F5; every other key goes into the document. So a
    // delete confirmation armed from the menu has no `y` and no `n` that
    // can reach it.
    let _serial = serially();
    let (ui, app, _dir) = editing("editor-delete");

    from_menu(&ui, "File", "Delete");
    let prompt = ui.get_content_prompt_text().to_string();

    // The keystroke that would answer it, delivered the way the editor
    // delivers one.
    ui.invoke_edit_key("y".into(), false, false);
    pump(&ui, &app);

    assert_eq!(
        prompt,
        "",
        "File > Delete armed {prompt:?} while the reader was typing, and the `y` \
         that would answer it went into the file instead: {:?}",
        app.borrow().edit_text()
    );
}

// ---- Open on the web (#534) ---------------------------------------------

/// A folder the directory plugin reads as a GitHub checkout: `HEAD` and a
/// `config` naming a remote, which is all it looks at. No `git` runs (rule
/// 8), and nothing here is a real repository anybody works in.
fn checkout(dir: &Path, name: &str) {
    let git = dir.join(name).join(".git");
    std::fs::create_dir_all(&git).expect("a checkout's git directory");
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");
    std::fs::write(
        git.join("config"),
        "[remote \"origin\"]\n\turl = git@github.com:owner/name.git\n",
    )
    .expect("config");
}

/// The row menu names where a repository's page is before the reader
/// clicks, and offers it enabled. Not clicked: that would open a browser on
/// the machine running the tests, and `App::open_on_the_web`'s own tests
/// prove what the click does with a launcher of their own.
#[test]
fn the_row_menu_offers_a_repository_on_the_host_it_came_from() {
    let _serial = serially();
    let dir = scratch("open-web-row-menu");
    checkout(&dir, "name");
    let (ui, app) = window_at(&dir);
    click_row(&ui, row_of(&ui, "name"));
    pump(&ui, &app);

    right_click_row(&ui, row_of(&ui, "name"));

    assert_eq!(
        item(&ui, "Open on github.com").accessible_enabled(),
        Some(true),
        "a GitHub checkout should be offered on github.com"
    );
}

/// Where there is nothing to open, both routes are drawn refused and a
/// press on them does nothing.
#[test]
fn open_on_the_web_is_refused_where_there_is_no_web_page() {
    let _serial = serially();
    let dir = scratch("open-web-refused");
    std::fs::create_dir_all(dir.join("plain")).expect("a plain folder");
    let (ui, app) = window_at(&dir);
    click_row(&ui, row_of(&ui, "plain"));
    pump(&ui, &app);
    let status = ui.get_status_text().to_string();

    right_click_row(&ui, row_of(&ui, "plain"));
    let row_item = item(&ui, "Open on the web");
    assert_eq!(row_item.accessible_enabled(), Some(false));
    row_item.mock_single_click(PointerEventButton::Left);
    pump(&ui, &app);
    assert_eq!(
        ui.get_status_text().to_string(),
        status,
        "a refused Open on the web in the row menu opens nothing and says nothing"
    );

    // A refused item keeps its menu open.
    close_context_menu(&ui, &app);
    click_row(&ui, row_of(&ui, "plain"));
    pump(&ui, &app);
    let status = ui.get_status_text().to_string();

    open_menu(&ui, "File");
    let menu_item = item(&ui, "Open on the web");
    assert_eq!(menu_item.accessible_enabled(), Some(false));
    menu_item.mock_single_click(PointerEventButton::Left);
    pump(&ui, &app);
    assert_eq!(
        ui.get_status_text().to_string(),
        status,
        "and the same from the File menu"
    );
}

// ---- Open a repository in the tools you work on it with (#581) ---------

/// The contents pane's row menu offers all five actions, enabled, for a
/// right-clicked working copy - what "choosing one hands the expected
/// command to the captured launcher" comes down to at the window: the
/// item is there and clickable. What clicking it then does is
/// `App::open_terminal_here` and its siblings' own job, proved with a
/// captured launcher of their own in `app.rs`'s unit tests; clicking a
/// real one here would open a real terminal on the machine running the
/// tests, the same reason `the_row_menu_offers_a_repository_on_the_host_it_came_from`
/// does not click "Open on the web".
#[test]
fn the_row_menu_offers_a_working_copy_every_way_to_open_it() {
    let _serial = serially();
    let dir = scratch("open-tools-row-menu");
    checkout(&dir, "name");
    let (ui, app) = window_at(&dir);
    click_row(&ui, row_of(&ui, "name"));
    pump(&ui, &app);

    right_click_row(&ui, row_of(&ui, "name"));

    for label in [
        "Open terminal here",
        "Copy path",
        "Copy remote address",
        "Show in Files",
    ] {
        assert_eq!(
            item(&ui, label).accessible_enabled(),
            Some(true),
            "{label} should be offered for a working copy"
        );
    }
}

/// "Copy remote address" is refused for a plain folder - nothing a working
/// copy's remote address could be copied from.
#[test]
fn copy_remote_address_is_disabled_for_a_plain_folder() {
    let _serial = serially();
    let dir = scratch("open-tools-plain-folder");
    std::fs::create_dir_all(dir.join("plain")).expect("a plain folder");
    let (ui, app) = window_at(&dir);
    click_row(&ui, row_of(&ui, "plain"));
    pump(&ui, &app);

    right_click_row(&ui, row_of(&ui, "plain"));

    assert_eq!(
        item(&ui, "Copy remote address").accessible_enabled(),
        Some(false),
        "a plain folder has no remote to copy"
    );
    for label in ["Open terminal here", "Copy path", "Show in Files"] {
        assert_eq!(
            item(&ui, label).accessible_enabled(),
            Some(true),
            "{label} should still be offered for a plain folder"
        );
    }
}

/// The folders pane's row menu offers the same five actions as the
/// contents pane's, for the row that was right-clicked: enabled for
/// "name", a checkout with a remote, except "Open in editor" - nothing is
/// configured for these tests to launch.
#[test]
fn the_folders_pane_row_menu_offers_what_a_row_can_do() {
    let _serial = serially();
    let dir = scratch("folder-row-menu");
    checkout(&dir, "name");
    let (ui, _app) = window_at(&dir);

    right_click_folder_row(&ui, folder_row_of(&ui, "name"));

    for (label, enabled) in [
        ("Open terminal here", true),
        ("Open in editor", false),
        ("Copy path", true),
        ("Copy remote address", true),
        ("Show in Files", true),
    ] {
        assert_eq!(
            item(&ui, label).accessible_enabled(),
            Some(enabled),
            "{label} should be {}",
            if enabled { "enabled" } else { "refused" }
        );
    }
}

/// A refused item in the folders pane's row menu does nothing when
/// clicked, the same as the contents pane's (rule: this file clicks a
/// refusal rather than trusting the property that draws it).
#[test]
fn a_refused_item_in_the_folders_pane_row_menu_does_nothing() {
    let _serial = serially();
    let dir = scratch("folder-row-menu-refused");
    std::fs::create_dir_all(dir.join("plain")).expect("a plain folder");
    let (ui, app) = window_at(&dir);

    right_click_folder_row(&ui, folder_row_of(&ui, "plain"));
    pump(&ui, &app);
    let status = ui.get_status_text().to_string();
    let row_item = item(&ui, "Open in editor");
    assert_eq!(row_item.accessible_enabled(), Some(false));
    row_item.mock_single_click(PointerEventButton::Left);
    pump(&ui, &app);

    assert_eq!(
        ui.get_status_text().to_string(),
        status,
        "a refused Open in editor opens nothing and says nothing"
    );
}

/// Dismisses whichever context menu is open, by clicking row 0 far to the
/// right of any menu's fixed 150px width - rather than several rows below
/// the menu, which #581's five new items made tall enough to cover: a
/// click meant to land on the pane underneath the menu instead landed on
/// one of the menu's own (greyed) items, which swallows a press without
/// dismissing anything.
fn close_context_menu(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    let origin = contents_area(ui).absolute_position();
    let position = LogicalPosition::new(origin.x + 400.0, origin.y + ROW_HEIGHT / 2.0);
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
    pump(ui, app);
}

/// Every item in a menu sits inside the menu's box.
///
/// The boxes were sized by hand, as an item count times the item height,
/// and the counts drifted: the row menu held six items in a box for five,
/// so Extract hung below its border. They now take their height from their
/// items, and this measures that it holds for the two menus that grew.
#[test]
fn every_menu_item_sits_inside_its_menus_box() {
    let _serial = serially();
    let dir = scratch("menu-boxes");
    file(&dir, "notes.txt", "x\n");
    let (ui, app) = window_at(&dir);
    click_row(&ui, row_of(&ui, "notes.txt"));
    pump(&ui, &app);

    let assert_contained = |ui: &MainWindow, layout_id: &str| {
        let layout = ElementHandle::find_by_element_id(ui, layout_id)
            .next()
            .unwrap_or_else(|| panic!("{layout_id} is drawn while its menu is open"));
        let bottom = layout.absolute_position().y + layout.size().height;
        for entry in open_items(ui) {
            let entry_bottom = entry.absolute_position().y + entry.size().height;
            assert!(
                entry_bottom <= bottom + 0.5,
                "{:?} ends at {entry_bottom} but its menu ends at {bottom}",
                entry.accessible_label()
            );
        }
    };

    right_click_row(&ui, row_of(&ui, "notes.txt"));
    assert_contained(&ui, "ContentsPane::row-menu-items");
    close_context_menu(&ui, &app);

    open_menu(&ui, "File");
    assert_contained(&ui, "MainWindow::file-menu-items");
}
