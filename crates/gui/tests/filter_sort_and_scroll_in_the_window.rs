//! Filter, sort and scroll, combined (#727).
//!
//! The graphical front end's forty-odd test files, before #725's flagship
//! journey and #726's operation seams, never followed a reader's whole
//! journey - every other suite is one interaction or one hand-off from a
//! fresh scratch directory and a fresh window. That leaves the seams
//! between features uncovered, which CLAUDE.md rule 14 says is where the
//! faults live. This file crosses the two seams #727's own inventory
//! names:
//!
//! - **Filter, sort and the selection.** The filter tests never then click
//!   a column header, and the sort tests never have a filter on. Filter
//!   multiplied by sort multiplied by selection-persistence was
//!   uncovered, although each alone is well tested. Writing the first of
//!   these journeys (below) found a real gap: clearing a filter reset the
//!   selection to row zero of the restored listing rather than back to the
//!   entry the reader had - `App::reset_selection_after_filter` now
//!   re-finds it by name, the same way `App::sort_by_column` already did.
//! - **Scroll and an operation.**
//!   `operations_in_the_window.rs::a_rename_prompt_stays_on_the_row_it_names`
//!   and its delete sibling run on an unscrolled listing - and "the prompt
//!   was drawn over the wrong row" is exactly the defect that file's own
//!   doc comment names. This file runs the same two prompts on a listing
//!   that has to scroll to reach them, and follows a delete through to the
//!   listing it leaves behind.
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
/// row and a scroll offset can be turned into a row index.
const ROW_HEIGHT: f32 = 20.0;

/// The Slint testing backend is process-wide, and these tests share one
/// service, so they run one at a time - the guard `operations_in_the_window.rs`
/// and `operation_seams_in_the_window.rs` both take.
static SERIAL: Mutex<()> = Mutex::new(());

/// Takes the shared lock, tolerating a previous test having panicked while
/// holding it - a poisoned lock would otherwise turn one failure into many.
fn serially() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---------------------------------------------------------------------
// Fixtures: real files, inside a scratch directory of this file's own.
// ---------------------------------------------------------------------

/// An empty directory of this test's own under the platform's temporary
/// directory. Nothing in this file ever touches a path outside it.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("rse-filter-sort-scroll-journeys")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// Writes a file into `dir` holding `body`, so a fixture can also pin its
/// size for a sort by the Size column.
fn file(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).expect("a scratch file");
}

/// Sixty filler files that sort ahead of every other name this file uses -
/// `contents_in_the_window.rs` and `operation_seams_in_the_window.rs` both
/// use sixty to guarantee the pane has to scroll, since one file would let
/// everything fit and prove nothing.
fn scroll_filler(dir: &Path) {
    for index in 0..60 {
        file(
            dir,
            &format!("aa-{index:02}.txt"),
            &format!("content of aa-{index:02}"),
        );
    }
}

// ---------------------------------------------------------------------
// The harness, in the shape `operation_seams_in_the_window.rs` uses.
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

/// Where the pane's clickable body starts, in window coordinates.
fn click_origin(ui: &MainWindow) -> LogicalPosition {
    ElementHandle::find_by_element_id(ui, "ContentsPane::click-area")
        .next()
        .expect("the contents pane has a click area")
        .absolute_position()
}

/// Clicks the contents pane `rows_down` rows below its first row. Assumes
/// an unscrolled pane, the way `operations_in_the_window.rs`'s `click_row`
/// does; a row past what a pixel click can reach unscrolled is selected
/// through [`content_row_index`] and `content-row-clicked` instead.
fn click_row(ui: &MainWindow, rows_down: f32) {
    let origin = click_origin(ui);
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

/// The row index of `name` in the drawn listing, as a `u16` both
/// [`row_of`] (for pixel arithmetic) and [`content_row_index`] (for the
/// row's own callback) convert from.
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

/// The row index of `name` in the drawn listing, as `content-row-clicked`
/// and [`Self::prompt_row_is_visible`] want it - for a row past the bottom
/// of an unscrolled window, which a pixel click cannot reach.
fn content_row_index(ui: &MainWindow, name: &str) -> i32 {
    i32::from(content_row(ui, name))
}

/// Whether the row index `index` (as `content-prompt-row` or a selected
/// row's own index reports it) is currently scrolled into view - measured
/// from the pane's own scroll offset and viewport height, never assumed
/// from the index alone, per the doc comment on `contents_in_the_window.rs`'s
/// `rows_on_screen`.
fn row_index_is_visible(ui: &MainWindow, index: i32) -> bool {
    let Ok(index) = u16::try_from(index) else {
        return false;
    };
    let top = f32::from(index) * ROW_HEIGHT;
    let scroll = ui.get_content_scroll_y();
    let viewport = ui.get_content_viewport_height();
    top + scroll >= 0.0 && top + scroll < viewport
}

/// Whether the row named `name` is actually drawn on screen - a second,
/// independent measurement from [`row_index_is_visible`]'s arithmetic.
/// `contents_pane.slint` names its rows "so a test can ask which rows are
/// on screen: the element search returns the visible ones" - a row
/// scrolled out of the viewport is clipped out of the search entirely, so
/// finding it here at all is the proof, with no position math to get
/// wrong the way arithmetic against the scrolled click-area's own origin
/// would (`contents_in_the_window.rs`'s `rows_on_screen` warns against
/// exactly that).
fn row_is_drawn_on_screen(ui: &MainWindow, name: &str) -> bool {
    ElementHandle::find_by_element_id(ui, "ContentsPane::listing-row")
        .any(|row| row.accessible_label().as_deref() == Some(name))
}

// ---------------------------------------------------------------------
// 1 and 2. Filter, sort and the selection, in both orders.
// ---------------------------------------------------------------------

/// Four files whose names and byte sizes are chosen so that: filtering to
/// "a" drops exactly one of them (`boron.txt`, the only name without an
/// "a"), and sorting by size never agrees with sorting by name - so a test
/// that only checked "the same position" rather than "the same entry"
/// would be fooled twice over.
fn build_four_files(dir: &Path) {
    file(dir, "alpha.txt", "AAAA");
    file(dir, "boron.txt", "BBB");
    file(dir, "zap.txt", "Z");
    file(dir, "zebra.txt", "ZZ");
}

/// 1. Select a row, narrow the filter around it, sort by a column, and the
///    same *entry* is still selected - not the same position, since
///    filtering and then sorting moves it twice. Clear the filter and the
///    selection is still on it, with the sort still in force.
#[test]
fn narrowing_then_sorting_keeps_the_same_entry_selected() {
    let _serial = serially();
    let dir = scratch("narrow-then-sort");
    build_four_files(&dir);
    let (ui, _app) = window_at(&dir);

    assert_eq!(
        listing(&ui),
        vec!["alpha.txt", "boron.txt", "zap.txt", "zebra.txt"],
        "the opening listing should be sorted by name"
    );
    click_row(&ui, row_of(&ui, "zebra.txt"));
    assert_eq!(selected(&ui), vec!["zebra.txt".to_owned()]);

    // Narrow around it: "a" matches every name but boron.txt.
    ui.invoke_filter_focus_requested();
    type_text(&ui, "a");
    assert_eq!(
        listing(&ui),
        vec!["alpha.txt", "zap.txt", "zebra.txt"],
        "the filter should have dropped boron.txt, the only name without an a"
    );
    assert_eq!(
        selected(&ui),
        vec!["zebra.txt".to_owned()],
        "narrowing the filter should not move the selection off zebra.txt, \
         even though it is no longer the first row"
    );

    // Sort by Size: ascending, zap.txt (1 byte) sorts ahead of zebra.txt
    // (2 bytes) sorts ahead of alpha.txt (4 bytes) - a different position
    // for zebra.txt than the name-sorted filter left it at.
    ui.invoke_content_sort_requested(1);
    assert_eq!(
        listing(&ui),
        vec!["zap.txt", "zebra.txt", "alpha.txt"],
        "sorting by size should reorder the filtered listing"
    );
    assert_eq!(
        selected(&ui),
        vec!["zebra.txt".to_owned()],
        "the sort should keep the same entry selected across the reorder"
    );

    // Clear the filter: boron.txt is back, still sorted by size.
    ui.invoke_status_clear_filter_clicked();
    assert_eq!(
        listing(&ui),
        vec!["zap.txt", "zebra.txt", "boron.txt", "alpha.txt"],
        "clearing the filter should restore boron.txt without disturbing \
         the size sort - if it had reset to a name sort, this would read \
         alpha, boron, zap, zebra"
    );
    assert_eq!(
        selected(&ui),
        vec!["zebra.txt".to_owned()],
        "clearing the filter should leave the reader on the entry they \
         had selected, not reset to the first row of the restored listing"
    );
    assert_eq!(
        ui.get_content_sort_column(),
        1,
        "the sort should still be in force after the filter clears"
    );
}

/// 2. The other order, because a reader does both: sort first, then
///    narrow, and the same entry stays selected across both changes and
///    the filter clearing afterward.
#[test]
fn sorting_then_narrowing_keeps_the_same_entry_selected() {
    let _serial = serially();
    let dir = scratch("sort-then-narrow");
    build_four_files(&dir);
    let (ui, _app) = window_at(&dir);

    click_row(&ui, row_of(&ui, "zebra.txt"));
    assert_eq!(selected(&ui), vec!["zebra.txt".to_owned()]);

    ui.invoke_content_sort_requested(1);
    assert_eq!(
        listing(&ui),
        vec!["zap.txt", "zebra.txt", "boron.txt", "alpha.txt"],
        "sorting by size should reorder the whole listing"
    );
    assert_eq!(
        selected(&ui),
        vec!["zebra.txt".to_owned()],
        "sorting should keep the same entry selected"
    );

    ui.invoke_filter_focus_requested();
    type_text(&ui, "a");
    assert_eq!(
        listing(&ui),
        vec!["zap.txt", "zebra.txt", "alpha.txt"],
        "the filter should drop boron.txt without disturbing the size sort"
    );
    assert_eq!(
        selected(&ui),
        vec!["zebra.txt".to_owned()],
        "narrowing after a sort should keep the same entry selected too - \
         zebra.txt is not the first row of the filtered, size-sorted listing"
    );

    ui.invoke_status_clear_filter_clicked();
    assert_eq!(
        listing(&ui),
        vec!["zap.txt", "zebra.txt", "boron.txt", "alpha.txt"]
    );
    assert_eq!(
        selected(&ui),
        vec!["zebra.txt".to_owned()],
        "clearing the filter should still leave the reader on zebra.txt"
    );
    assert_eq!(ui.get_content_sort_column(), 1, "the sort should persist");
}

/// 3. A filter that matches nothing says so, offering a way to clear it
///    rather than the empty-directory message, and clearing it restores
///    the full listing with a selection on it again - not stuck on none.
///    Once every row is filtered out there is nothing left pointing at
///    which one the reader had; restoring the listing's own shape and
///    leaving a real selection on it is what this asserts, the same as
///    the model-level test `a_filter_matching_nothing_offers_a_way_to_clear_it_not_the_empty_message`.
#[test]
fn narrowing_to_nothing_says_so_and_clearing_it_restores_the_listing() {
    let _serial = serially();
    let dir = scratch("narrow-to-nothing");
    file(&dir, "one.txt", "1");
    file(&dir, "two.txt", "2");
    file(&dir, "three.txt", "3");
    let (ui, _app) = window_at(&dir);

    click_row(&ui, row_of(&ui, "two.txt"));
    assert_eq!(selected(&ui), vec!["two.txt".to_owned()]);

    ui.invoke_filter_focus_requested();
    type_text(&ui, "zzz");
    assert!(
        listing(&ui).is_empty(),
        "a filter matching nothing should empty the listing: {:?}",
        listing(&ui)
    );
    assert_eq!(
        ui.get_message_title().as_str(),
        "No name matches \"zzz\"",
        "the pane should say what did not match, not the empty-directory message"
    );
    assert!(
        ui.get_message_show_clear_filter(),
        "a way to clear the filter should be offered"
    );
    assert!(
        !ui.get_message_show_retry() && !ui.get_message_show_choose(),
        "retry and choose are for a broken Repos Directory, not a filter"
    );

    ui.invoke_status_clear_filter_clicked();
    assert_eq!(
        listing(&ui),
        vec![
            "one.txt".to_owned(),
            "three.txt".to_owned(),
            "two.txt".to_owned()
        ],
        "clearing the filter should restore every file"
    );
    assert_eq!(
        ui.get_message_title().as_str(),
        "",
        "the message should clear too"
    );
    assert_eq!(
        selected(&ui).len(),
        1,
        "clearing the filter should leave exactly one row selected, not none: {:?}",
        selected(&ui)
    );
}

// ---------------------------------------------------------------------
// 4. A rename prompt, and a delete confirmation, on a scrolled listing.
// ---------------------------------------------------------------------

/// Renames `rename-target.txt`, past the fold, and proves the prompt
/// stayed on its row throughout - one step of the journey below, split out
/// so the test that calls it, and its delete sibling, reads as the story
/// rather than tripping `clippy::too_many_lines`.
fn rename_a_scrolled_row_and_check_the_prompt_stayed_on_it(
    ui: &MainWindow,
    app: &Rc<RefCell<App>>,
    dir: &Path,
) {
    ui.invoke_content_row_clicked(content_row_index(ui, "rename-target.txt"));
    assert!(
        ui.get_content_scroll_y() < -ROW_HEIGHT,
        "selecting a row past sixty filler files should have scrolled to \
         it; scroll is {}",
        ui.get_content_scroll_y()
    );

    press_key(ui, Key::F2);
    let target_row = content_row_index(ui, "rename-target.txt");
    assert_eq!(
        ui.get_content_prompt_row(),
        target_row,
        "the prompt should open on rename-target.txt's own row, not row 0"
    );
    assert!(
        row_index_is_visible(ui, ui.get_content_prompt_row()),
        "the row the prompt names should be visible on screen; scroll {} \
         viewport {}",
        ui.get_content_scroll_y(),
        ui.get_content_viewport_height()
    );
    assert!(
        row_is_drawn_on_screen(ui, "rename-target.txt"),
        "rename-target.txt's own row should be drawn inside the viewport, \
         measured from the element Slint drew"
    );

    // The prompt is an overlay on one row, not a modal dialog: a different
    // row past the fold is still clickable underneath it.
    ui.invoke_content_row_clicked(content_row_index(ui, "rename-bystander.txt"));
    assert_eq!(
        ui.get_content_prompt_text().as_str(),
        "Rename to:  rename-target.txt",
        "still renaming rename-target.txt, not whatever was just clicked"
    );
    let drawn_over = ui.get_content_prompt_row();
    assert_eq!(
        drawn_over,
        content_row_index(ui, "rename-target.txt"),
        "the prompt naming rename-target.txt should still be drawn on its \
         row, not the row that was just clicked underneath it"
    );

    clear_prompt(ui, "rename-target.txt".len());
    type_text(ui, "renamed.txt");
    press_key(ui, Key::Return);
    settle(ui, app);

    assert!(
        dir.join("renamed.txt").is_file(),
        "rename-target.txt was renamed"
    );
    assert!(
        dir.join("rename-bystander.txt").is_file(),
        "the bystander clicked underneath the prompt should be untouched"
    );
}

/// Deletes `delete-target.txt`, past the fold, and proves the confirmation
/// stayed on its row throughout - the delete sibling of the step above.
fn delete_a_scrolled_row_and_check_the_confirmation_stayed_on_it(
    ui: &MainWindow,
    app: &Rc<RefCell<App>>,
    dir: &Path,
) {
    ui.invoke_content_row_clicked(content_row_index(ui, "delete-target.txt"));
    assert!(
        ui.get_content_scroll_y() < -ROW_HEIGHT,
        "delete-target.txt is also past the fold"
    );

    press_key(ui, Key::Delete);
    let target_row = content_row_index(ui, "delete-target.txt");
    assert_eq!(
        ui.get_content_prompt_text().as_str(),
        "Delete delete-target.txt?  (y / n)"
    );
    assert_eq!(
        ui.get_content_prompt_row(),
        target_row,
        "the confirmation should open on delete-target.txt's own row"
    );
    assert!(
        row_index_is_visible(ui, ui.get_content_prompt_row()),
        "the row the confirmation names should be visible on screen"
    );
    assert!(row_is_drawn_on_screen(ui, "delete-target.txt"));

    ui.invoke_content_row_clicked(content_row_index(ui, "delete-bystander.txt"));
    assert_eq!(
        ui.get_content_prompt_text().as_str(),
        "Delete delete-target.txt?  (y / n)",
        "still confirming delete-target.txt"
    );
    let drawn_over = ui.get_content_prompt_row();
    assert_eq!(
        drawn_over,
        content_row_index(ui, "delete-target.txt"),
        "the confirmation naming delete-target.txt should still be drawn \
         on its row, not the bystander's"
    );

    press(ui, "y");
    settle(ui, app);

    assert!(
        !dir.join("delete-target.txt").exists(),
        "delete-target.txt should have been deleted"
    );
    assert!(
        dir.join("delete-bystander.txt").is_file(),
        "the bystander clicked underneath the confirmation should survive"
    );
}

/// 4. A rename prompt on a scrolled listing names, and is drawn over, the
///    row the reader is actually on - measured, not assumed - the same as
///    `operations_in_the_window.rs::a_rename_prompt_stays_on_the_row_it_names`,
///    but with the target past the fold. The same for a delete
///    confirmation.
#[test]
fn a_rename_prompt_and_a_delete_confirmation_stay_on_the_row_they_name_when_scrolled() {
    let _serial = serially();
    let dir = scratch("prompts-scrolled");
    scroll_filler(&dir);
    file(&dir, "rename-target.txt", "r");
    file(&dir, "rename-bystander.txt", "r");
    file(&dir, "delete-target.txt", "d");
    file(&dir, "delete-bystander.txt", "d");
    let (ui, app) = window_at(&dir);

    rename_a_scrolled_row_and_check_the_prompt_stayed_on_it(&ui, &app, &dir);
    delete_a_scrolled_row_and_check_the_confirmation_stayed_on_it(&ui, &app, &dir);
}

// ---------------------------------------------------------------------
// 5. An operation on a scrolled listing, and the listing afterwards.
// ---------------------------------------------------------------------

/// 5. An operation on a scrolled listing acts on the selected row, and the
///    listing afterwards still shows an accurate, visible state: the
///    deleted row is gone, and whatever is selected next is drawn where
///    the reader can actually see it, not left scrolled to a position
///    that no longer means anything.
#[test]
fn deleting_a_row_on_a_scrolled_listing_leaves_the_listing_accurate_afterwards() {
    let _serial = serially();
    let dir = scratch("operation-scrolled");
    scroll_filler(&dir);
    file(&dir, "keep-a.txt", "a");
    file(&dir, "target.txt", "t");
    file(&dir, "keep-b.txt", "b");
    let (ui, app) = window_at(&dir);

    ui.invoke_content_row_clicked(content_row_index(&ui, "target.txt"));
    assert!(
        ui.get_content_scroll_y() < -ROW_HEIGHT,
        "target.txt is past sixty filler files, so selecting it should scroll"
    );

    press_key(&ui, Key::Delete);
    press(&ui, "y");
    settle(&ui, &app);

    assert!(!dir.join("target.txt").exists(), "target.txt was deleted");
    assert!(
        !listing(&ui).contains(&"target.txt".to_owned()),
        "and dropped from the listing: {:?}",
        listing(&ui)
    );

    let after = selected(&ui);
    assert_eq!(
        after.len(),
        1,
        "exactly one row should be selected after the delete, not none: {after:?}"
    );
    assert!(
        listing(&ui).contains(&after[0]),
        "the selection has to be on a row that still exists: {after:?} vs {:?}",
        listing(&ui)
    );

    let index = ui
        .get_content_rows()
        .iter()
        .position(|row| row.selected)
        .expect("a selected row is drawn");
    let index = i32::try_from(index).expect("a small listing");
    assert!(
        row_index_is_visible(&ui, index),
        "the listing afterwards should still show the current selection, \
         scrolled into view rather than left at a stale offset; scroll {} \
         viewport {}",
        ui.get_content_scroll_y(),
        ui.get_content_viewport_height()
    );
    assert!(
        row_is_drawn_on_screen(&ui, &after[0]),
        "the selected row should be drawn inside the viewport, measured \
         from the element Slint drew"
    );
}
