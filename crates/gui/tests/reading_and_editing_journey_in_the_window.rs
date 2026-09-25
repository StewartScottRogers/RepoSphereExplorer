//! The flagship journey (#725): open at the Repos Directory, find a
//! repository by typing towards it, drill in, read a file, edit it and see
//! both the listing and the File pane catch up, then walk back out to
//! where the reader started.
//!
//! The graphical front end has forty-odd test files and, before this one,
//! not a single journey through it: every other suite is one interaction or
//! one hand-off from a fresh scratch directory and a fresh window. That
//! leaves the seams between features uncovered, which CLAUDE.md rule 14
//! says is where the faults live - and the seam this file exists for is
//! the one `file_pane_in_the_window.rs`'s
//! `saving_leaves_the_reader_on_the_file_they_saved` never checked: an
//! editor save reaching the Contents row's own size and modified columns,
//! and the File pane's preview re-reading what was just written, not only
//! the selection surviving.
//!
//! Full stack throughout: a real Repos Directory holding real `git`
//! checkouts, a real service on the private socket `common::ensure_service`
//! provides, and a real `MainWindow` joined to a real `App` by
//! `gui::wire_callbacks` - the function `main` calls, never a copy of it.
//! Every step is a dispatched pointer or keyboard event or a markup
//! callback, the way a reader reaches the application; nothing is measured
//! that was not drawn.
//!
//! Per rule 8 this drives `git` only inside directories it created under
//! the platform's temporary directory - [`git`] refuses anything else -
//! and only to *build* fixtures.

use gui::app::App;
use gui::{MainWindow, sync_ui};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{Key, PointerEventButton, WindowEvent};
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

/// The tab strip's height in `app.slint`, for aiming at the middle of a
/// strip whose position and width are measured, never assumed.
const TAB_HEIGHT: f32 = 24.0;

/// The Slint testing backend is a process-wide platform, and real `git`
/// checkouts are not something to build forty at once, so this suite's one
/// test takes the same guard `operations_in_the_window.rs` does.
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
    std::env::temp_dir().join("rse-reading-and-editing-journey")
}

/// An empty directory of this test's own. Nothing in this file ever
/// touches a path outside [`scratch_root`].
fn scratch(name: &str) -> PathBuf {
    let dir = scratch_root().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// Runs `git` in `dir`, which must be inside [`scratch_root`] - rule 8: this
/// application never runs a source control command on somebody's working
/// copy, and a test that builds fixtures with `git` is the one place a
/// stray path would do it.
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

/// Initialises a checkout at `dir` on `branch`, with an identity of its own
/// so a commit does not depend on the developer's global settings.
fn init_checkout(dir: &Path, branch: &str) {
    std::fs::create_dir_all(dir).expect("a scratch directory");
    git(dir, &["init", "--quiet", "--initial-branch", branch, "."]);
    git(dir, &["config", "user.name", "Repos Explorer Test"]);
    git(dir, &["config", "user.email", "test@example.invalid"]);
    git(dir, &["config", "core.autocrlf", "false"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

/// Commits everything in `dir`.
fn commit_everything(dir: &Path) {
    git(dir, &["add", "--all"]);
    git(dir, &["commit", "--quiet", "--message", "a commit"]);
}

/// Writes a file into `dir`.
fn write_file(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).expect("a scratch file");
}

/// A committed checkout at `root/name`, on `branch`, tracking `remote` when
/// one is given, holding each of `files` as a tracked file.
fn checkout(
    root: &Path,
    name: &str,
    branch: &str,
    remote: &str,
    files: &[(&str, &str)],
) -> PathBuf {
    let dir = root.join(name);
    init_checkout(&dir, branch);
    git(&dir, &["remote", "add", "origin", remote]);
    for (name, body) in files {
        write_file(&dir, name, body);
    }
    commit_everything(&dir);
    dir
}

/// Sets `path`'s modification time to `days_ago` days before now, so a
/// save's fresh timestamp is guaranteed to fall on a different day - and so
/// read as a different Modified column - rather than depending on the
/// column's minute-level formatting happening to roll over mid-test.
fn set_mtime_days_ago(path: &Path, days_ago: u64) {
    let at = std::time::SystemTime::now() - Duration::from_secs(days_ago * 24 * 60 * 60);
    std::fs::File::options()
        .write(true)
        .open(path)
        .expect("the fixture file opens for its mtime to be set")
        .set_modified(at)
        .expect("the platform can set a file's modified time");
}

// ---------------------------------------------------------------------
// The harness, in the shape `repository_in_the_window.rs` uses.
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
    status.starts_with("loading ")
        || matches!(
            status,
            "working..." | "deleting..." | "undoing..." | "saving..."
        )
}

/// Ticks the application the way the window's 100ms timer does, until every
/// in-flight request has landed and stayed landed. Not a sleep: the same
/// `tick` and `sync_ui` pair `main` runs, just as fast as the results
/// arrive.
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

/// Clicks the contents pane `rows_down` rows below its first row.
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

/// The names the contents pane is drawing, in the order it draws them. A
/// directory carries a trailing separator, which is stripped here so a test
/// can name a folder the way the filesystem does.
fn listing(ui: &MainWindow) -> Vec<String> {
    ui.get_content_rows()
        .iter()
        .map(|row| row.name.trim_end_matches('/').to_owned())
        .collect()
}

/// The names the contents pane is drawing as selected.
fn selected(ui: &MainWindow) -> Vec<String> {
    ui.get_content_rows()
        .iter()
        .filter(|row| row.selected)
        .map(|row| row.name.trim_end_matches('/').to_owned())
        .collect()
}

/// The row index of `name` in the drawn listing.
fn row_of(ui: &MainWindow, name: &str) -> f32 {
    let index = listing(ui)
        .iter()
        .position(|drawn| drawn == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing: {:?}", listing(ui)));
    let index = u16::try_from(index).expect("a small listing");
    f32::from(index)
}

/// The size and modified columns the contents pane draws for the row named
/// `name`, so a save's effect on them can be measured before and after
/// rather than assumed.
fn size_and_modified(ui: &MainWindow, name: &str) -> (String, String) {
    let row = ui
        .get_content_rows()
        .iter()
        .find(|row| row.name.trim_end_matches('/') == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing: {:?}", listing(ui)));
    (row.size.to_string(), row.modified.to_string())
}

/// The branch the contents pane draws beside the row named `name`.
fn branch_of(ui: &MainWindow, name: &str) -> String {
    ui.get_content_rows()
        .iter()
        .find(|row| row.name.trim_end_matches('/') == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing: {:?}", listing(ui)))
        .branch
        .to_string()
}

/// Whether the Contents pane is drawing the row named `name` as a working
/// copy.
fn drawn_as_a_checkout(ui: &MainWindow, name: &str) -> bool {
    ui.get_content_rows()
        .iter()
        .find(|row| row.name.trim_end_matches('/') == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing: {:?}", listing(ui)))
        .is_repository
}

/// What the File pane has on screen, whichever of its two text surfaces is
/// drawing it: the coloured one when the plugin described a language, and
/// the plain one otherwise. A reader cannot tell them apart, so neither
/// does this test.
fn shown(ui: &MainWindow) -> String {
    let coloured = ui
        .get_file_lines()
        .iter()
        .map(|line| {
            line.iter()
                .map(|run| run.text.to_string())
                .collect::<Vec<_>>()
                .concat()
        })
        .collect::<Vec<_>>()
        .join("\n");
    if coloured.is_empty() {
        ui.get_file_text().to_string()
    } else {
        coloured
    }
}

/// The tab labels the File pane is drawing.
fn tabs(ui: &MainWindow) -> Vec<String> {
    ui.get_file_tabs()
        .iter()
        .map(|label| label.to_string())
        .collect()
}

/// The tab strip, measured rather than assumed - the tabs share whatever
/// width the File pane has, and a constant here would let a test click a
/// tab that is off the edge of the pane and pass while the reader could not
/// reach it.
fn strip(ui: &MainWindow) -> ElementHandle {
    ElementHandle::find_by_element_id(ui, "FilePane::tab-strip")
        .next()
        .expect("a strip should be drawn")
}

/// Clicks the middle of tab `index`, as it is actually drawn.
fn click_tab(ui: &MainWindow, index: usize) {
    let strip = strip(ui);
    let count = u16::try_from(tabs(ui).len()).expect("a handful of tabs");
    let width = strip.size().width / f32::from(count);
    let index = u16::try_from(index).expect("a handful of tabs");
    let origin = strip.absolute_position();
    let position = LogicalPosition::new(
        origin.x + f32::from(index).mul_add(width, width / 2.0),
        origin.y + TAB_HEIGHT / 2.0,
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

// ---------------------------------------------------------------------
// The journey, one step per function so the story reads in the test below
// without tripping `clippy::too_many_lines` - each step is still real
// dispatched events against the one shared window, not a unit test of a
// method.
// ---------------------------------------------------------------------

/// A Repos Directory holding two real checkouts: alpha, tracking a text
/// file whose mtime is backdated so a save below has to move its Modified
/// column across a day boundary rather than depend on minute-level
/// formatting happening to roll over mid-test; and beta, alongside it.
fn build_repos_directory() -> PathBuf {
    let root = scratch("journey");
    checkout(
        &root,
        "alpha",
        "main",
        "https://github.com/owner/alpha.git",
        &[("notes.txt", "first line\n")],
    );
    checkout(
        &root,
        "beta",
        "develop",
        "https://gitlab.com/owner/beta.git",
        &[("readme.txt", "hello\n")],
    );
    set_mtime_days_ago(&root.join("alpha").join("notes.txt"), 2);
    root
}

/// 1. The listing draws, anchored at the Repos Directory, with both
///    checkouts marked as repositories and their branches shown.
fn assert_the_listing_is_anchored_with_both_checkouts_marked(ui: &MainWindow) {
    assert_eq!(
        listing(ui),
        vec!["alpha".to_owned(), "beta".to_owned()],
        "the listing should be anchored at the Repos Directory itself"
    );
    assert!(
        drawn_as_a_checkout(ui, "alpha") && drawn_as_a_checkout(ui, "beta"),
        "both checkouts should be marked as working copies"
    );
    assert_eq!(branch_of(ui, "alpha"), "main");
    assert_eq!(branch_of(ui, "beta"), "develop");
}

/// 2. Find alpha by typing towards it, rather than by index: land on beta
///    first, so the jump actually has somewhere to go from.
fn find_alpha_by_typing_towards_it(ui: &MainWindow) {
    click_row(ui, row_of(ui, "beta"));
    assert_eq!(selected(ui), vec!["beta".to_owned()]);
    press(ui, "a");
    assert_eq!(
        selected(ui),
        vec!["alpha".to_owned()],
        "typing 'a' should have jumped the listing to alpha"
    );
}

/// 3. Drill into it, and the breadcrumb says where the reader is.
fn drill_into_alpha_and_check_the_breadcrumb(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    press_key(ui, Key::Return);
    settle(ui, app);
    assert!(
        listing(ui).contains(&"notes.txt".to_owned()),
        "Return should have opened alpha's own listing: {:?}",
        listing(ui)
    );
    assert_eq!(
        ui.get_breadcrumbs()
            .iter()
            .last()
            .map(|crumb| crumb.to_string()),
        Some("alpha".to_owned()),
        "the breadcrumb should end on the folder the reader drilled into"
    );
}

/// 4. Select the text file; the File pane shows the plugin's view of it,
///    and the status line says what it is. Returns the row's size and
///    modified columns as drawn now, so a save's effect on them can be
///    measured rather than assumed.
fn select_notes_and_check_the_preview(ui: &MainWindow, app: &Rc<RefCell<App>>) -> (String, String) {
    click_row(ui, row_of(ui, "notes.txt"));
    settle(ui, app);
    assert_eq!(
        shown(ui),
        "first line",
        "the File pane should be showing the plugin's view of notes.txt"
    );
    assert!(
        ui.get_status_text().contains("notes.txt"),
        "the status line should say what is selected; it reads {:?}",
        ui.get_status_text()
    );
    size_and_modified(ui, "notes.txt")
}

/// 5. Open the editor, type at the caret, and save.
fn type_and_save_an_edit(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    click_tab(ui, tabs(ui).len() - 1);
    assert!(ui.get_editing_file(), "the last tab should open the editor");
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: "X".into() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text: "X".into() });
    assert_eq!(
        app.borrow().edit_text(),
        "Xfirst line\n",
        "the keystroke should have reached the editor without a click first, \
         the way opening the editor from a tab click has to (CLAUDE.md rule 14)"
    );
    ui.invoke_save_requested();
    settle(ui, app);
}

/// 6. The listing's row for that file shows the new size and a newer
///    modified time - the seam this order exists for, not only that the
///    selection survived the save. 7. The File pane's preview re-reads the
///    file, and shows what was typed. 8. The bytes on disk hold what was
///    typed.
fn assert_the_row_and_preview_caught_up(
    ui: &MainWindow,
    root: &Path,
    size_before: &str,
    modified_before: &str,
) {
    let (size_after, modified_after) = size_and_modified(ui, "notes.txt");
    assert_ne!(
        size_after, size_before,
        "the Contents row's Size column should reflect the saved file"
    );
    assert_ne!(
        modified_after, modified_before,
        "the Contents row's Modified column should reflect the saved file"
    );
    assert_eq!(
        selected(ui),
        vec!["notes.txt".to_owned()],
        "saving should not move the reader off the file they saved"
    );
    assert!(!ui.get_editing_file(), "saving should close the editor");
    assert_eq!(
        shown(ui),
        "Xfirst line",
        "the File pane's preview should re-read the file rather than keep \
         showing what was open in the editor"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("alpha").join("notes.txt"))
            .expect("notes.txt is readable"),
        "Xfirst line\n",
        "the edit should have reached the file"
    );
}

/// 9. Walk back out to the Repos Directory, and the reader is where they
///    started.
fn walk_back_out_to_the_repos_directory(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    press_with(ui, &char::from(Key::UpArrow).to_string(), &[Key::Alt]);
    settle(ui, app);
    assert_eq!(
        listing(ui),
        vec!["alpha".to_owned(), "beta".to_owned()],
        "Alt+Up should have walked back out to the Repos Directory"
    );
}

/// Launch at a Repos Directory holding two real checkouts, then, in one
/// test: find one by typing towards it, drill into it, read a text file,
/// edit and save it, see the listing's own row and the File pane's preview
/// catch up, and walk back out to where the reader started.
#[test]
fn a_reader_opens_at_the_repos_directory_reads_edits_and_walks_back_out() {
    let _serial = serially();
    let root = build_repos_directory();
    let (ui, app) = window_at(&root);

    assert_the_listing_is_anchored_with_both_checkouts_marked(&ui);
    find_alpha_by_typing_towards_it(&ui);
    drill_into_alpha_and_check_the_breadcrumb(&ui, &app);
    let (size_before, modified_before) = select_notes_and_check_the_preview(&ui, &app);
    type_and_save_an_edit(&ui, &app);
    assert_the_row_and_preview_caught_up(&ui, &root, &size_before, &modified_before);
    walk_back_out_to_the_repos_directory(&ui, &app);
}
