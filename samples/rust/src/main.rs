//! Prints an index of the directory named on the command line.
//!
//! Argument parsing only; everything else is in the library, so it can be
//! tested without a process.

use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(root) = std::env::args().nth(1) else {
        eprintln!("usage: tree-indexer <directory>");
        return ExitCode::from(2);
    };

    match tree_indexer::index(std::path::Path::new(&root)) {
        Ok(entries) => {
            for entry in &entries {
                println!("{}", entry.relative.display());
            }
            eprintln!("{} entries", entries.len());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("tree-indexer: {err}");
            ExitCode::FAILURE
        }
    }
}
