//! Driving the "Go to Repository" switcher, Find, and All Repositories
//! (#647) through the real terminal event loop, wired the way `main` wires
//! it - the same shape `terminal_in_the_window.rs` uses (CLAUDE.md rule 14):
//! a real `App` talking to a real service on this test binary's private
//! socket, asserted on the buffer it draws and on the listing it lands on,
//! rather than on `App`'s own key-handling methods alone.

mod common;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use std::sync::{Mutex, MutexGuard};
use tui::Events;
use tui::app::App;

/// One service at a time (see `terminal_in_the_window.rs`'s own `SERIAL`):
/// a burst of connections from several tests at once starves some of them.
static SERIAL: Mutex<()> = Mutex::new(());

fn serially() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn drawn(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

/// One key event with modifiers, read once - `common::QueuedKeys` only
/// carries a bare [`KeyCode`], which cannot express a Ctrl+P.
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

fn press(terminal: &mut Terminal<TestBackend>, app: &mut App, code: KeyCode) -> String {
    press_with_modifiers(terminal, app, code, KeyModifiers::NONE)
}

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

fn wait_for(terminal: &mut Terminal<TestBackend>, app: &mut App, needle: &str) -> String {
    let mut nothing = common::QueuedKeys::new(std::iter::empty());
    // Generous rather than tight (#741): the loop returns the moment
    // `needle` appears, so a longer deadline costs nothing while the real
    // round trip to the service and to `git` is working.
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

fn git_init(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).expect("a repository directory");
    let made = std::process::Command::new("git")
        .args(["init", "--quiet", "--initial-branch", "main", "."])
        .current_dir(dir)
        .status()
        .expect("git should be on PATH");
    assert!(made.success(), "git init should make the fixture");
}

#[test]
fn ctrl_p_switcher_ranks_like_the_graphical_front_end_and_enter_moves_the_listing() {
    let _serial = serially();
    common::ensure_service();
    let root = common::scratch("switcher-window");
    for name in ["TankSwarmCode", "atscode", "xtxsxc"] {
        git_init(&root.join(name));
    }

    let mut app = App::new(root.clone());
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("a test terminal");
    common::settle(&mut terminal, &mut app);

    let opened = press_with_modifiers(
        &mut terminal,
        &mut app,
        KeyCode::Char('p'),
        KeyModifiers::CONTROL,
    );
    assert!(
        opened.contains("Go to Repository"),
        "Ctrl+P should open the switcher: {opened}"
    );

    let mut typed = opened;
    for c in "tsc".chars() {
        typed = press(&mut terminal, &mut app, KeyCode::Char(c));
    }
    // The word-start match should be highlighted first, the same ranking
    // `switcher::score` gives the graphical front end's own switcher.
    assert!(
        typed.contains("TankSwarmCode"),
        "the word-start match should be shown: {typed}"
    );

    let closed = press(&mut terminal, &mut app, KeyCode::Enter);
    assert!(
        !closed.contains("Go to Repository"),
        "Enter should have closed the switcher: {closed}"
    );
    common::settle(&mut terminal, &mut app);
    assert_eq!(
        app.selected_content_name(),
        Some("TankSwarmCode"),
        "Enter should have moved the listing's selection to the top match"
    );
}

/// `Request::FindNames` reads the machine's own configured Repos
/// Directory rather than taking one as a parameter (see
/// `crates/gui/tests/certificates_in_the_window.rs`'s own note on
/// `Request::FindCertificates`, which reads it the same way): a real
/// end-to-end walk through that shared, process-wide config is not what a
/// window test can afford to depend on without risking flakiness against
/// whatever else is running. So this proves the prompt itself - opening,
/// typing, Escape leaving the listing exactly as it was - and
/// `app::tests::find_shows_results_with_their_repository_and_truncated_flag_and_enter_goes_there`
/// proves what a real answer does with a hand-built one instead.
#[test]
fn ctrl_f_opens_find_and_escape_leaves_the_listing_exactly_as_it_was() {
    let _serial = serially();
    common::ensure_service();
    let root = common::scratch("find-window");
    git_init(&root.join("repo-one"));

    let mut app = App::new(root.clone());
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("a test terminal");
    common::settle(&mut terminal, &mut app);
    let before = app.selected_content_name().map(str::to_owned);

    let opened = press_with_modifiers(
        &mut terminal,
        &mut app,
        KeyCode::Char('f'),
        KeyModifiers::CONTROL,
    );
    assert!(opened.contains("Find"), "Ctrl+F should open Find: {opened}");

    let typed = press(&mut terminal, &mut app, KeyCode::Char('n'));
    assert!(
        typed.contains('n'),
        "the typed query should be drawn: {typed}"
    );

    let closed = press(&mut terminal, &mut app, KeyCode::Esc);
    assert!(
        !closed.contains("Find (Enter/Esc)"),
        "Escape should have closed the prompt: {closed}"
    );
    assert_eq!(
        app.selected_content_name(),
        before.as_deref(),
        "and left the listing exactly as it was"
    );
}

#[test]
fn ctrl_r_all_repositories_lists_a_nested_working_copy_and_refreshes() {
    let _serial = serially();
    common::ensure_service();
    let root = common::scratch("all-repositories-window");
    git_init(&root.join("group").join("nested-repo"));

    let mut app = App::new(root.clone());
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("a test terminal");
    common::settle(&mut terminal, &mut app);

    let opened = press_with_modifiers(
        &mut terminal,
        &mut app,
        KeyCode::Char('r'),
        KeyModifiers::CONTROL,
    );
    assert!(
        opened.contains("All Repositories"),
        "Ctrl+R should open All Repositories: {opened}"
    );

    let settled = wait_for(&mut terminal, &mut app, "nested-repo");
    assert!(
        settled.contains("group"),
        "the nested repository's location below the root should be shown: {settled}"
    );

    let refreshed_view = press(&mut terminal, &mut app, KeyCode::F(5));
    assert!(
        refreshed_view.contains("All Repositories"),
        "F5 should keep the view open while it scans again: {refreshed_view}"
    );
    let refreshed = wait_for(&mut terminal, &mut app, "nested-repo");
    assert!(refreshed.contains("group"), "{refreshed}");

    let closed = press(&mut terminal, &mut app, KeyCode::Esc);
    assert!(
        !closed.contains("All Repositories"),
        "Escape should leave the listing exactly as it was: {closed}"
    );
}
