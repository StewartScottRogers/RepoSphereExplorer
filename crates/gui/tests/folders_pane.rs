//! Checks what the folders pane draws, which is the half of the
//! navigation tree that Rust cannot see.
//!
//! `App::folder_rows` and `App::chevron_hit` are unit-tested where they
//! live. What those tests cannot say is whether the pane draws a row per
//! entry, or whether it indents by the same sixteen pixels the hit rule
//! measures against. If the two ever disagree, every click on a chevron
//! below the root lands on the wrong thing, and nothing else notices.

use gui::{FolderRow, MainWindow};
use i_slint_backend_testing::ElementHandle;
use slint::{ComponentHandle, Image, ModelRc, SharedString, VecModel};

/// One indent level in `app.slint`, and `app::FOLDER_INDENT` in Rust.
const INDENT: f32 = 16.0;

fn row(name: &str, depth: i32, expandable: bool) -> FolderRow {
    FolderRow {
        icon: Image::default(),
        name: SharedString::from(name),
        depth,
        expandable,
        expanded: false,
        is_repository: false,
    }
}

/// A window showing a root with a child and a grandchild, so the indent
/// can be measured across three levels rather than inferred from one.
fn shown_window() -> MainWindow {
    let ui = MainWindow::new().expect("the window should build");
    ui.set_folder_rows(ModelRc::new(VecModel::from(vec![
        row("repos", 0, true),
        row("Forge", 1, true),
        row("src", 2, false),
    ])));
    ui.show().expect("the window should show");
    ui
}

fn labels(ui: &MainWindow) -> Vec<String> {
    ElementHandle::find_by_element_id(ui, "FoldersPane::tree-row")
        .filter_map(|handle| handle.accessible_label().map(|label| label.to_string()))
        .collect()
}

#[test]
fn the_pane_draws_a_row_for_every_folder_in_the_tree() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();

    assert_eq!(labels(&ui), vec!["repos", "Forge", "src"]);
}

#[test]
fn each_level_of_the_tree_indents_by_what_the_click_rule_measures() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();

    let left_edges: Vec<f32> = ElementHandle::find_by_element_id(&ui, "FoldersPane::folder-icon")
        .map(|handle| handle.absolute_position().x)
        .collect();
    assert_eq!(left_edges.len(), 3, "one icon per row");

    for (level, pair) in left_edges.windows(2).enumerate() {
        let step = pair[1] - pair[0];
        assert!(
            (step - INDENT).abs() < 0.5,
            "level {level} to {} should step {INDENT}px, and steps {step}px. \
             The indent drawn here and `app::FOLDER_INDENT`, which decides \
             whether a click landed on a chevron, have to be one number.",
            level + 1
        );
    }
}
