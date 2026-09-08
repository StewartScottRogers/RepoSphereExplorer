//! Checks that a pane scrolls its selected row into view.
//!
//! `sync_ui` (`crates/gui/src/lib.rs`) sets `content_selected` /
//! `folder_selected` directly on every render, whatever moved the
//! selection - a click, type-ahead, an arrow key, Home/End, paging, or a
//! reselect after an operation - so driving the property directly exercises
//! the same `changed selected` callback in `app.slint` that all of those
//! paths funnel through.
//!
//! Verified through the front end's own hit testing, the way
//! `row_hit_mapping.rs` verifies clicks: a click at the screen position a
//! row should now occupy only lands on that row if the pane actually
//! scrolled it there.

use gui::{ContentRow, MainWindow};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, Image, LogicalPosition, ModelRc, SharedString, VecModel};
use std::cell::Cell;
use std::rc::Rc;

const ROW_HEIGHT: f32 = 20.0;
/// Many more rows than any plausible pane height holds at once, so the
/// fixture is guaranteed to need scrolling regardless of the window it
/// renders in.
const ROW_COUNT: usize = 200;

/// A details row with only its name filled in; the other columns play no
/// part in what these tests measure.
fn row(name: &str) -> ContentRow {
    ContentRow {
        icon: Image::default(),
        name: SharedString::from(name),
        size: SharedString::new(),
        kind: SharedString::new(),
        modified: SharedString::new(),
        selected: false,
    }
}

/// A window with `count` contents rows, shown and ready for pointer events.
fn shown_window(count: usize) -> MainWindow {
    let ui = MainWindow::new().expect("the window should build");
    ui.set_content_rows(ModelRc::new(VecModel::from(
        (0..count)
            .map(|i| row(&format!("row-{i}")))
            .collect::<Vec<_>>(),
    )));
    ui.show().expect("the window should show");
    ui
}

/// The contents pane's scrollable viewport: its top-left corner in window
/// coordinates, and its visible height.
fn contents_viewport(ui: &MainWindow) -> (LogicalPosition, f32) {
    let handle = ElementHandle::find_by_element_id(ui, "ContentsPane::scroll")
        .next()
        .expect("the contents pane has a scroll view");
    (handle.absolute_position(), handle.size().height)
}

/// `ROW_COUNT` rows at `ROW_HEIGHT` each, without the precision-losing `as`
/// cast from `usize` to `f32`.
fn full_listing_height() -> f32 {
    let count = u16::try_from(ROW_COUNT).expect("the test uses a few hundred rows");
    f32::from(count) * ROW_HEIGHT
}

/// `value` as the `i32` the generated Slint setters expect.
fn index(value: usize) -> i32 {
    i32::try_from(value).expect("the test uses a few hundred rows")
}

/// Clicks at window position `(x, y)` and reports the row that responded,
/// or `None` if `content-row-clicked` did not fire.
fn click_at(ui: &MainWindow, x: f32, y: f32) -> Option<usize> {
    let clicked = Rc::new(Cell::new(None));
    ui.on_content_row_clicked({
        let clicked = Rc::clone(&clicked);
        move |i| clicked.set(usize::try_from(i).ok())
    });
    let position = LogicalPosition::new(x, y);
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
    clicked.get()
}

#[test]
fn selecting_a_row_below_the_fold_scrolls_it_to_the_bottom() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window(ROW_COUNT);
    let (origin, visible_height) = contents_viewport(&ui);
    assert!(
        visible_height < full_listing_height(),
        "the fixture should hold more rows than the pane can show at once"
    );

    let target = ROW_COUNT - 1;
    ui.set_content_selected(index(target));

    // If the pane scrolled the smallest amount that brings the row fully
    // into view, it is now the last row visible - its middle sits one row
    // height above the bottom edge of the viewport.
    let y = origin.y + visible_height - ROW_HEIGHT / 2.0;
    assert_eq!(
        click_at(&ui, origin.x + 20.0, y),
        Some(target),
        "the selected row should be the last one visible, not scrolled a full page"
    );
}

#[test]
fn selecting_a_row_above_the_fold_scrolls_it_to_the_top() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window(ROW_COUNT);
    let (origin, _) = contents_viewport(&ui);

    // Scroll down first, so the top of the listing is off screen.
    ui.set_content_selected(index(ROW_COUNT - 1));
    ui.set_content_selected(index(0));

    let y = origin.y + ROW_HEIGHT / 2.0;
    assert_eq!(
        click_at(&ui, origin.x + 20.0, y),
        Some(0),
        "the selected row should be the first one visible, at the top of the viewport"
    );
}

#[test]
fn selecting_an_already_visible_row_leaves_the_scroll_position_untouched() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window(ROW_COUNT);
    let (origin, _) = contents_viewport(&ui);

    // Row 0 is visible from the start; selecting another row still on
    // screen should not move the pane, so row 0 stays exactly where it was.
    ui.set_content_selected(1);

    let y = origin.y + ROW_HEIGHT / 2.0;
    assert_eq!(
        click_at(&ui, origin.x + 20.0, y),
        Some(0),
        "selecting a row already on screen should not scroll row 0 out of its spot"
    );
}

#[test]
fn a_folder_shorter_than_the_pane_never_scrolls() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window(3);
    let (origin, _) = contents_viewport(&ui);

    ui.set_content_selected(2);

    let y = origin.y + ROW_HEIGHT / 2.0;
    assert_eq!(
        click_at(&ui, origin.x + 20.0, y),
        Some(0),
        "a listing shorter than the pane should never scroll, so row 0 stays at the top"
    );
}

#[test]
fn the_folders_pane_also_scrolls_the_selection_into_view() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = MainWindow::new().expect("the window should build");
    ui.set_folder_rows(ModelRc::new(VecModel::from(
        (0..ROW_COUNT)
            .map(|i| SharedString::from(format!("folder-{i}")))
            .collect::<Vec<_>>(),
    )));
    ui.show().expect("the window should show");

    let scroll = ElementHandle::find_by_element_id(&ui, "Pane::scroll")
        .next()
        .expect("the folders pane has a scroll view");
    let origin = scroll.absolute_position();
    let visible_height = scroll.size().height;
    assert!(
        visible_height < full_listing_height(),
        "the fixture should hold more rows than the pane can show at once"
    );

    let target = ROW_COUNT - 1;
    ui.set_folder_selected(index(target));

    let clicked = Rc::new(Cell::new(None));
    ui.on_folder_row_clicked({
        let clicked = Rc::clone(&clicked);
        move |i| clicked.set(usize::try_from(i).ok())
    });
    let position = LogicalPosition::new(
        origin.x + 20.0,
        origin.y + visible_height - ROW_HEIGHT / 2.0,
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

    assert_eq!(
        clicked.get(),
        Some(target),
        "the folders pane should scroll the same way the contents pane does"
    );
}
