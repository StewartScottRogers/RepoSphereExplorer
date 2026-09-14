//! Driving the Contents pane in the real window, wired to a real `App`
//! the way the application wires it.
//!
//! `row_hit_mapping` clicks a window with rows pushed straight into it and
//! no `App` behind them; `scroll_into_view` measures where rows are drawn
//! and never clicks. The half nobody has is the one a reader meets: a real
//! listing, scrolled, clicked, sorted and typed at, with the application
//! behind it deciding what the click meant.
//!
//! `main`'s `wire_rows`, `wire_commands` and `wire_content_operations` are
//! private, so the wiring below is a copy of them. That is a hole worth
//! naming: a defect in `main`'s own wiring cannot be seen from here.

use gui::app::App;
use gui::{ContentRow, MainWindow, sync_ui};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model as _};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Row height in `app.slint`'s panes.
const ROW_HEIGHT: f32 = 20.0;

/// A directory of this test's own.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("repos-explorer-contents-{name}"));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    directory
}

// The window's callbacks come from the crate's own wiring, which is what
// `main` calls. A copy here would be a second thing to keep right, and a
// test of a copy proves nothing about what a reader gets.

/// A shown window, wired to an application listing `directory`.
fn window_on(directory: &Path) -> (MainWindow, Rc<RefCell<App>>) {
    let app = Rc::new(RefCell::new(App::new(directory.to_path_buf())));
    {
        let mut app = app.borrow_mut();
        let entries = service::list_directory(directory).expect("the directory lists");
        app.apply_contents_result_for_test(&[], protocol::Response::Directory { entries });
    }
    let ui = MainWindow::new().expect("the window should build");
    gui::wire_callbacks(&ui, &app);
    sync_ui(&ui, &app.borrow());
    ui.show().expect("the window should show");
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    (ui, app)
}

/// A listing of `count` plain files, named so their order is obvious.
fn files(name: &str, count: u16) -> (MainWindow, Rc<RefCell<App>>) {
    let directory = scratch(name);
    for index in 0..count {
        std::fs::write(directory.join(format!("file-{index:02}.txt")), "x")
            .expect("the fixture is written");
    }
    window_on(&directory)
}

/// Where the pane's clickable body starts, in window coordinates.
fn click_origin(ui: &MainWindow) -> LogicalPosition {
    ElementHandle::find_by_element_id(ui, "ContentsPane::click-area")
        .next()
        .expect("the contents pane has a click area")
        .absolute_position()
}

/// The rows the pane is drawing, in the order they appear on screen, with
/// the absolute y of each one's middle.
///
/// Measured rather than counted from the click area: the click area is
/// *inside* the scrolled body, so once the listing has scrolled its own
/// origin sits above the viewport and arithmetic from it points at
/// nothing drawn.
fn rows_on_screen(ui: &MainWindow) -> Vec<(String, f32)> {
    let mut rows: Vec<(String, f32)> =
        ElementHandle::find_by_element_id(ui, "ContentsPane::listing-row")
            .filter_map(|row| {
                let label = row.accessible_label()?.to_string();
                Some((label, row.absolute_position().y + row.size().height / 2.0))
            })
            .collect();
    rows.sort_by(|left, right| left.1.total_cmp(&right.1));
    rows
}

/// The `nth` row currently on screen: its name, and where to click it.
fn nth_row_on_screen(ui: &MainWindow, nth: usize) -> (String, LogicalPosition) {
    let rows = rows_on_screen(ui);
    let (name, y) = rows
        .get(nth)
        .unwrap_or_else(|| panic!("the pane should be drawing at least {} rows", nth + 1))
        .clone();
    (name, LogicalPosition::new(click_origin(ui).x + 20.0, y))
}

/// The name drawn at absolute window position `y`, if a row is drawn there.
fn row_drawn_at(ui: &MainWindow, y: f32) -> Option<String> {
    ElementHandle::find_by_element_id(ui, "ContentsPane::listing-row")
        .find(|row| {
            let top = row.absolute_position().y;
            y >= top && y < top + row.size().height
        })
        .and_then(|row| row.accessible_label().map(|label| label.to_string()))
}

/// The name of the row the application treats as selected.
fn selected_name(app: &Rc<RefCell<App>>) -> String {
    let app = app.borrow();
    app.content_rows()
        .get(app.content_selected())
        .map(|row| row.name.clone())
        .unwrap_or_default()
}

/// Every row the window is drawing as selected, by name.
fn highlighted(ui: &MainWindow) -> Vec<String> {
    let rows = ui.get_content_rows();
    (0..rows.row_count())
        .filter_map(|index| rows.row_data(index))
        .filter(|row: &ContentRow| row.selected)
        .map(|row| row.name.to_string())
        .collect()
}

/// A press and release of `button` at an absolute window position.
fn click_at(ui: &MainWindow, position: LogicalPosition, button: PointerEventButton) {
    let window = ui.window();
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

/// Clicks the first row, which is how a reader gives the Contents pane the
/// keyboard. The application opens with the Folders pane focused, so an
/// arrow key before this moves the tree, not the listing.
fn focus_the_listing(ui: &MainWindow) {
    let origin = click_origin(ui);
    left_click(
        ui,
        LogicalPosition::new(origin.x + 20.0, origin.y + ROW_HEIGHT / 2.0),
    );
}

/// A left click at an absolute window position.
fn left_click(ui: &MainWindow, position: LogicalPosition) {
    click_at(ui, position, PointerEventButton::Left);
}

/// A keystroke dispatched to the window, as a real keyboard does it.
fn press(ui: &MainWindow, text: &str) {
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.into() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text: text.into() });
}

/// A key with Shift held, pressed and released around it the way a
/// keyboard sends one. Slint takes its modifier state from these events,
/// so a Shift shortcut cannot be dispatched any other way.
fn press_shift(ui: &MainWindow, key: Key) {
    let shift = slint::SharedString::from(char::from(Key::Shift).to_string());
    let text = slint::SharedString::from(char::from(key).to_string());
    let window = ui.window();
    window.dispatch_event(WindowEvent::KeyPressed {
        text: shift.clone(),
    });
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
    window.dispatch_event(WindowEvent::KeyReleased { text: shift });
}

/// Four files, listed, with the first row clicked so the listing has the
/// keyboard and an anchor to extend from.
fn four_rows(name: &str) -> (MainWindow, Rc<RefCell<App>>) {
    let directory = scratch(name);
    for file in ["alpha.txt", "bravo.txt", "charlie.txt", "delta.txt"] {
        std::fs::write(directory.join(file), "x").expect("the fixture is written");
    }
    let (ui, app) = window_on(&directory);
    ui.invoke_content_row_clicked(0);
    (ui, app)
}

/// The listing shows the files it was given, and a click selects one.
///
/// The baseline everything below depends on: if this fails, nothing else
/// in the file means anything.
#[test]
fn a_click_on_an_unscrolled_listing_selects_the_row_under_the_pointer() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = files("plain", 60);
    let origin = click_origin(&ui);

    let y = origin.y + 3.0f32.mul_add(ROW_HEIGHT, ROW_HEIGHT / 2.0);
    let drawn = row_drawn_at(&ui, y).expect("a row is drawn there");
    left_click(&ui, LogicalPosition::new(origin.x + 20.0, y));

    assert_eq!(
        selected_name(&app),
        drawn,
        "the application acted on the row that is drawn under the pointer"
    );
}

/// The suspicion: the pane scrolls, and the click handler derives the row
/// from `mouse-y` inside the scrolled body. If the two origins disagree,
/// a reader who has scrolled down clicks one name and gets another.
#[test]
fn a_click_after_scrolling_selects_the_row_under_the_pointer() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = files("scrolled", 60);

    // Arrow to the far end of the listing, which is what scrolls the pane -
    // the same route a reader takes, not a property poked from the side.
    focus_the_listing(&ui);
    for _ in 0..50 {
        press(
            &ui,
            &char::from(slint::platform::Key::DownArrow).to_string(),
        );
    }
    assert!(
        ui.get_content_scroll_y() < -ROW_HEIGHT,
        "the pane should have scrolled by now; it is at {}",
        ui.get_content_scroll_y()
    );

    let (drawn, position) = nth_row_on_screen(&ui, 3);
    left_click(&ui, position);

    assert_eq!(
        selected_name(&app),
        drawn,
        "a click after scrolling has to act on the row under the pointer, \
         not on the one that used to be there"
    );
}

/// A reader who scrolls with the wheel and then clicks is in the same
/// position as one who scrolled with the keyboard.
#[test]
fn a_click_after_a_wheel_scroll_selects_the_row_under_the_pointer() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = files("wheel-click", 60);
    let origin = click_origin(&ui);

    let over = LogicalPosition::new(origin.x + 20.0, origin.y + 40.0);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position: over });
    ui.window().dispatch_event(WindowEvent::PointerScrolled {
        position: over,
        delta_x: 0.0,
        delta_y: -10.0 * ROW_HEIGHT,
    });
    assert!(
        ui.get_content_scroll_y() < -ROW_HEIGHT,
        "the wheel should have scrolled the listing; it is at {}",
        ui.get_content_scroll_y()
    );

    let y = origin.y + 3.0f32.mul_add(ROW_HEIGHT, ROW_HEIGHT / 2.0);
    let drawn = row_drawn_at(&ui, y).expect("a row is drawn there");
    left_click(&ui, LogicalPosition::new(origin.x + 20.0, y));

    assert_eq!(selected_name(&app), drawn);
}

/// The wheel has to survive the next render.
///
/// `sync_ui` writes `content-scroll-y` from the selected row on every
/// call, and `main` calls it on a 100ms timer. A reader who wheels the
/// listing past the selected row therefore has a tenth of a second before
/// the pane is dragged back to where the selection is.
#[test]
fn a_wheel_scroll_survives_the_next_sync() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = files("wheel", 60);
    let origin = click_origin(&ui);

    let over = LogicalPosition::new(origin.x + 20.0, origin.y + 40.0);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position: over });
    ui.window().dispatch_event(WindowEvent::PointerScrolled {
        position: over,
        delta_x: 0.0,
        delta_y: -10.0 * ROW_HEIGHT,
    });
    let scrolled = ui.get_content_scroll_y();
    assert!(scrolled < -ROW_HEIGHT, "the wheel moved the listing");

    // What the timer in `main` does, ten times a second, with nothing
    // having happened in between.
    sync_ui(&ui, &app.borrow());

    assert!(
        (ui.get_content_scroll_y() - scrolled).abs() < 1.0,
        "a wheel scroll must not be undone by the next render: the reader \
         left the listing at {scrolled} and the next sync put it at {}",
        ui.get_content_scroll_y()
    );
}

/// Clicking a column header sorts the listing.
#[test]
fn clicking_a_column_header_sorts_the_listing() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = scratch("sort");
    for (name, size) in [("charlie.txt", 300), ("alpha.txt", 100), ("bravo.txt", 20)] {
        std::fs::write(directory.join(name), "x".repeat(size)).expect("the fixture is written");
    }
    let (ui, app) = window_on(&directory);
    assert_eq!(app.borrow().sort_column(), 0, "name order to begin with");

    let header = ElementHandle::find_by_accessible_label(&ui, "Size")
        .find(|handle| handle.size().height > 0.0 && handle.size().width > 0.0)
        .expect("the Size column header is drawn");
    let at = header.absolute_position();
    left_click(
        &ui,
        LogicalPosition::new(
            at.x + header.size().width / 2.0,
            at.y + header.size().height / 2.0,
        ),
    );

    assert_eq!(
        app.borrow().sort_column(),
        1,
        "a click on the Size header should sort by size"
    );
}

/// Sorting reorders the rows under the reader; the highlight has to come
/// with the file it was on, not stay on the row number it happened to be.
#[test]
fn sorting_keeps_the_highlight_on_the_file_that_was_selected() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = scratch("sort-highlight");
    // Smallest last in name order, so sorting by size really does move the
    // selected row rather than leaving it where it was.
    for (name, size) in [("alpha.txt", 300), ("bravo.txt", 200), ("charlie.txt", 100)] {
        std::fs::write(directory.join(name), "x".repeat(size)).expect("the fixture is written");
    }
    let (ui, app) = window_on(&directory);

    // Select the last row by name order: charlie.
    let (name, position) = nth_row_on_screen(&ui, 2);
    assert_eq!(name, "charlie.txt");
    left_click(&ui, position);
    assert_eq!(highlighted(&ui), vec!["charlie.txt".to_owned()]);

    // Sort by size, which puts charlie first.
    ui.invoke_content_sort_requested(1);

    assert_eq!(
        selected_name(&app),
        "charlie.txt",
        "the lead row should still be the file that was selected"
    );
    assert_eq!(
        highlighted(&ui),
        vec!["charlie.txt".to_owned()],
        "and the window should be drawing that file as the selected one"
    );
}

/// Double-clicking a folder drills into it.
#[test]
fn a_double_click_on_a_folder_drills_into_it() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = scratch("drill");
    std::fs::create_dir_all(directory.join("inner")).expect("the fixture is written");
    std::fs::write(directory.join("inner").join("deep.txt"), "x").expect("the fixture is written");
    let (ui, app) = window_on(&directory);
    assert_eq!(app.borrow().content_rows()[0].name, "inner/");

    let origin = click_origin(&ui);
    let position = LogicalPosition::new(origin.x + 20.0, origin.y + ROW_HEIGHT / 2.0);
    let window = ui.window();
    window.dispatch_event(WindowEvent::PointerMoved { position });
    for _ in 0..2 {
        window.dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        window.dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
    }

    assert_eq!(
        app.borrow().breadcrumbs().last().map(String::as_str),
        Some("inner"),
        "a double click on a folder row should have drilled into it"
    );
}

/// A rubber band selects every row it covered.
#[test]
fn a_marquee_selects_every_row_it_covered() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = files("marquee", 20);
    let origin = click_origin(&ui);
    let window = ui.window();

    let from = LogicalPosition::new(origin.x + 20.0, origin.y + ROW_HEIGHT / 2.0);
    window.dispatch_event(WindowEvent::PointerMoved { position: from });
    window.dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button: PointerEventButton::Left,
    });
    // Down four rows, a pixel at a time-ish, so the drag is unmistakable.
    for step in 1u8..=4 {
        let position = LogicalPosition::new(
            origin.x + 20.0,
            f32::from(step).mul_add(ROW_HEIGHT, origin.y + ROW_HEIGHT / 2.0),
        );
        window.dispatch_event(WindowEvent::PointerMoved { position });
    }
    let to = LogicalPosition::new(origin.x + 20.0, 4.0f32.mul_add(ROW_HEIGHT, origin.y + 10.0));
    window.dispatch_event(WindowEvent::PointerReleased {
        position: to,
        button: PointerEventButton::Left,
    });

    assert_eq!(
        app.borrow().selected_count(),
        5,
        "a band from row 0 to row 4 covers five rows"
    );
    assert_eq!(
        highlighted(&ui).len(),
        5,
        "and the window draws all five as selected"
    );
}

/// Type-ahead: a letter pressed on the window jumps the listing to the
/// next name starting with it.
#[test]
fn a_typed_letter_jumps_to_the_matching_row() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = scratch("type-ahead");
    for name in ["alpha.txt", "bravo.txt", "charlie.txt"] {
        std::fs::write(directory.join(name), "x").expect("the fixture is written");
    }
    let (ui, app) = window_on(&directory);
    // The window opens with the tree focused, and a letter goes to the
    // pane the reader is in. Clicking a row is how they get here.
    ui.invoke_content_row_clicked(0);
    assert_eq!(selected_name(&app), "alpha.txt");

    press(&ui, "c");

    assert_eq!(
        selected_name(&app),
        "charlie.txt",
        "a typed letter should jump to the next name beginning with it"
    );
}

/// The mirror of `folders_in_the_window`'s tree type-ahead: with the
/// listing focused, a letter moves the listing and leaves the tree alone.
///
/// It used to move the listing whichever pane was focused, which is how
/// the two tests above passed while the defect was live - the window
/// opens on the tree, so they were being answered by the wrong pane.
#[test]
fn a_typed_letter_with_the_listing_focused_leaves_the_tree_alone() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = scratch("type-ahead-listing-only");
    for name in ["alpha.txt", "bravo.txt", "charlie.txt"] {
        std::fs::write(directory.join(name), "x").expect("the fixture is written");
    }
    let (ui, app) = window_on(&directory);
    ui.invoke_content_row_clicked(0);
    let tree_before = app.borrow().folder_selected();

    press(&ui, "c");

    assert_eq!(
        selected_name(&app),
        "charlie.txt",
        "the focused pane is the listing, so the letter moves the listing"
    );
    assert_eq!(
        app.borrow().folder_selected(),
        tree_before,
        "and the tree, which is not the focused pane, does not move"
    );
}

/// The keyboard has to come back to the listing after the editor closes.
///
/// The editing surface takes focus the moment it is created. When it goes
/// away the window's own focus scope is what the Contents pane's arrows
/// and type-ahead run on, so if focus is not handed back the listing is
/// dead to the keyboard.
#[test]
fn the_keyboard_comes_back_to_the_listing_after_the_editor_closes() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = scratch("after-editing");
    for name in ["alpha.rs", "bravo.rs", "charlie.rs"] {
        std::fs::write(directory.join(name), "fn main() {}\n").expect("the fixture is written");
    }
    let (ui, app) = window_on(&directory);
    {
        let mut app = app.borrow_mut();
        let view = service::view_file(&directory.join("alpha.rs")).expect("the file opens");
        app.show_file_view_for_test(view);
        app.begin_file_edit();
        assert!(app.editing_file(), "the editor is open");
    }
    sync_ui(&ui, &app.borrow());

    // And now it closes, the way Escape closes it.
    app.borrow_mut().cancel_pending();
    sync_ui(&ui, &app.borrow());
    assert!(!app.borrow().editing_file(), "the editor is closed");

    press(&ui, "c");

    assert_eq!(
        selected_name(&app),
        "charlie.rs",
        "with the editor gone the keyboard belongs to the listing again"
    );
}

/// Escape closes the editor, which is the only way back to the listing
/// short of saving.
#[test]
fn escape_closes_the_editor_and_gives_the_listing_its_keyboard_back() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = scratch("escape");
    for name in ["alpha.rs", "bravo.rs"] {
        std::fs::write(directory.join(name), "fn main() {}\n").expect("the fixture is written");
    }
    let (ui, app) = window_on(&directory);
    {
        let mut app = app.borrow_mut();
        let view = service::view_file(&directory.join("alpha.rs")).expect("the file opens");
        app.show_file_view_for_test(view);
        app.begin_file_edit();
    }
    sync_ui(&ui, &app.borrow());

    press(&ui, &char::from(slint::platform::Key::Escape).to_string());

    assert!(
        !app.borrow().editing_file(),
        "Escape should have closed the editor"
    );
}

/// Arrowing down moves the selection, through the window's own key scope.
#[test]
fn the_arrows_move_the_selection_through_the_window() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = files("arrows", 20);
    focus_the_listing(&ui);
    assert_eq!(selected_name(&app), "file-00.txt");

    press(
        &ui,
        &char::from(slint::platform::Key::DownArrow).to_string(),
    );

    assert_eq!(
        selected_name(&app),
        "file-01.txt",
        "a real Down key should move the listing's selection"
    );
}

/// Arrowing past the fold has to keep the selected row on screen: a
/// highlight nobody can see is a listing that has lost its place.
#[test]
fn the_selected_row_stays_on_screen_while_arrowing_down() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = files("arrow-scroll", 60);
    focus_the_listing(&ui);

    for _ in 0..40 {
        press(
            &ui,
            &char::from(slint::platform::Key::DownArrow).to_string(),
        );
    }

    let selected = selected_name(&app);
    let drawn: Vec<String> = ElementHandle::find_by_element_id(&ui, "ContentsPane::listing-row")
        .filter_map(|row| row.accessible_label().map(|label| label.to_string()))
        .collect();
    assert!(
        drawn.contains(&selected),
        "the selected row {selected} should be one of the rows on screen; \
         the pane is showing {drawn:?}"
    );
}

/// Double-clicking after scrolling drills into the folder under the
/// pointer, not one the reader scrolled past.
#[test]
fn a_double_click_after_scrolling_opens_the_folder_under_the_pointer() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = scratch("drill-scrolled");
    for index in 0..60u16 {
        std::fs::create_dir_all(directory.join(format!("dir-{index:02}")))
            .expect("the fixture is written");
    }
    let (ui, app) = window_on(&directory);
    focus_the_listing(&ui);
    for _ in 0..50 {
        press(
            &ui,
            &char::from(slint::platform::Key::DownArrow).to_string(),
        );
    }
    assert!(ui.get_content_scroll_y() < -ROW_HEIGHT, "the pane scrolled");

    let (drawn, position) = nth_row_on_screen(&ui, 3);
    let window = ui.window();
    window.dispatch_event(WindowEvent::PointerMoved { position });
    for _ in 0..2 {
        window.dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        window.dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
    }

    assert_eq!(
        app.borrow().breadcrumbs().last().map(String::as_str),
        Some(drawn.trim_end_matches('/')),
        "a double click after scrolling should open the folder drawn under \
         the pointer"
    );
}

/// A rubber band drawn over a scrolled listing covers the rows it was
/// drawn over.
#[test]
fn a_marquee_after_scrolling_covers_the_rows_it_was_drawn_over() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, _app) = files("marquee-scrolled", 60);
    focus_the_listing(&ui);
    for _ in 0..50 {
        press(
            &ui,
            &char::from(slint::platform::Key::DownArrow).to_string(),
        );
    }
    assert!(ui.get_content_scroll_y() < -ROW_HEIGHT, "the pane scrolled");

    let window = ui.window();
    // Rows 1 to 5, not 0 to 4: the topmost drawn row can be half off the
    // top edge, and pressing it scrolls the listing a few pixels under the
    // pointer before the drag has started.
    let path: Vec<LogicalPosition> = (1..=5).map(|nth| nth_row_on_screen(&ui, nth).1).collect();
    let (first, from) = nth_row_on_screen(&ui, 1);
    let (last, to) = nth_row_on_screen(&ui, 5);

    window.dispatch_event(WindowEvent::PointerMoved { position: from });
    window.dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button: PointerEventButton::Left,
    });
    for position in path {
        window.dispatch_event(WindowEvent::PointerMoved { position });
    }
    window.dispatch_event(WindowEvent::PointerReleased {
        position: to,
        button: PointerEventButton::Left,
    });

    let selected = highlighted(&ui);
    assert_eq!(
        selected.len(),
        5,
        "a band over five drawn rows selects five rows, and selected {selected:?}"
    );
    assert_eq!(
        selected.first().map(String::as_str),
        Some(first.as_str()),
        "starting at the row the band started on"
    );
    assert_eq!(
        selected.last().map(String::as_str),
        Some(last.as_str()),
        "and ending at the row it ended on"
    );
}

/// Ctrl+click adds the row under the pointer to the selection, scrolled
/// or not.
#[test]
fn a_ctrl_click_after_scrolling_adds_the_row_under_the_pointer() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = files("ctrl-scrolled", 60);
    focus_the_listing(&ui);
    for _ in 0..50 {
        press(
            &ui,
            &char::from(slint::platform::Key::DownArrow).to_string(),
        );
    }
    let lead = selected_name(&app);

    let (drawn, position) = nth_row_on_screen(&ui, 2);
    assert_ne!(drawn, lead, "a different row from the one already selected");

    let window = ui.window();
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::KeyPressed {
        text: char::from(slint::platform::Key::Control).to_string().into(),
    });
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::KeyReleased {
        text: char::from(slint::platform::Key::Control).to_string().into(),
    });

    let selected = highlighted(&ui);
    assert!(
        selected.contains(&drawn) && selected.contains(&lead),
        "Ctrl+click should have added {drawn} to the selection, leaving \
         {lead} in it; the selection is {selected:?}"
    );
}

/// Sorting reorders the listing under the reader, so the rows it draws as
/// selected have to be the files that were selected - not the row numbers
/// they happened to occupy.
#[test]
fn sorting_keeps_a_multiple_selection_on_the_files_it_was_on() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = scratch("sort-multi");
    for (name, size) in [
        ("alpha.txt", 400),
        ("bravo.txt", 300),
        ("charlie.txt", 200),
        ("delta.txt", 100),
    ] {
        std::fs::write(directory.join(name), "x".repeat(size)).expect("the fixture is written");
    }
    let (ui, _app) = window_on(&directory);

    // The first two by name: alpha and bravo.
    ui.invoke_content_row_clicked(0);
    ui.invoke_content_row_shift_clicked(1);
    assert_eq!(
        highlighted(&ui),
        vec!["alpha.txt".to_owned(), "bravo.txt".to_owned()]
    );

    // Sort by size, which reverses the listing: delta, charlie, bravo, alpha.
    ui.invoke_content_sort_requested(1);

    // Read in row order, so the pair comes back reversed; which files are
    // selected is the question, and the order they are drawn in is the
    // sort's business.
    let mut after = highlighted(&ui);
    after.sort();
    assert_eq!(
        after,
        vec!["alpha.txt".to_owned(), "bravo.txt".to_owned()],
        "the same two files should still be the selected ones after a sort"
    );
}

/// The sharp end of a stale selection: what Delete says it is about to
/// delete has to be the file the reader has highlighted.
#[test]
fn delete_after_a_sort_names_the_file_that_is_highlighted() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = scratch("sort-delete");
    for (name, size) in [("alpha.txt", 300), ("bravo.txt", 200), ("charlie.txt", 100)] {
        std::fs::write(directory.join(name), "x".repeat(size)).expect("the fixture is written");
    }
    let (ui, app) = window_on(&directory);

    let (name, position) = nth_row_on_screen(&ui, 2);
    assert_eq!(name, "charlie.txt");
    left_click(&ui, position);

    // Sort by size, which moves charlie from last to first.
    ui.invoke_content_sort_requested(1);
    app.borrow_mut().request_delete();

    assert_eq!(
        app.borrow().prompt_text(),
        "Delete charlie.txt?  (y / n)",
        "Delete has to act on the file the reader selected, whatever a \
         sort did to the row numbers"
    );
}

/// Type-ahead has to jump to a name in the listing as it is now drawn.
#[test]
fn type_ahead_after_a_sort_jumps_to_the_row_it_names() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = scratch("sort-type-ahead");
    for (name, size) in [("alpha.txt", 300), ("bravo.txt", 200), ("charlie.txt", 100)] {
        std::fs::write(directory.join(name), "x".repeat(size)).expect("the fixture is written");
    }
    let (ui, app) = window_on(&directory);
    // Type-ahead answers the focused pane, and the window opens on the
    // tree; a reader clicks into the listing before typing in it.
    ui.invoke_content_row_clicked(0);
    ui.invoke_content_sort_requested(1);

    press(&ui, "b");

    assert_eq!(selected_name(&app), "bravo.txt");
    assert_eq!(
        highlighted(&ui),
        vec!["bravo.txt".to_owned()],
        "the row the window draws as selected is the one type-ahead found"
    );
}

/// Saving is the other way out of the editor, and it is a key too.
#[test]
fn control_s_saves_and_closes_the_editor() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = scratch("control-s");
    std::fs::write(directory.join("alpha.rs"), "fn main() {}\n").expect("the fixture is written");
    let (ui, app) = window_on(&directory);
    {
        let mut app = app.borrow_mut();
        let view = service::view_file(&directory.join("alpha.rs")).expect("the file opens");
        app.show_file_view_for_test(view);
        app.begin_file_edit();
        assert!(app.editing_file(), "the editor is open");
    }
    sync_ui(&ui, &app.borrow());

    let window = ui.window();
    let control = char::from(slint::platform::Key::Control).to_string();
    window.dispatch_event(WindowEvent::KeyPressed {
        text: control.clone().into(),
    });
    window.dispatch_event(WindowEvent::KeyPressed { text: "s".into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: "s".into() });
    window.dispatch_event(WindowEvent::KeyReleased {
        text: control.into(),
    });

    assert!(
        !app.borrow().editing_file(),
        "Ctrl+S should have saved and closed the editor; if neither this \
         nor Escape works the keyboard cannot leave the editor at all"
    );
}

/// Shift+Down grows the selection instead of moving it.
///
/// There was no keyboard way to select a range at all: the window's key
/// scope sent every arrow to `selection-moved`, which replaces the
/// selection, so a reader had to use the mouse or Ctrl+A for everything.
/// D6 settles multi-select, and #504 made it matter - Copy and Cut now
/// honour the whole selection, so building one is worth doing.
#[test]
fn shift_down_extends_the_selection_rather_than_moving_it() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = four_rows("shift-down");
    assert_eq!(highlighted(&ui), vec!["alpha.txt".to_owned()]);

    press_shift(&ui, Key::DownArrow);
    assert_eq!(
        highlighted(&ui),
        vec!["alpha.txt".to_owned(), "bravo.txt".to_owned()],
        "one Shift+Down should have selected two rows, not moved to the second"
    );

    press_shift(&ui, Key::DownArrow);
    assert_eq!(
        highlighted(&ui),
        vec![
            "alpha.txt".to_owned(),
            "bravo.txt".to_owned(),
            "charlie.txt".to_owned()
        ],
        "and again should make three"
    );
    assert_eq!(
        selected_name(&app),
        "charlie.txt",
        "the lead row is the far end, so a further Shift+Down keeps growing"
    );
}

/// A plain arrow still replaces the selection. This is the behaviour that
/// has to survive the change, not the one being added.
#[test]
fn a_plain_arrow_still_collapses_the_selection_to_one_row() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, _app) = four_rows("plain-arrow");

    press_shift(&ui, Key::DownArrow);
    press_shift(&ui, Key::DownArrow);
    assert_eq!(highlighted(&ui).len(), 3, "three are selected");

    press(&ui, &char::from(Key::DownArrow).to_string());

    assert_eq!(
        highlighted(&ui),
        vec!["delta.txt".to_owned()],
        "a plain arrow moves, and moving replaces the selection"
    );
}

/// Shift+Up back over the anchor flips the range rather than growing it
/// the other way - the anchor is where the range is measured from, and it
/// does not follow the lead row.
#[test]
fn shift_up_back_over_the_anchor_flips_the_range() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = scratch("shift-flip");
    for file in ["alpha.txt", "bravo.txt", "charlie.txt", "delta.txt"] {
        std::fs::write(directory.join(file), "x").expect("the fixture is written");
    }
    let (ui, _app) = window_on(&directory);
    // Anchor on the third row, so there is somewhere to go in both
    // directions.
    ui.invoke_content_row_clicked(2);

    press_shift(&ui, Key::DownArrow);
    assert_eq!(
        highlighted(&ui),
        vec!["charlie.txt".to_owned(), "delta.txt".to_owned()]
    );

    press_shift(&ui, Key::UpArrow);
    assert_eq!(
        highlighted(&ui),
        vec!["charlie.txt".to_owned()],
        "back onto the anchor leaves the anchor alone"
    );

    press_shift(&ui, Key::UpArrow);
    assert_eq!(
        highlighted(&ui),
        vec!["bravo.txt".to_owned(), "charlie.txt".to_owned()],
        "and past it the range runs the other way from the same anchor"
    );
}

/// Shift+End and Shift+Home take the range to the ends.
#[test]
fn shift_end_and_shift_home_extend_to_the_ends() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, _app) = four_rows("shift-ends");

    press_shift(&ui, Key::End);
    assert_eq!(
        highlighted(&ui).len(),
        4,
        "Shift+End selects to the last row"
    );

    ui.invoke_content_row_clicked(3);
    press_shift(&ui, Key::Home);
    assert_eq!(
        highlighted(&ui).len(),
        4,
        "and Shift+Home from the last row selects back to the first"
    );
}

/// The whole point of #511: a range built from the keyboard is the range
/// Copy takes.
#[test]
fn a_range_built_with_shift_is_what_copy_takes() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = four_rows("shift-then-copy");

    press_shift(&ui, Key::DownArrow);
    press_shift(&ui, Key::DownArrow);
    ui.invoke_clipboard_copy_requested();

    assert!(
        app.borrow().status_text().contains("3 items"),
        "the status bar should report the three rows Shift built; it said {:?}",
        app.borrow().status_text()
    );
}
