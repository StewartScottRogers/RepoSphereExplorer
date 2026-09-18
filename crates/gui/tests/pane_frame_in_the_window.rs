//! #615: the three panes now sit inside a shared `PaneFrame` rather than
//! each drawing its own title and border. This is the seam a unit test on
//! any one pane's `.slint` cannot reach: whether the real window actually
//! nests each pane inside a `PaneFrame`, and whether that frame's border
//! actually follows which pane holds the keyboard, driven through the
//! real `App` the way `main` wires it (rule 14).

use gui::app::App;
use gui::{MainWindow, sync_ui};
use i_slint_backend_testing::ElementHandle;
use slint::ComponentHandle;
use slint::platform::WindowEvent;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// An empty directory of this test's own under the platform's temporary
/// directory.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("rse-pane-frame").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// A shown window on a real `App`, wired as `main` wires it.
fn window_on(directory: &std::path::Path) -> (MainWindow, Rc<RefCell<App>>) {
    let app = Rc::new(RefCell::new(App::new(directory.to_path_buf())));
    let ui = MainWindow::new().expect("the window should build");
    gui::wire_callbacks(&ui, &app);
    sync_ui(&ui, &app.borrow());
    ui.show().expect("the window should show");
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    (ui, app)
}

/// Every `PaneFrame::title-row` on screen: one per pane, wherever a
/// `PaneFrame` is nested into.
fn title_rows(ui: &MainWindow) -> Vec<ElementHandle> {
    ElementHandle::find_by_element_id(ui, "PaneFrame::title-row").collect()
}

#[test]
fn each_of_the_three_panes_is_drawn_inside_its_own_pane_frame() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = scratch("three-frames");
    let (ui, _app) = window_on(&dir);

    let rows = title_rows(&ui);
    assert_eq!(
        rows.len(),
        3,
        "the Folders, Contents and File panes should each nest a PaneFrame, found {}",
        rows.len()
    );

    // Each pane's own root sits directly under its `PaneFrame`, so a
    // frame's title row shares the left edge of the pane it belongs to -
    // proof that the frame is the one wrapping that pane, not some other.
    let folders_left = ElementHandle::find_by_element_type_name(&ui, "FoldersPane")
        .next()
        .expect("the Folders pane is drawn")
        .absolute_position()
        .x;
    let contents_left = ElementHandle::find_by_element_type_name(&ui, "ContentsPane")
        .next()
        .expect("the Contents pane is drawn")
        .absolute_position()
        .x;
    let file_left = ElementHandle::find_by_element_type_name(&ui, "FilePane")
        .next()
        .expect("the File pane is drawn")
        .absolute_position()
        .x;

    let mut row_lefts: Vec<f32> = rows.iter().map(|row| row.absolute_position().x).collect();
    row_lefts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut pane_lefts = [folders_left, contents_left, file_left];
    pane_lefts.sort_by(|a, b| a.partial_cmp(b).unwrap());

    for (row_left, pane_left) in row_lefts.iter().zip(pane_lefts.iter()) {
        assert!(
            (row_left - pane_left).abs() < 0.5,
            "a PaneFrame's title row should start where its pane starts: \
             row at {row_left}, pane at {pane_left}"
        );
    }
}

#[test]
fn a_pane_frames_border_follows_which_pane_the_keyboard_is_in() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = scratch("border-follows-focus");
    let (ui, app) = window_on(&dir);

    let accent = ui.global::<gui::Theme>().get_accent();
    let unfocused = ui.global::<gui::Theme>().get_border();

    // Folders starts focused (`App::new`'s default), so its frame's
    // border should already be drawn in the accent and the other two
    // should not.
    assert_eq!(app.borrow().focus_index(), 0, "Folders is focused first");
    assert_eq!(ui.get_folders_frame_border_colour(), accent);
    assert_eq!(ui.get_contents_frame_border_colour(), unfocused);
    assert_eq!(ui.get_file_frame_border_colour(), unfocused);

    // One step right moves the keyboard to Contents; its frame takes the
    // accent border and Folders' frame gives it up.
    app.borrow_mut().cycle_focus(1);
    sync_ui(&ui, &app.borrow());
    assert_eq!(app.borrow().focus_index(), 1, "Contents is focused now");
    assert_eq!(ui.get_folders_frame_border_colour(), unfocused);
    assert_eq!(ui.get_contents_frame_border_colour(), accent);
    assert_eq!(ui.get_file_frame_border_colour(), unfocused);

    // One more step moves it to the File pane's frame.
    app.borrow_mut().cycle_focus(1);
    sync_ui(&ui, &app.borrow());
    assert_eq!(app.borrow().focus_index(), 2, "File is focused now");
    assert_eq!(ui.get_folders_frame_border_colour(), unfocused);
    assert_eq!(ui.get_contents_frame_border_colour(), unfocused);
    assert_eq!(ui.get_file_frame_border_colour(), accent);
}
