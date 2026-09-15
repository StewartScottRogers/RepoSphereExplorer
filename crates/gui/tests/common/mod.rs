//! What every graphical integration suite needs before it can open a
//! window: a service to talk to. Shared here so a suite has one
//! `ensure_service` to call rather than a copy to keep in step with the
//! other suites' copies.

use std::sync::Once;
use std::time::{Duration, Instant};

/// Makes sure something is answering on the service's local socket. A
/// service the developer already has running answers; otherwise one is run
/// on a background thread of this test process.
pub fn ensure_service() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        // A service of this test binary's own, on a socket nobody else
        // holds - never the reader's running service, and never another
        // test binary's, whose undo journal it would share. See
        // `protocol::use_private_socket`.
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
