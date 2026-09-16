//! Command line entry point for the setup program: reads the options,
//! decides whether there is a window, and hands the work to `setup`.

use setup::arguments;
use std::process::ExitCode;

fn main() -> ExitCode {
    let given: Vec<String> = std::env::args().skip(1).collect();
    let arguments = match arguments::parse(given.clone()) {
        Ok(arguments) => arguments,
        Err(message) => {
            eprintln!("ReposExplorerSetup: {message}\n\n{}", arguments::USAGE);
            return ExitCode::FAILURE;
        }
    };
    if arguments.help {
        println!("{}", arguments::USAGE);
        return ExitCode::SUCCESS;
    }

    let running = match std::env::current_exe() {
        Ok(running) => running,
        Err(err) => {
            eprintln!("ReposExplorerSetup: could not find this program's own file: {err}");
            return ExitCode::FAILURE;
        }
    };
    let task = match setup::task_for(&arguments, running) {
        Ok(task) => task,
        Err(err) => {
            eprintln!("ReposExplorerSetup: {err}");
            return ExitCode::FAILURE;
        }
    };

    // An uninstall started from inside the install folder - which is what
    // the Settings > Apps button does - cannot delete the file it is running
    // from, so it starts a copy somewhere else and leaves the field to it.
    if let setup::window::Task::Uninstall(prefix) = &task {
        match setup::relaunch_outside(prefix, &given) {
            Ok(true) => return ExitCode::SUCCESS,
            Ok(false) => {}
            Err(err) => {
                eprintln!("ReposExplorerSetup: {err}");
                return ExitCode::FAILURE;
            }
        }
    }

    if arguments.quiet {
        return match task.run(&mut |line| println!("{line}")) {
            Ok(_) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("ReposExplorerSetup: {err}");
                ExitCode::FAILURE
            }
        };
    }
    match setup::window::show(&task) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("ReposExplorerSetup: could not open a window: {err}");
            ExitCode::FAILURE
        }
    }
}
