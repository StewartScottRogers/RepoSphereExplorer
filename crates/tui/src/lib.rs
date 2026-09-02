//! Ratatui front end: renders state and sends intents to the service.

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{Name, Stream};
use protocol::{Request, Response};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, List, ListItem, Paragraph};
use std::io;

/// Connects to the service's local socket and asks it to list `path`.
///
/// # Errors
/// Returns an error if the service cannot be reached or the round trip
/// fails.
pub fn fetch_directory_listing(socket_name: Name<'_>, path: &str) -> io::Result<Response> {
    let mut conn = Stream::connect(socket_name)?;
    protocol::write_message(
        &mut conn,
        &Request::ListDirectory {
            path: path.to_owned(),
        },
    )?;
    protocol::read_message(&mut conn)
}

/// Renders a directory listing, or an error, into `area` of `frame`.
pub fn render(frame: &mut Frame<'_>, area: Rect, response: &Response) {
    let block = Block::bordered().title("RepoSphereExplorer");
    match response {
        Response::Directory { entries } => {
            let items: Vec<ListItem<'_>> = entries
                .iter()
                .map(|entry| {
                    let label = if entry.is_dir {
                        format!("{}/", entry.name)
                    } else {
                        entry.name.clone()
                    };
                    ListItem::new(label)
                })
                .collect();
            frame.render_widget(List::new(items).block(block), area);
        }
        Response::Error { message } => {
            frame.render_widget(Paragraph::new(message.as_str()).block(block), area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{fetch_directory_listing, render};
    use interprocess::local_socket::traits::Listener as _;
    use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, ToNsName};
    use protocol::{DirectoryEntry, Request, Response};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn unique_socket_name() -> String {
        format!(
            "rse-tui-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    #[test]
    fn fetches_a_directory_listing_over_the_socket() {
        let name = unique_socket_name();
        let listener = ListenerOptions::new()
            .name(name.as_str().to_ns_name::<GenericNamespaced>().unwrap())
            .create_sync()
            .unwrap();

        let server = std::thread::spawn(move || {
            let mut conn: Stream = listener.accept().unwrap();
            let request: Request = protocol::read_message(&mut conn).unwrap();
            assert_eq!(
                request,
                Request::ListDirectory {
                    path: "some/path".to_owned()
                }
            );
            let response = Response::Directory {
                entries: vec![DirectoryEntry {
                    name: "file.txt".to_owned(),
                    is_dir: false,
                }],
            };
            protocol::write_message(&mut conn, &response).unwrap();
        });

        let response = fetch_directory_listing(
            name.as_str().to_ns_name::<GenericNamespaced>().unwrap(),
            "some/path",
        )
        .unwrap();
        server.join().unwrap();

        match response {
            Response::Directory { entries } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].name, "file.txt");
            }
            Response::Error { message } => panic!("unexpected error: {message}"),
        }
    }

    #[test]
    fn renders_directory_entries_into_the_frame() {
        let backend = TestBackend::new(20, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        let response = Response::Directory {
            entries: vec![DirectoryEntry {
                name: "src".to_owned(),
                is_dir: true,
            }],
        };

        terminal
            .draw(|frame| render(frame, frame.area(), &response))
            .unwrap();

        let contents: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(contents.contains("src/"));
    }
}
