//! Entry point for the service process.

use std::process::ExitCode;

fn main() -> ExitCode {
    if std::env::args().any(|arg| arg == updater::VERIFY_FLAG) {
        return ExitCode::SUCCESS;
    }
    if std::env::args().any(|arg| arg == "--self-update") {
        return self_update();
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(notice) = updater::startup_notice(&exe)
    {
        eprintln!("{notice}");
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
    let name = protocol::socket_name()?;
    eprintln!("listening on {name:?}");
    let listener = service::bind_reclaiming_stale(name)?;
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
        Ok(updater::Outcome::InsideAppImage { appimage }) => {
            println!("{}", updater::appimage_advice("service", &appimage));
            ExitCode::SUCCESS
        }
        Ok(updater::Outcome::RolledBack { attempted, to }) => {
            println!("service: update to v{attempted} did not start; rolled back to v{to}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("update failed: {err}");
            ExitCode::FAILURE
        }
    }
}
