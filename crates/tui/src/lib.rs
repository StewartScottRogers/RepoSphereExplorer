//! Ratatui front end: renders state and sends intents to the service.

pub mod app;

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{Name, Stream};
use plugin_api::PluginPresentation;
use protocol::{Request, Response};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, List, ListItem, Paragraph};
use std::io;

/// Connects to the service's local socket and sends it `request`.
///
/// # Errors
/// Returns an error if the service cannot be reached or the round trip
/// fails.
pub fn send_request(socket_name: Name<'_>, request: &Request) -> io::Result<Response> {
    let mut conn = Stream::connect(socket_name)?;
    protocol::write_message(&mut conn, request)?;
    protocol::read_message(&mut conn)
}

/// Every presentation plugin linked into this front end.
///
/// Hand-registered: a registration macro would be structure with no second
/// caller to justify it while seven entries can still be read at a glance
/// (see `plugin-api`'s crate docs).
const PRESENTATION_PLUGINS: &[&dyn PluginPresentation] = &[
    &plugin_text::TextPresentation,
    &plugin_python::PythonPresentation,
    &plugin_elixir::ElixirPresentation,
    &plugin_ruby::RubyPresentation,
    &plugin_php::PhpPresentation,
    &plugin_perl::PerlPresentation,
    &plugin_javascript::JavaScriptPresentation,
    &plugin_typescript::TypeScriptPresentation,
    &plugin_rust::RustPresentation,
    &plugin_go::GoPresentation,
    &plugin_java::JavaPresentation,
    &plugin_kotlin::KotlinPresentation,
    &plugin_csharp::CSharpPresentation,
    &plugin_objective_c::ObjectiveCPresentation,
    &plugin_cpp::CppPresentation,
    &plugin_c::CPresentation,
    &plugin_swift::SwiftPresentation,
    &plugin_dockerfile::DockerfilePresentation,
    &plugin_shell::ShellPresentation,
    &plugin_powershell::PowerShellPresentation,
    &plugin_r::RPresentation,
    &plugin_haskell::HaskellPresentation,
    &plugin_scala::ScalaPresentation,
    &plugin_sql::SqlPresentation,
    &plugin_clojure::ClojurePresentation,
    &plugin_dart::DartPresentation,
    &plugin_erlang::ErlangPresentation,
    &plugin_julia::JuliaPresentation,
    &plugin_fortran::FortranPresentation,
    &plugin_assembly::AssemblyPresentation,
    &plugin_vimscript::VimscriptPresentation,
    &plugin_graphql::GraphQlPresentation,
    &plugin_solidity::SolidityPresentation,
    &plugin_svg::SvgPresentation,
    &plugin_html::HtmlPresentation,
    &plugin_xml::XmlPresentation,
    &plugin_restructuredtext::RestructuredTextPresentation,
    &plugin_jupyter_notebook::NotebookPresentation,
    &plugin_json::JsonPresentation,
    &plugin_toml::TomlPresentation,
    &plugin_csv::CsvPresentation,
    &plugin_msgpack::MsgpackPresentation,
    &plugin_makefile::MakefilePresentation,
    &plugin_image::ImagePresentation,
    &plugin_word_document::WordDocumentPresentation,
    &plugin_spreadsheet::SpreadsheetPresentation,
    &plugin_presentation::PresentationPresentation,
    &plugin_epub::EpubPresentation,
    &plugin_comic_archive::ComicArchivePresentation,
    &plugin_video::VideoPresentation,
    &plugin_audio::AudioPresentation,
    &plugin_archive::ArchivePresentation,
    &plugin_pdf::PdfPresentation,
    &plugin_parquet::ParquetPresentation,
    &plugin_avro::AvroPresentation,
    &plugin_sqlite::SqlitePresentation,
    &plugin_directory::DirectoryPresentation,
];

/// Turns a plugin's view data into displayable lines, via whichever
/// registered presentation plugin matches `plugin`.
fn present(plugin: &str, data: &serde_json::Value) -> Vec<String> {
    match PRESENTATION_PLUGINS
        .iter()
        .find(|candidate| candidate.name() == plugin)
    {
        Some(candidate) => candidate.present(data),
        None => vec![format!("no presentation for plugin `{plugin}`")],
    }
}

/// Renders a directory listing, a file view, or an error, into `area` of
/// `frame`, inside `block`.
pub(crate) fn render_with_block(
    frame: &mut Frame<'_>,
    area: Rect,
    response: &Response,
    block: Block<'_>,
) {
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
        Response::FileView { plugin, data } => {
            let lines = present(plugin, data);
            frame.render_widget(Paragraph::new(lines.join("\n")).block(block), area);
        }
        Response::Error { message } => {
            frame.render_widget(Paragraph::new(message.as_str()).block(block), area);
        }
        Response::Done => {
            frame.render_widget(Paragraph::new("done").block(block), area);
        }
    }
}

/// Renders a directory listing, a file view, or an error, into `area` of
/// `frame`.
pub fn render(frame: &mut Frame<'_>, area: Rect, response: &Response) {
    render_with_block(
        frame,
        area,
        response,
        Block::bordered().title("RepoSphereExplorer"),
    );
}

#[cfg(test)]
mod tests {
    use super::{render, send_request};
    use interprocess::local_socket::traits::Listener as _;
    use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, ToNsName};
    use protocol::{DirectoryEntry, Request, Response};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn unique_socket_name() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        format!(
            "rse-tui-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
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

        let response = send_request(
            name.as_str().to_ns_name::<GenericNamespaced>().unwrap(),
            &Request::ListDirectory {
                path: "some/path".to_owned(),
            },
        )
        .unwrap();
        server.join().unwrap();

        match response {
            Response::Directory { entries } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].name, "file.txt");
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn renders_a_file_view_through_its_plugin() {
        let backend = TestBackend::new(20, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        let response = Response::FileView {
            plugin: "text".to_owned(),
            data: serde_json::json!({ "content": "hi", "truncated": false }),
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
        assert!(contents.contains("hi"));
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
