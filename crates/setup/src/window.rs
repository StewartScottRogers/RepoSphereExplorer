//! The window a double-click opens.
//!
//! The work itself is [`crate::install`] and [`crate::uninstall`]; this is
//! the part a person watches. It runs on a worker thread so the window keeps
//! drawing, and every line those two report is appended to the window as it
//! happens - a setup program that vanishes without saying what it did is the
//! thing this exists to avoid.

use crate::{Destination, SetupError, Source};
use slint::ComponentHandle;
use std::path::PathBuf;
use std::sync::mpsc;

// Slint's generated component code (from build.rs, compiling ui/setup.slint)
// carries no doc comments; scope the exception to this module rather than
// the whole crate, as `crates/gui/src/lib.rs` does.
#[allow(missing_docs)]
mod generated {
    slint::include_modules!();
}
use generated::SetupWindow;

/// What the window was opened to do.
#[derive(Debug, Clone)]
pub enum Task {
    /// Install a release.
    Install {
        /// Where its files come from.
        source: Source,
        /// Where they go.
        destination: Destination,
        /// The target triple to install for.
        target: String,
    },
    /// Remove the install at this prefix.
    Uninstall(PathBuf),
}

impl Task {
    /// Runs the task, reporting each step.
    ///
    /// # Errors
    ///
    /// Whatever the install or the uninstall refused.
    pub fn run(&self, report: &mut crate::Report<'_>) -> Result<Option<PathBuf>, SetupError> {
        match self {
            Task::Install {
                source,
                destination,
                target,
            } => crate::install(source, destination, target, report)
                .map(|plan| Some(plan.application())),
            Task::Uninstall(prefix) => crate::uninstall(prefix, report).map(|()| None),
        }
    }

    /// The heading the window opens with.
    #[must_use]
    pub fn heading(&self) -> String {
        match self {
            Task::Install { .. } => "Install Repos Explorer".to_owned(),
            Task::Uninstall(_) => "Remove Repos Explorer".to_owned(),
        }
    }

    /// What the window says it is about to do, and where.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Task::Install {
                source,
                destination,
                ..
            } => format!(
                "{} will be downloaded, checked against its signed manifest, and installed \
                 in {}. Nothing is placed unless every file matches. No administrator \
                 rights are needed.",
                match source {
                    Source::Latest => "The latest release".to_owned(),
                    Source::Tag(tag) => format!("Release {tag}"),
                    Source::Directory(directory) =>
                        format!("The release in {}", directory.display()),
                },
                destination.prefix.display()
            ),
            Task::Uninstall(prefix) => format!(
                "Everything the install in {} placed will be removed: the files, the Start \
                 menu entry and the Settings > Apps entry. Your Repos Directory \
                 configuration and journal are left alone.",
                prefix.display()
            ),
        }
    }

    /// The button that starts it.
    #[must_use]
    pub fn go_label(&self) -> String {
        match self {
            Task::Install { .. } => "Install".to_owned(),
            Task::Uninstall(_) => "Remove".to_owned(),
        }
    }
}

/// One line of the account the window shows.
enum Step {
    /// Something that was done.
    Did(String),
    /// The end: the application to offer to start, or what went wrong.
    Ended(Result<Option<PathBuf>, String>),
}

/// Opens the window, does `task`, and stays open until it is closed, so the
/// outcome is read rather than guessed at.
///
/// An install waits for its button: the window is what the person who
/// double-clicked the file is shown first, and it says what is about to
/// happen and where. An uninstall does not wait, because the only way to it
/// is the Uninstall button in Settings, which has already asked.
///
/// # Errors
///
/// When Slint cannot open a window at all. What the task itself refused is
/// shown in the window and is not an error here: it has been reported.
pub fn show(task: &Task) -> Result<(), slint::PlatformError> {
    // The software renderer, always. This window draws four pieces of text
    // and three buttons, so nothing is lost by it, and a machine with no
    // working Open Graphics Library (OpenGL) driver - the case #556 was
    // filed for - would otherwise fail as the window is realised, which for
    // a setup program means vanishing before it has said anything.
    if std::env::var_os("SLINT_BACKEND").is_none() {
        slint::BackendSelector::new()
            .backend_name("winit-software".into())
            .select()?;
    }

    let ui = SetupWindow::new()?;
    ui.set_heading(task.heading().into());
    ui.set_summary(task.summary().into());
    ui.set_go_label(task.go_label().into());

    let (sender, receiver) = mpsc::channel::<Step>();
    let go_task = task.clone();
    let go_ui = ui.as_weak();
    ui.on_go(move || {
        let Some(ui) = go_ui.upgrade() else { return };
        ui.set_busy(true);
        ui.set_progress("working...\n".into());
        let sender = sender.clone();
        let task = go_task.clone();
        std::thread::spawn(move || {
            let report_sender = sender.clone();
            let outcome = task.run(&mut |line| {
                let _ = report_sender.send(Step::Did(line));
            });
            let _ = sender.send(Step::Ended(outcome.map_err(|err| err.to_string())));
        });
    });

    if matches!(task, Task::Uninstall(_)) {
        ui.set_ask(false);
        ui.invoke_go();
    }

    let launch_ui = ui.as_weak();
    ui.on_launch(move || {
        let Some(ui) = launch_ui.upgrade() else {
            return;
        };
        let application = PathBuf::from(ui.get_application().to_string());
        if let Some(directory) = application.parent() {
            let _ = std::process::Command::new(&application)
                .current_dir(directory)
                .spawn();
        }
        let _ = ui.hide();
    });

    let close_ui = ui.as_weak();
    ui.on_close_window(move || {
        if let Some(ui) = close_ui.upgrade() {
            let _ = ui.hide();
        }
    });

    // The worker thread talks to the window through the channel this drains,
    // rather than reaching into it: Slint's components belong to the thread
    // that made them.
    let tick_ui = ui.as_weak();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(50),
        move || {
            let Some(ui) = tick_ui.upgrade() else { return };
            while let Ok(step) = receiver.try_recv() {
                match step {
                    Step::Did(line) => {
                        ui.set_progress(format!("{}{line}\n", ui.get_progress()).into());
                    }
                    Step::Ended(Ok(application)) => {
                        ui.set_busy(false);
                        ui.set_finished(true);
                        if let Some(application) = application {
                            ui.set_application(application.display().to_string().into());
                            ui.set_can_launch(true);
                        }
                    }
                    Step::Ended(Err(message)) => {
                        ui.set_busy(false);
                        ui.set_finished(true);
                        ui.set_failed(true);
                        ui.set_progress(format!("{}\n{message}\n", ui.get_progress()).into());
                    }
                }
            }
        },
    );

    ui.run()
}
