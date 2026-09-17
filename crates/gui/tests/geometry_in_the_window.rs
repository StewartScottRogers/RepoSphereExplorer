//! Remembering the window's own geometry, driven through a real window
//! wired by the same function the application uses.
//!
//! The rules themselves are unit-tested twice over: `settings` decides
//! whether a remembered value is usable and where a window off every
//! display should go, and `GeometryTracker` decides what to save and when
//! a window needs putting back. Neither has ever met a window. This file
//! measures the seam between them and `gui::wire_window_geometry`, which
//! is where a remembered layout is actually applied - per rule 14, and
//! because the fault this feature nearly shipped with (a maximised window
//! never correcting bounds that named a display since unplugged) lived in
//! the wiring while both halves were green.

use gui::MainWindow;
use gui::settings::WindowGeometry;
use slint::ComponentHandle;

/// A remembered layout, at `x` with a fixed size.
const fn remembered(x: f32, maximized: bool) -> WindowGeometry {
    WindowGeometry {
        x,
        y: 120.0,
        width: 900.0,
        height: 640.0,
        maximized,
    }
}

/// A window wired the way `main` wires it, opening at `geometry`.
fn window_opening_at(geometry: Option<WindowGeometry>) -> MainWindow {
    i_slint_backend_testing::init_no_event_loop();
    let ui = MainWindow::new().expect("the window should build");
    let tracker = gui::wire_window_geometry(&ui, geometry);
    // The tracker is what `main` keeps; the window is what it shows.
    assert_eq!(
        tracker.borrow().closing_at(false),
        geometry.map(|geometry| WindowGeometry {
            maximized: false,
            ..geometry
        }),
        "the wiring should start from what was remembered"
    );
    ui.show().expect("the window should show");
    ui
}

#[test]
fn a_remembered_size_is_applied_to_the_window_before_it_is_shown() {
    let ui = window_opening_at(Some(remembered(300.0, false)));

    let size = ui.window().size().to_logical(ui.window().scale_factor());
    assert!(
        (size.width - 900.0).abs() < 1.0 && (size.height - 640.0).abs() < 1.0,
        "the window should open at the remembered size; it opened at {size:?}"
    );
}

#[test]
fn what_is_saved_is_what_the_window_has_now() {
    let ui = window_opening_at(Some(remembered(300.0, false)));
    let tracker = gui::wire_window_geometry(&ui, Some(remembered(300.0, false)));

    gui::observe_window_geometry(&ui, &tracker);
    let saved = gui::geometry_to_save(&ui, &tracker).expect("a window has geometry to save");

    assert!(!saved.maximized, "this window is not maximised");
    assert!(
        (saved.width - 900.0).abs() < 1.0,
        "the saved width should be the window's own; it was {}",
        saved.width
    );
}

#[test]
fn with_nothing_remembered_the_window_keeps_its_own_size_and_still_saves_it() {
    let ui = window_opening_at(None);
    let tracker = gui::wire_window_geometry(&ui, None);

    gui::observe_window_geometry(&ui, &tracker);
    let saved = gui::geometry_to_save(&ui, &tracker).expect("a window has geometry to save");

    assert!(
        saved.width > 0.0 && saved.height > 0.0,
        "a first run saves the size the window chose for itself: {saved:?}"
    );
}

#[test]
fn nothing_saved_for_the_window_names_a_folder_or_a_file() {
    let ui = window_opening_at(Some(remembered(300.0, false)));
    let tracker = gui::wire_window_geometry(&ui, Some(remembered(300.0, false)));
    gui::observe_window_geometry(&ui, &tracker);

    let saved = gui::geometry_to_save(&ui, &tracker).expect("a window has geometry to save");

    // D7: every launch opens at the Repos Directory. The layout is
    // remembered; where the reader was is not. `WindowGeometry` holds four
    // numbers and a flag, and this test fails the moment a path is added.
    let written = format!("{saved:?}");
    assert!(
        !written.contains('\\') && !written.contains('/'),
        "the remembered geometry must name no path: {written}"
    );
}
