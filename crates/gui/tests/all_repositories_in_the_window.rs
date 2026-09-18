//! The All Repositories view (#591), driven end to end through a real
//! window on a real [`App`], against real checkouts built by real `git` in
//! a scratch directory nested the way the work order describes:
//! `repos/github/<owner>/<project>`.
//!
//! `service`'s own tests prove the background scan finds the right
//! checkouts at the right depths; `app`'s own tests prove the Contents
//! pane's Location column and the status bar's wording against hand-built
//! responses. Neither half can see what a reader meets: View > All
//! Repositories clicked in a real menu, rows landing in a real Contents
//! pane once a real service has walked a real nested tree, and Return
//! taking the reader to a real folder. That seam is what this file
//! measures, per CLAUDE.md rule 14.
//!
//! Per rule 8 this drives `git` only inside directories it created under
//! the platform's temporary directory - [`git`] refuses anything else.

use gui::app::App;
use gui::{MainWindow, sync_ui};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, Model as _};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

mod common;
use common::ensure_service;

/// One window and one service at a time: Slint's testing backend is a
/// process-wide platform.
static SERIAL: Mutex<()> = Mutex::new(());

/// Takes the shared lock, tolerating a previous test having panicked while
/// holding it - a poisoned lock would otherwise turn one failure into many.
fn serially() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---------------------------------------------------------------------
// Fixtures: real checkouts, built by real git, inside a scratch directory.
// ---------------------------------------------------------------------

/// The one directory under which every fixture in this file is built.
fn scratch_root() -> PathBuf {
    std::env::temp_dir().join("rse-all-repositories-in-the-window")
}

/// An empty directory of this test's own. Nothing in this file ever
/// touches a path outside [`scratch_root`].
fn scratch(name: &str) -> PathBuf {
    let dir = scratch_root().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// Runs `git` in `dir`, which must be inside [`scratch_root`] (rule 8).
fn git(dir: &Path, arguments: &[&str]) {
    assert!(
        dir.starts_with(scratch_root()),
        "refusing to run git outside the scratch directory: {}",
        dir.display()
    );
    let output = Command::new("git")
        .args(arguments)
        .current_dir(dir)
        .output()
        .expect("git should be on PATH");
    assert!(
        output.status.success(),
        "git {arguments:?} failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A committed checkout at `root/name`, `name` possibly a nested path such
/// as `"github/owner/project"`.
fn checkout(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    git(&dir, &["init", "--quiet", "--initial-branch", "main", "."]);
    git(&dir, &["config", "user.name", "Repos Explorer Test"]);
    git(&dir, &["config", "user.email", "test@example.invalid"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    std::fs::write(dir.join("tracked.txt"), "the committed body\n").expect("a scratch file");
    git(&dir, &["add", "--all"]);
    git(&dir, &["commit", "--quiet", "--message", "a commit"]);
    dir
}

// ---------------------------------------------------------------------
// The harness, in the shape `repository_in_the_window.rs` uses.
// ---------------------------------------------------------------------

/// A shown window on a real `App` rooted at `root`, wired by the crate's
/// own wiring - the same call `main` makes.
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

/// Ticks the application the way the window's 100ms timer does, until
/// every in-flight request has landed and stayed landed.
fn settle(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut quiet = 0u32;
    while Instant::now() < deadline {
        {
            let mut app = app.borrow_mut();
            app.tick();
            sync_ui(ui, &app);
        }
        if still_working(&ui.get_status_text()) {
            quiet = 0;
        } else {
            quiet += 1;
            if quiet >= 20 {
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

/// The names the contents pane is drawing, in the order it draws them.
fn listing(ui: &MainWindow) -> Vec<String> {
    ui.get_content_rows()
        .iter()
        .map(|row| row.name.trim_end_matches('/').to_owned())
        .collect()
}

/// What the Contents pane's Location column says for the row named `name`.
fn location_of(ui: &MainWindow, name: &str) -> String {
    ui.get_content_rows()
        .iter()
        .find(|row| row.name.trim_end_matches('/') == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing: {:?}", listing(ui)))
        .kind
        .to_string()
}

/// A click at `handle`'s position, the way a mouse presses and releases in
/// place.
fn click_element(handle: &ElementHandle) {
    handle.mock_single_click(PointerEventButton::Left);
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

/// Opens View > All Repositories through the real menu.
fn open_all_repositories_from_the_menu(ui: &MainWindow) {
    open_menu(ui, "View");
    item(ui, "All Repositories").mock_single_click(PointerEventButton::Left);
}

// ---------------------------------------------------------------------

#[test]
fn view_all_repositories_shows_every_working_copy_nested_up_to_three_levels_with_its_location() {
    let _serial = serially();
    let root = scratch("nested-listing");
    checkout(&root, "top-level");
    checkout(&root, "github/owner/project");
    let (ui, app) = window_at(&root);

    open_all_repositories_from_the_menu(&ui);
    settle_the_scan(&ui, &app);

    assert!(
        listing(&ui).contains(&"top-level".to_owned()),
        "a direct child should be found too (acceptance check 6): {:?}",
        listing(&ui)
    );
    assert!(
        listing(&ui).contains(&"project".to_owned()),
        "a checkout nested three levels down should be found: {:?}",
        listing(&ui)
    );
    assert_eq!(
        location_of(&ui, "top-level"),
        ".",
        "a direct child's location is the root itself"
    );
    assert_eq!(
        location_of(&ui, "project"),
        "github/owner",
        "the Location column names the nested checkout's parent path"
    );
}

#[test]
fn the_folders_trees_own_entry_opens_the_same_view() {
    let _serial = serially();
    let root = scratch("tree-entry");
    checkout(&root, "top-level");
    let (ui, app) = window_at(&root);

    let entry = ElementHandle::find_by_accessible_label(&ui, "All repositories")
        .next()
        .expect("the Folders tree has an All repositories entry");
    click_element(&entry);
    settle_the_scan(&ui, &app);

    assert!(listing(&ui).contains(&"top-level".to_owned()));
}

#[test]
fn returning_on_a_nested_row_goes_to_its_real_folder() {
    let _serial = serially();
    let root = scratch("return-to-real-folder");
    checkout(&root, "github/owner/project");
    let (ui, app) = window_at(&root);

    open_all_repositories_from_the_menu(&ui);
    settle_the_scan(&ui, &app);
    assert_eq!(listing(&ui), vec!["project".to_owned()]);

    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: char::from(Key::Return).into(),
    });
    ui.window().dispatch_event(WindowEvent::KeyReleased {
        text: char::from(Key::Return).into(),
    });
    settle(&ui, &app);

    assert_eq!(
        PathBuf::from(app.borrow().current_path()),
        root.join("github").join("owner"),
        "Return should land in the repository's own real folder"
    );
    assert!(
        listing(&ui).contains(&"project".to_owned()),
        "and that folder's own listing should hold it: {:?}",
        listing(&ui)
    );
}
