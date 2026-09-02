//! Command line entry point for `RepoSphereExplorer`.

use clap::{Parser, Subcommand};
use repo_sphere_explorer::describe;
use std::process::ExitCode;

/// Explores a sphere of GitHub repositories.
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Register a target and report what the explorer would do with it.
    Explore {
        /// Repository in `owner/name` form.
        target: String,
    },
    /// Checks for and applies an update to this binary.
    SelfUpdate,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Explore { target } => {
            println!("{}", describe(&target));
            ExitCode::SUCCESS
        }
        Command::SelfUpdate => self_update(),
    }
}

fn self_update() -> ExitCode {
    match updater::self_update("repo_sphere_explorer") {
        Ok(updater::Outcome::UpToDate { version }) => {
            println!("repo_sphere_explorer is up to date (v{version})");
            ExitCode::SUCCESS
        }
        Ok(updater::Outcome::Updated { from, to }) => {
            println!("repo_sphere_explorer updated: v{from} -> v{to}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("update failed: {err}");
            ExitCode::FAILURE
        }
    }
}
