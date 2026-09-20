//! Walks `tui::bindings::BINDINGS` against the real event loop, per
//! CLAUDE.md rule 14 and #638's first acceptance check: every entry's keys,
//! dispatched the way a real terminal's would be - through [`tui::tick`],
//! not a direct call to `App::handle_key` - must reach the action the table
//! says it does. A binding whose modifiers are matched wrong, or that is
//! pointed at the wrong action, fails here rather than only when a reader
//! finds the key does nothing.

mod common;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use tui::Events;
use tui::app::{App, Focus};
use tui::bindings::{Action, BINDINGS, Binding, Owner};

/// The service's undo stack (#646) is one per process, not scoped to the
/// caller that filled it - `common::ensure_service` starts exactly one for
/// this whole test binary, shared by every test function below. Each of
/// the four functions otherwise keeps its own operations inside its own
/// scratch folder, which is enough isolation on its own - except for Undo,
/// which asks a global "what happened last" that a concurrently running
/// function could answer first. Held for the whole body of each `#[test]`
/// below, the same shape `crates/tui/tests/terminal_in_the_window.rs` uses
/// for its own process-global state.
static SERIAL: Mutex<()> = Mutex::new(());

fn serially() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Everything drawn into `terminal`, run together into one string.
fn drawn(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    (0..area.height)
        .flat_map(|y| (0..area.width).map(move |x| (x, y)))
        .map(|(x, y)| buffer[(x, y)].symbol())
        .collect()
}

/// One key event, read once by [`tui::tick`] - `common::QueuedKeys` only
/// ever queues bare [`KeyCode`]s with no modifier, which is not enough to
/// press a binding such as Ctrl+Q.
struct OneKey(Option<KeyEvent>);

impl Events for OneKey {
    fn poll(&mut self, _timeout: Duration) -> std::io::Result<bool> {
        Ok(self.0.is_some())
    }

    fn read(&mut self) -> std::io::Result<Event> {
        Ok(Event::Key(
            self.0.take().expect("poll said an event was ready"),
        ))
    }
}

/// Presses one key event through the real loop and returns what is drawn
/// once it has settled - a keyless second tick for the lag `tui::tick`
/// itself documents, between the key that was handled and the frame that
/// shows it.
fn press_key(terminal: &mut Terminal<TestBackend>, app: &mut App, event: KeyEvent) -> String {
    let mut events = OneKey(Some(event));
    tui::tick(terminal, app, &mut events, Duration::ZERO).expect("a draw");
    let mut nothing = common::QueuedKeys::new(std::iter::empty());
    tui::tick(terminal, app, &mut nothing, Duration::ZERO).expect("a draw");
    drawn(terminal)
}

/// As [`press_key`], for a plain key with no modifiers held - the setup
/// steps below, moving into position before the binding under test.
fn press(terminal: &mut Terminal<TestBackend>, app: &mut App, code: KeyCode) -> String {
    press_key(terminal, app, KeyEvent::new(code, KeyModifiers::NONE))
}

/// Presses `binding`'s own keys - its code, with its own modifiers held.
fn press_binding(terminal: &mut Terminal<TestBackend>, app: &mut App, binding: &Binding) -> String {
    press_key(
        terminal,
        app,
        KeyEvent::new(binding.code, binding.modifiers),
    )
}

/// Draws with no key pressed until `needle` appears or a deadline passes -
/// the real round trip to the service takes a moment to answer.
fn wait_for(terminal: &mut Terminal<TestBackend>, app: &mut App, needle: &str) -> String {
    let mut nothing = common::QueuedKeys::new(std::iter::empty());
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        tui::tick(terminal, app, &mut nothing, Duration::ZERO).expect("a draw");
        let contents = drawn(terminal);
        if contents.contains(needle) || std::time::Instant::now() >= deadline {
            return contents;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn new_app_and_terminal(root: PathBuf) -> (Terminal<TestBackend>, App) {
    let app = App::new(root);
    let terminal = Terminal::new(TestBackend::new(80, 14)).expect("a test terminal");
    (terminal, app)
}

/// A scratch folder with two text files of distinguishable content, a
/// subfolder, and a third file after it - laid out so the sorted Contents
/// listing is `subdir/`, `aaa.txt`, `mmm.txt`, `zzz.txt`: a folder always
/// sorts before a file, whichever column the pane is sorted by (#640).
fn contents_scratch(name: &str) -> PathBuf {
    let root = common::scratch(name);
    std::fs::write(root.join("aaa.txt"), "AAA-CONTENT").expect("aaa.txt is written");
    std::fs::write(root.join("mmm.txt"), "MMM-CONTENT").expect("mmm.txt is written");
    std::fs::create_dir_all(root.join("subdir")).expect("subdir is created");
    std::fs::write(root.join("zzz.txt"), "ZZZ-CONTENT").expect("zzz.txt is written");
    root
}

/// A scratch folder with two subfolders, `alpha` (holding a nested `inner`
/// folder) and `beta`, so the sorted Folders tree is `alpha/`, `beta/`.
fn folders_scratch(name: &str) -> PathBuf {
    let root = common::scratch(name);
    std::fs::create_dir_all(root.join("alpha").join("inner")).expect("alpha/inner is created");
    std::fs::create_dir_all(root.join("beta")).expect("beta is created");
    root
}

/// A scratch folder holding one text file, `long.txt`, with sixty numbered
/// lines - long enough that the File pane's default height cannot show it
/// all at once, so a scroll binding actually has somewhere to move to
/// (#643).
fn file_pane_scratch(name: &str) -> PathBuf {
    let root = common::scratch(name);
    let content = (1..=60)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(root.join("long.txt"), content).expect("long.txt is written");
    root
}

#[test]
fn every_global_binding_reaches_the_action_it_names() {
    let _serial = serially();
    common::ensure_service();

    for binding in BINDINGS.iter().filter(|b| b.owner == Owner::Global) {
        let root = common::scratch(&format!("global-{}", binding.description));
        let (mut terminal, mut app) = new_app_and_terminal(root.clone());
        common::settle(&mut terminal, &mut app);

        match binding.action {
            Action::Quit | Action::CancelOrQuit => {
                press_binding(&mut terminal, &mut app, binding);
                assert!(
                    app.should_quit,
                    "{} should have quit with nothing in flight",
                    binding.description
                );
            }
            Action::FocusNext => {
                assert_eq!(app.focus(), Focus::Folders);
                press_binding(&mut terminal, &mut app, binding);
                assert_eq!(app.focus(), Focus::Contents);
            }
            Action::FocusPrevious => {
                assert_eq!(app.focus(), Focus::Folders);
                press_binding(&mut terminal, &mut app, binding);
                assert_eq!(app.focus(), Focus::File);
            }
            Action::GoBack => {
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert!(
                    shown.contains("nowhere to go back to"),
                    "with nothing visited yet, {} should say so: {shown:?}",
                    binding.description
                );
            }
            Action::GoForward => {
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert!(
                    shown.contains("nowhere to go forward to"),
                    "with nothing visited yet, {} should say so: {shown:?}",
                    binding.description
                );
            }
            Action::NavigateAboveRoot => {
                let parent = root
                    .parent()
                    .expect("a scratch root under the temp directory has a parent")
                    .to_string_lossy()
                    .into_owned();
                press_binding(&mut terminal, &mut app, binding);
                assert_eq!(
                    app.address_path(),
                    parent,
                    "{} should step above the root, to its parent",
                    binding.description
                );
            }
            Action::WidenPane => assert_widen_pane(&mut terminal, &mut app, binding),
            Action::NarrowPane => assert_narrow_pane(&mut terminal, &mut app, binding),
            Action::ToggleMaximize => assert_toggle_maximize(&mut terminal, &mut app, binding),
            Action::StartUndo => assert_start_undo(&mut terminal, &mut app, binding),
            Action::OpenSwitcher => {
                assert_opens_overlay(&mut terminal, &mut app, binding, "Go to Repository");
            }
            Action::StartFind => {
                assert_opens_overlay(&mut terminal, &mut app, binding, "Find (Enter/Esc)");
            }
            Action::OpenAllRepositories => {
                assert_opens_overlay(&mut terminal, &mut app, binding, "All Repositories");
            }
            Action::OpenReposRoots => {
                assert_opens_overlay(&mut terminal, &mut app, binding, "Repos Directory");
            }
            Action::OpenKeyboardReference => {
                assert_opens_overlay(&mut terminal, &mut app, binding, "Keyboard Reference");
            }
            Action::OpenCommandPalette => {
                assert_opens_overlay(&mut terminal, &mut app, binding, "Command Palette");
            }
            Action::OpenCertificates => {
                assert_opens_overlay(&mut terminal, &mut app, binding, "Certificates");
            }
            Action::Refresh => assert_refresh(&mut terminal, &mut app, binding, &root),
            other => panic!("no assertion written for the global action {other:?}"),
        }
    }
}

/// [`Action::StartUndo`]'s own assertion, split out of
/// `every_global_binding_reaches_the_action_it_names` to keep it under
/// clippy's line count (#650). The undo stack is one per service process
/// (`common::ensure_service`), shared with every other binding this test
/// binary presses - an empty stack cannot be assumed here. Undo something
/// this test made itself instead: create a folder, then undo it away.
fn assert_start_undo(terminal: &mut Terminal<TestBackend>, app: &mut App, binding: &Binding) {
    press(terminal, app, KeyCode::Tab);
    press(terminal, app, KeyCode::Char('D'));
    for c in "temp".chars() {
        press(terminal, app, KeyCode::Char(c));
    }
    press(terminal, app, KeyCode::Enter);
    let shown = wait_for(terminal, app, "temp");
    assert!(
        shown.contains("temp"),
        "setup: the folder should exist before undoing it: {shown:?}"
    );

    press_binding(terminal, app, binding);
    let mut shown = drawn(terminal);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while shown.contains("temp") && std::time::Instant::now() < deadline {
        let mut nothing = common::QueuedKeys::new(std::iter::empty());
        tui::tick(terminal, app, &mut nothing, Duration::ZERO).expect("a draw");
        shown = drawn(terminal);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !shown.contains("temp"),
        "{} should have undone the folder it just created: {shown:?}",
        binding.description
    );
}

/// [`Action::Refresh`]'s own assertion (#675), split out of
/// `every_global_binding_reaches_the_action_it_names` to keep it under
/// clippy's line count (#650). Presses the binding once with nothing on
/// disk yet, so the pane has a repository row to select in the first
/// place, then again after a sibling file appears and the repository's
/// branch changes underneath the running front end - asserting the new
/// file is drawn, the branch shown is the new one rather than the one
/// first read, the row selected before the second press is still
/// selected, and the status line says the listing was re-read.
fn assert_refresh(
    terminal: &mut Terminal<TestBackend>,
    app: &mut App,
    binding: &Binding,
    root: &Path,
) {
    let repository = root.join("repo");
    std::fs::create_dir_all(&repository).expect("a repository fixture directory");
    let made = std::process::Command::new("git")
        .args(["init", "--quiet", "--initial-branch", "main", "."])
        .current_dir(&repository)
        .status()
        .expect("git should be on PATH");
    assert!(made.success(), "git init should make the fixture");

    press_binding(terminal, app, binding);
    let shown = wait_for(terminal, app, "main");
    assert!(
        shown.contains("main"),
        "setup: the repository's branch should be shown once its listing loads: {shown:?}"
    );
    assert_eq!(
        app.selected_content_name(),
        Some("repo"),
        "setup: the only row should already be selected"
    );

    // Behind the running front end: a new sibling file appears, and the
    // repository's checked-out branch changes - neither of which a stale
    // listing, served from what the first press already fetched, could
    // ever show.
    std::fs::write(root.join("new-file.txt"), "new").expect("a new sibling file");
    let switched = std::process::Command::new("git")
        .args(["symbolic-ref", "HEAD", "refs/heads/other"])
        .current_dir(&repository)
        .status()
        .expect("git should be on PATH");
    assert!(
        switched.success(),
        "switching the unborn branch should succeed"
    );

    press_binding(terminal, app, binding);
    // Waited for last, since the File pane's own facts land a tick or two
    // after the listing itself does - by the time the new branch shows,
    // the new file has already been drawn too.
    let shown = wait_for(terminal, app, "other");
    assert!(
        shown.contains("new-file.txt"),
        "{} should show a file written to disk since the last listing: {shown:?}",
        binding.description
    );
    assert!(
        shown.contains("other"),
        "{} should re-read the repository's branch rather than reuse it: {shown:?}",
        binding.description
    );
    assert_eq!(
        app.selected_content_name(),
        Some("repo"),
        "{} should keep the row that was selected, since it still exists",
        binding.description
    );
    assert!(
        shown.contains("listing re-read"),
        "{} should say the listing was re-read: {shown:?}",
        binding.description
    );
}

/// [`Action::OpenSwitcher`], [`Action::StartFind`],
/// [`Action::OpenAllRepositories`] (#647) and [`Action::OpenReposRoots`]
/// (#648)'s own assertion, split out of
/// `every_global_binding_reaches_the_action_it_names` to keep it under
/// clippy's line count (#650): pressing the binding should draw the named
/// overlay's own title.
fn assert_opens_overlay(
    terminal: &mut Terminal<TestBackend>,
    app: &mut App,
    binding: &Binding,
    needle: &str,
) {
    let shown = press_binding(terminal, app, binding);
    assert!(
        shown.contains(needle),
        "{} should open the {needle:?} overlay: {shown:?}",
        binding.description
    );
}

/// [`Action::WidenPane`]'s own assertion, split out of
/// `every_global_binding_reaches_the_action_it_names` to keep it under
/// clippy's line count (#650).
fn assert_widen_pane(terminal: &mut Terminal<TestBackend>, app: &mut App, binding: &Binding) {
    press_binding(terminal, app, binding);
    let first = app
        .pane_widths()
        .expect("a resize should set explicit widths")
        .folders;
    press_binding(terminal, app, binding);
    let second = app
        .pane_widths()
        .expect("still set after a second press")
        .folders;
    assert!(
        second > first,
        "{} should grow the focused pane each press: {first} then {second}",
        binding.description
    );
}

/// As [`assert_widen_pane`], for [`Action::NarrowPane`].
fn assert_narrow_pane(terminal: &mut Terminal<TestBackend>, app: &mut App, binding: &Binding) {
    press_binding(terminal, app, binding);
    let first = app
        .pane_widths()
        .expect("a resize should set explicit widths")
        .folders;
    press_binding(terminal, app, binding);
    let second = app
        .pane_widths()
        .expect("still set after a second press")
        .folders;
    assert!(
        second < first,
        "{} should shrink the focused pane each press: {first} then {second}",
        binding.description
    );
}

/// As [`assert_widen_pane`], for [`Action::ToggleMaximize`].
fn assert_toggle_maximize(terminal: &mut Terminal<TestBackend>, app: &mut App, binding: &Binding) {
    let before = drawn(terminal);
    assert!(
        before.contains("Folders") && before.contains("Contents"),
        "{before:?}"
    );

    let maximized = press_binding(terminal, app, binding);
    assert!(
        maximized.contains("Folders"),
        "the maximised pane should still draw: {maximized:?}"
    );
    assert!(
        !maximized.contains("Contents"),
        "the other panes should not draw while maximised: {maximized:?}"
    );

    let restored = press_binding(terminal, app, binding);
    assert!(
        restored.contains("Contents"),
        "pressing it again should restore the three-pane layout: {restored:?}"
    );
}

#[test]
fn every_contents_binding_reaches_the_action_it_names() {
    let _serial = serially();
    common::ensure_service();

    for binding in BINDINGS.iter().filter(|b| b.owner == Owner::Contents) {
        let root = contents_scratch(&format!(
            "contents-{}-{:?}",
            binding.description, binding.code
        ));
        let (mut terminal, mut app) = new_app_and_terminal(root);
        wait_for(&mut terminal, &mut app, "aaa.txt");
        // Contents-owned bindings answer only once the Contents pane has
        // focus - proven by `every_global_binding_reaches_the_action_it_names`.
        press(&mut terminal, &mut app, KeyCode::Tab);

        match binding.action {
            Action::StartDelete
            | Action::StartRename
            | Action::StartCopy
            | Action::StartExtract => {
                assert_start_prompt(&mut terminal, &mut app, binding);
            }
            Action::ContentsDown => {
                // subdir/ is selected first; one row down reaches aaa.txt.
                press(&mut terminal, &mut app, KeyCode::Down);
                wait_for(&mut terminal, &mut app, "AAA-CONTENT");
                press_binding(&mut terminal, &mut app, binding);
                let shown = wait_for(&mut terminal, &mut app, "MMM-CONTENT");
                assert!(
                    shown.contains("MMM-CONTENT"),
                    "moving down should preview the next row: {shown:?}"
                );
            }
            Action::ContentsUp => {
                press(&mut terminal, &mut app, KeyCode::Down);
                wait_for(&mut terminal, &mut app, "AAA-CONTENT");
                press(&mut terminal, &mut app, KeyCode::Down);
                wait_for(&mut terminal, &mut app, "MMM-CONTENT");
                press_binding(&mut terminal, &mut app, binding);
                let shown = wait_for(&mut terminal, &mut app, "AAA-CONTENT");
                assert!(
                    shown.contains("AAA-CONTENT"),
                    "moving back up should preview the previous row again: {shown:?}"
                );
            }
            Action::ContentsOpen => {
                // subdir/ sorts first among subdir/, aaa.txt, mmm.txt,
                // zzz.txt - it is already the selected row.
                press_binding(&mut terminal, &mut app, binding);
                assert_eq!(
                    app.focus(),
                    Focus::Folders,
                    "opening the subfolder row should hand focus to the tree"
                );
            }
            Action::ContentsSortName => {
                let shown = drawn(&terminal);
                assert!(
                    shown.find("aaa.txt").unwrap() < shown.find("zzz.txt").unwrap(),
                    "the default sort is ascending by name: {shown:?}"
                );
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert!(
                    shown.find("zzz.txt").unwrap() < shown.find("aaa.txt").unwrap(),
                    "pressing the same sort key again should reverse the order: {shown:?}"
                );
            }
            Action::ContentsSortType | Action::ContentsSortSize | Action::ContentsSortModified => {
                // Every file here ties on type, size and modified time, so
                // the name tiebreak decides the order - and is itself
                // reversed along with everything else on the second press,
                // proving the direction toggled.
                press_binding(&mut terminal, &mut app, binding);
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert!(
                    shown.find("zzz.txt").unwrap() < shown.find("aaa.txt").unwrap(),
                    "sorting by {:?} and pressing it again should reverse direction: {shown:?}",
                    binding.action
                );
            }
            Action::StartFilter => assert_start_filter(&mut terminal, &mut app, binding),
            Action::ToggleChangedFilter => {
                assert_toggle_changed_filter(&mut terminal, &mut app, binding);
            }
            Action::ExtendContentsDown
            | Action::ExtendContentsUp
            | Action::ExtendContentsHome
            | Action::ExtendContentsEnd
            | Action::ToggleContentsSelected
            | Action::InvertContentsSelection => {
                assert_selection_binding(&mut terminal, &mut app, binding);
            }
            Action::StartCreateDirectory | Action::StartCreateFile => {
                assert_start_create(&mut terminal, &mut app, binding);
            }
            Action::StartOpen
            | Action::ClipboardCopy
            | Action::ClipboardCut
            | Action::ClipboardPaste => {
                assert_open_or_clipboard(&mut terminal, &mut app, binding);
            }
            Action::ContentsOpenInEditor
            | Action::ContentsCopyPath
            | Action::ContentsCopyRemoteAddress
            | Action::ContentsShowInFileManager
            | Action::ContentsOpenOnTheWeb => {
                assert_open_in_tools(&mut terminal, &mut app, binding);
            }
            other => panic!("no assertion written for the contents action {other:?}"),
        }
    }
}

/// [`Action::StartDelete`], [`Action::StartRename`], [`Action::StartCopy`]
/// and [`Action::StartExtract`]'s own assertion, split out of
/// `every_contents_binding_reaches_the_action_it_names` to keep it under
/// clippy's line count (#650). Each one prompts for the row one Down from
/// subdir/ - aaa.txt - and now draws as a real modal (#646) naming the
/// full path rather than the row alone, so the check looks for the title,
/// the full path, and the typed text, rather than one fixed sentence.
fn assert_start_prompt(terminal: &mut Terminal<TestBackend>, app: &mut App, binding: &Binding) {
    press(terminal, app, KeyCode::Down);
    let full_path = PathBuf::from(app.address_path())
        .join("aaa.txt")
        .display()
        .to_string();
    let shown = press_binding(terminal, app, binding);
    let (title, input) = match binding.action {
        Action::StartDelete => ("Delete aaa.txt?", None),
        Action::StartRename => ("Rename", Some("aaa.txt")),
        Action::StartCopy => ("Copy", Some("aaa.txt")),
        Action::StartExtract => ("Extract", Some("aaa")),
        other => panic!("assert_start_prompt was not written for {other:?}"),
    };
    assert!(
        shown.contains(title),
        "{shown:?} should show the modal's title, {title:?}"
    );
    assert!(
        shown.contains(&full_path),
        "{shown:?} should name the full path, not just the row (#524)"
    );
    if let Some(input) = input {
        assert!(
            shown.contains(input),
            "{shown:?} should show the typed text, {input:?}"
        );
    }
}

/// The selection bindings' own assertions (#676), split out of
/// `every_contents_binding_reaches_the_action_it_names` to keep it under
/// clippy's line count (#650). Every row a selected row's own name
/// carries a `*` glyph (`crates/tui/src/app.rs`'s `contents_row`), so the
/// drawn buffer alone says how many rows are selected.
fn assert_selection_binding(
    terminal: &mut Terminal<TestBackend>,
    app: &mut App,
    binding: &Binding,
) {
    // Not a bare `*`: the scratch root this test runs under is itself
    // named after `binding.code` (`Char('*')`, for the invert binding
    // under test here), so a bare glyph count would also catch the
    // breadcrumb naming the folder rather than only a marked row.
    let marked = |shown: &str, name: &str| shown.contains(&format!("* {name}"));
    match binding.action {
        Action::ExtendContentsDown => {
            // subdir/ is selected first; extending down twice should
            // cover it, aaa.txt and mmm.txt - three of the four rows.
            press_binding(terminal, app, binding);
            let shown = press_binding(terminal, app, binding);
            assert!(
                marked(&shown, "subdir/") && marked(&shown, "aaa.txt") && marked(&shown, "mmm.txt"),
                "extending the selection down twice should mark three rows: {shown:?}"
            );
            assert!(
                !marked(&shown, "zzz.txt"),
                "the fourth row should not be marked yet: {shown:?}"
            );
        }
        Action::ExtendContentsUp => {
            // Move to the last row, zzz.txt, then extend up twice to
            // cover it, mmm.txt and aaa.txt.
            press(terminal, app, KeyCode::Down);
            press(terminal, app, KeyCode::Down);
            press(terminal, app, KeyCode::Down);
            press_binding(terminal, app, binding);
            let shown = press_binding(terminal, app, binding);
            assert!(
                marked(&shown, "aaa.txt") && marked(&shown, "mmm.txt") && marked(&shown, "zzz.txt"),
                "extending the selection up twice should mark three rows: {shown:?}"
            );
            assert!(
                !marked(&shown, "subdir/"),
                "the first row should not be marked yet: {shown:?}"
            );
        }
        Action::ExtendContentsHome => {
            // Move to the last row, then extend to the top - every row.
            press(terminal, app, KeyCode::Down);
            press(terminal, app, KeyCode::Down);
            press(terminal, app, KeyCode::Down);
            let shown = press_binding(terminal, app, binding);
            assert!(
                marked(&shown, "subdir/")
                    && marked(&shown, "aaa.txt")
                    && marked(&shown, "mmm.txt")
                    && marked(&shown, "zzz.txt"),
                "extending to the top should mark every row: {shown:?}"
            );
        }
        Action::ExtendContentsEnd => {
            let shown = press_binding(terminal, app, binding);
            assert!(
                marked(&shown, "subdir/")
                    && marked(&shown, "aaa.txt")
                    && marked(&shown, "mmm.txt")
                    && marked(&shown, "zzz.txt"),
                "extending to the bottom should mark every row: {shown:?}"
            );
        }
        Action::ToggleContentsSelected => {
            let shown = press_binding(terminal, app, binding);
            assert!(
                marked(&shown, "subdir/"),
                "Insert should mark the row it was pressed on, even after the \
                 cursor itself has moved on: {shown:?}"
            );
        }
        Action::InvertContentsSelection => {
            let shown = press_binding(terminal, app, binding);
            assert!(
                marked(&shown, "subdir/")
                    && marked(&shown, "aaa.txt")
                    && marked(&shown, "mmm.txt")
                    && marked(&shown, "zzz.txt"),
                "inverting an empty selection should mark every row: {shown:?}"
            );
        }
        other => panic!("assert_selection_binding was not written for {other:?}"),
    }
}

/// [`Action::StartCreateDirectory`] and [`Action::StartCreateFile`]'s own
/// assertion, split out of `every_contents_binding_reaches_the_action_it_names`
/// to keep it under clippy's line count (#650): typing a name and
/// confirming should create it through the service and select the new row
/// once the reloaded listing lands.
fn assert_start_create(terminal: &mut Terminal<TestBackend>, app: &mut App, binding: &Binding) {
    let title = match binding.action {
        Action::StartCreateDirectory => "New folder",
        Action::StartCreateFile => "New file",
        other => panic!("assert_start_create was not written for {other:?}"),
    };
    let shown = press_binding(terminal, app, binding);
    assert!(
        shown.contains(title),
        "{shown:?} should show the {title} modal"
    );

    for c in "fresh".chars() {
        press(terminal, app, KeyCode::Char(c));
    }
    press(terminal, app, KeyCode::Enter);

    let shown = wait_for(terminal, app, "fresh");
    assert!(
        shown.contains("fresh"),
        "{} should create the new row and reload the listing: {shown:?}",
        binding.description
    );
}

/// [`Action::StartOpen`], [`Action::ClipboardCopy`], [`Action::ClipboardCut`]
/// and [`Action::ClipboardPaste`]'s own assertion, split out of
/// `every_contents_binding_reaches_the_action_it_names` to keep it under
/// clippy's line count (#650).
fn assert_open_or_clipboard(
    terminal: &mut Terminal<TestBackend>,
    app: &mut App,
    binding: &Binding,
) {
    match binding.action {
        Action::StartOpen => {
            press(terminal, app, KeyCode::Down);
            press_binding(terminal, app, binding);
            let shown = wait_for(terminal, app, "opened");
            assert!(
                shown.contains("opened"),
                "{} should report what Open handed over: {shown:?}",
                binding.description
            );
        }
        Action::ClipboardCopy => {
            press(terminal, app, KeyCode::Down);
            let shown = press_binding(terminal, app, binding);
            assert!(
                shown.contains("1 item copied"),
                "{} should say what it copied: {shown:?}",
                binding.description
            );
        }
        Action::ClipboardCut => {
            press(terminal, app, KeyCode::Down);
            let shown = press_binding(terminal, app, binding);
            assert!(
                shown.contains("1 item cut"),
                "{} should say what it cut: {shown:?}",
                binding.description
            );
        }
        Action::ClipboardPaste => {
            // Copy aaa.txt then paste it back into the same folder: a real
            // round trip that ends in the same refusal the service gives
            // any other collision, worded exactly as it is (#646, item 7).
            press(terminal, app, KeyCode::Down);
            press_key(
                terminal,
                app,
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            );
            press_binding(terminal, app, binding);
            let shown = wait_for(terminal, app, "already exists");
            assert!(
                shown.contains("already exists"),
                "{} onto the same name should surface the service's own refusal: {shown:?}",
                binding.description
            );
        }
        other => panic!("assert_open_or_clipboard was not written for {other:?}"),
    }
}

/// [`Action::ContentsOpenInEditor`], [`Action::FoldersOpenInEditor`],
/// [`Action::ContentsCopyPath`], [`Action::FoldersCopyPath`],
/// [`Action::ContentsCopyRemoteAddress`], [`Action::FoldersCopyRemoteAddress`],
/// [`Action::ContentsShowInFileManager`] and
/// [`Action::FoldersShowInFileManager`]'s own assertion (#674), plus
/// [`Action::ContentsOpenOnTheWeb`]'s (#679), shared between
/// `every_contents_binding_reaches_the_action_it_names` and
/// `every_folders_binding_reaches_the_action_it_names`: a terminal reader
/// gets no window appearing to confirm any of these worked, so the status
/// line is the only place to check. The row each fixture starts on - a
/// plain folder, not a working copy - is never a working copy with a
/// remote, which is exactly what proves "Copy remote address" reports the
/// no-remote case, and "Open on the web" the no-web-page one, rather than
/// silently doing nothing.
fn assert_open_in_tools(terminal: &mut Terminal<TestBackend>, app: &mut App, binding: &Binding) {
    match binding.action {
        Action::ContentsOpenInEditor | Action::FoldersOpenInEditor => {
            let shown = press_binding(terminal, app, binding);
            assert!(
                shown.contains("editor"),
                "{} should report its outcome on the status line: {shown:?}",
                binding.description
            );
        }
        Action::ContentsShowInFileManager | Action::FoldersShowInFileManager => {
            let shown = press_binding(terminal, app, binding);
            assert!(
                shown.contains("file manager"),
                "{} should report its outcome on the status line: {shown:?}",
                binding.description
            );
        }
        Action::ContentsCopyPath | Action::FoldersCopyPath => {
            let shown = press_binding(terminal, app, binding);
            assert!(
                shown.contains("the path"),
                "{} should report its outcome on the status line: {shown:?}",
                binding.description
            );
        }
        Action::ContentsCopyRemoteAddress | Action::FoldersCopyRemoteAddress => {
            let shown = press_binding(terminal, app, binding);
            assert!(
                shown.contains("this folder has no remote address"),
                "{} on a plain folder should say so rather than copying nothing \
                 silently: {shown:?}",
                binding.description
            );
        }
        Action::ContentsOpenOnTheWeb => {
            let shown = press_binding(terminal, app, binding);
            assert!(
                shown.contains("this folder has no web page to open"),
                "{} on a plain folder should say so rather than opening nothing \
                 silently: {shown:?}",
                binding.description
            );
        }
        other => panic!("assert_open_in_tools was not written for {other:?}"),
    }
}

/// [`Action::StartFilter`]'s own assertion, split out of
/// `every_contents_binding_reaches_the_action_it_names` to keep it under
/// clippy's line count (#650).
fn assert_start_filter(terminal: &mut Terminal<TestBackend>, app: &mut App, binding: &Binding) {
    let shown = press_binding(terminal, app, binding);
    assert!(
        shown.contains("Filter:"),
        "{shown:?} should show the filter prompt"
    );
    let shown = press(terminal, app, KeyCode::Char('z'));
    assert!(
        shown.contains("zzz.txt") && !shown.contains("aaa.txt") && !shown.contains("mmm.txt"),
        "typing into the filter should narrow the listing to the matching name: {shown:?}"
    );
}

/// As [`assert_start_filter`], for [`Action::ToggleChangedFilter`].
fn assert_toggle_changed_filter(
    terminal: &mut Terminal<TestBackend>,
    app: &mut App,
    binding: &Binding,
) {
    let shown = press_binding(terminal, app, binding);
    // The pane is too narrow here to keep the whole sentence on one row,
    // so this checks the words rather than the exact phrase - and that
    // the narrowing itself took hold.
    assert!(
        shown.contains("No repositories")
            && shown.contains("uncommitted changes")
            && !shown.contains("aaa.txt"),
        "narrowing to changed repositories where none are tracked should say so: {shown:?}"
    );
    let shown = press_binding(terminal, app, binding);
    assert!(
        shown.contains("aaa.txt"),
        "pressing it again should lift the narrowing: {shown:?}"
    );
}

#[test]
fn every_folders_binding_reaches_the_action_it_names() {
    let _serial = serially();
    common::ensure_service();

    for binding in BINDINGS.iter().filter(|b| b.owner == Owner::Folders) {
        let root = folders_scratch(&format!(
            "folders-{}-{:?}",
            binding.description, binding.code
        ));
        let (mut terminal, mut app) = new_app_and_terminal(root);
        wait_for(&mut terminal, &mut app, "alpha/");
        // Folders is the default focus, so no Tab is needed here.

        match binding.action {
            Action::FoldersDown => {
                press_binding(&mut terminal, &mut app, binding);
                let shown = wait_for(&mut terminal, &mut app, "inner");
                assert!(
                    shown.contains("inner"),
                    "moving onto alpha should re-list its own contents: {shown:?}"
                );
            }
            Action::FoldersUp => {
                press(&mut terminal, &mut app, KeyCode::Down);
                wait_for(&mut terminal, &mut app, "inner");
                press(&mut terminal, &mut app, KeyCode::Down);
                press_binding(&mut terminal, &mut app, binding);
                let shown = wait_for(&mut terminal, &mut app, "inner");
                assert!(
                    shown.contains("inner"),
                    "moving back up onto alpha should re-list its contents again: {shown:?}"
                );
            }
            Action::FoldersExpand => {
                press(&mut terminal, &mut app, KeyCode::Down);
                // Selecting alpha already lists its own contents, "inner/"
                // among them - one occurrence, from the Contents pane
                // alone. Expanding must add a second: alpha's child, now
                // also a row of the Folders tree itself.
                wait_for(&mut terminal, &mut app, "inner");
                let before = drawn(&terminal).matches("inner").count();
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert!(
                    shown.matches("inner").count() > before,
                    "expanding alpha should add its child as a tree row, not just \
                     leave it in the Contents pane already showing it: {shown:?}"
                );
            }
            Action::FoldersCollapse => {
                press(&mut terminal, &mut app, KeyCode::Down);
                press(&mut terminal, &mut app, KeyCode::Right);
                wait_for(&mut terminal, &mut app, "inner");
                let before = drawn(&terminal).matches("inner").count();
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert!(
                    shown.matches("inner").count() < before,
                    "collapsing alpha should remove its child's tree row - the \
                     Contents pane still names it, so this must not go to zero: {shown:?}"
                );
            }
            Action::FoldersOpenInEditor
            | Action::FoldersCopyPath
            | Action::FoldersCopyRemoteAddress
            | Action::FoldersShowInFileManager => {
                assert_open_in_tools(&mut terminal, &mut app, binding);
            }
            other => panic!("no assertion written for the folders action {other:?}"),
        }
    }
}

#[test]
fn every_file_binding_reaches_the_action_it_names() {
    let _serial = serially();
    common::ensure_service();

    for binding in BINDINGS.iter().filter(|b| b.owner == Owner::File) {
        let root = file_pane_scratch(&format!("file-{}-{:?}", binding.description, binding.code));
        let (mut terminal, mut app) = new_app_and_terminal(root);
        wait_for(&mut terminal, &mut app, "line 1");
        // File-owned bindings answer only once the File pane has focus -
        // proven by `every_global_binding_reaches_the_action_it_names`.
        press(&mut terminal, &mut app, KeyCode::Tab);
        press(&mut terminal, &mut app, KeyCode::Tab);
        assert_eq!(app.focus(), Focus::File);

        match binding.action {
            Action::FileScrollDown => {
                let before = drawn(&terminal);
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert_ne!(
                    before, shown,
                    "{} should have scrolled the text down by one line",
                    binding.description
                );
            }
            Action::FileScrollUp => {
                press(&mut terminal, &mut app, KeyCode::Down);
                let before = drawn(&terminal);
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert_ne!(
                    before, shown,
                    "{} should have scrolled the text back up",
                    binding.description
                );
            }
            Action::FileScrollPageDown => {
                let before = drawn(&terminal);
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert_ne!(
                    before, shown,
                    "{} should have scrolled the text down by a page",
                    binding.description
                );
            }
            Action::FileScrollPageUp => {
                press(&mut terminal, &mut app, KeyCode::End);
                let before = drawn(&terminal);
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert_ne!(
                    before, shown,
                    "{} should have scrolled the text back up by a page",
                    binding.description
                );
            }
            Action::FileScrollHome => {
                press(&mut terminal, &mut app, KeyCode::End);
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert!(
                    shown.contains("line 1"),
                    "{} should scroll back to the file's start: {shown:?}",
                    binding.description
                );
            }
            Action::FileScrollEnd => {
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert!(
                    shown.contains("line 60"),
                    "{} should reach the file's last line: {shown:?}",
                    binding.description
                );
            }
            Action::FileViewPrevious | Action::FileViewNext => {
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert!(
                    shown.contains("line 1"),
                    "{} should still show the file's own text: {shown:?}",
                    binding.description
                );
            }
            Action::FileActivateView => {
                // long.txt's plugin offers Preview and Text; Edit is
                // appended after them (#645).
                press(&mut terminal, &mut app, KeyCode::Right);
                press(&mut terminal, &mut app, KeyCode::Right);
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert!(
                    shown.contains("Editing"),
                    "{} should start editing once the Edit view is showing: {shown:?}",
                    binding.description
                );
            }
            other => panic!("no assertion written for the file action {other:?}"),
        }
    }
}
