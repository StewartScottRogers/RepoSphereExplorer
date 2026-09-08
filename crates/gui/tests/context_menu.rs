//! The contents pane's right-click menus, driven through Slint's own
//! hit-testing rather than the windowing system.
//!
//! These menus were `PopupWindow`s and nothing in them could be chosen. A
//! popup cannot be positioned from its enclosing component; without an
//! explicit size it is laid out 0x0, which leaves its contents drawn but
//! outside the only region that can be hit; and its default close policy
//! dismisses it on the press, before the item's `TouchArea` sees the
//! release. They are ordinary elements of the pane now, so a click reaches
//! them - which is what these tests assert.

use gui::{ContentRow, MainWindow};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, Image, LogicalPosition, ModelRc, SharedString, VecModel};
use std::cell::Cell;
use std::rc::Rc;

/// Row height in `app.slint`'s panes, so a click can be aimed at a row.
const ROW_HEIGHT: f32 = 20.0;

/// Context menu width in `app.slint`. The command bar carries buttons with
/// the same labels as the menu items, so matches are narrowed to elements
/// the width of a menu - otherwise a search for "Rename" finds the toolbar
/// button first.
const MENU_WIDTH: f32 = 150.0;

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
        is_repository: false,
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

/// The open menu's items labelled `label`, ignoring same-named elements
/// elsewhere in the window.
fn menu_items(ui: &MainWindow, label: &str) -> Vec<ElementHandle> {
    ElementHandle::find_by_accessible_label(ui, label)
        .filter(|item| (item.size().width - MENU_WIDTH).abs() < f32::EPSILON)
        .collect()
}

/// Clicks the open menu's item labelled `label`.
fn choose(ui: &MainWindow, label: &str) {
    let items = menu_items(ui, label);
    assert_eq!(
        items.len(),
        1,
        "exactly one open menu item should be labelled {label:?}"
    );
    items[0].mock_single_click(PointerEventButton::Left);
}

#[test]
fn choosing_rename_from_the_row_menu_invokes_its_callback() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();
    let fired = Rc::new(Cell::new(false));
    ui.on_content_rename_requested({
        let fired = Rc::clone(&fired);
        move || fired.set(true)
    });

    right_click(&ui, 0.0);
    choose(&ui, "Rename");

    assert!(
        fired.get(),
        "choosing Rename should reach content-rename-requested"
    );
}

#[test]
fn choosing_new_folder_from_the_empty_area_menu_invokes_its_callback() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();
    let fired = Rc::new(Cell::new(false));
    ui.on_new_folder_requested({
        let fired = Rc::clone(&fired);
        move || fired.set(true)
    });

    // Well past the two rows, which is what opens the empty-area menu.
    right_click(&ui, 20.0);
    choose(&ui, "New Folder");

    assert!(
        fired.get(),
        "choosing New Folder should reach new-folder-requested"
    );
}

#[test]
fn a_menu_opens_only_once_the_pane_is_right_clicked() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();

    assert_eq!(
        menu_items(&ui, "Rename").len(),
        0,
        "no menu before the right-click"
    );

    right_click(&ui, 0.0);

    assert_eq!(
        menu_items(&ui, "Rename").len(),
        1,
        "the row menu is open afterwards"
    );
}
