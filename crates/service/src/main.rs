//! Entry point for the service process.

use std::process::ExitCode;

fn main() -> ExitCode {
    if std::env::args().any(|arg| arg == "--self-update") {
        return self_update();
    }

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> std::io::Result<()> {
    let listener = service::bind(protocol::socket_name()?)?;
    eprintln!("listening on {}", protocol::SOCKET_NAME);
    service::run(&listener)
}

/// Checks for and applies an update to this binary, per §4.2 of
/// GUIDANCE.md.
fn self_update() -> ExitCode {
    match updater::self_update("service") {
        Ok(updater::Outcome::UpToDate { version }) => {
            println!("service is up to date (v{version})");
            ExitCode::SUCCESS
        }
        Ok(updater::Outcome::Updated { from, to }) => {
            println!("service updated: v{from} -> v{to}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("update failed: {err}");
            ExitCode::FAILURE
        }
    }
}
