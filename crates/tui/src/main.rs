//! Command line entry point for the Ratatui front end.

use protocol::{Request, Response};
use ratatui::crossterm::event::{self, Event, KeyCode};
use std::env;
use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    let path = env::args().nth(1).unwrap_or_else(|| ".".to_owned());

    let socket_name = match protocol::socket_name() {
        Ok(name) => name,
        Err(err) => {
            eprintln!("could not resolve the service's socket name: {err}");
            return ExitCode::FAILURE;
        }
    };

    let request = Request::Open { path };
    let response = match tui::send_request(socket_name, &request) {
        Ok(response) => response,
        Err(err) => {
            eprintln!("could not reach the service (start it with `cargo run -p service`): {err}");
            return ExitCode::FAILURE;
        }
    };

    if let Response::Error { message } = &response {
        eprintln!("service reported an error: {message}");
        return ExitCode::FAILURE;
    }

    if let Err(err) = show(&response) {
        eprintln!("terminal error: {err}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

/// Renders `response` in the alternate screen until the user presses `q` or
/// Esc.
fn show(response: &Response) -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = loop {
        terminal.draw(|frame| tui::render(frame, frame.area(), response))?;
        match event::read()? {
            Event::Key(key) if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) => {
                break Ok(());
            }
            _ => {}
        }
    };
    ratatui::restore();
    result
}
