//! Use-case journey (#728): change the Repos Directory, and everything
//! moves with it.
//!
//! The Folders tree, the Contents listing, the address bar, All
//! Repositories and the Ctrl+P switcher each read the Repos Directory
//! independently, so each is a place a change to it can fail to reach.
//! D9 settles the root as a list with one entry marked active, and D8
//! settles the boundary around it as soft - one configuration point, not a
//! rule spread through the navigation code. This file is the seam CLAUDE.md
//! rule 14 asks for: nothing before it drove the change through the real
//! menu and looked at what every surface then shows.
//!
//! Full stack throughout: a real Repos Directory holding real working
//! copies, a real service on the private socket `common::ensure_service`
//! provides - its own settings file redirected by
//! `service::repos::use_private_config_dir` so a real `SetReposRoot` here
//! never touches the machine's own configuration - and a real `MainWindow`
//! joined to a real `App` by `gui::wire_callbacks`, the function `main`
//! calls, never a copy of it.

use gui::app::App;
use gui::{MainWindow, sync_ui};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, Model as _};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

mod common;
use common::ensure_service;

/// The Slint testing backend is a process-wide platform, and this suite
/// changes the one configured Repos Directory more than once, so its tests
/// run one at a time, the way `operations_in_the_window.rs` does.
static SERIAL: Mutex<()> = Mutex::new(());

/// Takes the shared lock, tolerating a previous test having panicked while
/// holding it - a poisoned lock would otherwise turn one failure into many.
fn serially() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---------------------------------------------------------------------
// Fixtures: real folders under a scratch directory of this file's own.
// ---------------------------------------------------------------------

/// The one directory under which every fixture in this file is built.
fn scratch_root() -> PathBuf {
    std::env::temp_dir().join("rse-change-repos-directory")
}

/// An empty directory of this test's own. Nothing in this file ever
/// touches a path outside [`scratch_root`].
fn scratch(name: &str) -> PathBuf {
    let dir = scratch_root().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// A folder the directory plugin reads as a working copy: `HEAD` is all it
/// looks at for one (`switcher_in_the_window.rs` uses the same fixture).
/// No `git` runs (rule 8), and this journey does not need a real checkout -
/// only that every surface agrees on which folders are repositories.
fn checkout(root: &Path, name: &str) {
    let git = root.join(name).join(".git");
    std::fs::create_dir_all(&git).expect("a checkout's git directory");
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");
}

// ---------------------------------------------------------------------
// The harness, in the shape `reading_and_editing_journey_in_the_window.rs`
// uses.
// ---------------------------------------------------------------------

/// A shown window on a real `App` rooted at `root`, wired by the crate's own
/// wiring - the same call `main` makes.
fn window_at(root: &Path) -> (MainWindow, Rc<RefCell<App>>) {
    ensure_service();
    i_slint_backend_testing::init_no_event_loop();
    let app = Rc::new(RefCell::new(App::new(root.to_path_buf())));
    let ui = MainWindow::new().expect("the window should build");
    sync_ui(&ui, &app.borrow());
    gui::wire_callbacks(&ui, &app);
    ui.show().expect("the window should show");
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    settle(&ui, &app);
    (ui, app)
}

/// Whether `status` is one of the transient lines an in-flight request puts
/// up, which is how [`settle`] knows the application is still working.
fn still_working(status: &str) -> bool {
    status.starts_with("loading ") || matches!(status, "working..." | "deleting..." | "undoing...")
}

/// Ticks the application the way the window's 100ms timer does, until every
/// in-flight request has landed and stayed landed - `SetReposRoot` and the
/// listing that follows it included, since neither puts a line in
/// [`still_working`]'s list on its own.
fn settle(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut quiet = 0u32;
    while Instant::now() < deadline {
        {
            let mut app = app.borrow_mut();
            app.tick();
            sync_ui(ui, &app);
        }
        if still_working(&ui.get_status_text()) || app.borrow().is_busy() {
            quiet = 0;
        } else {
            quiet += 1;
            if quiet >= 8 {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "the application never settled; status: {}",
        ui.get_status_text()
    );
}

/// Ticks until the All Repositories scan reports itself done - the status
/// bar stops saying "Looking for repositories".
fn settle_the_scan(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        {
            let mut app = app.borrow_mut();
            app.tick();
            sync_ui(ui, &app);
        }
        if !ui.get_status_text().contains("Looking for repositories") {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the scan never finished; status: {}", ui.get_status_text());
}

/// Presses `text` as a key with `modifiers` held. Slint tracks modifier
/// state from the modifier key's own press, so holding one means pressing
/// and releasing it around the key itself.
fn press_with(ui: &MainWindow, text: &str, modifiers: &[Key]) {
    let window = ui.window();
    for modifier in modifiers {
        window.dispatch_event(WindowEvent::KeyPressed {
            text: char::from(*modifier).into(),
        });
    }
    window.dispatch_event(WindowEvent::KeyPressed { text: text.into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: text.into() });
    for modifier in modifiers.iter().rev() {
        window.dispatch_event(WindowEvent::KeyReleased {
            text: char::from(*modifier).into(),
        });
    }
}

/// Presses `text` as a key with nothing held.
fn press(ui: &MainWindow, text: &str) {
    press_with(ui, text, &[]);
}

/// Presses a named key such as `Key::Return` with nothing held.
fn press_key(ui: &MainWindow, key: Key) {
    press_with(ui, &char::from(key).to_string(), &[]);
}

/// Types `text` one character at a time, the way a keyboard delivers it.
fn type_text(ui: &MainWindow, text: &str) {
    for c in text.chars() {
        press(ui, &c.to_string());
    }
}

/// Clears a pre-filled prompt by backspacing over every character of it -
/// the pattern `operations_in_the_window.rs::clear_prompt` uses for rename.
fn clear_prompt(ui: &MainWindow, characters: usize) {
    for _ in 0..characters {
        press_key(ui, Key::Backspace);
    }
}

/// Ctrl+P, dispatched as the chord a keyboard actually reports: the
/// modifier down, the letter, then both released in reverse
/// (`switcher_in_the_window.rs::press_ctrl_p`).
fn press_ctrl_p(ui: &MainWindow) {
    let window = ui.window();
    let control = slint::SharedString::from(char::from(Key::Control).to_string());
    window.dispatch_event(WindowEvent::KeyPressed {
        text: control.clone(),
    });
    window.dispatch_event(WindowEvent::KeyPressed { text: "p".into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: "p".into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: control });
}

/// The path typed into the Repos Directory prompt so far, once File > Repos
/// Directory... - or the message's own "Choose Repos Directory..." - has
/// opened it: the prompt's own text with its fixed leading question
/// stripped off.
fn seeded_repos_root_input(ui: &MainWindow) -> String {
    ui.get_content_prompt_text()
        .to_string()
        .strip_prefix("Where are your repositories?  ")
        .expect("the Repos Directory prompt should be open")
        .to_owned()
}

/// Every menu item currently on screen, in whichever menu is open.
fn open_items(ui: &MainWindow) -> Vec<ElementHandle> {
    ElementHandle::find_by_element_type_name(ui, "ContextMenuItem").collect()
}

/// The open menu's item labelled `label`.
fn item(ui: &MainWindow, label: &str) -> ElementHandle {
    let mut found: Vec<ElementHandle> = open_items(ui)
        .into_iter()
        .filter(|item| item.accessible_label().is_some_and(|found| found == label))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "exactly one open menu item should be labelled {label:?}"
    );
    found.remove(0)
}

/// Opens the menu-bar menu titled `title`, the topmost element carrying
/// that label.
fn open_menu(ui: &MainWindow, title: &str) {
    let mut titles: Vec<ElementHandle> =
        ElementHandle::find_by_accessible_label(ui, title).collect();
    titles.sort_by(|a, b| {
        a.absolute_position()
            .y
            .partial_cmp(&b.absolute_position().y)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    titles
        .first()
        .unwrap_or_else(|| panic!("the menu bar has no title {title:?}"))
        .mock_single_click(PointerEventButton::Left);
    assert!(
        !open_items(ui).is_empty(),
        "clicking the {title:?} title should have opened a menu"
    );
}

/// Opens `title` and chooses `label` from it.
fn from_menu(ui: &MainWindow, title: &str, label: &str) {
    open_menu(ui, title);
    item(ui, label).mock_single_click(PointerEventButton::Left);
}

/// The names the Contents pane is drawing, in the order it draws them. A
/// directory carries a trailing separator, which is stripped here so a test
/// can name a folder the way the filesystem does.
fn listing(ui: &MainWindow) -> Vec<String> {
    ui.get_content_rows()
        .iter()
        .map(|row| row.name.trim_end_matches('/').to_owned())
        .collect()
}

/// The names the Folders tree is drawing, root first, in the order it
/// draws them.
fn folder_names(ui: &MainWindow) -> Vec<String> {
    ui.get_folder_rows()
        .iter()
        .map(|row| row.name.to_string())
        .collect()
}

/// The last segment the address bar is drawing - the folder actually
/// browsed.
fn address_leaf(ui: &MainWindow) -> String {
    ui.get_breadcrumbs()
        .iter()
        .last()
        .map(|crumb| crumb.to_string())
        .unwrap_or_default()
}

/// Opens View > All Repositories through the real menu.
fn open_all_repositories_from_the_menu(ui: &MainWindow) {
    from_menu(ui, "View", "All Repositories");
}

/// The names the Ctrl+P switcher is offering right now.
fn switcher_names(ui: &MainWindow) -> Vec<String> {
    ui.get_switcher_rows()
        .iter()
        .map(|row| row.name.to_string())
        .collect()
}

/// Types `target` into the Repos Directory prompt, opened from the menu,
/// and confirms it - the same steps a reader takes to change where the
/// application opens from now on. Asserts D9's active root moved to
/// `target`, through the same service the window is using.
fn set_repos_directory(ui: &MainWindow, app: &Rc<RefCell<App>>, target: &Path) {
    from_menu(ui, "File", "Repos Directory...");
    clear_prompt(ui, seeded_repos_root_input(ui).len());
    type_text(ui, &target.to_string_lossy());
    press_key(ui, Key::Return);
    settle(ui, app);

    assert_eq!(
        gui::app::opening().root,
        target,
        "the service should now open here from now on (D9's active root)"
    );
}

/// Checks that every surface which reads the Repos Directory agrees: each
/// name in `present` is shown, each name in `absent` is not - the Contents
/// listing, the Folders tree (rooted at `root` itself), the address bar,
/// All Repositories and the Ctrl+P switcher.
fn assert_every_surface_shows_only(
    ui: &MainWindow,
    app: &Rc<RefCell<App>>,
    root: &Path,
    present: &[&str],
    absent: &[&str],
) {
    let root_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .expect("a scratch root has a name")
        .to_owned();

    let shown = listing(ui);
    for name in present {
        assert!(
            shown.contains(&(*name).to_owned()),
            "the Contents listing should hold {name}: {shown:?}"
        );
    }
    for name in absent {
        assert!(
            !shown.contains(&(*name).to_owned()),
            "the Contents listing should not hold {name}: {shown:?}"
        );
    }

    let tree = folder_names(ui);
    assert_eq!(
        tree.first(),
        Some(&root_name),
        "the Folders tree's own row should be the current root: {tree:?}"
    );
    for name in present {
        assert!(
            tree.contains(&(*name).to_owned()),
            "the Folders tree should hold {name}: {tree:?}"
        );
    }
    for name in absent {
        assert!(
            !tree.contains(&(*name).to_owned()),
            "the Folders tree should not hold {name}: {tree:?}"
        );
    }

    assert_eq!(
        address_leaf(ui),
        root_name,
        "the address bar should end on the current root"
    );

    open_all_repositories_from_the_menu(ui);
    settle_the_scan(ui, app);
    let scanned = listing(ui);
    for name in present {
        assert!(
            scanned.contains(&(*name).to_owned()),
            "All Repositories should find {name}: {scanned:?}"
        );
    }
    for name in absent {
        assert!(
            !scanned.contains(&(*name).to_owned()),
            "All Repositories should not find {name}: {scanned:?}"
        );
    }
    press_key(ui, Key::Escape);

    press_ctrl_p(ui);
    assert!(ui.get_switcher_open(), "Ctrl+P should open the switcher");
    let offered = switcher_names(ui);
    for name in present {
        assert!(
            offered.contains(&(*name).to_owned()),
            "the switcher should offer {name}: {offered:?}"
        );
    }
    for name in absent {
        assert!(
            !offered.contains(&(*name).to_owned()),
            "the switcher should not offer {name}: {offered:?}"
        );
    }
    press_key(ui, Key::Escape);
    assert!(!ui.get_switcher_open(), "Escape should close the switcher");
}

// ---------------------------------------------------------------------

/// The journey: open at one Repos Directory, change to a second through the
/// real File > Repos Directory... prompt, see every surface follow, then
/// change back and see everything return.
#[test]
fn changing_the_repos_directory_moves_every_surface_that_reads_it() {
    let _serial = serially();
    let root_one = scratch("root-one");
    checkout(&root_one, "alpha");
    checkout(&root_one, "beta");
    let root_two = scratch("root-two");
    checkout(&root_two, "gamma");
    checkout(&root_two, "delta");

    let (ui, app) = window_at(&root_one);

    assert_every_surface_shows_only(
        &ui,
        &app,
        &root_one,
        &["alpha", "beta"],
        &["gamma", "delta"],
    );

    set_repos_directory(&ui, &app, &root_two);
    assert_every_surface_shows_only(
        &ui,
        &app,
        &root_two,
        &["gamma", "delta"],
        &["alpha", "beta"],
    );

    set_repos_directory(&ui, &app, &root_one);
    assert_every_surface_shows_only(
        &ui,
        &app,
        &root_one,
        &["alpha", "beta"],
        &["gamma", "delta"],
    );
}

/// Setting the Repos Directory to somewhere missing shows the titled
/// message with Retry rather than an operating-system error, and choosing
/// a real folder afterwards recovers.
///
/// The message's Retry and Choose buttons are reached by keyboard - Tab
/// moves the highlight, Return activates it - rather than a simulated
/// click: the Contents pane's own full-size click area
/// (`panes/contents_pane.slint`'s `click-area`) is declared, and therefore
/// drawn, after the message, so a pointer press aimed at either button
/// lands on it instead. Tab and Return are the reachable path the message
/// was built for (`contents_message_in_the_window.rs`'s own "keyboard
/// reachable" tests use it the same way).
#[test]
fn a_missing_repos_directory_shows_the_titled_message_and_a_real_one_recovers() {
    let _serial = serially();
    let good_root = scratch("recovery-good");
    checkout(&good_root, "alpha");
    let missing_root = scratch_root().join("recovery-missing");
    let _ = std::fs::remove_dir_all(&missing_root);

    let (ui, app) = window_at(&good_root);

    from_menu(&ui, "File", "Repos Directory...");
    clear_prompt(&ui, seeded_repos_root_input(&ui).len());
    type_text(&ui, &missing_root.to_string_lossy());
    press_key(&ui, Key::Return);
    settle(&ui, &app);

    assert!(
        ui.get_message_title().contains("is not available"),
        "a missing root should show the titled message, not an operating-system \
         error; the title is {:?}",
        ui.get_message_title()
    );
    assert_eq!(ui.get_message_detail(), "The folder does not exist");
    assert!(ui.get_message_show_retry());
    assert!(ui.get_message_show_choose());

    // Tab from Retry (the message's first button) to Choose, and Return
    // activates it - the same as `App::move_message_focus` and
    // `activate_focused_message_button`'s own unit tests exercise.
    press_key(&ui, Key::Tab);
    press_key(&ui, Key::Return);
    clear_prompt(&ui, seeded_repos_root_input(&ui).len());
    type_text(&ui, &good_root.to_string_lossy());
    press_key(&ui, Key::Return);
    settle(&ui, &app);

    assert_eq!(
        ui.get_message_title(),
        "",
        "a real folder should clear the missing-root message"
    );
    assert_eq!(listing(&ui), vec!["alpha".to_owned()]);
}
