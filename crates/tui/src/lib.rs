//! Ratatui front end: renders state and sends intents to the service.

pub mod app;

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{Name, Stream};
use plugin_api::{FolderPresentation, PluginPresentation};
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
    &plugin_crystal::CrystalPresentation,
    &plugin_ruby::RubyPresentation,
    &plugin_php::PhpPresentation,
    &plugin_perl::PerlPresentation,
    &plugin_prolog::PrologPresentation,
    &plugin_javascript::JavaScriptPresentation,
    &plugin_svelte::SveltePresentation,
    &plugin_typescript::TypeScriptPresentation,
    &plugin_rust::RustPresentation,
    &plugin_go::GoPresentation,
    &plugin_java::JavaPresentation,
    &plugin_kotlin::KotlinPresentation,
    &plugin_groovy::GroovyPresentation,
    &plugin_csharp::CSharpPresentation,
    &plugin_vbnet::VbNetPresentation,
    &plugin_objective_c::ObjectiveCPresentation,
    &plugin_cpp::CppPresentation,
    &plugin_c::CPresentation,
    &plugin_swift::SwiftPresentation,
    &plugin_dockerfile::DockerfilePresentation,
    &plugin_shell::ShellPresentation,
    &plugin_powershell::PowerShellPresentation,
    &plugin_tcl::TclPresentation,
    &plugin_r::RPresentation,
    &plugin_haskell::HaskellPresentation,
    &plugin_fsharp::FSharpPresentation,
    &plugin_ocaml::OCamlPresentation,
    &plugin_nim::NimPresentation,
    &plugin_elm::ElmPresentation,
    &plugin_scala::ScalaPresentation,
    &plugin_sql::SqlPresentation,
    &plugin_clojure::ClojurePresentation,
    &plugin_scheme::SchemePresentation,
    &plugin_dart::DartPresentation,
    &plugin_erlang::ErlangPresentation,
    &plugin_julia::JuliaPresentation,
    &plugin_fortran::FortranPresentation,
    &plugin_ada::AdaPresentation,
    &plugin_assembly::AssemblyPresentation,
    &plugin_vimscript::VimscriptPresentation,
    &plugin_graphql::GraphQlPresentation,
    &plugin_solidity::SolidityPresentation,
    &plugin_svg::SvgPresentation,
    &plugin_vue::VuePresentation,
    &plugin_html::HtmlPresentation,
    &plugin_xml::XmlPresentation,
    &plugin_restructuredtext::RestructuredTextPresentation,
    &plugin_jupyter_notebook::NotebookPresentation,
    &plugin_json::JsonPresentation,
    &plugin_terraform::TerraformPresentation,
    &plugin_toml::TomlPresentation,
    &plugin_csv::CsvPresentation,
    &plugin_msgpack::MsgpackPresentation,
    &plugin_makefile::MakefilePresentation,
    &plugin_image::ImagePresentation,
    &plugin_psd::PsdPresentation,
    &plugin_font::FontPresentation,
    &plugin_executable::ExecutablePresentation,
    &plugin_wasm::WasmPresentation,
    &plugin_model3d::Model3dPresentation,
    &plugin_geojson::GeoJsonPresentation,
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
    &plugin_hdf5::Hdf5Presentation,
    &plugin_disk_image::DiskImagePresentation,
    &plugin_package_archive::PackageArchivePresentation,
    &plugin_certificate::CertificatePresentation,
    &plugin_directory::DirectoryPresentation,
    &plugin_markdown::MarkdownPresentation,
    &plugin_yaml::YamlPresentation,
    &plugin_ini::IniPresentation,
    &plugin_properties::PropertiesPresentation,
    &plugin_diff::DiffPresentation,
    &plugin_asciidoc::AsciidocPresentation,
    &plugin_orgmode::OrgmodePresentation,
    &plugin_latex::LatexPresentation,
    &plugin_bibtex::BibtexPresentation,
    &plugin_editorconfig::EditorconfigPresentation,
    &plugin_dotenv::DotenvPresentation,
    &plugin_codeowners::CodeownersPresentation,
    &plugin_cargolock::CargolockPresentation,
    &plugin_npmlock::NpmlockPresentation,
    &plugin_roff::RoffPresentation,
    &plugin_ignorefile::IgnorefilePresentation,
    &plugin_webmanifest::WebmanifestPresentation,
    &plugin_sourcemap::SourcemapPresentation,
    &plugin_gitconfig::GitconfigPresentation,
    &plugin_gitattributes::GitattributesPresentation,
    &plugin_helmchart::HelmchartPresentation,
    &plugin_gitlabci::GitlabciPresentation,
    &plugin_jsonschema::JsonschemaPresentation,
    &plugin_githubactions::GithubactionsPresentation,
    &plugin_kubernetes::KubernetesPresentation,
    &plugin_maven::MavenPresentation,
    &plugin_msbuild::MsbuildPresentation,
    &plugin_protobuf::ProtobufPresentation,
    &plugin_thrift::ThriftPresentation,
    &plugin_flatbuffers::FlatbuffersPresentation,
    &plugin_antlr::AntlrPresentation,
    &plugin_yacc::YaccPresentation,
    &plugin_lex::LexPresentation,
    &plugin_openapi::OpenapiPresentation,
    &plugin_requirements::RequirementsPresentation,
    &plugin_yarnlock::YarnlockPresentation,
    &plugin_pnpmlock::PnpmlockPresentation,
    &plugin_solution::SolutionPresentation,
    &plugin_jenkinsfile::JenkinsfilePresentation,
    &plugin_ansible::AnsiblePresentation,
    &plugin_cloudformation::CloudformationPresentation,
    &plugin_systemdunit::SystemdunitPresentation,
    &plugin_nginxconf::NginxconfPresentation,
    &plugin_apacheconf::ApacheconfPresentation,
    &plugin_caddyfile::CaddyfilePresentation,
    &plugin_sshconfig::SshconfigPresentation,
    &plugin_lua::LuaPresentation,
    &plugin_zig::ZigPresentation,
    &plugin_dlang::DlangPresentation,
    &plugin_pascal::PascalPresentation,
    &plugin_cobol::CobolPresentation,
    &plugin_verilog::VerilogPresentation,
    &plugin_vhdl::VhdlPresentation,
    &plugin_matlab::MatlabPresentation,
    &plugin_elisp::ElispPresentation,
    &plugin_awk::AwkPresentation,
    &plugin_batchfile::BatchfilePresentation,
    &plugin_jsonlines::JsonlinesPresentation,
    &plugin_json5::Json5Presentation,
    &plugin_purescript::PurescriptPresentation,
    &plugin_gleam::GleamPresentation,
    &plugin_cbor::CborPresentation,
    &plugin_bson::BsonPresentation,
    &plugin_arrow::ArrowPresentation,
    &plugin_orc::OrcPresentation,
    &plugin_numpy::NumpyPresentation,
    &plugin_gzip::GzipPresentation,
    &plugin_tar::TarPresentation,
    &plugin_pickle::PicklePresentation,
    &plugin_bzip2::Bzip2Presentation,
    &plugin_zstd::ZstdPresentation,
    &plugin_xz::XzPresentation,
    &plugin_sevenzip::SevenzipPresentation,
    &plugin_jar::JarPresentation,
    &plugin_apk::ApkPresentation,
    &plugin_wheel::WheelPresentation,
    &plugin_rubygem::RubygemPresentation,
    &plugin_nuget::NugetPresentation,
    &plugin_apkpkg::ApkpkgPresentation,
    &plugin_javaclass::JavaclassPresentation,
    &plugin_pyc::PycPresentation,
    &plugin_minidump::MinidumpPresentation,
    &plugin_pcap::PcapPresentation,
    &plugin_css::CssPresentation,
    &plugin_dotnetassembly::DotnetassemblyPresentation,
    &plugin_sass::SassPresentation,
    &plugin_bicep::BicepPresentation,
    &plugin_nix::NixPresentation,
    &plugin_starlark::StarlarkPresentation,
    &plugin_cmake::CmakePresentation,
    &plugin_meson::MesonPresentation,
    &plugin_ninja::NinjaPresentation,
    &plugin_cue::CuePresentation,
    &plugin_rego::RegoPresentation,
    &plugin_gemfilelock::GemfilelockPresentation,
    &plugin_composerlock::ComposerlockPresentation,
    &plugin_pythonlock::PythonlockPresentation,
    &plugin_gosum::GosumPresentation,
];

/// Turns a plugin's view data into displayable lines, via whichever
/// registered presentation plugin matches `plugin`.
/// Every folder presentation plugin linked into this front end.
///
/// Separate from [`PRESENTATION_PLUGINS`] because a folder can be several
/// things at once - a working copy that is also a Cargo workspace - and
/// each plugin that recognises it contributes its own lines.
const FOLDER_PRESENTATION_PLUGINS: &[&dyn FolderPresentation] =
    &[&plugin_project_cargo::CargoProjectPresentation];

/// Turns a folder plugin's view data into displayable lines.
fn present_folder(plugin: &str, data: &serde_json::Value) -> Vec<String> {
    match FOLDER_PRESENTATION_PLUGINS
        .iter()
        .find(|candidate| candidate.name() == plugin)
    {
        Some(candidate) => candidate.present(data),
        None => vec![format!("no presentation for folder plugin `{plugin}`")],
    }
}

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
        Response::FileView { plugin, data, also } => {
            let mut lines = present(plugin, data);
            // A folder is several things at once, and each folder plugin
            // that recognises it adds its lines below the folder's own.
            for extra in also {
                lines.push(String::new());
                lines.extend(present_folder(&extra.plugin, &extra.data));
            }
            frame.render_widget(Paragraph::new(lines.join("\n")).block(block), area);
        }
        Response::Error { message } => {
            frame.render_widget(Paragraph::new(message.as_str()).block(block), area);
        }
        Response::Done => {
            frame.render_widget(Paragraph::new("done").block(block), area);
        }
        // The terminal front end keeps its own path argument until its own
        // realignment work order; it has no reason to ask for the roots, and
        // renders the reply as what it is if it somehow receives one.
        Response::ReposRoots { roots, default } => {
            let text = roots.iter().map(|root| root.path.as_str()).fold(
                format!("Repos Directory (default {default}):"),
                |text, path| {
                    format!(
                        "{text}
{path}"
                    )
                },
            );
            frame.render_widget(Paragraph::new(text).block(block), area);
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
                    size: 0,
                    modified: None,
                    repository: None,
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
            also: Vec::new(),
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
                size: 0,
                modified: None,
                repository: None,
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
    #[test]
    fn renders_a_folder_and_the_project_it_holds() {
        let backend = TestBackend::new(40, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let response = Response::FileView {
            plugin: "text".to_owned(),
            data: serde_json::json!({ "content": "folder", "truncated": false }),
            also: vec![protocol::PluginView {
                plugin: "project-cargo".to_owned(),
                data: serde_json::json!({
                    "kind": "package",
                    "package": {
                        "name": "instrument-log",
                        "version": "2.3.0",
                        "edition": "2024",
                        "rust_version": null,
                        "description": null
                    },
                    "members": [],
                    "dependencies": 3,
                    "dev_dependencies": 0,
                    "build_dependencies": 0
                }),
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
        assert!(
            contents.contains("folder"),
            "the folder keeps its own lines"
        );
        assert!(
            contents.contains("instrument-log"),
            "and the project lines are added: {contents}"
        );
    }

    /// Everything [`render`] puts on a `width` x `height` terminal.
    fn drawn(width: u16, height: u16, response: &Response) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        terminal
            .draw(|frame| render(frame, frame.area(), response))
            .expect("a draw into the test backend");
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[test]
    fn a_view_from_a_plugin_this_front_end_does_not_carry_is_named_not_left_blank() {
        let response = Response::FileView {
            plugin: "no-such-plugin".to_owned(),
            data: serde_json::json!({ "content": "unreachable" }),
            also: Vec::new(),
        };

        let contents = drawn(44, 6, &response);

        assert!(
            contents.contains("no presentation for plugin"),
            "a missing presentation half should say which one: {contents}"
        );
        assert!(
            contents.contains("no-such-plugin"),
            "and name it: {contents}"
        );
    }

    #[test]
    fn a_folder_plugin_this_front_end_does_not_carry_is_named_too() {
        let response = Response::FileView {
            plugin: "text".to_owned(),
            data: serde_json::json!({ "content": "folder", "truncated": false }),
            also: vec![protocol::PluginView {
                plugin: "no-such-folder-plugin".to_owned(),
                data: serde_json::json!({}),
            }],
        };

        let contents = drawn(50, 8, &response);

        assert!(
            contents.contains("folder"),
            "the folder still says what it is: {contents}"
        );
        assert!(
            contents.contains("no presentation for folder plugin"),
            "and the gap is named rather than silently dropped: {contents}"
        );
    }

    #[test]
    fn every_folder_view_is_added_below_the_ones_before_it() {
        // A folder is several things at once, and each plugin that
        // recognises it adds its own lines - so two of them must both be
        // there, not one in place of the other.
        let cargo = serde_json::json!({
            "kind": "package",
            "package": {
                "name": "instrument-log",
                "version": "2.3.0",
                "edition": "2024",
                "rust_version": null,
                "description": null
            },
            "members": [],
            "dependencies": 3,
            "dev_dependencies": 0,
            "build_dependencies": 0
        });
        let response = Response::FileView {
            plugin: "text".to_owned(),
            data: serde_json::json!({ "content": "the folder itself", "truncated": false }),
            also: vec![
                protocol::PluginView {
                    plugin: "project-cargo".to_owned(),
                    data: cargo,
                },
                protocol::PluginView {
                    plugin: "second-folder-plugin".to_owned(),
                    data: serde_json::json!({}),
                },
            ],
        };

        let contents = drawn(60, 16, &response);

        assert!(contents.contains("the folder itself"), "{contents}");
        assert!(contents.contains("instrument-log"), "{contents}");
        assert!(contents.contains("second-folder-plugin"), "{contents}");
    }

    #[test]
    fn a_repos_roots_reply_is_drawn_as_the_list_it_is() {
        let response = Response::ReposRoots {
            roots: vec![
                protocol::ReposRoot {
                    path: "/home/ada/repos".to_owned(),
                    active: true,
                },
                protocol::ReposRoot {
                    path: "/mnt/work".to_owned(),
                    active: false,
                },
            ],
            default: "/home/ada/repos".to_owned(),
        };

        let contents = drawn(44, 8, &response);

        assert!(contents.contains("Repos Directory"), "{contents}");
        assert!(contents.contains("/home/ada/repos"), "{contents}");
        assert!(
            contents.contains("/mnt/work"),
            "every root is listed, not only the active one: {contents}"
        );
    }

    #[test]
    fn a_finished_operation_says_so_rather_than_leaving_the_pane_empty() {
        let contents = drawn(20, 4, &Response::Done);

        assert!(contents.contains("done"), "{contents}");
    }

    #[test]
    fn an_error_is_drawn_as_the_message_the_service_sent() {
        let response = Response::Error {
            message: "permission denied".to_owned(),
        };

        let contents = drawn(30, 4, &response);

        assert!(contents.contains("permission denied"), "{contents}");
    }

    #[test]
    fn every_kind_of_reply_draws_into_an_area_with_no_room_for_its_border() {
        let replies = [
            Response::Done,
            Response::Error {
                message: "boom".to_owned(),
            },
            Response::Directory {
                entries: vec![DirectoryEntry {
                    name: "src".to_owned(),
                    is_dir: true,
                    size: 0,
                    modified: None,
                    repository: None,
                }],
            },
            Response::FileView {
                plugin: "text".to_owned(),
                data: serde_json::json!({ "content": "hi", "truncated": false }),
                also: Vec::new(),
            },
            Response::ReposRoots {
                roots: Vec::new(),
                default: "/home/ada/repos".to_owned(),
            },
        ];

        for response in &replies {
            for (width, height) in [(0, 0), (1, 1), (2, 1), (1, 2), (3, 3)] {
                let contents = drawn(width, height, response);
                assert_eq!(
                    contents.chars().count(),
                    usize::from(width) * usize::from(height),
                    "{response:?} at {width}x{height} did not fill the area it was given"
                );
            }
        }
    }

    #[test]
    fn a_listing_of_multibyte_names_draws_into_a_pane_too_narrow_for_them() {
        let response = Response::Directory {
            entries: vec![
                DirectoryEntry {
                    name: "日本語のフォルダ".to_owned(),
                    is_dir: true,
                    size: 0,
                    modified: None,
                    repository: None,
                },
                DirectoryEntry {
                    name: "café-notes.txt".to_owned(),
                    is_dir: false,
                    size: 0,
                    modified: None,
                    repository: None,
                },
            ],
        };

        for width in 1..20_u16 {
            let contents = drawn(width, 5, &response);
            assert!(
                !contents.contains('\u{fffd}'),
                "a pane {width} cells wide cut a character in half"
            );
        }
    }

    #[test]
    fn a_request_to_a_socket_nobody_is_listening_on_fails_rather_than_hanging() {
        // This is the failure `app::opening` leans on: with no service to
        // answer, it must come back as an error so the front end can fall
        // back and say why, rather than block a launch.
        let name = unique_socket_name();

        let result = send_request(
            name.as_str()
                .to_ns_name::<GenericNamespaced>()
                .expect("a valid namespaced socket name"),
            &Request::ReposRoots,
        );

        assert!(
            result.is_err(),
            "connecting to a socket that was never created should fail"
        );
    }
}
