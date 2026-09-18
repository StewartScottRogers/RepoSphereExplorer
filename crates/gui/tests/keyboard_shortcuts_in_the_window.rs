//! Walks `shortcuts::BINDINGS` against a real window, per CLAUDE.md rule 14
//! and #585's first acceptance check: every entry's keys, dispatched
//! through the same `key-scope` `ui/app.slint` gives the window, must
//! reach the callback the table says it does. A binding removed from the
//! markup, given the wrong modifiers, or pointed at the wrong callback,
//! fails here rather than only in the reference sheet nobody reads until
//! the key stops working.

use gui::app::App;
use gui::shortcuts::{self, Fires};
use gui::{MainWindow, sync_ui};
use slint::ComponentHandle;
use slint::platform::WindowEvent;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// An empty directory of this test's own.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("rse-keyboard-shortcuts")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// A shown window on a real `App`, with the keyboard in its window-level
/// scope - no callback is wired, since this test asks only whether a key
/// reaches the right one, not what the application then does with it.
fn window(name: &str) -> (MainWindow, Rc<RefCell<App>>) {
    let dir = scratch(name);
    let app = Rc::new(RefCell::new(App::new(dir)));
    let ui = MainWindow::new().expect("the window should build");
    sync_ui(&ui, &app.borrow());
    ui.show().expect("the window should show");
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    (ui, app)
}

/// Dispatches `binding`'s modifiers, then its key, then releases both in
/// reverse - real `WindowEvent`s, the way a keyboard actually reports a
/// chord, rather than a direct call to a Slint callback.
fn press(ui: &MainWindow, binding: &shortcuts::Binding) {
    let window = ui.window();
    let modifiers = binding.modifiers();
    for key in &modifiers {
        window.dispatch_event(WindowEvent::KeyPressed {
            text: char::from(*key).into(),
        });
    }
    let text: slint::SharedString = binding.dispatch_text().into();
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
    for key in modifiers.iter().rev() {
        window.dispatch_event(WindowEvent::KeyReleased {
            text: char::from(*key).into(),
        });
    }
}

/// Installs a spy on exactly the one callback `fires` names, so pressing a
/// binding's keys can be checked against it without needing the rest of
/// `wire_callbacks` - and without one binding's spy answering for another
/// that happens to share a callback with a different argument.
fn install_spy(ui: &MainWindow, fires: Fires, log: &Rc<RefCell<Vec<String>>>) {
    macro_rules! spy {
        ($setter:ident, $label:literal) => {{
            let log = log.clone();
            ui.$setter(move || log.borrow_mut().push($label.to_owned()));
        }};
    }
    macro_rules! spy_arg {
        ($setter:ident, $name:literal) => {{
            let log = log.clone();
            ui.$setter(move |n: i32| log.borrow_mut().push(format!("{}({n})", $name)));
        }};
    }
    match fires {
        Fires::BackRequested => spy!(on_back_requested, "back-requested"),
        Fires::ForwardRequested => spy!(on_forward_requested, "forward-requested"),
        Fires::PaneCycled(_) => spy_arg!(on_pane_cycled, "pane-cycled"),
        Fires::RefreshRequested => spy!(on_refresh_requested, "refresh-requested"),
        Fires::UndoRequested => spy!(on_undo_requested, "undo-requested"),
        Fires::SaveRequested => spy!(on_save_requested, "save-requested"),
        Fires::EdgeRequested(_) => spy_arg!(on_edge_requested, "edge-requested"),
        Fires::EdgeExtended(_) => spy_arg!(on_edge_extended, "edge-extended"),
        Fires::SelectionMoved(_) => spy_arg!(on_selection_moved, "selection-moved"),
        Fires::SelectionExtended(_) => spy_arg!(on_selection_extended, "selection-extended"),
        Fires::ParentRequested => spy!(on_parent_requested, "parent-requested"),
        Fires::ReturnPressed => spy!(on_return_pressed, "return-pressed"),
        Fires::ContentRenameRequested => {
            spy!(on_content_rename_requested, "content-rename-requested");
        }
        Fires::DeleteRequested => spy!(on_delete_requested, "delete-requested"),
        Fires::BackspacePressed => spy!(on_backspace_pressed, "backspace-pressed"),
        Fires::NewFolderRequested => spy!(on_new_folder_requested, "new-folder-requested"),
        Fires::NewFileRequested => spy!(on_new_file_requested, "new-file-requested"),
        Fires::SelectAllRequested => spy!(on_select_all_requested, "select-all-requested"),
        Fires::ClipboardCopyRequested => {
            spy!(on_clipboard_copy_requested, "clipboard-copy-requested");
        }
        Fires::ClipboardCutRequested => {
            spy!(on_clipboard_cut_requested, "clipboard-cut-requested");
        }
        Fires::ClipboardPasteRequested => {
            spy!(on_clipboard_paste_requested, "clipboard-paste-requested");
        }
        Fires::KeyText(_) => {
            let log = log.clone();
            ui.on_key_text(move |text| {
                log.borrow_mut()
                    .push(format!("key-text({:?})", text.as_str()));
            });
        }
        Fires::PathEditRequested => spy!(on_path_edit_requested, "path-edit-requested"),
        Fires::EditRequested => spy!(on_edit_requested, "edit-requested"),
        Fires::FindRequested => spy!(on_find_requested, "find-requested"),
        Fires::FilterFocusRequested => spy!(on_filter_focus_requested, "filter-focus-requested"),
        Fires::SwitcherOpenRequested => {
            spy!(on_switcher_open_requested, "switcher-open-requested");
        }
        Fires::CancelRequested => spy!(on_cancel_requested, "cancel-requested"),
        Fires::ZoomInRequested => spy!(on_zoom_in_requested, "zoom-in-requested"),
        Fires::ZoomOutRequested => spy!(on_zoom_out_requested, "zoom-out-requested"),
        Fires::ZoomResetRequested => spy!(on_zoom_reset_requested, "zoom-reset-requested"),
    }
}

#[test]
fn every_table_binding_reaches_the_callback_it_names() {
    i_slint_backend_testing::init_no_event_loop();
    let (ui, _app) = window("bindings");

    for binding in shortcuts::BINDINGS {
        let log = Rc::new(RefCell::new(Vec::new()));
        install_spy(&ui, binding.fires, &log);

        press(&ui, binding);

        assert_eq!(
            *log.borrow(),
            vec![binding.fires.label()],
            "{} ({}) should fire {}",
            binding.label(false),
            binding.description,
            binding.fires.label()
        );
    }
}
