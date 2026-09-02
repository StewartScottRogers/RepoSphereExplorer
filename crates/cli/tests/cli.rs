//! End-to-end tests for the command line interface.

use assert_cmd::Command;
use predicates::str::contains;

#[test]
fn explore_prints_the_target() {
    Command::cargo_bin("repo_sphere_explorer")
        .unwrap()
        .args(["explore", "acme/widgets"])
        .assert()
        .success()
        .stdout(contains("acme/widgets"));
}

#[test]
fn missing_subcommand_is_an_error() {
    Command::cargo_bin("repo_sphere_explorer")
        .unwrap()
        .assert()
        .failure();
}
