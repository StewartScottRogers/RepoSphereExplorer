//! The File pane while the editor is open: the command row, the caret
//! readout, and the surface under them.
//!
//! Rendering that state is the point. The commands are tested in `app`
//! and the surface in `code_editor`; what neither does is build the
//! window with the editor open and lay it out, which is where a binding
//! that cannot be evaluated - a division by a count that is zero, a
//! string built from a number - would show itself.

use gui::{ColouredRun, MainWindow};
use i_slint_backend_testing::ElementHandle;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

fn line(text: &str) -> ModelRc<ColouredRun> {
    ModelRc::new(VecModel::from(vec![ColouredRun {
        text: SharedString::from(text),
        class: 0,
    }]))
}

/// A window with a file open in the editor, laid out.
fn editing_window() -> MainWindow {
    let ui = MainWindow::new().expect("the window should build");
    ui.set_file_tabs(ModelRc::new(VecModel::from(vec![SharedString::from(
        "Editing",
    )])));
    ui.set_editing_file(true);
    ui.set_editing_in_colour(true);
    ui.set_edit_lines(ModelRc::new(VecModel::from(vec![
        line("fn main() {"),
        line("}"),
    ])));
    ui.set_edit_longest_line(11);
    ui.set_edit_line(1);
    ui.set_edit_column(1);
    ui.show().expect("the window should show");
    ui
}

#[test]
fn the_pane_lays_out_with_the_editor_open() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = editing_window();

    assert_eq!(
        ElementHandle::find_by_element_id(&ui, "CodeEditor::caret").count(),
        1,
        "the editing surface is drawn"
    );
}

#[test]
fn the_command_row_is_there_while_editing_and_gone_when_it_is_not() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = editing_window();

    let labels: Vec<String> = ElementHandle::find_by_element_id(&ui, "CommandButton::command-text")
        .filter_map(|handle| handle.accessible_label().map(|label| label.to_string()))
        .collect();
    for wanted in ["Save", "Undo", "Redo", "Cut", "Copy", "Paste", "Close"] {
        assert!(
            labels.iter().any(|seen| seen.starts_with(wanted)),
            "the editor's own {wanted} should be in the pane; it drew {labels:?}"
        );
    }

    ui.set_editing_file(false);
    let after: Vec<String> = ElementHandle::find_by_element_id(&ui, "CommandButton::command-text")
        .filter_map(|handle| handle.accessible_label().map(|label| label.to_string()))
        .collect();
    assert!(
        !after.iter().any(|seen| seen == "Close"),
        "and is gone when the editor is not open: {after:?}"
    );
}

#[test]
fn the_caret_readout_says_where_the_caret_is() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = editing_window();
    ui.set_edit_line(4);
    ui.set_edit_column(12);

    let texts: Vec<String> = ElementHandle::find_by_element_id(&ui, "FilePane::caret-readout")
        .filter_map(|handle| handle.accessible_label().map(|label| label.to_string()))
        .collect();
    assert!(
        texts.iter().any(|text| text == "Ln 4, Col 12"),
        "every Windows editor puts this at the bottom: {texts:?}"
    );
}
