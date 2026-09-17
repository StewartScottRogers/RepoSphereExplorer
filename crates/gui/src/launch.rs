//! Building the command line for handing a folder to a program the user
//! already has - a terminal, an editor, or the platform's file manager
//! (#581).
//!
//! Every decision here is a pure function of its inputs rather than of
//! `cfg!`/`std::env` read internally, so one test run can assert what each
//! of the three platforms would do without needing to run on it. The one
//! exception is [`on_path`], which does the real `PATH` search these
//! decisions are parameterised on; it is a thin, untested leaf, the same
//! way `open::that_detached` is untested at the call in `lib.rs`.

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

/// A command built for a folder, not yet run: what [`Platform::current`]'s
/// real launcher passes to `std::process::Command`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// The program to run.
    pub program: String,
    /// The arguments to run it with.
    pub args: Vec<String>,
    /// The directory to start it in, when that is how it is told where to
    /// open. Set on the process itself rather than passed as a switch,
    /// because a switch is one more thing a program may not accept:
    /// Windows PowerShell 5.1 has no `-WorkingDirectory` (#610's review).
    pub current_dir: Option<String>,
}

impl Launch {
    fn new(program: &str, args: impl IntoIterator<Item = String>) -> Self {
        Self {
            program: program.to_owned(),
            args: args.into_iter().collect(),
            current_dir: None,
        }
    }

    fn in_dir(mut self, dir: &str) -> Self {
        self.current_dir = Some(dir.to_owned());
        self
    }
}

/// The command that opens a terminal in `path`.
///
/// Windows prefers Windows Terminal (`wt -d <path>`) and falls back to
/// PowerShell started in that folder when it is not on the `PATH`. Linux
/// prefers `xdg-terminal-exec`, the XDG-specified way to ask for "a
/// terminal, whichever one the desktop has configured", started in that
/// folder, and falls back to `x-terminal-emulator` in the same way. macOS
/// has one answer: `open -a Terminal <path>`, since `open` is always
/// present. Every terminal is also started with `path` as its working
/// directory, so the fallbacks do not depend on a switch for it. `preferred_available` means "Windows Terminal is on the PATH"
/// on Windows, "`xdg-terminal-exec` is on the PATH" on Linux, and is
/// unused on macOS.
#[must_use]
pub fn terminal_launch(platform: Platform, path: &Path, preferred_available: bool) -> Launch {
    let path = path.to_string_lossy().into_owned();
    match platform {
        Platform::Windows if preferred_available => {
            Launch::new("wt", ["-d".to_owned(), path.clone()]).in_dir(&path)
        }
        // Windows PowerShell 5.1, on every Windows install, has no
        // `-WorkingDirectory` switch; it starts where it is started.
        Platform::Windows => Launch::new("powershell", ["-NoExit".to_owned()]).in_dir(&path),
        Platform::MacOs => Launch::new(
            "open",
            ["-a".to_owned(), "Terminal".to_owned(), path.clone()],
        )
        .in_dir(&path),
        Platform::Linux if preferred_available => {
            Launch::new("xdg-terminal-exec", Vec::new()).in_dir(&path)
        }
        Platform::Linux => Launch::new("x-terminal-emulator", Vec::new()).in_dir(&path),
    }
}

/// The command that opens `path` in an editor: the `editor` setting when
/// one is configured, otherwise `code <path>` when Visual Studio Code is on
/// the `PATH`, otherwise `None` - there is nothing to launch, and the menu
/// item that would launch it stays disabled.
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

/// What `platform`'s file manager is called, for the menu item that opens
/// it: "Show in File Explorer", "Show in Finder" or "Show in Files".
#[must_use]
pub const fn file_manager_label(platform: Platform) -> &'static str {
    match platform {
        Platform::Windows => "Show in File Explorer",
        Platform::MacOs => "Show in Finder",
        Platform::Linux => "Show in Files",
    }
}

/// Whether `program` is on the `PATH` - a real filesystem search, used only
/// by the window's wiring to decide which branch of [`terminal_launch`] or
/// [`editor_launch`] to ask for; the decision itself is what is tested.
#[must_use]
pub fn on_path(program: &str) -> bool {
    find_on_path(program).is_some()
}

/// Where `program` is on the `PATH`, as the full path to run.
///
/// Needed on Windows as well as for [`on_path`]: Visual Studio Code's
/// `code` is `code.cmd` there, and `std::process::Command::new("code")`
/// does not try `.cmd`, so Open in editor was enabled and then failed with
/// "program not found". The launcher runs what this finds.
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
    use super::{
        Launch, Platform, editor_launch, file_manager_label, file_manager_launch, find_in,
        terminal_launch,
    };
    use std::path::Path;

    fn launch(program: &str, args: &[&str]) -> Launch {
        Launch {
            program: program.to_owned(),
            args: args.iter().map(|&arg| arg.to_owned()).collect(),
            current_dir: None,
        }
    }

    fn launch_in(program: &str, args: &[&str], dir: &str) -> Launch {
        Launch {
            current_dir: Some(dir.to_owned()),
            ..launch(program, args)
        }
    }

    #[test]
    fn windows_terminal_is_preferred_when_present() {
        assert_eq!(
            terminal_launch(Platform::Windows, Path::new(r"C:\repos\one"), true),
            launch_in("wt", &["-d", r"C:\repos\one"], r"C:\repos\one")
        );
    }

    #[test]
    fn windows_falls_back_to_powershell_without_windows_terminal() {
        assert_eq!(
            terminal_launch(Platform::Windows, Path::new(r"C:\repos\one"), false),
            launch_in("powershell", &["-NoExit"], r"C:\repos\one")
        );
    }

    #[test]
    fn macos_opens_terminal_app() {
        assert_eq!(
            terminal_launch(Platform::MacOs, Path::new("/Users/me/repos/one"), false),
            launch_in(
                "open",
                &["-a", "Terminal", "/Users/me/repos/one"],
                "/Users/me/repos/one"
            )
        );
    }

    #[test]
    fn linux_prefers_xdg_terminal_exec_when_present() {
        assert_eq!(
            terminal_launch(Platform::Linux, Path::new("/home/me/repos/one"), true),
            launch_in("xdg-terminal-exec", &[], "/home/me/repos/one")
        );
    }

    #[test]
    fn linux_falls_back_to_x_terminal_emulator() {
        assert_eq!(
            terminal_launch(Platform::Linux, Path::new("/home/me/repos/one"), false),
            launch_in("x-terminal-emulator", &[], "/home/me/repos/one")
        );
    }

    #[test]
    fn no_windows_terminal_is_told_its_folder_by_a_switch_powershell_lacks() {
        let fallback = terminal_launch(Platform::Windows, Path::new(r"C:\repos\one"), false);
        assert!(
            fallback
                .args
                .iter()
                .all(|arg| !arg.to_lowercase().contains("workingdirectory")),
            "Windows PowerShell 5.1 has no -WorkingDirectory: {fallback:?}"
        );
    }

    #[test]
    fn windows_runs_code_cmd_not_the_extensionless_script_beside_it() {
        let dir = std::env::temp_dir().join(format!("rse-find-on-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("code.cmd"), "@echo off\n").unwrap();
        // Beside it, as in Visual Studio Code's own `bin`: the script for
        // other platforms, which Windows cannot run.
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
        assert_eq!(
            find_in([dir.clone()], "code.cmd", true),
            Some(dir.join("code.cmd"))
        );
        assert_eq!(find_in([dir.clone()], "absent", true), None);

        let _ = std::fs::remove_dir_all(&dir);
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
    fn each_platform_names_its_own_file_manager() {
        assert_eq!(
            file_manager_label(Platform::Windows),
            "Show in File Explorer"
        );
        assert_eq!(file_manager_label(Platform::MacOs), "Show in Finder");
        assert_eq!(file_manager_label(Platform::Linux), "Show in Files");
    }
}
