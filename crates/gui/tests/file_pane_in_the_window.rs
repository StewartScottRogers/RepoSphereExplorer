//! The File pane - its tab strip, and which view of which file it is
//! showing - driven through a real window on a real [`App`], against real
//! files in a scratch directory and the repository's own `samples/` tree.
//!
//! `file_views.rs` measures the strip with no `App` behind it: it proves a
//! click lands on the tab it was aimed at, and nothing about what that tab
//! then shows. The `app` module's own unit tests drive `App` with no window
//! in front of it: they prove `file_tabs` and `file_view_index` answer
//! correctly, and nothing about whether the pane is drawing the file the
//! reader just selected. This file is the seam between them - CLAUDE.md
//! rule 14 - and the questions it asks are the ones neither half can:
//! after a click in the Contents pane, is the pane showing *that* file,
//! on a tab that still exists, with the editor offered only where an
//! editor makes sense.
//!
//! Every test here builds the window through `gui::wire_callbacks`, the
//! same function `main` calls, and reaches it only through dispatched
//! pointer and keyboard events or through the callbacks the markup
//! invokes. Where a tab is clicked, the strip is measured with
//! [`ElementHandle`] rather than derived from a constant: a tab drawn off
//! the right-hand edge of the pane is exactly the defect a width constant
//! would hide.
//!
//! Nothing here writes outside its own scratch directory under the
//! platform's temporary directory; `samples/` is only ever read.

use gui::app::App;
use gui::{MainWindow, sync_ui};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

mod common;
use common::ensure_service;

/// Row height in `app.slint`'s contents pane, so a click can be aimed at a
/// row. The tab strip's width is deliberately *not* a constant - see
/// [`click_tab`].
const ROW_HEIGHT: f32 = 20.0;

/// The tab strip's height in `app.slint`. Only used to aim at the middle
/// of a strip whose position and width are measured.
const TAB_HEIGHT: f32 = 24.0;

/// Tests in this file share one service, so they run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

/// Takes the shared lock, tolerating a previous test having panicked while
/// holding it - a poisoned lock would otherwise turn one failure into many.
fn serially() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// An empty directory of this test's own under the platform's temporary
/// directory. Nothing in these tests ever writes outside one of these.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("rse-file-pane").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// The repository's own `samples/` tree, which these tests only read.
fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// Writes a file into `dir`.
fn file(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).expect("a scratch file");
}

/// A shown window on a real `App` rooted at `root`, wired as `main` wires
/// it, with the opening listing already loaded.
///
/// The Slint testing platform belongs to the thread that installed it, and
/// the test harness gives each test a thread of its own, so the guard is
/// per-thread rather than global.
fn window_at(root: &Path) -> (MainWindow, Rc<RefCell<App>>) {
    ensure_service();
    thread_local! {
        static PLATFORM: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    PLATFORM.with(|installed| {
        if !installed.replace(true) {
            i_slint_backend_testing::init_no_event_loop();
        }
    });
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
        // The status bar alone is not enough: a file preview is requested
        // without one, so watching it settled while the picture was still
        // arriving and the pane still held the file before it.
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

/// Clicks the contents pane `rows_down` rows below its first row.
fn click_row(ui: &MainWindow, rows_down: f32) {
    let pane = ElementHandle::find_by_element_id(ui, "ContentsPane::click-area")
        .next()
        .expect("the contents pane has a click area");
    let origin = pane.absolute_position();
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

/// Clicks the row `name` is drawn on, and waits for the pane to settle.
fn select(ui: &MainWindow, app: &Rc<RefCell<App>>, name: &str) {
    let drawn = listing(ui);
    let index = drawn
        .iter()
        .position(|row| row == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing: {drawn:?}"));
    let index = u16::try_from(index).expect("a small listing");
    click_row(ui, f32::from(index));
    pump(ui, app);
}

/// The tab labels the pane is drawing.
fn tabs(ui: &MainWindow) -> Vec<String> {
    ui.get_file_tabs()
        .iter()
        .map(|label| label.to_string())
        .collect()
}

/// The tab strip, or `None` where none is drawn.
///
/// The strip itself, not a tab's click area: each tab has one of those now,
/// and the first of them is one tab wide, which is not what a click is
/// measured against.
fn strip(ui: &MainWindow) -> Option<ElementHandle> {
    ElementHandle::find_by_element_id(ui, "MainWindow::tab-strip").next()
}

/// The File pane itself, which the strip shares its width with.
fn pane(ui: &MainWindow) -> ElementHandle {
    ElementHandle::find_by_element_id(ui, "MainWindow::file-pane")
        .next()
        .expect("the window has a File pane")
}

/// Clicks the middle of tab `index`, as it is actually drawn.
///
/// The strip is measured rather than assumed: the tabs share whatever
/// width the File pane has, so a constant here would let a test click a
/// tab that is off the edge of the pane and pass while the reader could
/// not reach it.
fn click_tab(ui: &MainWindow, index: usize) {
    let strip = strip(ui).expect("a strip should be drawn");
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
/// does a test asking what the pane says.
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

/// The runs the editing surface is drawing, which is where its colouring
/// shows.
fn editor_runs(ui: &MainWindow) -> Vec<Vec<String>> {
    ui.get_edit_lines()
        .iter()
        .map(|line| line.iter().map(|run| run.text.to_string()).collect())
        .collect()
}

// ---------------------------------------------------------------------
// The strip itself.
// ---------------------------------------------------------------------

/// A text file offers the plugin's two views and then the pane's own way
/// into the editor, on a strip that fits inside the pane.
///
/// The width is measured against the pane rather than against a number:
/// the Edit tab is the only sign the pane gives that an editor exists, and
/// it was once drawn past the right-hand edge of a default-width pane.
#[test]
fn a_text_file_offers_preview_text_and_edit_on_a_strip_that_fits_the_pane() {
    let _serial = serially();
    let dir = scratch("strip");
    file(&dir, "alpha.rs", "fn alpha() {}\n");
    let (ui, app) = window_at(&dir);

    select(&ui, &app, "alpha.rs");

    assert_eq!(
        tabs(&ui),
        vec!["Preview".to_owned(), "Text".to_owned(), "Edit".to_owned()],
        "a text file is the plugin's two views and then the editor"
    );
    let strip = strip(&ui).expect("three tabs should draw a strip");
    let pane = pane(&ui);
    assert!(
        strip.absolute_position().x >= pane.absolute_position().x
            && strip.absolute_position().x + strip.size().width
                <= pane.absolute_position().x + pane.size().width + 0.5,
        "the strip runs from {} to {} inside a pane that runs from {} to {}, \
         so a tab is drawn where nobody can click it",
        strip.absolute_position().x,
        strip.absolute_position().x + strip.size().width,
        pane.absolute_position().x,
        pane.absolute_position().x + pane.size().width
    );
}

/// The Text tab shows the file, and only the file - not the plugin's
/// summary of it.
#[test]
fn the_text_tab_shows_the_file_and_nothing_the_plugin_added() {
    let _serial = serially();
    let dir = scratch("text-tab");
    file(&dir, "alpha.rs", "fn alpha() {}\n");
    let (ui, app) = window_at(&dir);
    select(&ui, &app, "alpha.rs");

    assert!(
        shown(&ui).starts_with("functions: alpha"),
        "the Preview opens with the plugin's summary: {:?}",
        shown(&ui)
    );

    click_tab(&ui, 1);

    assert_eq!(ui.get_file_tab_index(), 1, "the Text tab is the active one");
    assert_eq!(
        shown(&ui),
        "fn alpha() {}",
        "the Text tab is the file itself, with nothing prepended"
    );
}

/// Clicking the last tab opens the editor on the file that is selected,
/// and the plugin's views go away while it is open.
#[test]
fn the_last_tab_opens_the_editor_and_takes_the_views_away() {
    let _serial = serially();
    let dir = scratch("edit-tab");
    file(&dir, "alpha.rs", "fn alpha() {}\n");
    let (ui, app) = window_at(&dir);
    select(&ui, &app, "alpha.rs");

    click_tab(&ui, 2);

    assert!(ui.get_editing_file(), "the Edit tab opens the editor");
    assert_eq!(
        tabs(&ui),
        vec!["Editing".to_owned()],
        "a stray click on a view would discard what somebody has typed"
    );
    assert!(
        strip(&ui).is_none(),
        "one tab is not a choice, so nothing is drawn to click"
    );
    assert_eq!(
        app.borrow().edit_text(),
        "fn alpha() {}\n",
        "the editor holds the file that was selected"
    );
}

/// Escape closes the editor and puts the reader back on the tab they left,
/// showing the same file.
#[test]
fn escape_puts_the_reader_back_on_the_tab_they_left() {
    let _serial = serially();
    let dir = scratch("back-out");
    file(&dir, "alpha.rs", "fn alpha() {}\n");
    let (ui, app) = window_at(&dir);
    select(&ui, &app, "alpha.rs");
    click_tab(&ui, 1);
    click_tab(&ui, 2);

    ui.invoke_cancel_requested();
    pump(&ui, &app);

    assert!(!ui.get_editing_file(), "Escape closes the editor");
    assert_eq!(
        tabs(&ui),
        vec!["Preview".to_owned(), "Text".to_owned(), "Edit".to_owned()],
        "and the views come back"
    );
    assert_eq!(
        ui.get_file_tab_index(),
        1,
        "on the tab the reader was reading when they opened the editor"
    );
    assert_eq!(shown(&ui), "fn alpha() {}", "showing the same file");
}

// ---------------------------------------------------------------------
// Which file the pane is showing.
// ---------------------------------------------------------------------

/// Selecting another file while a later tab is active shows the new file,
/// not the old one under a new name.
#[test]
fn selecting_another_file_from_the_text_tab_shows_the_new_file() {
    let _serial = serially();
    let dir = scratch("another-file");
    file(&dir, "alpha.rs", "fn alpha() {}\n");
    file(&dir, "zeta.rs", "fn zeta() {}\n");
    let (ui, app) = window_at(&dir);
    select(&ui, &app, "alpha.rs");
    click_tab(&ui, 1);

    select(&ui, &app, "zeta.rs");

    assert!(
        shown(&ui).contains("zeta"),
        "the pane should be showing the file that is selected: {:?}",
        shown(&ui)
    );
    assert!(
        !shown(&ui).contains("alpha"),
        "and not still the one before it: {:?}",
        shown(&ui)
    );
}

/// A file whose type offers fewer views must not leave the strip pointing
/// past its own end.
#[test]
fn a_type_with_fewer_views_does_not_leave_the_tab_index_past_the_strip() {
    let _serial = serially();
    let dir = scratch("fewer-views");
    file(&dir, "alpha.rs", "fn alpha() {}\n");
    std::fs::create_dir(dir.join("sub")).expect("a scratch folder");
    file(&dir.join("sub"), "inner.txt", "inner\n");
    let (ui, app) = window_at(&dir);
    select(&ui, &app, "alpha.rs");
    click_tab(&ui, 1);
    assert_eq!(ui.get_file_tab_index(), 1, "reading the Text tab");

    select(&ui, &app, "sub");

    let tabs = tabs(&ui);
    assert!(
        usize::try_from(ui.get_file_tab_index()).expect("a tab index") < tabs.len(),
        "tab {} of a strip holding {tabs:?} is a tab that is not there",
        ui.get_file_tab_index()
    );
    assert!(
        strip(&ui).is_none(),
        "a folder offers one view, so there is nothing to choose between"
    );
    assert!(
        shown(&ui).contains("entry") || shown(&ui).contains("entries"),
        "and the pane describes the folder: {:?}",
        shown(&ui)
    );
}

/// A folder is not something the pane offers to edit.
#[test]
fn a_folder_is_not_offered_to_the_editor() {
    let _serial = serially();
    let dir = scratch("folder-row");
    std::fs::create_dir(dir.join("sub")).expect("a scratch folder");
    let (ui, app) = window_at(&dir);

    select(&ui, &app, "sub");

    assert!(
        !tabs(&ui).iter().any(|tab| tab == "Edit"),
        "a folder has no text to edit, so no Edit tab: {:?}",
        tabs(&ui)
    );
    assert!(!ui.get_can_edit(), "and the command is greyed with it");
}

/// An empty folder selects nothing, and the pane says nothing rather than
/// keeping whatever was there last.
#[test]
fn an_empty_folder_leaves_the_pane_empty() {
    let _serial = serially();
    let dir = scratch("empty-folder");
    let (ui, _app) = window_at(&dir);

    assert!(listing(&ui).is_empty(), "nothing to select");
    assert!(tabs(&ui).is_empty(), "so no tabs: {:?}", tabs(&ui));
    assert!(strip(&ui).is_none(), "and no strip");
    assert_eq!(shown(&ui), "", "and nothing on screen");
}

/// An empty file is still a file: it has the type's views, and what it
/// shows is nothing - not the last file that was selected.
#[test]
fn an_empty_file_shows_nothing_rather_than_the_file_before_it() {
    let _serial = serially();
    let dir = scratch("empty-file");
    file(&dir, "alpha.rs", "fn alpha() {}\n");
    file(&dir, "empty.txt", "");
    let (ui, app) = window_at(&dir);
    select(&ui, &app, "alpha.rs");

    select(&ui, &app, "empty.txt");

    assert_eq!(
        shown(&ui),
        "",
        "an empty file has nothing to show, and what was there before is \
         another file"
    );
}

/// A file no plugin can read draws no strip and offers no editor; the pane
/// says why instead.
#[test]
fn a_file_the_plugins_cannot_read_draws_no_strip() {
    let _serial = serially();
    let dir = scratch("unreadable");
    file(&dir, "alpha.rs", "fn alpha() {}\n");
    std::fs::write(dir.join("weird.zzz"), [0xff_u8, 0xfe, 0x00, 0x01, 0x02])
        .expect("a scratch file");
    let (ui, app) = window_at(&dir);
    select(&ui, &app, "alpha.rs");

    select(&ui, &app, "weird.zzz");

    assert!(
        tabs(&ui).is_empty(),
        "there is nothing there to look at two ways: {:?}",
        tabs(&ui)
    );
    assert!(strip(&ui).is_none(), "so no strip is drawn");
    assert!(!ui.get_can_edit(), "and no editor is offered");
    assert!(
        !shown(&ui).is_empty() && !shown(&ui).contains("alpha"),
        "the pane says why rather than keeping the last file: {:?}",
        shown(&ui)
    );
}

/// A file too large to be held whole is readable but not editable: saving
/// a capped view back would discard the rest of it.
#[test]
fn a_file_too_large_to_hold_whole_is_read_but_not_edited() {
    let _serial = serially();
    let dir = scratch("too-large");
    file(&dir, "big.txt", &"x".repeat(70 * 1024));
    let (ui, app) = window_at(&dir);

    select(&ui, &app, "big.txt");

    assert_eq!(
        tabs(&ui),
        vec!["Preview".to_owned(), "Text".to_owned()],
        "read two ways, but not offered to an editor that could only save \
         part of it"
    );
    click_tab(&ui, 1);
    assert_eq!(
        ui.get_file_tab_index(),
        1,
        "and the two tabs it does have are clickable"
    );
}

/// A picture is drawn as a picture, and is not offered to the editor.
#[test]
fn a_picture_is_drawn_and_not_offered_to_the_editor() {
    let _serial = serially();
    let (ui, app) = window_at(&samples().join("image"));

    select(&ui, &app, "logo.png");

    assert!(ui.get_file_has_graphic(), "a picture reads as the picture");
    assert_eq!(
        tabs(&ui),
        vec!["Preview".to_owned()],
        "a picture carries no text, so there is one view and no editor"
    );
    assert!(strip(&ui).is_none(), "and so no strip");
}

/// A file deleted underneath the pane is replaced by whatever the folder
/// now holds, rather than left on screen.
#[test]
fn a_file_deleted_underneath_the_pane_is_not_left_on_screen() {
    let _serial = serially();
    let dir = scratch("deleted-underneath");
    file(&dir, "alpha.rs", "fn alpha() {}\n");
    file(&dir, "zeta.rs", "fn zeta() {}\n");
    let (ui, app) = window_at(&dir);
    select(&ui, &app, "zeta.rs");

    std::fs::remove_file(dir.join("zeta.rs")).expect("the fixture is removed");
    ui.invoke_refresh_requested();
    pump(&ui, &app);

    assert_eq!(
        listing(&ui),
        vec!["alpha.rs".to_owned()],
        "the listing drops what is gone"
    );
    assert!(
        !shown(&ui).contains("zeta"),
        "and the pane is not still showing it: {:?}",
        shown(&ui)
    );
}

// ---------------------------------------------------------------------
// Where the two halves disagree.
// ---------------------------------------------------------------------

/// Saving leaves the reader on the file they saved.
///
/// `save_file_edit` sends the write and `apply_operation_result` reloads
/// the folder behind it, but nothing tells the reload which entry to put
/// the selection back on - so `apply_contents_result` drops it to row 0.
/// Every other operation that reloads the folder sets `reselect` first;
/// `refresh` sets it precisely so F5 does not move anybody.
#[test]
fn saving_leaves_the_reader_on_the_file_they_saved() {
    let _serial = serially();
    let dir = scratch("save-selection");
    file(&dir, "alpha.rs", "fn alpha() {}\n");
    file(&dir, "zeta.rs", "fn zeta() {}\n");
    let (ui, app) = window_at(&dir);
    select(&ui, &app, "zeta.rs");
    click_tab(&ui, 2);
    ui.invoke_edit_key("X".into(), false, false);

    ui.invoke_save_requested();
    pump(&ui, &app);

    assert_eq!(
        std::fs::read_to_string(dir.join("zeta.rs")).expect("the file is on disk"),
        "Xfn zeta() {}\n",
        "the edit reached the file"
    );
    assert_eq!(
        selected(&ui),
        vec!["zeta.rs".to_owned()],
        "saving a file should not move the reader off it"
    );
    assert!(
        shown(&ui).contains("zeta"),
        "and the pane should go back to showing what was just saved: {:?}",
        shown(&ui)
    );
}

/// The open editor keeps the colouring of the file it is holding.
///
/// `App::edit_lines` asks `file_view_plugin` which plugin to classify with,
/// and that reads `file_view` - the *previewed* file - rather than the file
/// the editor was opened on. Selecting another row in the Contents pane
/// leaves the editor open (by design: the markup's `changed focus-pane`
/// handler exists for exactly that) but swaps the classifier underneath it.
#[test]
fn the_open_editor_keeps_the_colouring_of_the_file_it_holds() {
    let _serial = serially();
    let dir = scratch("editor-colouring");
    file(&dir, "alpha.rs", "fn alpha() { let mark = 1; }\n");
    file(&dir, "beta.json", "{\"mark\": 1}\n");
    let (ui, app) = window_at(&dir);
    select(&ui, &app, "alpha.rs");
    click_tab(&ui, 2);
    let before = editor_runs(&ui);
    assert!(
        !before.is_empty(),
        "the coloured surface is the one under test"
    );

    select(&ui, &app, "beta.json");

    assert!(
        ui.get_editing_file(),
        "the editor stays open on the file it was opened on"
    );
    assert_eq!(
        app.borrow().edit_text(),
        "fn alpha() { let mark = 1; }\n",
        "holding the same text"
    );
    assert_eq!(
        editor_runs(&ui),
        before,
        "the text did not change, so neither should the way it is coloured"
    );
}

/// The Text tab of a file that is also a picture shows the text.
///
/// `sync_ui` sets `file-has-graphic` from `App::file_graphic`, which never
/// looks at which view is selected, so the 220px picture band stays on
/// screen on every tab. An SVG is the case that has both: the Preview is
/// the drawing, and the Text tab is the markup - which is then read
/// through a pane the picture is still taking the top of.
#[test]
fn the_text_tab_of_a_picture_that_is_also_text_shows_the_text() {
    let _serial = serially();
    let (ui, app) = window_at(&samples().join("svg"));

    select(&ui, &app, "dashboard.svg");
    assert!(
        ui.get_file_has_graphic(),
        "the Preview of an svg is the drawing"
    );
    assert_eq!(
        tabs(&ui),
        vec!["Preview".to_owned(), "Text".to_owned(), "Edit".to_owned()],
        "and it is text as well, so it has a Text tab"
    );

    click_tab(&ui, 1);

    assert!(
        shown(&ui).contains("<svg"),
        "the Text tab is the markup: {:?}",
        shown(&ui).chars().take(60).collect::<String>()
    );
    assert!(
        !ui.get_file_has_graphic(),
        "the Text tab is where the file is read plainly, and the picture is \
         what the Preview is for"
    );
}

/// F5 re-reads the folder; it should not also change what the reader is
/// reading.
///
/// `refresh` sets `reselect` so the selection survives the reload, but the
/// reload ends in `load_file_view`, and every view that arrives goes
/// through `show_file_view`, which resets `file_view_index` to 0.
#[test]
fn refreshing_leaves_the_reader_on_the_tab_they_were_reading() {
    let _serial = serially();
    let dir = scratch("refresh-tab");
    file(&dir, "alpha.rs", "fn alpha() {}\n");
    let (ui, app) = window_at(&dir);
    select(&ui, &app, "alpha.rs");
    click_tab(&ui, 1);
    assert_eq!(ui.get_file_tab_index(), 1, "reading the Text tab");

    ui.invoke_refresh_requested();
    pump(&ui, &app);

    assert_eq!(
        selected(&ui),
        vec!["alpha.rs".to_owned()],
        "F5 keeps the selection, which is what `reselect` is for"
    );
    assert_eq!(
        ui.get_file_tab_index(),
        1,
        "and re-reading the same file should leave the reader on the same \
         view of it"
    );
}

/// Each tab is its own pointer target, tiled across the strip.
///
/// The strip used to be one click area with the tab worked out from the
/// pointer's distance along it. A tab that knows when the pointer is over
/// *it* is what lets the strip answer a reader who is only considering a
/// tab - the hover this pane had none of - and what this asserts is the
/// shape that makes that possible: one target per tab, side by side,
/// covering the pane between them.
#[test]
fn each_tab_is_its_own_pointer_target() {
    let _serial = serially();
    let dir = scratch("tab-targets");
    file(&dir, "notes.txt", "one\ntwo\n");
    let (ui, app) = window_at(&dir);
    select(&ui, &app, "notes.txt");

    let labels = tabs(&ui);
    assert!(labels.len() > 1, "a text file offers more than one view");
    let targets: Vec<ElementHandle> =
        ElementHandle::find_by_element_id(&ui, "MainWindow::tab-touch").collect();
    assert_eq!(
        targets.len(),
        labels.len(),
        "one target per tab, not one for the whole strip"
    );

    let strip = strip(&ui).expect("a strip should be drawn");
    let count = u16::try_from(labels.len()).expect("a handful of tabs");
    let width = strip.size().width / f32::from(count);
    for (index, target) in targets.iter().enumerate() {
        let index = u16::try_from(index).expect("a handful of tabs");
        let expected = f32::from(index).mul_add(width, strip.absolute_position().x);
        assert!(
            (target.absolute_position().x - expected).abs() < 1.0,
            "tab {index} should start at {expected}, not {}",
            target.absolute_position().x
        );
        assert!(
            (target.size().width - width).abs() < 1.0,
            "tab {index} should be {width} wide, not {}",
            target.size().width
        );
    }
}
