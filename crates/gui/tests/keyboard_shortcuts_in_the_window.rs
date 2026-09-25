//! Walks `shortcuts::BINDINGS` against a real window, per CLAUDE.md rule 14
//! and #585's first acceptance check: every entry's keys, dispatched
//! through the same `key-scope` `ui/app.slint` gives the window, must
//! reach the callback the table says it does. A binding removed from the
//! markup, given the wrong modifiers, or pointed at the wrong callback,
//! fails here rather than only in the reference sheet nobody reads until
//! the key stops working.
//!
//! #721 adds GUIDANCE.md §2.3's macOS behaviour profile - Command in place
//! of Control - so every window built here is given an explicit `os`
//! rather than the host's own, and the walk below runs once per
//! non-macOS profile so a change here cannot silently stop proving
//! Windows and Linux unchanged (its own acceptance check).

use gui::app::App;
use gui::shortcuts::{self, Fires};
use gui::{MainWindow, sync_ui};
use slint::ComponentHandle;
use slint::platform::{Key, WindowEvent};
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

/// A shown window on a real `App` built under `os`'s behaviour profile
/// (#721) - no callback is wired, since this test asks only whether a key
/// reaches the right one, not what the application then does with it.
fn window(name: &str, os: &'static str) -> (MainWindow, Rc<RefCell<App>>) {
    let dir = scratch(name);
    let mut app = App::new(dir);
    app.set_os_for_test(os);
    let app = Rc::new(RefCell::new(app));
    let ui = MainWindow::new().expect("the window should build");
    sync_ui(&ui, &app.borrow());
    ui.show().expect("the window should show");
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    (ui, app)
}

/// Dispatches `modifiers`, then `binding`'s own key, then releases both in
/// reverse - real `WindowEvent`s, the way a keyboard actually reports a
/// chord, rather than a direct call to a Slint callback.
fn press_with(ui: &MainWindow, binding: &shortcuts::Binding, modifiers: &[Key]) {
    let window = ui.window();
    for key in modifiers {
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

/// [`press_with`], with `binding`'s own modifiers resolved for `os`
/// (#721) - Command in place of Control under the macOS profile.
fn press(ui: &MainWindow, binding: &shortcuts::Binding, os: &str) {
    press_with(ui, binding, &binding.modifiers(os));
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
        Fires::ChangedFilterToggled => {
            spy!(on_changed_filter_toggled, "changed-filter-toggled");
        }
    }
}

/// The full table walk under one behaviour profile: every binding's keys,
/// resolved for `os`, reach exactly the callback it names.
fn assert_every_binding_reaches_its_callback(os: &'static str) {
    let (ui, _app) = window(&format!("bindings-{os}"), os);

    for binding in shortcuts::BINDINGS {
        let log = Rc::new(RefCell::new(Vec::new()));
        install_spy(&ui, binding.fires, &log);

        // Delete's own row holds no modifier in the table - plain Delete
        // is what every profile fires on save one, the macOS profile,
        // where Trash (#721 item D) asks for Command too. That is a
        // narrower rule than the table's generic Control-or-Command
        // resolution (#721 item A), so it is dispatched here rather than
        // folded into `Binding::modifiers`.
        if os == "macos" && matches!(binding.fires, Fires::DeleteRequested) {
            press_with(&ui, binding, &[Key::Meta]);
        } else {
            press(&ui, binding, os);
        }

        assert_eq!(
            *log.borrow(),
            vec![binding.fires.label()],
            "on {os}: {} ({}) should fire {}",
            binding.label(os == "macos"),
            binding.description,
            binding.fires.label()
        );
    }
}

#[test]
fn every_table_binding_reaches_the_callback_it_names_on_macos() {
    i_slint_backend_testing::init_no_event_loop();
    assert_every_binding_reaches_its_callback("macos");
}

/// #721's fourth acceptance check: Windows' bindings are unchanged by the
/// macOS profile - the same walk as the macOS run above, still on Control.
#[test]
fn every_table_binding_reaches_the_callback_it_names_on_windows() {
    i_slint_backend_testing::init_no_event_loop();
    assert_every_binding_reaches_its_callback("windows");
}

/// #721's fourth acceptance check: Linux's bindings are unchanged by the
/// macOS profile - the same walk as the macOS run above, still on Control.
#[test]
fn every_table_binding_reaches_the_callback_it_names_on_linux() {
    i_slint_backend_testing::init_no_event_loop();
    assert_every_binding_reaches_its_callback("linux");
}

/// #721's second acceptance check: a Command chord fires under the macOS
/// profile, and the identical physical chord does not fire under the
/// other two - proved for a representative Control binding (Undo) rather
/// than the whole table, which the walks above already cover on their own
/// modifier.
#[test]
fn a_command_chord_fires_undo_only_under_the_macos_profile() {
    i_slint_backend_testing::init_no_event_loop();
    let undo = shortcuts::BINDINGS
        .iter()
        .find(|b| matches!(b.fires, Fires::UndoRequested) && b.control)
        .expect("Undo holds Control/Command in the table");

    for os in ["windows", "linux", "macos"] {
        let (ui, _app) = window(&format!("command-chord-{os}"), os);
        let log = Rc::new(RefCell::new(Vec::new()));
        install_spy(&ui, undo.fires, &log);

        press_with(&ui, undo, &[Key::Meta]);

        if os == "macos" {
            assert_eq!(
                *log.borrow(),
                vec![undo.fires.label()],
                "Cmd+Z should fire Undo under the macOS profile"
            );
        } else {
            assert!(
                log.borrow().is_empty(),
                "Cmd+Z should not fire Undo under the {os} profile, only Ctrl+Z should"
            );
        }
    }
}

/// #721 item 3: Cmd+Delete moves to Trash under the macOS profile - bare
/// Delete, which every other profile still fires on, is refused there,
/// and only the Command chord reaches `delete-requested()`.
#[test]
fn delete_requires_command_under_the_macos_profile_only() {
    i_slint_backend_testing::init_no_event_loop();
    let delete = shortcuts::BINDINGS
        .iter()
        .find(|b| matches!(b.fires, Fires::DeleteRequested))
        .expect("Delete is in the table");

    for os in ["windows", "linux", "macos"] {
        let (ui, _app) = window(&format!("delete-{os}"), os);
        let log = Rc::new(RefCell::new(Vec::new()));
        install_spy(&ui, Fires::DeleteRequested, &log);

        press_with(&ui, delete, &[]);
        if os == "macos" {
            assert!(
                log.borrow().is_empty(),
                "bare Delete should not fire delete-requested under the macOS profile"
            );
        } else {
            assert_eq!(
                *log.borrow(),
                vec![Fires::DeleteRequested.label()],
                "bare Delete should still fire delete-requested on {os}"
            );
        }

        log.borrow_mut().clear();
        press_with(&ui, delete, &[Key::Meta]);
        if os == "macos" {
            assert_eq!(
                *log.borrow(),
                vec![Fires::DeleteRequested.label()],
                "Cmd+Delete should fire delete-requested under the macOS profile"
            );
        }
    }
}
