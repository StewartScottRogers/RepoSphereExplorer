//! Use-case test (#731): zoom, and then use the application at that zoom.
//!
//! `zoom_in_the_window.rs` measures what zoom changes - row height, text
//! size, pane widths - and stops there. Nothing zooms and then clicks a
//! row, so hit mapping at a row height other than the default is unproven;
//! and nothing zooms with the editor open, so the caret's column
//! arithmetic at a scaled row height is unproven either. That is the same
//! shape of fault as #586, where the scroll arithmetic had to be taught to
//! agree with what is drawn at the current zoom rather than with the
//! hundred-percent row height - CLAUDE.md rule 14.
//!
//! Full stack throughout: a real Repos Directory of real files on disk, a
//! real service on the private socket `common::ensure_service` provides,
//! and a real `MainWindow` joined to a real `App` by `gui::wire_callbacks`,
//! the function `main` calls, never a copy of it. Every step is a
//! dispatched pointer or keyboard event or a markup callback; every row
//! clicked is found by asking the window what it actually drew, never by
//! assuming the hundred-percent row height or the un-scrolled position.

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

/// The tab strip's height in `app.slint`, for aiming at the middle of a
/// strip whose position and width are measured, never assumed.
const TAB_HEIGHT: f32 = 24.0;

/// The Slint testing backend is a process-wide platform, so this suite's
/// tests take the same guard `operations_in_the_window.rs` does.
static SERIAL: Mutex<()> = Mutex::new(());

/// Takes the shared lock, tolerating a previous test having panicked while
/// holding it - a poisoned lock would otherwise turn one failure into many.
fn serially() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The one directory under which every fixture in this file is built.
fn scratch_root() -> PathBuf {
    std::env::temp_dir().join("rse-zoom-then-use")
}

/// An empty directory of this test's own. Nothing in this file ever
/// touches a path outside [`scratch_root`].
fn scratch(name: &str) -> PathBuf {
    let dir = scratch_root().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// Writes a file into `dir`.
fn write_file(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).expect("a scratch file");
}

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

/// Steps the real zoom commands - the same callbacks Ctrl+Plus/Minus and
/// the View menu fire - until `App` reports `target`, which must be one of
/// `gui::zoom::STEPS` or this never returns.
fn zoom_to(ui: &MainWindow, app: &Rc<RefCell<App>>, target: u16) {
    while app.borrow().zoom_percent() < target {
        ui.invoke_zoom_in_requested();
    }
    while app.borrow().zoom_percent() > target {
        ui.invoke_zoom_out_requested();
    }
    assert_eq!(app.borrow().zoom_percent(), target);
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

/// Backspaces `characters` times, to clear a rename prompt's pre-filled
/// name before typing a new one.
fn clear_prompt(ui: &MainWindow, characters: usize) {
    for _ in 0..characters {
        press_key(ui, Key::Backspace);
    }
}

/// The names the contents pane is drawing, in the order it draws them.
fn listing(ui: &MainWindow) -> Vec<String> {
    ui.get_content_rows()
        .iter()
        .map(|row| row.name.to_string())
        .collect()
}

/// The names the contents pane is drawing as selected.
fn selected(ui: &MainWindow) -> Vec<String> {
    ui.get_content_rows()
        .iter()
        .filter(|row| row.selected)
        .map(|row| row.name.to_string())
        .collect()
}

/// Where `name`'s row is actually drawn right now - at whatever zoom and
/// whatever scroll offset are currently in effect - so a click can be aimed
/// at the row it looks like it is on rather than at a position computed
/// from the hundred-percent row height or an assumed lack of scrolling.
fn drawn_row_middle(ui: &MainWindow, name: &str) -> LogicalPosition {
    let row = ElementHandle::find_by_element_id(ui, "ContentsPane::listing-row")
        .find(|row| {
            row.accessible_label()
                .is_some_and(|label| label.as_str() == name)
        })
        .unwrap_or_else(|| panic!("{name} is not drawn on screen: {:?}", listing(ui)));
    let at = row.absolute_position();
    LogicalPosition::new(at.x + 20.0, at.y + row.size().height / 2.0)
}

/// The name of a row solidly inside the pane's visible band right now -
/// measured, since which rows are on screen at all depends on the scroll
/// offset. The middle of what is visible, not the first or last: a row
/// whose edge merely touches the fold is still found by the element
/// search, and a click aimed at its middle can land a pixel short of the
/// pane it belongs to (`scroll_into_view.rs` names the same trap).
fn a_row_safely_on_screen(ui: &MainWindow) -> String {
    let mut visible: Vec<(String, f32)> =
        ElementHandle::find_by_element_id(ui, "ContentsPane::listing-row")
            .filter_map(|row| {
                Some((
                    row.accessible_label()?.to_string(),
                    row.absolute_position().y,
                ))
            })
            .collect();
    visible.sort_by(|left, right| left.1.total_cmp(&right.1));
    visible
        .get(visible.len() / 2)
        .map(|(name, _)| name.clone())
        .expect("a row should be drawn")
}

/// Clicks at `position`, the way a pointer does.
fn click_at(ui: &MainWindow, position: LogicalPosition) {
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

/// Clicks the row named `name`, wherever it is actually drawn.
fn click_named_row(ui: &MainWindow, name: &str) {
    click_at(ui, drawn_row_middle(ui, name));
}

/// The tab labels the File pane is drawing.
fn tabs(ui: &MainWindow) -> Vec<String> {
    ui.get_file_tabs()
        .iter()
        .map(|label| label.to_string())
        .collect()
}

/// The tab strip, measured rather than assumed.
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
    click_at(ui, position);
}

/// Where the editing surface starts on screen.
fn surface_origin(ui: &MainWindow) -> LogicalPosition {
    let body = ElementHandle::find_by_element_id(ui, "CodeEditor::body")
        .next()
        .expect("the editing surface is drawn");
    let at = body.absolute_position();
    LogicalPosition::new(at.x, at.y)
}

// ---------------------------------------------------------------------
// Journey 1: zoom in, click the row under the pointer; zoom out, do it
// again.
// ---------------------------------------------------------------------

#[test]
fn a_click_at_a_non_default_zoom_selects_the_row_drawn_under_the_pointer() {
    let _serial = serially();
    let dir = scratch("click-at-zoom");
    write_file(&dir, "alpha.txt", "a");
    write_file(&dir, "beta.txt", "b");
    write_file(&dir, "gamma.txt", "g");
    write_file(&dir, "delta.txt", "d");
    let (ui, app) = window_at(&dir);

    // Zoomed in, a click aimed at the row the pane actually drew - not at
    // `index * 20px` - has to select that row.
    zoom_to(&ui, &app, 200);
    click_named_row(&ui, "gamma.txt");
    assert_eq!(
        selected(&ui),
        vec!["gamma.txt".to_owned()],
        "at 200% zoom a click on gamma.txt's drawn row should select it"
    );

    // And the same the other way: zoomed out, a shorter row, a different
    // target.
    zoom_to(&ui, &app, 80);
    click_named_row(&ui, "beta.txt");
    assert_eq!(
        selected(&ui),
        vec!["beta.txt".to_owned()],
        "at 80% zoom a click on beta.txt's drawn row should select it"
    );
}

// ---------------------------------------------------------------------
// Journey 2: zoom, then scroll, then click - the two corrections compose.
// ---------------------------------------------------------------------

#[test]
fn zoom_then_scroll_then_click_compose() {
    let _serial = serially();
    let dir = scratch("zoom-scroll-click");
    let names: Vec<String> = (0..30)
        .map(|index| format!("file-{index:02}.txt"))
        .collect();
    for name in &names {
        write_file(&dir, name, "x");
    }
    let (ui, app) = window_at(&dir);

    zoom_to(&ui, &app, 150);

    // A click gives the listing the keyboard, the way a reader's first
    // click on it does, before arrowing down past the fold - real key
    // events, not a jump to an index. #662's cautionary tale is exactly
    // this: a scroll that agrees with the wrong row height stops moving
    // the listing a full row short.
    click_named_row(&ui, &names[0]);
    for _ in 0..25 {
        press_key(&ui, Key::DownArrow);
    }
    assert_eq!(
        selected(&ui),
        vec!["file-25.txt".to_owned()],
        "twenty-five Down presses from the top should land on file-25.txt"
    );
    assert!(
        ElementHandle::find_by_element_id(&ui, "ContentsPane::listing-row").any(|row| row
            .accessible_label()
            .is_some_and(|label| label.as_str() == "file-25.txt")),
        "the selected row should have been scrolled into view at the zoomed row height"
    );

    // Whichever row the scroll actually put on screen, a click there has
    // to select that row - the zoomed row height and the scroll offset
    // composed correctly, not fought each other.
    let on_screen = a_row_safely_on_screen(&ui);
    click_named_row(&ui, &on_screen);
    assert_eq!(
        selected(&ui),
        vec![on_screen],
        "a click on a row drawn on screen should select it, after zooming and scrolling"
    );
}

// ---------------------------------------------------------------------
// Journey 3: zoom with the editor open.
// ---------------------------------------------------------------------

#[test]
fn zoom_with_the_editor_open_keeps_the_caret_and_maps_clicks_to_characters() {
    let _serial = serially();
    let dir = scratch("editor-zoom");
    write_file(
        &dir,
        "notes.txt",
        "abcdefghijklmnopqrstuvwxyz\nsecond line\n",
    );
    let (ui, app) = window_at(&dir);

    click_named_row(&ui, "notes.txt");
    settle(&ui, &app);
    click_tab(&ui, tabs(&ui).len() - 1);
    assert!(ui.get_editing_file(), "the last tab should open the editor");

    // A click well into the first line, at the default zoom.
    let origin = surface_origin(&ui);
    click_at(&ui, LogicalPosition::new(origin.x + 120.0, origin.y + 8.0));
    let caret_at_100 = app.borrow().edit_caret();
    assert_eq!(caret_at_100.0, 0, "the click landed on the first line");
    assert!(
        caret_at_100.1 > 0,
        "a click 120px in should reach a column past the first; reached {}",
        caret_at_100.1
    );

    // Zooming must not move a caret the reader already placed.
    zoom_to(&ui, &app, 200);
    assert_eq!(
        app.borrow().edit_caret(),
        caret_at_100,
        "zooming should not move the caret"
    );

    // Each character is wider now, so the same pixel offset from the
    // (possibly moved) surface origin reaches an earlier column - proving
    // the click-to-column arithmetic tracks the zoomed cell width rather
    // than the hundred-percent one.
    let origin = surface_origin(&ui);
    click_at(&ui, LogicalPosition::new(origin.x + 120.0, origin.y + 8.0));
    let caret_at_200 = app.borrow().edit_caret();
    assert_eq!(
        caret_at_200.0, 0,
        "the click still landed on the first line"
    );
    assert!(
        caret_at_200.1 > 0 && caret_at_200.1 < caret_at_100.1,
        "at 200% zoom the same 120px offset should reach an earlier column \
         than at 100%: 100% reached {}, 200% reached {}",
        caret_at_100.1,
        caret_at_200.1
    );

    // And typing there inserts exactly where the zoomed click put the
    // caret.
    press(&ui, "Z");
    let text = app.borrow().edit_text();
    let first_line = text.lines().next().expect("a first line");
    assert_eq!(
        first_line.chars().nth(caret_at_200.1),
        Some('Z'),
        "the letter should have landed where the zoomed click put the caret; \
         first line reads {first_line:?}"
    );
}

// ---------------------------------------------------------------------
// Journey 4: zoom, then run an operation.
// ---------------------------------------------------------------------

#[test]
fn zoom_then_rename_operates_on_the_row_the_zoomed_click_selected() {
    let _serial = serially();
    let dir = scratch("zoom-rename");
    write_file(&dir, "alpha.txt", "a");
    write_file(&dir, "beta.txt", "b");
    write_file(&dir, "gamma.txt", "g");
    let (ui, app) = window_at(&dir);

    zoom_to(&ui, &app, 150);
    click_named_row(&ui, "beta.txt");
    assert_eq!(selected(&ui), vec!["beta.txt".to_owned()]);

    press_key(&ui, Key::F2);
    assert_eq!(
        ui.get_content_prompt_text().as_str(),
        "Rename to:  beta.txt",
        "the prompt should offer the row the zoomed click selected"
    );
    assert_eq!(
        ui.get_content_prompt_row(),
        listing(&ui)
            .iter()
            .position(|name| name == "beta.txt")
            .map(|index| i32::try_from(index).expect("a small listing"))
            .expect("beta.txt is listed"),
        "the prompt should sit on beta.txt's own row at the zoomed row height"
    );

    clear_prompt(&ui, "beta.txt".len());
    type_text(&ui, "renamed.txt");
    press_key(&ui, Key::Return);
    settle(&ui, &app);

    assert!(dir.join("renamed.txt").is_file(), "beta.txt was renamed");
    assert!(!dir.join("beta.txt").exists(), "the old name is gone");
    assert_eq!(
        selected(&ui),
        vec!["renamed.txt".to_owned()],
        "the renamed file stays selected"
    );

    // Still zoomed, still hit-testable: a click on the renamed row's own
    // drawn position still selects it.
    click_named_row(&ui, "alpha.txt");
    assert_eq!(selected(&ui), vec!["alpha.txt".to_owned()]);
    click_named_row(&ui, "renamed.txt");
    assert_eq!(selected(&ui), vec!["renamed.txt".to_owned()]);
}

// ---------------------------------------------------------------------
// Journey 5: zoom to the extremes.
// ---------------------------------------------------------------------

#[test]
fn zoom_to_the_extremes_leaves_every_row_clickable_and_inside_its_pane() {
    let _serial = serially();
    let dir = scratch("zoom-extremes");
    let names = ["alpha.txt", "beta.txt", "gamma.txt"];
    for name in names {
        write_file(&dir, name, "x");
    }
    let (ui, app) = window_at(&dir);

    for extreme in [80, 200] {
        zoom_to(&ui, &app, extreme);

        // The pane's own width scales with zoom (`contents-width: 470px *
        // Zoom.factor`), so its bounds are measured fresh at each extreme
        // rather than carried over from before the zoom.
        let pane = ElementHandle::find_by_element_id(&ui, "ContentsPane::click-area")
            .next()
            .expect("the contents pane has a click area");
        let pane_origin = pane.absolute_position();
        let pane_right = pane_origin.x + pane.size().width;

        for name in names {
            click_named_row(&ui, name);
            assert_eq!(
                selected(&ui),
                vec![name.to_owned()],
                "at {extreme}% zoom a click on {name}'s drawn row should still select it"
            );
        }

        // Nothing drawn off the pane it belongs to: every row starts no
        // further left than the pane's click area and ends no further
        // right than it either, at either extreme of the zoom range.
        for row in ElementHandle::find_by_element_id(&ui, "ContentsPane::listing-row") {
            let at = row.absolute_position();
            let right = at.x + row.size().width;
            assert!(
                at.x >= pane_origin.x - 0.5,
                "at {extreme}% zoom a row should not start left of its pane"
            );
            assert!(
                right <= pane_right + 0.5,
                "at {extreme}% zoom a row should not run past the right edge \
                 of its pane: row ends at {right}, pane ends at {pane_right}"
            );
        }
    }
}
