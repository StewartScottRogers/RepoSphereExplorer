//! What the editing surface draws, which is the half Rust cannot see.
//!
//! `document.rs` and `editor.rs` are tested where they live and say
//! nothing about pixels. The questions only this can answer are whether
//! the caret is drawn where the column says it is, whether a click at
//! that pixel gives the column back, and whether a long document puts
//! its whole length in the element tree.
//!
//! The first two are one question asked twice: if the caret rule and the
//! click rule ever disagree, every click lands a column off and nothing
//! else in the suite notices.

use gui::{CodeEditorHarness, ColouredRun};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

/// The row height in `app.slint`.
const ROW_HEIGHT: f32 = 16.0;

/// Many more lines than a pane can show, so virtualisation has
/// something to leave out.
const LINES: usize = 400;

fn run(text: &str) -> ColouredRun {
    ColouredRun {
        text: SharedString::from(text),
        class: 0,
    }
}

fn line(text: &str) -> ModelRc<ColouredRun> {
    ModelRc::new(VecModel::from(vec![run(text)]))
}

fn shown_window() -> CodeEditorHarness {
    let ui = CodeEditorHarness::new().expect("the harness should build");
    let lines: Vec<ModelRc<ColouredRun>> = (0..LINES)
        .map(|index| line(&format!("line {index:03} of the document")))
        .collect();
    ui.set_lines(ModelRc::new(VecModel::from(lines)));
    ui.set_longest_line(30);
    ui.show().expect("the harness should show");
    ui
}

/// Where the caret is drawn, relative to the surface.
fn caret(ui: &CodeEditorHarness) -> (f32, f32) {
    let handle = ElementHandle::find_by_element_id(ui, "CodeEditor::caret")
        .next()
        .expect("the caret is drawn");
    let origin = ElementHandle::find_by_element_id(ui, "CodeEditor::body")
        .next()
        .expect("the surface is drawn")
        .absolute_position();
    let at = handle.absolute_position();
    (at.x - origin.x, at.y - origin.y)
}

#[test]
fn the_caret_is_drawn_where_the_column_says_it_is() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();
    let cell = ui.get_cell_width();
    let gutter = 4.0;
    assert!(cell > 0.0, "a character has to have a width: {cell}");

    // Three columns on three lines, measured rather than eyeballed.
    for (row, column) in [(0, 0), (1, 7), (2, 23)] {
        ui.set_caret_line(row);
        ui.set_caret_column(column);
        let (x, y) = caret(&ui);
        #[allow(clippy::cast_precision_loss)]
        let wanted_x = gutter + column as f32 * cell;
        #[allow(clippy::cast_precision_loss)]
        let wanted_y = row as f32 * ROW_HEIGHT;
        assert!(
            (x - wanted_x).abs() < 0.5 && (y - wanted_y).abs() < 0.5,
            "row {row} column {column} should draw the caret at \
             ({wanted_x}, {wanted_y}) and drew it at ({x}, {y})"
        );
    }
}

#[test]
fn a_click_at_the_caret_gives_back_the_column_it_was_drawn_for() {
    // The inverse of the rule above. If the two disagree, every click
    // lands a column off - and both halves look right on their own.
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();
    let cell = ui.get_cell_width();
    let gutter = 4.0;

    for column in [0, 1, 7, 23] {
        #[allow(clippy::cast_precision_loss)]
        let x = gutter + column as f32 * cell;
        assert_eq!(
            ui.invoke_column_at(x),
            column,
            "a click at the pixel of column {column} is column {column}"
        );
    }
}

#[test]
fn a_click_in_a_characters_second_half_belongs_to_the_boundary_after_it() {
    // The boundary case, and the one an off-by-one hides in: the caret
    // goes to the nearer side of the character clicked, which is what
    // every editor does and what makes clicking between two letters
    // land where the pointer looks like it is.
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();
    let cell = ui.get_cell_width();
    let gutter = 4.0;

    let just_before_middle = gutter + 5.0 * cell + cell * 0.49;
    let just_after_middle = gutter + 5.0 * cell + cell * 0.51;
    assert_eq!(ui.invoke_column_at(just_before_middle), 5);
    assert_eq!(ui.invoke_column_at(just_after_middle), 6);
}

#[test]
fn only_the_visible_rows_are_drawn_and_scrolling_changes_which() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();

    let drawn = || -> Vec<String> {
        ElementHandle::find_by_element_id(&ui, "CodeEditor::editor-run")
            .filter_map(|handle| handle.accessible_label().map(|label| label.to_string()))
            .collect()
    };

    let first = drawn();
    assert!(
        first.len() < LINES,
        "a {LINES}-line document should not put every line in the tree; \
         it put {}",
        first.len()
    );
    assert!(
        first.iter().any(|text| text.contains("line 000")),
        "and it starts at the top: {:?}",
        first.first()
    );

    ui.set_scroll_y(-100.0 * ROW_HEIGHT);
    let later = drawn();
    assert!(
        !later.iter().any(|text| text.contains("line 000")),
        "after scrolling a hundred rows the first line is no longer drawn"
    );
    assert!(
        later.iter().any(|text| text.contains("line 10")),
        "and what is drawn is where it was scrolled to: {:?}",
        later.first()
    );
}

#[test]
fn a_selection_across_three_lines_draws_three_bands() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();
    ui.set_selection_start_line(1);
    ui.set_selection_start_column(4);
    ui.set_selection_end_line(3);
    ui.set_selection_end_column(6);

    for name in ["band-head", "band-middle", "band-tail"] {
        assert_eq!(
            ElementHandle::find_by_element_id(&ui, &format!("CodeEditor::{name}")).count(),
            1,
            "{name} should be drawn: the tail of the first line, the whole \
             of the middle, and the head of the last"
        );
    }

    let middle = ElementHandle::find_by_element_id(&ui, "CodeEditor::band-middle")
        .next()
        .expect("the middle band is drawn");
    assert!(
        (middle.size().height - ROW_HEIGHT).abs() < 0.5,
        "the middle band covers the one whole line between: {}px",
        middle.size().height
    );
}

#[test]
fn no_selection_draws_no_band_at_all() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();
    ui.set_selection_start_line(-1);
    for name in ["band-head", "band-middle", "band-tail"] {
        assert_eq!(
            ElementHandle::find_by_element_id(&ui, &format!("CodeEditor::{name}")).count(),
            0,
            "{name} should not be drawn when nothing is selected"
        );
    }
}

#[test]
fn the_surface_has_one_touch_area_and_not_one_per_row() {
    // Slint 1.17.1 leaves every per-row `TouchArea` but the first
    // permanently unresponsive. The folders and contents panes both work
    // around it; this says the editor did not reintroduce it.
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();
    assert_eq!(
        ElementHandle::find_by_element_id(&ui, "CodeEditor::surface").count(),
        1,
        "one touch area for the whole surface"
    );
}

#[test]
fn a_keystroke_reaches_rust_verbatim_with_its_modifiers() {
    // Acceptance check 6: through the component, not only through the
    // model beneath it. `editor.rs` proves what a key *means*; this
    // proves the surface hands it over unchanged - text, shift and
    // control - because a component that swallowed shift would leave
    // every test in `editor.rs` passing and no selection possible.
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();

    let seen: Rc<RefCell<Vec<(String, bool, bool)>>> = Rc::new(RefCell::new(Vec::new()));
    {
        let seen = Rc::clone(&seen);
        ui.on_key(move |text, shift, control| {
            seen.borrow_mut().push((text.to_string(), shift, control));
        });
    }

    let window = ui.window();
    window.dispatch_event(WindowEvent::KeyPressed { text: "x".into() });
    window.dispatch_event(WindowEvent::KeyPressed {
        text: char::from(Key::Shift).into(),
    });
    window.dispatch_event(WindowEvent::KeyPressed {
        text: char::from(Key::RightArrow).into(),
    });

    let seen = seen.borrow();
    assert!(
        seen.iter().any(|(text, _, _)| text == "x"),
        "a typed character arrives as itself: {seen:?}"
    );
    assert!(
        seen.iter()
            .any(|(text, shift, _)| text.starts_with(char::from(Key::RightArrow)) && *shift),
        "and an arrow arrives as the arrow, with shift still held: {seen:?}"
    );
}
