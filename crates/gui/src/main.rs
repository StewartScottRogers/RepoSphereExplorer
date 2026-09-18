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
    if env::args().any(|arg| arg == gui::renderer::PROBE_FLAG) {
        if let Err(err) = gui::renderer::probe() {
            eprintln!("{err}");
            std::process::exit(1);
        }
        return Ok(());
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
    // What "Open in editor" (#581) launches, decided once at startup rather
    // than on every timer tick: a settings-file read and a PATH search on
    // every tick would be real, pointless work for a fact that does not
    // change while the window is open.
    app.borrow_mut()
        .set_editor(gui::settings::load_editor(), gui::launch::on_path("code"));
    // Before the window exists: once it does, the renderer is fixed.
    eprintln!("{}", gui::renderer::select()?.message());
    let ui = MainWindow::new()?;
    // Before the window is shown: a free desktop reads the id off the
    // window as it is realised, and matches it to the desktop entry the
    // Linux install wrote.
    gui::name_the_window()?;
    if let Some(widths) = gui::settings::load_pane_widths() {
        ui.set_folders_width(widths.folders);
        ui.set_contents_width(widths.contents);
    }
    // The zoom step (#586), the same "restore before the window shows"
    // shape the pane widths above already have. A missing or invalid
    // saved value leaves `App` at `zoom::DEFAULT`, applied to `ui` by the
    // `sync_ui` call below.
    if let Some(percent) = gui::settings::load_zoom() {
        app.borrow_mut().set_zoom_percent(percent);
    }
    // Applied before the window is shown, the same as the pane widths
    // above. The wiring is in the library so a window test drives the same
    // code this does (rule 14).
    let geometry_tracker = gui::wire_window_geometry(&ui, gui::settings::load_window_geometry());
    sync_ui(&ui, &app.borrow());

    gui::wire_callbacks(&ui, &app);

    let timer = Timer::default();
    let tick_app = app.clone();
    let tick_ui = ui.as_weak();
    let tick_geometry_tracker = geometry_tracker.clone();
    timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
        let mut app = tick_app.borrow_mut();
        app.tick();
        if let Some(ui) = tick_ui.upgrade() {
            sync_ui(&ui, &app);
            gui::ask_for_visible_statuses(&ui, &mut app);
            gui::fit_pane_widths_to_window(&ui);
            gui::observe_window_geometry(&ui, &tick_geometry_tracker);
        }
    });

    ui.run()?;
    // Written on the way out rather than on every drag or move: the layout
    // only has to survive to the next run.
    gui::settings::save_pane_widths(gui::settings::PaneWidths {
        folders: ui.get_folders_width(),
        contents: ui.get_contents_width(),
    });
    if let Some(geometry) = gui::geometry_to_save(&ui, &geometry_tracker) {
        gui::settings::save_window_geometry(geometry);
    }
    gui::settings::save_zoom(app.borrow().zoom_percent());
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
        Ok(updater::Outcome::InsideAppImage { appimage }) => {
            println!("{}", updater::appimage_advice("gui", &appimage));
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
    let service = dir.join(service_name);
    let spawned = |breakaway: bool| {
        let mut command = std::process::Command::new(&service);
        command
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        detach(&mut command, breakaway);
        command.spawn().map(|_| ())
    };
    // A job object that does not allow breaking away - a continuous
    // integration runner's, and some terminals' and launchers' - refuses the
    // whole process rather than the one flag. A service that ends with the
    // job is better than none, so ask again without it.
    spawned(true).or_else(|err| {
        if cfg!(windows) {
            spawned(false)
        } else {
            Err(err)
        }
    })
}

#[cfg(windows)]
fn detach(command: &mut std::process::Command, breakaway: bool) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    let breakaway = if breakaway {
        CREATE_BREAKAWAY_FROM_JOB
    } else {
        0
    };
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | breakaway);
}

#[cfg(not(windows))]
fn detach(_command: &mut std::process::Command, _breakaway: bool) {}
