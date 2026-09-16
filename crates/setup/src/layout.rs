//! What an install is: the files it places, the receipt it writes, and the
//! two entries it makes in Windows.
//!
//! One definition, read twice. The setup program installs straight from it.
//! `scripts/install.ps1` carries a rendered copy of it, between
//! [`GENERATED_BEGIN`] and [`GENERATED_END`]; `the_script_carries_this_layout`
//! in `tests/one_definition.rs` fails when the committed script has drifted
//! from what [`powershell`] renders, so the two cannot come to disagree about
//! where a file goes, what the receipt says, or what Windows is told.
//!
//! The mechanics still differ - a script makes its shortcut through the
//! Component Object Model (COM) and this program writes the Shell Link file
//! itself - but "what an install is" is the state left behind, and that lives
//! here. `the_script_and_the_program_leave_the_same_state` in
//! `tests/one_state.rs` runs both on Windows and compares what each left.

use std::path::{Path, PathBuf};

/// The binaries an install places, named as the release manifest names them,
/// in the order the receipt lists them.
pub const INSTALLED: [&str; 3] = ["RepoSphereExplorerGui", "RepoSphereExplorerTui", "service"];

/// The binary the Start menu shortcut and the Settings > Apps icon point at.
pub const APPLICATION_BINARY: &str = "RepoSphereExplorerGui";

/// The setup program's own copy, left in the install folder so the Uninstall
/// button in Settings has something to run long after the downloaded copy is
/// gone. `install.ps1` leaves a copy of itself there for the same reason;
/// each way in leaves its own uninstaller, and both leave everything else
/// identical.
pub const SETUP_PROGRAM: &str = "ReposExplorerSetup.exe";

/// The receipt: one line per path the install placed, and one for the
/// registry key it made. Uninstall removes what this names and nothing else.
pub const RECEIPT: &str = "installed-files.txt";

/// A receipt line starting with this is a registry key to remove rather than
/// a path. `install.sh` writes paths only; nothing outside Windows reads or
/// writes a line of this shape.
pub const REGISTRY_LINE: &str = "registry:";

/// The name Windows shows: the Start menu entry, and `DisplayName` in
/// Settings > Apps.
pub const APPLICATION: &str = "Repos Explorer";

/// `Publisher` in Settings > Apps.
pub const PUBLISHER: &str = "Stewart Scott Rogers";

/// The shortcut's description, which Windows shows as its tooltip.
pub const SUMMARY: &str =
    "Repos Explorer - a front door to the working copies your source control checks code out into";

/// The Settings > Apps key, below `HKEY_CURRENT_USER` (HKCU): this user's own
/// hive, so no administrator rights are needed and no other account is
/// touched.
pub const REGISTRY_KEY_PATH: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\ReposExplorer";

/// How a receipt and `install.ps1` spell [`REGISTRY_KEY_PATH`]: PowerShell's
/// own drive notation for the same hive.
pub const REGISTRY_KEY: &str =
    r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ReposExplorer";

/// The prefix a receipt's registry line and `install.ps1` put before a key
/// path under [`windows_registry::CURRENT_USER`]: stripping it turns the
/// spelling the receipt carries into the one the registry wants.
pub const REGISTRY_DRIVE: &str = r"HKCU:\";

/// Below `%LOCALAPPDATA%`: where the files go.
pub const PREFIX_UNDER_LOCAL_APPLICATION_DATA: &str = r"Programs\RepoSphereExplorer";

/// Below `%APPDATA%`: where the Start menu shortcut goes.
pub const START_MENU_UNDER_APPLICATION_DATA: &str = r"Microsoft\Windows\Start Menu\Programs";

/// The first line of the block `install.ps1` carries.
pub const GENERATED_BEGIN: &str = "# --- generated from crates/setup/src/layout.rs ---";

/// The last line of that block.
pub const GENERATED_END: &str = "# --- end generated ---";

/// The file name a manifest binary is installed under.
///
/// Always the Windows name: this program installs on Windows, whatever
/// machine a test happens to run it on.
#[must_use]
pub fn executable(binary: &str) -> String {
    format!("{binary}.exe")
}

/// The Start menu shortcut's file name.
#[must_use]
pub fn shortcut_name() -> String {
    format!("{APPLICATION}.lnk")
}

/// `%LOCALAPPDATA%\Programs\RepoSphereExplorer`, or `None` off Windows,
/// where there is no such variable.
#[must_use]
pub fn default_prefix() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|local| Path::new(&local).join(PREFIX_UNDER_LOCAL_APPLICATION_DATA))
}

/// This user's Start menu Programs folder, or `None` off Windows.
#[must_use]
pub fn default_start_menu_directory() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(|roaming| Path::new(&roaming).join(START_MENU_UNDER_APPLICATION_DATA))
}

/// One install, fully placed: where it goes and what it is told to call
/// itself.
#[derive(Debug, Clone)]
pub struct Plan {
    /// The install folder.
    pub prefix: PathBuf,
    /// Where the Start menu shortcut goes.
    pub start_menu_directory: PathBuf,
    /// The Settings > Apps key, in the receipt's `HKCU:\...` spelling.
    pub registry_key: String,
    /// The release version, as the manifest gives it: `DisplayVersion`.
    pub version: String,
    /// The setup program to leave behind as the uninstaller - this running
    /// binary, in every case but a test.
    pub uninstaller_source: PathBuf,
}

impl Plan {
    /// Where `binary` is installed to.
    #[must_use]
    pub fn destination(&self, binary: &str) -> PathBuf {
        self.prefix.join(executable(binary))
    }

    /// The graphical application, which the shortcut and the Apps entry's
    /// icon point at.
    #[must_use]
    pub fn application(&self) -> PathBuf {
        self.destination(APPLICATION_BINARY)
    }

    /// The uninstaller's copy in the install folder.
    #[must_use]
    pub fn uninstaller(&self) -> PathBuf {
        self.prefix.join(SETUP_PROGRAM)
    }

    /// The Start menu shortcut.
    #[must_use]
    pub fn shortcut(&self) -> PathBuf {
        self.start_menu_directory.join(shortcut_name())
    }

    /// The receipt.
    #[must_use]
    pub fn receipt(&self) -> PathBuf {
        self.prefix.join(RECEIPT)
    }

    /// Every path an install places, in the order the receipt lists them:
    /// the binaries, the uninstaller beside them, then the shortcut.
    #[must_use]
    pub fn placed(&self) -> Vec<PathBuf> {
        let mut placed: Vec<PathBuf> = INSTALLED
            .iter()
            .map(|binary| self.destination(binary))
            .collect();
        placed.push(self.uninstaller());
        placed.push(self.shortcut());
        placed
    }

    /// The receipt's text: every placed path, then the registry key.
    #[must_use]
    pub fn receipt_text(&self) -> String {
        let mut lines: Vec<String> = self
            .placed()
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        lines.push(format!("{REGISTRY_LINE}{}", self.registry_key));
        lines.join("\r\n") + "\r\n"
    }

    /// What the Uninstall button in Settings > Apps runs: the copy of the
    /// setup program left in the install folder, against this install.
    #[must_use]
    pub fn uninstall_command(&self) -> String {
        format!(
            "\"{}\" --uninstall --prefix \"{}\"",
            self.uninstaller().display(),
            self.prefix.display()
        )
    }

    /// The string values of the Settings > Apps entry, in the order
    /// `install.ps1` writes them. `EstimatedSize`, `NoModify` and `NoRepair`
    /// are numbers and are written beside these.
    #[must_use]
    pub fn registry_strings(&self) -> Vec<(&'static str, String)> {
        vec![
            ("DisplayName", APPLICATION.to_owned()),
            ("DisplayVersion", self.version.clone()),
            ("Publisher", PUBLISHER.to_owned()),
            ("DisplayIcon", self.application().display().to_string()),
            ("InstallLocation", self.prefix.display().to_string()),
            ("UninstallString", self.uninstall_command()),
        ]
    }
}

/// The lines a receipt holds, split into what they mean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Removal {
    /// A path to delete.
    Path(PathBuf),
    /// A registry key to delete, in the receipt's `HKCU:\...` spelling.
    RegistryKey(String),
}

/// Reads a receipt into what uninstall has to remove, skipping blank lines.
///
/// A receipt `install.ps1` wrote opens with a byte order mark, which is what
/// Windows PowerShell puts in front of the text it writes. It is dropped
/// here rather than left on the front of the first path, so this program can
/// remove an install the script made.
#[must_use]
pub fn removals(receipt: &str) -> Vec<Removal> {
    receipt
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(|line| match line.strip_prefix(REGISTRY_LINE) {
            Some(key) => Removal::RegistryKey(key.to_owned()),
            None => Removal::Path(PathBuf::from(line)),
        })
        .collect()
}

/// Renders the block `scripts/install.ps1` carries, so the script installs
/// to the same places under the same names as this program.
#[must_use]
pub fn powershell() -> String {
    let binaries = INSTALLED
        .iter()
        .map(|binary| format!("'{binary}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "\
{GENERATED_BEGIN}
# One definition of what an install is, shared with the setup program:
# `crates/setup/src/layout.rs` renders this block, and `cargo test -p setup`
# fails if the copy below has drifted from it. Edit that file, not this block.
$Installed = @({binaries})
$Receipt = '{RECEIPT}'
$Application = '{APPLICATION}'
$Publisher = '{PUBLISHER}'
$Summary = '{SUMMARY}'
# A receipt line is a path to remove, unless it starts with this, in which
# case the rest of it is a registry key to remove. install.sh writes paths
# only; nothing outside Windows reads or writes a line of this shape.
$RegistryLine = '{REGISTRY_LINE}'
if (-not $Prefix) {{ $Prefix = Join-Path $env:LOCALAPPDATA '{PREFIX_UNDER_LOCAL_APPLICATION_DATA}' }}
if (-not $StartMenuDirectory) {{ $StartMenuDirectory = Join-Path $env:APPDATA '{START_MENU_UNDER_APPLICATION_DATA}' }}
if (-not $UninstallRegistryKey) {{ $UninstallRegistryKey = '{REGISTRY_KEY}' }}
{GENERATED_END}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path under the fixture prefix, spelled the way this platform
    /// spells it. These are Windows paths in use, but the tests also run on
    /// the continuous integration runner, where `join` writes `/` - and what
    /// each assertion is about is the shape of what is built, not the
    /// separator the host happens to use.
    fn under(prefix: &str, name: &str) -> String {
        PathBuf::from(prefix).join(name).display().to_string()
    }

    fn plan() -> Plan {
        Plan {
            prefix: PathBuf::from(r"C:\prefix"),
            start_menu_directory: PathBuf::from(r"C:\menu"),
            registry_key: REGISTRY_KEY.to_owned(),
            version: "1.2.3".to_owned(),
            uninstaller_source: PathBuf::from(r"C:\downloads\ReposExplorerSetup.exe"),
        }
    }

    #[test]
    fn an_install_places_three_binaries_an_uninstaller_and_a_shortcut() {
        let placed = plan().placed();
        let names: Vec<String> = placed
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "RepoSphereExplorerGui.exe",
                "RepoSphereExplorerTui.exe",
                "service.exe",
                "ReposExplorerSetup.exe",
                "Repos Explorer.lnk",
            ]
        );
    }

    #[test]
    fn the_receipt_lists_every_placed_path_then_the_registry_key() {
        let plan = plan();
        let receipt = plan.receipt_text();
        let lines: Vec<&str> = receipt.lines().collect();
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0], under(r"C:\prefix", "RepoSphereExplorerGui.exe"));
        assert_eq!(lines[4], under(r"C:\menu", "Repos Explorer.lnk"));
        assert_eq!(lines[5], format!("registry:{REGISTRY_KEY}"));
    }

    #[test]
    fn a_receipt_reads_back_as_the_paths_and_the_key_it_lists() {
        let plan = plan();
        let removals = removals(&plan.receipt_text());
        assert_eq!(removals.len(), 6);
        assert_eq!(removals[0], Removal::Path(plan.application()));
        assert_eq!(removals[5], Removal::RegistryKey(REGISTRY_KEY.to_owned()));
    }

    #[test]
    fn the_uninstall_button_runs_the_copy_left_in_the_install_folder() {
        assert_eq!(
            plan().uninstall_command(),
            format!(
                r#""{}" --uninstall --prefix "C:\prefix""#,
                under(r"C:\prefix", "ReposExplorerSetup.exe")
            )
        );
    }

    #[test]
    fn the_apps_entry_names_the_release_and_points_at_the_application() {
        let values = plan().registry_strings();
        assert_eq!(values[0], ("DisplayName", "Repos Explorer".to_owned()));
        assert_eq!(values[1], ("DisplayVersion", "1.2.3".to_owned()));
        assert_eq!(
            values[3],
            (
                "DisplayIcon",
                under(r"C:\prefix", "RepoSphereExplorerGui.exe")
            )
        );
    }
}
