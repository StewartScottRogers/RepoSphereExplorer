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
use std::path::PathBuf;
use std::time::Duration;
use tui::Events;
use tui::app::{App, Focus};
use tui::bindings::{Action, BINDINGS, Binding, Owner};

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
/// listing is `aaa.txt`, `mmm.txt`, `subdir/`, `zzz.txt`.
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

#[test]
fn every_global_binding_reaches_the_action_it_names() {
    common::ensure_service();

    for binding in BINDINGS.iter().filter(|b| b.owner == Owner::Global) {
        let (mut terminal, mut app) =
            new_app_and_terminal(common::scratch(&format!("global-{}", binding.description)));
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
            other => panic!("no assertion written for the global action {other:?}"),
        }
    }
}

#[test]
fn every_contents_binding_reaches_the_action_it_names() {
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
            Action::StartDelete => {
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert!(
                    shown.contains("Delete aaa.txt? y/n"),
                    "{shown:?} should show the delete prompt for the selected row"
                );
            }
            Action::StartRename => {
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert!(
                    shown.contains("Rename to: aaa.txt_"),
                    "{shown:?} should show the rename prompt for the selected row"
                );
            }
            Action::StartCopy => {
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert!(
                    shown.contains("Copy to: aaa.txt_"),
                    "{shown:?} should show the copy prompt for the selected row"
                );
            }
            Action::StartExtract => {
                let shown = press_binding(&mut terminal, &mut app, binding);
                assert!(
                    shown.contains("Extract to: aaa_"),
                    "{shown:?} should show the extract prompt, stemmed from the selected row"
                );
            }
            Action::ContentsDown => {
                wait_for(&mut terminal, &mut app, "AAA-CONTENT");
                press_binding(&mut terminal, &mut app, binding);
                let shown = wait_for(&mut terminal, &mut app, "MMM-CONTENT");
                assert!(
                    shown.contains("MMM-CONTENT"),
                    "moving down should preview the next row: {shown:?}"
                );
            }
            Action::ContentsUp => {
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
                // aaa.txt, mmm.txt, subdir/, zzz.txt - two rows down lands
                // on the one folder among them.
                press(&mut terminal, &mut app, KeyCode::Down);
                press(&mut terminal, &mut app, KeyCode::Down);
                press_binding(&mut terminal, &mut app, binding);
                assert_eq!(
                    app.focus(),
                    Focus::Folders,
                    "opening the subfolder row should hand focus to the tree"
                );
            }
            other => panic!("no assertion written for the contents action {other:?}"),
        }
    }
}

#[test]
fn every_folders_binding_reaches_the_action_it_names() {
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
            other => panic!("no assertion written for the folders action {other:?}"),
        }
    }
}
