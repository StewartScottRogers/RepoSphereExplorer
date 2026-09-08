//! The File pane's view switcher, driven through Slint's own hit-testing.
//!
//! A type that offers more than one view gets a tab per view; a type that
//! offers one gets no switcher at all, so the single-view types look exactly
//! as they did before. A click has to reach `file-view-selected` with the
//! index of the tab it landed on: the pane draws whichever view the
//! application hands back, so a click that maps to the wrong tab - or to
//! nothing - leaves a switcher that only looks like one.

use gui::MainWindow;
use i_slint_backend_testing::ElementHandle;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, SharedString, VecModel};
use std::cell::Cell;
use std::rc::Rc;

/// Tab metrics in `app.slint`, which the click mapping is derived from.
const TAB_WIDTH: f32 = 80.0;
const TAB_HEIGHT: f32 = 24.0;

/// A shown window whose File pane is previewing a type offering `views`.
fn window_with_views(views: &[&str]) -> MainWindow {
    let ui = MainWindow::new().expect("the window should build");
    ui.set_file_views(ModelRc::new(VecModel::from(
        views
            .iter()
            .map(|name| SharedString::from(*name))
            .collect::<Vec<_>>(),
    )));
    ui.set_file_text(SharedString::from("some preview"));
    ui.show().expect("the window should show");
    ui
}

/// The switcher's click area, or `None` where no switcher is drawn.
fn switcher(ui: &MainWindow) -> Option<ElementHandle> {
    ElementHandle::find_by_element_id(ui, "MainWindow::tab-touch").next()
}

/// Clicks the middle of tab `index`, as it is drawn.
fn click_tab(ui: &MainWindow, index: usize) {
    let strip = switcher(ui).expect("a switcher should be drawn");
    let origin = strip.absolute_position();
    let index = u16::try_from(index).expect("the test uses a handful of tabs");
    let position = LogicalPosition::new(
        origin.x + f32::from(index).mul_add(TAB_WIDTH, TAB_WIDTH / 2.0),
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

/// The view index the window last asked for, after clicking tab `index`.
fn asked_for(ui: &MainWindow, index: usize) -> i32 {
    let chosen = Rc::new(Cell::new(-1));
    ui.on_file_view_selected({
        let chosen = Rc::clone(&chosen);
        move |view| chosen.set(view)
    });
    click_tab(ui, index);
    chosen.get()
}

#[test]
fn a_type_offering_two_views_gets_a_switcher_wide_enough_to_hit() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = window_with_views(&["Preview", "Text"]);

    let strip = switcher(&ui).expect("two views should draw a switcher");

    assert!(
        strip.size().width >= 2.0 * TAB_WIDTH && strip.size().height > 0.0,
        "a strip laid out at {:?} could not be clicked",
        strip.size()
    );
}

#[test]
fn a_type_offering_one_view_gets_no_switcher() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = window_with_views(&["Preview"]);

    assert!(
        switcher(&ui).is_none(),
        "a single-view type should look as it always did"
    );
}

#[test]
fn clicking_a_tab_asks_for_the_view_it_landed_on() {
    i_slint_backend_testing::init_no_event_loop();

    for index in 0..3 {
        let ui = window_with_views(&["Preview", "Text", "Outline"]);
        assert_eq!(
            asked_for(&ui, index),
            i32::try_from(index).expect("a handful of tabs"),
            "clicking tab {index} should ask for view {index}"
        );
    }
}

#[test]
fn the_editor_replaces_the_switcher_rather_than_sitting_under_it() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = window_with_views(&["Preview", "Text"]);
    ui.set_editing_file(true);

    assert!(
        switcher(&ui).is_none(),
        "an open editor is the pane; switching views under it would be a way to lose typed text"
    );
}
