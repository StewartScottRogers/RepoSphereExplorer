//! The two ways in leave one state, not two.
//!
//! `scripts/install.ps1` and this program install the same release into two
//! prefixes of this test's own, and everything Windows is left holding is
//! compared: the files, the receipt, the Start menu shortcut and the
//! Settings > Apps entry. Then each install is removed by *this program*,
//! reading the receipt the other one wrote.
//!
//! Windows only: the script is PowerShell, and a shortcut and a registry
//! entry are things only Windows has. The workspace's own checks run on
//! Linux, where `one_definition.rs` guards the same agreement from the other
//! side - it compares the definition both are written from.
#![cfg(windows)]

use setup::layout::{self, Plan};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where this test's registry entries go: not where a real install's go.
const TEST_KEYS: &str = r"Software\RepoSphereExplorerSetupTest";

const TARGET: &str = "x86_64-pc-windows-msvc";
const VERSION: &str = "9.9.9";

struct Scratch {
    directory: PathBuf,
    keys: String,
}

impl Scratch {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("rse-setup-one-state-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("a scratch folder should be makeable");
        Self {
            directory,
            keys: format!(r"{TEST_KEYS}\one-state-{}", std::process::id()),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
        let _ = Command::new("reg")
            .args(["delete", &format!(r"HKCU\{}", self.keys), "/f"])
            .output();
    }
}

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root should be two folders up")
}

fn powershell(command: &str) -> String {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            command,
        ])
        .output()
        .expect("PowerShell should run on Windows");
    assert!(
        output.status.success(),
        "PowerShell refused `{command}`:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

/// A folder of release files and their manifest, as `-FromDirectory` and
/// `--from-directory` take it. The digests are real; the signature is not,
/// which is what `-UnsignedTestManifest` is for and why the program's side
/// places the files rather than installing them.
fn release_directory(scratch: &Scratch) -> PathBuf {
    let release = scratch.directory.join("release");
    fs::create_dir_all(&release).unwrap();
    let mut assets = Vec::new();
    for binary in layout::INSTALLED {
        let name = format!("{binary}-{TARGET}.exe");
        let bytes = format!("the {binary} binary").into_bytes();
        assets.push(updater::TargetAsset {
            binary: (*binary).to_owned(),
            target: TARGET.to_owned(),
            url: format!("https://example.invalid/releases/{name}"),
            sha256: updater::sha256_hex(&bytes),
            signature: "00".repeat(64),
        });
        fs::write(release.join(&name), bytes).unwrap();
    }
    fs::write(
        release.join("manifest.json"),
        serde_json::to_string(&updater::Manifest {
            version: VERSION.to_owned(),
            targets: assets,
        })
        .unwrap(),
    )
    .unwrap();
    release
}

/// The text of a receipt, with everything that is this install's own
/// replaced, so two installs in two places can be compared line for line.
fn shape_of(text: &str, plan: &Plan, uninstaller: &str) -> String {
    text.trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace(&plan.prefix.display().to_string(), "<prefix>")
        .replace(&plan.start_menu_directory.display().to_string(), "<menu>")
        .replace(&plan.registry_key, "<key>")
        .replace(uninstaller, "<uninstaller>")
}

/// What the shortcut at `path` says, as Windows reads it back.
fn shortcut(path: &Path) -> String {
    powershell(&format!(
        "$shell = New-Object -ComObject WScript.Shell; \
         $link = $shell.CreateShortcut('{}'); \
         foreach ($part in 'TargetPath', 'WorkingDirectory', 'Description', 'IconLocation') \
         {{ Write-Output ($part + '=' + $link.$part) }}",
        path.display()
    ))
}

/// What the Settings > Apps entry at `key` holds.
fn apps_entry(key: &str) -> String {
    powershell(&format!(
        "$entry = Get-ItemProperty -LiteralPath '{key}'; \
         foreach ($name in 'DisplayName', 'DisplayVersion', 'Publisher', 'DisplayIcon', \
         'InstallLocation', 'EstimatedSize', 'NoModify', 'NoRepair', 'UninstallString') \
         {{ Write-Output ($name + '=' + $entry.$name) }}"
    ))
}

fn value(entry: &str, name: &str) -> String {
    entry
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{name}=")))
        .unwrap_or_else(|| panic!("{name} should be in:\n{entry}"))
        .to_owned()
}

/// Installs `release` with `scripts/install.ps1`, into places of this test's
/// own.
fn by_the_script(scratch: &Scratch, release: &Path) -> Plan {
    let script = repository().join("scripts/install.ps1");
    let plan = Plan {
        prefix: scratch.directory.join("by-the-script"),
        start_menu_directory: scratch.directory.join("script-menu"),
        registry_key: format!(r"HKCU:\{}\script", scratch.keys),
        version: VERSION.to_owned(),
        uninstaller_source: script.clone(),
    };
    let run = Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &script.display().to_string(),
            "-FromDirectory",
            &release.display().to_string(),
            "-UnsignedTestManifest",
            "-Prefix",
            &plan.prefix.display().to_string(),
            "-StartMenuDirectory",
            &plan.start_menu_directory.display().to_string(),
            "-UninstallRegistryKey",
            &plan.registry_key,
        ])
        .output()
        .expect("PowerShell should run on Windows");
    assert!(
        run.status.success(),
        "install.ps1 refused:\n{}\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    plan
}

/// Installs the same release with this program, into a prefix beside it.
///
/// Placed rather than installed: the fixture's manifest carries no real
/// signature, and this program has no flag that would overlook one.
fn by_the_program(scratch: &Scratch) -> Plan {
    let uninstaller_source = scratch.directory.join("downloaded-setup.exe");
    fs::write(&uninstaller_source, b"a setup program").unwrap();
    let plan = Plan {
        prefix: scratch.directory.join("by-the-program"),
        start_menu_directory: scratch.directory.join("program-menu"),
        registry_key: format!(r"HKCU:\{}\program", scratch.keys),
        version: VERSION.to_owned(),
        uninstaller_source,
    };
    let files: Vec<(String, Vec<u8>)> = layout::INSTALLED
        .iter()
        .map(|binary| {
            (
                (*binary).to_owned(),
                format!("the {binary} binary").into_bytes(),
            )
        })
        .collect();
    setup::place(&plan, &files, &mut |_| {}).expect("the program should install");
    plan
}

/// The Settings > Apps entry: the same values either way, but for the
/// uninstaller each one left behind, which is its own by design.
fn the_same_apps_entry(script_plan: &Plan, program_plan: &Plan) {
    let script_entry = apps_entry(&script_plan.registry_key);
    let program_entry = apps_entry(&program_plan.registry_key);
    for name in [
        "DisplayName",
        "DisplayVersion",
        "Publisher",
        "NoModify",
        "NoRepair",
    ] {
        assert_eq!(
            value(&script_entry, name),
            value(&program_entry, name),
            "{name} should be the same either way"
        );
    }
    for name in ["DisplayIcon", "InstallLocation"] {
        assert_eq!(
            value(&script_entry, name)
                .replace(&script_plan.prefix.display().to_string(), "<prefix>"),
            value(&program_entry, name)
                .replace(&program_plan.prefix.display().to_string(), "<prefix>"),
            "{name} should name the same place in the install folder"
        );
    }
    assert_eq!(value(&program_entry, "DisplayName"), layout::APPLICATION);
    assert_eq!(value(&program_entry, "DisplayVersion"), VERSION);
    assert_ne!(value(&program_entry, "EstimatedSize"), String::new());
    assert!(
        value(&program_entry, "UninstallString").contains(layout::SETUP_PROGRAM),
        "the Uninstall button should run the setup program left in the install folder: {}",
        value(&program_entry, "UninstallString")
    );
}

#[test]
fn the_script_and_the_program_leave_the_same_state() {
    let scratch = Scratch::new();
    let release = release_directory(&scratch);
    let script_plan = by_the_script(&scratch, &release);
    let program_plan = by_the_program(&scratch);

    // The binaries: the same names, the same bytes.
    for binary in layout::INSTALLED {
        let by_the_script = script_plan.destination(binary);
        let by_the_program = program_plan.destination(binary);
        assert!(
            by_the_script.exists() && by_the_program.exists(),
            "both should have placed {binary}"
        );
        assert_eq!(
            fs::read(&by_the_script).unwrap(),
            fs::read(&by_the_program).unwrap(),
            "{binary} should be the same file either way"
        );
    }

    // The receipt: the same lines, in the same order, naming the same things.
    let by_the_script = shape_of(
        &fs::read_to_string(script_plan.receipt()).expect("the script writes a receipt"),
        &script_plan,
        "install.ps1",
    );
    let by_the_program = shape_of(
        &fs::read_to_string(program_plan.receipt()).expect("the program writes a receipt"),
        &program_plan,
        layout::SETUP_PROGRAM,
    );
    assert_eq!(
        by_the_script, by_the_program,
        "the two receipts should describe the same install"
    );

    // The Start menu shortcut: the same target, folder, description and icon.
    let script_shortcut = shortcut(&script_plan.shortcut());
    let program_shortcut = shortcut(&program_plan.shortcut());
    assert_eq!(
        shape_of(&script_shortcut, &script_plan, "install.ps1"),
        shape_of(&program_shortcut, &program_plan, layout::SETUP_PROGRAM),
        "the two shortcuts should point at the same thing in the same way"
    );
    assert_eq!(
        value(&program_shortcut, "TargetPath"),
        program_plan.application().display().to_string(),
        "the shortcut should start the graphical application"
    );
    assert_eq!(value(&program_shortcut, "Description"), layout::SUMMARY);

    the_same_apps_entry(&script_plan, &program_plan);

    // And this program removes either one, from the receipt the other wrote.
    for plan in [&script_plan, &program_plan] {
        setup::uninstall(&plan.prefix, &mut |_| {}).expect("it should uninstall");
        assert!(
            !plan.prefix.exists(),
            "{} should be gone",
            plan.prefix.display()
        );
        assert!(!plan.shortcut().exists(), "the shortcut should be gone");
        let key = plan.registry_key.replace("HKCU:\\", "HKCU\\");
        let left = Command::new("reg").args(["query", &key]).output().unwrap();
        assert!(
            !left.status.success(),
            "{key} should be gone: {}",
            String::from_utf8_lossy(&left.stdout)
        );
    }
}
