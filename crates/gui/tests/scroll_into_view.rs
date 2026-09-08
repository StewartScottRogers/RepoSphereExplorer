//! Checks that a pane's scroll offset moves what is on screen, and that the
//! offset the rule gives puts the selected row inside the viewport.
//!
//! Two halves, deliberately:
//!
//! - `gui::scroll_offset_for` is a pure function, unit-tested in `lib.rs`.
//!   It holds the rule: the smallest move that brings the row into view.
//! - This file checks the wiring that rule depends on - that setting
//!   `content-scroll-y` really does move the listing, by exactly that much,
//!   and that the row then lies where the rule says it does.
//!
//! The rule used to live in a `changed selected` handler in `app.slint`.
//! Such a handler is never dispatched without an event loop, so nothing
//! could see whether it had run: a test that clicked the bottom row of an
//! unscrolled pane got the row that was already there and called it a pass.
//! Four of the five tests written against it passed for that reason.
//!
//! These measure the listing's own position instead of clicking it. Pointer
//! dispatch in the testing backend reads geometry cached at the last draw,
//! so a click after a scroll answers with the row that *was* there - which
//! is a property of the harness, not of the application.

use gui::{ContentRow, MainWindow, scroll_offset_for};
use i_slint_backend_testing::ElementHandle;
use slint::{ComponentHandle, Image, ModelRc, SharedString, VecModel};

/// Row height in `app.slint`'s panes.
const ROW_HEIGHT: f32 = 20.0;

/// Many more rows than any plausible pane height holds at once, so the
/// fixture needs scrolling whatever window it renders in.
const ROW_COUNT: u16 = 200;

/// A details row with only its name filled in; the other columns play no
/// part in what these tests measure.
fn row(name: &str) -> ContentRow {
    ContentRow {
        icon: Image::default(),
        is_repository: false,
        name: SharedString::from(name),
        size: SharedString::new(),
        kind: SharedString::new(),
        modified: SharedString::new(),
        selected: false,
    }
}

fn shown_window() -> MainWindow {
    let ui = MainWindow::new().expect("the window should build");
    ui.set_content_rows(ModelRc::new(VecModel::from(
        (0..ROW_COUNT)
            .map(|index| row(&format!("row-{index:03}")))
            .collect::<Vec<_>>(),
    )));
    ui.show().expect("the window should show");
    ui
}

/// The rows the pane is showing, by name, in the order they appear.
///
/// The element search returns what is visible, which is exactly the
/// question these tests ask: did the pane scroll far enough to bring the
/// row on screen, and no further than it had to.
fn visible_rows(ui: &MainWindow) -> Vec<String> {
    ElementHandle::find_by_element_id(ui, "ContentsPane::listing-row")
        .filter_map(|handle| handle.accessible_label().map(|label| label.to_string()))
        .collect()
}

/// The name row `index` carries, as `shown_window` built it.
fn name_of(index: u16) -> String {
    format!("row-{index:03}")
}

/// Where row `index` sits on screen, or `None` when it is not drawn.
///
/// Measured rather than counted: the element search treats a row whose
/// bottom edge merely touches the top of the viewport as visible, so
/// "which row is first" is off by one at exactly the boundary these tests
/// are about.
fn row_top(ui: &MainWindow, index: u16) -> Option<f32> {
    let wanted = name_of(index);
    ElementHandle::find_by_element_id(ui, "ContentsPane::listing-row")
        .find(|handle| {
            handle
                .accessible_label()
                .is_some_and(|label| label.as_str() == wanted)
        })
        .map(|handle| handle.absolute_position().y)
}

/// How much of the listing the pane shows.
fn viewport_height(ui: &MainWindow) -> f32 {
    let height = ui.get_content_viewport_height();
    assert!(height > 0.0, "the pane should have been laid out by now");
    height
}

#[test]
fn the_offset_moves_the_listing_by_exactly_that_much() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();
    assert_eq!(
        visible_rows(&ui).first().map(String::as_str),
        Some(name_of(0).as_str()),
        "unscrolled, the listing starts at its first row"
    );

    let viewport_top = row_top(&ui, 0).expect("the first row is on screen at rest");

    ui.set_content_scroll_y(-12.0 * ROW_HEIGHT);

    assert_eq!(
        row_top(&ui, 12),
        Some(viewport_top),
        "row 12 should now sit where row 0 was: the offset has to move the          listing, or the rule computing it is moot"
    );
}

#[test]
fn the_rule_puts_a_row_below_the_fold_at_the_bottom_edge() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();
    let height = viewport_height(&ui);
    let viewport_top = row_top(&ui, 0).expect("the first row is on screen at rest");
    assert!(
        height < f32::from(ROW_COUNT) * ROW_HEIGHT,
        "the fixture should hold more rows than the pane can show at once"
    );

    let target = ROW_COUNT - 1;
    ui.set_content_scroll_y(scroll_offset_for(usize::from(target), height, 0.0));

    let top = row_top(&ui, target).expect("the target row should be on screen");
    assert!(
        (top + ROW_HEIGHT - (viewport_top + height)).abs() < 0.5,
        "the row should end at the bottom edge - the smallest move that gets          it there, not a page: row ends at {}, viewport at {}",
        top + ROW_HEIGHT,
        viewport_top + height
    );
}

#[test]
fn the_rule_puts_a_row_above_the_fold_at_the_top_edge() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();
    let height = viewport_height(&ui);
    let viewport_top = row_top(&ui, 0).expect("the first row is on screen at rest");

    // Scrolled deep into the listing, then the selection jumps back up.
    let scrolled = scroll_offset_for(usize::from(ROW_COUNT - 1), height, 0.0);
    let target: u16 = 40;
    ui.set_content_scroll_y(scroll_offset_for(usize::from(target), height, scrolled));

    assert_eq!(
        row_top(&ui, target),
        Some(viewport_top),
        "a row above the fold comes to the top edge"
    );
}

#[test]
fn a_row_already_on_screen_leaves_the_listing_where_it_was() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();
    let height = viewport_height(&ui);
    let before = visible_rows(&ui);

    ui.set_content_scroll_y(scroll_offset_for(1, height, 0.0));

    assert_eq!(
        visible_rows(&ui),
        before,
        "a selection already in view must not move the listing under the reader"
    );
}
