//! Typing into the real window, wired to a real `App` the way the
//! application wires it.
//!
//! Every other test in this crate holds one half. `document` and
//! `editor` know what a keystroke means but have never met a window;
//! `code_editor` and `editor_chrome` know what is drawn but have no
//! `App` behind them. The half nobody had is the one that decides
//! whether a letter lands where the caret is — which is the report this
//! file exists for: *"Editor does not insert at caret."*
//!
//! So: `MainWindow::new`, `App::new`, `gui::wire_editor` — the same
//! function `main` calls — and then real `WindowEvent`s.

use gui::app::App;
use gui::{MainWindow, sync_ui, wire_editor};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model as _};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// A directory of this test's own.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("repos-explorer-window-{name}"));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    directory
}

/// A shown window and the application behind it, with `text` open in the
/// editor and the keyboard in it.
fn editing(name: &str, text: &str) -> (MainWindow, Rc<RefCell<App>>) {
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
        app.begin_file_edit();
        assert!(
            app.editing_in_colour(),
            "the coloured surface is the one under test"
        );
    }

    let ui = MainWindow::new().expect("the window should build");
    wire_editor(&ui, &app);
    sync_ui(&ui, &app.borrow());
    ui.show().expect("the window should show");
    // The surface takes the keyboard when it is clicked; nothing has
    // clicked it here, so it is given focus the way the pane gives it.
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    (ui, app)
}

/// Sends `text` as a keystroke, through the window.
fn press(ui: &MainWindow, text: &str) {
    ui.invoke_edit_key(text.into(), false, false);
}

fn press_key(ui: &MainWindow, key: Key) {
    press(ui, &char::from(key).to_string());
}

/// Clicks at a line and column, as the surface reports one.
fn click(ui: &MainWindow, line: i32, column: i32) {
    ui.invoke_edit_pressed(line, column);
}

#[test]
fn a_letter_typed_at_the_start_lands_at_the_start() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("start", "fn main() {}\n");

    press(&ui, "x");

    assert_eq!(app.borrow().edit_text(), "xfn main() {}\n");
    assert_eq!(app.borrow().edit_caret(), (0, 1), "and the caret moved on");
}

#[test]
fn a_letter_typed_after_a_click_lands_where_the_click_was() {
    // The report. A click puts the caret somewhere; the next letter has
    // to go there and nowhere else.
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("click", "one\ntwo\nthree\n");

    click(&ui, 1, 2);
    assert_eq!(
        app.borrow().edit_caret(),
        (1, 2),
        "the click put the caret two into the second line"
    );

    press(&ui, "X");

    assert_eq!(
        app.borrow().edit_text(),
        "one\ntwXo\nthree\n",
        "and the letter went there"
    );
    assert_eq!(app.borrow().edit_caret(), (1, 3));
}

#[test]
fn a_word_typed_after_a_click_stays_together_where_it_was_put() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("word", "alpha\nbeta\n");

    click(&ui, 1, 4);
    for letter in ["!", "?", "."] {
        press(&ui, letter);
    }

    assert_eq!(
        app.borrow().edit_text(),
        "alpha\nbeta!?.\n",
        "each letter after the last, not scattered"
    );
}

#[test]
fn the_arrows_move_the_caret_the_window_reports() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("arrows", "one\ntwo\nthree\n");

    press_key(&ui, Key::DownArrow);
    press_key(&ui, Key::RightArrow);
    assert_eq!(app.borrow().edit_caret(), (1, 1));

    // And what the pane is told matches what the document holds. If
    // these ever differ the caret is drawn in one place and types in
    // another, which is exactly what "does not insert at caret" looks
    // like from the outside.
    assert_eq!(ui.get_edit_caret_line(), 1);
    assert_eq!(ui.get_edit_caret_column(), 1);
}

#[test]
fn what_the_pane_draws_is_what_the_document_holds() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("drawn", "ab\ncd\n");

    click(&ui, 1, 1);
    press(&ui, "Z");

    let lines = ui.get_edit_lines();
    let drawn: Vec<String> = (0..lines.row_count())
        .map(|row| {
            let line = lines.row_data(row).expect("a drawn line");
            (0..line.row_count())
                .filter_map(|run| line.row_data(run).map(|run| run.text.to_string()))
                .collect::<String>()
        })
        .collect();
    assert_eq!(drawn, vec!["ab".to_owned(), "cZd".to_owned()]);
    assert_eq!(app.borrow().edit_text(), "ab\ncZd\n");
}

#[test]
fn backspace_takes_the_character_before_the_caret_and_not_another() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("backspace", "abcdef\n");

    click(&ui, 0, 3);
    press_key(&ui, Key::Backspace);

    assert_eq!(app.borrow().edit_text(), "abdef\n", "the c, not the a");
    assert_eq!(app.borrow().edit_caret(), (0, 2));
}

#[test]
fn return_splits_the_line_where_the_caret_is() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("return", "abcdef\n");

    click(&ui, 0, 3);
    press_key(&ui, Key::Return);

    assert_eq!(app.borrow().edit_text(), "abc\ndef\n");
    assert_eq!(app.borrow().edit_caret(), (1, 0));
}

#[test]
fn typing_over_a_selection_replaces_exactly_that() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("selection", "keep this word\n");

    click(&ui, 0, 5);
    for _ in 0..4 {
        ui.invoke_edit_key(char::from(Key::RightArrow).to_string().into(), true, false);
    }
    assert_eq!(
        app.borrow().edit_text(),
        "keep this word\n",
        "selecting changes nothing"
    );

    press(&ui, "that");

    assert_eq!(app.borrow().edit_text(), "keep that word\n");
}

#[test]
fn a_click_past_the_end_of_a_line_lands_at_its_end() {
    // Clicking in the empty space to the right of a short line is what
    // everybody does, and it must not put the caret on another line.
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("past-end", "ab\nlonger line\n");

    click(&ui, 0, 40);
    press(&ui, "!");

    assert_eq!(app.borrow().edit_text(), "ab!\nlonger line\n");
}

#[test]
fn a_click_below_the_last_line_lands_on_the_last_line() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("below", "one\ntwo\n");

    click(&ui, 99, 0);
    press(&ui, "!");

    assert_eq!(app.borrow().edit_text(), "one\n!two\n");
}

#[test]
fn typing_into_an_empty_file_works_at_all() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("empty", "");

    press(&ui, "h");
    press(&ui, "i");

    assert_eq!(app.borrow().edit_text(), "hi");
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

/// Where the editing surface starts on screen.
fn surface_origin(ui: &MainWindow) -> (f32, f32) {
    let body = i_slint_backend_testing::ElementHandle::find_by_element_id(ui, "CodeEditor::body")
        .next()
        .expect("the editing surface is drawn");
    let at = body.absolute_position();
    (at.x, at.y)
}

/// A real click, at a real pixel, has to reach a real column.
///
/// `editing_in_the_window`'s other tests call `edit-pressed` directly,
/// which is everything except the one computation Slint does: turning a
/// pointer position into a line and a column. If `cell-width` were
/// zero, and it is measured from a `Text` that is deliberately
/// invisible, every click in the pane would report column nought, the
/// caret would jump to the start of the line, and typing would land
/// there. From outside that is exactly "the editor does not insert at
/// the caret".
#[test]
fn a_real_click_across_a_line_reaches_more_than_column_nought() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("pixels", "abcdefghijklmnopqrstuvwxyz\nsecond line\n");
    let (x, y) = surface_origin(&ui);

    // Far enough along the first line that no plausible character width
    // puts it at the start.
    click_at(&ui, x + 120.0, y + 8.0);

    let (line, column) = app.borrow().edit_caret();
    assert_eq!(line, 0, "a click on the first row is on the first line");
    assert!(
        column > 0,
        "a click 120 pixels into a line of text has to reach a column past \
         the first; it reached {column}. A cell width of zero would do \
         exactly this."
    );
}

#[test]
fn clicks_further_along_a_line_reach_further_columns() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("pixels-order", "abcdefghijklmnopqrstuvwxyz\n");
    let (x, y) = surface_origin(&ui);

    click_at(&ui, x + 40.0, y + 8.0);
    let near = app.borrow().edit_caret().1;
    click_at(&ui, x + 120.0, y + 8.0);
    let far = app.borrow().edit_caret().1;

    assert!(
        far > near,
        "further along the line is a later column: 40px reached {near}, \
         120px reached {far}"
    );
}

/// A real keystroke, dispatched to the window, has to reach the editor.
///
/// Every key test above calls `edit-key` directly, which skips the one
/// question a reader actually meets: which focus scope has the
/// keyboard. The window has one of its own - it handles Escape, Delete
/// and the type-ahead that jumps around the Contents pane - and the
/// editing surface has another. If the window's holds the keyboard
/// while the editor is open, typing goes somewhere else entirely and
/// the file does not change at the caret because it does not change at
/// all.
#[test]
fn a_real_keystroke_reaches_the_editor_and_not_the_window() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("focus", "abc\n");

    // Click the surface first, which is what gives it the keyboard -
    // and what a reader does before typing.
    let (x, y) = surface_origin(&ui);
    click_at(&ui, x + 4.0, y + 8.0);

    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: "Z".into() });

    assert_eq!(
        app.borrow().edit_text(),
        "Zabc\n",
        "a key pressed on the window reached the editor. If this is \
         unchanged the keyboard is with the window's own focus scope, \
         and typing is going to the Contents pane's type-ahead."
    );
}

#[test]
fn a_real_keystroke_reaches_the_editor_without_a_click_first() {
    // A reader who opened the editor from the Edit tab has not clicked
    // the surface. If the keyboard only arrives on a click, the first
    // thing they type goes to the window instead.
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = editing("focus-no-click", "abc\n");

    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: "Z".into() });

    assert_eq!(
        app.borrow().edit_text(),
        "Zabc\n",
        "opening the editor has to put the keyboard in it"
    );
}
