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
use ratatui::crossterm::event::KeyCode;
use std::sync::{Mutex, MutexGuard};
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

/// Repeatedly draws with no key pressed until `needle` appears in the
/// buffer or a deadline passes, returning whatever was last drawn - the
/// real round trip to the service takes a moment to answer.
fn wait_for(terminal: &mut Terminal<TestBackend>, app: &mut App, needle: &str) -> String {
    let mut nothing = common::QueuedKeys::new(std::iter::empty());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
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
    // to its default help text once the root's own listing has answered.
    let contents = wait_for(&mut terminal, &mut app, "Tab: switch pane");

    assert!(contents.contains("Folders"), "{contents}");
    assert!(contents.contains("Contents"), "{contents}");
    assert!(contents.contains("File"), "{contents}");
    assert!(
        contents.contains("Tab: switch pane"),
        "the status line's help text should be drawn: {contents}"
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
        prompted.contains("Delete notes.txt? y/n"),
        "the confirmation prompt should be drawn: {prompted}"
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
