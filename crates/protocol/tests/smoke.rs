//! The `smoke` binary against a stand-in service on a private socket.

use interprocess::local_socket::traits::Listener as _;
use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, ListenerOptions, NameType, ToFsName, ToNsName,
};
use protocol::{DirectoryEntry, Request, Response};
use std::process::Command;
use std::thread;

/// Answers one `ListDirectory` with `names`, on a socket of this test's own,
/// and returns that socket's name.
fn stand_in_service(
    test: &str,
    names: &'static [&'static str],
) -> (String, thread::JoinHandle<Request>) {
    let socket = format!("rse-smoke-{test}-{}.sock", std::process::id());
    let name = if GenericNamespaced::is_supported() {
        socket.clone().to_ns_name::<GenericNamespaced>().unwrap()
    } else {
        std::env::temp_dir()
            .join(&socket)
            .to_fs_name::<GenericFilePath>()
            .unwrap()
    };
    let listener = ListenerOptions::new().name(name).create_sync().unwrap();
    let server = thread::spawn(move || {
        let mut conn = listener.accept().unwrap();
        let request: Request = protocol::read_message(&mut conn).unwrap();
        let entries = names
            .iter()
            .map(|name| DirectoryEntry {
                name: (*name).to_owned(),
                is_dir: false,
                size: 0,
                modified: None,
                repository: None,
            })
            .collect();
        protocol::write_message(&mut conn, &Response::Directory { entries }).unwrap();
        request
    });
    (socket, server)
}

#[test]
fn succeeds_when_the_service_lists_the_expected_entry() {
    let (socket, server) = stand_in_service("found", &["marker.txt", "other"]);

    let output = Command::new(env!("CARGO_BIN_EXE_smoke"))
        .args(["--socket", &socket, "/scratch", "marker.txt"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        server.join().unwrap(),
        Request::ListDirectory {
            path: "/scratch".to_owned()
        }
    );
}

#[test]
fn fails_when_the_listing_lacks_the_expected_entry() {
    let (socket, server) = stand_in_service("missing", &["other"]);

    let output = Command::new(env!("CARGO_BIN_EXE_smoke"))
        .args(["--socket", &socket, "/scratch", "marker.txt"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not include marker.txt"));
    server.join().unwrap();
}
