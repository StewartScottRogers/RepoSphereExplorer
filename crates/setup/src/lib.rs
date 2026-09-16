//! The Windows setup program: one download, verified, installed where
//! `scripts/install.ps1` installs it.
//!
//! A reader downloads `ReposExplorerSetup.exe` and double-clicks it. It reads
//! the signed update manifest, takes the graphical application, the terminal
//! application and the service for this machine, checks each one against the
//! manifest exactly as [`updater::verify_bytes`] does - refusing to place
//! anything if a single file fails - and then places them, makes the Start
//! menu shortcut and the Settings > Apps entry, and writes the receipt that
//! `--uninstall` reads back.
//!
//! The script stays, for people who prefer it and for the distribution
//! check's script path. What an install *is* lives once, in [`layout`]; see
//! that module for how the two are kept from drifting.

pub mod arguments;
pub mod layout;
pub mod window;
#[cfg(windows)]
mod windows;

use arguments::Arguments;
use layout::{Plan, Removal};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use updater::{Manifest, TargetAsset};
use window::Task;

/// Where a release's files come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The manifest the updater itself polls: today's release.
    Latest,
    /// One published release, by its tag.
    Tag(String),
    /// A folder of already-built release files and their `manifest.json`,
    /// which is how the release workflow checks a release before anybody can
    /// download it. The files are verified exactly as downloaded ones are.
    Directory(PathBuf),
}

/// Where an install goes. Every field but the prefix exists so a second copy
/// installed beside the real one - the distribution check's - can be pointed
/// somewhere harmless rather than taking the first copy's Start menu
/// shortcut and Apps entry.
#[derive(Debug, Clone)]
pub struct Destination {
    /// The install folder.
    pub prefix: PathBuf,
    /// Where the Start menu shortcut goes.
    pub start_menu_directory: PathBuf,
    /// The Settings > Apps key, in the receipt's `HKCU:\...` spelling.
    pub registry_key: String,
    /// The setup program to leave behind as the uninstaller.
    pub uninstaller_source: PathBuf,
}

impl Destination {
    /// The plan for installing `version` here.
    #[must_use]
    pub fn plan(&self, version: &str) -> Plan {
        Plan {
            prefix: self.prefix.clone(),
            start_menu_directory: self.start_menu_directory.clone(),
            registry_key: self.registry_key.clone(),
            version: version.to_owned(),
            uninstaller_source: self.uninstaller_source.clone(),
        }
    }
}

/// Reads a command line into the task it asks for, filling in this
/// machine's own places for whatever it did not say.
///
/// `uninstaller_source` is the running setup program: what an install leaves
/// behind for the Settings > Apps Uninstall button to run.
///
/// # Errors
///
/// [`SetupError::Usage`] when there is nowhere to install to and the command
/// line did not say where - which is every system but Windows, where
/// `scripts/install.sh` is the way in.
pub fn task_for(arguments: &Arguments, uninstaller_source: PathBuf) -> Result<Task, SetupError> {
    let Some(prefix) = arguments.prefix.clone().or_else(layout::default_prefix) else {
        return Err(nowhere_to_install("--prefix"));
    };
    if arguments.uninstall {
        return Ok(Task::Uninstall(prefix));
    }
    let Some(start_menu_directory) = arguments
        .start_menu_directory
        .clone()
        .or_else(layout::default_start_menu_directory)
    else {
        return Err(nowhere_to_install("--start-menu-directory"));
    };
    let source = match (&arguments.from_directory, &arguments.tag) {
        (Some(directory), _) => Source::Directory(directory.clone()),
        (None, Some(tag)) => Source::Tag(tag.clone()),
        (None, None) => Source::Latest,
    };
    Ok(Task::Install {
        source,
        destination: Destination {
            prefix,
            start_menu_directory,
            registry_key: arguments
                .registry_key
                .clone()
                .unwrap_or_else(|| layout::REGISTRY_KEY.to_owned()),
            uninstaller_source,
        },
        target: updater::current_target().to_owned(),
    })
}

fn nowhere_to_install(option: &str) -> SetupError {
    if cfg!(windows) {
        SetupError::Usage(format!(
            "this user has no %LOCALAPPDATA% or %APPDATA%, so there is nowhere to install to; \
             give {option}"
        ))
    } else {
        SetupError::Usage(
            "this program installs Repos Explorer on Windows; on this system install it with \
             scripts/install.sh"
                .to_owned(),
        )
    }
}

/// Moves an uninstall out of the folder it is about to delete, and says
/// whether it did.
///
/// The Settings > Apps Uninstall button runs the copy of this program in the
/// install folder, and Windows will not let a running program's own file be
/// deleted. So that copy starts another from a temporary folder, hands it
/// the same command line, and exits; the new one waits for the old one's
/// file to be let go of, as it already waits for the application's. The
/// temporary copy is left where Windows clears temporary files.
///
/// # Errors
///
/// When this program cannot be found, copied, or started again.
pub fn relaunch_outside(prefix: &Path, arguments: &[String]) -> Result<bool, SetupError> {
    let running = std::env::current_exe()?;
    if !running.starts_with(std::path::absolute(prefix)?) {
        return Ok(false);
    }
    let directory = std::env::temp_dir().join(format!("rse-uninstall-{}", std::process::id()));
    fs::create_dir_all(&directory)?;
    let copy = directory.join(layout::SETUP_PROGRAM);
    fs::copy(&running, &copy)?;
    std::process::Command::new(&copy).args(arguments).spawn()?;
    Ok(true)
}

/// Everything this program refuses to do, and why.
#[derive(Debug)]
pub enum SetupError {
    /// The command line asked for something that is not a thing to ask for.
    Usage(String),
    /// There is already an install at this prefix.
    AlreadyInstalled(PathBuf),
    /// There is no install at this prefix to remove.
    NothingInstalled(PathBuf),
    /// The manifest could not be read.
    Manifest(String),
    /// The release publishes nothing for this machine.
    NotPublished {
        /// The release's version.
        version: String,
        /// The binaries it does not publish for this target.
        missing: Vec<String>,
        /// The target that was looked for.
        target: String,
        /// What it does publish for that target.
        published: Vec<String>,
    },
    /// A file did not match the manifest, so nothing was placed.
    Refused {
        /// The file's name, as the release publishes it.
        name: String,
        /// What was wrong with it.
        reason: String,
    },
    /// Reading, writing or removing a file failed.
    Io(io::Error),
    /// Windows refused to write the Start menu shortcut or the Apps entry.
    Windows(String),
}

impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetupError::Usage(message) | SetupError::Windows(message) => write!(f, "{message}"),
            SetupError::AlreadyInstalled(prefix) => write!(
                f,
                "Repos Explorer is already installed at {}; uninstall it first",
                prefix.display()
            ),
            SetupError::NothingInstalled(prefix) => write!(
                f,
                "nothing installed by this program at {} (no {} there)",
                prefix.display(),
                layout::RECEIPT
            ),
            SetupError::Manifest(message) => {
                write!(f, "could not read the update manifest: {message}")
            }
            SetupError::NotPublished {
                version,
                missing,
                target,
                published,
            } => write!(
                f,
                "release {version} publishes no {} for {target} (it publishes: {})",
                missing.join(", "),
                published.join(", ")
            ),
            SetupError::Refused { name, reason } => write!(f, "refusing {name}: {reason}"),
            SetupError::Io(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for SetupError {}

impl From<io::Error> for SetupError {
    fn from(err: io::Error) -> Self {
        SetupError::Io(err)
    }
}

/// Reports a step as it happens, so the window and a `--quiet` console see
/// the same account of what was done.
pub type Report<'a> = dyn FnMut(String) + 'a;

/// The assets this machine's install needs, in the order the receipt lists
/// them.
///
/// # Errors
///
/// [`SetupError::NotPublished`] when the release publishes no build of one of
/// them for `target`, naming what it does publish - the message a reader
/// meets when they run a setup program older than the release it is asked
/// for.
pub fn choose_assets<'a>(
    manifest: &'a Manifest,
    target: &str,
) -> Result<Vec<&'a TargetAsset>, SetupError> {
    let mut chosen = Vec::with_capacity(layout::INSTALLED.len());
    let mut missing = Vec::new();
    for binary in layout::INSTALLED {
        match manifest.find(binary, target) {
            Some(asset) => chosen.push(asset),
            None => missing.push(binary.to_owned()),
        }
    }
    if missing.is_empty() {
        return Ok(chosen);
    }
    Err(SetupError::NotPublished {
        version: manifest.version.clone(),
        missing,
        target: target.to_owned(),
        published: manifest
            .targets
            .iter()
            .filter(|asset| asset.target == target)
            .map(|asset| asset.binary.clone())
            .collect(),
    })
}

/// The file name a release publishes `asset` under: the last segment of its
/// address, which is also the name it carries in a folder of built files.
#[must_use]
pub fn asset_file_name(asset: &TargetAsset) -> &str {
    asset.url.rsplit('/').next().unwrap_or(&asset.url)
}

/// Checks `bytes` against `asset` and hands them back, or refuses them.
///
/// The same check the in-application updater applies to an update: the
/// digest the manifest publishes, and the manifest's signature over that
/// digest, against the public key compiled into this binary.
///
/// # Errors
///
/// [`SetupError::Refused`] when the digest differs - the file is not the one
/// that was released - or when it matches but the signature does not verify.
pub fn verified(asset: &TargetAsset, bytes: Vec<u8>) -> Result<Vec<u8>, SetupError> {
    match updater::verify_bytes(asset, &bytes) {
        Ok(()) => Ok(bytes),
        Err(updater::UpdateError::HashMismatch) => Err(SetupError::Refused {
            name: asset_file_name(asset).to_owned(),
            reason: "its digest does not match the manifest, so it is not the file that was \
                     released"
                .to_owned(),
        }),
        Err(err) => Err(SetupError::Refused {
            name: asset_file_name(asset).to_owned(),
            reason: err.to_string(),
        }),
    }
}

/// Reads the release's manifest.
///
/// # Errors
///
/// [`SetupError::Manifest`] when it cannot be fetched, read or parsed.
pub fn manifest_for(source: &Source) -> Result<Manifest, SetupError> {
    match source {
        Source::Latest => updater::fetch_manifest(updater::MANIFEST_URL)
            .map_err(|err| SetupError::Manifest(err.to_string())),
        Source::Tag(tag) => updater::fetch_manifest(&format!(
            "https://github.com/StewartScottRogers/RepoSphereExplorer/releases/download/{tag}/manifest.json"
        ))
        .map_err(|err| SetupError::Manifest(format!("release {tag}: {err}"))),
        Source::Directory(directory) => {
            let path = directory.join("manifest.json");
            let text = fs::read_to_string(&path).map_err(|err| {
                SetupError::Manifest(format!("{}: {err}", path.display()))
            })?;
            serde_json::from_str(&text)
                .map_err(|err| SetupError::Manifest(format!("{}: {err}", path.display())))
        }
    }
}

/// Fetches one asset's bytes, unverified.
fn bytes_for(source: &Source, asset: &TargetAsset) -> Result<Vec<u8>, SetupError> {
    match source {
        Source::Directory(directory) => {
            let path = directory.join(asset_file_name(asset));
            fs::read(&path).map_err(|err| {
                SetupError::Manifest(format!("could not read {}: {err}", path.display()))
            })
        }
        Source::Latest | Source::Tag(_) => updater::download(&asset.url).map_err(|err| {
            SetupError::Manifest(format!("could not download {}: {err}", asset.url))
        }),
    }
}

/// Installs a release: reads its manifest, takes every file this machine
/// needs, verifies them all, and only then places anything.
///
/// # Errors
///
/// [`SetupError::AlreadyInstalled`] when there is already an install at the
/// prefix, and whatever reading the manifest, downloading a file, verifying
/// one or writing the install fails with. Nothing is placed unless every
/// file verified.
pub fn install(
    source: &Source,
    destination: &Destination,
    target: &str,
    report: &mut Report<'_>,
) -> Result<Plan, SetupError> {
    if destination.prefix.join(layout::RECEIPT).exists() {
        return Err(SetupError::AlreadyInstalled(destination.prefix.clone()));
    }
    let manifest = manifest_for(source)?;
    report(format!("release {} for {target}", manifest.version));

    let assets = choose_assets(&manifest, target)?;
    let mut files = Vec::with_capacity(assets.len());
    for asset in assets {
        let name = asset_file_name(asset);
        report(format!("checking {name}"));
        files.push((
            asset.binary.clone(),
            verified(asset, bytes_for(source, asset)?)?,
        ));
    }

    let plan = destination.plan(&manifest.version);
    place(&plan, &files, report)?;
    report(format!(
        "installed Repos Explorer {} in {}",
        manifest.version,
        plan.prefix.display()
    ));
    Ok(plan)
}

/// Writes an install: the verified binaries, the uninstaller beside them,
/// the Start menu shortcut, the Settings > Apps entry, and the receipt that
/// names every one of them.
///
/// Separate from [`install`] because this is what an install *is*: tests
/// drive it with files of their own, and `install.ps1` is held to leaving
/// the same thing behind.
///
/// # Errors
///
/// Whatever writing a file, the shortcut or the registry entry fails with.
pub fn place(
    plan: &Plan,
    files: &[(String, Vec<u8>)],
    report: &mut Report<'_>,
) -> Result<(), SetupError> {
    fs::create_dir_all(&plan.prefix)?;
    for (binary, bytes) in files {
        let destination = plan.destination(binary);
        fs::write(&destination, bytes)?;
        report(format!("placed {}", destination.display()));
    }
    fs::copy(&plan.uninstaller_source, plan.uninstaller())?;
    report(format!("placed {}", plan.uninstaller().display()));

    write_shortcut(plan)?;
    report(format!("placed {}", plan.shortcut().display()));

    let kilobytes = plan
        .placed()
        .iter()
        .filter_map(|path| fs::metadata(path).ok())
        .map(|metadata| metadata.len())
        .sum::<u64>()
        / 1024;
    register(plan, u32::try_from(kilobytes).unwrap_or(u32::MAX))?;
    report(format!("registered {}", plan.registry_key));

    fs::write(plan.receipt(), plan.receipt_text())?;
    Ok(())
}

/// Removes an install, reading back the receipt it wrote: the files, the
/// Start menu shortcut and the Settings > Apps entry, and nothing else.
///
/// # Errors
///
/// [`SetupError::NothingInstalled`] when there is no receipt at `prefix`,
/// and whatever removing one of the things it names fails with.
pub fn uninstall(prefix: &Path, report: &mut Report<'_>) -> Result<(), SetupError> {
    let receipt_path = prefix.join(layout::RECEIPT);
    let receipt = fs::read_to_string(&receipt_path)
        .map_err(|_| SetupError::NothingInstalled(prefix.to_path_buf()))?;

    stop_processes_in(prefix, report);
    for removal in layout::removals(&receipt) {
        match removal {
            Removal::Path(path) => {
                if path.exists() {
                    remove_when_unlocked(&path)?;
                    report(format!("removed {}", path.display()));
                }
            }
            Removal::RegistryKey(key) => {
                if remove_registry_key(&key)? {
                    report(format!("removed {key}"));
                }
            }
        }
    }
    remove_when_unlocked(&receipt_path)?;

    let left: Vec<String> = fs::read_dir(prefix)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    if left.is_empty() {
        fs::remove_dir(prefix)?;
        report(format!("removed {}", prefix.display()));
    } else {
        report(format!(
            "left {} in place; it still holds: {}",
            prefix.display(),
            left.join(", ")
        ));
    }
    Ok(())
}

/// Deletes a file, waiting for Windows to let go of it.
///
/// An executable that has just been stopped is still locked for a moment
/// after its process is gone, and so is one an antivirus scanner is reading;
/// an uninstall that deleted straight away failed with "access is denied"
/// and left the install behind (#558). `install.ps1` waits the same way.
fn remove_when_unlocked(path: &Path) -> io::Result<()> {
    const ATTEMPTS: u32 = 40;
    const DELAY: std::time::Duration = std::time::Duration::from_millis(250);
    for attempt in 1..=ATTEMPTS {
        match fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(err) if attempt == ATTEMPTS => return Err(err),
            Err(_) => std::thread::sleep(DELAY),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn write_shortcut(plan: &Plan) -> Result<(), SetupError> {
    windows::write_shortcut(plan)
}

/// Off Windows there is no Start menu to put a shortcut in. The rest of an
/// install is the same everywhere, which is what lets this workspace's
/// checks - which run on Linux - test it.
#[cfg(not(windows))]
#[allow(clippy::unnecessary_wraps)]
fn write_shortcut(_plan: &Plan) -> Result<(), SetupError> {
    Ok(())
}

#[cfg(windows)]
fn register(plan: &Plan, kilobytes: u32) -> Result<(), SetupError> {
    windows::register(plan, kilobytes)
}

/// Off Windows there is no Settings > Apps to appear in.
#[cfg(not(windows))]
#[allow(clippy::unnecessary_wraps)]
fn register(_plan: &Plan, _kilobytes: u32) -> Result<(), SetupError> {
    Ok(())
}

#[cfg(windows)]
fn remove_registry_key(key: &str) -> Result<bool, SetupError> {
    windows::remove_registry_key(key)
}

/// A receipt written off Windows carries no registry line, so there is never
/// one to remove.
#[cfg(not(windows))]
#[allow(clippy::unnecessary_wraps)]
fn remove_registry_key(_key: &str) -> Result<bool, SetupError> {
    Ok(false)
}

#[cfg(windows)]
fn stop_processes_in(prefix: &Path, report: &mut Report<'_>) {
    windows::stop_processes_in(prefix, report);
}

/// Nothing off Windows runs from an install this program made.
#[cfg(not(windows))]
fn stop_processes_in(_prefix: &Path, _report: &mut Report<'_>) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(binary: &str, target: &str) -> TargetAsset {
        TargetAsset {
            binary: binary.to_owned(),
            target: target.to_owned(),
            url: format!("https://example.invalid/releases/{binary}-{target}.exe"),
            sha256: updater::sha256_hex(binary.as_bytes()),
            signature: "00".repeat(64),
        }
    }

    fn windows_manifest() -> Manifest {
        Manifest {
            version: "9.9.9".to_owned(),
            targets: layout::INSTALLED
                .iter()
                .map(|binary| asset(binary, "x86_64-pc-windows-msvc"))
                .chain(std::iter::once(asset("verify", "x86_64-pc-windows-msvc")))
                .collect(),
        }
    }

    #[test]
    fn a_target_takes_the_three_binaries_an_install_places() {
        let manifest = windows_manifest();
        let chosen = choose_assets(&manifest, "x86_64-pc-windows-msvc").unwrap();
        let names: Vec<&str> = chosen.iter().map(|asset| asset.binary.as_str()).collect();
        assert_eq!(
            names,
            ["RepoSphereExplorerGui", "RepoSphereExplorerTui", "service"]
        );
    }

    #[test]
    fn a_release_that_publishes_nothing_for_this_machine_says_what_it_does_publish() {
        let manifest = windows_manifest();
        let err = choose_assets(&manifest, "aarch64-apple-darwin").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("release 9.9.9 publishes no"), "{message}");
        assert!(message.contains("RepoSphereExplorerGui"), "{message}");
        assert!(message.contains("for aarch64-apple-darwin"), "{message}");
    }

    #[test]
    fn a_file_whose_bytes_changed_after_signing_is_refused() {
        let asset = asset("service", "x86_64-pc-windows-msvc");
        let err = verified(&asset, b"service tampered".to_vec()).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("refusing service-x86_64-pc-windows-msvc.exe"),
            "{message}"
        );
        assert!(
            message.contains("its digest does not match the manifest"),
            "{message}"
        );
    }

    #[test]
    fn a_file_the_manifest_did_not_sign_is_refused_even_when_its_digest_matches() {
        let asset = asset("service", "x86_64-pc-windows-msvc");
        let err = verified(&asset, b"service".to_vec()).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("signature verification failed"),
            "{message}"
        );
    }
}
