//! Checks that the colours reach the pane, and that they are eight
//! different colours.
//!
//! `crates/syntax` proves the tokeniser and `classification.rs` proves
//! every plugin's spans. Neither says the pane *draws* them: the runs
//! could be computed perfectly and painted in one colour, and every other
//! test would still pass.

use gui::{ColouredRun, MainWindow, Theme};
use i_slint_backend_testing::ElementHandle;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

/// `plugin_api::Class` as `sync_ui` numbers it and `Theme.syntax-colour`
/// maps it. Literals on purpose: if one half is renumbered without the
/// other, this should notice rather than follow along.
const PLAIN: i32 = 0;
const KEYWORD: i32 = 1;
const STRING: i32 = 4;
const COMMENT: i32 = 6;

/// How many classes there are. Eight, and eight colours.
const CLASSES: i32 = 8;

fn run(text: &str, class: i32) -> ColouredRun {
    ColouredRun {
        text: SharedString::from(text),
        class,
    }
}

fn line(runs: Vec<ColouredRun>) -> ModelRc<ColouredRun> {
    ModelRc::new(VecModel::from(runs))
}

fn drawn(ui: &MainWindow) -> Vec<String> {
    ElementHandle::find_by_element_id(ui, "FilePane::syntax-run")
        .filter_map(|handle| handle.accessible_label().map(|label| label.to_string()))
        .collect()
}

#[test]
fn a_classified_file_is_drawn_run_by_run_rather_than_as_one_string() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = MainWindow::new().expect("the window should build");
    ui.set_file_lines(ModelRc::new(VecModel::from(vec![
        line(vec![
            run("let", KEYWORD),
            run(" name = ", PLAIN),
            run("\"value\"", STRING),
        ]),
        line(vec![run("// a note", COMMENT)]),
    ])));
    ui.show().expect("the window should show");

    let seen = drawn(&ui);
    assert_eq!(
        seen,
        vec!["let", " name = ", "\"value\"", "// a note"],
        "four runs across two lines, each its own element and so each its \
         own colour"
    );
}

#[test]
fn an_unclassified_file_falls_back_to_the_plain_text_it_always_drew() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = MainWindow::new().expect("the window should build");
    ui.set_file_lines(ModelRc::new(VecModel::from(
        Vec::<ModelRc<ColouredRun>>::new(),
    )));
    ui.set_file_text(SharedString::from("nothing describes this format"));
    ui.show().expect("the window should show");

    assert!(
        drawn(&ui).is_empty(),
        "with no runs the coloured view is not drawn at all, and the plain \
         one takes its place"
    );
}

#[test]
fn every_class_gets_a_colour_of_its_own_in_both_schemes() {
    // A table where two classes shared a colour would read as one class.
    // Checked through the function the pane itself calls.
    i_slint_backend_testing::init_no_event_loop();
    let ui = MainWindow::new().expect("the window should build");
    let theme = ui.global::<Theme>();

    let mut colours: Vec<String> = (0..CLASSES)
        .map(|class| format!("{:?}", theme.invoke_syntax_colour(class)))
        .collect();
    let total = colours.len();
    colours.sort();
    colours.dedup();

    assert_eq!(
        colours.len(),
        total,
        "two of the {CLASSES} classes share a colour, so a reader cannot \
         tell them apart: {colours:?}"
    );
}
