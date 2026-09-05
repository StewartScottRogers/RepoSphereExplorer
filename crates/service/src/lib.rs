//! The fat process: filesystem traversal, indexing, operations, and plugin cores.

use interprocess::local_socket::traits::Listener as _;
use interprocess::local_socket::{Listener, ListenerOptions, Name, Stream};
use plugin_api::PluginCore;
use protocol::{DirectoryEntry, Request, Response};
use serde::Serialize;
use std::fs;
use std::io;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};

/// Number of bytes read from the start of a file when sniffing its type.
///
/// Large enough to cover ISO 9660's Primary Volume Descriptor, whose
/// `CD001` standard identifier sits at offset 32769 rather than near the
/// start of the file like every other sniffed format's magic.
const SNIFF_PREFIX_LEN: u64 = 32_774;

/// Every content-sniffed core plugin linked into this service, in sniffing
/// priority order. The directory plugin is dispatched separately (see
/// [`view_file`]) since it has no file bytes to sniff.
///
/// Hand-registered: a registration macro would be structure with no second
/// caller to justify it while twelve entries can still be read at a glance
/// (see `plugin-api`'s crate docs).
const CORE_PLUGINS: &[&dyn PluginCore] = &[
    &plugin_perl::PerlCore,
    &plugin_php::PhpCore,
    &plugin_elixir::ElixirCore,
    &plugin_ruby::RubyCore,
    &plugin_python::PythonCore,
    &plugin_svelte::SvelteCore,
    &plugin_typescript::TypeScriptCore,
    &plugin_javascript::JavaScriptCore,
    &plugin_rust::RustCore,
    &plugin_makefile::MakefileCore,
    &plugin_go::GoCore,
    &plugin_java::JavaCore,
    &plugin_kotlin::KotlinCore,
    &plugin_groovy::GroovyCore,
    &plugin_csharp::CSharpCore,
    &plugin_objective_c::ObjectiveCCore,
    &plugin_cpp::CppCore,
    &plugin_c::CCore,
    &plugin_swift::SwiftCore,
    &plugin_dockerfile::DockerfileCore,
    &plugin_shell::ShellCore,
    &plugin_powershell::PowerShellCore,
    &plugin_r::RCore,
    &plugin_haskell::HaskellCore,
    &plugin_fsharp::FSharpCore,
    &plugin_ocaml::OCamlCore,
    &plugin_nim::NimCore,
    &plugin_elm::ElmCore,
    &plugin_scala::ScalaCore,
    &plugin_sql::SqlCore,
    &plugin_clojure::ClojureCore,
    &plugin_dart::DartCore,
    &plugin_erlang::ErlangCore,
    &plugin_julia::JuliaCore,
    &plugin_fortran::FortranCore,
    &plugin_assembly::AssemblyCore,
    &plugin_vimscript::VimscriptCore,
    &plugin_graphql::GraphQlCore,
    &plugin_solidity::SolidityCore,
    &plugin_svg::SvgCore,
    &plugin_vue::VueCore,
    &plugin_html::HtmlCore,
    &plugin_xml::XmlCore,
    &plugin_restructuredtext::RestructuredTextCore,
    &plugin_jupyter_notebook::NotebookCore,
    &plugin_model3d::Model3dCore,
    &plugin_geojson::GeoJsonCore,
    &plugin_json::JsonCore,
    &plugin_toml::TomlCore,
    &plugin_csv::CsvCore,
    &plugin_msgpack::MsgpackCore,
    &plugin_certificate::CertificateCore,
    &plugin_text::TextCore,
    &plugin_image::ImageCore,
    &plugin_psd::PsdCore,
    &plugin_font::FontCore,
    &plugin_executable::ExecutableCore,
    &plugin_wasm::WasmCore,
    &plugin_word_document::WordDocumentCore,
    &plugin_spreadsheet::SpreadsheetCore,
    &plugin_presentation::PresentationCore,
    &plugin_epub::EpubCore,
    &plugin_comic_archive::ComicArchiveCore,
    &plugin_video::VideoCore,
    &plugin_audio::AudioCore,
    &plugin_archive::ArchiveCore,
    &plugin_pdf::PdfCore,
    &plugin_parquet::ParquetCore,
    &plugin_avro::AvroCore,
    &plugin_sqlite::SqliteCore,
    &plugin_hdf5::Hdf5Core,
    &plugin_disk_image::DiskImageCore,
    &plugin_package_archive::PackageArchiveCore,
];

/// The directory-as-file plugin, dispatched directly by [`view_file`] when
/// the path is a directory rather than through content sniffing.
const DIRECTORY_PLUGIN: &dyn PluginCore = &plugin_directory::DirectoryCore;

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

/// Views the path through whichever registered plugin recognises it: the
/// directory plugin if `path` is a directory, otherwise whichever content
/// plugin's `sniff` matches.
///
/// # Errors
/// Returns an error if `path` cannot be read.
pub fn view_file(path: &Path) -> io::Result<Response> {
    if fs::metadata(path)?.is_dir() {
        return Ok(Response::FileView {
            plugin: DIRECTORY_PLUGIN.name().to_owned(),
            data: DIRECTORY_PLUGIN.view(path)?,
        });
    }
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

/// Where operations are journaled by default:
/// `<data-local-dir>/RepoSphereExplorer/journal.jsonl`.
fn default_journal_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("RepoSphereExplorer").join("journal.jsonl"))
}

/// One line of the operations journal: GUIDANCE.md §2.1.5 requires
/// destructive operations to be "journaled so the action can be described
/// after the fact".
#[derive(Debug, Serialize)]
struct JournalEntry<'a> {
    at_unix_secs: u64,
    operation: &'a str,
    targets: &'a [String],
    outcome: &'a str,
}

/// Appends one line describing `operation` on `targets` to the journal at
/// `path`. Best-effort: a journaling failure is reported to stderr and
/// never propagated, so it can't block the operation it's recording.
fn journal_to(path: &Path, operation: &str, targets: &[String], outcome: &io::Result<()>) {
    let entry = JournalEntry {
        at_unix_secs: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs()),
        operation,
        targets,
        outcome: &match outcome {
            Ok(()) => "ok".to_owned(),
            Err(err) => err.to_string(),
        },
    };
    let Ok(line) = serde_json::to_string(&entry) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut file) => {
            if let Err(err) = writeln!(file, "{line}") {
                eprintln!("could not write journal entry: {err}");
            }
        }
        Err(err) => eprintln!("could not open journal at {}: {err}", path.display()),
    }
}

/// Appends one line to the default journal, if one is resolvable. See
/// [`journal_to`].
fn journal(operation: &str, targets: &[String], outcome: &io::Result<()>) {
    if let Some(path) = default_journal_path() {
        journal_to(&path, operation, targets, outcome);
    }
}

/// Renames (moves) `from` to `to`, journaling the attempt.
///
/// # Errors
/// Returns an error if the rename fails.
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    let result = fs::rename(from, to);
    journal(
        "rename",
        &[from.display().to_string(), to.display().to_string()],
        &result,
    );
    result
}

/// Copies the file at `from` to `to`, journaling the attempt.
///
/// # Errors
/// Returns an error if the copy fails.
pub fn copy(from: &Path, to: &Path) -> io::Result<()> {
    let result = fs::copy(from, to).map(|_| ());
    journal(
        "copy",
        &[from.display().to_string(), to.display().to_string()],
        &result,
    );
    result
}

/// Deletes every path in `paths` - the exact, confirmed target set per
/// GUIDANCE.md §2.1.5, never a pattern the service resolves itself -
/// journaling the attempt.
///
/// # Errors
/// Returns an error if any path cannot be deleted; earlier paths in the
/// list may already have been removed.
pub fn delete(paths: &[String]) -> io::Result<()> {
    let result = (|| {
        for path in paths {
            let path = Path::new(path);
            if fs::metadata(path)?.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    })();
    journal("delete", paths, &result);
    result
}

/// Extracts the archive at `archive` into `destination`, journaling the
/// attempt.
///
/// # Errors
/// Returns an error if the archive cannot be extracted.
pub fn extract(archive: &Path, destination: &Path) -> io::Result<()> {
    let result = plugin_archive::extract(archive, destination);
    journal(
        "extract",
        &[
            archive.display().to_string(),
            destination.display().to_string(),
        ],
        &result,
    );
    result
}

/// Runs `operation` and turns its result into a [`Response`].
fn respond_to_operation(operation: io::Result<()>) -> Response {
    match operation {
        Ok(()) => Response::Done,
        Err(err) => Response::Error {
            message: err.to_string(),
        },
    }
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
        Request::Rename { from, to } => {
            respond_to_operation(rename(Path::new(from), Path::new(to)))
        }
        Request::Copy { from, to } => respond_to_operation(copy(Path::new(from), Path::new(to))),
        Request::Delete { paths } => respond_to_operation(delete(paths)),
        Request::Extract {
            archive,
            destination,
        } => respond_to_operation(extract(Path::new(archive), Path::new(destination))),
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
    use super::{
        bind, copy, delete, extract, handle_request, journal_to, list_directory, open, rename,
        serve_one, view_file,
    };
    use interprocess::local_socket::traits::Stream as _;
    use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};
    use protocol::{Request, Response};
    use std::fs;
    use std::io;

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
    fn views_a_directory_through_the_directory_plugin() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("note.txt"), b"hi").unwrap();

        let response = view_file(&dir).unwrap();

        match response {
            Response::FileView { plugin, data } => {
                assert_eq!(plugin, "directory");
                assert_eq!(data["entry_count"], 1);
            }
            other => panic!("unexpected response: {other:?}"),
        }

        fs::remove_dir_all(&dir).unwrap();
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
    fn renames_a_real_file() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let from = dir.join("old.txt");
        let to = dir.join("new.txt");
        fs::write(&from, "content").unwrap();

        rename(&from, &to).unwrap();

        assert!(!from.exists());
        assert_eq!(fs::read_to_string(&to).unwrap(), "content");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copies_a_real_file_leaving_the_source_in_place() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let from = dir.join("source.txt");
        let to = dir.join("copy.txt");
        fs::write(&from, "content").unwrap();

        copy(&from, &to).unwrap();

        assert_eq!(fs::read_to_string(&from).unwrap(), "content");
        assert_eq!(fs::read_to_string(&to).unwrap(), "content");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn deletes_exactly_the_given_files_and_directories() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(dir.join("sub")).unwrap();
        let file = dir.join("a.txt");
        let kept = dir.join("b.txt");
        fs::write(&file, "a").unwrap();
        fs::write(&kept, "b").unwrap();

        delete(&[
            file.to_string_lossy().into_owned(),
            dir.join("sub").to_string_lossy().into_owned(),
        ])
        .unwrap();

        assert!(!file.exists());
        assert!(!dir.join("sub").exists());
        assert!(kept.exists());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn extracts_an_archive_via_the_archive_plugins_operation() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let archive_path = dir.join("test.zip");
        let file = fs::File::create(&archive_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("inside.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut writer, b"payload").unwrap();
        writer.finish().unwrap();
        let destination = dir.join("out");

        extract(&archive_path, &destination).unwrap();

        assert_eq!(
            fs::read_to_string(destination.join("inside.txt")).unwrap(),
            "payload"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn journal_to_appends_a_line_describing_the_outcome() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let journal_path = dir.join("journal.jsonl");

        journal_to(
            &journal_path,
            "rename",
            &["a.txt".to_owned(), "b.txt".to_owned()],
            &Ok(()),
        );
        journal_to(
            &journal_path,
            "delete",
            &["c.txt".to_owned()],
            &Err(io::Error::other("boom")),
        );

        let contents = fs::read_to_string(&journal_path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);

        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["operation"], "rename");
        assert_eq!(first["outcome"], "ok");
        assert_eq!(first["targets"], serde_json::json!(["a.txt", "b.txt"]));

        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["operation"], "delete");
        assert_eq!(second["outcome"], "boom");

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
