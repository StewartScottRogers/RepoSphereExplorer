//! A repository's README (#584), driven end to end through a real window
//! on a real [`App`], against real checkouts made by real `git` in a
//! scratch directory.
//!
//! `plugin-directory`'s own `readme` module tests prove the excerpt parser
//! alone: the title, the second-heading stop, the line cap, badges and
//! links. Neither that crate nor `app`'s own unit tests can see what a
//! reader meets: a checkout selected with a click, its README's opening
//! drawn below its facts, and a link that opens the file in full. That
//! seam is what this file measures, per rule 14.
//!
//! Per rule 8 this drives `git` only inside directories it created under
//! the platform's temporary directory, and only to *build* fixtures.

use gui::app::App;
use gui::{MainWindow, sync_ui};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model as _};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

mod common;
use common::ensure_service;

/// Row height in `app.slint`'s contents pane, so a click can be aimed at a
/// row.
const ROW_HEIGHT: f32 = 20.0;

/// One window and one service at a time, matching
/// `repository_in_the_window.rs`'s own reasoning.
static SERIAL: Mutex<()> = Mutex::new(());

fn serially() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn scratch_root() -> PathBuf {
    std::env::temp_dir().join("rse-readme-in-the-window")
}

fn scratch(name: &str) -> PathBuf {
    let dir = scratch_root().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

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

fn init_checkout(dir: &Path, branch: &str) {
    std::fs::create_dir_all(dir).expect("a scratch directory");
    git(dir, &["init", "--quiet", "--initial-branch", branch, "."]);
    git(dir, &["config", "user.name", "Repos Explorer Test"]);
    git(dir, &["config", "user.email", "test@example.invalid"]);
    git(dir, &["config", "core.autocrlf", "false"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

fn commit_everything(dir: &Path) {
    git(dir, &["add", "--all"]);
    git(dir, &["commit", "--quiet", "--message", "a commit"]);
}

fn file(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).expect("a scratch file");
}

/// A committed checkout at `root/name`, on `main`, holding one tracked
/// file plus `readme_body` as `README.md` when given.
fn checkout(root: &Path, name: &str, readme_body: Option<&str>) -> PathBuf {
    let dir = root.join(name);
    init_checkout(&dir, "main");
    file(&dir, "tracked.txt", "the committed body\n");
    if let Some(body) = readme_body {
        file(&dir, "README.md", body);
    }
    commit_everything(&dir);
    dir
}

/// A plain folder at `root/name`, not a checkout, optionally holding a
/// `README.md` of its own - proving requirement 4's "not a working copy
/// gets no README section" needs more than a README on disk.
fn plain_folder(root: &Path, name: &str, readme_body: Option<&str>) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    file(&dir, "note.txt", "just a note\n");
    if let Some(body) = readme_body {
        file(&dir, "README.md", body);
    }
    dir
}

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

fn still_working(status: &str) -> bool {
    status.starts_with("loading ") || matches!(status, "working..." | "deleting..." | "undoing...")
}

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

fn settle_until_pane_says(ui: &MainWindow, app: &Rc<RefCell<App>>, marker: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        {
            let mut app = app.borrow_mut();
            app.tick();
            sync_ui(ui, &app);
        }
        if pane_text(ui).contains(marker) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "the File pane never showed {marker:?}; it reads:\n{}",
        pane_text(ui)
    );
}

/// The File pane's fact table, its plain text, and its README section's
/// own title and excerpt (#584 - drawn through dedicated properties rather
/// than `file-text`, since the `directory` plugin's fact table is never
/// empty and so its own `present` lines, which do carry the README, never
/// reach the pane), joined back into one string -
/// `repository_in_the_window.rs`'s own `pane_text`, extended.
fn pane_text(ui: &MainWindow) -> String {
    let facts: Vec<String> = ui
        .get_file_facts()
        .iter()
        .map(|fact| {
            if fact.label.is_empty() {
                String::new()
            } else {
                format!("{}: {}", fact.label, fact.full_value)
            }
        })
        .collect();
    format!(
        "{}\n{}\n{}\n{}",
        facts.join("\n"),
        ui.get_file_text(),
        ui.get_file_readme_title(),
        ui.get_file_readme_excerpt()
    )
}

fn click_row(ui: &MainWindow, rows_down: f32) {
    let pane = ElementHandle::find_by_element_id(ui, "ContentsPane::click-area")
        .next()
        .expect("the contents pane has a click area");
    let origin = pane.absolute_position();
    let position = LogicalPosition::new(
        origin.x + 20.0,
        origin.y + rows_down.mul_add(ROW_HEIGHT, ROW_HEIGHT / 2.0),
    );
    let window = ui.window();
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

fn listing(ui: &MainWindow) -> Vec<String> {
    ui.get_content_rows()
        .iter()
        .map(|row| row.name.trim_end_matches('/').to_owned())
        .collect()
}

fn row_of(ui: &MainWindow, name: &str) -> f32 {
    let index = listing(ui)
        .iter()
        .position(|drawn| drawn == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing: {:?}", listing(ui)));
    let index = u16::try_from(index).expect("a small listing");
    f32::from(index)
}

fn select_and_wait(ui: &MainWindow, app: &Rc<RefCell<App>>, name: &str, marker: &str) {
    click_row(ui, row_of(ui, name));
    settle_until_pane_says(ui, app, marker);
}

/// A click at `handle`'s position, matching
/// `repository_in_the_window.rs`'s own `click_element`.
fn click_element(ui: &MainWindow, handle: &ElementHandle) {
    let position = handle.absolute_position();
    let window = ui.window();
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

/// The File pane's Open README link - the only `StatusLink` on screen
/// while nothing has touched the status bar's own changed-count filter.
fn the_open_readme_link(ui: &MainWindow) -> ElementHandle {
    ElementHandle::find_by_element_id(ui, "StatusLink::link-touch")
        .next()
        .expect("the Open README link is drawn")
}

const README_BODY: &str = "\
# Kestrel

A small, cancellable queue.

## Details

More than the excerpt should ever show.
";

#[test]
fn a_working_copys_readme_shows_its_title_and_excerpt() {
    let _serial = serially();
    let root = scratch("shows-title-and-excerpt");
    checkout(&root, "alpha", Some(README_BODY));
    let (ui, app) = window_at(&root);

    select_and_wait(&ui, &app, "alpha", "Kestrel");

    let pane = pane_text(&ui);
    assert!(pane.contains("Kestrel"), "the title should show: {pane}");
    assert!(
        pane.contains("A small, cancellable queue."),
        "the excerpt should show: {pane}"
    );
    assert!(
        !pane.contains("More than the excerpt should ever show."),
        "text past the second heading should not show: {pane}"
    );
    assert_eq!(ui.get_file_readme_name(), "README.md");
}

#[test]
fn a_plain_folders_readme_shows_no_section_even_with_a_readme_on_disk() {
    let _serial = serially();
    let root = scratch("plain-folder-no-section");
    plain_folder(&root, "notes", Some(README_BODY));
    let (ui, app) = window_at(&root);

    select_and_wait(&ui, &app, "notes", "Entries: 2");

    assert_eq!(
        ui.get_file_readme_name().to_string(),
        String::new(),
        "a folder that is not a working copy should show no README section"
    );
    assert!(
        !pane_text(&ui).contains("Kestrel"),
        "no title should show for a plain folder's README: {}",
        pane_text(&ui)
    );
}

#[test]
fn opening_the_readme_selects_it_in_contents() {
    let _serial = serially();
    let root = scratch("open-readme-selects-it");
    checkout(&root, "alpha", Some(README_BODY));
    let (ui, app) = window_at(&root);

    select_and_wait(&ui, &app, "alpha", "Kestrel");
    assert_ne!(
        listing(&ui)[usize::try_from(ui.get_content_selected()).unwrap()],
        "README.md",
        "the checkout itself, not its README, should be selected to start with"
    );

    click_element(&ui, &the_open_readme_link(&ui));
    settle(&ui, &app);

    assert_eq!(
        app.borrow().breadcrumbs().last().map(String::as_str),
        Some("alpha"),
        "Open README should open the checkout it belongs to"
    );
    let selected = listing(&ui)[usize::try_from(ui.get_content_selected()).unwrap()].clone();
    assert_eq!(
        selected, "README.md",
        "Open README should select the file in Contents"
    );
}
