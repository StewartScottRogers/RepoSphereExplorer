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
//!
//! That probe costs about a third of a second, measured on a release build,
//! which is more than the window takes to appear otherwise. So its answer is
//! remembered beside the pane widths, stamped with the size and modification
//! time of the binary that was probed: a launch of the same build trusts the
//! remembered answer, and a new build, a missing file or a damaged one
//! probes again.

use crate::MainWindow;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, UNIX_EPOCH};

/// The command-line flag that turns this binary into a renderer probe.
pub const PROBE_FLAG: &str = "--probe-renderer";

/// Slint's own environment variable naming a backend and renderer.
const SLINT_BACKEND: &str = "SLINT_BACKEND";

/// Which binary a remembered answer was probed with. Two numbers the
/// filesystem already keeps, rather than reading megabytes to hash them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    /// Size of the executable, in bytes.
    pub size: u64,
    /// Modification time of the executable, in seconds since the epoch.
    pub modified: u64,
}

/// What the probe found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The default renderer started.
    Started,
    /// It did not, and said this.
    Failed(String),
}

/// A probe's answer from an earlier run, and the binary it answered for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remembered {
    /// The executable that was probed.
    pub stamp: Stamp,
    /// What that probe found.
    pub outcome: Outcome,
}

/// Where a renderer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A probe run just now.
    Probed,
    /// A probe run by an earlier launch of this same build.
    Remembered,
    /// Nothing was probed: this platform does not need it.
    NotProbed,
}

/// The renderer this run uses, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// `SLINT_BACKEND` names one; Slint reads the variable itself.
    FromEnvironment(String),
    /// Slint's default renderer.
    Default {
        /// Where that answer came from.
        source: Source,
    },
    /// The software renderer, because the default one could not start.
    Software {
        /// What the default renderer reported.
        reason: String,
        /// Where that answer came from.
        source: Source,
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
            Self::Default {
                source: Source::Probed,
            } => "renderer: default, it started on this machine".to_string(),
            Self::Default {
                source: Source::Remembered,
            } => "renderer: default, as remembered from an earlier run of this build".to_string(),
            Self::Default {
                source: Source::NotProbed,
            } => "renderer: default, SLINT_BACKEND is not set".to_string(),
            Self::Software {
                reason,
                source: Source::Remembered,
            } => format!(
                "renderer: software, remembered from an earlier run of this build, \
                 where the default renderer could not start: {reason}"
            ),
            Self::Software { reason, .. } => {
                format!("renderer: software, the default renderer could not start: {reason}")
            }
        }
    }

    /// What this run learned and should remember, or `None` when it learned
    /// nothing new.
    #[must_use]
    pub fn learned(&self) -> Option<Outcome> {
        match self {
            Self::Default {
                source: Source::Probed,
            } => Some(Outcome::Started),
            Self::Software {
                reason,
                source: Source::Probed,
            } => Some(Outcome::Failed(reason.clone())),
            _ => None,
        }
    }
}

/// Decides the renderer.
///
/// `slint_backend` is the value of `SLINT_BACKEND`, if any, and wins over
/// everything else. `stamp` is this executable as it is now, and
/// `remembered` what an earlier run wrote down; they are trusted only while
/// they describe the same binary. `probe` tries the default renderer, is
/// called only when nothing else answers, and returns `None` where this
/// platform is not probed.
pub fn choose(
    slint_backend: Option<&str>,
    stamp: Option<Stamp>,
    remembered: Option<Remembered>,
    probe: impl FnOnce() -> Option<Result<(), String>>,
) -> Choice {
    // Slint treats an empty value as unset, and so does this.
    if let Some(value) = slint_backend.map(str::trim).filter(|v| !v.is_empty()) {
        return Choice::FromEnvironment(value.to_string());
    }
    if let Some(remembered) = remembered.filter(|r| Some(r.stamp) == stamp) {
        return match remembered.outcome {
            Outcome::Started => Choice::Default {
                source: Source::Remembered,
            },
            Outcome::Failed(reason) => Choice::Software {
                reason,
                source: Source::Remembered,
            },
        };
    }
    match probe() {
        None => Choice::Default {
            source: Source::NotProbed,
        },
        Some(Ok(())) => Choice::Default {
            source: Source::Probed,
        },
        Some(Err(reason)) => Choice::Software {
            reason,
            source: Source::Probed,
        },
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
    let stamp = current_stamp();
    let choice = choose(slint_backend.as_deref(), stamp, load(), || {
        cfg!(windows).then(probe_in_child)
    });
    if let (Some(stamp), Some(outcome)) = (stamp, choice.learned()) {
        save(&Remembered { stamp, outcome });
    }
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

/// Size and modification time of the running binary, or `None` where the
/// filesystem will not say - in which case nothing is remembered and every
/// launch probes.
fn current_stamp() -> Option<Stamp> {
    let metadata = std::fs::metadata(std::env::current_exe().ok()?).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(Stamp {
        size: metadata.len(),
        modified,
    })
}

/// `<data-local-dir>/RepoSphereExplorer/renderer.json`, beside the window's
/// other settings, or `None` where the platform reports no such directory.
fn remembered_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("RepoSphereExplorer").join("renderer.json"))
}

/// Reads back what an earlier run wrote. A missing, unreadable or malformed
/// file is not an error worth reporting: it means the probe runs again.
fn load() -> Option<Remembered> {
    let text = std::fs::read_to_string(remembered_path()?).ok()?;
    remembered_from(&serde_json::from_str(&text).ok()?)
}

/// The remembered answer held in `value`, or `None` if it is not all there.
fn remembered_from(value: &serde_json::Value) -> Option<Remembered> {
    let stamp = Stamp {
        size: value.get("executable_size")?.as_u64()?,
        modified: value.get("executable_modified")?.as_u64()?,
    };
    let outcome = match value.get("default_renderer")?.as_str()? {
        "started" => Outcome::Started,
        "failed" => Outcome::Failed(
            value
                .get("reason")?
                .as_str()
                .filter(|reason| !reason.is_empty())?
                .to_string(),
        ),
        _ => return None,
    };
    Some(Remembered { stamp, outcome })
}

/// Writes down what the probe found, creating the directory if needed.
/// Best-effort: a launch that cannot remember simply probes again next
/// time.
fn save(remembered: &Remembered) {
    let Some(path) = remembered_path() else {
        return;
    };
    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return;
    }
    if let Ok(text) = serde_json::to_string_pretty(&value_for(remembered)) {
        let _ = std::fs::write(&path, text);
    }
}

/// `remembered` as the file holds it. The other half of
/// [`remembered_from`], so that a test can put one through both.
fn value_for(remembered: &Remembered) -> serde_json::Value {
    let mut value = serde_json::json!({
        "executable_size": remembered.stamp.size,
        "executable_modified": remembered.stamp.modified,
        "default_renderer": match remembered.outcome {
            Outcome::Started => "started",
            Outcome::Failed(_) => "failed",
        },
    });
    if let Outcome::Failed(reason) = &remembered.outcome {
        value["reason"] = serde_json::Value::String(reason.clone());
    }
    value
}

#[cfg(test)]
mod tests {
    use super::{Choice, Outcome, Remembered, Source, Stamp, choose, remembered_from, value_for};

    /// The binary as this run finds it.
    const THIS_BUILD: Stamp = Stamp {
        size: 90_000_000,
        modified: 1_760_000_000,
    };

    /// The same path, rebuilt since.
    const LAST_BUILD: Stamp = Stamp {
        size: 89_000_000,
        modified: 1_750_000_000,
    };

    /// What a machine without OpenGL reports.
    const FAILURE: &str =
        "Failed to initialize OpenGL driver: Could not locate glCreateShader symbol";

    fn remembering(stamp: Stamp, outcome: Outcome) -> Remembered {
        Remembered { stamp, outcome }
    }

    #[test]
    fn slint_backend_is_respected_and_nothing_is_probed() {
        let choice = choose(Some("winit-femtovg"), Some(THIS_BUILD), None, || {
            panic!("an explicit SLINT_BACKEND must not be second-guessed")
        });
        assert_eq!(choice, Choice::FromEnvironment("winit-femtovg".into()));
        assert_eq!(
            choice.message(),
            "renderer: winit-femtovg, as SLINT_BACKEND asks"
        );
        assert_eq!(choice.learned(), None, "nothing was probed to remember");
    }

    #[test]
    fn slint_backend_beats_a_remembered_answer() {
        let choice = choose(
            Some("winit-femtovg"),
            Some(THIS_BUILD),
            Some(remembering(THIS_BUILD, Outcome::Failed(FAILURE.into()))),
            || panic!("not probed"),
        );
        assert_eq!(choice, Choice::FromEnvironment("winit-femtovg".into()));
    }

    #[test]
    fn an_empty_slint_backend_counts_as_unset() {
        assert_eq!(
            choose(Some("  "), Some(THIS_BUILD), None, || Some(Ok(()))),
            Choice::Default {
                source: Source::Probed
            }
        );
    }

    #[test]
    fn a_machine_whose_default_renderer_starts_keeps_it() {
        let choice = choose(None, Some(THIS_BUILD), None, || Some(Ok(())));
        assert_eq!(
            choice,
            Choice::Default {
                source: Source::Probed
            }
        );
        assert_eq!(
            choice.message(),
            "renderer: default, it started on this machine"
        );
        assert_eq!(choice.learned(), Some(Outcome::Started));
    }

    #[test]
    fn a_default_renderer_that_cannot_start_falls_back_to_software() {
        let choice = choose(None, Some(THIS_BUILD), None, || Some(Err(FAILURE.into())));
        assert_eq!(
            choice,
            Choice::Software {
                reason: FAILURE.into(),
                source: Source::Probed
            }
        );
        assert_eq!(
            choice.message(),
            format!("renderer: software, the default renderer could not start: {FAILURE}")
        );
        assert_eq!(choice.learned(), Some(Outcome::Failed(FAILURE.into())));
    }

    #[test]
    fn a_platform_that_is_not_probed_keeps_the_default() {
        let choice = choose(None, Some(THIS_BUILD), None, || None);
        assert_eq!(
            choice,
            Choice::Default {
                source: Source::NotProbed
            }
        );
        assert_eq!(
            choice.message(),
            "renderer: default, SLINT_BACKEND is not set"
        );
        assert_eq!(choice.learned(), None);
    }

    #[test]
    fn a_remembered_answer_for_this_build_is_used_without_probing() {
        let choice = choose(
            None,
            Some(THIS_BUILD),
            Some(remembering(THIS_BUILD, Outcome::Started)),
            || panic!("a remembered answer must not cost another probe"),
        );
        assert_eq!(
            choice,
            Choice::Default {
                source: Source::Remembered
            }
        );
        assert_eq!(
            choice.message(),
            "renderer: default, as remembered from an earlier run of this build"
        );
        assert_eq!(choice.learned(), None, "already written down");
    }

    #[test]
    fn a_remembered_failure_opens_the_window_without_probing_either() {
        let choice = choose(
            None,
            Some(THIS_BUILD),
            Some(remembering(THIS_BUILD, Outcome::Failed(FAILURE.into()))),
            || panic!("a remembered answer must not cost another probe"),
        );
        assert_eq!(
            choice,
            Choice::Software {
                reason: FAILURE.into(),
                source: Source::Remembered
            }
        );
        assert_eq!(
            choice.message(),
            format!(
                "renderer: software, remembered from an earlier run of this build, \
                 where the default renderer could not start: {FAILURE}"
            )
        );
    }

    #[test]
    fn an_answer_remembered_for_another_build_is_probed_again() {
        let choice = choose(
            None,
            Some(THIS_BUILD),
            Some(remembering(LAST_BUILD, Outcome::Failed(FAILURE.into()))),
            || Some(Ok(())),
        );
        assert_eq!(
            choice,
            Choice::Default {
                source: Source::Probed
            },
            "an update can bring the driver with it"
        );
    }

    #[test]
    fn nothing_remembered_means_a_probe() {
        assert_eq!(
            choose(None, Some(THIS_BUILD), None, || Some(Ok(()))),
            Choice::Default {
                source: Source::Probed
            }
        );
    }

    #[test]
    fn an_unstampable_binary_probes_every_time() {
        let choice = choose(
            None,
            None,
            Some(remembering(THIS_BUILD, Outcome::Started)),
            || Some(Ok(())),
        );
        assert_eq!(
            choice,
            Choice::Default {
                source: Source::Probed
            }
        );
    }

    #[test]
    fn a_written_answer_reads_back_as_it_was_written() {
        for outcome in [Outcome::Started, Outcome::Failed(FAILURE.into())] {
            let remembered = Remembered {
                stamp: THIS_BUILD,
                outcome,
            };
            let value = value_for(&remembered);
            assert_eq!(remembered_from(&value), Some(remembered));
        }
    }

    #[test]
    fn a_damaged_file_is_no_answer_at_all() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({ "executable_size": 1, "executable_modified": 2 }),
            serde_json::json!({
                "executable_size": "big", "executable_modified": 2, "default_renderer": "started"
            }),
            serde_json::json!({
                "executable_size": 1, "executable_modified": 2, "default_renderer": "maybe"
            }),
            serde_json::json!({
                "executable_size": 1, "executable_modified": 2, "default_renderer": "failed"
            }),
            serde_json::json!({
                "executable_size": 1, "executable_modified": 2,
                "default_renderer": "failed", "reason": ""
            }),
        ] {
            assert_eq!(remembered_from(&value), None, "{value} is not an answer");
        }
    }
}
