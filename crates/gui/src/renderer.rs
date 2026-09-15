//! Which renderer the window draws with (#556).
//!
//! Slint's default renderer draws through the Open Graphics Library
//! (OpenGL). On a Windows machine without a working OpenGL driver - a
//! graphics-processor-less continuous integration runner, a remote desktop
//! session, some virtual machines - it fails as the window is realised,
//! inside the event loop, with "Failed to initialize OpenGL driver: Could
//! not locate glCreateShader symbol". By then the choice cannot be undone in
//! the same process: Slint's platform is set once per thread, and winit
//! builds one event loop per process.
//!
//! So the choice is made before Slint starts. A short-lived copy of this
//! binary, run with [`PROBE_FLAG`], realises a hidden window with the
//! default renderer and exits; if it fails, this process selects the
//! software renderer, which is compiled in, before it creates its window.
//! The window stays in the process that was launched, so whatever started
//! the application still sees that process own it. `SLINT_BACKEND`, when
//! set, is an explicit instruction and wins over all of this.

use crate::MainWindow;
use std::process::{Command, Stdio};
use std::time::Duration;

/// The command-line flag that turns this binary into a renderer probe.
pub const PROBE_FLAG: &str = "--probe-renderer";

/// Slint's own environment variable naming a backend and renderer.
const SLINT_BACKEND: &str = "SLINT_BACKEND";

/// The renderer this run uses, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// `SLINT_BACKEND` names one; Slint reads the variable itself.
    FromEnvironment(String),
    /// Slint's default renderer. `probed` says whether it was seen to start
    /// on this machine, or was not checked on this platform.
    Default {
        /// The probe ran and the default renderer started.
        probed: bool,
    },
    /// The software renderer, because the default one could not start.
    Software {
        /// What the default renderer reported.
        reason: String,
    },
}

impl Choice {
    /// The one line written to the error output naming the renderer and
    /// the reason for it.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::FromEnvironment(value) => {
                format!("renderer: {value}, as SLINT_BACKEND asks")
            }
            Self::Default { probed: true } => {
                "renderer: default, it started on this machine".to_string()
            }
            Self::Default { probed: false } => {
                "renderer: default, SLINT_BACKEND is not set".to_string()
            }
            Self::Software { reason } => {
                format!("renderer: software, the default renderer could not start: {reason}")
            }
        }
    }
}

/// Decides the renderer. `slint_backend` is the value of `SLINT_BACKEND`, if
/// any; `probe` tries the default renderer and is called only when nothing
/// overrides it, returning `None` where this platform is not probed.
pub fn choose(
    slint_backend: Option<&str>,
    probe: impl FnOnce() -> Option<Result<(), String>>,
) -> Choice {
    // Slint treats an empty value as unset, and so does this.
    if let Some(value) = slint_backend.map(str::trim).filter(|v| !v.is_empty()) {
        return Choice::FromEnvironment(value.to_string());
    }
    match probe() {
        None => Choice::Default { probed: false },
        Some(Ok(())) => Choice::Default { probed: true },
        Some(Err(reason)) => Choice::Software { reason },
    }
}

/// Decides the renderer for this process and puts the decision into
/// effect, before any window exists. Returns the decision, for its
/// [`Choice::message`].
///
/// The probe runs on Windows only: that is where the default renderer's
/// setup can appear to succeed without a working driver and then fail, as
/// Slint's own source notes beside the check that raises the error.
///
/// # Errors
///
/// When Slint refuses the software renderer.
pub fn select() -> Result<Choice, slint::PlatformError> {
    let slint_backend = std::env::var(SLINT_BACKEND).ok();
    let choice = choose(slint_backend.as_deref(), || {
        cfg!(windows).then(probe_in_child)
    });
    if matches!(choice, Choice::Software { .. }) {
        slint::BackendSelector::new()
            .backend_name("winit-software".into())
            .select()?;
    }
    Ok(choice)
}

/// Runs this binary again with [`PROBE_FLAG`] and reports whether the
/// default renderer started there. The error is the probe's last line of
/// error output, which is Slint's own message.
fn probe_in_child() -> Result<(), String> {
    let exe = std::env::current_exe()
        .map_err(|err| format!("could not find this binary to probe with: {err}"))?;
    let output = Command::new(exe)
        .arg(PROBE_FLAG)
        .env_remove(SLINT_BACKEND)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| format!("could not run the renderer probe: {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(stderr
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map_or_else(
            || format!("the renderer probe exited with {}", output.status),
            str::to_string,
        ))
}

/// The probe itself: realises the main window, hidden, with the default
/// renderer, and leaves the event loop at once. Slint creates every window
/// that exists when the event loop starts, shown or not, and that creation
/// is where the default renderer sets up OpenGL.
///
/// # Errors
///
/// Whatever the renderer reports.
pub fn probe() -> Result<(), slint::PlatformError> {
    let _window = MainWindow::new()?;
    slint::Timer::single_shot(Duration::ZERO, || {
        let _ = slint::quit_event_loop();
    });
    slint::run_event_loop()
}

#[cfg(test)]
mod tests {
    use super::{Choice, choose};

    #[test]
    fn slint_backend_is_respected_and_nothing_is_probed() {
        let choice = choose(Some("winit-femtovg"), || {
            panic!("an explicit SLINT_BACKEND must not be second-guessed")
        });
        assert_eq!(choice, Choice::FromEnvironment("winit-femtovg".into()));
        assert_eq!(
            choice.message(),
            "renderer: winit-femtovg, as SLINT_BACKEND asks"
        );
    }

    #[test]
    fn slint_backend_naming_software_is_respected_too() {
        assert_eq!(
            choose(Some("winit-software"), || panic!("not probed")),
            Choice::FromEnvironment("winit-software".into())
        );
    }

    #[test]
    fn an_empty_slint_backend_counts_as_unset() {
        assert_eq!(
            choose(Some("  "), || Some(Ok(()))),
            Choice::Default { probed: true }
        );
    }

    #[test]
    fn a_machine_whose_default_renderer_starts_keeps_it() {
        let choice = choose(None, || Some(Ok(())));
        assert_eq!(choice, Choice::Default { probed: true });
        assert_eq!(
            choice.message(),
            "renderer: default, it started on this machine"
        );
    }

    #[test]
    fn a_default_renderer_that_cannot_start_falls_back_to_software() {
        let failure = "Failed to initialize OpenGL driver: Could not locate glCreateShader symbol";
        let choice = choose(None, || Some(Err(failure.to_string())));
        assert_eq!(
            choice,
            Choice::Software {
                reason: failure.into()
            }
        );
        assert_eq!(
            choice.message(),
            format!("renderer: software, the default renderer could not start: {failure}")
        );
    }

    #[test]
    fn a_platform_that_is_not_probed_keeps_the_default() {
        let choice = choose(None, || None);
        assert_eq!(choice, Choice::Default { probed: false });
        assert_eq!(
            choice.message(),
            "renderer: default, SLINT_BACKEND is not set"
        );
    }
}
