//! The File pane's tab strip, driven through Slint's own hit-testing.
//!
//! More than one tab gets a strip; one tab gets none. A click has to reach
//! `file-tab-selected` with the index of the tab it landed on: the pane
//! draws whatever the application hands back, so a click that maps to the
//! wrong tab - or to nothing - leaves a strip that only looks like one.
//!
//! What is in the strip is the application's business and is tested in
//! `app`: the plugin's views, then an Edit tab for a file that can be
//! edited, and while the editor is open nothing but `Editing`.

use gui::MainWindow;
use i_slint_backend_testing::ElementHandle;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, SharedString, VecModel};
use std::cell::Cell;
use std::rc::Rc;

/// The strip's height in `app.slint`. Its width is not a constant -
/// the tabs share whatever the File pane is - so it is measured rather
/// than assumed. Assuming it is how a third tab came to be drawn off the
/// right-hand edge of a default-width pane while every test passed.
const TAB_HEIGHT: f32 = 24.0;

/// A shown window whose File pane offers `views` as its tabs.
fn window_with_views(views: &[&str]) -> MainWindow {
    let ui = MainWindow::new().expect("the window should build");
    ui.set_file_tabs(ModelRc::new(VecModel::from(
        views
            .iter()
            .map(|name| SharedString::from(*name))
            .collect::<Vec<_>>(),
    )));
    ui.set_file_text(SharedString::from("some preview"));
    ui.show().expect("the window should show");
    ui
}

/// The switcher itself, or `None` where none is drawn.
///
/// The strip, not a tab's click area: every tab has one of those now, so
/// that a tab knows when the pointer is over it, and the first of them is
/// one tab wide - which is not what a click across the strip is measured
/// against.
fn switcher(ui: &MainWindow) -> Option<ElementHandle> {
    ElementHandle::find_by_element_id(ui, "MainWindow::tab-strip").next()
}

/// Clicks the middle of tab `index`, as it is drawn.
fn click_tab(ui: &MainWindow, index: usize, tabs: usize) {
    let strip = switcher(ui).expect("a switcher should be drawn");
    let origin = strip.absolute_position();
    #[allow(clippy::cast_precision_loss)]
    let width = strip.size().width / tabs as f32;
    let index = u16::try_from(index).expect("the test uses a handful of tabs");
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

/// The view index the window last asked for, after clicking tab `index`.
fn asked_for(ui: &MainWindow, index: usize, tabs: usize) -> i32 {
    let chosen = Rc::new(Cell::new(-1));
    ui.on_file_tab_selected({
        let chosen = Rc::clone(&chosen);
        move |view| chosen.set(view)
    });
    click_tab(ui, index, tabs);
    chosen.get()
}

#[test]
fn a_type_offering_two_views_gets_a_switcher_wide_enough_to_hit() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = window_with_views(&["Preview", "Text"]);

    let strip = switcher(&ui).expect("two views should draw a switcher");

    // Wide enough that each of the two tabs is worth aiming at, and
    // tall enough to hit. A number rather than a multiple of a constant,
    // because the tabs now share whatever width the pane has.
    assert!(
        strip.size().width >= 80.0 && strip.size().height > 0.0,
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
            asked_for(&ui, index, 3),
            i32::try_from(index).expect("a handful of tabs"),
            "clicking tab {index} should ask for view {index}"
        );
    }
}

#[test]
fn one_tab_draws_no_strip_at_all() {
    // This is what keeps an open editor from having anything to click:
    // `App::file_tabs` returns the single `Editing` tab while it is open,
    // and a single tab draws nothing. Switching views under an editor
    // would be a way to lose typed text.
    i_slint_backend_testing::init_no_event_loop();
    let ui = window_with_views(&["Editing"]);

    assert!(
        switcher(&ui).is_none(),
        "one tab is not a choice, so there is nothing to draw"
    );
}

/// Clicking the Edit tab asks for the Edit tab, and not for a view.
///
/// The whole of #495 is that nothing in the pane said an editor existed.
/// This is the click a reader now makes instead of hunting a toolbar
/// button: the last tab, on a file that can be edited. `App::file_tabs`
/// puts it there and `App::select_file_tab` opens the editor on it;
/// this is the middle link, that the strip maps the click to the right
/// index.
#[test]
fn clicking_the_edit_tab_asks_for_the_edit_tab() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = window_with_views(&["Preview", "Text", "Edit"]);

    assert_eq!(
        asked_for(&ui, 2, 3),
        2,
        "the last tab is the way into the editor, so a click on it has to \
         arrive as the last index rather than as a view"
    );
}
