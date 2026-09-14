//! Command line entry point for the Slint front end: the three panes,
//! opening at this machine's Repos Directory.
//!
//! A path given on the command line still wins, for a one-off look and for
//! the test harness. That is not a session restore (decision D7): it is an
//! explicit instruction, given now, rather than a memory of where somebody
//! happened to be when they last closed the window.

use gui::app::App;
use gui::{MainWindow, sync_ui};
use slint::{ComponentHandle, Timer, TimerMode};
use std::cell::RefCell;
use std::env;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// How long to wait for a freshly spawned service to come up, and how often
/// to poll it while waiting.
const SERVICE_START_TIMEOUT: Duration = Duration::from_secs(2);
const SERVICE_START_POLL: Duration = Duration::from_millis(100);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if env::args().any(|arg| arg == "--self-update") {
        return self_update();
    }

    let explicit = env::args().nth(1).map(PathBuf::from);

    if let Err(err) = ensure_service_running() {
        eprintln!(
            "could not reach the service, and could not start it either \
             (place a `service` binary next to this one, or start it yourself with \
             `cargo run -p service`): {err}"
        );
        return Err(err.into());
    }

    // Where to open: the path given on the command line, or the configured
    // Repos Directory, or - on a first run - the platform's default, offered
    // rather than assumed.
    let opening = gui::app::opening();
    let ask_for_root = explicit.is_none() && opening.ask;
    let root = explicit.unwrap_or_else(|| opening.root.clone());

    let app = Rc::new(RefCell::new(App::new(root)));
    if ask_for_root {
        app.borrow_mut()
            .begin_repos_root_edit(&opening.root.to_string_lossy());
    }
    let ui = MainWindow::new()?;
    if let Some(widths) = gui::settings::load_pane_widths() {
        ui.set_folders_width(widths.folders);
        ui.set_contents_width(widths.contents);
    }
    sync_ui(&ui, &app.borrow());

    gui::wire_callbacks(&ui, &app);

    let timer = Timer::default();
    let tick_app = app.clone();
    let tick_ui = ui.as_weak();
    timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
        let mut app = tick_app.borrow_mut();
        app.tick();
        if let Some(ui) = tick_ui.upgrade() {
            sync_ui(&ui, &app);
        }
    });

    ui.run()?;
    // Written on the way out rather than on every drag: a splitter moves a
    // pixel at a time, and the layout only has to survive to the next run.
    gui::settings::save_pane_widths(gui::settings::PaneWidths {
        folders: ui.get_folders_width(),
        contents: ui.get_contents_width(),
    });
    Ok(())
}

/// Checks for and applies an update to this binary, per §4.2 of
/// GUIDANCE.md.
fn self_update() -> Result<(), Box<dyn std::error::Error>> {
    match updater::self_update("RepoSphereExplorerGui") {
        Ok(updater::Outcome::UpToDate { version }) => {
            println!("gui is up to date (v{version})");
            Ok(())
        }
        Ok(updater::Outcome::Updated { from, to }) => {
            println!("gui updated: v{from} -> v{to}");
            Ok(())
        }
        Err(err) => Err(err.into()),
    }
}

/// Connects to the service's local socket, spawning the service as a
/// detached background process first if nothing answers. See `tui`'s
/// `main.rs` for the identical approach and rationale.
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

fn try_connect() -> io::Result<interprocess::local_socket::Stream> {
    use interprocess::local_socket::traits::Stream as _;
    interprocess::local_socket::Stream::connect(protocol::socket_name()?)
}

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
