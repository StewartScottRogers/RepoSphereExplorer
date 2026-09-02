//! The fat process: filesystem traversal, indexing, operations, and plugin cores.

use interprocess::local_socket::traits::Listener as _;
use interprocess::local_socket::{Listener, ListenerOptions, Name, Stream};
use plugin_api::PluginCore;
use protocol::{DirectoryEntry, Request, Response};
use std::fs;
use std::io;
use std::io::Read;
use std::path::Path;

/// Number of bytes read from the start of a file when sniffing its type.
const SNIFF_PREFIX_LEN: u64 = 512;

/// Every core plugin linked into this service, in sniffing priority order.
///
/// Hand-registered: with a single plugin, a registration macro would be
/// structure with no second caller to justify it (see `plugin-api`'s crate
/// docs).
const CORE_PLUGINS: &[&dyn PluginCore] = &[&plugin_text::TextCore];

/// Lists the immediate contents of `path`, sorted by name.
///
/// # Errors
/// Returns an error if `path` cannot be read as a directory.
pub fn list_directory(path: &Path) -> io::Result<Vec<DirectoryEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type()?.is_dir();
        entries.push(DirectoryEntry { name, is_dir });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Reads a bounded prefix from the start of the file at `path`.
fn read_prefix(path: &Path) -> io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    let mut buf = Vec::new();
    file.take(SNIFF_PREFIX_LEN).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Finds the first registered plugin that recognises `path`'s content.
fn sniff(path: &Path) -> io::Result<Option<&'static dyn PluginCore>> {
    let prefix = read_prefix(path)?;
    Ok(CORE_PLUGINS
        .iter()
        .find(|plugin| plugin.sniff(&prefix))
        .copied())
}

/// Views the file at `path` through whichever registered plugin recognises
/// it.
///
/// # Errors
/// Returns an error if `path` cannot be read.
pub fn view_file(path: &Path) -> io::Result<Response> {
    Ok(match sniff(path)? {
        Some(plugin) => Response::FileView {
            plugin: plugin.name().to_owned(),
            data: plugin.view(path)?,
        },
        None => Response::Error {
            message: format!("no plugin recognises {}", path.display()),
        },
    })
}

/// Lists `path` if it is a directory, otherwise views it through whichever
/// registered plugin recognises it.
fn open(path: &Path) -> Response {
    let result = match fs::metadata(path) {
        Ok(meta) if meta.is_dir() => {
            list_directory(path).map(|entries| Response::Directory { entries })
        }
        Ok(_) => view_file(path),
        Err(err) => Err(err),
    };
    result.unwrap_or_else(|err| Response::Error {
        message: err.to_string(),
    })
}

/// Computes the response for one request.
#[must_use]
pub fn handle_request(request: &Request) -> Response {
    match request {
        Request::ListDirectory { path } => match list_directory(Path::new(path)) {
            Ok(entries) => Response::Directory { entries },
            Err(err) => Response::Error {
                message: err.to_string(),
            },
        },
        Request::ViewFile { path } => {
            view_file(Path::new(path)).unwrap_or_else(|err| Response::Error {
                message: err.to_string(),
            })
        }
        Request::Open { path } => open(Path::new(path)),
    }
}

/// Starts listening on the local socket identified by `name`.
///
/// # Errors
/// Returns an error if the socket is already in use or cannot be created.
pub fn bind(name: Name<'_>) -> io::Result<Listener> {
    ListenerOptions::new().name(name).create_sync()
}

/// Accepts one connection on `listener`, answers exactly one request on it,
/// then returns.
///
/// # Errors
/// Returns an error if accepting the connection or the request/response
/// round trip fails.
pub fn serve_one(listener: &Listener) -> io::Result<()> {
    let mut conn: Stream = listener.accept()?;
    let request: Request = protocol::read_message(&mut conn)?;
    let response = handle_request(&request);
    protocol::write_message(&mut conn, &response)
}

/// Runs the service loop: accepts connections and answers one request on
/// each, forever.
///
/// # Errors
/// Never returns `Ok`; this signature only exists so callers can use `?`.
pub fn run(listener: &Listener) -> io::Result<()> {
    loop {
        if let Err(err) = serve_one(listener) {
            eprintln!("connection error: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{bind, handle_request, list_directory, open, serve_one, view_file};
    use interprocess::local_socket::traits::Stream as _;
    use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};
    use protocol::{Request, Response};
    use std::fs;

    fn unique_socket_name() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        format!(
            "rse-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }

    #[test]
    fn lists_a_directory_sorted_by_name() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("b.txt"), b"").unwrap();
        fs::write(dir.join("a.txt"), b"").unwrap();

        let entries = list_directory(&dir).unwrap();

        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["a.txt", "b.txt", "sub"]
        );
        assert!(entries.iter().find(|e| e.name == "sub").unwrap().is_dir);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn views_a_text_file_through_the_text_plugin() {
        let path = std::env::temp_dir().join(unique_socket_name());
        fs::write(&path, "hello\nworld\n").unwrap();

        let response = view_file(&path).unwrap();

        match response {
            Response::FileView { plugin, data } => {
                assert_eq!(plugin, "text");
                assert_eq!(data["content"], "hello\nworld\n");
            }
            other => panic!("unexpected response: {other:?}"),
        }

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn opens_a_directory_as_a_listing_and_a_file_as_a_view() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("note.txt");
        fs::write(&file, "hi").unwrap();

        assert!(matches!(open(&dir), Response::Directory { .. }));
        assert!(matches!(open(&file), Response::FileView { .. }));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reports_an_error_for_a_missing_directory() {
        let missing = std::env::temp_dir().join(unique_socket_name());
        let request = Request::ListDirectory {
            path: missing.to_string_lossy().into_owned(),
        };

        assert!(matches!(handle_request(&request), Response::Error { .. }));
    }

    #[test]
    fn answers_a_list_directory_request_over_the_socket() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("file.txt"), b"").unwrap();

        let name = unique_socket_name();
        let listener = bind(name.as_str().to_ns_name::<GenericNamespaced>().unwrap()).unwrap();

        let dir_for_client = dir.clone();
        let client_name = name.clone();
        let client = std::thread::spawn(move || {
            let mut conn = Stream::connect(
                client_name
                    .as_str()
                    .to_ns_name::<GenericNamespaced>()
                    .unwrap(),
            )
            .unwrap();
            protocol::write_message(
                &mut conn,
                &Request::ListDirectory {
                    path: dir_for_client.to_string_lossy().into_owned(),
                },
            )
            .unwrap();
            protocol::read_message::<Response, _>(&mut conn).unwrap()
        });

        serve_one(&listener).unwrap();
        let response = client.join().unwrap();

        match response {
            Response::Directory { entries } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].name, "file.txt");
            }
            other => panic!("unexpected response: {other:?}"),
        }

        fs::remove_dir_all(&dir).unwrap();
    }
}
