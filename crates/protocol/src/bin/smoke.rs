//! Proves a running service answers a real protocol request, for
//! `distribution.yml`'s check of an installed copy.
//!
//! Usage: `smoke [--socket <name>] <directory> <expected-entry>`
//!
//! Waits up to fifteen seconds for the service's socket, asks it to list
//! `<directory>`, and succeeds only if the listing names `<expected-entry>`.
//! It never starts a service itself: the point is to talk to the one that
//! was installed. `--socket` is for this binary's own test, which cannot
//! use the shared name without meeting whatever service the machine runs.

use interprocess::local_socket::Stream;
use interprocess::local_socket::traits::Stream as _;
use protocol::{Request, Response};
use std::process::ExitCode;
use std::time::{Duration, Instant};

const WAIT_FOR_SERVICE: Duration = Duration::from_secs(15);

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--socket") && args.len() > 1 {
        let name = args.remove(1);
        args.remove(0);
        let _ = protocol::use_private_socket(name);
    }
    let [directory, expected] = args.as_slice() else {
        eprintln!("usage: smoke [--socket <name>] <directory> <expected-entry>");
        return ExitCode::from(2);
    };

    match list(directory) {
        Ok(Response::Directory { entries }) => {
            let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
            println!("the service listed {directory}: {names:?}");
            if names.contains(&expected.as_str()) {
                ExitCode::SUCCESS
            } else {
                eprintln!("the listing does not include {expected}");
                ExitCode::FAILURE
            }
        }
        Ok(other) => {
            eprintln!("the service answered with something other than a listing: {other:?}");
            ExitCode::FAILURE
        }
        Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
            // What a truncated reply used to say was "failed to fill whole
            // buffer", which is the io error's own words for "the bytes
            // stopped coming" and tells a reader nothing about where to
            // look. It is worth naming: the service was there, took the
            // request, and went away before finishing the answer - which is
            // what a service refusing a connection after accepting it looks
            // like from here, and is how #749 presented.
            eprintln!(
                "the service accepted the connection and then closed it \
                 before finishing its answer"
            );
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("no answer from the service: {err}");
            ExitCode::FAILURE
        }
    }
}

fn list(directory: &str) -> std::io::Result<Response> {
    let deadline = Instant::now() + WAIT_FOR_SERVICE;
    let mut conn = loop {
        match Stream::connect(protocol::socket_name()?) {
            Ok(conn) => break conn,
            Err(err) if Instant::now() >= deadline => return Err(err),
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    };
    protocol::write_message(
        &mut conn,
        &Request::ListDirectory {
            path: directory.to_owned(),
        },
    )?;
    protocol::read_message(&mut conn)
}
