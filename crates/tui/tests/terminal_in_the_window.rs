//! Driving the real terminal event loop, wired the way `main` wires it.
//!
//! `app`'s own tests know what a key means; `lib`'s know what a `Response`
//! draws. Neither has ever met the loop that ties them together - the draw,
//! tick, poll and dispatch order `run`'s body used to live in `main.rs`,
//! reached only by a real terminal. This is the first test to drive that
//! loop itself, through a `ratatui::backend::TestBackend` and a real `App`
//! talking to a real service on this test binary's private socket, and to
//! assert on the buffer it draws rather than on `App` alone (CLAUDE.md rule
//! 14).

mod common;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use std::sync::{Mutex, MutexGuard};
use tui::Events;
use tui::app::App;

/// One service at a time: these tests share a single-threaded test service
/// (see `common::ensure_service`), and a burst of connections from several
/// tests at once starves some of them. Mirrors
/// `crates/gui/tests/certificates_in_the_window.rs`.
static SERIAL: Mutex<()> = Mutex::new(());

/// Takes the shared lock, tolerating a previous test having panicked while
/// holding it - a poisoned lock would otherwise turn one failure into many.
fn serially() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Everything drawn into `terminal`, one string per row.
fn drawn_rows(terminal: &Terminal<TestBackend>) -> Vec<String> {
    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    (0..area.height)
        .map(|y| (0..area.width).map(|x| buffer[(x, y)].symbol()).collect())
        .collect()
}

/// As [`drawn_rows`], run together into one string.
fn drawn(terminal: &Terminal<TestBackend>) -> String {
    drawn_rows(terminal).concat()
}

/// Presses `code` and returns what is drawn once it has been handled.
///
/// A single [`tui::tick`] draws the state from *before* the key it goes on
/// to handle - the same lag the real loop has, where a keypress shows up on
/// the following frame. A second, keyless tick is the following frame.
fn press(terminal: &mut Terminal<TestBackend>, app: &mut App, code: KeyCode) -> String {
    let mut events = common::QueuedKeys::new([code]);
    tui::tick(terminal, app, &mut events, std::time::Duration::ZERO).expect("a draw");
    let mut nothing = common::QueuedKeys::new(std::iter::empty());
    tui::tick(terminal, app, &mut nothing, std::time::Duration::ZERO).expect("a draw");
    drawn(terminal)
}

/// One key event with modifiers, read once - `common::QueuedKeys` only
/// carries a bare [`KeyCode`], which cannot express a Ctrl+S.
struct OneKeyWithModifiers(Option<(KeyCode, KeyModifiers)>);

impl Events for OneKeyWithModifiers {
    fn poll(&mut self, _timeout: std::time::Duration) -> std::io::Result<bool> {
        Ok(self.0.is_some())
    }

    fn read(&mut self) -> std::io::Result<Event> {
        let (code, modifiers) = self.0.take().expect("poll said an event was ready");
        Ok(Event::Key(KeyEvent::new(code, modifiers)))
    }
}

/// As [`press`], but the key carries `modifiers` - for a Ctrl+S, which
/// [`press`] cannot express.
fn press_with_modifiers(
    terminal: &mut Terminal<TestBackend>,
    app: &mut App,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> String {
    let mut events = OneKeyWithModifiers(Some((code, modifiers)));
    tui::tick(terminal, app, &mut events, std::time::Duration::ZERO).expect("a draw");
    let mut nothing = common::QueuedKeys::new(std::iter::empty());
    tui::tick(terminal, app, &mut nothing, std::time::Duration::ZERO).expect("a draw");
    drawn(terminal)
}

/// Repeatedly draws with no key pressed until `needle` appears in the
/// buffer or a deadline passes, returning whatever was last drawn - the
/// real round trip to the service takes a moment to answer.
fn wait_for(terminal: &mut Terminal<TestBackend>, app: &mut App, needle: &str) -> String {
    let mut nothing = common::QueuedKeys::new(std::iter::empty());
    // Generous rather than tight (#741): the loop returns the moment
    // `needle` appears, so a longer deadline costs nothing while the real
    // round trip to the service is working.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        tui::tick(terminal, app, &mut nothing, std::time::Duration::ZERO).expect("a draw");
        let contents = drawn(terminal);
        if contents.contains(needle) || std::time::Instant::now() >= deadline {
            return contents;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn opening_at_the_repos_directory_draws_the_three_panes_and_the_status_line() {
    let _serial = serially();
    common::ensure_service();
    let root = common::scratch("opening");
    let mut app = App::new(root);
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("a test terminal");

    // The very first draw still says "loading..."; the status line settles
    // to its default help hint once the root's own listing has answered.
    let contents = wait_for(&mut terminal, &mut app, "? for keys");

    assert!(contents.contains("Folders"), "{contents}");
    assert!(contents.contains("Contents"), "{contents}");
    assert!(contents.contains("File"), "{contents}");
    assert!(
        contents.contains("? for keys"),
        "the status line's help hint should be drawn: {contents}"
    );
}

#[test]
fn moving_the_cursor_in_folders_re_lists_contents() {
    let _serial = serially();
    common::ensure_service();
    let root = common::scratch("folders-relist");
    std::fs::create_dir_all(root.join("alpha")).expect("alpha is created");
    std::fs::write(root.join("alpha").join("one.txt"), "hi").expect("one.txt is written");
    std::fs::create_dir_all(root.join("beta")).expect("beta is created");

    let mut app = App::new(root);
    let mut terminal = Terminal::new(TestBackend::new(80, 14)).expect("a test terminal");
    let opened = wait_for(&mut terminal, &mut app, "alpha/");
    assert!(
        opened.contains("beta/"),
        "the root's own listing should show both folders: {opened}"
    );

    press(&mut terminal, &mut app, KeyCode::Down);
    let moved = wait_for(&mut terminal, &mut app, "one.txt");

    assert!(
        moved.contains("one.txt"),
        "moving onto alpha should re-list its own contents: {moved}"
    );
}

#[test]
fn a_key_that_opens_a_prompt_draws_it() {
    let _serial = serially();
    common::ensure_service();
    let root = common::scratch("prompt-draws");
    std::fs::write(root.join("notes.txt"), "hi").expect("notes.txt is written");

    let mut app = App::new(root);
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("a test terminal");
    wait_for(&mut terminal, &mut app, "notes.txt");

    press(&mut terminal, &mut app, KeyCode::Tab);
    let prompted = press(&mut terminal, &mut app, KeyCode::Delete);

    assert!(
        prompted.contains("Delete notes.txt?") && prompted.contains("y/n"),
        "the confirmation modal should be drawn: {prompted}"
    );
}

#[test]
fn q_leaves_the_loop() {
    let _serial = serially();
    common::ensure_service();
    let root = common::scratch("quit-on-q");
    let mut app = App::new(root);
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("a test terminal");

    let mut events = common::QueuedKeys::new([KeyCode::Char('q')]);
    tui::run(&mut terminal, &mut app, &mut events).expect("the loop runs");

    assert!(app.should_quit, "q should have left the loop");
}

#[test]
fn esc_leaves_the_loop_with_nothing_in_flight() {
    let _serial = serially();
    common::ensure_service();
    let root = common::scratch("quit-on-esc");
    let mut app = App::new(root);
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("a test terminal");
    // Let the folder's own (empty) listing settle, so nothing is pending -
    // otherwise Esc cancels that instead of quitting.
    common::settle(&mut terminal, &mut app);

    let mut events = common::QueuedKeys::new([KeyCode::Esc]);
    tui::run(&mut terminal, &mut app, &mut events).expect("the loop runs");

    assert!(
        app.should_quit,
        "Esc with nothing in flight should have left the loop"
    );
}

#[test]
fn tests_do_not_reach_the_developers_own_service() {
    let _serial = serially();
    common::ensure_service();

    let name = protocol::socket_name().expect("the platform has a socket name");
    let debug = format!("{name:?}");

    assert!(
        !debug.contains(protocol::SOCKET_NAME),
        "the socket in use must not be the shared default every developer's \
         own service listens on: {debug}"
    );
}

/// Every repository row on screen is asked about, however the reader got
/// there (#641).
///
/// `App` used to keep a scroll model of its own while the table re-derived
/// its window from the selected row on each frame. The two agree while a
/// reader holds Down and part company after a jump: sort the listing, step
/// back up one, and a row plainly on screen had never been asked about and
/// kept the not-known marker for ever (the review of #672). The table's own
/// offset is the one answer now, and this drives it through a real terminal
/// rather than through `App` alone.
#[test]
fn a_row_that_comes_into_view_after_a_sort_is_asked_about() {
    let _serial = serially();
    common::ensure_service();
    let root = common::scratch("statuses-after-a-sort");
    // Enough rows to need scrolling in a short terminal, named so that
    // sorting the other way moves the selection a long way.
    for index in 0..20u32 {
        let repository = root.join(format!("repo-{index:02}"));
        std::fs::create_dir_all(&repository).expect("a fixture directory");
        // Real checkouts: the working-tree status of a hand-made `.git`
        // answers at once, which would hide the very marker this test is
        // about.
        let made = std::process::Command::new("git")
            .args(["init", "--quiet", "--initial-branch", "main", "."])
            .current_dir(&repository)
            .status()
            .expect("git should be on PATH");
        assert!(made.success(), "git init should make the fixture");
        std::fs::write(repository.join("tracked.txt"), "a file\n").expect("a tracked file");
        // Committed, so the working tree has an answer to give: an
        // uncommitted checkout answers "cannot tell" at once, which is not
        // the marker this test is about.
        for arguments in [
            vec!["add", "--all"],
            vec![
                "-c",
                "user.name=Repos Explorer Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "--message",
                "a commit",
            ],
        ] {
            let ran = std::process::Command::new("git")
                .args(&arguments)
                .current_dir(&repository)
                .status()
                .expect("git should be on PATH");
            assert!(ran.success(), "git {arguments:?} should make the fixture");
        }
    }

    let mut terminal = Terminal::new(TestBackend::new(100, 12)).expect("a test terminal");
    let mut app = App::new(root.clone());
    common::settle(&mut terminal, &mut app);

    // Into the listing, then a long way down it.
    press(&mut terminal, &mut app, KeyCode::Tab);
    for _ in 0..14 {
        press(&mut terminal, &mut app, KeyCode::Down);
    }
    // Sort the other way: the selected row keeps its place in the listing
    // and everything around it moves.
    press(&mut terminal, &mut app, KeyCode::Char('n'));
    let after_a_step_back = press(&mut terminal, &mut app, KeyCode::Up);
    assert!(
        !after_a_step_back.is_empty(),
        "the listing is drawn after the sort"
    );

    // What the front end believes is on screen has to be what it drew:
    // its status requests are scoped by that belief, so a row it has wrong
    // is a row nobody ever asks about.
    let settled = wait_for(&mut terminal, &mut app, "repo-");
    // The Contents pane's own columns, inside its borders: the Folders
    // tree down the left holds the same names, and the three panes are laid
    // out 25/35/40 with a one-column border each side.
    let contents_columns = |row: &str| row.chars().skip(26).take(33).collect::<String>();
    let drawn_names: Vec<String> = drawn_rows(&terminal)
        .into_iter()
        .filter_map(|row| {
            contents_columns(&row)
                .split_whitespace()
                .find(|word| word.starts_with("repo-"))
                .map(|name| name.trim_end_matches('/').to_owned())
        })
        .collect();
    assert!(
        !drawn_names.is_empty(),
        "repository rows are on screen: {settled}"
    );

    let believed: Vec<String> = app
        .visible_rows()
        .filter_map(|index| app.content_name_at(index))
        .map(|name| name.trim_end_matches('/').to_owned())
        .collect();
    assert_eq!(
        drawn_names, believed,
        "the rows the front end asks about are the rows it drew"
    );

    let unanswered: Vec<String> = drawn_rows(&terminal)
        .into_iter()
        .map(|row| contents_columns(&row))
        .filter(|row| row.contains("repo-") && row.contains('\u{2026}'))
        .collect();
    assert!(
        unanswered.is_empty(),
        "and every one of them has an answer; these do not:\n{}",
        unanswered.join("\n")
    );
}

/// Steps the File pane to its `"Edit"` view and presses Enter to start
/// editing (#645) - two Right presses past a plain text file's `Preview`
/// and `Text` views, then activation.
fn open_the_editor(terminal: &mut Terminal<TestBackend>, app: &mut App) -> String {
    press(terminal, app, KeyCode::Tab);
    press(terminal, app, KeyCode::Tab);
    press(terminal, app, KeyCode::Right);
    press(terminal, app, KeyCode::Right);
    press(terminal, app, KeyCode::Enter)
}

#[test]
fn typing_and_saving_writes_the_file_through_the_service() {
    let _serial = serially();
    common::ensure_service();
    let root = common::scratch("edit-and-save");
    let path = root.join("notes.txt");
    std::fs::write(&path, "hello").expect("notes.txt is written");

    let mut app = App::new(root);
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("a test terminal");
    wait_for(&mut terminal, &mut app, "hello");

    let editing = open_the_editor(&mut terminal, &mut app);
    assert!(
        editing.contains("Editing"),
        "entering the Edit view should say plainly which it is in: {editing}"
    );

    press(&mut terminal, &mut app, KeyCode::End);
    press(&mut terminal, &mut app, KeyCode::Char('!'));

    press_with_modifiers(
        &mut terminal,
        &mut app,
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    );
    wait_for(&mut terminal, &mut app, "hello!");

    let saved = std::fs::read_to_string(&path).expect("the file can be read back");
    assert_eq!(
        saved, "hello!",
        "the service should have written exactly what was typed"
    );
}

#[test]
fn leaving_the_editor_with_unsaved_changes_asks_first_and_declining_keeps_them() {
    let _serial = serially();
    common::ensure_service();
    let root = common::scratch("edit-discard-prompt");
    let path = root.join("notes.txt");
    std::fs::write(&path, "hello").expect("notes.txt is written");

    let mut app = App::new(root);
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("a test terminal");
    wait_for(&mut terminal, &mut app, "hello");
    open_the_editor(&mut terminal, &mut app);

    press(&mut terminal, &mut app, KeyCode::Char('!'));

    let prompted = press(&mut terminal, &mut app, KeyCode::Esc);
    assert!(
        prompted.contains("Discard changes?")
            && prompted.contains("notes.txt")
            && prompted.contains("y/n"),
        "leaving with unsaved changes should ask first, naming the file: {prompted}"
    );

    let declined = press(&mut terminal, &mut app, KeyCode::Char('n'));
    assert!(
        declined.contains("Editing"),
        "declining should stay in the editor: {declined}"
    );

    press_with_modifiers(
        &mut terminal,
        &mut app,
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    );
    wait_for(&mut terminal, &mut app, "!hello");

    let saved = std::fs::read_to_string(&path).expect("the file can be read back");
    assert_eq!(
        saved, "!hello",
        "declining to discard should have kept the typed character"
    );
}

#[test]
fn saving_a_file_that_is_no_longer_valid_utf8_is_refused_with_the_services_own_words() {
    let _serial = serially();
    common::ensure_service();
    let root = common::scratch("edit-save-non-utf8");
    let path = root.join("notes.txt");
    std::fs::write(&path, "hello").expect("notes.txt is written");

    let mut app = App::new(root);
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("a test terminal");
    wait_for(&mut terminal, &mut app, "hello");
    open_the_editor(&mut terminal, &mut app);
    press(&mut terminal, &mut app, KeyCode::Char('!'));

    // Something else changes the file underneath the edit: what the
    // service reads back before writing is no longer text at all. Reading
    // the same corrupted bytes here gives the exact words the service will
    // independently produce doing the same read, rather than guessing at
    // the standard library's wording.
    let corrupted: &[u8] = &[0xFF, 0xFE, 0x00, 0x01, 0x80];
    std::fs::write(&path, corrupted).expect("the file is corrupted on disk");
    let expected_message = std::fs::read_to_string(&path)
        .expect_err("the corrupted bytes are not valid UTF-8")
        .to_string();

    press_with_modifiers(
        &mut terminal,
        &mut app,
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    );
    let refused = wait_for(&mut terminal, &mut app, &expected_message);

    assert!(
        refused.contains(&expected_message),
        "the service's own refusal should reach the reader unchanged: {refused}"
    );
    assert_eq!(
        std::fs::read(&path).expect("the file can still be read"),
        corrupted,
        "a refused write must not have touched the file's bytes"
    );
}
