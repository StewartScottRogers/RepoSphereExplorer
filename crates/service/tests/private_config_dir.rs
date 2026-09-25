//! [`service::repos::use_private_config_dir`], proven in a process of its
//! own.
//!
//! The Repos Directory list `set_active_root` writes lives at a fixed,
//! machine-wide path (`repos.rs`'s `config_path`), so a test that exercised
//! a real write without redirecting it would corrupt whoever's settings are
//! really stored there. This is the one test that settles the override, and
//! it lives in a binary of its own - `cargo test` gives every integration
//! test file its own process - so nothing else in the crate can call
//! `config_path` before it, or in between its own before-and-after reads,
//! the way two tests sharing one binary could race each other.

use std::path::PathBuf;

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rse-private-config-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

#[test]
fn a_private_config_dir_keeps_a_real_write_off_the_machines_own_settings() {
    let dir = scratch();
    assert!(
        service::repos::use_private_config_dir(dir.join("data")),
        "nothing in this process should have settled the config directory yet"
    );

    let target = dir.join("workspace");
    std::fs::create_dir_all(&target).expect("a directory to become the active root");
    service::repos::set_active_root(&target).expect("a real directory is accepted");

    assert_eq!(
        service::repos::roots()
            .into_iter()
            .find(|root| root.active)
            .map(|root| root.path),
        Some(target.to_string_lossy().into_owned()),
        "the write should have landed under the private directory, not the real one"
    );
    assert!(
        dir.join("data")
            .join("RepoSphereExplorer")
            .join("repos.json")
            .is_file(),
        "and it should be readable back from exactly there"
    );
}
