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

/// How long to wait for a freshly spawned service to come up, and how often
/// to poll it while waiting.
const SERVICE_START_TIMEOUT: Duration = Duration::from_secs(2);
const SERVICE_START_POLL: Duration = Duration::from_millis(100);

fn main() -> ExitCode {
    if env::args().any(|arg| arg == "--self-update") {
        return self_update();
    }

    let root = env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);

    if let Err(err) = ensure_service_running() {
        eprintln!(
            "could not reach the service, and could not start it either \
             (place a `service` binary next to this one, or start it yourself with \
             `cargo run -p service`): {err}"
        );
        return ExitCode::FAILURE;
    }

    if let Err(err) = run(root) {
        eprintln!("terminal error: {err}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

/// Connects to the service's local socket, spawning the service as a
/// detached background process first if nothing answers. The spawned
/// service outlives this front end: per §2 of GUIDANCE.md, "one service
/// instance may serve both front ends at once", so later launches of `tui`
/// (or `gui`, once it exists) just connect to the same one.
fn ensure_service_running() -> io::Result<()> {
    if try_connect().is_ok() {
        return Ok(());
    }

    spawn_service()?;

    let deadline = std::time::Instant::now() + SERVICE_START_TIMEOUT;
    while std::time::Instant::now() < deadline {
        std::thread::sleep(SERVICE_START_POLL);
        if try_connect().is_ok() {
            return Ok(());
        }
    }
    try_connect().map(|_| ())
}

fn try_connect() -> io::Result<Stream> {
    Stream::connect(protocol::socket_name()?)
}

/// Launches the `service` binary expected to sit next to this one, detached
/// so it keeps running after this process exits.
fn spawn_service() -> io::Result<()> {
    let exe = env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| io::Error::other("the running binary has no parent directory"))?;
    let service_name = if cfg!(windows) {
        "service.exe"
    } else {
        "service"
    };
    let mut command = std::process::Command::new(dir.join(service_name));
    command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    detach(&mut command);
    command.spawn()?;
    Ok(())
}

/// Detaches `command`'s future child from this process's console and job
/// object, so it outlives this process rather than being torn down with it
/// (a plain `spawn` inherits both on Windows).
#[cfg(windows)]
fn detach(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
}

#[cfg(not(windows))]
fn detach(_command: &mut std::process::Command) {}

/// Checks for and applies an update to this binary, per §4.2 of
/// GUIDANCE.md.
fn self_update() -> ExitCode {
    match updater::self_update("RepoSphereExplorerTui") {
        Ok(updater::Outcome::UpToDate { version }) => {
            println!("RepoSphereExplorerTui is up to date (v{version})");
            ExitCode::SUCCESS
        }
        Ok(updater::Outcome::Updated { from, to }) => {
            println!("RepoSphereExplorerTui updated: v{from} -> v{to}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("update failed: {err}");
            ExitCode::FAILURE
        }
    }
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
