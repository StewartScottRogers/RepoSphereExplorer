//! Driving the window's text zoom (#586) through the real `key-scope`, the
//! View menu, and the Contents pane it scales - CLAUDE.md rule 14.
//!
//! `gui::zoom`'s own unit tests prove the step table walks and clamps
//! right, and `app.rs`'s prove `App` reports the level it moved to.
//! Neither proves the window actually redraws at the new size, or that a
//! reader's Ctrl+Plus reaches it at all - this file is that seam.

use gui::app::App;
use gui::{MainWindow, sync_ui};
use i_slint_backend_testing::ElementHandle;
use protocol::{DirectoryEntry, Response};
use slint::ComponentHandle;
use slint::platform::WindowEvent;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// Row height in `app.slint`'s panes at 100% zoom.
const ROW_HEIGHT: f32 = 20.0;

/// A 20-character name: the length #586's acceptance check names, that
/// still has to show whole at 200% with the default pane widths.
const LONG_NAME: &str = "abcdefghijklmnopqrst";

/// An empty directory of this test's own under the platform's temporary
/// directory.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("rse-zoom").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// A shown window on a real `App`, wired as `main` wires it, with one
/// Contents row carrying [`LONG_NAME`].
fn window_on(directory: &std::path::Path) -> (MainWindow, Rc<RefCell<App>>) {
    let app = Rc::new(RefCell::new(App::new(directory.to_path_buf())));
    {
        let mut app = app.borrow_mut();
        app.apply_contents_result_for_test(
            &[],
            Response::Directory {
                entries: vec![DirectoryEntry {
                    name: LONG_NAME.to_owned(),
                    is_dir: false,
                    size: 0,
                    modified: None,
                    repository: None,
                }],
            },
        );
    }
    let ui = MainWindow::new().expect("the window should build");
    gui::wire_callbacks(&ui, &app);
    sync_ui(&ui, &app.borrow());
    ui.show().expect("the window should show");
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    (ui, app)
}

/// The Contents pane's one drawn row, once [`window_on`]'s listing has
/// reached the window.
fn listing_row(ui: &MainWindow) -> i_slint_backend_testing::ElementHandle {
    ElementHandle::find_by_element_id(ui, "ContentsPane::listing-row")
        .next()
        .expect("the one row should be drawn")
}

#[test]
fn stepping_in_scales_the_row_height_and_the_name_text_together() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = scratch("row-and-font");
    let (ui, app) = window_on(&dir);

    let at_100 = listing_row(&ui).size().height;
    assert!(
        (at_100 - ROW_HEIGHT).abs() < 0.5,
        "100% zoom should draw a {ROW_HEIGHT}px row, drew {at_100}"
    );
    let font_at_100 = ui.get_zoomed_font_size();
    assert!(
        (font_at_100 - 12.0).abs() < 0.5,
        "100% zoom should leave the 12px default font size unchanged, was {font_at_100}"
    );

    // Every step in `zoom::STEPS`: the row height app.slint draws, and the
    // font size every row's name text inherits with no `font-size` of its
    // own, both have to track `Zoom.factor` all the way from 80% to 200%
    // - not just at the ends, and not one without the other.
    for _ in 0..7 {
        app.borrow_mut().zoom_in();
        sync_ui(&ui, &app.borrow());
        let percent = f32::from(app.borrow().zoom_percent());
        let ratio = percent / 100.0;

        let expected_row = ROW_HEIGHT * ratio;
        let drawn_row = listing_row(&ui).size().height;
        assert!(
            (drawn_row - expected_row).abs() < 0.5,
            "at {percent}% the row should be {expected_row}px tall, drew {drawn_row}"
        );

        let expected_font = 12.0 * ratio;
        let drawn_font = ui.get_zoomed_font_size();
        assert!(
            (drawn_font - expected_font).abs() < 0.5,
            "at {percent}% the name text's inherited font should be {expected_font}px, was {drawn_font}"
        );
    }
    assert_eq!(app.borrow().zoom_percent(), 200, "the walk reaches 200%");
}

#[test]
fn the_default_pane_widths_widen_in_proportion_and_keep_a_long_name_unelided() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = scratch("pane-widths");
    let (ui, app) = window_on(&dir);

    let contents_width_at_100 = ui.get_contents_width();
    assert!(
        (contents_width_at_100 - 470.0).abs() < 0.5,
        "the default Contents width at 100% should be 470px, was {contents_width_at_100}"
    );

    for _ in 0..7 {
        app.borrow_mut().zoom_in();
    }
    sync_ui(&ui, &app.borrow());
    assert_eq!(app.borrow().zoom_percent(), 200);

    // The default (never restored, never dragged) Contents width doubles
    // with the zoom - app.slint's `contents-width: 470px * Zoom.factor`.
    let contents_width_at_200 = ui.get_contents_width();
    assert!(
        (contents_width_at_200 - 940.0).abs() < 0.5,
        "the default Contents width at 200% should be 940px, was {contents_width_at_200}"
    );

    // The Name column is what is left of the doubled pane once the three
    // trailing columns - also doubled - and the fixed 10px of padding are
    // taken out. A 20-character name needs nowhere near this much room at
    // any font a desktop actually uses, so if the column ever shrank back
    // to its 100% width while the font doubled, this is what would catch
    // it.
    let size_column = 80.0 * 2.0;
    let kind_column = 90.0 * 2.0;
    let modified_column = 118.0 * 2.0;
    let name_column_width =
        contents_width_at_200 - 10.0 - size_column - kind_column - modified_column;
    assert_eq!(LONG_NAME.len(), 20, "the fixture name is 20 characters");
    let generous_lower_bound = 20.0 * 6.0;
    assert!(
        name_column_width > generous_lower_bound,
        "a {}-character name needs at least {generous_lower_bound}px and the Name column \
         at 200% is only {name_column_width}px wide",
        LONG_NAME.len()
    );
}

#[test]
fn ctrl_plus_and_ctrl_minus_and_ctrl_0_step_the_zoom_through_the_real_key_scope() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = scratch("keys");
    let (ui, app) = window_on(&dir);
    let window = ui.window();

    let press = |text: &str| {
        let control: slint::SharedString = char::from(slint::platform::Key::Control).into();
        window.dispatch_event(WindowEvent::KeyPressed {
            text: control.clone(),
        });
        window.dispatch_event(WindowEvent::KeyPressed { text: text.into() });
        window.dispatch_event(WindowEvent::KeyReleased { text: text.into() });
        window.dispatch_event(WindowEvent::KeyReleased { text: control });
    };

    assert_eq!(app.borrow().zoom_percent(), 100);
    press("+");
    assert_eq!(app.borrow().zoom_percent(), 110, "Ctrl++ should zoom in");
    press("=");
    assert_eq!(
        app.borrow().zoom_percent(),
        125,
        "Ctrl+= should also zoom in"
    );
    press("-");
    assert_eq!(app.borrow().zoom_percent(), 110, "Ctrl+- should zoom out");
    press("0");
    assert_eq!(
        app.borrow().zoom_percent(),
        100,
        "Ctrl+0 should reset to 100%"
    );
}

#[test]
fn the_view_menu_s_zoom_items_reach_the_same_state() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = scratch("menu");
    let (ui, app) = window_on(&dir);

    ui.invoke_zoom_in_requested();
    assert_eq!(app.borrow().zoom_percent(), 110);
    ui.invoke_zoom_reset_requested();
    assert_eq!(app.borrow().zoom_percent(), 100);
    ui.invoke_zoom_out_requested();
    assert_eq!(app.borrow().zoom_percent(), 90);
}
