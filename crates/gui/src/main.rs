//! Command line entry point for the Slint front end: the three-pane
//! explorer, rooted at an optional path argument (default `.`).

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

    let root = env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);

    if let Err(err) = ensure_service_running() {
        eprintln!(
            "could not reach the service, and could not start it either \
             (place a `service` binary next to this one, or start it yourself with \
             `cargo run -p service`): {err}"
        );
        return Err(err.into());
    }

    let app = Rc::new(RefCell::new(App::new(root)));
    let ui = MainWindow::new()?;
    sync_ui(&ui, &app.borrow());

    wire_callbacks(&ui, &app);

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
    Ok(())
}

/// Checks for and applies an update to this binary, per §4.2 of
/// GUIDANCE.md.
fn self_update() -> Result<(), Box<dyn std::error::Error>> {
    match updater::self_update("gui") {
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

fn wire_callbacks(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    let index = |i: i32| usize::try_from(i).unwrap_or(usize::MAX);

    macro_rules! on_row_event {
        ($setter:ident, $method:ident) => {{
            let app = app.clone();
            let ui_weak = ui.as_weak();
            ui.$setter(move |i| {
                let mut app = app.borrow_mut();
                app.$method(index(i));
                if let Some(ui) = ui_weak.upgrade() {
                    sync_ui(&ui, &app);
                }
            });
        }};
    }

    on_row_event!(on_folder_row_clicked, select_folder);
    on_row_event!(on_folder_row_double_clicked, toggle_folder);
    on_row_event!(on_content_row_clicked, select_content);
    on_row_event!(on_content_row_double_clicked, open_content);

    macro_rules! on_event {
        ($setter:ident, $method:ident) => {{
            let app = app.clone();
            let ui_weak = ui.as_weak();
            ui.$setter(move || {
                let mut app = app.borrow_mut();
                app.$method();
                if let Some(ui) = ui_weak.upgrade() {
                    sync_ui(&ui, &app);
                }
            });
        }};
    }

    on_event!(on_cancel_requested, cancel_pending);
    on_event!(on_delete_requested, request_delete);
    on_event!(on_return_pressed, handle_return);
    on_event!(on_backspace_pressed, backspace);

    let text_app = app.clone();
    let text_ui = ui.as_weak();
    ui.on_key_text(move |text| {
        let mut app = text_app.borrow_mut();
        app.handle_key_text(&text);
        if let Some(ui) = text_ui.upgrade() {
            sync_ui(&ui, &app);
        }
    });
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
