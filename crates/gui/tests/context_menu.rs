//! Geometry checks for the contents pane's right-click menus.
//!
//! Both `PopupWindow`s were declared without a size. A `PopupWindow` with no
//! size of its own is laid out 0x0, and a child with no `x` is centred in its
//! parent - so each menu's body was placed at `x: -75px`, its full width
//! outside the popup that owns it. Nothing clipped it, so the menu looked
//! perfectly normal on screen, but hit-testing is bounded by the popup, which
//! left every item in both menus unhittable.
//!
//! These tests dispatch through Slint's own hit-testing rather than the
//! windowing system, and assert the property that was violated: an item has
//! to lie inside the popup that owns it to be reachable at all.

use gui::{ContentRow, MainWindow};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, SharedString, VecModel};

/// Row height in `app.slint`'s panes, so a click can be aimed at a row.
const ROW_HEIGHT: f32 = 20.0;

/// Menu width in `app.slint`, which is what an item has to fit inside.
const MENU_WIDTH: f32 = 150.0;

/// A details row with only its name filled in; the other columns play no
/// part in what these tests measure.
fn row(name: &str) -> ContentRow {
    ContentRow {
        glyph: SharedString::from("\u{25AA}"),
        name: SharedString::from(name),
        size: SharedString::new(),
        kind: SharedString::new(),
        modified: SharedString::new(),
        selected: false,
    }
}

/// A window with two contents rows, shown and ready for pointer events.
fn shown_window() -> MainWindow {
    let ui = MainWindow::new().expect("the window should build");
    ui.set_content_rows(ModelRc::new(VecModel::from(vec![
        row("a.txt"),
        row("b.txt"),
    ])));
    ui.show().expect("the window should show");
    ui
}

/// Right-clicks the contents pane `rows_down` rows below its first row. Past
/// the last row that opens the empty-area menu; on one, the row menu.
fn right_click(ui: &MainWindow, rows_down: f32) {
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
        button: PointerEventButton::Right,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Right,
    });
}

/// Asserts that the menu item labelled `label` is open and lies within the
/// bounds of the popup that owns it, which is what makes it hittable.
fn assert_item_is_inside_its_popup(ui: &MainWindow, label: &str) {
    let item = ElementHandle::find_by_accessible_label(ui, label)
        .next()
        .unwrap_or_else(|| panic!("the open menu should offer a {label:?} item"));
    let position = item.absolute_position();
    let size = item.size();

    assert!(
        position.x >= 0.0 && position.y >= 0.0,
        "{label:?} is laid out at {position:?}, outside the popup that owns it"
    );
    // A popup with no size of its own collapses its child to nothing, which
    // would satisfy a bounds check while leaving the item just as unhittable.
    assert!(
        (size.width - MENU_WIDTH).abs() < f32::EPSILON,
        "{label:?} is {}px wide, not the menu's {MENU_WIDTH}px",
        size.width
    );
    assert!(
        position.x + size.width <= MENU_WIDTH,
        "{label:?} runs from {} to {} across a {MENU_WIDTH}px menu",
        position.x,
        position.x + size.width
    );
}

#[test]
fn a_right_click_on_a_row_opens_a_menu_whose_items_are_inside_it() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();

    assert_eq!(
        ElementHandle::find_by_accessible_label(&ui, "Rename").count(),
        0,
        "no menu before the right-click"
    );

    right_click(&ui, 0.0);

    for label in ["Open", "Rename", "Copy", "Delete", "Extract"] {
        assert_item_is_inside_its_popup(&ui, label);
    }
}

#[test]
fn a_right_click_on_empty_space_opens_a_menu_whose_items_are_inside_it() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();

    // Well past the two rows, which is what opens the empty-area menu.
    right_click(&ui, 20.0);

    for label in ["New Folder", "New File"] {
        assert_item_is_inside_its_popup(&ui, label);
    }
}
