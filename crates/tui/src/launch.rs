//! Building the command line for handing a row to a program the reader
//! already has - the configured editor or the platform's file manager
//! (#674), mirroring `gui::launch`.
//!
//! Every decision here is a pure function of its inputs rather than of
//! `cfg!`/`std::env` read internally, so one test run can assert what each
//! of the three platforms would do without needing to run on it. The one
//! exception is [`on_path`], which does the real `PATH` search these
//! decisions are parameterised on; it is a thin, untested leaf.

use std::path::{Path, PathBuf};

/// The operating system a command is being built for, passed in rather than
/// read from `cfg!` so every branch can be exercised from one test run
/// regardless of the host running it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Microsoft Windows.
    Windows,
    /// Apple macOS.
    MacOs,
    /// Any other platform this application runs on.
    Linux,
}

impl Platform {
    /// The platform this binary is actually running on.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(target_os = "windows") {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Linux
        }
    }
}

/// A command built for a row, not yet run: what [`Platform::current`]'s
/// real launcher passes to `std::process::Command`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// The program to run.
    pub program: String,
    /// The arguments to run it with.
    pub args: Vec<String>,
}

impl Launch {
    fn new(program: &str, args: impl IntoIterator<Item = String>) -> Self {
        Self {
            program: program.to_owned(),
            args: args.into_iter().collect(),
        }
    }
}

/// The command that opens `path` in an editor: the `editor` setting when
/// one is configured, otherwise `code <path>` when Visual Studio Code is on
/// the `PATH`, otherwise `None` - there is nothing to launch.
#[must_use]
pub fn editor_launch(
    editor_setting: Option<&str>,
    code_available: bool,
    path: &Path,
) -> Option<Launch> {
    let path = path.to_string_lossy().into_owned();
    if let Some(editor) = editor_setting.filter(|editor| !editor.trim().is_empty()) {
        return Some(Launch::new(editor, [path]));
    }
    code_available.then(|| Launch::new("code", [path]))
}

/// The command that opens the platform's file manager with `path` selected.
///
/// Windows Explorer and macOS Finder both have a "select this item in its
/// parent folder" flag; Linux has no such standard across file managers, so
/// `xdg-open` opens the parent folder itself rather than claiming to select
/// anything inside it.
#[must_use]
pub fn file_manager_launch(platform: Platform, path: &Path) -> Launch {
    match platform {
        Platform::Windows => {
            Launch::new("explorer", [format!("/select,{}", path.to_string_lossy())])
        }
        Platform::MacOs => Launch::new(
            "open",
            ["-R".to_owned(), path.to_string_lossy().into_owned()],
        ),
        Platform::Linux => {
            let parent = path.parent().unwrap_or(path);
            Launch::new("xdg-open", [parent.to_string_lossy().into_owned()])
        }
    }
}

/// Whether `program` is on the `PATH` - a real filesystem search, used only
/// by the window's wiring to decide which branch of [`editor_launch`] to ask
/// for; the decision itself is what is tested.
#[must_use]
pub fn on_path(program: &str) -> bool {
    find_on_path(program).is_some()
}

/// Where `program` is on the `PATH`, as the full path to run.
///
/// Needed on Windows as well as for [`on_path`]: Visual Studio Code's
/// `code` is `code.cmd` there, and `std::process::Command::new("code")`
/// does not try `.cmd`, so a spawn would be enabled and then fail with
/// "program not found".
#[must_use]
pub fn find_on_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    find_in(std::env::split_paths(&path), program, cfg!(windows))
}

/// [`find_on_path`] over given directories, so the search can be tested
/// without touching the process's own `PATH`.
fn find_in(
    dirs: impl IntoIterator<Item = PathBuf>,
    program: &str,
    windows: bool,
) -> Option<PathBuf> {
    // On Windows a name without an extension is not something that runs:
    // Visual Studio Code's `bin` holds both `code.cmd` and `code`, a shell
    // script for other platforms, and running that fails with "%1 is not a
    // valid Win32 application". So the bare name counts only when it
    // already has an extension.
    let has_extension = Path::new(program).extension().is_some();
    let extensions: &[&str] = match (windows, has_extension) {
        (true, false) => &[".exe", ".cmd", ".bat"],
        _ => &[""],
    };
    dirs.into_iter().find_map(|dir| {
        extensions
            .iter()
            .map(|ext| dir.join(format!("{program}{ext}")))
            .find(|candidate| candidate.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::{Launch, Platform, editor_launch, file_manager_launch, find_in};
    use std::path::Path;

    fn launch(program: &str, args: &[&str]) -> Launch {
        Launch {
            program: program.to_owned(),
            args: args.iter().map(|&arg| arg.to_owned()).collect(),
        }
    }

    #[test]
    fn a_configured_editor_wins_over_code() {
        assert_eq!(
            editor_launch(Some("subl"), true, Path::new("/repos/one")),
            Some(launch("subl", &["/repos/one"]))
        );
    }

    #[test]
    fn code_is_used_when_nothing_is_configured() {
        assert_eq!(
            editor_launch(None, true, Path::new("/repos/one")),
            Some(launch("code", &["/repos/one"]))
        );
    }

    #[test]
    fn a_blank_editor_setting_is_treated_as_unset() {
        assert_eq!(
            editor_launch(Some("   "), true, Path::new("/repos/one")),
            Some(launch("code", &["/repos/one"]))
        );
    }

    #[test]
    fn nothing_is_launched_with_no_editor_and_no_code() {
        assert_eq!(editor_launch(None, false, Path::new("/repos/one")), None);
    }

    #[test]
    fn windows_explorer_selects_the_folder_in_its_parent() {
        assert_eq!(
            file_manager_launch(Platform::Windows, Path::new(r"C:\repos\one")),
            launch("explorer", &[r"/select,C:\repos\one"])
        );
    }

    #[test]
    fn macos_finder_reveals_the_folder() {
        assert_eq!(
            file_manager_launch(Platform::MacOs, Path::new("/Users/me/repos/one")),
            launch("open", &["-R", "/Users/me/repos/one"])
        );
    }

    #[test]
    fn linux_opens_the_parent_folder() {
        assert_eq!(
            file_manager_launch(Platform::Linux, Path::new("/home/me/repos/one")),
            launch("xdg-open", &["/home/me/repos"])
        );
    }

    #[test]
    fn windows_runs_code_cmd_not_the_extensionless_script_beside_it() {
        let dir = std::env::temp_dir().join(format!("rse-tui-find-on-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("code.cmd"), "@echo off\n").unwrap();
        std::fs::write(dir.join("code"), "#!/usr/bin/env sh\n").unwrap();

        assert_eq!(
            find_in([dir.clone()], "code", true),
            Some(dir.join("code.cmd")),
            "on Windows `code` is `code.cmd`, which a plain spawn does not find"
        );
        assert_eq!(
            find_in([dir.clone()], "code", false),
            Some(dir.join("code"))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
