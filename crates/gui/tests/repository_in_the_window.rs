//! Repository awareness, driven end to end through a real window on a real
//! [`App`], against real checkouts made by real `git` in a scratch
//! directory.
//!
//! This is what the application is *for* (GUIDANCE.md 2.4 and 2.5): the
//! Contents pane marks a child of the Repos Directory that is a working
//! copy and names its provider, a folder that is not stays visible and
//! looks different, and the File pane reports the provider, the branch
//! checked out, the remote it tracks and what its tracked files look like.
//!
//! Every half of that is unit-tested alone. `plugin-directory` tests the
//! presentation against hand-written view data; `app`'s own tests test the
//! Type column against hand-written entries. Neither half can see what a
//! reader meets: a checkout on disk, selected with a click, described in
//! the pane the window is drawing. That seam is what this file measures,
//! per rule 14.
//!
//! Per rule 8 this drives `git` only inside directories it created under
//! the platform's temporary directory - [`git`] refuses anything else -
//! and only to *build* fixtures. Nothing here asks the application to run
//! a source control command, because it does not have one to run.

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

/// One window and one service at a time: Slint's testing backend is a
/// process-wide platform, and forty `git` processes at once is nobody's
/// idea of a fixture.
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
    std::env::temp_dir().join("rse-repository-in-the-window")
}

/// An empty directory of this test's own. Nothing in this file ever
/// touches a path outside [`scratch_root`].
fn scratch(name: &str) -> PathBuf {
    let dir = scratch_root().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// Runs `git` in `dir`, which must be inside [`scratch_root`].
///
/// The check is the point rather than a formality: rule 8 says this
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

/// Initialises a checkout at `dir` on `branch`, with an identity of its
/// own so a commit does not depend on the developer's global settings.
fn init_checkout(dir: &Path, branch: &str) {
    std::fs::create_dir_all(dir).expect("a scratch directory");
    git(dir, &["init", "--quiet", "--initial-branch", branch, "."]);
    git(dir, &["config", "user.name", "Repos Explorer Test"]);
    git(dir, &["config", "user.email", "test@example.invalid"]);
    // Line endings the test wrote are the line endings the index records,
    // so a freshly committed tree really is unchanged.
    git(dir, &["config", "core.autocrlf", "false"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

/// Commits everything in `dir`.
fn commit_everything(dir: &Path) {
    git(dir, &["add", "--all"]);
    git(dir, &["commit", "--quiet", "--message", "a commit"]);
}

/// Writes a file into `dir`.
fn file(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).expect("a scratch file");
}

/// A committed checkout at `root/name`, on `branch`, tracking `remote`
/// when one is given, holding `files` plain files.
fn checkout(root: &Path, name: &str, branch: &str, remote: Option<&str>, files: usize) -> PathBuf {
    let dir = root.join(name);
    init_checkout(&dir, branch);
    if let Some(remote) = remote {
        git(&dir, &["remote", "add", "origin", remote]);
    }
    for index in 0..files {
        file(
            &dir,
            &format!("tracked-{index}.txt"),
            "the committed body\n",
        );
    }
    commit_everything(&dir);
    dir
}

/// A plain folder at `root/name` holding `files` plain files - not a
/// checkout, and entitled to stay in the listing anyway (GUIDANCE.md 2.5).
fn plain_folder(root: &Path, name: &str, files: usize) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    for index in 0..files {
        file(&dir, &format!("note-{index}.txt"), "just a note\n");
    }
    dir
}

// ---------------------------------------------------------------------
// The harness, in the shape `operations_in_the_window.rs` uses.
// ---------------------------------------------------------------------

/// A shown window on a real `App` rooted at `root`, wired by the crate's
/// own wiring - the same call `main` makes - with the opening listing and
/// the first preview already loaded.
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

/// Whether `status` is one of the transient lines an in-flight request
/// puts up, which is how [`settle`] knows the application is still
/// working.
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

/// Ticks until the File pane is drawing `marker`, so an assertion about
/// what the pane says is made against the preview that actually arrived
/// rather than against whatever the previous selection left behind.
///
/// `marker` is chosen per fixture to be something only the newly selected
/// folder's own preview can contain - an entry count no other fixture
/// shares - so waiting for it cannot paper over the staleness these tests
/// are looking for.
fn settle_until_pane_says(ui: &MainWindow, app: &Rc<RefCell<App>>, marker: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        {
            let mut app = app.borrow_mut();
            app.tick();
            sync_ui(ui, &app);
        }
        if ui.get_file_text().contains(marker) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "the File pane never showed {marker:?}; it reads:\n{}",
        ui.get_file_text()
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
/// directory carries a trailing separator, which is stripped here so a
/// test can name a folder the way the filesystem does.
fn listing(ui: &MainWindow) -> Vec<String> {
    ui.get_content_rows()
        .iter()
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

/// Where the row named `name` sits in the listing, counting from 0.
fn index_of(ui: &MainWindow, name: &str) -> usize {
    listing(ui)
        .iter()
        .position(|drawn| drawn == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing: {:?}", listing(ui)))
}

/// What the Contents pane's Type column says for the row named `name`.
fn kind_of(ui: &MainWindow, name: &str) -> String {
    ui.get_content_rows()
        .iter()
        .find(|row| row.name.trim_end_matches('/') == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing: {:?}", listing(ui)))
        .kind
        .to_string()
}

/// Whether the Contents pane is drawing the row named `name` as a working
/// copy - which is what puts its name in the accent colour and in bold.
fn drawn_as_a_checkout(ui: &MainWindow, name: &str) -> bool {
    ui.get_content_rows()
        .iter()
        .find(|row| row.name.trim_end_matches('/') == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing: {:?}", listing(ui)))
        .is_repository
}

/// Selects the folder named `name` in the contents pane and waits for its
/// own preview, recognised by `marker`.
fn select_and_wait(ui: &MainWindow, app: &Rc<RefCell<App>>, name: &str, marker: &str) {
    click_row(ui, row_of(ui, name));
    settle_until_pane_says(ui, app, marker);
}

// ---------------------------------------------------------------------
// The listing: which folders are checkouts, and what the others show.
// ---------------------------------------------------------------------

#[test]
fn a_checkout_is_marked_in_the_listing_and_a_plain_folder_is_not() {
    let _serial = serially();
    let root = scratch("listing");
    checkout(
        &root,
        "alpha",
        "main",
        Some("https://github.com/owner/alpha.git"),
        1,
    );
    plain_folder(&root, "notes", 3);
    let (ui, _app) = window_at(&root);

    assert!(
        listing(&ui).contains(&"notes".to_owned()),
        "a folder that is not a checkout stays visible: {:?}",
        listing(&ui)
    );
    assert!(
        drawn_as_a_checkout(&ui, "alpha"),
        "a folder with a .git marker should be drawn as a working copy"
    );
    assert!(
        !drawn_as_a_checkout(&ui, "notes"),
        "a folder with no .git marker should not be drawn as a working copy"
    );
    assert_eq!(
        kind_of(&ui, "alpha"),
        "github.com",
        "the Type column names the provider a checkout came from"
    );
    assert_eq!(
        kind_of(&ui, "notes"),
        "File folder",
        "a plain folder is still a plain folder"
    );
}

// ---------------------------------------------------------------------
// The File pane's working-copy panel.
// ---------------------------------------------------------------------

#[test]
fn the_file_pane_reports_the_provider_branch_and_remote_of_the_selected_checkout() {
    let _serial = serially();
    let root = scratch("panel");
    checkout(
        &root,
        "alpha",
        "main",
        Some("https://github.com/owner/alpha.git"),
        1,
    );
    let (ui, app) = window_at(&root);

    // ".git" plus one tracked file: an entry count no other fixture here
    // shares, so waiting for it proves alpha's own preview arrived.
    select_and_wait(&ui, &app, "alpha", "2 entries");
    let pane = ui.get_file_text().to_string();

    assert!(
        pane.contains("Source control working copy"),
        "the pane should lead with what the folder is; it reads:\n{pane}"
    );
    assert!(
        pane.contains("Provider: github.com"),
        "the pane should name the provider (GUIDANCE.md 2.4); it reads:\n{pane}"
    );
    assert!(
        pane.contains("Branch: main"),
        "the pane should name the branch checked out; it reads:\n{pane}"
    );
    assert!(
        pane.contains("Remote: https://github.com/owner/alpha.git"),
        "the pane should name the remote it tracks; it reads:\n{pane}"
    );
    assert!(
        pane.contains("Working tree: no uncommitted changes to tracked files"),
        "a checkout committed a moment ago has no uncommitted changes; it \
         reads:\n{pane}"
    );
}

#[test]
fn a_checkout_with_a_changed_tracked_file_says_so_in_the_file_pane() {
    let _serial = serially();
    let root = scratch("dirty");
    let alpha = checkout(
        &root,
        "alpha",
        "main",
        Some("https://github.com/owner/alpha.git"),
        1,
    );
    // Committed, then edited: a tracked file that no longer matches what
    // was staged, which is exactly what the panel is for.
    file(
        &alpha,
        "tracked-0.txt",
        "the body after somebody edited it, which is a different length\n",
    );
    let (ui, app) = window_at(&root);

    select_and_wait(&ui, &app, "alpha", "2 entries");
    let pane = ui.get_file_text().to_string();

    assert!(
        pane.contains("Working tree: 1 tracked file changed"),
        "the pane should say the working tree has an uncommitted change; it \
         reads:\n{pane}"
    );
}

#[test]
fn a_checkout_with_no_remote_says_so_in_both_panes() {
    let _serial = serially();
    let root = scratch("no-remote");
    checkout(&root, "alpha", "main", None, 1);
    let (ui, app) = window_at(&root);

    assert_eq!(
        kind_of(&ui, "alpha"),
        "Repository",
        "a checkout with no remote has no provider to name, and says what it is"
    );

    select_and_wait(&ui, &app, "alpha", "2 entries");
    let pane = ui.get_file_text().to_string();

    assert!(
        pane.contains("Remote: none configured"),
        "the pane should say there is no remote rather than going quiet; it \
         reads:\n{pane}"
    );
    assert!(
        !pane.contains("Provider:"),
        "there is no provider to name without a remote; it reads:\n{pane}"
    );
}

#[test]
fn a_detached_head_says_so_rather_than_naming_a_branch() {
    let _serial = serially();
    let root = scratch("detached");
    let alpha = checkout(
        &root,
        "alpha",
        "main",
        Some("https://github.com/owner/alpha.git"),
        1,
    );
    git(&alpha, &["checkout", "--quiet", "--detach", "HEAD"]);
    let (ui, app) = window_at(&root);

    select_and_wait(&ui, &app, "alpha", "2 entries");
    let pane = ui.get_file_text().to_string();

    assert!(
        pane.contains("Source control working copy"),
        "a detached head is still a working copy; it reads:\n{pane}"
    );
    assert!(
        pane.contains("Branch: none checked out (detached head)"),
        "the pane should say no branch is out rather than naming a commit; it \
         reads:\n{pane}"
    );
}

#[test]
fn a_checkout_with_no_commits_is_still_a_checkout() {
    let _serial = serially();
    let root = scratch("empty-checkout");
    let alpha = root.join("alpha");
    init_checkout(&alpha, "main");
    git(
        &alpha,
        &["remote", "add", "origin", "git@github.com:owner/alpha.git"],
    );
    let (ui, app) = window_at(&root);

    assert!(
        drawn_as_a_checkout(&ui, "alpha"),
        "`git init` with nothing committed is still a .git marker"
    );
    assert_eq!(
        kind_of(&ui, "alpha"),
        "github.com",
        "the provider comes from the remote, not from having commits"
    );

    // Only ".git" is in it.
    select_and_wait(&ui, &app, "alpha", "1 entry");
    let pane = ui.get_file_text().to_string();

    assert!(
        pane.contains("Source control working copy"),
        "a fresh checkout should say what it is; it reads:\n{pane}"
    );
    assert!(
        pane.contains("Branch: main"),
        "HEAD names the branch that will be committed to; it reads:\n{pane}"
    );
}

// ---------------------------------------------------------------------
// A .git marker that is a file: a linked worktree.
// ---------------------------------------------------------------------

/// A clone at `root/clone` tracking `remote`, plus a linked worktree of it
/// at `root/linked` on branch `side`. The worktree's `.git` is a *file*
/// pointing at `clone/.git/worktrees/linked`.
///
/// The worktree is given an untracked file of its own so that its preview
/// counts three entries where the clone's counts two. Both folders hold
/// the same committed file, and without that the two previews would be
/// indistinguishable - which is exactly the shape that lets a test read a
/// stale pane and call it a pass.
fn clone_with_linked_worktree(root: &Path, remote: &str) {
    let clone = checkout(root, "clone", "main", Some(remote), 1);
    git(
        &clone,
        &["worktree", "add", "--quiet", "-b", "side", "../linked"],
    );
    // Untracked, so it changes nothing the working-tree line counts.
    file(&root.join("linked"), "scratch-note.txt", "not committed\n");
}

#[test]
fn a_worktree_whose_git_marker_is_a_file_is_detected_as_a_checkout() {
    let _serial = serially();
    let root = scratch("worktree-detect");
    clone_with_linked_worktree(root.as_path(), "https://github.com/owner/clone.git");
    let (ui, app) = window_at(&root);

    assert!(
        drawn_as_a_checkout(&ui, "linked"),
        "a worktree's .git is a file rather than a directory, and it is still \
         a working copy: {:?}",
        listing(&ui)
    );

    // ".git" file, the tracked file, and the untracked note: three, where
    // the clone beside it has two.
    select_and_wait(&ui, &app, "linked", "3 entries");
    let pane = ui.get_file_text().to_string();

    assert!(
        pane.contains("Branch: side"),
        "a worktree has its own HEAD and its own branch; it reads:\n{pane}"
    );
}

#[test]
fn a_worktree_reports_the_remote_its_clone_tracks() {
    let _serial = serially();
    let root = scratch("worktree-remote");
    clone_with_linked_worktree(root.as_path(), "https://github.com/owner/clone.git");
    let (ui, app) = window_at(&root);

    select_and_wait(&ui, &app, "linked", "3 entries");
    let pane = ui.get_file_text().to_string();

    assert!(
        pane.contains("Remote: https://github.com/owner/clone.git"),
        "a worktree of a GitHub clone tracks the clone's remote - the File \
         pane is reading `config` from the worktree's own git directory, \
         which never has one, instead of from the common directory beside \
         it; it reads:\n{pane}"
    );
    assert!(
        pane.contains("Provider: github.com"),
        "and with no remote found there is no provider to name either; it \
         reads:\n{pane}"
    );
}

#[test]
fn the_type_column_names_the_provider_of_a_worktree() {
    let _serial = serially();
    let root = scratch("worktree-kind");
    clone_with_linked_worktree(root.as_path(), "https://github.com/owner/clone.git");
    let (ui, _app) = window_at(&root);

    assert_eq!(
        kind_of(&ui, "linked"),
        "github.com",
        "the clone beside it reads {:?}: two checkouts of the same GitHub \
         repository, and only one of them names the provider",
        kind_of(&ui, "clone")
    );
}

// ---------------------------------------------------------------------
// Moving between folders: whose branch is the pane reporting?
// ---------------------------------------------------------------------

#[test]
fn moving_from_one_checkout_to_another_reports_the_second_ones_branch() {
    let _serial = serially();
    let root = scratch("two-checkouts");
    checkout(
        &root,
        "alpha",
        "main",
        Some("https://github.com/owner/alpha.git"),
        1,
    );
    checkout(
        &root,
        "beta",
        "release",
        Some("https://gitlab.com/owner/beta.git"),
        3,
    );
    let (ui, app) = window_at(&root);

    // ".git" plus one, and ".git" plus three: each fixture's preview is
    // recognisable without looking at the fields under test.
    select_and_wait(&ui, &app, "alpha", "2 entries");
    assert!(
        ui.get_file_text().contains("Branch: main"),
        "alpha is on main; the pane reads:\n{}",
        ui.get_file_text()
    );

    select_and_wait(&ui, &app, "beta", "4 entries");
    let pane = ui.get_file_text().to_string();

    assert!(
        pane.contains("Branch: release"),
        "the pane should report beta's branch, not the one alpha left \
         behind; it reads:\n{pane}"
    );
    assert!(
        pane.contains("Provider: gitlab.com"),
        "and beta's provider; it reads:\n{pane}"
    );
    assert!(
        !pane.contains("alpha"),
        "nothing of alpha's should still be on screen; it reads:\n{pane}"
    );
}

#[test]
fn leaving_a_checkout_for_a_plain_folder_stops_reporting_source_control() {
    let _serial = serially();
    let root = scratch("checkout-then-folder");
    checkout(
        &root,
        "alpha",
        "main",
        Some("https://github.com/owner/alpha.git"),
        1,
    );
    plain_folder(&root, "notes", 3);
    let (ui, app) = window_at(&root);

    select_and_wait(&ui, &app, "alpha", "2 entries");
    assert!(
        ui.get_file_text().contains("Source control working copy"),
        "the checkout was described first"
    );

    select_and_wait(&ui, &app, "notes", "3 entries");
    let pane = ui.get_file_text().to_string();

    assert!(
        !pane.contains("Source control working copy"),
        "a plain folder is not a working copy, and the panel from the folder \
         before it must not still be on screen; it reads:\n{pane}"
    );
    assert!(
        !pane.contains("Branch:"),
        "a plain folder has no branch; it reads:\n{pane}"
    );
}

#[test]
fn coming_back_from_inside_a_checkout_still_reports_that_checkouts_branch() {
    let _serial = serially();
    let root = scratch("in-and-out");
    checkout(
        &root,
        "alpha",
        "main",
        Some("https://github.com/owner/alpha.git"),
        1,
    );
    checkout(
        &root,
        "beta",
        "release",
        Some("https://gitlab.com/owner/beta.git"),
        3,
    );
    let (ui, app) = window_at(&root);

    select_and_wait(&ui, &app, "alpha", "2 entries");

    // Into alpha, and back out again the way the toolbar's Up does.
    click_row(&ui, row_of(&ui, "alpha"));
    press_key(&ui, Key::Return);
    settle(&ui, &app);
    assert!(
        listing(&ui).contains(&"tracked-0.txt".to_owned()),
        "Return opened the checkout: {:?}",
        listing(&ui)
    );

    press_with(&ui, &char::from(Key::UpArrow).to_string(), &[Key::Alt]);
    settle(&ui, &app);
    assert!(
        listing(&ui).contains(&"beta".to_owned()),
        "Alt+Up came back out to the Repos Directory: {:?}",
        listing(&ui)
    );

    select_and_wait(&ui, &app, "beta", "4 entries");
    let pane = ui.get_file_text().to_string();

    assert!(
        pane.contains("Branch: release"),
        "after a trip into alpha and back, selecting beta reports beta; it \
         reads:\n{pane}"
    );

    select_and_wait(&ui, &app, "alpha", "2 entries");
    let pane = ui.get_file_text().to_string();

    assert!(
        pane.contains("Branch: main"),
        "and alpha still reports its own branch; it reads:\n{pane}"
    );
}

// ---------------------------------------------------------------------
// A folder is several things at once (rule 9, D12).
// ---------------------------------------------------------------------

#[test]
fn a_checkout_that_is_also_a_cargo_project_reports_both() {
    let _serial = serially();
    let root = scratch("checkout-and-project");
    let alpha = root.join("alpha");
    init_checkout(&alpha, "main");
    git(
        &alpha,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/owner/alpha.git",
        ],
    );
    file(
        &alpha,
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    commit_everything(&alpha);
    let (ui, app) = window_at(&root);

    select_and_wait(&ui, &app, "alpha", "2 entries");
    let pane = ui.get_file_text().to_string();

    assert!(
        pane.contains("Source control working copy"),
        "the folder's own description comes first; it reads:\n{pane}"
    );
    assert!(
        pane.contains("Package: demo 0.1.0"),
        "a folder plugin's lines are added below, never in place of, what the \
         folder already reports (D12); it reads:\n{pane}"
    );
}

// ---------------------------------------------------------------------
// Branch and uncommitted changes on every repository row (#535).
// ---------------------------------------------------------------------

/// What the Contents pane draws after the row named `name`: its branch
/// and the marker beside it.
fn branch_and_marker(ui: &MainWindow, name: &str) -> (String, String) {
    let row = ui
        .get_content_rows()
        .iter()
        .find(|row| row.name.trim_end_matches('/') == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing: {:?}", listing(ui)));
    (row.branch.to_string(), row.marker.to_string())
}

/// Ticks the way `main`'s timer does - apply what arrived, draw, ask for
/// the statuses of the rows on screen - until no row still says its status
/// is not known.
fn settle_statuses(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        {
            let mut app = app.borrow_mut();
            app.tick();
            sync_ui(ui, &app);
            gui::ask_for_visible_statuses(ui, &mut app);
        }
        if ui
            .get_content_rows()
            .iter()
            .all(|row| row.marker != gui::app::NOT_KNOWN_YET_MARKER)
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "the statuses never arrived; the rows read {:?}",
        ui.get_content_rows()
            .iter()
            .map(|row| format!("{} {} {}", row.name, row.branch, row.marker))
            .collect::<Vec<_>>()
    );
}

#[test]
fn every_checkout_row_draws_its_branch_before_any_status_has_arrived() {
    let _serial = serially();
    let root = scratch("branches-first");
    checkout(&root, "alpha", "main", None, 1);
    checkout(&root, "beta", "develop", None, 1);
    plain_folder(&root, "notes", 1);
    // Opened and settled, but nothing has asked for a status yet: the
    // listing is on screen without waiting for one.
    let (ui, _app) = window_at(&root);

    assert_eq!(
        branch_and_marker(&ui, "alpha"),
        ("main".to_owned(), gui::app::NOT_KNOWN_YET_MARKER.to_owned()),
        "the branch is drawn at once, and the changes say they are not known yet"
    );
    assert_eq!(
        branch_and_marker(&ui, "beta"),
        (
            "develop".to_owned(),
            gui::app::NOT_KNOWN_YET_MARKER.to_owned()
        )
    );
    assert_eq!(
        branch_and_marker(&ui, "notes"),
        (String::new(), String::new()),
        "a plain folder has neither"
    );
}

#[test]
fn each_checkout_row_marks_uncommitted_changes_as_they_really_are() {
    let _serial = serially();
    let root = scratch("markers");
    checkout(&root, "alpha", "main", None, 2);
    let beta = checkout(&root, "beta", "main", None, 2);
    // Committed, then edited: a tracked file that differs from the index.
    file(
        &beta,
        "tracked-1.txt",
        "the body after somebody edited it, which is a different length\n",
    );
    let gamma = checkout(&root, "gamma", "main", None, 1);
    git(&gamma, &["checkout", "--quiet", "--detach"]);
    // Untracked files are not counted, and must not make a row look dirty.
    let delta = checkout(&root, "delta", "main", None, 1);
    file(&delta, "untracked.txt", "new\n");
    let (ui, app) = window_at(&root);

    settle_statuses(&ui, &app);

    assert_eq!(
        branch_and_marker(&ui, "alpha"),
        ("main".to_owned(), String::new()),
        "a checkout committed a moment ago has no marker"
    );
    assert_eq!(
        branch_and_marker(&ui, "beta"),
        ("main".to_owned(), gui::app::CHANGED_MARKER.to_owned()),
        "an edited tracked file marks its row"
    );
    assert_eq!(
        branch_and_marker(&ui, "gamma"),
        ("detached".to_owned(), String::new()),
        "a detached head says so"
    );
    assert_eq!(
        branch_and_marker(&ui, "delta"),
        ("main".to_owned(), String::new())
    );
    let status = ui.get_status_text().to_string();
    assert!(
        status.contains("4 repositories, 1 with uncommitted changes"),
        "the status bar sums the listing up; it reads: {status}"
    );
    assert!(!status.contains("not known"), "{status}");
}

// ---------------------------------------------------------------------
// How the selected checkout's branch stands against its upstream (#537).
// ---------------------------------------------------------------------

#[test]
fn a_checkout_behind_its_remote_says_how_far_in_the_file_pane() {
    let _serial = serially();
    let root = scratch("behind");
    let alpha = checkout(
        &root,
        "alpha",
        "main",
        Some("https://github.com/owner/alpha.git"),
        1,
    );
    // What a fetch would have left: `origin/main` two commits past the
    // local branch, and `main` set to track it. Built by `git` in the
    // scratch directory; nothing contacts a remote.
    git(&alpha, &["branch", "--quiet", "ahead-of-us"]);
    git(&alpha, &["switch", "--quiet", "ahead-of-us"]);
    for index in 0..2 {
        file(
            &alpha,
            "tracked-0.txt",
            &format!("upstream change {index}\n"),
        );
        commit_everything(&alpha);
    }
    git(
        &alpha,
        &["update-ref", "refs/remotes/origin/main", "ahead-of-us"],
    );
    git(&alpha, &["switch", "--quiet", "main"]);
    git(
        &alpha,
        &["branch", "--quiet", "--delete", "--force", "ahead-of-us"],
    );
    git(&alpha, &["config", "branch.main.remote", "origin"]);
    git(&alpha, &["config", "branch.main.merge", "refs/heads/main"]);
    let (ui, app) = window_at(&root);

    select_and_wait(&ui, &app, "alpha", "2 entries");
    let pane = ui.get_file_text().to_string();

    assert!(
        pane.contains("Branch: main - 2 behind origin/main (never fetched)"),
        "the pane should say how far behind its upstream the branch is; it \
         reads:\n{pane}"
    );
}

// ---------------------------------------------------------------------
// The name has priority over the branch beside it (#573), and the
// uncommitted-changes marker names itself for a mouse or screen reader
// user who does not know its glyph (#574).
// ---------------------------------------------------------------------

/// The `nth` element the pane draws with `element_id`, in the order the
/// per-row `for` loop in `app.slint` instantiates them - the same order
/// `ui.get_content_rows()` lists the rows in, since both come from one
/// loop over the same model.
fn nth_drawn(ui: &MainWindow, element_id: &str, index: usize) -> ElementHandle {
    ElementHandle::find_by_element_id(ui, element_id)
        .nth(index)
        .unwrap_or_else(|| panic!("no {element_id} at position {index}"))
}

#[test]
fn a_changed_repositorys_marker_names_itself_and_a_clean_ones_does_not() {
    let _serial = serially();
    let root = scratch("marker-words");
    checkout(&root, "alpha", "main", None, 1);
    let beta = checkout(&root, "beta", "main", None, 1);
    // Committed, then edited: a tracked file that differs from the index.
    file(
        &beta,
        "tracked-0.txt",
        "the body after somebody edited it\n",
    );
    let (ui, app) = window_at(&root);

    settle_statuses(&ui, &app);

    let alpha_marker = nth_drawn(&ui, "ContentsPane::marker-text", index_of(&ui, "alpha"));
    let beta_marker = nth_drawn(&ui, "ContentsPane::marker-text", index_of(&ui, "beta"));

    assert!(
        alpha_marker
            .accessible_label()
            .is_none_or(|label| label.is_empty()),
        "a clean checkout has no marker to explain, so its accessible label \
         should say nothing either"
    );
    assert_eq!(
        beta_marker
            .accessible_label()
            .map(|label| label.to_string()),
        Some("Uncommitted changes to tracked files".to_owned()),
        "an edited checkout's marker should name what it means, not just \
         show a glyph"
    );
}

#[test]
fn a_long_name_keeps_its_full_width_beside_a_long_branch_at_the_default_contents_width() {
    let _serial = serially();
    let root = scratch("name-priority-default-width");
    checkout(
        &root,
        "AgenticCliOptions",
        "chore/solution-drift-model-refresh-agenttools",
        None,
        1,
    );
    let (ui, _app) = window_at(&root);
    assert!(
        (ui.get_contents_width() - 470.0).abs() < f32::EPSILON,
        "the pane should still be at its default width"
    );

    let index = index_of(&ui, "AgenticCliOptions");
    let name = nth_drawn(&ui, "ContentsPane::name-text", index);

    // Crushed to a letter or two, the bug this reproduces, draws at
    // something close to the 24px floor `min-width` gives it; a name this
    // short shown in full is well over three times that.
    assert!(
        name.size().width > 80.0,
        "the name should keep its own width even with a long branch beside \
         it; it drew at {}px",
        name.size().width
    );
}

#[test]
fn a_narrower_contents_width_hides_the_branch_while_the_marker_stays_beside_the_name() {
    let _serial = serially();
    let root = scratch("name-priority-narrow-width");
    checkout(
        &root,
        "AgenticCliOptions",
        "chore/solution-drift-model-refresh-agenttools",
        None,
        1,
    );
    let (ui, _app) = window_at(&root);
    ui.set_contents_width(250.0);

    let index = index_of(&ui, "AgenticCliOptions");
    let marker = nth_drawn(&ui, "ContentsPane::marker-text", index);

    // A hidden element is left out of an element search altogether, so the
    // branch is not looked up by position: the only repository row here
    // must have no branch drawn with any width.
    let drawn: Vec<f32> = ElementHandle::find_by_element_id(&ui, "ContentsPane::branch-text")
        .map(|branch| branch.size().width)
        .filter(|width| *width >= 1.0)
        .collect();
    assert!(
        drawn.is_empty(),
        "a branch this long has no room left at a 250px contents width and \
         should disappear rather than sit there as a sliver of ellipsis; it \
         drew at {drawn:?}px"
    );
    assert!(
        marker.size().width > 0.0,
        "the uncommitted-changes marker stays attached to the name once the \
         branch beside it is gone"
    );
}
