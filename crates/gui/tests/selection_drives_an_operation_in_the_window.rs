//! Use-case journey (#730): a selection made in one pane drives an
//! operation in another.
//!
//! The graphical front end has 41 test files and, before this one, not one
//! of them followed a reader's whole journey across the seam between the
//! Folders tree and the Contents pane's own operations. `shared_selection_in_the_window.rs`
//! proves only that clicking in Folders, then Contents, then the File pane
//! leaves every pane and `App::selection()` agreeing at rest; nothing then
//! *does* anything with that selection. This file is what CLAUDE.md rule 14
//! asks for: the seam between "the Folders tree says where a reader is" and
//! "an operation acts there" was never driven end to end.
//!
//! Full stack throughout: a real Repos Directory of real files on disk, a
//! real service on the private socket `common::ensure_service` provides,
//! and a real `MainWindow` joined to a real `App` by `gui::wire_callbacks` -
//! the function `main` calls, never a copy of it. Every step is a
//! dispatched pointer or keyboard event or a markup callback, the way a
//! reader reaches the application; nothing is measured that was not drawn.

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

/// Row height in `app.slint`'s contents pane, so a click can be aimed at a
/// row.
const ROW_HEIGHT: f32 = 20.0;

/// The File pane's tab strip height, so a click can be aimed at a tab
/// (`reading_and_editing_journey_in_the_window.rs` measures the same way).
const TAB_HEIGHT: f32 = 24.0;

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
    let dir = std::env::temp_dir()
        .join("rse-selection-drives-operation")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// Writes a file into `dir`.
fn file(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).expect("a scratch file");
}

/// Sets `path`'s modification time to two days ago, so a save's fresh
/// timestamp is guaranteed to fall on a different day - read as a
/// different Modified column - rather than depending on the column's
/// minute-level formatting happening to roll over mid-test
/// (`reading_and_editing_journey_in_the_window.rs`'s own
/// `set_mtime_days_ago` does the same).
fn set_mtime_days_ago(path: &Path, days_ago: u64) {
    let at = std::time::SystemTime::now() - Duration::from_secs(days_ago * 24 * 60 * 60);
    std::fs::File::options()
        .write(true)
        .open(path)
        .expect("the fixture file opens for its mtime to be set")
        .set_modified(at)
        .expect("the platform can set a file's modified time");
}

// ---------------------------------------------------------------------
// The harness, in the shape `operation_seams_in_the_window.rs` uses.
// ---------------------------------------------------------------------

/// A shown window on a real `App` rooted at `root`, wired by the crate's
/// own wiring - the same call `main` makes.
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
/// in-flight request has landed and stayed landed.
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

/// Control-clicks a contents row, which adds it to the selection.
fn ctrl_click_row(ui: &MainWindow, rows_down: f32) {
    click_row_with(ui, rows_down, &[Key::Control]);
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

/// The row index of `name` in the drawn listing, as a distance in rows.
fn row_of(ui: &MainWindow, name: &str) -> f32 {
    let index = listing(ui)
        .iter()
        .position(|drawn| drawn == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing: {:?}", listing(ui)));
    f32::from(u16::try_from(index).expect("a small listing"))
}

/// The size and modified columns the contents pane draws for the row named
/// `name`.
fn size_and_modified(ui: &MainWindow, name: &str) -> (String, String) {
    let row = ui
        .get_content_rows()
        .iter()
        .find(|row| row.name.trim_end_matches('/') == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing: {:?}", listing(ui)));
    (row.size.to_string(), row.modified.to_string())
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
/// rather than assumed.
fn click_tab(ui: &MainWindow, index: usize) {
    let strip = ElementHandle::find_by_element_id(ui, "FilePane::tab-strip")
        .next()
        .expect("a strip should be drawn");
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

/// The Folders tree row index whose name is `name`, the way a reader would
/// find it on screen - not assumed from build order, since the tree's own
/// flattening decides it.
fn folder_row_index(app: &Rc<RefCell<App>>, name: &str) -> i32 {
    let index = app
        .borrow()
        .folder_rows()
        .iter()
        .position(|row| row.name == name)
        .unwrap_or_else(|| panic!("{name} is not in the Folders tree"));
    i32::try_from(index).expect("a small tree")
}

/// The names the Folders tree is currently drawing, at whatever depth, the
/// way `operation_seams_in_the_window.rs`'s `tree_names` does.
fn tree_names(app: &Rc<RefCell<App>>) -> Vec<String> {
    app.borrow()
        .folder_rows()
        .iter()
        .map(|row| row.name.clone())
        .collect()
}

/// Every menu item currently on screen, in whichever menu is open.
fn open_items(ui: &MainWindow) -> Vec<ElementHandle> {
    ElementHandle::find_by_element_type_name(ui, "ContextMenuItem").collect()
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
        "exactly one open menu item should be labelled {label:?}"
    );
    found.remove(0)
}

/// Opens the menu-bar menu titled `title`, the topmost element carrying
/// that label.
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
    item(ui, label).mock_single_click(PointerEventButton::Left);
}

// ---------------------------------------------------------------------
// 1. Select a folder in the Folders tree, then rename it through the
//    application.
// ---------------------------------------------------------------------

/// A folder navigated to through the Folders tree - `docs` - holds a child
/// folder, `old-name`. Renaming `old-name` (selected the ordinary way, in
/// Contents) leaves the disk, the Contents listing and the Folders tree
/// itself all agreeing on the new name, with the selection sitting on it.
///
/// The Folders tree's own selection is what decides *which* directory
/// Contents and the rename act in - `App::selected_dir_path` reads the
/// tree's current node, not the Contents pane's own state - so reaching
/// `docs` by clicking its row in the tree, rather than double-clicking
/// through Contents, is the seam this journey exists to drive for real.
#[test]
fn renaming_a_folder_reached_through_the_tree_leaves_every_surface_agreeing() {
    let _serial = serially();
    let root = scratch("rename-via-tree");
    std::fs::create_dir_all(root.join("docs").join("old-name")).expect("nested fixture folders");
    let (ui, app) = window_at(&root);

    assert_eq!(listing(&ui), vec!["docs".to_owned()]);

    // Select "docs" in the Folders tree - not by opening it through
    // Contents - which is a real navigation into it.
    let docs_row = folder_row_index(&app, "docs");
    ui.invoke_folder_row_clicked(docs_row, 999.0);
    settle(&ui, &app);
    assert_eq!(
        listing(&ui),
        vec!["old-name".to_owned()],
        "the tree's own selection should have driven the Contents listing"
    );

    // Now the ordinary rename, on what Contents shows inside the folder the
    // tree navigated to.
    click_row(&ui, row_of(&ui, "old-name"));
    press_key(&ui, Key::F2);
    clear_prompt(&ui, "old-name".len());
    type_text(&ui, "reports");
    press_key(&ui, Key::Return);
    settle(&ui, &app);

    assert!(
        root.join("docs").join("reports").is_dir(),
        "the folder was renamed on disk"
    );
    assert!(
        !root.join("docs").join("old-name").exists(),
        "the old name is gone from disk"
    );
    assert_eq!(
        listing(&ui),
        vec!["reports".to_owned()],
        "the Contents listing should show the new name"
    );
    assert_eq!(
        selected(&ui),
        vec!["reports".to_owned()],
        "the selection should be on the renamed folder"
    );

    // Expand "docs" in the tree - a real chevron click, one indent in from
    // the root's own chevron column since "docs" is a depth-1 row
    // (`folders_in_the_window.rs`'s "child chevron" test measures the same
    // offset) - and the tree itself agrees too.
    ui.invoke_folder_row_clicked(docs_row, 28.0);
    assert!(
        tree_names(&app).contains(&"reports".to_owned()),
        "the Folders tree should show the new name: {:?}",
        tree_names(&app)
    );
    assert!(
        !tree_names(&app).contains(&"old-name".to_owned()),
        "and not the old one: {:?}",
        tree_names(&app)
    );
}

// ---------------------------------------------------------------------
// 2. Select in Folders, copy, select elsewhere in Folders, paste.
// ---------------------------------------------------------------------

/// Copy and paste, with both the source and the destination folder reached
/// purely by clicking their rows in the Folders tree - never by opening a
/// folder through Contents, which is the route `operations_in_the_window.rs`'s
/// own `copy_and_paste_moves_a_file_into_another_directory` already covers.
/// `App::paste_from_clipboard` writes into `selected_dir_path()`, which is
/// the tree's current node, so this proves that destination is honoured
/// when the tree - and only the tree - chose it.
#[test]
fn pasting_lands_wherever_the_tree_had_navigated_to() {
    let _serial = serially();
    let root = scratch("copy-paste-via-tree");
    std::fs::create_dir(root.join("source")).expect("a source folder");
    std::fs::create_dir(root.join("target")).expect("a target folder");
    file(&root.join("source"), "note.txt", "hello");
    let (ui, app) = window_at(&root);

    let source_row = folder_row_index(&app, "source");
    ui.invoke_folder_row_clicked(source_row, 999.0);
    settle(&ui, &app);
    assert_eq!(listing(&ui), vec!["note.txt".to_owned()]);

    click_row(&ui, row_of(&ui, "note.txt"));
    press_with(&ui, "c", &[Key::Control]);
    assert!(ui.get_can_paste(), "Ctrl+C should have put note.txt aside");

    let target_row = folder_row_index(&app, "target");
    ui.invoke_folder_row_clicked(target_row, 999.0);
    settle(&ui, &app);
    assert_eq!(listing(&ui), Vec::<String>::new(), "target starts empty");

    press_with(&ui, "v", &[Key::Control]);
    settle(&ui, &app);

    assert!(
        root.join("target").join("note.txt").is_file(),
        "the paste should have landed in the folder the tree had navigated \
         to, not somewhere Contents last happened to show"
    );
    assert!(
        root.join("source").join("note.txt").is_file(),
        "a copy leaves the original where it was"
    );
    assert_eq!(
        listing(&ui),
        vec!["note.txt".to_owned()],
        "the Contents listing shows it"
    );
    assert_eq!(
        selected(&ui),
        vec!["note.txt".to_owned()],
        "and it is selected in the folder it landed in"
    );
}

// ---------------------------------------------------------------------
// 3. Select several rows in Contents, then delete them from the menu.
// ---------------------------------------------------------------------

/// A multiple selection made in Contents, deleted through the menu bar
/// rather than the Delete key, with the confirmation naming how many.
///
/// The Folders pane's own row menu (`the_folders_pane_row_menu_offers_what_a_row_can_do`,
/// `menus_in_the_window.rs`) never offers Delete - only the "open this
/// folder in another program" commands `app.slint` documents as acting on
/// "whichever row was right-clicked there instead". Delete, like Open and
/// Rename, is one of the commands the File menu and the Contents pane's own
/// row menu share, and both always act on the Contents pane's selection
/// (same file, same comment) regardless of which pane last held the
/// keyboard. So the menu route this journey drives is File > Delete: the
/// one menu a multi-selection made in Contents can actually reach, proven
/// here with the confirmation naming the count rather than one name.
#[test]
fn deleting_a_multiple_selection_from_the_menu_names_the_count() {
    let _serial = serially();
    let root = scratch("multi-delete-via-menu");
    file(&root, "one.txt", "1");
    file(&root, "two.txt", "2");
    file(&root, "three.txt", "3");
    let (ui, app) = window_at(&root);

    click_row(&ui, row_of(&ui, "one.txt"));
    ctrl_click_row(&ui, row_of(&ui, "three.txt"));
    assert_eq!(
        selected(&ui).len(),
        2,
        "two rows should be selected: {:?}",
        selected(&ui)
    );

    from_menu(&ui, "File", "Delete");

    assert_eq!(
        ui.get_content_prompt_text().as_str(),
        "Delete 2 items?  (y / n)",
        "the confirmation should name how many, not one of them"
    );

    press(&ui, "y");
    settle(&ui, &app);

    assert!(!root.join("one.txt").exists(), "one.txt was deleted");
    assert!(!root.join("three.txt").exists(), "three.txt was deleted");
    assert!(
        root.join("two.txt").is_file(),
        "two.txt was never selected and must survive"
    );
}

// ---------------------------------------------------------------------
// 4. Select in the File pane's own surface, and the listing still agrees.
// ---------------------------------------------------------------------

/// `shared_selection_in_the_window.rs`'s one test walks Folders, then
/// Contents, then the File pane - and stops at switching which view the
/// File pane shows, never reaching into the editing surface itself.
/// `reading_and_editing_journey_in_the_window.rs`'s own flagship types at
/// the caret's default position, deliberately without a click first. This
/// is the third pane in that chain, completed: select the file through
/// Contents, then place the caret with a real click on the editing
/// surface itself, type there, save, and see the Contents row's own
/// columns and the File pane's re-read preview agree with exactly where
/// the click landed, not merely that something was typed.
#[test]
fn a_caret_placed_by_clicking_the_file_pane_lands_where_the_listing_can_see_it() {
    let _serial = serially();
    let root = scratch("caret-click-in-file-pane");
    file(&root, "demo.rs", "fn main() {\n    let x = 1;\n}\n");
    set_mtime_days_ago(&root.join("demo.rs"), 2);
    let (ui, app) = window_at(&root);

    click_row(&ui, row_of(&ui, "demo.rs"));
    settle(&ui, &app);
    let (size_before, modified_before) = size_and_modified(&ui, "demo.rs");

    click_tab(&ui, tabs(&ui).len() - 1);
    assert!(ui.get_editing_file(), "the last tab should open the editor");

    // Line 1, column 4: right after the four spaces of indentation, before
    // "let" - a real click on the surface, the same callback a pointer
    // press there dispatches to.
    ui.invoke_edit_pressed(1, 4);

    for c in ["Z", "Z"] {
        ui.window()
            .dispatch_event(WindowEvent::KeyPressed { text: c.into() });
        ui.window()
            .dispatch_event(WindowEvent::KeyReleased { text: c.into() });
    }
    assert_eq!(
        app.borrow().edit_text(),
        "fn main() {\n    ZZlet x = 1;\n}\n",
        "the typed letters should have landed exactly where the click put \
         the caret"
    );

    press_with(&ui, "s", &[Key::Control]);
    settle(&ui, &app);

    assert_eq!(
        std::fs::read_to_string(root.join("demo.rs")).expect("demo.rs is readable"),
        "fn main() {\n    ZZlet x = 1;\n}\n",
        "the edit should have reached the file at the clicked position"
    );

    let (size_after, modified_after) = size_and_modified(&ui, "demo.rs");
    assert_ne!(
        size_after, size_before,
        "the Contents row's Size column should reflect the saved file"
    );
    assert_ne!(
        modified_after, modified_before,
        "the Contents row's Modified column should reflect the saved file"
    );
    assert!(!ui.get_editing_file(), "saving should close the editor");
    assert!(
        shown(&ui).contains("ZZlet x = 1;"),
        "the File pane's preview should re-read the file rather than keep \
         showing something stale: {:?}",
        shown(&ui)
    );
}

// ---------------------------------------------------------------------
// 5. An operation on a selection that has since gone.
// ---------------------------------------------------------------------

/// The file behind a selection disappears - deleted by something other
/// than this application, with no refresh in between - and a rename
/// attempted on it is refused with a reason from the service, not a panic
/// and not a silent success.
#[test]
fn a_rename_on_a_selection_that_has_since_gone_is_refused_with_a_reason() {
    let _serial = serially();
    let root = scratch("stale-selection");
    file(&root, "ghost.txt", "x");
    let (ui, app) = window_at(&root);

    click_row(&ui, row_of(&ui, "ghost.txt"));
    settle(&ui, &app);
    // The status bar falls back to a summary of the folder whenever
    // nothing else has anything to say - "N item(s)" - so that alone is
    // not proof of a reason. Captured now, with nothing wrong yet, as the
    // one text a silently-swallowed failure would fall back to.
    let idle_status = ui.get_status_text().to_string();

    // Gone from under the application's feet - no delete through the UI,
    // no refresh, so the window's own idea of the world is now stale.
    std::fs::remove_file(root.join("ghost.txt")).expect("the file is removable");

    press_key(&ui, Key::F2);
    clear_prompt(&ui, "ghost.txt".len());
    type_text(&ui, "renamed.txt");
    press_key(&ui, Key::Return);
    settle(&ui, &app);

    assert_ne!(
        ui.get_status_text().to_string(),
        idle_status,
        "a rename of a file that has vanished should say why it failed, \
         not fall back to the same summary an untroubled folder shows"
    );
    assert!(
        !root.join("renamed.txt").exists(),
        "nothing should have been created from a source that does not exist"
    );
    assert!(
        !root.join("ghost.txt").exists(),
        "and nothing should have resurrected the original either"
    );
}
