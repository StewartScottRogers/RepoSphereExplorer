//! Walks every menu item, command-bar button and navigation button, and
//! asserts each one reaches its callback.
//!
//! Driven through Slint's own hit-testing rather than the windowing system,
//! so it runs headless in CI. The window here is the UI layer alone, with no
//! `App` behind it: what is checked is that each control is wired and
//! reachable, not what the operation then goes on to do, which the `app`
//! module's own tests cover.

use gui::{ContentRow, MainWindow};
use i_slint_backend_testing::ElementHandle;
use slint::platform::PointerEventButton;
use slint::{ComponentHandle, Image, ModelRc, SharedString, VecModel};
use std::cell::Cell;
use std::rc::Rc;

/// Narrowest a dropdown menu gets in `app.slint`. The command-bar buttons
/// are far narrower, which is what tells the two apart when they share a
/// label: "Rename" is both a menu item and a button.
const MENU_MIN_WIDTH: f32 = 150.0;

fn row(name: &str) -> ContentRow {
    ContentRow {
        icon: Image::default(),
        name: SharedString::from(name),
        size: SharedString::new(),
        kind: SharedString::new(),
        modified: SharedString::new(),
        selected: false,
    }
}

/// A shown window with a row to act on and every "is this possible" flag
/// set, so no command is greyed out.
fn ready_window() -> MainWindow {
    let ui = MainWindow::new().expect("the window should build");
    ui.set_content_rows(ModelRc::new(VecModel::from(vec![row("a.txt")])));
    ui.set_has_selection(true);
    ui.set_can_paste(true);
    ui.set_content_is_archive(true);
    ui.set_can_go_back(true);
    ui.set_can_go_forward(true);
    ui.show().expect("the window should show");
    ui
}

/// Clicks the widest element labelled `label` whose width falls in the given
/// range.
fn click_labelled(ui: &MainWindow, label: &str, min_width: f32, max_width: f32) {
    let mut matches: Vec<ElementHandle> = ElementHandle::find_by_accessible_label(ui, label)
        .filter(|item| item.size().width >= min_width && item.size().width < max_width)
        .collect();
    matches.sort_by(|a, b| {
        b.size()
            .width
            .partial_cmp(&a.size().width)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let target = matches.first().unwrap_or_else(|| {
        panic!("nothing labelled {label:?} between {min_width} and {max_width} wide")
    });
    target.mock_single_click(PointerEventButton::Left);
}

/// Opens the menu titled `title`, then chooses `item` from it.
fn choose_from_menu(ui: &MainWindow, title: &str, item: &str) {
    click_labelled(ui, title, 0.0, MENU_MIN_WIDTH);
    click_labelled(ui, item, MENU_MIN_WIDTH, f32::MAX);
}

/// Clicks the command-bar or navigation button labelled `label`.
fn click_button(ui: &MainWindow, label: &str) {
    click_labelled(ui, label, 0.0, MENU_MIN_WIDTH);
}

/// A flag a callback raises, so a test can assert the callback ran.
fn flag() -> Rc<Cell<bool>> {
    Rc::new(Cell::new(false))
}

/// Wires `setter` to raise `fired`.
macro_rules! watch {
    ($ui:expr, $setter:ident, $fired:expr) => {{
        let fired = Rc::clone(&$fired);
        $ui.$setter(move || fired.set(true));
    }};
}

#[test]
fn every_file_menu_item_reaches_its_callback() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = ready_window();
    let (folder, file, open, rename, delete, quit) =
        (flag(), flag(), flag(), flag(), flag(), flag());
    watch!(ui, on_new_folder_requested, folder);
    watch!(ui, on_new_file_requested, file);
    watch!(ui, on_content_open_requested, open);
    watch!(ui, on_content_rename_requested, rename);
    watch!(ui, on_delete_requested, delete);
    watch!(ui, on_quit_requested, quit);

    for (item, fired) in [
        ("New Folder", &folder),
        ("New File", &file),
        ("Open", &open),
        ("Rename", &rename),
        ("Delete", &delete),
        ("Exit", &quit),
    ] {
        choose_from_menu(&ui, "File", item);
        assert!(fired.get(), "File > {item} did not reach its callback");
    }
}

#[test]
fn every_edit_menu_item_reaches_its_callback() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = ready_window();
    let (cut, copy, paste, all) = (flag(), flag(), flag(), flag());
    watch!(ui, on_clipboard_cut_requested, cut);
    watch!(ui, on_clipboard_copy_requested, copy);
    watch!(ui, on_clipboard_paste_requested, paste);
    watch!(ui, on_select_all_requested, all);

    for (item, fired) in [
        ("Cut", &cut),
        ("Copy", &copy),
        ("Paste", &paste),
        ("Select All", &all),
    ] {
        choose_from_menu(&ui, "Edit", item);
        assert!(fired.get(), "Edit > {item} did not reach its callback");
    }
}

#[test]
fn every_view_menu_item_reaches_its_callback() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = ready_window();
    let (refresh, parent) = (flag(), flag());
    watch!(ui, on_refresh_requested, refresh);
    watch!(ui, on_parent_requested, parent);
    let sorted = Rc::new(Cell::new(-1));
    ui.on_content_sort_requested({
        let sorted = Rc::clone(&sorted);
        move |column| sorted.set(column)
    });

    choose_from_menu(&ui, "View", "Refresh");
    assert!(refresh.get(), "View > Refresh did not reach its callback");

    choose_from_menu(&ui, "View", "Up One Level");
    assert!(
        parent.get(),
        "View > Up One Level did not reach its callback"
    );

    for (item, column) in [
        ("Sort by Name", 0),
        ("Sort by Size", 1),
        ("Sort by Type", 2),
        ("Sort by Modified", 3),
    ] {
        sorted.set(-1);
        choose_from_menu(&ui, "View", item);
        assert_eq!(
            sorted.get(),
            column,
            "View > {item} sorted the wrong column"
        );
    }
}

#[test]
fn the_help_menu_reaches_its_callback() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = ready_window();
    let about = flag();
    watch!(ui, on_about_requested, about);

    choose_from_menu(&ui, "Help", "About RepoSphereExplorer");

    assert!(about.get(), "Help > About did not reach its callback");
}

#[test]
fn every_command_bar_button_reaches_its_callback() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = ready_window();
    let (folder, file, cut, copy, paste) = (flag(), flag(), flag(), flag(), flag());
    let (rename, delete, extract, refresh) = (flag(), flag(), flag(), flag());
    watch!(ui, on_new_folder_requested, folder);
    watch!(ui, on_new_file_requested, file);
    watch!(ui, on_clipboard_cut_requested, cut);
    watch!(ui, on_clipboard_copy_requested, copy);
    watch!(ui, on_clipboard_paste_requested, paste);
    watch!(ui, on_content_rename_requested, rename);
    watch!(ui, on_delete_requested, delete);
    watch!(ui, on_content_extract_requested, extract);
    watch!(ui, on_refresh_requested, refresh);

    for (button, fired) in [
        ("New folder", &folder),
        ("New file", &file),
        ("Cut", &cut),
        ("Copy", &copy),
        ("Paste", &paste),
        ("Rename", &rename),
        ("Delete", &delete),
        ("Extract", &extract),
        ("Refresh", &refresh),
    ] {
        click_button(&ui, button);
        assert!(
            fired.get(),
            "the {button} button did not reach its callback"
        );
    }
}

#[test]
fn every_navigation_button_reaches_its_callback() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = ready_window();
    let (back, forward, up) = (flag(), flag(), flag());
    watch!(ui, on_back_requested, back);
    watch!(ui, on_forward_requested, forward);
    watch!(ui, on_parent_requested, up);

    for (glyph, fired) in [
        ("\u{2190}", &back),
        ("\u{2192}", &forward),
        ("\u{2191}", &up),
    ] {
        click_button(&ui, glyph);
        assert!(fired.get(), "the {glyph} button did not reach its callback");
    }
}

#[test]
fn a_command_with_nothing_to_act_on_is_not_clickable() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = MainWindow::new().expect("the window should build");
    ui.set_content_rows(ModelRc::new(VecModel::from(vec![row("a.txt")])));
    // No selection, no clipboard, nothing archive-shaped: Cut, Paste and
    // Extract all have nothing to do and are drawn greyed out.
    ui.show().expect("the window should show");
    let (cut, paste, extract) = (flag(), flag(), flag());
    watch!(ui, on_clipboard_cut_requested, cut);
    watch!(ui, on_clipboard_paste_requested, paste);
    watch!(ui, on_content_extract_requested, extract);

    for label in ["Cut", "Paste", "Extract"] {
        click_button(&ui, label);
    }

    assert!(!cut.get(), "Cut fired without a selection");
    assert!(!paste.get(), "Paste fired with an empty clipboard");
    assert!(
        !extract.get(),
        "Extract fired on something that is not an archive"
    );
}

#[test]
fn every_column_header_sorts_its_own_column() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = ready_window();
    let sorted = Rc::new(Cell::new(-1));
    ui.on_content_sort_requested({
        let sorted = Rc::clone(&sorted);
        move |column| sorted.set(column)
    });

    for (header, column) in [("Name", 0), ("Size", 1), ("Type", 2), ("Modified", 3)] {
        sorted.set(-1);
        click_button(&ui, header);
        assert_eq!(
            sorted.get(),
            column,
            "the {header} header sorted the wrong column"
        );
    }
}

#[test]
fn a_breadcrumb_navigates_to_its_own_segment() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = ready_window();
    ui.set_breadcrumbs(ModelRc::new(VecModel::from(vec![
        SharedString::from("Z:\\"),
        SharedString::from("repos"),
        SharedString::from("project"),
    ])));
    let clicked = Rc::new(Cell::new(-1));
    ui.on_breadcrumb_requested({
        let clicked = Rc::clone(&clicked);
        move |index| clicked.set(index)
    });

    click_button(&ui, "repos");

    assert_eq!(clicked.get(), 1, "the second segment was clicked");
}
