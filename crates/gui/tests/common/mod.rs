//! What the graphical integration suites share.
//!
//! Integration test files are separate binaries, so code they have in
//! common lives in a module each of them declares - the place Cargo does
//! not mistake for a test binary of its own.

use std::sync::Once;
use std::time::{Duration, Instant};

/// Starts a service for this test binary, once, and waits until it answers.
///
/// On a socket of its own, named for this process, and bound or the test
/// fails - never a silent fallback to whatever already holds a socket. The
/// shared socket name, with that fallback, meant these suites carried out
/// real file operations and undos through another test binary's service,
/// sharing its undo journal, or through the reader's own running Repos
/// Explorer. See `protocol::use_private_socket`.
///
/// One copy, because five had already drifted: one suite never waited for
/// the service to answer, and a safety change had to be made in five places
/// at once.
pub fn ensure_service() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        assert!(
            protocol::use_private_socket(format!(
                "reposphereexplorer-test-{}.sock",
                std::process::id()
            )),
            "the socket was chosen before this harness could make it private"
        );
        let name = protocol::socket_name().expect("the platform has a socket name");
        let listener = service::bind(name).expect("a private socket is free to bind");
        std::thread::spawn(move || {
            let _ = service::run(&listener);
        });
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        use interprocess::local_socket::traits::Stream as _;
        let name = protocol::socket_name().expect("the platform has a socket name");
        if interprocess::local_socket::Stream::connect(name).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("no service answered on the local socket");
}
