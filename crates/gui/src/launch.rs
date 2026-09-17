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

use std::path::Path;

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
}

impl Launch {
    fn new(program: &str, args: impl IntoIterator<Item = String>) -> Self {
        Self {
            program: program.to_owned(),
            args: args.into_iter().collect(),
        }
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
/// present. `preferred_available` means "Windows Terminal is on the PATH"
/// on Windows, "`xdg-terminal-exec` is on the PATH" on Linux, and is
/// unused on macOS.
#[must_use]
pub fn terminal_launch(platform: Platform, path: &Path, preferred_available: bool) -> Launch {
    let path = path.to_string_lossy().into_owned();
    match platform {
        Platform::Windows if preferred_available => Launch::new("wt", ["-d".to_owned(), path]),
        Platform::Windows => Launch::new("powershell", [format!("-WorkingDirectory={path}")]),
        Platform::MacOs => Launch::new("open", ["-a".to_owned(), "Terminal".to_owned(), path]),
        Platform::Linux if preferred_available => Launch::new("xdg-terminal-exec", [path]),
        Platform::Linux => Launch::new(
            "x-terminal-emulator",
            [format!("--working-directory={path}")],
        ),
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
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        if cfg!(windows) {
            ["", ".exe", ".cmd", ".bat"]
                .iter()
                .any(|ext| dir.join(format!("{program}{ext}")).is_file())
        } else {
            dir.join(program).is_file()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{
        Launch, Platform, editor_launch, file_manager_label, file_manager_launch, terminal_launch,
    };
    use std::path::Path;

    fn launch(program: &str, args: &[&str]) -> Launch {
        Launch {
            program: program.to_owned(),
            args: args.iter().map(|&arg| arg.to_owned()).collect(),
        }
    }

    #[test]
    fn windows_terminal_is_preferred_when_present() {
        assert_eq!(
            terminal_launch(Platform::Windows, Path::new(r"C:\repos\one"), true),
            launch("wt", &["-d", r"C:\repos\one"])
        );
    }

    #[test]
    fn windows_falls_back_to_powershell_without_windows_terminal() {
        assert_eq!(
            terminal_launch(Platform::Windows, Path::new(r"C:\repos\one"), false),
            launch("powershell", &[r"-WorkingDirectory=C:\repos\one"])
        );
    }

    #[test]
    fn macos_opens_terminal_app() {
        assert_eq!(
            terminal_launch(Platform::MacOs, Path::new("/Users/me/repos/one"), false),
            launch("open", &["-a", "Terminal", "/Users/me/repos/one"])
        );
    }

    #[test]
    fn linux_prefers_xdg_terminal_exec_when_present() {
        assert_eq!(
            terminal_launch(Platform::Linux, Path::new("/home/me/repos/one"), true),
            launch("xdg-terminal-exec", &["/home/me/repos/one"])
        );
    }

    #[test]
    fn linux_falls_back_to_x_terminal_emulator() {
        assert_eq!(
            terminal_launch(Platform::Linux, Path::new("/home/me/repos/one"), false),
            launch(
                "x-terminal-emulator",
                &["--working-directory=/home/me/repos/one"]
            )
        );
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
