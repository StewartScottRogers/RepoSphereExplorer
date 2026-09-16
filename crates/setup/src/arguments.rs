//! The command line: what a double-click does not give, and a script does.
//!
//! A reader gives none of these. The distribution check gives `--quiet` and
//! `--prefix`, so it can install a release without a window; the Settings >
//! Apps Uninstall button gives `--uninstall --prefix`, because that is what
//! the install wrote into the registry.

use crate::layout;
use std::path::PathBuf;

/// What the command line asked for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Arguments {
    /// Remove an install rather than make one.
    pub uninstall: bool,
    /// Do the work without opening a window, reporting on the console.
    pub quiet: bool,
    /// Say what the options are and do nothing.
    pub help: bool,
    /// Where to install, or what to remove.
    pub prefix: Option<PathBuf>,
    /// The release to install. `None` is the latest.
    pub tag: Option<String>,
    /// Install from a folder of already-built release files and their
    /// `manifest.json`.
    pub from_directory: Option<PathBuf>,
    /// Where the Start menu shortcut goes.
    pub start_menu_directory: Option<PathBuf>,
    /// The Settings > Apps key to write.
    pub registry_key: Option<String>,
}

/// What `--help` prints, and what a bad command line is answered with.
pub const USAGE: &str = "\
ReposExplorerSetup - installs Repos Explorer for the current user.

Double-click it to install the latest release. No administrator rights are
needed, and nothing is placed unless every downloaded file matches the
release's signed manifest.

  --uninstall                     remove the install and everything it made
  --quiet                         no window; report on the console instead
  --prefix <path>                 where to install, or what to remove
  --tag <tag>                     a release to install, such as v0.7.0
  --from-directory <path>         install from a folder of release files and
                                  their manifest.json, still fully verified
  --start-menu-directory <path>   where the Start menu shortcut goes
  --uninstall-registry-key <key>  the Settings > Apps key, which must be
                                  under HKCU:\\ - HKEY_CURRENT_USER, this
                                  user's own registry hive
  --help                          this

The last two exist so a second copy installed beside a real one - the
release check's - can be pointed somewhere harmless. Uninstall needs
neither: it reads both back from the receipt the install wrote.";

/// Reads a command line.
///
/// # Errors
///
/// A description of what was wrong with it, for a reader to see beside
/// [`USAGE`].
pub fn parse<I: IntoIterator<Item = String>>(arguments: I) -> Result<Arguments, String> {
    let mut parsed = Arguments::default();
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        let mut value = |name: &str| {
            arguments
                .next()
                .ok_or_else(|| format!("{name} needs a value after it"))
        };
        match argument.as_str() {
            "--uninstall" => parsed.uninstall = true,
            "--quiet" => parsed.quiet = true,
            "--help" | "-h" => parsed.help = true,
            "--prefix" => parsed.prefix = Some(PathBuf::from(value("--prefix")?)),
            "--tag" => parsed.tag = Some(value("--tag")?),
            "--from-directory" => {
                parsed.from_directory = Some(PathBuf::from(value("--from-directory")?));
            }
            "--start-menu-directory" => {
                parsed.start_menu_directory = Some(PathBuf::from(value("--start-menu-directory")?));
            }
            "--uninstall-registry-key" => {
                parsed.registry_key = Some(value("--uninstall-registry-key")?);
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    if parsed.tag.is_some() && parsed.from_directory.is_some() {
        return Err("--tag and --from-directory cannot be used together".to_owned());
    }
    if parsed.uninstall && (parsed.tag.is_some() || parsed.from_directory.is_some()) {
        return Err("--uninstall takes no release to install".to_owned());
    }
    if let Some(key) = &parsed.registry_key
        && !key.starts_with(layout::REGISTRY_DRIVE)
    {
        return Err(format!(
            "--uninstall-registry-key must be under {}, this user's own hive",
            layout::REGISTRY_DRIVE
        ));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(line: &str) -> Result<Arguments, String> {
        parse(line.split_whitespace().map(str::to_owned))
    }

    #[test]
    fn a_double_click_gives_nothing_and_means_install_the_latest_release() {
        let parsed = parse_words("").unwrap();
        assert_eq!(parsed, Arguments::default());
        assert!(!parsed.uninstall);
        assert!(!parsed.quiet);
        assert!(parsed.tag.is_none());
    }

    #[test]
    fn the_distribution_check_installs_one_release_into_a_prefix_without_a_window() {
        let parsed = parse_words("--quiet --prefix C:\\temp\\check --tag v0.7.0").unwrap();
        assert!(parsed.quiet);
        assert_eq!(parsed.prefix, Some(PathBuf::from(r"C:\temp\check")));
        assert_eq!(parsed.tag, Some("v0.7.0".to_owned()));
    }

    #[test]
    fn the_apps_entry_removes_the_install_it_was_written_for() {
        let parsed = parse_words("--uninstall --prefix C:\\p").unwrap();
        assert!(parsed.uninstall);
        assert_eq!(parsed.prefix, Some(PathBuf::from(r"C:\p")));
    }

    #[test]
    fn an_option_with_nothing_after_it_is_refused() {
        assert_eq!(
            parse_words("--prefix").unwrap_err(),
            "--prefix needs a value after it"
        );
    }

    #[test]
    fn two_releases_at_once_are_refused() {
        assert_eq!(
            parse_words("--tag v0.7.0 --from-directory release").unwrap_err(),
            "--tag and --from-directory cannot be used together"
        );
    }

    #[test]
    fn a_registry_key_outside_this_users_hive_is_refused() {
        let err = parse_words("--uninstall-registry-key HKLM:\\Software\\X").unwrap_err();
        assert!(err.contains("must be under HKCU:\\"), "{err}");
    }

    #[test]
    fn an_option_nobody_has_is_refused_by_name() {
        assert_eq!(
            parse_words("--purge").unwrap_err(),
            "unknown option --purge"
        );
    }
}
