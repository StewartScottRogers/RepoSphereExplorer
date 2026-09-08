//! Checks that a click lands on the row it looks like it landed on.
//!
//! The contents pane draws each row at `i * row-height` inside a scrollable
//! body and derives the clicked index from the same origin, so the two should
//! agree exactly. This measures that rather than trusting it.

use gui::{ContentRow, MainWindow};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, Image, LogicalPosition, ModelRc, SharedString, VecModel};
use std::cell::Cell;
use std::rc::Rc;

const ROW_HEIGHT: f32 = 20.0;
/// Menu width in `app.slint`. An item carries its accessible label on both
/// its background and the `Text` inside it, so matches are narrowed to the
/// one that is the full width of the menu.
const MENU_WIDTH: f32 = 150.0;
/// Row count as a float, for the pixel arithmetic below.
const ROW_COUNT: f32 = 6.0;
const ROWS: [&str; 6] = ["a.txt", "b.txt", "c.txt", "d.txt", "e.txt", "f.txt"];

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

fn shown_window() -> MainWindow {
    let ui = MainWindow::new().expect("the window should build");
    ui.set_content_rows(ModelRc::new(VecModel::from(
        ROWS.iter().map(|name| row(name)).collect::<Vec<_>>(),
    )));
    ui.show().expect("the window should show");
    ui
}

/// How many menu items carry `label`, counting each item once.
fn menu_items(ui: &MainWindow, label: &str) -> usize {
    ElementHandle::find_by_accessible_label(ui, label)
        .filter(|item| (item.size().width - MENU_WIDTH).abs() < f32::EPSILON)
        .count()
}

/// The vertical middle of row `index`, as it is drawn.
fn row_middle(index: usize) -> f32 {
    let index = u16::try_from(index).expect("the test uses a handful of rows");
    f32::from(index).mul_add(ROW_HEIGHT, ROW_HEIGHT / 2.0)
}

/// Where the pane's clickable body starts, in window coordinates.
fn body_origin(ui: &MainWindow) -> LogicalPosition {
    ElementHandle::find_by_element_id(ui, "ContentsPane::click-area")
        .next()
        .expect("the contents pane has a click area")
        .absolute_position()
}

#[test]
fn a_click_selects_the_row_it_lands_on() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();
    let clicked = Rc::new(Cell::new(usize::MAX));
    ui.on_content_row_clicked({
        let clicked = Rc::clone(&clicked);
        move |i| clicked.set(usize::try_from(i).unwrap_or(usize::MAX))
    });

    let origin = body_origin(&ui);
    let window = ui.window();
    for expected in 0..ROWS.len() {
        // The vertical middle of the row as it is drawn.
        let y = row_middle(expected);
        let position = LogicalPosition::new(origin.x + 20.0, origin.y + y);
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
            expected,
            "a click {y}px down the pane body should select row {expected}"
        );
    }
}

#[test]
fn a_right_click_past_the_last_row_opens_the_empty_area_menu() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();

    let origin = body_origin(&ui);
    // One pixel below the last row is already empty space.
    let y = ROW_COUNT.mul_add(ROW_HEIGHT, 1.0);
    let position = LogicalPosition::new(origin.x + 20.0, origin.y + y);
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

    assert_eq!(
        menu_items(&ui, "New Folder"),
        1,
        "the empty-area menu should be the one that opened"
    );
    assert_eq!(
        menu_items(&ui, "Rename"),
        0,
        "the row menu should not have opened"
    );
}

#[test]
fn a_right_click_on_the_last_row_opens_the_row_menu() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();

    let origin = body_origin(&ui);
    // The middle of the last row, which is still a row.
    let y = row_middle(ROWS.len() - 1);
    let position = LogicalPosition::new(origin.x + 20.0, origin.y + y);
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

    assert_eq!(
        menu_items(&ui, "Rename"),
        1,
        "the row menu should be the one that opened"
    );
}
