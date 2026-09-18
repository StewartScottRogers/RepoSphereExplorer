//! The Folders pane, and pane focus, with both halves joined.
//!
//! `folders_pane.rs` measures what the pane draws with no application
//! behind it; `row_hit_mapping.rs` clicks rows pushed straight into the
//! window; `App`'s own unit tests know what a click index means and have
//! never met a window. The half nobody has is the one a reader meets:
//! which focus scope holds the keyboard when they press an arrow key,
//! and whether a pixel in the tree reaches the row it is drawn on.
//!
//! `main`'s `wire_rows`, `wire_commands` and `wire_editor` are private,
//! so the wiring below is a copy of them. That is a hole worth naming: a
//! defect in `main`'s own wiring cannot be seen from here.
//!
//! The application talks to the service over its socket, and `App` has no
//! public way to be handed a listing, so this file starts a service in a
//! background thread (or uses the one already running) and pumps `tick`
//! until the answer arrives - which is what the application's timer does.

use gui::app::App;
use gui::{MainWindow, sync_ui};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

mod common;
use common::ensure_service;

/// A directory of this test's own: two child folders and two files.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("repos-explorer-folders-{name}"));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(directory.join("alpha").join("inner")).expect("a scratch directory");
    std::fs::create_dir_all(directory.join("beta")).expect("a second child folder");
    // Several lines, so a Down arrow that reaches the editor has
    // somewhere to go and says so.
    std::fs::write(
        directory.join("demo.rs"),
        "fn main() {\n    let x = 1;\n    let y = 2;\n}\n",
    )
    .expect("a file to edit");
    std::fs::write(directory.join("notes.txt"), "hello\n").expect("a second file");
    directory
}

// The window's callbacks come from the crate's own wiring, which is what
// `main` calls. A copy here would be a second thing to keep right, and a
// test of a copy proves nothing about what a reader gets.

/// Ticks the application until `done`, the way the window's timer does.
fn pump(ui: &MainWindow, app: &Rc<RefCell<App>>, what: &str, done: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        {
            let mut app = app.borrow_mut();
            app.tick();
            sync_ui(ui, &app);
            if done(&app) {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the service never produced {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A shown window, wired to an application listing `directory`, with the
/// root's own listing already in.
fn window_on(directory: &Path) -> (MainWindow, Rc<RefCell<App>>) {
    ensure_service();
    let app = Rc::new(RefCell::new(App::new(directory.to_path_buf())));
    let ui = MainWindow::new().expect("the window should build");
    gui::wire_callbacks(&ui, &app);
    sync_ui(&ui, &app.borrow());
    ui.show().expect("the window should show");
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    pump(&ui, &app, "a listing of the root", |app| {
        !app.content_rows().is_empty() && app.folder_rows().len() > 1
    });
    (ui, app)
}

fn window(name: &str) -> (MainWindow, Rc<RefCell<App>>, PathBuf) {
    let directory = scratch(name);
    let (ui, app) = window_on(&directory);
    (ui, app, directory)
}

/// The folders pane's row rectangles, in the order they are drawn.
fn folder_rows(ui: &MainWindow) -> Vec<ElementHandle> {
    ElementHandle::find_by_element_id(ui, "FoldersPane::tree-row").collect()
}

/// Clicks the window at an absolute pixel, the way a pointer does.
fn click_at(ui: &MainWindow, x: f32, y: f32) {
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
}

/// Clicks `offset` pixels in from the left edge of folders row `row`.
fn click_row(ui: &MainWindow, row: usize, offset: f32) {
    let rows = folder_rows(ui);
    let handle = rows.get(row).expect("the pane draws that row");
    let at = handle.absolute_position();
    let size = handle.size();
    click_at(ui, at.x + offset, at.y + size.height / 2.0);
}

/// Presses and releases a key on the window, the way a keyboard does.
fn press(ui: &MainWindow, key: Key) {
    let text = slint::SharedString::from(char::from(key).to_string());
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

/// A typed character, as a keyboard sends one.
fn press_text(ui: &MainWindow, text: &str) {
    let text = slint::SharedString::from(text);
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

/// Opens the editor on `demo.rs`, the way the Edit tab does, and clicks
/// the surface - which is what gives it the keyboard.
fn open_the_editor(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    let row = {
        let app = app.borrow();
        app.content_rows()
            .iter()
            .position(|row| row.name == "demo.rs")
            .expect("the fixture file is listed")
    };
    app.borrow_mut().select_content(row);
    pump(ui, app, "a preview of demo.rs", App::can_edit);
    {
        let mut app = app.borrow_mut();
        app.begin_file_edit();
        assert!(app.editing_file(), "the editor is open");
        assert!(app.editing_in_colour(), "and it is the coloured surface");
    }
    sync_ui(ui, &app.borrow());
    let body = ElementHandle::find_by_element_id(ui, "CodeEditor::body")
        .next()
        .expect("the editing surface is drawn");
    let at = body.absolute_position();
    click_at(ui, at.x + 4.0, at.y + 8.0);
}

#[test]
fn the_pane_draws_a_row_for_every_folder_the_application_reports() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app, _) = window("rows");

    assert_eq!(
        folder_rows(&ui).len(),
        app.borrow().folder_rows().len(),
        "the pane and the application agree on how many rows there are"
    );
    assert!(
        folder_rows(&ui).len() >= 3,
        "the root and its two child folders"
    );
}

#[test]
fn a_real_click_on_a_folder_row_selects_that_row() {
    // A pixel, through Slint's row arithmetic, to the index
    // `click_folder` acts on.
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app, _) = window("click");

    click_row(&ui, 2, 80.0);

    assert_eq!(
        app.borrow().folder_selected(),
        2,
        "the click reached the third row, not the first"
    );
}

#[test]
fn a_real_click_on_a_chevron_collapses_the_row_it_is_on() {
    // `chevron_hit` is unit tested on its own, and `folders_pane`
    // measures the indent. What neither can say is whether the x the
    // pane reports is measured from the same edge `chevron_hit`
    // measures from. A pane border or a scroll view's padding in
    // between, and every chevron click misses.
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app, _) = window("chevron");
    assert!(
        app.borrow().folder_rows()[0].expanded,
        "the root starts expanded"
    );

    // The chevron column of a depth-0 row runs from 4 to 20 pixels in.
    click_row(&ui, 0, 12.0);

    assert!(
        !app.borrow().folder_rows()[0].expanded,
        "a click on the chevron collapsed the root"
    );
}

#[test]
fn a_real_click_on_a_child_chevron_lands_on_that_chevron() {
    // A child's chevron is one indent further in. This is the click
    // `folders_pane`'s indent test exists to protect, driven for real.
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app, _) = window("child-chevron");
    // A row only grows a chevron once its own listing has been in, so
    // select it first - which is what a reader does.
    click_row(&ui, 1, 80.0);
    pump(&ui, &app, "a listing of alpha", |app| {
        app.folder_rows()[1].expandable
    });

    click_row(&ui, 1, 28.0);

    assert!(
        app.borrow().folder_rows()[1].expanded,
        "a click on the child's chevron expanded it"
    );
}

#[test]
fn a_real_click_on_a_name_does_not_collapse_the_row() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app, _) = window("name");

    click_row(&ui, 0, 80.0);

    assert!(
        app.borrow().folder_rows()[0].expanded,
        "the name selects; only the chevron folds"
    );
}

#[test]
fn a_real_arrow_key_moves_the_folders_selection() {
    // The baseline for every focus test below: with nothing else going
    // on, a key pressed on the window reaches the window's own focus
    // scope and moves the tree.
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app, _) = window("arrow");
    assert_eq!(app.borrow().focus_index(), 0, "focus starts on the tree");

    press(&ui, Key::DownArrow);

    assert_eq!(
        app.borrow().folder_selected(),
        1,
        "Down moved the selection in the tree"
    );
}

#[test]
fn the_arrow_keys_still_work_after_a_row_has_been_clicked() {
    // The most ordinary thing anybody does in a tree: click a folder,
    // then walk with the arrow keys. The pane's rows are a `TouchArea`,
    // which takes no keyboard focus of its own, so this asks whether a
    // click leaves the window's focus scope holding the keyboard.
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app, _) = window("click-then-key");

    click_row(&ui, 1, 80.0);
    assert_eq!(app.borrow().folder_selected(), 1, "the click selected it");

    press(&ui, Key::DownArrow);

    assert_eq!(
        app.borrow().folder_selected(),
        2,
        "Down after a click has to keep walking the tree"
    );
}

#[test]
fn a_real_right_arrow_moves_focus_on_and_the_window_is_told() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app, _) = window("cycle");

    press(&ui, Key::RightArrow);

    assert_eq!(app.borrow().focus_index(), 1, "focus moved to the contents");
    assert_eq!(
        ui.get_focus_pane(),
        1,
        "and the window was told, so the accent moves with it"
    );
}

#[test]
fn clicking_a_folder_row_while_the_editor_is_open_gives_the_tree_the_keyboard() {
    // The reader is editing a file and clicks a folder in the tree to go
    // somewhere else. The application marks the Folders pane focused and
    // draws the accent there - but the keyboard was handed to the
    // editing surface's own focus scope when it was clicked. If the next
    // arrow key does not move the tree, the pane that looks focused is
    // not the one listening.
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app, _) = window("editor-then-tree");
    open_the_editor(&ui, &app);

    click_row(&ui, 1, 80.0);
    assert_eq!(
        app.borrow().focus_index(),
        0,
        "the click put the application's focus on the tree"
    );
    let before = app.borrow().folder_selected();
    let caret = app.borrow().edit_caret();

    press(&ui, Key::DownArrow);

    assert_eq!(
        app.borrow().edit_caret(),
        caret,
        "the key moved the caret in a file the reader has stopped looking \
         at. The Folders pane is drawn as the focused one and the editing \
         surface is the one holding the keyboard."
    );
    assert_ne!(
        app.borrow().folder_selected(),
        before,
        "Down had to move the tree's selection, because the tree is the \
         pane the click focused"
    );
}

#[test]
fn escape_closes_the_editor_when_the_editor_has_the_keyboard() {
    // Escape is the way out of the editor, and the window's focus scope
    // is what listens for it. The surface's own scope accepts every key
    // unconditionally, so anything it does not use can never get past
    // it.
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app, _) = window("escape");
    open_the_editor(&ui, &app);

    press(&ui, Key::Escape);

    assert!(
        !app.borrow().editing_file(),
        "Escape had to close the editor; the editor makes no use of it, \
         so it has to reach the window"
    );
}

#[test]
fn the_keyboard_still_reaches_the_window_after_the_editor_closes() {
    // The editing surface takes the keyboard when it is clicked. When it
    // is destroyed - saved, cancelled, or the file deselected - nothing
    // gives the keyboard back, and a window with no focused element
    // answers no key at all.
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app, _) = window("after-editor");
    open_the_editor(&ui, &app);

    app.borrow_mut().cancel_file_edit();
    assert!(!app.borrow().editing_file(), "the editor is shut");
    sync_ui(&ui, &app.borrow());

    let before = app.borrow().focus_index();
    press(&ui, Key::RightArrow);

    assert_ne!(
        app.borrow().focus_index(),
        before,
        "an arrow key after the editor closed has to reach the window's \
         focus scope. If it does not, the window is deaf to the keyboard \
         until it is closed and opened again."
    );
}

#[test]
fn the_selected_folder_row_is_brought_on_screen() {
    // `scroll_offset_for` is unit tested against numbers. What nothing
    // checks is whether the number it is given - the pane's viewport
    // height - is the pane's real one.
    i_slint_backend_testing::init_no_event_loop();
    let directory = std::env::temp_dir().join("repos-explorer-folders-scroll");
    let _ = std::fs::remove_dir_all(&directory);
    for index in 0..60 {
        std::fs::create_dir_all(directory.join(format!("folder-{index:02}")))
            .expect("a long fixture tree");
    }
    let (ui, app) = window_on(&directory);

    assert!(
        ui.get_folders_viewport_height() > 0.0,
        "the pane has to report how much of the listing it is showing; it \
         reported {}",
        ui.get_folders_viewport_height()
    );

    app.borrow_mut().select_folder(55);
    sync_ui(&ui, &app.borrow());

    assert!(
        ui.get_folders_scroll_y() < 0.0,
        "selecting a row far down the tree has to scroll the pane to it; \
         the offset stayed at {}",
        ui.get_folders_scroll_y()
    );
}

/// A typed letter reaches the pane that is drawn as the focused one.
///
/// The window opens on the tree, and a letter used to move the *listing*
/// regardless - so the arrows walked the tree while typing jumped
/// something the reader was not looking at. The tree gets type-ahead of
/// its own rather than going silent: a Repos Directory is a list of
/// repository folders, and three letters is how anybody reaches one.
#[test]
fn a_typed_letter_with_the_tree_focused_moves_the_tree() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = scratch("tree-type-ahead");
    for name in ["alpha", "bravo", "charlie"] {
        std::fs::create_dir_all(directory.join(name)).expect("the fixture is written");
    }
    let (ui, app) = window_on(&directory);
    pump(&ui, &app, "the tree's children", |app| {
        app.folder_rows().len() >= 4
    });
    assert_eq!(
        app.borrow().folder_selected(),
        0,
        "the tree starts on its root, which is the focused pane"
    );
    let listing_before = app.borrow().content_selected();

    press_text(&ui, "c");

    let selected = app.borrow().folder_selected();
    let name = app
        .borrow()
        .folder_rows()
        .get(selected)
        .map(|row| row.name.clone())
        .unwrap_or_default();
    assert_eq!(
        name, "charlie",
        "a letter typed with the tree focused should jump the tree to the \
         next folder beginning with it; it landed on row {selected}"
    );
    assert_eq!(
        app.borrow().content_selected(),
        listing_before,
        "and the listing, which is not the focused pane, does not move"
    );
}

/// Type-ahead in the tree walks what is drawn, not what is hidden: a
/// collapsed folder's children are not part of the list the reader can
/// see, so a letter must not jump into them.
#[test]
fn tree_type_ahead_only_reaches_rows_that_are_drawn() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = scratch("tree-type-ahead-hidden");
    std::fs::create_dir_all(directory.join("alpha").join("zulu")).expect("the fixture is written");
    std::fs::create_dir_all(directory.join("bravo")).expect("the fixture is written");
    let (ui, app) = window_on(&directory);
    pump(&ui, &app, "the tree's children", |app| {
        app.folder_rows().len() >= 3
    });
    let drawn = app.borrow().folder_rows().len();

    press_text(&ui, "z");

    assert_eq!(
        app.borrow().folder_rows().len(),
        drawn,
        "nothing should have been expanded"
    );
    assert_eq!(
        app.borrow().folder_selected(),
        0,
        "zulu is inside a collapsed folder, so it is not on screen and a \
         typed letter has nothing to jump to"
    );
}
