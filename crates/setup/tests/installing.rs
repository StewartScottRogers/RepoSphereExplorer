//! What an install leaves behind, and what removing it takes away.
//!
//! These drive the placement and the removal directly, with files of the
//! test's own, because that is the part that is the same on every machine.
//! The Start menu shortcut and the Settings > Apps entry are Windows's;
//! `one_state.rs` checks those against the script, there.

use setup::layout::{self, Plan};
use std::fs;
use std::path::{Path, PathBuf};

/// Where this test's registry entries go: not where a real install's go.
const TEST_KEYS: &str = r"Software\RepoSphereExplorerSetupTest";

/// A folder and a registry subtree of one test's own, both removed when the
/// test is done with them.
struct Scratch {
    directory: PathBuf,
    key: String,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let directory =
            std::env::temp_dir().join(format!("rse-setup-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("a scratch folder should be makeable");
        Self {
            directory,
            key: format!(r"{TEST_KEYS}\{name}-{}", std::process::id()),
        }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
        if cfg!(windows) {
            // reg.exe rather than this crate's own registry code: a test
            // cleans up after itself even when the code it is testing is
            // what failed.
            let _ = std::process::Command::new("reg")
                .args(["delete", &format!(r"HKCU\{}", self.key), "/f"])
                .output();
        }
    }
}

/// An install of three named files, with an uninstaller small enough to be a
/// test fixture.
fn plan(scratch: &Scratch) -> Plan {
    let uninstaller_source = scratch.join("downloaded-setup.exe");
    fs::write(&uninstaller_source, b"a setup program").unwrap();
    Plan {
        prefix: scratch.join("prefix"),
        start_menu_directory: scratch.join("menu"),
        registry_key: format!(r"HKCU:\{}", scratch.key),
        version: "9.9.9".to_owned(),
        uninstaller_source,
    }
}

fn files() -> Vec<(String, Vec<u8>)> {
    layout::INSTALLED
        .iter()
        .map(|binary| {
            (
                (*binary).to_owned(),
                format!("the {binary} binary").into_bytes(),
            )
        })
        .collect()
}

fn place(plan: &Plan) -> Vec<String> {
    let mut said = Vec::new();
    setup::place(plan, &files(), &mut |line| said.push(line)).expect("the install should be made");
    said
}

fn names_in(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .expect("the folder should be readable")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn destination(plan: &Plan) -> setup::Destination {
    setup::Destination {
        prefix: plan.prefix.clone(),
        start_menu_directory: plan.start_menu_directory.clone(),
        registry_key: plan.registry_key.clone(),
        uninstaller_source: plan.uninstaller_source.clone(),
    }
}

/// A folder of release files and their manifest, as `--from-directory`
/// takes it. `tamper` is the binary whose bytes are changed after its digest
/// was taken, if any.
fn release_directory(scratch: &Scratch, target: &str, tamper: Option<&str>) -> PathBuf {
    let release = scratch.join("release");
    fs::create_dir_all(&release).unwrap();
    let mut assets = Vec::new();
    for binary in layout::INSTALLED {
        let name = format!("{binary}-{target}.exe");
        let bytes = format!("the {binary} binary").into_bytes();
        let signed = updater::sha256_hex(&bytes);
        let mut written = bytes;
        if tamper == Some(binary) {
            written.extend_from_slice(b"tampered");
        }
        fs::write(release.join(&name), written).unwrap();
        assets.push(updater::TargetAsset {
            binary: (*binary).to_owned(),
            target: target.to_owned(),
            url: format!("https://example.invalid/releases/{name}"),
            sha256: signed,
            signature: "00".repeat(64),
        });
    }
    let manifest = updater::Manifest {
        version: "9.9.9".to_owned(),
        targets: assets,
    };
    fs::write(
        release.join("manifest.json"),
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();
    release
}

#[test]
fn an_install_places_the_binaries_the_uninstaller_and_the_receipt() {
    let scratch = Scratch::new("places");
    let plan = plan(&scratch);
    place(&plan);

    assert_eq!(
        names_in(&plan.prefix),
        [
            "RepoSphereExplorerGui.exe",
            "RepoSphereExplorerTui.exe",
            "ReposExplorerSetup.exe",
            "installed-files.txt",
            "service.exe",
        ]
    );
    assert_eq!(
        fs::read(plan.application()).unwrap(),
        b"the RepoSphereExplorerGui binary"
    );
    assert_eq!(fs::read(plan.uninstaller()).unwrap(), b"a setup program");
}

#[test]
fn the_receipt_names_every_path_that_was_placed_and_the_registry_key() {
    let scratch = Scratch::new("receipt");
    let plan = plan(&scratch);
    place(&plan);

    let receipt = fs::read_to_string(plan.receipt()).expect("the receipt should be there");
    let removals = layout::removals(&receipt);
    let paths: Vec<PathBuf> = removals
        .iter()
        .filter_map(|removal| match removal {
            layout::Removal::Path(path) => Some(path.clone()),
            layout::Removal::RegistryKey(_) => None,
        })
        .collect();
    assert_eq!(paths, plan.placed());
    assert_eq!(
        removals.last(),
        Some(&layout::Removal::RegistryKey(plan.registry_key.clone()))
    );
    for path in &paths {
        // The shortcut is Windows's, and is written nowhere else.
        if cfg!(windows) || path != &plan.shortcut() {
            assert!(path.exists(), "{} should have been placed", path.display());
        }
    }
}

#[test]
fn an_install_says_what_it_did_as_it_does_it() {
    let scratch = Scratch::new("said");
    let plan = plan(&scratch);
    let said = place(&plan);
    assert!(
        said.iter()
            .any(|line| line == &format!("placed {}", plan.application().display())),
        "{said:?}"
    );
    assert!(
        said.iter()
            .any(|line| line == &format!("registered {}", plan.registry_key)),
        "{said:?}"
    );
}

#[test]
fn uninstall_removes_exactly_what_the_receipt_names_and_leaves_the_rest() {
    let scratch = Scratch::new("removes");
    let plan = plan(&scratch);
    place(&plan);
    let kept = plan.prefix.join("something-else.txt");
    fs::write(&kept, b"not ours").unwrap();

    let mut said = Vec::new();
    setup::uninstall(&plan.prefix, &mut |line| said.push(line)).expect("it should uninstall");

    for path in plan.placed() {
        assert!(!path.exists(), "{} should be gone", path.display());
    }
    assert!(!plan.receipt().exists(), "the receipt should be gone");
    assert!(
        kept.exists(),
        "a file the install did not place should stay"
    );
    assert!(
        said.iter()
            .any(|line| line.starts_with("left") && line.contains("something-else.txt")),
        "it should say what it left behind: {said:?}"
    );
}

#[test]
fn uninstall_takes_the_install_folder_too_when_nothing_of_anyone_elses_is_in_it() {
    let scratch = Scratch::new("folder");
    let plan = plan(&scratch);
    place(&plan);

    setup::uninstall(&plan.prefix, &mut |_| {}).expect("it should uninstall");
    assert!(!plan.prefix.exists(), "the install folder should be gone");
}

#[test]
fn a_second_uninstall_is_refused_because_there_is_nothing_left_to_remove() {
    let scratch = Scratch::new("second");
    let plan = plan(&scratch);
    place(&plan);
    setup::uninstall(&plan.prefix, &mut |_| {}).expect("the first should uninstall");

    let err = setup::uninstall(&plan.prefix, &mut |_| {}).expect_err("the second should refuse");
    let message = err.to_string();
    assert!(message.contains("nothing installed"), "{message}");
    assert!(message.contains("installed-files.txt"), "{message}");
}

#[test]
fn installing_over_an_install_is_refused_rather_than_written_through() {
    let scratch = Scratch::new("over");
    let plan = plan(&scratch);
    place(&plan);

    let err = setup::install(
        &setup::Source::Directory(scratch.join("no-such-release")),
        &destination(&plan),
        "x86_64-pc-windows-msvc",
        &mut |_| {},
    )
    .expect_err("a second install should be refused");
    assert!(err.to_string().contains("already installed"), "{err}");
}

#[test]
fn a_file_changed_after_signing_is_refused_and_nothing_is_placed() {
    let scratch = Scratch::new("tampered");
    let target = "x86_64-pc-windows-msvc";
    let release = release_directory(&scratch, target, Some("RepoSphereExplorerGui"));
    let plan = plan(&scratch);

    let mut said = Vec::new();
    let err = setup::install(
        &setup::Source::Directory(release),
        &destination(&plan),
        target,
        &mut |line| said.push(line),
    )
    .expect_err("a tampered file should be refused");

    let message = err.to_string();
    assert!(
        message.contains(&format!("refusing RepoSphereExplorerGui-{target}.exe")),
        "{message}"
    );
    assert!(
        message.contains("its digest does not match the manifest"),
        "{message}"
    );
    assert!(
        !plan.prefix.exists(),
        "nothing should have been placed: {} is there",
        plan.prefix.display()
    );
    assert!(
        !said.iter().any(|line| line.starts_with("placed")),
        "it should not have placed anything: {said:?}"
    );
}

#[test]
fn a_file_the_release_did_not_sign_is_refused_even_though_its_digest_matches() {
    let scratch = Scratch::new("unsigned");
    let target = "x86_64-pc-windows-msvc";
    let release = release_directory(&scratch, target, None);
    let plan = plan(&scratch);

    let err = setup::install(
        &setup::Source::Directory(release),
        &destination(&plan),
        target,
        &mut |_| {},
    )
    .expect_err("a manifest nobody signed should be refused");
    assert!(
        err.to_string().contains("signature verification failed"),
        "{err}"
    );
    assert!(!plan.prefix.exists(), "nothing should have been placed");
}

#[test]
fn a_release_with_nothing_for_this_machine_says_so_before_it_downloads_anything() {
    let scratch = Scratch::new("notpublished");
    let release = release_directory(&scratch, "x86_64-pc-windows-msvc", None);
    let plan = plan(&scratch);

    let err = setup::install(
        &setup::Source::Directory(release),
        &destination(&plan),
        "aarch64-apple-darwin",
        &mut |_| {},
    )
    .expect_err("a release with nothing for this target should be refused");
    let message = err.to_string();
    assert!(message.contains("release 9.9.9 publishes no"), "{message}");
    assert!(message.contains("aarch64-apple-darwin"), "{message}");
}
