//! Command line entry point for the Ratatui front end: the three-pane
//! explorer, rooted at an optional path argument (default `.`).

use interprocess::local_socket::Stream;
use interprocess::local_socket::traits::Stream as _;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use std::env;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;
use tui::app::{App, render_app};

fn main() -> ExitCode {
    let root = env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);

    if let Err(err) = check_service_reachable() {
        eprintln!("could not reach the service (start it with `cargo run -p service`): {err}");
        return ExitCode::FAILURE;
    }

    if let Err(err) = run(root) {
        eprintln!("terminal error: {err}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

/// Confirms the service's socket accepts a connection before the terminal
/// is taken over, so a dead service fails with a plain message instead of
/// an alternate-screen app that can never load anything.
fn check_service_reachable() -> io::Result<()> {
    let name = protocol::socket_name()?;
    Stream::connect(name)?;
    Ok(())
}

/// Runs the three-pane explorer's event loop until the user quits.
fn run(root: PathBuf) -> io::Result<()> {
    let mut app = App::new(root);
    let mut terminal = ratatui::init();
    while !app.should_quit {
        terminal.draw(|frame| render_app(frame, frame.area(), &app))?;
        app.tick();
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key.code);
        }
    }
    ratatui::restore();
    Ok(())
}
