//! Which handler a keystroke reaches while the editor is open.
//!
//! `editing_in_the_window` proved that a letter typed into the window
//! lands at the caret. It never asked the other half of that question:
//! what becomes of the keys the editor does *not* want. The window has a
//! focus scope of its own - Escape, F2, Ctrl+N, Ctrl+S, Alt+arrows - and
//! the editing surface has another, nested inside it. Whether a shortcut
//! works while a file is open is settled entirely by the join between
//! the two, and nothing in this crate has looked at the join.
//!
//! So: a real `MainWindow`, a real `App`, and `gui::wire_callbacks` - the
//! same function `main` calls - rather than a copy of it. A copy would
//! prove nothing about what a reader gets (rule 14): what a failure needs
//! to say is not just what happened but which handler made it happen, so
//! `gui::begin_command_log_for_test` records the name of every
//! window-level command `wire_callbacks` itself dispatches.

use gui::app::App;
use gui::{MainWindow, sync_ui};
use slint::ComponentHandle;
use slint::platform::{Key, WindowEvent};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// A directory of this test's own.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("repos-explorer-keys-{name}"));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    directory
}

/// The window with `demo.rs` selected but the editor closed.
fn browsing(name: &str, text: &str) -> (MainWindow, Rc<RefCell<App>>) {
    let directory = scratch(name);
    std::fs::write(directory.join("demo.rs"), text).expect("the fixture is written");

    let app = Rc::new(RefCell::new(App::new(directory.clone())));
    {
        let mut app = app.borrow_mut();
        let entries = service::list_directory(&directory).expect("the directory lists");
        app.apply_contents_result_for_test(&[], protocol::Response::Directory { entries });
        app.select_content(0);
        let view = service::view_file(&directory.join("demo.rs")).expect("the file opens");
        app.show_file_view_for_test(view);
    }

    let ui = MainWindow::new().expect("the window should build");
    gui::begin_command_log_for_test();
    gui::wire_callbacks(&ui, &app);
    sync_ui(&ui, &app.borrow());
    ui.show().expect("the window should show");
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    (ui, app)
}

/// The same window, with the file open in the editor.
fn editing(name: &str, text: &str) -> (MainWindow, Rc<RefCell<App>>) {
    let (ui, app) = browsing(name, text);
    {
        let mut app = app.borrow_mut();
        app.begin_file_edit();
        assert!(
            app.editing_in_colour(),
            "the coloured surface is the one under test"
        );
    }
    sync_ui(&ui, &app.borrow());
    (ui, app)
}

/// A real keystroke, through the window, with no modifier held.
fn press(ui: &MainWindow, text: &str) {
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.into() });
}

fn press_key(ui: &MainWindow, key: Key) {
    press(ui, &char::from(key).to_string());
}

/// A real keystroke with a modifier held down, pressed and released
/// around it the way a keyboard sends one. Slint takes its modifier
/// state from these events, so a shortcut cannot be dispatched any other
/// way.
fn press_with(ui: &MainWindow, modifier: Key, text: &str) {
    let held = char::from(modifier).to_string();
    let window = ui.window();
    window.dispatch_event(WindowEvent::KeyPressed {
        text: held.as_str().into(),
    });
    window.dispatch_event(WindowEvent::KeyPressed { text: text.into() });
    window.dispatch_event(WindowEvent::KeyReleased {
        text: held.as_str().into(),
    });
}

fn press_control(ui: &MainWindow, text: &str) {
    press_with(ui, Key::Control, text);
}

/// Clicks the window at an absolute pixel, the way a pointer does.
fn click_at(ui: &MainWindow, x: f32, y: f32) {
    let position = slint::LogicalPosition::new(x, y);
    let window = ui.window();
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: slint::platform::PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: slint::platform::PointerEventButton::Left,
    });
}

/// Clicks the command button whose label reads `label`.
///
/// Every `CommandButton` draws its label in a `Text`, which Slint gives
/// an accessible label of its own, and the button's touch area covers
/// it - so finding the words and clicking them is what a reader does.
fn click_command(ui: &MainWindow, label: &str) {
    let button = i_slint_backend_testing::ElementHandle::find_by_accessible_label(ui, label)
        .next()
        .unwrap_or_else(|| panic!("a command button labelled {label:?} is on screen"));
    let at = button.absolute_position();
    let size = button.size();
    click_at(ui, at.x + size.width / 2.0, at.y + size.height / 2.0);
}

/// Clicks the command button labelled `label` that is inside the File
/// pane, rather than the one in the window's toolbar.
///
/// The toolbar and the editor's own row both offer an Undo, spelled the
/// same way, and the toolbar's comes first in the tree - so a test that
/// asked only for the words would always get the toolbar's and never
/// learn anything about the row.
fn click_pane_command(ui: &MainWindow, label: &str) {
    let pane = i_slint_backend_testing::ElementHandle::find_by_element_type_name(ui, "FilePane")
        .next()
        .expect("the File pane is drawn");
    let left = pane.absolute_position().x;
    let button = i_slint_backend_testing::ElementHandle::find_by_accessible_label(ui, label)
        .find(|element| element.absolute_position().x >= left)
        .unwrap_or_else(|| panic!("the File pane offers a {label:?} button"));
    let at = button.absolute_position();
    let size = button.size();
    click_at(ui, at.x + size.width / 2.0, at.y + size.height / 2.0);
}

// -- 1. Escape ------------------------------------------------------------

/// Escape is the way out of an editor, and the window is what knows it:
/// `cancel-requested` runs `cancel_pending`, which discards the edit and
/// says so in the status bar. With the editor open the surface's own
/// focus scope sees the key first, and if it keeps it there is no
/// keyboard way out at all.
#[test]
fn escape_while_editing_discards_the_edit() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("escape", "fn main() {}\n");

    press(&ui, "x");
    assert_eq!(app.borrow().edit_text(), "xfn main() {}\n", "typing works");

    press_key(&ui, Key::Escape);

    assert!(
        !app.borrow().editing_file(),
        "Escape has to close the editor. The window's focus scope never \
         saw the key: the editing surface accepts every key, and the \
         editor itself does nothing with Escape, so the keystroke is \
         dropped between them. The window was asked for {:?}",
        gui::command_log_for_test()
    );
}

/// The same thing said the other way round, so a failure says whether
/// the key was swallowed or merely mishandled.
#[test]
fn escape_while_editing_reaches_the_windows_focus_scope() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, _app) = editing("escape-reaches", "fn main() {}\n");

    press_key(&ui, Key::Escape);

    assert!(
        gui::command_log_for_test().contains(&"cancel".to_owned()),
        "the window's key handler never ran for Escape; it saw {:?}",
        gui::command_log_for_test()
    );
}

/// And with the editor closed it still does, which is the control: the
/// wiring in this file is wiring that works.
#[test]
fn escape_without_the_editor_reaches_the_window() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, _app) = browsing("escape-closed", "fn main() {}\n");

    press_key(&ui, Key::Escape);

    assert!(
        gui::command_log_for_test().contains(&"cancel".to_owned()),
        "with no editor open Escape reaches the window: {:?}",
        gui::command_log_for_test()
    );
}

// -- 2. The window's other shortcuts --------------------------------------

/// The shortcuts that act on the Contents pane belong to the pane, and
/// while a file is being typed into they should stay out of the way.
/// This is the half of the join that is *meant* to swallow.
#[test]
fn the_contents_panes_shortcuts_stay_out_of_the_editors_way() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, _app) = editing("contents-keys", "fn main() {}\n");

    press_key(&ui, Key::F2);
    press_control(&ui, "n");
    press_with(&ui, Key::Alt, &char::from(Key::LeftArrow).to_string());
    press_key(&ui, Key::DownArrow);

    assert!(
        gui::command_log_for_test().is_empty(),
        "F2, Ctrl+N, Alt+Left and the arrows must not move the Contents \
         pane while a file is open in the editor, but the window saw {:?}",
        gui::command_log_for_test()
    );
}

/// Ctrl+S is the other half. It is not a Contents-pane shortcut, it is
/// the editor's own, and the editor has no handler for it - so if the
/// surface keeps the key, the most ordinary thing a reader can do to an
/// open file has no keyboard at all.
#[test]
fn ctrl_s_while_editing_saves_the_file() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("ctrl-s", "fn main() {}\n");

    press(&ui, "x");
    press_control(&ui, "s");

    assert!(
        gui::command_log_for_test().contains(&"save".to_owned()),
        "Ctrl+S has to save. The editing surface accepted the key and did \
         nothing with it, so the window's Save never ran. The window was \
         asked for {:?}",
        gui::command_log_for_test()
    );
    assert!(!app.borrow().editing_file(), "and saving closes the editor");
}

/// F5 likewise: a key the editor has no use for, on a window that does.
#[test]
fn f5_while_editing_still_refreshes() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, _app) = editing("f5", "fn main() {}\n");

    press_key(&ui, Key::F5);

    assert!(
        gui::command_log_for_test().contains(&"refresh".to_owned()),
        "F5 is not an editor key; it should reach the window. It saw {:?}",
        gui::command_log_for_test()
    );
}

// -- 3. Ctrl+Z ------------------------------------------------------------

/// The pane's own Undo undoes typing. The keyboard's Ctrl+Z has to mean
/// the same thing, or it reaches the window's handler and asks the
/// service to undo the last *file operation* - a rename or a delete
/// somewhere else entirely - while the reader is looking at their text.
#[test]
fn ctrl_z_while_editing_undoes_the_typing() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("ctrl-z", "fn main() {}\n");

    press(&ui, "x");
    assert_eq!(app.borrow().edit_text(), "xfn main() {}\n");

    press_control(&ui, "z");

    assert_eq!(
        app.borrow().edit_text(),
        "fn main() {}\n",
        "Ctrl+Z undoes the letter that was typed"
    );
    assert!(
        !gui::command_log_for_test().contains(&"undo".to_owned()),
        "and it must not reach the window's Undo, which undoes a file \
         operation on disk. The window was asked for {:?}",
        gui::command_log_for_test()
    );
    assert_ne!(
        app.borrow().status_text(),
        "undoing...",
        "the status line is the tell: that is the file-operation undo"
    );
}

/// Ctrl+Y is redo in the editor, and the window has no handler for it,
/// so this only asks that the editor got it.
#[test]
fn ctrl_y_while_editing_redoes_the_typing() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("ctrl-y", "fn main() {}\n");

    press(&ui, "x");
    press_control(&ui, "z");
    press_control(&ui, "y");

    assert_eq!(
        app.borrow().edit_text(),
        "xfn main() {}\n",
        "redo puts the letter back"
    );
}

/// With the editor closed, Ctrl+Z is the file-operation undo again.
#[test]
fn ctrl_z_without_the_editor_undoes_a_file_operation() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, _app) = browsing("ctrl-z-closed", "fn main() {}\n");

    press_control(&ui, "z");

    assert!(
        gui::command_log_for_test().contains(&"undo".to_owned()),
        "with no editor open Ctrl+Z is the window's undo: {:?}",
        gui::command_log_for_test()
    );
}

// -- 4. Save from the pane's own row --------------------------------------

/// The pane's row and the toolbar both offer Save. A reader who uses one
/// has to get what the other gives.
#[test]
fn save_from_the_panes_own_row_writes_the_file() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("row-save", "fn main() {}\n");

    press(&ui, "x");
    assert!(ui.get_edit_modified(), "the row's Save is enabled now");

    // The row's own button reads "Save *" once the text has changed,
    // which is what tells it apart from the File menu's.
    click_command(&ui, "Save *");

    assert!(
        gui::command_log_for_test().contains(&"save".to_owned()),
        "the pane's Save asks the same of the window as the File menu's \
         does; it asked {:?}",
        gui::command_log_for_test()
    );
    assert!(!app.borrow().editing_file(), "and the editor closed");
    assert_eq!(app.borrow().status_text(), "saving...");
}

/// Save left the window's command bar for the File menu (#580); the pane's
/// own row keeps its Save regardless, tested just above.
#[test]
fn save_from_the_file_menu_writes_the_file() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("file-menu-save", "fn main() {}\n");

    press(&ui, "x");
    click_command(&ui, "File");
    click_command(&ui, "Save");

    assert!(
        gui::command_log_for_test().contains(&"save".to_owned()),
        "the File menu's Save ran; it saw {:?}",
        gui::command_log_for_test()
    );
    assert!(!app.borrow().editing_file());
    assert_eq!(app.borrow().status_text(), "saving...");
}

/// The pane's Close discards, the way Escape is meant to.
#[test]
fn close_from_the_panes_own_row_discards_the_edit() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("row-close", "fn main() {}\n");

    press(&ui, "x");
    click_command(&ui, "Close");

    assert!(!app.borrow().editing_file(), "the editor closed");
    assert_eq!(app.borrow().status_text(), "edit discarded");
}

/// The row's Undo undoes typing - the behaviour the keyboard's Ctrl+Z is
/// measured against above.
#[test]
fn undo_from_the_panes_own_row_undoes_the_typing() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("row-undo", "fn main() {}\n");

    press(&ui, "x");
    click_pane_command(&ui, "Undo");

    assert_eq!(
        app.borrow().edit_text(),
        "fn main() {}\n",
        "the row's Undo took the letter back"
    );
    assert!(
        !gui::command_log_for_test().contains(&"undo".to_owned()),
        "and it went to the editor's own undo, not the window's: {:?}",
        gui::command_log_for_test()
    );
}

/// The window's toolbar carries an Undo too, and while a file is open it
/// has to mean the same thing. It used to reach past what somebody was
/// typing and put back a file they had deleted earlier.
#[test]
fn undo_from_the_windows_toolbar_undoes_the_typing_while_editing() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("toolbar-undo", "fn main() {}\n");

    press(&ui, "x");
    click_command(&ui, "Undo");

    assert!(
        gui::command_log_for_test().contains(&"undo".to_owned()),
        "the toolbar's Undo is the window's; it saw {:?}",
        gui::command_log_for_test()
    );
    assert_eq!(
        app.borrow().edit_text(),
        "fn main() {}\n",
        "and while the editor is open it undoes typing"
    );
    assert_ne!(
        app.borrow().status_text(),
        "undoing...",
        "not the filesystem's undo, which would put back a file deleted \
         somewhere else entirely"
    );
}

/// What the editor calls a page has to be what the editor shows.
///
/// `edit-visible-rows` is measured in the window, from the File pane's
/// height less the 22px heading above it, because Slint cannot name an
/// element inside an `if`. The editor's own row and its caret readout
/// are two more strips inside that `if`, 46px between them, and neither
/// is subtracted - so Page Down moves the caret further than the reader
/// can see, every time.
#[test]
fn a_page_is_what_the_editor_actually_shows() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, _app) = editing("page", "fn main() {}\n");

    let body = i_slint_backend_testing::ElementHandle::find_by_element_id(&ui, "CodeEditor::body")
        .next()
        .expect("the editing surface is drawn");
    // One line of text, so the surface is exactly as tall as the space
    // it was given, and its height in 16px rows is the page a reader
    // sees.
    // A height in pixels divided by a row is a row count; the fraction
    // is the part of a row the reader cannot use, which is exactly what
    // the floor is for.
    #[allow(clippy::cast_possible_truncation)]
    let drawn = (body.size().height / 16.0).floor() as i32;

    assert!(
        (ui.get_edit_visible_rows() - drawn).abs() <= 1,
        "the window says a page is {} rows; the surface shows {}",
        ui.get_edit_visible_rows(),
        drawn
    );
}

// -- 5. Where the keyboard goes afterwards --------------------------------

/// Saving takes the editing surface off the screen, and with it whatever
/// held the keyboard. If nothing takes it back the window is deaf: no
/// arrows, no Escape, no type-ahead, until something is clicked.
#[test]
fn after_a_save_the_keyboard_comes_back_to_the_window() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("after-save", "fn main() {}\n");

    press(&ui, "x");
    click_command(&ui, "Save *");
    assert!(!app.borrow().editing_file(), "the editor is gone");
    gui::clear_command_log_for_test();

    press_key(&ui, Key::DownArrow);

    assert!(
        gui::command_log_for_test().contains(&"move 1".to_owned()),
        "after the editor closes the window's own focus scope has to hold \
         the keyboard again, or every shortcut is dead until a pane is \
         clicked. The window saw {:?}",
        gui::command_log_for_test()
    );
}

#[test]
fn after_a_close_the_keyboard_comes_back_to_the_window() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("after-close", "fn main() {}\n");

    click_command(&ui, "Close");
    assert!(!app.borrow().editing_file());
    gui::clear_command_log_for_test();

    press_key(&ui, Key::DownArrow);

    assert!(
        gui::command_log_for_test().contains(&"move 1".to_owned()),
        "closing the editor gives the keyboard back to the window; it saw {:?}",
        gui::command_log_for_test()
    );
}

/// And the round trip: close the editor, open it again, type. The
/// surface takes the keyboard when it is created, so a second opening
/// has to work as well as the first.
#[test]
fn the_editor_takes_the_keyboard_again_when_it_is_reopened() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("reopen", "fn main() {}\n");

    click_command(&ui, "Close");
    app.borrow_mut().begin_file_edit();
    sync_ui(&ui, &app.borrow());

    press(&ui, "Z");

    assert_eq!(
        app.borrow().edit_text(),
        "Zfn main() {}\n",
        "the second opening of the editor has the keyboard too"
    );
}
