//! What the terminal integration suites share.
//!
//! Integration test files are separate binaries, so code they have in
//! common lives in a module each of them declares - the place Cargo does
//! not mistake for a test binary of its own. Mirrors
//! `crates/gui/tests/common/mod.rs`.

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Once;
use std::time::{Duration, Instant};
use tui::Events;
use tui::app::App;

/// Starts a service for this test binary, once, and waits until it answers.
///
/// On a socket of its own, named for this process, and bound or the test
/// fails - never a silent fallback to whatever already holds a socket. The
/// shared socket name, with that fallback, meant a test carried out real
/// file operations through another test binary's service, or through the
/// reader's own running Repos Explorer. See `protocol::use_private_socket`.
pub fn ensure_service() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        assert!(
            protocol::use_private_socket(format!(
                "reposphereexplorer-tui-test-{}.sock",
                std::process::id()
            )),
            "the socket was chosen before this harness could make it private"
        );
        let name = protocol::socket_name().expect("the platform has a socket name");
        let listener = service::bind(name).expect("a private socket is free to bind");
        std::thread::spawn(move || {
            let _ = service::run(&listener);
        });
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        use interprocess::local_socket::traits::Stream as _;
        let name = protocol::socket_name().expect("the platform has a socket name");
        if interprocess::local_socket::Stream::connect(name).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("no service answered on the local socket");
}

/// A directory of this test's own, under the system temp directory, emptied
/// first so an earlier run's leftovers cannot leak in.
pub fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("repos-explorer-tui-window-{name}"));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    directory
}

/// A fixed sequence of key presses, read one at a time by [`tui::tick`] or
/// [`tui::run`] in place of a real terminal's input.
pub struct QueuedKeys(VecDeque<KeyCode>);

impl QueuedKeys {
    /// Queues `codes`, in the order they will be read.
    pub fn new(codes: impl IntoIterator<Item = KeyCode>) -> Self {
        Self(codes.into_iter().collect())
    }
}

impl Events for QueuedKeys {
    fn poll(&mut self, _timeout: Duration) -> std::io::Result<bool> {
        Ok(!self.0.is_empty())
    }

    fn read(&mut self) -> std::io::Result<Event> {
        let code = self.0.pop_front().expect("poll said an event was ready");
        Ok(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }
}

/// Runs [`tui::tick`] with no event, repeatedly, giving a background
/// request time to answer over the private socket - what a test does
/// instead of reading `App`'s private pending-request state directly.
pub fn settle(terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, app: &mut App) {
    let mut nothing = QueuedKeys::new(std::iter::empty());
    for _ in 0..50 {
        tui::tick(terminal, app, &mut nothing, Duration::ZERO).expect("a draw");
        std::thread::sleep(Duration::from_millis(10));
    }
}
