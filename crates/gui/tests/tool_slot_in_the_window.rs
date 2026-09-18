//! The tool slot's picker (#616), driven through a real window on a real
//! `App` the way `main` wires it (rule 14). A unit test on `ToolRegistry`
//! can prove the *default* tool is chosen correctly; it cannot prove that
//! choosing one in the picker actually reaches the pane a reader sees, or
//! that the picker stays hidden while only the editor applies. That seam
//! is what this file drives - registering a second tool of its own, since
//! no other one is compiled in yet.

use gui::app::{App, Selection, Target};
use gui::tools::Tool;
use gui::{MainWindow, sync_ui, wire_callbacks};
use slint::{ComponentHandle as _, Model as _};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// A tool this file alone registers: it applies only to a `.txt` file, so
/// selecting one is the only way its title and picker entry appear.
struct TxtTool;

impl Tool for TxtTool {
    fn id(&self) -> &'static str {
        "txt-tool"
    }

    fn title(&self) -> &'static str {
        "Txt tool"
    }

    fn applies_to(&self, selection: &Selection) -> bool {
        matches!(
            &selection.target,
            Some(Target::File { name })
                if std::path::Path::new(name).extension().is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
        )
    }
}

/// An empty directory of this test's own under the platform's temporary
/// directory.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("rse-tool-slot").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn file(dir: &Path, name: &str) {
    std::fs::write(dir.join(name), "content").expect("a scratch file");
}

/// A shown window on a real `App`, wired as `main` wires it, with
/// `TxtTool` registered alongside the compiled-in editor and its two
/// entries already loaded and listed.
fn window_with_txt_tool(name: &str) -> (MainWindow, Rc<RefCell<App>>) {
    let dir = scratch(name);
    file(&dir, "main.rs");
    file(&dir, "notes.txt");

    let mut app = App::new(dir.clone());
    let entries = service::list_directory(&dir).expect("the directory lists");
    app.apply_contents_result_for_test(&[], protocol::Response::Directory { entries });
    app.register_tool_for_test(Box::new(TxtTool));
    let app = Rc::new(RefCell::new(app));

    let ui = MainWindow::new().expect("the window should build");
    wire_callbacks(&ui, &app);
    sync_ui(&ui, &app.borrow());
    ui.show().expect("the window should show");
    (ui, app)
}

/// Selects the contents row named `name`, and syncs the window.
fn select(ui: &MainWindow, app: &Rc<RefCell<App>>, name: &str) {
    let mut app = app.borrow_mut();
    let index = app
        .content_rows()
        .iter()
        .position(|row| row.name == name)
        .expect("the row is in the listing");
    app.select_content(index);
    sync_ui(ui, &app);
}

#[test]
fn the_picker_is_hidden_for_a_file_only_the_editor_applies_to() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = window_with_txt_tool("hidden");

    select(&ui, &app, "main.rs");

    assert_eq!(
        ui.get_tool_titles().row_count(),
        0,
        "only the editor applies to a .rs file, so the picker has nothing to offer"
    );
    assert_eq!(ui.get_active_tool_title(), "File");
    assert!(ui.get_tool_showing_editor());
}

#[test]
fn the_picker_is_shown_once_a_second_tool_also_applies() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = window_with_txt_tool("shown");

    select(&ui, &app, "notes.txt");

    let titles: Vec<String> = (0..ui.get_tool_titles().row_count())
        .map(|i| {
            ui.get_tool_titles()
                .row_data(i)
                .expect("a title")
                .to_string()
        })
        .collect();
    assert_eq!(
        titles,
        vec!["File".to_owned(), "Txt tool".to_owned()],
        "both the editor and the registered tool apply to a .txt file"
    );
}

#[test]
fn choosing_the_registered_tool_shows_its_title_and_its_own_content() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = window_with_txt_tool("choose");

    select(&ui, &app, "notes.txt");
    assert!(
        ui.get_tool_showing_editor(),
        "the editor is still the default until the picker is asked"
    );

    ui.invoke_tool_selected(1);
    sync_ui(&ui, &app.borrow());

    assert_eq!(ui.get_active_tool_title(), "Txt tool");
    assert!(
        !ui.get_tool_showing_editor(),
        "the chosen tool is not the editor, so its own content draws instead"
    );
}

#[test]
fn a_tool_that_stops_applying_hands_the_slot_back_to_the_editor() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, app) = window_with_txt_tool("hands-back");

    select(&ui, &app, "notes.txt");
    ui.invoke_tool_selected(1);
    sync_ui(&ui, &app.borrow());
    assert_eq!(ui.get_active_tool_title(), "Txt tool");

    // A `.rs` file is not one `TxtTool` applies to.
    select(&ui, &app, "main.rs");

    assert_eq!(ui.get_active_tool_title(), "File");
    assert!(ui.get_tool_showing_editor());
}
