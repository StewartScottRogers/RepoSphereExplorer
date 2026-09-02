//! Command line entry point for `RepoSphereExplorer`.

use clap::{Parser, Subcommand};
use repo_sphere_explorer::describe;

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
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Explore { target } => println!("{}", describe(&target)),
    }
}
