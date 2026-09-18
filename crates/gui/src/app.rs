//! Application state for the three-pane explorer, independent of Slint so
//! it is unit-testable without a display. See `tui::app` for the sibling
//! Ratatui implementation: per §3.1 each front end owns its own
//! presentation half, so the two are separate, not shared, despite the
//! similar shape.

use crate::document::Document;
use crate::editor;
use crate::launch::{self, Launch, Platform};
use crate::switcher;
use plugin_api::{
    Class, Fact, FolderPresentation, Graphic, Icon, PREVIEW_VIEW, PluginPresentation, Span,
    TEXT_VIEW, UNKNOWN_ICON,
};
use protocol::{DirectoryEntry, ReposRoot, RepositoryInfo, RepositoryKind, Request, Response};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

/// Every presentation plugin linked into this front end.
///
/// Hand-registered: a registration macro would be structure with no second
/// caller to justify it while seven entries can still be read at a glance
/// (see `plugin-api`'s crate docs).
/// Every presentation half, in registration order.
///
/// Public so a test can walk the whole catalogue and hold each plugin to
/// the contract on [`plugin_api::Span`]: a classifier whose spans do not
/// cover their text would slice a string at a byte that is not a
/// character boundary the first time somebody opened that format.
pub const PRESENTATION_PLUGINS: &[&dyn PluginPresentation] = &[
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

/// The icon for an entry, from whichever presentation plugin claims its
/// extension. GUIDANCE.md §3 makes the icon a plugin's own property, so the
/// front end looks one up instead of keeping a table of its own: a name no
/// plugin claims gets `UNKNOWN_ICON`, and a directory gets the directory
/// plugin's icon.
///
/// Matching is by name, never by content. A listing marks hundreds of rows
/// at once and Explorer picks its icons the same way; content sniffing stays
/// in the service, choosing which viewer opens a file once one is picked.
#[must_use]
pub fn icon_for(name: &str, is_dir: bool) -> Icon {
    if is_dir {
        return plugin_directory::DirectoryPresentation.icon();
    }
    let extension = std::path::Path::new(name)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_lowercase);
    // A format named rather than suffixed - `Dockerfile`, `Makefile` - is
    // matched on the whole name instead.
    let key = extension.unwrap_or_else(|| name.to_lowercase());
    PRESENTATION_PLUGINS
        .iter()
        .find(|plugin| plugin.extensions().contains(&key.as_str()))
        .map_or(UNKNOWN_ICON, |plugin| plugin.icon())
}

/// The small branch mark a working copy's folder icon carries (#579),
/// naming the provider where one is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RepositoryMark {
    /// `github.com`.
    GitHub,
    /// `gitlab.com`.
    GitLab,
    /// `bitbucket.org`.
    Bitbucket,
    /// `dev.azure.com`.
    AzureDevOps,
    /// A checkout with no remote, or a host this project does not name -
    /// a self-hosted GitLab or Bitbucket instance included. Still a
    /// working copy, just not one of the four named providers.
    Generic,
}

/// The mark drawn on a folder's icon, from what the listing already read
/// (GUIDANCE.md §2.5): `None` for a plain folder, so a reader who cannot
/// tell the Contents pane's accent colour from its foreground text still
/// sees a working copy drawn differently (GUIDANCE.md §2.4).
#[must_use]
pub fn repository_mark(is_repository: bool, provider: Option<&str>) -> Option<RepositoryMark> {
    if !is_repository {
        return None;
    }
    Some(match provider {
        Some("github.com") => RepositoryMark::GitHub,
        Some("gitlab.com") => RepositoryMark::GitLab,
        Some("bitbucket.org") => RepositoryMark::Bitbucket,
        Some("dev.azure.com") => RepositoryMark::AzureDevOps,
        _ => RepositoryMark::Generic,
    })
}

/// The picture a plugin offers for `data`, if its type is one.
#[must_use]
pub fn present_graphic(plugin: &str, data: &serde_json::Value) -> Option<Graphic> {
    PRESENTATION_PLUGINS
        .iter()
        .find(|candidate| candidate.name() == plugin)
        .and_then(|candidate| candidate.graphic(data))
}

/// Every folder presentation plugin linked into this front end.
///
/// Separate from [`PRESENTATION_PLUGINS`] because folder plugins answer a
/// different question. A file has one type. A folder can be a source
/// control working copy and a Cargo workspace at once, and each plugin
/// that recognises it contributes its own lines.
const FOLDER_PRESENTATION_PLUGINS: &[&dyn FolderPresentation] =
    &[&plugin_project_cargo::CargoProjectPresentation];

/// Turns a folder plugin's view data into displayable lines.
#[must_use]
pub fn present_folder(plugin: &str, data: &serde_json::Value) -> Vec<String> {
    match FOLDER_PRESENTATION_PLUGINS
        .iter()
        .find(|candidate| candidate.name() == plugin)
    {
        Some(candidate) => candidate.present(data),
        None => vec![format!("no presentation for folder plugin `{plugin}`")],
    }
}

/// Turns a plugin's view data into displayable lines, via whichever
/// registered presentation plugin matches `plugin`.
#[must_use]
pub fn present(plugin: &str, data: &serde_json::Value) -> Vec<String> {
    match PRESENTATION_PLUGINS
        .iter()
        .find(|candidate| candidate.name() == plugin)
    {
        Some(candidate) => candidate.present(data),
        None => vec![format!("no presentation for plugin `{plugin}`")],
    }
}

/// The File pane's fact table for `data`, via whichever registered
/// presentation plugin matches `plugin`. Empty for an unrecognised plugin,
/// the same as a plugin that has none of its own.
#[must_use]
pub fn facts(plugin: &str, data: &serde_json::Value) -> Vec<Fact> {
    PRESENTATION_PLUGINS
        .iter()
        .find(|candidate| candidate.name() == plugin)
        .map_or_else(Vec::new, |candidate| candidate.facts(data))
}

/// Renders the view named `view` of `data`, via whichever registered
/// presentation plugin matches `plugin`.
#[must_use]
pub fn present_view(plugin: &str, view: &str, data: &serde_json::Value) -> Vec<String> {
    match PRESENTATION_PLUGINS
        .iter()
        .find(|candidate| candidate.name() == plugin)
    {
        Some(candidate) => candidate.present_view(view, data),
        None => vec![format!("no presentation for plugin `{plugin}`")],
    }
}

/// A directory node in the folders pane's tree. Only directories appear
/// here; files live in the contents pane.
#[derive(Debug)]
pub struct FolderNode {
    /// Full path this node represents.
    pub path: PathBuf,
    /// Display name (the path's final component).
    pub name: String,
    /// Whether this node's children are shown.
    pub expanded: bool,
    /// Subdirectories, once fetched.
    pub children: Option<Vec<FolderNode>>,
    /// Whether this folder is a source control working copy (#579), from
    /// the same listing that populated `children`.
    pub is_repository: bool,
    /// The working copy's provider, where the listing found one.
    pub provider: Option<String>,
    /// The working copy's remote, as written in its own configuration
    /// (#581) - what "Copy remote address" copies for a Folders pane row.
    pub remote: Option<String>,
    /// The working copy's branch, `None` for a detached head - what the
    /// Go to Repository switcher (#590) shows beside a result, since a
    /// repository the switcher lists may not be the folder on screen.
    pub branch: Option<String>,
}

impl FolderNode {
    /// Creates the tree's root node, expanded by default.
    #[must_use]
    pub fn root(path: PathBuf) -> Self {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        Self {
            path,
            name,
            expanded: true,
            children: None,
            is_repository: false,
            provider: None,
            remote: None,
            branch: None,
        }
    }

    fn node_at(&self, indices: &[usize]) -> Option<&FolderNode> {
        match indices.split_first() {
            None => Some(self),
            Some((&first, rest)) => self.children.as_ref()?.get(first)?.node_at(rest),
        }
    }

    fn node_at_mut(&mut self, indices: &[usize]) -> Option<&mut FolderNode> {
        match indices.split_first() {
            None => Some(self),
            Some((&first, rest)) => self.children.as_mut()?.get_mut(first)?.node_at_mut(rest),
        }
    }

    fn set_children_from(&mut self, entries: &[DirectoryEntry]) {
        let mut previous: HashMap<String, FolderNode> = self
            .children
            .take()
            .unwrap_or_default()
            .into_iter()
            .map(|node| (node.name.clone(), node))
            .collect();

        let path = &self.path;
        self.children = Some(
            entries
                .iter()
                .filter(|entry| entry.is_dir)
                .map(|entry| {
                    let mut node = previous.remove(&entry.name).unwrap_or_else(|| FolderNode {
                        path: path.join(&entry.name),
                        name: entry.name.clone(),
                        expanded: false,
                        children: None,
                        is_repository: false,
                        provider: None,
                        remote: None,
                        branch: None,
                    });
                    // Read fresh each time rather than only on first sight:
                    // a folder can turn into a working copy (or stop being
                    // one) between listings, same as its children can.
                    node.is_repository = entry.repository.is_some();
                    node.provider = entry
                        .repository
                        .as_ref()
                        .and_then(|repository| repository.provider.clone());
                    node.remote = entry
                        .repository
                        .as_ref()
                        .and_then(|repository| repository.remote.clone());
                    node.branch = entry
                        .repository
                        .as_ref()
                        .and_then(|repository| repository.branch.clone());
                    node
                })
                .collect(),
        );
    }

    /// Flattens the visible (expanded) tree into `(depth, index_path)` pairs
    /// in display order, root first.
    fn flatten(&self) -> Vec<(usize, Vec<usize>)> {
        let mut rows = Vec::new();
        self.flatten_into(0, &mut Vec::new(), &mut rows);
        rows
    }

    fn flatten_into(
        &self,
        depth: usize,
        path: &mut Vec<usize>,
        rows: &mut Vec<(usize, Vec<usize>)>,
    ) {
        rows.push((depth, path.clone()));
        if self.expanded
            && let Some(children) = &self.children
        {
            for (index, child) in children.iter().enumerate() {
                path.push(index);
                child.flatten_into(depth + 1, path, rows);
                path.pop();
            }
        }
    }
}

/// Which pane last received user interaction, for the "focused" highlight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    /// The folders tree pane.
    Folders,
    /// The current folder's contents pane.
    Contents,
    /// The selected file's preview pane.
    File,
}

/// Everything that says what the reader is looking at, gathered into one
/// read-only value. [`App::selection`] is the only way to get one; no pane
/// may write any of these fields directly, only ask for a change through an
/// intent method such as [`App::select_folder`] or
/// [`App::extend_selection_to`], which updates `App`'s own state and is
/// then reflected here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// The selected row in the Folders tree.
    pub folder: usize,
    /// The lead Contents row - the one the preview and the rename/copy
    /// prompts act on.
    pub content: usize,
    /// Every selected Contents row, `content` among them whenever this is
    /// non-empty.
    pub contents: std::collections::BTreeSet<usize>,
    /// The row a Shift range extends from.
    pub anchor: usize,
    /// Which of the previewed type's views the File pane is showing.
    pub file_view_index: usize,
    /// Whether the File pane is editing rather than previewing.
    pub editing: bool,
    /// Which pane last received user interaction.
    pub focus: Pane,
}

/// Whether the app is idling, waiting on a delete confirmation, or editing
/// a name for a rename/copy/extract operation.
#[derive(Debug)]
enum Mode {
    Normal,
    ConfirmDelete {
        paths: Vec<PathBuf>,
        name: String,
    },
    RenameInput {
        path: PathBuf,
        input: String,
    },
    CopyInput {
        path: PathBuf,
        input: String,
    },
    ExtractInput {
        path: PathBuf,
        input: String,
    },
    /// The address bar turned into a text field, the way Explorer's does on
    /// Ctrl+L or a click past the last segment.
    PathInput {
        input: String,
    },
    /// The prompt for the Repos Directory: on a first run, and whenever the
    /// user asks to change it. Shaped like the path prompt because it is the
    /// same act - typing where to look - but it settles where every future
    /// launch begins, so it is its own mode rather than a flag on that one.
    ReposRootInput {
        input: String,
    },
    /// Typing a name to find across every repository (#536), in the
    /// address bar the way a path is typed.
    FindInput {
        input: String,
    },
    /// Ctrl+P / Cmd+P's "Go to Repository" switcher (#590): a query fuzzy-
    /// matched against the Repos Directory's repositories in an overlay of
    /// its own. Unlike the Contents pane's filter (#582), which narrows
    /// whatever folder is already open, Enter here jumps to a repository
    /// outright - so it takes over the keyboard as its own mode rather
    /// than a flag on `Mode::Normal` the way that filter is.
    Switcher {
        query: String,
    },
}

/// A search's answer, shown in the Contents pane in place of the listing.
struct Found {
    /// What was searched for, for the status bar.
    query: String,
    /// The Repos Directory that was searched; every match is relative to it.
    root: PathBuf,
    matches: Vec<protocol::NameMatch>,
    /// The listing's selection before the results replaced it, so Escape
    /// puts the reader back where they were.
    previous_selected: usize,
}

/// The All Repositories view (#591): every working copy the service has
/// found so far up to three folder levels below the Repos Directory, shown
/// in the Contents pane in place of the listing, the way [`Found`]'s
/// results are.
struct AllRepositoriesView {
    /// The Repos Directory the scan is below; every entry's location is
    /// relative to it.
    root: PathBuf,
    /// What the background scan has found so far, in the order it met them.
    entries: Vec<protocol::AllRepositoryEntry>,
    /// Whether the scan has finished, so the status bar can stop counting.
    done: bool,
    /// The listing's selection before this view replaced it, so Escape puts
    /// the reader back where they were.
    previous_selected: usize,
}

/// What to do once a pending operation completes successfully, beyond the
/// reload every operation already triggers.
#[derive(Debug)]
enum AfterOperation {
    /// Drop straight into inline rename for a just-created entry, matching
    /// Explorer's own "create, then retype the name" flow.
    EnterRename { path: PathBuf, input: String },
}

/// Picks a default name for a new entry, de-duplicated against `existing`
/// (e.g. `"New folder"`, then `"New folder (2)"`, `"New folder (3)"`, ...)
/// so create never sends a request that is doomed to collide.
fn dedup_name(taken: &[String], base: &str) -> String {
    if !taken.iter().any(|name| name == base) {
        return base.to_owned();
    }
    let (stem, extension) = split_extension(base);
    let mut n = 2;
    loop {
        let candidate = format!("{stem} ({n}){extension}");
        if !taken.iter().any(|name| name == &candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Whether `c` belongs in a typed prompt.
///
/// Slint reports a held key - Shift, Control, Alt - as a key press whose
/// text is a control character, and its named keys as code points from
/// U+F700, and any the window's key handler does not recognise reaches the
/// prompt. Taking them typed an invisible `\u{10}` in front of every
/// capital, since Shift is held first, and a `\u{11}` in front of every
/// Ctrl+V - so a rename to `Notes.txt` asked the filesystem for a name it
/// refuses. Pasted text is held to the same rule, so a tab on the clipboard
/// does not get in by the other door.
fn typeable(c: char) -> bool {
    !c.is_control() && !('\u{f700}'..='\u{f8ff}').contains(&c)
}

/// A name split into what to number and what to keep on the end.
///
/// The count used to go after the whole filename, so a copy of `notes.txt`
/// was `notes.txt (2)` - a name with no extension at all. To this
/// application that is not a text file: no plugin recognises it, the File
/// pane cannot preview it, and the listing gives it the generic icon. In a
/// Repos Explorer, where the plugin registry is how a file becomes
/// readable, an operation that strips a file's type is more than cosmetic.
///
/// A leading dot is part of the name, not a separator, so `.gitignore`
/// numbers as `.gitignore (2)` rather than growing a stray dot. A name with
/// no dot at all, and a folder, keep today's behaviour because there is no
/// extension to sit in front of.
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(at) if at > 0 => name.split_at(at),
        _ => (name, ""),
    }
}

fn send_request(request: &Request) -> io::Result<Response> {
    use interprocess::local_socket::traits::Stream as _;
    let mut conn = interprocess::local_socket::Stream::connect(protocol::socket_name()?)?;
    protocol::write_message(&mut conn, request)?;
    protocol::read_message(&mut conn)
}

/// Where the application opens, and whether it has to ask first.
///
/// Per decision D7 there is no last-location restore: the answer is the
/// configured Repos Directory, and nothing else. When nothing is configured
/// (a first run) this reports the platform's default and says it should be
/// offered rather than assumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opening {
    /// The directory to open at.
    pub root: PathBuf,
    /// Whether the user should be asked to confirm or change it, because
    /// nothing is configured yet.
    pub ask: bool,
}

/// Asks the service where this machine's Repos Directory is.
///
/// The service owns the configuration, as it owns everything else that
/// outlives a window. A front end that cannot reach it falls back to the
/// platform default and asks, which is the same thing a first run does.
#[must_use]
pub fn opening() -> Opening {
    match send_request(&Request::ReposRoots) {
        Ok(Response::ReposRoots { roots, default }) => {
            let active = roots
                .into_iter()
                .find(|root: &ReposRoot| root.active)
                .map(|root| PathBuf::from(root.path));
            match active {
                Some(root) => Opening { root, ask: false },
                None => Opening {
                    root: PathBuf::from(default),
                    ask: true,
                },
            }
        }
        _ => Opening {
            root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            ask: true,
        },
    }
}

fn spawn_request(request: Request) -> Receiver<io::Result<Response>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(send_request(&request));
    });
    rx
}

/// Resolves `name` against `path`'s parent directory, as a string suitable
/// for a request's `to`/`destination` field.
fn sibling_path(path: &std::path::Path, name: &str) -> String {
    path.parent()
        .map_or_else(|| PathBuf::from(name), |parent| parent.join(name))
        .to_string_lossy()
        .into_owned()
}

/// Formats a byte count for the status bar, e.g. `1.2 MB`.
///
/// `bytes` comes from summing a folder's own entry count (at most a few
/// million even for an enormous directory), so the `f64` round trip below
/// never loses meaningful precision for display purposes.
#[allow(clippy::cast_precision_loss)]
fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = "B";
    for candidate in UNITS {
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
        unit = candidate;
    }
    if unit == "B" {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {unit}")
    }
}

/// How many characters a fact table's value column shows before eliding,
/// at the File pane's default width. Not derived from a measured pixel
/// width - Slint's own `overflow: elide` is that, cutting from the end,
/// and stays as the safety net for a narrower pane - but from the same
/// budget a branch or remote address needs to show both where it starts
/// and where it ends (#576).
const FACT_VALUE_BUDGET: usize = 40;

/// `value`, unchanged if it already fits in `budget` characters, or its
/// first and last few characters joined by an ellipsis so both ends still
/// show - unlike end-eliding, which would show only the start.
fn middle_elide(value: &str, budget: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= budget {
        return value.to_owned();
    }
    // A third of the budget for the tail keeps most of it for the start,
    // which is where a branch or provider name's own words usually sit.
    let tail = budget / 3;
    let head = budget - tail - 1;
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}…{end}")
}

/// Explorer's "Type" column: `"File folder"` for a directory, otherwise the
/// uppercased extension as `"RS file"`, or plain `"File"` when there is none.
fn format_kind(name: &str, is_dir: bool) -> String {
    format_kind_of(name, is_dir, None)
}

/// How many characters fit in the Type column's default width. Not derived
/// from a measured pixel width, for the same reason [`FACT_VALUE_BUDGET`]
/// is not: the column's own `overflow: elide` (`kind-column` in app.slint)
/// stays the safety net for a narrower pane, rather than the first line of
/// defence (#578).
const KIND_COLUMN_BUDGET: usize = 22;

/// As [`format_kind`], but for an entry the service has told us something
/// about as a source control working copy.
///
/// A repository says so, and names the provider it came from, because that
/// is the fact somebody opening their workspace is looking for. An ordinary
/// folder is still an ordinary folder - visible, and plainly different
/// (GUIDANCE.md 2.5).
fn format_kind_of(name: &str, is_dir: bool, repository: Option<&RepositoryInfo>) -> String {
    if let Some(repository) = repository {
        // A worktree or submodule says so instead of the generic "Git
        // repository" - the distinction from an ordinary clone is the
        // fact somebody scanning the column is looking for (#587).
        return match &repository.kind {
            RepositoryKind::Worktree { .. } => {
                with_provider("Worktree", repository.provider.as_deref())
            }
            RepositoryKind::Submodule { .. } => {
                with_provider("Submodule", repository.provider.as_deref())
            }
            RepositoryKind::Clone => match &repository.provider {
                // "Git repository · github.com" names both halves of the
                // fact, but a long provider address can still overrun the
                // column that "Repository (github.com)" already overran
                // (#578) - so it gives way to the shorter
                // "Repository · github.com", which keeps the provider and
                // drops only the word that was already said by the row's
                // own accent colour and bold name.
                Some(provider) => {
                    let named = format!("Git repository · {provider}");
                    if named.chars().count() <= KIND_COLUMN_BUDGET {
                        named
                    } else {
                        format!("Repository · {provider}")
                    }
                }
                // A checkout with no remote has no provider to name, and
                // says what it is instead.
                None => "Git repository".to_owned(),
            },
        };
    }
    if is_dir {
        return "File folder".to_owned();
    }
    std::path::Path::new(name)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .filter(|extension| !extension.is_empty())
        .map_or_else(
            || "File".to_owned(),
            |extension| format!("{} file", extension.to_uppercase()),
        )
}

/// `noun`, with the provider appended after a middle dot when there is
/// one: `"Worktree · github.com"`, or plain `"Worktree"` for a checkout
/// with no remote configured (#587).
fn with_provider(noun: &str, provider: Option<&str>) -> String {
    match provider {
        Some(provider) => format!("{noun} · {provider}"),
        None => noun.to_owned(),
    }
}

/// A modified time as `YYYY-MM-DD HH:MM`, from seconds since the Unix epoch.
/// Rendered in UTC: the service reports the timestamp in epoch seconds and
/// this front end has no timezone database to convert it with, so a label
/// that is unambiguous beats one that is quietly wrong by an offset.
fn format_timestamp(seconds: Option<u64>) -> String {
    let Some(seconds) = seconds else {
        return String::new();
    };
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let time_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let (hour, minute) = (time_of_day / 3_600, (time_of_day % 3_600) / 60);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// The time an entry's Modified column sorts and displays by: a
/// repository's last activity when it has one, and the folder's or file's
/// own modification time otherwise (#588).
fn effective_modified(entry: &DirectoryEntry) -> Option<u64> {
    entry
        .repository
        .as_ref()
        .and_then(|repository| repository.last_activity)
        .or(entry.modified)
}

/// Converts days since 1970-01-01 into a civil `(year, month, day)`, by
/// Howard Hinnant's `civil_from_days`. Avoids taking on a date library for
/// one column.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = u32::try_from(day_of_year - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Whether a clipboard entry was copied or cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipboardMode {
    /// Ctrl+C: paste leaves the original in place.
    Copy,
    /// Ctrl+X: paste moves it.
    Cut,
}

/// Which column the contents pane is sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    /// Entry name, case-insensitively.
    Name,
    /// Size in bytes.
    Size,
    /// The "Type" column's text.
    Kind,
    /// Last modified time.
    Modified,
}

impl SortKey {
    /// The column index the UI uses for this key.
    const fn index(self) -> i32 {
        match self {
            Self::Name => 0,
            Self::Size => 1,
            Self::Kind => 2,
            Self::Modified => 3,
        }
    }

    /// The key a column index names, if any.
    const fn from_index(index: i32) -> Option<Self> {
        match index {
            0 => Some(Self::Name),
            1 => Some(Self::Size),
            2 => Some(Self::Kind),
            3 => Some(Self::Modified),
            _ => None,
        }
    }
}

/// One folders-pane row, as the navigation tree renders it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderRow {
    /// The folder icon, the same one the listing beside it draws.
    pub icon: Icon,
    /// The folder's own name, with no path and no trailing separator.
    pub name: String,
    /// How deep in the tree the row sits. The pane indents by this.
    pub depth: usize,
    /// Whether the row has a chevron in front of it.
    pub expandable: bool,
    /// Whether that chevron points down.
    pub expanded: bool,
    /// The branch mark this row's icon carries, or `None` for a plain
    /// folder (#579).
    pub mark: Option<RepositoryMark>,
}

/// How far one level of the tree indents, in pixels.
pub const FOLDER_INDENT: f32 = 16.0;

/// The width of the chevron column in front of a folder's icon.
pub const FOLDER_CHEVRON: f32 = 16.0;

/// The padding inside a pane's left edge, before the first row content.
pub const FOLDER_PADDING: f32 = 4.0;

/// Whether `x` pixels in from a folders-pane row's left edge falls on the
/// chevron of a row at `depth`, drawn at `zoom` (#586) - `app.slint` scales
/// the padding, indent and chevron column by the same factor, so the hit
/// test has to move with them or a click would land beside the chevron it
/// looks like it landed on.
///
/// In Rust rather than in `app.slint` for the reason given on
/// [`crate::scroll_offset_for`]: a rule written in that file cannot be
/// exercised without an event loop, and every layout rule this project has
/// got wrong was one that lived there.
#[must_use]
pub fn chevron_hit(x: f32, depth: usize, zoom: f32) -> bool {
    // `depth` is a tree level, and a tree deep enough to overflow this has
    // long since run out of pane to indent into.
    let Ok(level) = u16::try_from(depth) else {
        return false;
    };
    let start = zoom.mul_add(FOLDER_PADDING, f32::from(level) * FOLDER_INDENT * zoom);
    x >= start && x < start + FOLDER_CHEVRON * zoom
}

/// A file open in the editor.
struct Edit {
    /// Where a save goes.
    path: PathBuf,
    /// The file as it was opened, so the pane can say whether there is
    /// anything to save without the reader having to try it.
    original: String,
    /// The text and the caret.
    document: Document,
    /// Whether the hand-written surface is drawing it, or Slint's plain
    /// box. See [`editor::code_editor_suits`]: a file the surface cannot
    /// place a caret in, or cannot receive typing for, opens in the
    /// plain one instead, and the status bar says why.
    coloured: bool,
    /// The presentation half that opened this file, so the colouring
    /// follows the document rather than the preview.
    ///
    /// Taken once, when the editor opens. Reading it from `file_view`
    /// instead meant clicking another file in the Contents pane
    /// re-tokenised the open document as that file's type: open a `.rs`
    /// file, click a `.json` one, and the Rust somebody was typing lost
    /// its Rust colouring while its text and caret sat untouched.
    plugin: Option<&'static str>,
}

/// How many names one search asks for. The service caps it too.
const FIND_LIMIT: usize = 500;

/// A search result as a Contents row: its path within its repository in the
/// wide Name column, and the repository's name where the Type column would
/// be.
///
/// The path goes in the column that stretches because it is what tells one
/// result from the next. With the bare file name there and the folder in a
/// narrow column, two hundred `Cargo.toml` rows all read
/// `Cargo.toml  PrototypeRust...  PrototypeRustMono...` - found, and
/// impossible to choose between.
fn found_row(found_match: &protocol::NameMatch) -> ContentRow {
    let path = found_match.path.as_str();
    let name = path.rsplit('/').next().unwrap_or(path);
    let (repository, within) = match found_match.repository.as_deref() {
        Some(repository) => {
            let within = if repository.is_empty() {
                path
            } else {
                path.strip_prefix(repository)
                    .and_then(|rest| rest.strip_prefix('/'))
                    .filter(|rest| !rest.is_empty())
                    .unwrap_or(path)
            };
            let label = repository
                .rsplit('/')
                .next()
                .filter(|last| !last.is_empty())
                .unwrap_or(".");
            (label.to_owned(), within)
        }
        None => ("-".to_owned(), path),
    };
    ContentRow {
        branch: String::new(),
        marker: String::new(),
        marker_tooltip: String::new(),
        marker_warning: false,
        icon: icon_for(name, found_match.is_dir),
        is_dir: found_match.is_dir,
        name: if found_match.is_dir {
            format!("{within}/")
        } else {
            within.to_owned()
        },
        size: String::new(),
        kind: repository,
        modified: String::new(),
        is_repository: false,
        mark: None,
        stale_marker: String::new(),
        stale_tooltip: String::new(),
    }
}

/// An All Repositories entry's own folder, `root/location/name` - `root`
/// joined straight with `name` when `location` is empty, a direct child of
/// the root.
fn all_repository_path(root: &Path, entry: &protocol::AllRepositoryEntry) -> PathBuf {
    if entry.location.is_empty() {
        root.join(&entry.name)
    } else {
        root.join(&entry.location).join(&entry.name)
    }
}

/// The key an All Repositories entry's working-tree status is asked for and
/// recorded under in [`App::row_statuses`]: `location/name`, or plain `name`
/// for a direct child of the root. `/` cannot appear inside a single path
/// component, so this can never collide with an ordinary listing row's own
/// name, and the same map serves both without a second one.
fn all_repository_status_key(entry: &protocol::AllRepositoryEntry) -> String {
    if entry.location.is_empty() {
        entry.name.clone()
    } else {
        format!("{}/{}", entry.location, entry.name)
    }
}

/// The presentation half registered under `name`.
fn plugin_presentation_named(name: &str) -> Option<&'static dyn PluginPresentation> {
    PRESENTATION_PLUGINS
        .iter()
        .copied()
        .find(|candidate| candidate.name() == name)
}

/// A command the editor offers in its own row.
///
/// Named rather than passed as a keystroke so a caller cannot ask for
/// something the row does not offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditCommand {
    /// Go back a step.
    Undo,
    /// Put back a step that was undone.
    Redo,
    /// Take the selection to the clipboard.
    Cut,
    /// Put the selection on the clipboard.
    Copy,
    /// Insert what is on the clipboard.
    Paste,
}

/// The tab that opens the editor, and the one shown while it is open.
///
/// Two words rather than one, because a tab reading `Edit` while an
/// editor is already open would look like a button that had stopped
/// working.
const EDIT_TAB: &str = "Edit";
const EDITING_TAB: &str = "Editing";

/// A line drawn in no colour at all, as one run.
fn plain_line(line: &str) -> Vec<ColouredRun> {
    vec![ColouredRun {
        text: line.to_owned(),
        class: Class::Plain,
    }]
}

/// The class the names after a summary label are, or `None` for a label
/// nobody has mapped.
///
/// **A whitelist, and it has to be.** The obvious rule - colour whatever
/// comes before the first colon - was tried against every fixture in
/// the repository and produced labels like `10.0.0.10 - - [08/Sep/2026`,
/// `{"id"` and `[0]`: an access log's address, a JSON key, an array
/// index. A plugin that pretty-prints its content rather than appending
/// it has no summary to speak of, and there is no telling the two apart
/// from the shape of a line.
///
/// So only the labels that name code entities are coloured, and every
/// other line is left exactly as it was. Guessing a class from an
/// unknown label is how a reader comes to trust a colour that means
/// nothing.
fn class_for_label(label: &str) -> Option<Class> {
    match label {
        "functions" | "methods" | "procedures" => Some(Class::Function),
        "classes" | "structs" | "traits" | "enums" | "types" | "interfaces" | "records" => {
            Some(Class::Type)
        }
        _ => None,
    }
}

/// A summary line split into its label and the values after it.
///
/// `None` for an indented line, which is a value belonging to the label
/// above it rather than a label of its own, and for a line with no
/// colon at all.
fn summary_label(line: &str) -> Option<(&str, &str)> {
    if line.starts_with(char::is_whitespace) {
        return None;
    }
    let (label, values) = line.split_once(':')?;
    Some((label, values))
}

/// A summary line as coloured runs, or `None` when it is a line this
/// knows nothing about and should not touch.
///
/// The label is drawn as a keyword because that is what it is: the
/// reserved word of the small format a plugin's summary is written in.
/// The names after it take the class the label says they are, so a type
/// looks the same whether a reader met it here or in the file below.
fn colour_summary_line(line: &str) -> Option<Vec<ColouredRun>> {
    let (label, values) = summary_label(line)?;
    let class = class_for_label(label)?;

    let mut runs = vec![
        ColouredRun {
            text: label.to_owned(),
            class: Class::Keyword,
        },
        ColouredRun {
            text: ":".to_owned(),
            class: Class::Punctuation,
        },
    ];
    let mut rest = values;
    while !rest.is_empty() {
        let (name, after) = rest.split_once(',').unwrap_or((rest, ""));
        if !name.is_empty() {
            // The space before a name belongs to nothing, so it is drawn
            // plain rather than given the name's colour.
            let trimmed = name.trim_start();
            let spaces = name.len() - trimmed.len();
            if spaces > 0 {
                runs.push(ColouredRun {
                    text: name[..spaces].to_owned(),
                    class: Class::Plain,
                });
            }
            if !trimmed.is_empty() {
                runs.push(ColouredRun {
                    text: trimmed.to_owned(),
                    class,
                });
            }
        }
        if after.is_empty() && !rest.contains(',') {
            break;
        }
        runs.push(ColouredRun {
            text: ",".to_owned(),
            class: Class::Punctuation,
        });
        rest = after;
    }
    Some(runs)
}

/// Where the file begins inside `preview`, or `None` when the Preview
/// does not end with it.
///
/// Anchored at the end rather than searched for, and that is the whole
/// of the rule: `present` is the plugin's summary followed by the
/// content, so whatever else is above, the file is the last lines. A
/// summary line that happens to repeat a line of the file therefore
/// changes nothing - the tail either matches whole or it does not.
///
/// `None` for a Preview that is all summary, which is most of the
/// plugins that read a structured format rather than a language.
fn file_starts_in_preview(preview: &[String], text: &str) -> Option<usize> {
    let file: Vec<&str> = text.lines().collect();
    if file.is_empty() {
        return None;
    }
    let from = preview.len().checked_sub(file.len())?;
    (preview[from..] == file[..]).then_some(from)
}

/// One coloured run of a line in the Text view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColouredRun {
    /// The run's text, with no newline in it.
    pub text: String,
    /// What the run is, which is what decides its colour.
    pub class: Class,
}

/// Splits `text` into lines, each a list of runs, using `spans`.
///
/// A span may cross a newline - a block comment usually does - so the
/// break is made here rather than asked of the classifier, which would
/// have to know how the pane lays text out to answer.
///
/// The trailing newline ends the last line rather than starting an empty
/// one, so a file ending in a newline does not draw a blank row that is
/// not in it.
fn colour_lines(text: &str, spans: &[Span]) -> Vec<Vec<ColouredRun>> {
    let mut lines: Vec<Vec<ColouredRun>> = Vec::new();
    let mut line: Vec<ColouredRun> = Vec::new();
    for span in spans {
        let Some(part) = text.get(span.start..span.start + span.len) else {
            continue;
        };
        let mut pieces = part.split('\n');
        if let Some(first) = pieces.next().filter(|first| !first.is_empty()) {
            line.push(ColouredRun {
                text: first.to_owned(),
                class: span.class,
            });
        }
        for piece in pieces {
            lines.push(std::mem::take(&mut line));
            if !piece.is_empty() {
                line.push(ColouredRun {
                    text: piece.to_owned(),
                    class: span.class,
                });
            }
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Drawn beside a repository row's branch when its tracked files have
/// uncommitted changes. `M`, so the shape itself reads "modified" the way
/// `git status` already does, rather than relying on the colour it is also
/// given (#574).
pub const CHANGED_MARKER: &str = "M";

/// Drawn beside a repository row's branch until its status has been asked
/// for and answered. Nothing at all would read as "no changes".
pub const NOT_KNOWN_YET_MARKER: &str = "\u{2026}";

/// Drawn beside a repository row's branch when the answer came back and
/// cannot say: the index could not be read, or the count stopped short
/// without finding a change.
pub const CANNOT_TELL_MARKER: &str = "?";

/// The words a marker's tooltip and accessible label share; empty for no
/// marker at all, which is what a clean row draws.
fn marker_tooltip(marker: &str) -> &'static str {
    match marker {
        CHANGED_MARKER => "Uncommitted changes to tracked files",
        NOT_KNOWN_YET_MARKER => "Checking for changes…",
        CANNOT_TELL_MARKER => "Could not tell whether there are changes",
        _ => "",
    }
}

/// Drawn after a repository row's branch when its last fetch is too old to
/// trust the ahead/behind counts it was measured against, or it has a
/// remote and has never been fetched at all (#589). Matches the File
/// pane's own threshold for the same staleness (#576).
pub const STALE_FETCH_MARKER: &str = "\u{23F0}";

/// How old a fetch has to be, in seconds, before [`STALE_FETCH_MARKER`] is
/// drawn - 30 days, the same threshold the File pane already reads
/// `STALE_FETCH` as (#576).
const STALE_FETCH_SECONDS: u64 = 30 * 24 * 60 * 60;

/// Whether a repository row's last fetch is too old to trust, or it has a
/// remote and was never fetched. A repository with no remote is never
/// stale - there is nothing for it to have fetched (#589). `now` is a
/// parameter so a test can fix the clock.
fn fetch_is_stale(last_fetch: Option<u64>, has_remote: bool, now: u64) -> bool {
    if !has_remote {
        return false;
    }
    match last_fetch {
        Some(at) => now.saturating_sub(at) > STALE_FETCH_SECONDS,
        None => true,
    }
}

/// [`STALE_FETCH_MARKER`]'s tooltip and accessible label: `Last fetched 61
/// days ago; ahead and behind counts may be out of date`, or `Never
/// fetched` for a repository that has a remote but has never fetched it.
fn stale_fetch_tooltip(last_fetch: Option<u64>, now: u64) -> String {
    match last_fetch {
        Some(at) => {
            let days = now.saturating_sub(at) / 86_400;
            let noun = if days == 1 { "day" } else { "days" };
            format!("Last fetched {days} {noun} ago; ahead and behind counts may be out of date")
        }
        None => "Never fetched".to_owned(),
    }
}

/// Seconds since `UNIX_EPOCH`, for comparing against a repository's
/// `last_fetch` - `0` on a clock that reads before the epoch, which never
/// happens on a real machine.
fn now_epoch_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// What the Contents pane knows about one repository row's working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RowStatus {
    /// Asked for, not answered yet.
    Waiting,
    /// Answered: the summary, or `None` when the service could not tell.
    Answered(Option<protocol::WorkingTreeSummary>),
}

/// One contents row, as the details view renders it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentRow {
    /// The type's icon, from its plugin.
    pub icon: Icon,
    /// Whether this row is a source control working copy. Drawn differently
    /// from a plain folder: the Repos Directory is a workspace, and which of
    /// its folders are checkouts is the first thing to see.
    pub is_repository: bool,
    /// Whether the row is a directory, which the icon is drawn as.
    pub is_dir: bool,
    /// Entry name, with a trailing `/` for a directory.
    pub name: String,
    /// Formatted size, empty for a directory.
    pub size: String,
    /// The "Type" column.
    pub kind: String,
    /// Formatted modified time, empty when the filesystem reports none.
    pub modified: String,
    /// A working copy's branch - `detached` when it is on none - drawn
    /// after its name. Empty for any other row.
    pub branch: String,
    /// Beside the branch: [`CHANGED_MARKER`], [`NOT_KNOWN_YET_MARKER`],
    /// [`CANNOT_TELL_MARKER`], or empty for no uncommitted changes.
    pub marker: String,
    /// What `marker` means, in words: shown in a tooltip and given as its
    /// accessible label, since a screen reader user and a mouse user who
    /// does not know the glyphs both get nothing from `marker` alone.
    /// Empty exactly when `marker` is.
    pub marker_tooltip: String,
    /// Whether `marker` is [`CHANGED_MARKER`], so the front end can give it
    /// the warning colour rather than the neutral one the other markers
    /// use.
    pub marker_warning: bool,
    /// The branch mark this row's icon carries, or `None` for a plain
    /// folder or a file (#579).
    pub mark: Option<RepositoryMark>,
    /// [`STALE_FETCH_MARKER`] when the repository's last fetch is too old
    /// to trust, empty otherwise (#589).
    pub stale_marker: String,
    /// What `stale_marker` means, in words: shown in a tooltip and given as
    /// its accessible label. Empty exactly when `stale_marker` is.
    pub stale_tooltip: String,
}

/// One match in the Go to Repository switcher (#590).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitcherRow {
    /// The repository's name.
    pub name: String,
    /// Its path relative to the Repos Directory, minus its own name -
    /// empty until #591 lets a result be nested rather than a direct
    /// child, dimmed beside `name` once it is not.
    pub path: String,
    /// Its branch, `detached` when it is on none - as [`ContentRow::branch`].
    pub branch: String,
    /// As [`ContentRow::marker`], but [`NOT_KNOWN_YET_MARKER`] whenever the
    /// Repos Directory is not the folder on screen, since nothing has asked
    /// this repository's working tree for its status in that case.
    pub marker: String,
    /// As [`ContentRow::marker_tooltip`].
    pub marker_tooltip: String,
    /// As [`ContentRow::marker_warning`].
    pub marker_warning: bool,
    /// Whether this is the switcher's highlighted result.
    pub selected: bool,
}

/// One row of the File pane's fact table (#576).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactRow {
    /// The right-aligned label: `"Branch"`, `"Provider"`. Empty for a
    /// blank row separating a working copy's own facts from the folder
    /// facts that follow.
    pub label: String,
    /// The value, elided to [`FACT_VALUE_BUDGET`] characters when longer.
    pub display_value: String,
    /// The value in full, for a front end to show on hover when it
    /// differs from `display_value`.
    pub full_value: String,
    /// Drawn in the secondary-text colour rather than the foreground.
    pub dim: bool,
}

/// The File pane's "Worktree of"/"Submodule of" line (#587): the clone a
/// worktree shares, or the outer working copy pinning a submodule.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RelatedRepository {
    /// The other working copy is still there - a link to it.
    Link { label: String, path: PathBuf },
    /// A worktree's clone has been removed: nowhere to link to, so the
    /// line says so instead.
    Gone { label: String },
}

/// The Contents pane's filter state (#582): a name typed into its field,
/// and/or the status bar's "N with uncommitted changes" link.
#[derive(Default)]
struct Filter {
    /// Typed into the filter field, matched case-insensitively as a
    /// substring. Empty when nothing is typed.
    text: String,
    /// Whether the status bar's changed-only link has narrowed the
    /// listing to just repositories with uncommitted changes.
    changed_only: bool,
    /// Whether the filter field has the keyboard: a typed character
    /// narrows the listing instead of jumping to a name.
    focused: bool,
}

/// Why the Repos Directory itself could not be listed (#592): decided by
/// looking at the path directly rather than at the service's answer,
/// which only ever sends a stringified `io::Error` with no way to tell
/// "does not exist" apart from "permission denied" without parsing
/// English out of it. Only ever computed for the Repos Directory's own
/// root - a subfolder that fails to list keeps today's status bar
/// message (issue #592, case 6).
#[derive(Debug, Clone, PartialEq, Eq)]
enum RootProblem {
    /// The path, or the drive it names, does not exist.
    NotThere { cause: NotThereCause },
    /// The path exists but could not be read - permission denied, or any
    /// other input/output error. Carries the service's own message,
    /// which is the detail the pane shows.
    NotReadable { message: String },
}

/// The likely reason a Repos Directory path is not there.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NotThereCause {
    /// The path names a drive letter (`Z:\repos`) whose drive itself is
    /// not connected - a mapped network drive before the VPN is up, or an
    /// external disk that is unplugged. Carries the drive, e.g. `"Z:"`.
    DriveNotConnected(String),
    /// The drive, if any, is there; the folder itself is not.
    FolderMissing,
}

impl NotThereCause {
    /// The second line under "is not available": the likely cause.
    fn describe(&self) -> String {
        match self {
            Self::DriveNotConnected(drive) => format!("Drive {drive} is not connected"),
            Self::FolderMissing => "The folder does not exist".to_owned(),
        }
    }
}

/// Whether `root` names a Windows drive letter (`Z:\repos`, or a
/// forward-slash spelling of the same thing), and if so, which one -
/// `"Z:"`. Read from the path's text rather than
/// [`std::path::Component::Prefix`], which only Windows' own path parser
/// ever produces: this way the "drive not connected" case is exercisable
/// by a unit test on any host, the Linux runner that gates every pull
/// request included.
fn drive_letter(root: &Path) -> Option<String> {
    let text = root.to_string_lossy();
    let mut chars = text.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    (chars.next() == Some(':')).then(|| format!("{letter}:"))
}

/// Classifies why `root` is not there: whether it names a drive that is
/// itself missing, or is an ordinary missing folder.
fn not_there_cause(root: &Path) -> NotThereCause {
    let Some(drive) = drive_letter(root) else {
        return NotThereCause::FolderMissing;
    };
    let mut drive_root = drive.clone();
    drive_root.push(std::path::MAIN_SEPARATOR);
    if std::fs::metadata(drive_root).is_err() {
        NotThereCause::DriveNotConnected(drive)
    } else {
        NotThereCause::FolderMissing
    }
}

/// Classifies why the Repos Directory's root listing failed, from the
/// path itself and the message the failed request already carried.
fn classify_root_problem(root: &Path, message: &str) -> RootProblem {
    match std::fs::metadata(root) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => RootProblem::NotThere {
            cause: not_there_cause(root),
        },
        _ => RootProblem::NotReadable {
            message: message.to_owned(),
        },
    }
}

/// What the Contents pane's centred message (#592) says, and which
/// buttons it offers - the pane draws its listing normally when this is
/// `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ContentsMessage {
    /// The Repos Directory, or the drive it is on, is not there.
    NotThere { title: String, cause: String },
    /// The Repos Directory exists but could not be read.
    NotReadable { title: String, detail: String },
    /// The Repos Directory lists, but holds nothing.
    Empty { title: String },
    /// A filter (#582) has narrowed the listing to nothing - never the
    /// "no repositories yet" message, which would be wrong and alarming
    /// over a Repos Directory that is not actually empty.
    FilterEmpty { title: String },
}

/// A button [`ContentsMessage`] offers, in the order Tab reaches them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageButton {
    Retry,
    Choose,
    ClearFilter,
}

/// The three-pane explorer's state.
pub struct App {
    root: FolderNode,
    folder_selected: usize,
    /// The last listing the service sent for the browsed folder, never
    /// itself filtered - what a cleared filter (#582) restores from.
    all_contents: Vec<DirectoryEntry>,
    /// What the Contents pane is actually drawing: `all_contents` when no
    /// filter is narrowing it, or the subset that passes one otherwise.
    /// Every existing index into this - `content_selected`, `selection`,
    /// `anchor` - keeps meaning "row on screen" whichever it holds.
    contents: Vec<DirectoryEntry>,
    /// The Contents pane's filter (#582): a name typed into its field
    /// and/or the status bar's changed-only link.
    filter: Filter,
    /// The highlighted row in the Go to Repository switcher's matches
    /// (#590), while `mode` is `Mode::Switcher`. An index into that
    /// query's matches, not into the full repository list, so it stays
    /// meaningful as the query - and so the set of matches - changes.
    switcher_selected: usize,
    /// Set alongside `reselect` by an operation, a save, or a manual
    /// refresh - a reload of the folder already on screen, as opposed to a
    /// navigation to a different one. Read once by the next listing to
    /// arrive, which is why the filter (#582) survives the former and is
    /// dropped by the latter.
    same_folder_reload: bool,
    content_selected: usize,
    file_view: Option<Response>,
    /// Which of the previewed type's views the pane is showing, as an index
    /// into [`App::file_views`]. Reset whenever a new view arrives, so a
    /// choice made for one file does not carry to the next.
    file_view_index: usize,
    /// The text being edited in the file pane, and the file it belongs to.
    /// `None` when the pane is showing a preview rather than an editor.
    editing_file: Option<Edit>,
    /// The entry name to reselect after the next listing arrives, set
    /// by whichever operation is about to change the folder in place.
    reselect: Option<String>,
    /// Which column the contents pane is sorted by, and in which direction.
    sort_key: SortKey,
    sort_ascending: bool,
    /// Folders visited, in order, and where in them Back/Forward currently
    /// sits. Explorer's arrows walk this rather than the folder tree.
    history: Vec<PathBuf>,
    history_index: usize,
    /// What Ctrl+C or Ctrl+X put aside for the next Ctrl+V.
    /// Every file put aside by the last Ctrl+C or Ctrl+X, and which it
    /// was. A set rather than one path: the pane supports a multiple
    /// selection and Delete already honours it, so Copy taking only the
    /// lead row dropped the rest without a word.
    clipboard: Option<(Vec<PathBuf>, ClipboardMode)>,
    /// Every selected contents row. `content_selected` is the lead row -
    /// the one the preview and the rename/copy prompts act on - and is kept
    /// inside this set whenever the set is non-empty.
    selection: std::collections::BTreeSet<usize>,
    /// The row a Shift range extends from.
    anchor: usize,
    status: Option<String>,
    /// Why the Repos Directory's own root last failed to list (#592),
    /// cleared the moment it lists successfully. `None` while it has
    /// never failed, or last listed fine.
    root_problem: Option<RootProblem>,
    /// How many ticks the Contents pane's message (#592) has been
    /// showing without a fresh listing - one tick is [`Self::tick`]
    /// called once, effectively 100ms at the front end's own timer.
    /// Reaching [`LISTING_MESSAGE_RETRY_TICKS`] re-lists in the
    /// background, so a drive that connects shows its contents without a
    /// click.
    listing_message_ticks: u32,
    /// Which of [`Self::message_buttons`] Tab has highlighted, while the
    /// Contents pane's message (#592) is showing.
    message_focus: usize,
    focus: Pane,
    pending_contents: Option<(Vec<usize>, Receiver<io::Result<Response>>)>,
    pending_file: Option<Receiver<io::Result<Response>>>,
    /// The file the outstanding preview was asked for, and the file the
    /// pane is showing now. Compared so that re-reading the file already
    /// on screen keeps the view the reader chose.
    pending_file_path: Option<PathBuf>,
    /// A search in flight, and the query it was for.
    pending_find: Option<(String, Receiver<io::Result<Response>>)>,
    /// A search's results, while they are what the Contents pane shows.
    ///
    /// The listing underneath stays loaded, so Escape is instant. While
    /// these are showing, every command that acts on a row by joining its
    /// name onto the browsed folder is refused - a result lives in some
    /// other folder, and that join would name a file the reader never
    /// picked. That is the defect the terminal front end had until #524.
    found: Option<Found>,
    /// The All Repositories view (#591), while it is what the Contents pane
    /// shows in place of the listing.
    all_repositories: Option<AllRepositoriesView>,
    /// The All Repositories scan's next poll, in flight.
    pending_all_repositories: Option<Receiver<io::Result<Response>>>,
    /// Working-tree status of the listing's repository rows that have been
    /// asked about, by entry name - or, for an All Repositories row, its
    /// location and name joined by `/`, which cannot collide with a plain
    /// name since `/` never appears inside one. Cleared when a listing
    /// lands, along with every request still outstanding, so an answer
    /// about one folder's `alpha` is never drawn on another folder's.
    row_statuses: HashMap<String, RowStatus>,
    /// Status requests in flight: the entry name each is for, and its
    /// answer.
    pending_statuses: Vec<(String, Receiver<io::Result<Response>>)>,
    shown_file_path: Option<PathBuf>,
    mode: Mode,
    pending_operation: Option<Receiver<io::Result<Response>>>,
    after_operation: Option<AfterOperation>,
    /// The `editor` setting, and whether Visual Studio Code is on the
    /// `PATH` - what "Open in editor" (#581) needs to know whether it has
    /// anything to launch. `None`/`false` until [`Self::set_editor`] is
    /// called, which the real window does once at startup; a test that
    /// wants the item enabled calls it directly, rather than this struct
    /// searching the `PATH` itself and making every test's result depend
    /// on what happens to be installed on the machine running it.
    editor_setting: Option<String>,
    code_on_path: bool,
    /// The window's text zoom (#586), one of [`crate::zoom::STEPS`].
    zoom_percent: u16,
}

/// Strips Windows' `\\?\` verbatim prefix from a canonicalized path.
/// `fs::canonicalize` adds one there, and it then travels through every
/// request into the messages the status bar shows, where `\\?\C:\dir\file`
/// is noise the reader has to look past. Only a drive path is unwrapped: a
/// verbatim UNC path (`\\?\UNC\server\share`) needs its prefix to keep
/// resolving, and paths on other platforms never carry one.
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let unwrapped = {
        let text = path.to_string_lossy();
        text.strip_prefix(r"\\?\")
            .filter(|rest| {
                let mut chars = rest.chars();
                matches!(
                    (chars.next(), chars.next(), chars.next()),
                    (Some(drive), Some(':'), Some('\\')) if drive.is_ascii_alphabetic()
                )
            })
            .map(str::to_owned)
    };
    unwrapped.map_or(path, PathBuf::from)
}

/// How many ticks the Contents pane's message (#592) shows before the
/// window re-lists in the background on its own - ten seconds at the
/// front end's 100ms timer.
const LISTING_MESSAGE_RETRY_TICKS: u32 = 100;

impl App {
    /// Starts a new explorer rooted at `root`, and kicks off loading its
    /// contents in the background.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        // Canonicalize so `root`'s ancestors are well-formed: a relative
        // root such as "." has `Path::parent()` return `Some("")` (an
        // empty path, not `None`), which `navigate_to_parent` would
        // otherwise treat as a real, requestable directory - re-rooting the
        // tree at "" and leaving every future request targeting a path
        // that resolves to nothing. Falls back to the given root if it
        // doesn't exist yet or canonicalization otherwise fails.
        let root = strip_verbatim_prefix(std::fs::canonicalize(&root).unwrap_or(root));
        let mut app = Self {
            root: FolderNode::root(root),
            folder_selected: 0,
            all_contents: Vec::new(),
            contents: Vec::new(),
            filter: Filter::default(),
            switcher_selected: 0,
            same_folder_reload: false,
            content_selected: 0,
            file_view: None,
            file_view_index: 0,
            editing_file: None,
            reselect: None,
            sort_key: SortKey::Name,
            sort_ascending: true,
            history: Vec::new(),
            history_index: 0,
            clipboard: None,
            selection: std::collections::BTreeSet::new(),
            anchor: 0,
            status: None,
            root_problem: None,
            listing_message_ticks: 0,
            message_focus: 0,
            focus: Pane::Folders,
            pending_contents: None,
            pending_file: None,
            pending_file_path: None,
            pending_find: None,
            found: None,
            all_repositories: None,
            pending_all_repositories: None,
            row_statuses: HashMap::new(),
            pending_statuses: Vec::new(),
            shown_file_path: None,
            mode: Mode::Normal,
            pending_operation: None,
            after_operation: None,
            editor_setting: None,
            code_on_path: false,
            zoom_percent: crate::zoom::DEFAULT,
        };
        app.load_contents_for_selected();
        app
    }

    fn selected_dir_path(&self) -> PathBuf {
        let rows = self.root.flatten();
        rows.get(self.folder_selected)
            .and_then(|(_, indices)| self.root.node_at(indices))
            .map_or_else(|| self.root.path.clone(), |node| node.path.clone())
    }

    fn load_contents_for_selected(&mut self) {
        // Any navigation, a refresh included, puts the listing back.
        self.found = None;
        self.all_repositories = None;
        self.pending_all_repositories = None;
        let rows = self.root.flatten();
        let Some((_, indices)) = rows.get(self.folder_selected).cloned() else {
            return;
        };
        let path = self
            .root
            .node_at(&indices)
            .map_or_else(|| self.root.path.clone(), |node| node.path.clone());
        let request = Request::ListDirectory {
            path: path.to_string_lossy().into_owned(),
        };
        self.pending_contents = Some((indices, spawn_request(request)));
        self.status = Some(format!("loading {}...", path.display()));
    }

    /// Shows `view` in the file pane, back at the type's first view. A view
    /// chosen for one file says nothing about the next, which may not even
    /// offer it.
    /// Plants a completed listing, for a test that has one from the
    /// service rather than a hand-written fixture.
    pub fn apply_contents_result_for_test(&mut self, indices: &[usize], response: Response) {
        self.apply_contents_result(indices, Ok(response));
    }

    /// Plants a completed file view, for the same reason.
    pub fn show_file_view_for_test(&mut self, view: Response) {
        self.show_file_view(Some(view));
    }

    fn show_file_view(&mut self, view: Option<Response>) {
        self.show_file_view_of(view, None);
    }

    /// Plants `view`, which is a view of `path`.
    ///
    /// The chosen view resets when the file changes - a tab picked for one
    /// file says nothing about the next. It must not reset when the *same*
    /// file comes back, which is what F5 does: the refresh kept the
    /// reader's row and then dropped them from the Text tab onto Preview,
    /// and so did a second click on the row they were already on.
    fn show_file_view_of(&mut self, view: Option<Response>, path: Option<PathBuf>) {
        let same_file = path.is_some() && path == self.shown_file_path;
        self.file_view = view;
        self.shown_file_path = path;
        if !same_file {
            self.file_view_index = 0;
        }
    }

    fn load_file_view(&mut self) {
        if let Some(found) = &self.found {
            let Some(found_match) = found.matches.get(self.content_selected) else {
                return;
            };
            let path = found.root.join(&found_match.path);
            self.pending_file_path = Some(path.clone());
            self.pending_file = Some(spawn_request(Request::ViewFile {
                path: path.to_string_lossy().into_owned(),
            }));
            return;
        }
        if let Some(view) = &self.all_repositories {
            let Some(entry) = view.entries.get(self.content_selected) else {
                return;
            };
            let path = all_repository_path(&view.root, entry);
            self.pending_file_path = Some(path.clone());
            self.pending_file = Some(spawn_request(Request::ViewFile {
                path: path.to_string_lossy().into_owned(),
            }));
            return;
        }
        let Some(entry) = self.contents.get(self.content_selected) else {
            self.show_file_view(None);
            self.pending_file = None;
            self.pending_file_path = None;
            return;
        };
        let path = self.selected_dir_path().join(&entry.name);
        let request = Request::ViewFile {
            path: path.to_string_lossy().into_owned(),
        };
        self.pending_file_path = Some(path);
        self.pending_file = Some(spawn_request(request));
    }

    /// Applies any background request results that have arrived since the
    /// last call. Call this periodically (e.g. from a UI timer).
    pub fn tick(&mut self) {
        if let Some((indices, rx)) = &self.pending_contents
            && let Ok(result) = rx.try_recv()
        {
            let indices = indices.clone();
            self.pending_contents = None;
            self.apply_contents_result(&indices, result);
        }
        if let Some(rx) = &self.pending_file
            && let Ok(result) = rx.try_recv()
        {
            self.pending_file = None;
            let asked_for = self.pending_file_path.take();
            let view = result.unwrap_or_else(|err| Response::Error {
                message: err.to_string(),
            });
            self.show_file_view_of(Some(view), asked_for);
        }
        if let Some(rx) = &self.pending_operation
            && let Ok(result) = rx.try_recv()
        {
            self.pending_operation = None;
            self.apply_operation_result(result);
        }
        if let Some((_, rx)) = &self.pending_find
            && let Ok(result) = rx.try_recv()
            && let Some((query, _)) = self.pending_find.take()
        {
            self.apply_find_result(&query, result);
        }
        if let Some(rx) = &self.pending_all_repositories
            && let Ok(result) = rx.try_recv()
        {
            self.pending_all_repositories = None;
            self.apply_all_repositories_result(result);
        }
        let mut answered = Vec::new();
        self.pending_statuses
            .retain(|(name, rx)| match rx.try_recv() {
                Ok(result) => {
                    answered.push((name.clone(), result));
                    false
                }
                Err(mpsc::TryRecvError::Empty) => true,
                Err(mpsc::TryRecvError::Disconnected) => {
                    answered.push((name.clone(), Err(io::Error::other("no answer"))));
                    false
                }
            });
        for (name, result) in answered {
            self.apply_status_result(&name, result);
        }
        // While the Contents pane's message (#592) is showing for the
        // Repos Directory itself - not there, not readable, or empty -
        // re-list in the background every ten seconds, so a drive that
        // connects shows its contents without a click. A filter (#582)
        // narrowing a real listing to nothing is not this: nothing about
        // the folder itself changes while a reader is typing into it.
        if self.pending_contents.is_some()
            || !matches!(
                self.contents_message(),
                Some(
                    ContentsMessage::NotThere { .. }
                        | ContentsMessage::NotReadable { .. }
                        | ContentsMessage::Empty { .. }
                )
            )
        {
            self.listing_message_ticks = 0;
        } else {
            self.listing_message_ticks += 1;
            if self.listing_message_ticks >= LISTING_MESSAGE_RETRY_TICKS {
                self.listing_message_ticks = 0;
                self.refresh();
            }
        }
    }

    /// Asks the service for the working-tree status of each repository row
    /// in `rows` - the rows on screen - that has not been asked about since
    /// the listing landed.
    ///
    /// One request per row, made after the listing is drawn rather than
    /// before: the branch is in the listing already, and the changes cost a
    /// pass over a checkout's tracked files that a listing of forty
    /// checkouts must not wait for (GUIDANCE.md 3.5, rule 9). A row already
    /// asked about is never asked again, answered or not.
    pub fn ask_for_statuses(&mut self, rows: std::ops::Range<usize>) {
        if self.found.is_some() {
            return;
        }
        if let Some(view) = &self.all_repositories {
            let root = view.root.clone();
            let end = rows.end.min(view.entries.len());
            let start = rows.start.min(end);
            let candidates: Vec<protocol::AllRepositoryEntry> =
                view.entries.get(start..end).unwrap_or_default().to_vec();
            for entry in candidates {
                let key = all_repository_status_key(&entry);
                if self.row_statuses.contains_key(&key) {
                    continue;
                }
                self.row_statuses.insert(key.clone(), RowStatus::Waiting);
                let request = Request::WorkingTreeStatus {
                    path: all_repository_path(&root, &entry)
                        .to_string_lossy()
                        .into_owned(),
                };
                self.pending_statuses.push((key, spawn_request(request)));
            }
            return;
        }
        let folder = self.selected_dir_path();
        let end = rows.end.min(self.contents.len());
        let start = rows.start.min(end);
        for entry in self.contents.get(start..end).unwrap_or_default() {
            if entry.repository.is_none() || self.row_statuses.contains_key(&entry.name) {
                continue;
            }
            self.row_statuses
                .insert(entry.name.clone(), RowStatus::Waiting);
            let request = Request::WorkingTreeStatus {
                path: folder.join(&entry.name).to_string_lossy().into_owned(),
            };
            self.pending_statuses
                .push((entry.name.clone(), spawn_request(request)));
        }
    }

    /// Plants a status answer for the row named `name`, for a test that
    /// cannot wait on a real checkout.
    pub fn apply_status_result_for_test(&mut self, name: &str, response: Response) {
        self.apply_status_result(name, Ok(response));
    }

    /// Records the answer for the row named `name` - if that row was asked
    /// about in the listing on screen. A row never asked about, or asked
    /// about in a listing since replaced, has nowhere for it to go.
    fn apply_status_result(&mut self, name: &str, result: io::Result<Response>) {
        let status = match result {
            Ok(Response::WorkingTree { status, .. }) => status,
            _ => None,
        };
        if let Some(row) = self.row_statuses.get_mut(name) {
            *row = RowStatus::Answered(status);
        }
    }

    /// The marker drawn beside the branch of the repository row `name`.
    fn marker_for(&self, name: &str) -> &'static str {
        match self.row_statuses.get(name) {
            None | Some(RowStatus::Waiting) => NOT_KNOWN_YET_MARKER,
            Some(RowStatus::Answered(Some(status))) if status.changed > 0 => CHANGED_MARKER,
            Some(RowStatus::Answered(Some(status))) if !status.partial => "",
            Some(RowStatus::Answered(_)) => CANNOT_TELL_MARKER,
        }
    }

    /// An All Repositories row (#591) as a Contents row: its branch and
    /// change marker filled in exactly as an ordinary listing row's are,
    /// visible rows only, and its Location - the parent folder its own row
    /// would otherwise not name - where the Type column would be.
    fn all_repository_row(&self, now: u64, entry: &protocol::AllRepositoryEntry) -> ContentRow {
        let marker = self.marker_for(&all_repository_status_key(entry));
        let stale = fetch_is_stale(
            entry.repository.last_fetch,
            entry.repository.remote.is_some(),
            now,
        );
        let stale_tooltip = if stale {
            stale_fetch_tooltip(entry.repository.last_fetch, now)
        } else {
            String::new()
        };
        ContentRow {
            icon: icon_for(&entry.name, true),
            is_dir: true,
            name: format!("{}/", entry.name),
            size: String::new(),
            kind: if entry.location.is_empty() {
                ".".to_owned()
            } else {
                entry.location.clone()
            },
            modified: String::new(),
            is_repository: true,
            mark: repository_mark(true, entry.repository.provider.as_deref()),
            branch: entry
                .repository
                .branch
                .clone()
                .unwrap_or_else(|| "detached".to_owned()),
            marker: marker.to_owned(),
            marker_tooltip: marker_tooltip(marker).to_owned(),
            marker_warning: marker == CHANGED_MARKER,
            stale_marker: if stale {
                STALE_FETCH_MARKER.to_owned()
            } else {
                String::new()
            },
            stale_tooltip,
        }
    }

    /// `, 12 repositories (3 not known)` for a listing that holds working
    /// copies, and nothing for one that holds none. The changed count that
    /// used to sit in this sentence is [`Self::status_changed_label`] now:
    /// the status bar draws it as a link (#582).
    fn repositories_summary(&self) -> String {
        let markers: Vec<&str> = self
            .contents
            .iter()
            .filter(|entry| entry.repository.is_some())
            .map(|entry| self.marker_for(&entry.name))
            .collect();
        if markers.is_empty() {
            return String::new();
        }
        let noun = if markers.len() == 1 {
            "repository"
        } else {
            "repositories"
        };
        let not_known = markers
            .iter()
            .filter(|marker| matches!(**marker, NOT_KNOWN_YET_MARKER | CANNOT_TELL_MARKER))
            .count();
        let not_known = if not_known == 0 {
            String::new()
        } else {
            format!(" ({not_known} not known)")
        };
        let stale = self.stale_fetch_count();
        let stale = if stale == 0 {
            String::new()
        } else {
            format!(", {stale} not fetched in 30 days")
        };
        format!(", {} {noun}{not_known}{stale}", markers.len())
    }

    /// How many listed repositories have a last fetch too old to trust, or
    /// a remote and no fetch at all - the same test [`STALE_FETCH_MARKER`]
    /// is drawn from, so this always agrees with the marker beside each
    /// row's branch (#589).
    fn stale_fetch_count(&self) -> usize {
        let now = now_epoch_seconds();
        self.contents
            .iter()
            .filter(|entry| {
                entry.repository.as_ref().is_some_and(|repository| {
                    fetch_is_stale(repository.last_fetch, repository.remote.is_some(), now)
                })
            })
            .count()
    }

    /// How many listed repositories carry uncommitted changes, by the same
    /// marker the row beside their branch already shows - so this always
    /// agrees with what clicking the status bar's link (#582) would filter
    /// the pane down to.
    fn changed_marker_count(&self) -> usize {
        self.contents
            .iter()
            .filter(|entry| entry.repository.is_some())
            .filter(|entry| self.marker_for(&entry.name) == CHANGED_MARKER)
            .count()
    }

    /// Rebuilds the displayed listing from [`Self::all_contents`] - the
    /// last one the service sent - by the filter typed into the Contents
    /// pane's field and/or the status bar's changed-only link (#582).
    /// Reads nothing new: narrowing is a view of a listing already in
    /// hand, not a fresh directory read.
    fn recompute_contents(&mut self) {
        self.contents = self
            .all_contents
            .iter()
            .filter(|entry| self.entry_passes_filter(entry))
            .cloned()
            .collect();
    }

    fn entry_passes_filter(&self, entry: &DirectoryEntry) -> bool {
        if self.filter.changed_only && self.marker_for(&entry.name) != CHANGED_MARKER {
            return false;
        }
        self.filter.text.is_empty()
            || entry
                .name
                .to_lowercase()
                .contains(&self.filter.text.to_lowercase())
    }

    /// Puts the selection back on the first row after a filter (#582)
    /// changes what the pane is showing: the entry the old index pointed
    /// to may now be a different row, or gone.
    fn reset_selection_after_filter(&mut self) {
        self.content_selected = 0;
        self.anchor = 0;
        self.selection.clear();
        if !self.contents.is_empty() {
            self.selection.insert(0);
        }
        self.load_file_view();
    }

    fn apply_contents_result(&mut self, indices: &[usize], result: io::Result<Response>) {
        self.status = None;
        let same_folder_reload = std::mem::take(&mut self.same_folder_reload);
        if indices.is_empty() {
            self.root_problem = None;
            self.listing_message_ticks = 0;
        }
        match result {
            Ok(Response::Directory { entries }) => {
                if let Some(node) = self.root.node_at_mut(indices) {
                    node.set_children_from(&entries);
                }
                // A new listing, even of the same folder, starts its
                // statuses again: what was known belonged to the rows it
                // replaces.
                self.row_statuses.clear();
                self.pending_statuses.clear();
                self.all_contents = entries;
                self.sort_contents();
                // The filter (#582) is a view of the folder on screen, so a
                // navigation to a different one drops it; a reload of this
                // same folder - after an operation, a save, or a manual
                // refresh - keeps it.
                if !same_folder_reload {
                    self.filter = Filter::default();
                }
                self.recompute_contents();
                // A listing arriving after an operation is the same folder
                // reloaded, so put the selection back on the entry that
                // operation produced rather than dropping it to the top.
                self.content_selected = self
                    .reselect
                    .take()
                    .and_then(|name| self.contents.iter().position(|entry| entry.name == name))
                    .unwrap_or(0);
                self.anchor = self.content_selected;
                self.selection.clear();
                if !self.contents.is_empty() {
                    self.selection.insert(self.content_selected);
                }
                self.load_file_view();
            }
            Ok(Response::Error { message }) => self.fail_contents_listing(indices, message),
            Ok(
                Response::FileView { .. }
                | Response::Done
                | Response::ReposRoots { .. }
                | Response::Names { .. }
                | Response::WorkingTree { .. }
                | Response::AllRepositories { .. },
            ) => {
                self.status = Some("expected a directory listing".to_owned());
            }
            Err(err) => {
                let message = err.to_string();
                self.fail_contents_listing(indices, message);
            }
        }
    }

    /// A directory listing failed: for the Repos Directory's own root,
    /// classifies why (#592) and clears what it was showing, so the
    /// Folders and File panes agree with the Contents pane's message
    /// (issue #592, case 5). A subfolder keeps today's status bar
    /// message instead (case 6).
    fn fail_contents_listing(&mut self, indices: &[usize], message: String) {
        if indices.is_empty() {
            self.root.children = None;
            self.all_contents.clear();
            self.contents.clear();
            self.selection.clear();
            self.content_selected = 0;
            self.anchor = 0;
            self.show_file_view(None);
            self.root_problem = Some(classify_root_problem(&self.root.path, &message));
        } else {
            self.status = Some(message);
        }
    }

    fn apply_operation_result(&mut self, result: io::Result<Response>) {
        let after = self.after_operation.take();
        match result {
            Ok(Response::Done) => {
                self.status = None;
                self.load_contents_for_selected();
                if let Some(AfterOperation::EnterRename { path, input }) = after {
                    self.mode = Mode::RenameInput { path, input };
                }
            }
            Ok(Response::Error { message }) => self.status = Some(message),
            Ok(_) => self.status = Some("unexpected response to operation".to_owned()),
            Err(err) => self.status = Some(err.to_string()),
        }
    }

    fn selected_entry_path(&self) -> Option<(PathBuf, String)> {
        let entry = self.contents.get(self.content_selected)?;
        Some((
            self.selected_dir_path().join(&entry.name),
            entry.name.clone(),
        ))
    }

    /// Asks for confirmation before deleting the selected contents row.
    pub fn request_delete(&mut self) {
        // A prompt already on screen owns the keyboard. Delete inside a
        // rename box is a reader clearing the pre-filled name, not asking
        // to delete anything - and arming a confirmation there left the
        // file one keystroke from the Recycle Bin, because the first
        // saw asked. `move_selection`, `select_all` and `undo` all guard
        // the same way.
        if !self.pane_command_allowed() {
            return;
        }
        let indices = self.selected_indices();
        if indices.is_empty() {
            return;
        }
        let dir = self.selected_dir_path();
        let paths: Vec<PathBuf> = indices
            .iter()
            .filter_map(|index| self.contents.get(*index))
            .map(|entry| dir.join(&entry.name))
            .collect();
        if paths.is_empty() {
            return;
        }
        let name = match indices.len() {
            1 => self
                .contents
                .get(indices[0])
                .map_or_else(String::new, |entry| entry.name.clone()),
            count => format!("{count} items"),
        };
        self.mode = Mode::ConfirmDelete { paths, name };
    }

    /// Confirms a pending delete confirmation, sending the delete request.
    pub fn confirm_delete(&mut self) {
        let Mode::ConfirmDelete { paths, .. } = std::mem::replace(&mut self.mode, Mode::Normal)
        else {
            return;
        };
        let request = Request::Delete {
            paths: paths
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
        };
        self.pending_operation = Some(spawn_request(request));
        self.same_folder_reload = true;
        self.status = Some("deleting...".to_owned());
    }

    /// Declines a pending delete confirmation, returning to normal mode.
    pub fn decline_delete(&mut self) {
        self.mode = Mode::Normal;
    }

    /// Starts editing a new name to rename the selected contents row to.
    pub fn request_rename(&mut self) {
        if !self.pane_command_allowed() {
            return;
        }
        if let Some((path, name)) = self.selected_entry_path() {
            self.mode = Mode::RenameInput { path, input: name };
        }
    }

    /// Starts editing a destination name to copy the selected contents row
    /// to, de-duplicated against the current listing: the source's own name
    /// always collides, and accepting it copies the file onto itself.
    pub fn request_copy(&mut self) {
        if !self.pane_command_allowed() {
            return;
        }
        if let Some((path, name)) = self.selected_entry_path() {
            let input = dedup_name(&self.content_names(), &name);
            self.mode = Mode::CopyInput { path, input };
        }
    }

    /// Starts editing a destination directory name to extract the selected
    /// contents row (an archive) into.
    pub fn request_extract(&mut self) {
        if !self.pane_command_allowed() {
            return;
        }
        if let Some((path, name)) = self.selected_entry_path() {
            let suggested = std::path::Path::new(&name)
                .file_stem()
                .map_or_else(|| name.clone(), |stem| stem.to_string_lossy().into_owned());
            self.mode = Mode::ExtractInput {
                path,
                input: suggested,
            };
        }
    }

    /// Creates a new, empty subdirectory of the currently-browsed folder
    /// under a de-duplicated default name, then drops into inline rename so
    /// the user can immediately retype it.
    pub fn request_new_folder(&mut self) {
        self.request_create(true);
    }

    /// Creates a new, empty file in the currently-browsed folder under a
    /// de-duplicated default name, then drops into inline rename so the
    /// user can immediately retype it.
    pub fn request_new_file(&mut self) {
        self.request_create(false);
    }

    fn request_create(&mut self, is_dir: bool) {
        if !self.pane_command_allowed() {
            return;
        }
        let base = if is_dir { "New folder" } else { "New file" };
        let name = dedup_name(&self.content_names(), base);
        let path = self.selected_dir_path().join(&name);
        let request = if is_dir {
            Request::CreateDirectory {
                path: path.to_string_lossy().into_owned(),
            }
        } else {
            Request::CreateFile {
                path: path.to_string_lossy().into_owned(),
            }
        };
        self.pending_operation = Some(spawn_request(request));
        self.reselect = Some(name.clone());
        self.same_folder_reload = true;
        self.after_operation = Some(AfterOperation::EnterRename { path, input: name });
        self.status = Some("working...".to_owned());
    }

    fn input_mut(&mut self) -> Option<&mut String> {
        match &mut self.mode {
            Mode::RenameInput { input, .. }
            | Mode::CopyInput { input, .. }
            | Mode::ExtractInput { input, .. }
            | Mode::PathInput { input }
            | Mode::FindInput { input }
            | Mode::ReposRootInput { input } => Some(input),
            // The switcher's own `type_into_switcher`/`switcher_backspace`
            // edit `query` directly, so a query change also resets which
            // match is highlighted - one generic `&mut String` cannot do
            // that.
            Mode::Normal | Mode::ConfirmDelete { .. } | Mode::Switcher { .. } => None,
        }
    }

    /// Ctrl+V, from the key, the toolbar or the Edit menu.
    ///
    /// While a prompt is taking typed text - the address bar, rename, copy
    /// to, extract to, the Repos Directory - it is the system clipboard's
    /// text that goes in. Otherwise it is the file paste it has always
    /// been.
    ///
    /// Every route to Paste used to go straight to the file clipboard, which
    /// refuses while a prompt is open, so copying a path from a terminal and
    /// pasting it into the address bar did nothing at all. Before the
    /// prompts were guarded it was worse: it pasted files into the folder
    /// while the reader was typing a path.
    pub fn paste(&mut self, clipboard: &mut dyn editor::Clipboard) {
        if self.input_mut().is_none() {
            self.paste_from_clipboard();
            return;
        }
        let Some(text) = clipboard.read() else {
            return;
        };
        // One line. A path copied from a terminal usually carries its
        // newline, and a prompt that confirms on Enter must not take one
        // as part of a name.
        let line = text.lines().next().unwrap_or_default();
        if let Some(input) = self.input_mut() {
            input.extend(line.chars().filter(|c| typeable(*c)));
        }
    }

    /// Confirms a pending rename/copy/extract input, sending its request.
    pub fn confirm_text_input(&mut self) {
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        // Each arm carries the name the folder will hold afterwards, so the
        // reloaded listing can put the selection back on it.
        let request = match mode {
            Mode::RenameInput { path, input } if !input.is_empty() => Some((
                Request::Rename {
                    items: vec![(
                        path.to_string_lossy().into_owned(),
                        sibling_path(&path, &input),
                    )],
                },
                input,
            )),
            Mode::CopyInput { path, input } if !input.is_empty() => Some((
                Request::Copy {
                    items: vec![(
                        path.to_string_lossy().into_owned(),
                        sibling_path(&path, &input),
                    )],
                },
                input,
            )),
            Mode::ExtractInput { path, input } if !input.is_empty() => Some((
                Request::Extract {
                    archive: path.to_string_lossy().into_owned(),
                    destination: sibling_path(&path, &input),
                },
                input,
            )),
            // A typed path is a navigation, not an operation on a file, so
            // it goes straight to browsing rather than through the operation
            // queue. A path that does not exist fails the way any other
            // listing does, with the service's own message.
            Mode::FindInput { input } if !input.trim().is_empty() => {
                let query = input.trim().to_owned();
                self.pending_find = Some((
                    query.clone(),
                    spawn_request(Request::FindNames {
                        query: query.clone(),
                        limit: FIND_LIMIT,
                    }),
                ));
                self.status = Some(format!("finding \"{query}\" in every repository..."));
                return;
            }
            Mode::PathInput { input } if !input.trim().is_empty() => {
                let target = PathBuf::from(input.trim());
                self.remember_current();
                self.push_history(target.clone());
                self.browse(target);
                return;
            }
            // Settling the Repos Directory does two things at once: it opens
            // there now, and it tells the service to open there every time
            // from now on. The service validates the path and refuses one
            // that is not a directory, so a typo answers rather than
            // silently taking effect at the next launch.
            Mode::ReposRootInput { input } if !input.trim().is_empty() => {
                let target = PathBuf::from(input.trim());
                self.pending_operation = Some(spawn_request(Request::SetReposRoot {
                    path: target.to_string_lossy().into_owned(),
                }));
                self.status = Some(format!("opening at {} from now on", target.display()));
                self.mode = Mode::Normal;
                self.push_history(target.clone());
                self.browse(target);
                return;
            }
            _ => None,
        };
        if let Some((request, produced)) = request {
            self.reselect = Some(produced);
            self.same_folder_reload = true;
            self.pending_operation = Some(spawn_request(request));
            self.status = Some("working...".to_owned());
        }
    }

    /// Handles a single character typed while a rename/copy/extract input
    /// is active; a no-op otherwise.
    pub fn type_char(&mut self, text: &str) {
        let Some(c) = text.chars().next() else {
            return;
        };
        if !typeable(c) {
            return;
        }
        if let Some(input) = self.input_mut() {
            input.push(c);
        }
    }

    /// Removes the last character of a pending rename/copy/extract input,
    /// or of the Contents pane's filter text (#582) while it has the
    /// keyboard; a no-op otherwise.
    pub fn backspace(&mut self) {
        if matches!(self.mode, Mode::Normal) && self.filter.focused {
            self.filter.text.pop();
            self.recompute_contents();
            self.reset_selection_after_filter();
            return;
        }
        if matches!(self.mode, Mode::Switcher { .. }) {
            self.switcher_backspace();
            return;
        }
        if let Some(input) = self.input_mut() {
            input.pop();
        }
    }

    /// Confirms a pending rename/copy/extract input on Return; on macOS,
    /// also starts a rename in normal mode, per §2.3's platform table (a
    /// delete confirmation uses y/n instead, via [`Self::handle_key_text`]).
    pub fn handle_return(&mut self) {
        self.handle_return_for_os(std::env::consts::OS);
    }

    /// [`Self::handle_return`], parameterized on the OS name so the
    /// macOS-specific behavior is exercisable from `cargo test` on any
    /// host, including the Linux CI runner that gates merges.
    fn handle_return_for_os(&mut self, os: &str) {
        match self.mode {
            Mode::RenameInput { .. }
            | Mode::CopyInput { .. }
            | Mode::ExtractInput { .. }
            | Mode::PathInput { .. }
            | Mode::FindInput { .. }
            | Mode::ReposRootInput { .. } => {
                self.confirm_text_input();
            }
            // The filter (#582) already narrows as it is typed; Return
            // just hands the keyboard back to the listing.
            Mode::Normal if self.filter.focused => self.filter.focused = false,
            // The Contents pane's message (#592) is showing, so Return
            // activates whichever button Tab highlighted rather than
            // renaming or opening a row that is not there.
            Mode::Normal if self.contents_message().is_some() => {
                self.activate_focused_message_button();
            }
            // Return renames on macOS, which is that platform's
            // convention and the reason this is parameterised at all.
            Mode::Normal if os == "macos" => self.request_rename(),
            Mode::Normal => self.activate_selection(),
            Mode::ConfirmDelete { .. } => {}
            Mode::Switcher { .. } => self.confirm_switcher(),
        }
    }

    /// Opens whatever is selected in the pane that has focus: a folder
    /// in the contents pane is drilled into, and a row in the folders
    /// tree is expanded or collapsed. Both are what double-clicking
    /// that row already does; this is the keyboard's way to the same
    /// place, and every file manager has it.
    ///
    /// A file is left alone. This application reads files rather than
    /// launching them (D10 in spirit: it looks, it does not drive), and
    /// selecting one has already shown it in the File pane.
    pub fn activate_selection(&mut self) {
        match self.focus {
            Pane::Folders => self.toggle_folder(self.folder_selected),
            Pane::Contents | Pane::File => self.open_content(self.content_selected),
        }
    }

    /// Dispatches one typed character by the current mode: a hotkey in
    /// normal mode (`r`/`c`/`x`), y/n during a delete confirmation, or an
    /// appended character during a rename/copy/extract input.
    pub fn handle_key_text(&mut self, text: &str) {
        match &self.mode {
            Mode::ConfirmDelete { .. } => match text {
                "y" | "Y" => self.confirm_delete(),
                "n" | "N" => self.decline_delete(),
                _ => {}
            },
            Mode::RenameInput { .. }
            | Mode::CopyInput { .. }
            | Mode::ExtractInput { .. }
            | Mode::PathInput { .. }
            | Mode::FindInput { .. }
            | Mode::ReposRootInput { .. } => {
                self.type_char(text);
            }
            // While the filter field has the keyboard (#582), a typed
            // character narrows the listing rather than jumping to a name.
            Mode::Normal if self.filter.focused => self.type_into_filter(text),
            // Explorer's type-ahead: a typed letter jumps to a name, it is
            // not a command. Rename, copy and extract are on F2, Ctrl+C and
            // the context menu.
            Mode::Normal => self.type_ahead(text),
            // The switcher's own query (#590), fuzzy-matched rather than
            // narrowing a listing.
            Mode::Switcher { .. } => self.type_into_switcher(text),
        }
    }

    /// Appends one typed character to the Contents pane's filter text
    /// (#582), narrowing the listing live. The same guard [`Self::type_char`]
    /// uses for a rename, so a control character never lands in either.
    fn type_into_filter(&mut self, text: &str) {
        let Some(c) = text.chars().next() else {
            return;
        };
        if !typeable(c) {
            return;
        }
        self.filter.text.push(c);
        self.recompute_contents();
        self.reset_selection_after_filter();
    }

    /// Whether the selected contents row previewed as an archive. The
    /// contents pane greys its Extract menu item out when this is false,
    /// since extracting anything else only ever produces an error.
    #[must_use]
    pub fn selected_is_archive(&self) -> bool {
        matches!(
            &self.file_view,
            Some(Response::FileView { plugin, .. }) if plugin == "archive"
        )
    }

    /// Moves the selection in the focused pane by `delta` rows, clamped to
    /// that pane's bounds. The file pane has no rows of its own, so an arrow
    /// there moves the contents pane - the one whose selection it is
    /// previewing. Ignored while a name is being typed, where the arrow keys
    /// belong to the input.
    pub fn move_selection(&mut self, delta: i32) {
        if let Mode::Switcher { query } = &self.mode {
            let Some(last) = self.switcher_matches(query).len().checked_sub(1) else {
                return;
            };
            self.switcher_selected = if delta < 0 {
                self.switcher_selected
                    .saturating_sub(delta.unsigned_abs() as usize)
            } else {
                self.switcher_selected
                    .saturating_add(delta.unsigned_abs() as usize)
                    .min(last)
            };
            return;
        }
        if !matches!(self.mode, Mode::Normal) {
            return;
        }
        let (len, current) = match self.focus {
            Pane::Folders => (self.root.flatten().len(), self.folder_selected),
            Pane::Contents | Pane::File => (self.listed_len(), self.content_selected),
        };
        let Some(last) = len.checked_sub(1) else {
            return;
        };
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            current
                .saturating_add(delta.unsigned_abs() as usize)
                .min(last)
        };
        match self.focus {
            Pane::Folders => self.select_folder(next),
            Pane::Contents | Pane::File => self.select_content(next),
        }
    }

    /// Moves focus one pane to the right (`delta` positive) or left,
    /// wrapping at either end. Ignored while a name is being typed, where
    /// the arrow keys belong to the input.
    pub fn cycle_focus(&mut self, delta: i32) {
        if !matches!(self.mode, Mode::Normal) {
            return;
        }
        self.focus = match (self.focus, delta < 0) {
            (Pane::Folders, false) | (Pane::File, true) => Pane::Contents,
            (Pane::Contents, false) | (Pane::Folders, true) => Pane::File,
            (Pane::File, false) | (Pane::Contents, true) => Pane::Folders,
        };
    }

    /// Cancels any pending request; a late result is simply discarded when
    /// it arrives, since its receiver is dropped.
    pub fn cancel_pending(&mut self) {
        if self.editing_file.is_some() {
            self.cancel_file_edit();
            return;
        }
        if matches!(self.mode, Mode::Normal)
            && let Some(found) = self.found.take()
        {
            self.pending_find = None;
            self.select_content(found.previous_selected);
            self.status = None;
            return;
        }
        if matches!(self.mode, Mode::Normal)
            && let Some(view) = self.all_repositories.take()
        {
            self.pending_all_repositories = None;
            self.select_content(view.previous_selected);
            self.status = None;
            return;
        }
        // Escape in Contents (#582): drops a filter the same way "clear"
        // does, rather than falling through to the generic cancel below,
        // which has nothing else pending to say "cancelled" about.
        if matches!(self.mode, Mode::Normal) && self.focus == Pane::Contents && self.filtering() {
            self.clear_filters();
            self.status = None;
            return;
        }
        let cancelled = self.pending_contents.take().is_some()
            | self.pending_file.take().is_some()
            | self.pending_operation.take().is_some();
        self.mode = Mode::Normal;
        if cancelled {
            self.status = Some("cancelled".to_owned());
        }
    }

    /// Selects folder row `index`, loading its contents.
    pub fn select_folder(&mut self, index: usize) {
        if index < self.root.flatten().len() {
            self.folder_selected = index;
            self.focus = Pane::Folders;
            self.load_contents_for_selected();
        }
    }

    /// Toggles expand/collapse for folder row `index`.
    pub fn toggle_folder(&mut self, index: usize) {
        let rows = self.root.flatten();
        let Some((_, indices)) = rows.get(index).cloned() else {
            return;
        };
        if let Some(node) = self.root.node_at_mut(&indices) {
            node.expanded = !node.expanded;
        }
        // Collapsing a folder takes its descendants out of the listing,
        // and the selection may have been one of them. Left pointing past
        // the end it selects nothing, and every rule that reads it -
        // which folder to list, which folder to drill into - quietly does
        // nothing instead. The folder just collapsed is where a reader
        // would expect to be, and is where Explorer leaves them.
        if self.folder_selected >= self.root.flatten().len() {
            self.folder_selected = index;
        }
    }

    /// Selects contents row `index`, loading its preview if it is a file.
    pub fn select_content(&mut self, index: usize) {
        if index < self.listed_len() {
            self.content_selected = index;
            self.anchor = index;
            self.selection.clear();
            self.selection.insert(index);
            self.focus = Pane::Contents;
            self.load_file_view();
        }
    }

    /// Ctrl+click: adds `index` to the selection, or takes it out again if it
    /// was already in. The lead row moves onto whatever was clicked, or onto
    /// another selected row when the lead itself is deselected.
    pub fn toggle_content(&mut self, index: usize) {
        if self.found.is_some() || self.all_repositories.is_some() {
            return;
        }
        if index >= self.contents.len() {
            return;
        }
        self.focus = Pane::Contents;
        self.anchor = index;
        if self.selection.remove(&index) {
            // Deselecting the lead row hands the lead to another selected
            // one, so the preview keeps showing something that is selected.
            if index == self.content_selected
                && let Some(next) = self.selection.iter().next().copied()
            {
                self.content_selected = next;
            }
        } else {
            self.selection.insert(index);
            self.content_selected = index;
        }
        self.load_file_view();
    }

    /// Shift+click: selects every row between the anchor and `index`.
    pub fn extend_selection_to(&mut self, index: usize) {
        if self.found.is_some() || self.all_repositories.is_some() {
            return;
        }
        if index >= self.contents.len() {
            return;
        }
        let (low, high) = if self.anchor <= index {
            (self.anchor, index)
        } else {
            (index, self.anchor)
        };
        self.selection = (low..=high).collect();
        self.content_selected = index;
        self.focus = Pane::Contents;
        self.load_file_view();
    }

    /// Shift+arrow: grows or shrinks the selected range to the row the
    /// arrow would have landed on, the way Shift+click extends to the row
    /// clicked. The anchor stays put, so a Shift+Up back over it flips the
    /// range rather than growing it the other way.
    ///
    /// Only the Contents pane has a range to extend; the tree selects one
    /// folder at a time, so a held Shift there means what no Shift means.
    pub fn extend_selection_by(&mut self, delta: i32) {
        if !matches!(self.mode, Mode::Normal) || !matches!(self.focus, Pane::Contents | Pane::File)
        {
            self.move_selection(delta);
            return;
        }
        if let Some(next) = self.row_offset_from(self.content_selected, delta) {
            self.extend_selection_to(next);
        }
    }

    /// Shift+Home and Shift+End: extends the range to the first or last
    /// row rather than moving to it.
    pub fn extend_selection_to_edge(&mut self, last: bool) {
        if !matches!(self.mode, Mode::Normal) || self.contents.is_empty() {
            return;
        }
        if !matches!(self.focus, Pane::Contents | Pane::File) {
            self.select_edge(last);
            return;
        }
        let index = if last { self.contents.len() - 1 } else { 0 };
        self.extend_selection_to(index);
    }

    /// The contents row `delta` away from `current`, clamped to the
    /// listing. `None` when there are no rows to land on.
    fn row_offset_from(&self, current: usize, delta: i32) -> Option<usize> {
        let last = self.contents.len().checked_sub(1)?;
        Some(if delta < 0 {
            current.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            current
                .saturating_add(delta.unsigned_abs() as usize)
                .min(last)
        })
    }

    /// A rubber-band drag: selects every row the band covered, inclusive.
    /// The lead row is the far end, where the pointer was released, so a
    /// following Shift+click extends from there.
    pub fn select_range(&mut self, from: usize, to: usize) {
        if self.found.is_some() || self.all_repositories.is_some() {
            return;
        }
        if self.contents.is_empty() {
            return;
        }
        let last = self.contents.len() - 1;
        let (from, to) = (from.min(last), to.min(last));
        let (low, high) = if from <= to { (from, to) } else { (to, from) };
        self.selection = (low..=high).collect();
        self.content_selected = to;
        self.anchor = from;
        self.focus = Pane::Contents;
        self.load_file_view();
    }

    /// Ctrl+A: selects every row in the folder.
    pub fn select_all(&mut self) {
        if !self.pane_command_allowed() || self.contents.is_empty() {
            return;
        }
        self.selection = (0..self.contents.len()).collect();
        self.focus = Pane::Contents;
    }

    /// Whether the clipboard holds something a paste would act on, which
    /// is what greys the Paste command out when it does not.
    #[must_use]
    pub const fn can_paste(&self) -> bool {
        self.clipboard.is_some()
    }

    /// Whether any row is selected, which the commands acting on a
    /// selection are enabled by.
    #[must_use]
    pub fn has_selection(&self) -> bool {
        !self.selection.is_empty()
    }

    /// Whether exactly the archive plugin previewed the lead row, which is
    /// the only case Extract can do anything in.
    #[must_use]
    pub fn can_extract(&self) -> bool {
        self.selected_is_archive()
    }

    /// Puts a line in the status bar. Used by Help > About, which has
    /// nothing else to report.
    pub fn report(&mut self, message: &str) {
        self.status = Some(message.to_owned());
    }

    /// Every selected row, in listing order.
    /// The names in the browsed folder, for the collision checks that only
    /// care what a thing is called.
    fn content_names(&self) -> Vec<String> {
        self.contents
            .iter()
            .map(|entry| entry.name.clone())
            .collect()
    }

    fn selected_indices(&self) -> Vec<usize> {
        self.selection.iter().copied().collect()
    }

    /// How many rows are selected.
    #[must_use]
    pub fn selected_count(&self) -> usize {
        self.selection.len()
    }

    /// Whether contents row `index` is part of the selection.
    #[must_use]
    pub fn is_selected(&self, index: usize) -> bool {
        self.selection.contains(&index)
    }

    /// Drills into contents row `index` if it is a directory, expanding and
    /// selecting it in the folders tree.
    pub fn open_content(&mut self, index: usize) {
        if self.found.is_some() {
            self.open_found(index);
            return;
        }
        if self.all_repositories.is_some() {
            self.open_all_repositories_entry(index);
            return;
        }
        let Some(entry) = self.contents.get(index).cloned() else {
            return;
        };
        if !entry.is_dir {
            return;
        }
        let rows = self.root.flatten();
        let Some((_, parent_indices)) = rows.get(self.folder_selected).cloned() else {
            return;
        };
        let Some(child_index) = self
            .root
            .node_at(&parent_indices)
            .and_then(|parent| parent.children.as_ref())
            .and_then(|children| children.iter().position(|node| node.name == entry.name))
        else {
            return;
        };

        let mut child_indices = parent_indices;
        child_indices.push(child_index);
        // Every folder on the way down, not only the one being opened.
        // `flatten` lists the rows a collapsed tree *shows*, so with an
        // ancestor closed the new row did not exist: the lookup below
        // found nothing, the selection stayed where it was, and the
        // listing reloaded the folder it was already showing. Return did
        // nothing and said nothing.
        //
        // Expanding the whole path is also the right answer rather than
        // merely a working one - it is what File Explorer does, and it
        // keeps the tree showing where the reader actually is.
        for depth in 0..=child_indices.len() {
            if let Some(node) = self.root.node_at_mut(&child_indices[..depth]) {
                node.expanded = true;
            }
        }

        let new_rows = self.root.flatten();
        if let Some(row) = new_rows.iter().position(|(_, idx)| idx == &child_indices) {
            self.folder_selected = row;
        }
        // Focus stays on the contents, which is what the reader is now
        // looking at. It used to move to the tree, and then the arrow
        // keys moved the tree instead of the listing, and a second
        // Return collapsed the folder just opened rather than going
        // one deeper.
        self.focus = Pane::Contents;
        self.load_contents_for_selected();
    }

    /// Navigates to the parent of the directory currently shown in the
    /// contents pane. A no-op if that directory has no parent (the
    /// filesystem root).
    pub fn navigate_to_parent(&mut self) {
        let Some(parent) = self.selected_dir_path().parent().map(PathBuf::from) else {
            return;
        };
        if !self.may_navigate() {
            return;
        }
        self.remember_current();
        self.push_history(parent.clone());
        let rows = self.root.flatten();
        let Some((_, indices)) = rows.get(self.folder_selected).cloned() else {
            return;
        };
        if let Some((_, parent_indices)) = indices.split_last() {
            let parent_indices = parent_indices.to_vec();
            if let Some(row) = rows.iter().position(|(_, idx)| idx == &parent_indices) {
                self.select_folder(row);
            }
            return;
        }
        self.browse(parent);
    }

    /// Ctrl+C: puts the selected entry aside for a later paste, leaving it
    /// where it is.
    pub fn copy_to_clipboard(&mut self) {
        self.set_clipboard(ClipboardMode::Copy);
    }

    /// Ctrl+X: puts the selected entry aside to be moved by a later paste.
    pub fn cut_to_clipboard(&mut self) {
        self.set_clipboard(ClipboardMode::Cut);
    }

    fn set_clipboard(&mut self, mode: ClipboardMode) {
        if !self.pane_command_allowed() {
            return;
        }
        // The whole selection, read the way `request_delete` reads it, so
        // the two agree about what "selected" means.
        let indices = self.selected_indices();
        let dir = self.selected_dir_path();
        let paths: Vec<PathBuf> = indices
            .iter()
            .filter_map(|index| self.contents.get(*index))
            .map(|entry| dir.join(&entry.name))
            .collect();
        if paths.is_empty() {
            return;
        }
        let verb = match mode {
            ClipboardMode::Copy => "copied",
            ClipboardMode::Cut => "cut",
        };
        // Named when there is one, counted when there are several - the
        // same rule that already produces "Delete 2 items?".
        let what = match indices.len() {
            1 => self
                .contents
                .get(indices[0])
                .map_or_else(String::new, |entry| entry.name.clone()),
            count => format!("{count} items"),
        };
        self.clipboard = Some((paths, mode));
        self.status = Some(format!("{what} {verb}"));
    }

    /// Ctrl+V: copies or moves whatever the clipboard holds into the folder
    /// being browsed. The destination name is de-duplicated, since both
    /// operations refuse to replace an existing entry.
    pub fn paste_from_clipboard(&mut self) {
        if !self.pane_command_allowed() {
            return;
        }
        let Some((sources, mode)) = self.clipboard.clone() else {
            return;
        };
        let destination_dir = self.selected_dir_path();
        // Each name is de-duplicated against the destination *and* against
        // the names this paste has already claimed, or two files landing
        // beside each other would both ask for the same free name.
        let mut taken = self.content_names();
        let mut items = Vec::new();
        let mut last = None;
        for source in &sources {
            let Some(name) = source
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
            else {
                continue;
            };
            // Pasting back into the folder it came from has to land beside
            // the original rather than on it.
            let name = if source.parent() == Some(destination_dir.as_path()) {
                dedup_name(&taken, &name)
            } else {
                name
            };
            taken.push(name.clone());
            items.push((
                source.to_string_lossy().into_owned(),
                destination_dir.join(&name).to_string_lossy().into_owned(),
            ));
            last = Some(name);
        }
        if items.is_empty() {
            return;
        }
        let request = match mode {
            ClipboardMode::Copy => Request::Copy { items },
            ClipboardMode::Cut => Request::Rename { items },
        };
        if mode == ClipboardMode::Cut {
            self.clipboard = None;
        }
        self.reselect = last;
        self.same_folder_reload = true;
        self.pending_operation = Some(spawn_request(request));
        self.status = Some("working...".to_owned());
    }

    /// Plants a file view, as a completed preview request would. Test-only:
    /// the real path arrives through [`Self::tick`].
    #[cfg(test)]
    fn set_file_view(&mut self, plugin: &str, data: serde_json::Value) {
        self.show_file_view(Some(Response::FileView {
            plugin: plugin.to_owned(),
            data,
            also: Vec::new(),
        }));
    }

    /// Plants a folder view carrying further views, as a completed preview
    /// of a folder several plugins recognise would. Test-only.
    #[cfg(test)]
    fn set_folder_view(
        &mut self,
        plugin: &str,
        data: serde_json::Value,
        also: Vec<protocol::PluginView>,
    ) {
        self.show_file_view(Some(Response::FileView {
            plugin: plugin.to_owned(),
            data,
            also,
        }));
    }

    /// The previewed file's text, when its plugin can edit it. `None` for a
    /// type that is not text, or a view holding only part of one - the
    /// plugin decides, per GUIDANCE.md §3.
    #[must_use]
    pub fn editable_text(&self) -> Option<String> {
        match &self.file_view {
            Some(Response::FileView { plugin, data, .. }) => PRESENTATION_PLUGINS
                .iter()
                .find(|candidate| candidate.name() == plugin)
                .and_then(|candidate| candidate.editable_text(data)),
            _ => None,
        }
    }

    /// Whether the selected file can be opened in the editor.
    #[must_use]
    pub fn can_edit(&self) -> bool {
        self.editing_file.is_none() && self.editable_text().is_some()
    }

    /// Whether the file pane is currently an editor.
    #[must_use]
    pub const fn editing_file(&self) -> bool {
        self.editing_file.is_some()
    }

    /// The text in the editor, or an empty string when it is closed.
    #[must_use]
    pub fn edit_text(&self) -> String {
        self.editing_file
            .as_ref()
            .map_or_else(String::new, |edit| edit.document.text().to_owned())
    }

    /// Whether the hand-written surface is drawing the file, rather than
    /// Slint's plain box.
    #[must_use]
    pub fn editing_in_colour(&self) -> bool {
        self.editing_file.as_ref().is_some_and(|edit| edit.coloured)
    }

    /// The editor's lines, coloured, for the surface to draw.
    #[must_use]
    pub fn edit_lines(&self) -> Vec<Vec<ColouredRun>> {
        let Some(edit) = self.editing_file.as_ref() else {
            return Vec::new();
        };
        if !edit.coloured {
            return Vec::new();
        }
        let text = edit.document.text();
        let spans = edit
            .plugin
            .and_then(plugin_presentation_named)
            .map(|plugin| plugin.classify(text))
            .filter(|spans| !spans.is_empty())
            .unwrap_or_else(|| vec![Span::new(0, text.len(), Class::Plain)]);
        colour_lines(text, &spans)
    }

    /// Which presentation half opened the file being viewed.
    fn file_view_plugin(&self) -> Option<&'static dyn PluginPresentation> {
        let Some(Response::FileView { plugin, .. }) = &self.file_view else {
            return None;
        };
        plugin_presentation_named(plugin)
    }

    /// Where the caret is, as a line and a column.
    #[must_use]
    pub fn edit_caret(&self) -> (usize, usize) {
        self.editing_file.as_ref().map_or((0, 0), |edit| {
            let caret = edit.document.caret();
            (edit.document.line_of(caret), edit.document.column_of(caret))
        })
    }

    /// The selection, as a start and an end in lines and columns.
    #[must_use]
    pub fn edit_selection(&self) -> Option<((usize, usize), (usize, usize))> {
        let edit = self.editing_file.as_ref()?;
        let range = edit.document.selection()?;
        Some((
            (
                edit.document.line_of(range.start),
                edit.document.column_of(range.start),
            ),
            (
                edit.document.line_of(range.end),
                edit.document.column_of(range.end),
            ),
        ))
    }

    /// The widest line, in columns, which is how far the surface scrolls.
    #[must_use]
    pub fn edit_longest_line(&self) -> usize {
        self.editing_file.as_ref().map_or(0, |edit| {
            edit.document
                .text()
                .lines()
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(0)
        })
    }

    /// Whether the file has been changed since it was opened.
    ///
    /// Compared against the text rather than counted from the undo
    /// stack, so typing a letter and taking it out again leaves nothing
    /// to save - which is what a reader means by unchanged.
    #[must_use]
    pub fn edit_modified(&self) -> bool {
        self.editing_file
            .as_ref()
            .is_some_and(|edit| edit.document.text() != edit.original)
    }

    /// Whether the editor has a step to go back to.
    #[must_use]
    pub fn edit_can_undo(&self) -> bool {
        self.editing_file
            .as_ref()
            .is_some_and(|edit| edit.document.can_undo())
    }

    /// Whether it has one to put back.
    #[must_use]
    pub fn edit_can_redo(&self) -> bool {
        self.editing_file
            .as_ref()
            .is_some_and(|edit| edit.document.can_redo())
    }

    /// Whether anything is selected, which is what Cut and Copy need.
    #[must_use]
    pub fn edit_has_selection(&self) -> bool {
        self.editing_file
            .as_ref()
            .is_some_and(|edit| edit.document.selection().is_some())
    }

    /// The caret's line and column, counted from one.
    ///
    /// One-based because that is what every editor a reader has used
    /// shows, and what every compiler error they will paste it into
    /// means.
    #[must_use]
    pub fn edit_position(&self) -> (usize, usize) {
        let (line, column) = self.edit_caret();
        (line + 1, column + 1)
    }

    /// One of the editor's own commands, run from the pane's row.
    ///
    /// Routed through the same keystroke handler the keyboard uses, so
    /// a button and its shortcut cannot come to mean different things.
    pub fn edit_command(&mut self, command: EditCommand, clipboard: &mut dyn editor::Clipboard) {
        let (text, control) = match command {
            EditCommand::Undo => ("z", true),
            EditCommand::Redo => ("y", true),
            EditCommand::Cut => ("x", true),
            EditCommand::Copy => ("c", true),
            EditCommand::Paste => ("v", true),
        };
        self.edit_key(clipboard, text, false, control, 1);
    }

    /// A keystroke, for the editor to make sense of.
    pub fn edit_key(
        &mut self,
        clipboard: &mut dyn editor::Clipboard,
        text: &str,
        shift: bool,
        control: bool,
        rows: usize,
    ) -> bool {
        let Some(edit) = self.editing_file.as_mut() else {
            return false;
        };
        editor::handle_key(&mut edit.document, clipboard, text, shift, control, rows)
    }

    /// A click in the editor.
    pub fn edit_click(&mut self, line: usize, column: usize, extend: bool) {
        if let Some(edit) = self.editing_file.as_mut() {
            editor::handle_click(&mut edit.document, line, column, extend);
        }
    }

    /// Opens the selected file in the editor, if its plugin can edit it.
    pub fn begin_file_edit(&mut self) {
        if !matches!(self.mode, Mode::Normal) {
            return;
        }
        let Some(text) = self.editable_text() else {
            return;
        };
        let Some((path, _)) = self.selected_entry_path() else {
            return;
        };
        let coloured = editor::code_editor_suits(&text);
        if !coloured {
            // Said rather than silently downgraded: a reader who has
            // just lost syntax colouring should be told why, not left to
            // wonder whether it is broken.
            self.status = Some(
                "editing as plain text: this file is in a script the coloured \
                 editor cannot place a caret in"
                    .to_owned(),
            );
        }
        let plugin = self.file_view_plugin().map(PluginPresentation::name);
        self.editing_file = Some(Edit {
            path,
            original: text.clone(),
            document: Document::new(text),
            coloured,
            plugin,
        });
        self.focus = Pane::File;
    }

    /// Takes the editor's text as the user has changed it.
    pub fn set_edit_text(&mut self, text: &str) {
        // Only the plain box needs this: it owns its own text while the
        // reader types, and hands it back on save. The coloured surface
        // reports every keystroke as it happens, so its document is
        // already current and overwriting it here would undo the caret.
        if let Some(edit) = self.editing_file.as_mut()
            && !edit.coloured
        {
            edit.document = Document::new(text);
        }
    }

    /// Ctrl+S: writes the editor's text back through the service, which is
    /// the only process that touches the filesystem.
    pub fn save_file_edit(&mut self) {
        let Some(edit) = self.editing_file.as_ref() else {
            return;
        };
        let (path, text) = (edit.path.clone(), edit.document.text().to_owned());
        // The write reloads the folder, and a reload with nothing to put
        // the selection back on lands it at row 0. Every other operation
        // that reloads says where to land first; this one did not, so
        // saving the last file in a folder threw the reader onto the
        // first one and started previewing it.
        self.reselect = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        self.same_folder_reload = true;
        self.editing_file = None;
        self.pending_operation = Some(spawn_request(Request::WriteFile {
            path: path.to_string_lossy().into_owned(),
            content: text,
        }));
        self.status = Some("saving...".to_owned());
    }

    /// Closes the editor without writing. The service keeps no record of a
    /// discarded edit, so this is the one place the text is lost - which is
    /// why the status bar says so rather than closing silently.
    pub fn cancel_file_edit(&mut self) {
        if self.editing_file.take().is_some() {
            self.status = Some("edit discarded".to_owned());
        }
    }

    /// Ctrl+L, F4, or a click past the last segment: turns the address bar
    /// into a text field holding the current path, for typing or pasting one.
    pub fn begin_path_edit(&mut self) {
        if !matches!(self.mode, Mode::Normal) {
            return;
        }
        self.mode = Mode::PathInput {
            input: self.selected_dir_path().to_string_lossy().into_owned(),
        };
    }

    /// Opens the prompt for the Repos Directory, seeded with `suggestion`.
    ///
    /// Used twice: on a first run, where the suggestion is the platform's
    /// default, and from the menu, where it is whatever is configured now.
    pub fn begin_repos_root_edit(&mut self, suggestion: &str) {
        self.mode = Mode::ReposRootInput {
            input: suggestion.to_owned(),
        };
    }

    /// Whether the application is asking where the repositories are.
    #[must_use]
    pub const fn choosing_repos_root(&self) -> bool {
        matches!(self.mode, Mode::ReposRootInput { .. })
    }

    /// The path being typed, or an empty string when the address bar is
    /// showing its segments.
    #[must_use]
    pub fn path_input(&self) -> String {
        match &self.mode {
            Mode::PathInput { input } => input.clone(),
            Mode::FindInput { input } => format!("Find: {input}"),
            _ => String::new(),
        }
    }

    /// Whether the address bar is currently a text field.
    #[must_use]
    pub const fn editing_path(&self) -> bool {
        matches!(self.mode, Mode::PathInput { .. } | Mode::FindInput { .. })
    }

    /// The browsed folder's full path, for the status bar and for copying.
    #[must_use]
    pub fn current_path(&self) -> String {
        self.selected_dir_path().to_string_lossy().into_owned()
    }

    /// Ctrl+Z: asks the service to reverse the last operation. The service
    /// holds what that is; this front end only asks, and reloads whatever
    /// comes back.
    pub fn undo(&mut self) {
        if !matches!(self.mode, Mode::Normal) {
            return;
        }
        // While the editor is open this is the editor's undo. It used to
        // be the filesystem's whatever was on screen, so the button
        // marked Undo reached past what somebody was typing and put back
        // a file they had deleted earlier - with nothing to tell the two
        // apart. In every Windows application, Undo inside an editor
        // undoes typing.
        if self.editing_file() {
            self.undo_edit();
            return;
        }
        self.pending_operation = Some(spawn_request(Request::Undo));
        self.same_folder_reload = true;
        self.status = Some("undoing...".to_owned());
    }

    /// The editor's undo, with no clipboard to reach for.
    pub fn undo_edit(&mut self) {
        if let Some(edit) = self.editing_file.as_mut() {
            edit.document.undo();
        }
    }

    /// F5: re-reads the folder being browsed, or, while All Repositories
    /// (#591) is open, discards its cached scan and starts another.
    pub fn refresh(&mut self) {
        if let Some(view) = &mut self.all_repositories {
            view.entries.clear();
            view.done = false;
            let root = view.root.clone();
            self.status = Some("Looking for repositories…".to_owned());
            self.pending_all_repositories = Some(spawn_request(Request::AllRepositories {
                root: root.to_string_lossy().into_owned(),
                refresh: true,
            }));
            return;
        }
        self.reselect = self
            .contents
            .get(self.content_selected)
            .map(|entry| entry.name.clone());
        self.same_folder_reload = true;
        self.load_contents_for_selected();
    }

    /// Moves the contents selection to the first or last row.
    pub fn select_edge(&mut self, last: bool) {
        if !matches!(self.mode, Mode::Normal) || self.listed_len() == 0 {
            return;
        }
        let index = if last { self.listed_len() - 1 } else { 0 };
        self.select_content(index);
    }

    /// Jumps to the first entry whose name starts with `prefix`, matched
    /// without regard to case, the way Explorer's type-ahead does. Search
    /// starts after the current row so repeated presses cycle through the
    /// matches.
    pub fn type_ahead(&mut self, prefix: &str) {
        if !matches!(self.mode, Mode::Normal)
            || prefix.is_empty()
            || self.found.is_some()
            || self.all_repositories.is_some()
        {
            return;
        }
        // The letter goes to the pane that is drawn as the focused one.
        // It always went to the listing, so with the tree focused - which
        // is how the window opens - the arrows moved the tree and a typed
        // letter jumped the listing behind it.
        //
        // The tree gets its own rather than going silent, because a
        // Repos Directory is a list of repository folders and three
        // letters is how anybody reaches one of them. Explorer's
        // navigation pane does the same.
        if matches!(self.focus, Pane::Folders) {
            self.type_ahead_in_tree(prefix);
            return;
        }
        let prefix = prefix.to_lowercase();
        let count = self.contents.len();
        let found = (1..=count)
            .map(|step| (self.content_selected + step) % count)
            .find(|index| {
                self.contents[*index]
                    .name
                    .to_lowercase()
                    .starts_with(&prefix)
            });
        if let Some(index) = found {
            self.select_content(index);
        }
    }

    /// Type-ahead over the visible tree rows. Only what is on screen: a
    /// letter should reach what the reader can see, and a collapsed
    /// folder's children are not part of the list they are looking at.
    fn type_ahead_in_tree(&mut self, prefix: &str) {
        let prefix = prefix.to_lowercase();
        let rows = self.root.flatten();
        let count = rows.len();
        if count == 0 {
            return;
        }
        let found = (1..=count)
            .map(|step| (self.folder_selected + step) % count)
            .find(|index| {
                rows.get(*index)
                    .and_then(|(_, indices)| self.root.node_at(indices))
                    .is_some_and(|node| node.name.to_lowercase().starts_with(&prefix))
            });
        if let Some(index) = found {
            self.select_folder(index);
        }
    }

    /// Re-roots the tree at `path` and browses it, without touching history.
    fn browse(&mut self, path: PathBuf) {
        self.root = FolderNode::root(path);
        self.folder_selected = 0;
        self.load_contents_for_selected();
    }

    /// Records `path` as the newest history entry. Anything ahead of the
    /// current position is dropped, the way a browser discards the forward
    /// stack once you navigate somewhere new. A repeat of the current entry
    /// is not recorded.
    fn push_history(&mut self, path: PathBuf) {
        if self.history.get(self.history_index) == Some(&path) {
            return;
        }
        if !self.history.is_empty() {
            self.history.truncate(self.history_index + 1);
        }
        self.history.push(path);
        self.history_index = self.history.len() - 1;
    }

    /// Records wherever the app is now, so Back has somewhere to return to.
    /// Called before any navigation that changes the browsed folder.
    fn remember_current(&mut self) {
        let current = self.selected_dir_path();
        if self.history.is_empty() {
            self.history.push(current);
            self.history_index = 0;
        } else {
            self.push_history(current);
        }
    }

    /// Whether Back has an earlier folder to return to.
    #[must_use]
    pub const fn can_go_back(&self) -> bool {
        self.history_index > 0
    }

    /// Whether anything has been asked of the service and not yet answered.
    ///
    /// For tests that drive a real window: the status bar is not a
    /// reliable "still working" signal, because a file preview is
    /// requested without setting one. A test that watched the status
    /// alone settled while the preview was still in flight and then
    /// asserted about a pane holding the file before it - which passed or
    /// failed depending on how long the plugin took.
    #[must_use]
    pub const fn is_busy(&self) -> bool {
        self.pending_contents.is_some()
            || self.pending_file.is_some()
            || self.pending_operation.is_some()
            || self.pending_find.is_some()
    }

    /// The browsed folder as one string, for the markup to notice that it
    /// changed. What the address bar draws comes from `breadcrumbs`; this
    /// is only the thing a `changed` handler can watch, because the
    /// breadcrumb model is rebuilt on every timer tick whether the folder
    /// moved or not.
    #[must_use]
    pub fn address_path(&self) -> String {
        self.selected_dir_path().to_string_lossy().into_owned()
    }

    /// Whether Open has anywhere to go: a folder to step into.
    ///
    /// Open was enabled on any selection and did nothing at all for a
    /// file - black, clickable and inert. It stays a navigation command
    /// rather than growing a second meaning, because the pointer reaches
    /// it by double-click and two clicks in the same place are easy to
    /// land by accident; a file is read in the File pane and edited from
    /// its Edit tab.
    #[must_use]
    pub fn can_open(&self) -> bool {
        self.contents
            .get(self.content_selected)
            .is_some_and(|entry| entry.is_dir)
    }

    /// Where the selected repository's web page is hosted - the provider
    /// the listing already shows - or `None` when there is nothing to open:
    /// a plain folder, a file, or a checkout with no remote a browser could
    /// reach.
    #[must_use]
    pub fn web_provider(&self) -> Option<String> {
        self.selected_web_page().map(|(provider, _)| provider)
    }

    /// Opens the selected repository's web page, at its branch, by handing
    /// the address to `launch`.
    ///
    /// The launcher is passed in rather than called here, so this can be
    /// tested without a browser opening; the window's wiring passes the
    /// platform's.
    pub fn open_on_the_web(&mut self, launch: impl FnOnce(&str) -> io::Result<()>) {
        let Some((_, address)) = self.selected_web_page() else {
            return;
        };
        self.status = Some(match launch(&address) {
            Ok(()) => format!("opened {address}"),
            Err(err) => format!("could not open a browser: {err}"),
        });
    }

    fn selected_web_page(&self) -> Option<(String, String)> {
        let repository = self
            .contents
            .get(self.content_selected)?
            .repository
            .as_ref()?;
        let address = plugin_directory::repository::web_address(
            repository.remote.as_deref()?,
            repository.branch.as_deref(),
        )?;
        let provider = repository
            .provider
            .clone()
            .unwrap_or_else(|| "the web".to_owned());
        Some((provider, address))
    }

    /// The folder a Contents-pane row menu item or a File-menu item hands
    /// to a terminal, an editor or the file manager (#581): the selected
    /// row, when it is a folder. `None` for a selected file, or nothing
    /// selected at all - the same gate [`Self::can_open`] uses, since both
    /// mean "step into this folder".
    fn selected_content_folder(&self) -> Option<PathBuf> {
        let entry = self.contents.get(self.content_selected)?;
        entry
            .is_dir
            .then(|| self.selected_dir_path().join(&entry.name))
    }

    /// The remote address a Contents-pane row's "Copy remote address"
    /// copies, or `None` when the row is not a working copy with a remote.
    fn selected_content_remote(&self) -> Option<String> {
        self.contents
            .get(self.content_selected)?
            .repository
            .as_ref()?
            .remote
            .clone()
    }

    /// The tree node a Folders-pane row menu item acts on: the selected row.
    fn selected_folder_node(&self) -> Option<&FolderNode> {
        let rows = self.root.flatten();
        let (_, indices) = rows.get(self.folder_selected)?;
        self.root.node_at(indices)
    }

    /// Whether the Contents pane's selected row is a working copy with a
    /// remote address to copy (#581).
    #[must_use]
    pub fn can_copy_selected_remote_address(&self) -> bool {
        self.selected_content_remote().is_some()
    }

    /// Whether the Folders pane's selected row is a working copy with a
    /// remote address to copy (#581).
    #[must_use]
    pub fn can_copy_folder_remote_address(&self) -> bool {
        self.selected_folder_node()
            .is_some_and(|node| node.remote.is_some())
    }

    /// Whether "Open in editor" (#581) has anywhere to send a folder: an
    /// `editor` setting, or Visual Studio Code on the `PATH`. Set once at
    /// startup by [`Self::set_editor`].
    #[must_use]
    pub fn editor_available(&self) -> bool {
        self.editor_setting.is_some() || self.code_on_path
    }

    /// What this platform's file manager is called (#581): "Show in File
    /// Explorer", "Show in Finder" or "Show in Files".
    #[must_use]
    pub fn file_manager_label(&self) -> &'static str {
        launch::file_manager_label(Platform::current())
    }

    /// Whether "Open in editor" is enabled for the Contents pane's row menu
    /// and the File menu: a folder selected, with something to open it in.
    #[must_use]
    pub fn can_open_selected_in_editor(&self) -> bool {
        self.can_open() && self.editor_available()
    }

    /// Records what "Open in editor" (#581) has to launch: the `editor`
    /// setting from the settings file, and whether Visual Studio Code is on
    /// the `PATH`.
    ///
    /// Called once, by the real window at startup
    /// (`gui::settings::load_editor`, `gui::launch::on_path("code")`); a
    /// test calls it directly to force a deterministic scenario, rather
    /// than this struct searching the `PATH` itself and making every
    /// test's result depend on what happens to be installed on the machine
    /// running it.
    pub fn set_editor(&mut self, editor_setting: Option<String>, code_on_path: bool) {
        self.editor_setting = editor_setting;
        self.code_on_path = code_on_path;
    }

    /// The window's text zoom (#586), a percentage.
    #[must_use]
    pub fn zoom_percent(&self) -> u16 {
        self.zoom_percent
    }

    /// [`Self::zoom_percent`] as the multiplier `app.slint` scales text,
    /// row heights and icon sizes by.
    #[must_use]
    pub fn zoom_factor(&self) -> f32 {
        f32::from(self.zoom_percent) / 100.0
    }

    /// Sets the zoom to `percent` outright, with no status message - what
    /// the real window does once at startup with a remembered level
    /// (`gui::settings::load_zoom`), the same as [`Self::set_editor`] for
    /// the `editor` setting. A test that wants a status message to go with
    /// the change calls [`Self::zoom_in`], [`Self::zoom_out`] or
    /// [`Self::zoom_reset`] instead.
    pub fn set_zoom_percent(&mut self, percent: u16) {
        self.zoom_percent = percent;
    }

    /// Steps the zoom to the level above the current one, stopping at the
    /// top of `zoom::STEPS`, and says so in the status bar.
    pub fn zoom_in(&mut self) {
        self.set_zoom_and_report(crate::zoom::step_in(self.zoom_percent));
    }

    /// Steps the zoom to the level below the current one, stopping at the
    /// bottom of `zoom::STEPS`, and says so in the status bar.
    pub fn zoom_out(&mut self) {
        self.set_zoom_and_report(crate::zoom::step_out(self.zoom_percent));
    }

    /// Puts the zoom back to `zoom::DEFAULT`, and says so in the status bar.
    pub fn zoom_reset(&mut self) {
        self.set_zoom_and_report(crate::zoom::DEFAULT);
    }

    /// Moves to `percent`, and flashes it in the status bar - but only when
    /// it actually moved, the same reasoning `zoom_in` and `zoom_out`
    /// stopping at the ends of the table already carries: zooming in
    /// already at 200% should not claim the status bar with a message
    /// that says nothing happened.
    fn set_zoom_and_report(&mut self, percent: u16) {
        if percent != self.zoom_percent {
            self.zoom_percent = percent;
            self.status = Some(format!("Zoom {percent}%"));
        }
    }

    fn report_launch(&mut self, what: &str, result: io::Result<()>) {
        self.status = Some(match result {
            Ok(()) => format!("opened {what}"),
            Err(err) => format!("could not open {what}: {err}"),
        });
    }

    /// Builds and hands off the terminal command for `path` (#581): Windows
    /// Terminal or PowerShell, `xdg-terminal-exec` or `x-terminal-emulator`,
    /// or `open -a Terminal` on macOS - see `launch::terminal_launch`.
    fn launch_terminal(&mut self, path: &Path, launch: impl FnOnce(&Launch) -> io::Result<()>) {
        let platform = Platform::current();
        let preferred_available = match platform {
            Platform::Windows => launch::on_path("wt"),
            Platform::Linux => launch::on_path("xdg-terminal-exec"),
            Platform::MacOs => false,
        };
        let command = launch::terminal_launch(platform, path, preferred_available);
        let result = launch(&command);
        self.report_launch("a terminal", result);
    }

    fn launch_editor(&mut self, path: &Path, launch: impl FnOnce(&Launch) -> io::Result<()>) {
        let Some(command) =
            launch::editor_launch(self.editor_setting.as_deref(), self.code_on_path, path)
        else {
            return;
        };
        let result = launch(&command);
        self.report_launch("an editor", result);
    }

    fn launch_file_manager(&mut self, path: &Path, launch: impl FnOnce(&Launch) -> io::Result<()>) {
        let command = launch::file_manager_launch(Platform::current(), path);
        let result = launch(&command);
        self.report_launch("the file manager", result);
    }

    /// Opens a terminal at the Contents pane's selected folder (#581).
    pub fn open_terminal_here(&mut self, launch: impl FnOnce(&Launch) -> io::Result<()>) {
        if let Some(path) = self.selected_content_folder() {
            self.launch_terminal(&path, launch);
        }
    }

    /// Opens a terminal at the Folders pane's selected folder (#581).
    pub fn open_terminal_at_folder(&mut self, launch: impl FnOnce(&Launch) -> io::Result<()>) {
        let path = self.selected_dir_path();
        self.launch_terminal(&path, launch);
    }

    /// Opens the Contents pane's selected folder in an editor (#581).
    pub fn open_selected_in_editor(&mut self, launch: impl FnOnce(&Launch) -> io::Result<()>) {
        if let Some(path) = self.selected_content_folder() {
            self.launch_editor(&path, launch);
        }
    }

    /// Opens the Folders pane's selected folder in an editor (#581).
    pub fn open_folder_in_editor(&mut self, launch: impl FnOnce(&Launch) -> io::Result<()>) {
        let path = self.selected_dir_path();
        self.launch_editor(&path, launch);
    }

    /// Shows the Contents pane's selected folder in the platform's file
    /// manager (#581).
    pub fn show_selected_in_file_manager(
        &mut self,
        launch: impl FnOnce(&Launch) -> io::Result<()>,
    ) {
        if let Some(path) = self.selected_content_folder() {
            self.launch_file_manager(&path, launch);
        }
    }

    /// Shows the Folders pane's selected folder in the platform's file
    /// manager (#581).
    pub fn show_folder_in_file_manager(&mut self, launch: impl FnOnce(&Launch) -> io::Result<()>) {
        let path = self.selected_dir_path();
        self.launch_file_manager(&path, launch);
    }

    /// Copies the Contents pane's selected folder's full path (#581).
    pub fn copy_selected_path(&mut self, write: impl FnOnce(&str)) {
        if let Some(path) = self.selected_content_folder() {
            write(&path.to_string_lossy());
        }
    }

    /// Copies the Folders pane's selected folder's full path (#581).
    pub fn copy_folder_path(&mut self, write: impl FnOnce(&str)) {
        write(&self.selected_dir_path().to_string_lossy());
    }

    /// Copies the Contents pane's selected folder's remote address (#581).
    pub fn copy_selected_remote_address(&mut self, write: impl FnOnce(&str)) {
        if let Some(remote) = self.selected_content_remote() {
            write(&remote);
        }
    }

    /// Copies the Folders pane's selected folder's remote address (#581).
    pub fn copy_folder_remote_address(&mut self, write: impl FnOnce(&str)) {
        if let Some(remote) = self
            .selected_folder_node()
            .and_then(|node| node.remote.clone())
        {
            write(&remote);
        }
    }

    /// Whether a command that acts on the Contents pane may run.
    ///
    /// Two things forbid it, and both were enforced only in the markup's
    /// key scope - so the keyboard respected them and the menu bar, which
    /// is clicked rather than typed, walked straight past.
    ///
    /// A prompt on screen is unfinished work, and replacing it throws away
    /// what the reader was part-way through typing. A file open in the
    /// editor is the sharper case: the editor holds the keyboard, so a
    /// question armed behind it cannot be answered at all, and the
    /// keystroke that would answer it goes into the file instead.
    ///
    /// GUIDANCE.md §2 keeps business rules out of the front ends. This is
    /// that rule one level further in: the guard belongs where every route
    /// has to pass, not on one of the ways in.
    fn pane_command_allowed(&self) -> bool {
        matches!(self.mode, Mode::Normal)
            && self.editing_file.is_none()
            && self.found.is_none()
            && self.all_repositories.is_none()
    }

    /// Whether Up has anywhere to go, so the button can be drawn refused
    /// at the top of a tree the way Back and Forward already are.
    #[must_use]
    pub fn can_go_up(&self) -> bool {
        self.selected_dir_path().parent().is_some()
    }

    /// Whether Back, Forward and Up may run, and clears the address bar's
    /// own prompt when they do.
    ///
    /// These three are the address bar's, and the only prompt that competes
    /// with them is the address bar's own: a reader who presses Ctrl+L and
    /// then clicks one of the arrows has changed their mind about typing a
    /// path, so the field goes and the click is honoured. They used to run
    /// straight through it, leaving the bar showing a stale path in an open
    /// field - and Enter there navigated back to that stale text, silently
    /// undoing the click.
    ///
    /// A prompt over a row is different. Renaming or deleting a file is not
    /// finished, and navigating away from it without a word would discard
    /// it, so these refuse instead.
    fn may_navigate(&mut self) -> bool {
        match self.mode {
            Mode::Normal => true,
            Mode::PathInput { .. } | Mode::FindInput { .. } => {
                self.mode = Mode::Normal;
                true
            }
            _ => false,
        }
    }

    /// Whether Forward has a folder to return to.
    #[must_use]
    pub fn can_go_forward(&self) -> bool {
        self.history_index + 1 < self.history.len()
    }

    /// Goes back one folder in history.
    pub fn go_back(&mut self) {
        if !self.can_go_back() || !self.may_navigate() {
            return;
        }
        self.remember_current();
        self.history_index -= 1;
        if let Some(path) = self.history.get(self.history_index).cloned() {
            self.browse(path);
        }
    }

    /// Goes forward one folder in history.
    pub fn go_forward(&mut self) {
        if !self.can_go_forward() || !self.may_navigate() {
            return;
        }
        self.history_index += 1;
        if let Some(path) = self.history.get(self.history_index).cloned() {
            self.browse(path);
        }
    }

    /// The browsed folder's path as address-bar segments, each paired with
    /// the path that segment names. A Windows path starts with a prefix and
    /// a root component (`Z:` then `\`); those are one place, so they are
    /// one segment.
    fn breadcrumb_paths(&self) -> Vec<(String, PathBuf)> {
        let path = self.selected_dir_path();
        let mut crumbs: Vec<(String, PathBuf)> = Vec::new();
        let mut so_far = PathBuf::new();
        for component in path.components() {
            so_far.push(component);
            let text = component.as_os_str().to_string_lossy().into_owned();
            match component {
                std::path::Component::RootDir => match crumbs.last_mut() {
                    // `Z:` and the separator after it name one place.
                    Some((label, target)) => {
                        label.push(std::path::MAIN_SEPARATOR);
                        target.clone_from(&so_far);
                    }
                    None => crumbs.push((text, so_far.clone())),
                },
                _ => crumbs.push((text, so_far.clone())),
            }
        }
        crumbs
    }

    /// Labels for the address bar, root first and the browsed folder last.
    #[must_use]
    pub fn breadcrumbs(&self) -> Vec<String> {
        self.breadcrumb_paths()
            .into_iter()
            .map(|(label, _)| label)
            .collect()
    }

    /// Browses the folder named by breadcrumb `index`.
    pub fn navigate_to_breadcrumb(&mut self, index: i32) {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        let crumbs = self.breadcrumb_paths();
        let Some((_, target)) = crumbs.get(index) else {
            return;
        };
        if target == &self.selected_dir_path() {
            return;
        }
        let target = target.clone();
        self.remember_current();
        self.push_history(target.clone());
        self.browse(target);
    }

    /// The folders pane's rows, one per visible tree row.
    #[must_use]
    pub fn folder_rows(&self) -> Vec<FolderRow> {
        self.root
            .flatten()
            .iter()
            .map(|(depth, indices)| {
                let node = self.root.node_at(indices);
                FolderRow {
                    icon: icon_for(node.map_or("", |n| n.name.as_str()), true),
                    name: node.map_or_else(|| "?".to_owned(), |n| n.name.clone()),
                    depth: *depth,
                    // A node whose children have never been fetched reads as
                    // a leaf, which is what the pane has always shown: the
                    // tree learns a folder has subfolders by being opened,
                    // and guessing before then would put a chevron in front
                    // of every empty folder in the listing.
                    expandable: node.is_some_and(|n| n.children.is_some()),
                    expanded: node.is_some_and(|n| n.expanded),
                    mark: node
                        .and_then(|n| repository_mark(n.is_repository, n.provider.as_deref())),
                }
            })
            .collect()
    }

    /// Handles a click on folder row `index`, `x` pixels in from the left
    /// edge of the pane.
    ///
    /// The chevron opens and closes the row without moving the selection,
    /// and the rest of the row selects it - which is how File Explorer's
    /// navigation pane behaves, and the reason the pointer position has to
    /// come this far in rather than being resolved in `app.slint`.
    pub fn click_folder(&mut self, index: usize, x: f32) {
        let row = self.folder_rows().into_iter().nth(index);
        let zoom = self.zoom_factor();
        if row.is_some_and(|row| row.expandable && chevron_hit(x, row.depth, zoom)) {
            self.toggle_folder(index);
        } else {
            self.select_folder(index);
        }
    }

    /// Index of the selected row in [`Self::folder_labels`].
    #[must_use]
    pub fn folder_selected(&self) -> usize {
        self.folder_selected
    }

    /// Display labels for the contents pane.
    #[must_use]
    pub fn content_labels(&self) -> Vec<String> {
        self.contents
            .iter()
            .map(|entry| {
                if entry.is_dir {
                    format!("{}/", entry.name)
                } else {
                    entry.name.clone()
                }
            })
            .collect()
    }

    /// Orders `all_contents` by the current sort column. Directories come
    /// first whichever column is chosen, the way Explorer groups them, and
    /// the name is the tiebreak so the order is total and stable. Sorts
    /// the unfiltered listing, not the pane's filtered view of it (#582),
    /// so the order survives a filter being typed or cleared.
    fn sort_contents(&mut self) {
        let key = self.sort_key;
        let ascending = self.sort_ascending;
        self.all_contents.sort_by(|a, b| {
            let ordering = match key {
                SortKey::Name => std::cmp::Ordering::Equal,
                SortKey::Size => a.size.cmp(&b.size),
                SortKey::Kind => {
                    format_kind(&a.name, a.is_dir).cmp(&format_kind(&b.name, b.is_dir))
                }
                SortKey::Modified => effective_modified(a).cmp(&effective_modified(b)),
            }
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name));
            let ordering = if ascending {
                ordering
            } else {
                ordering.reverse()
            };
            b.is_dir.cmp(&a.is_dir).then(ordering)
        });
    }

    /// Sorts by `column`, reversing the direction if it is already the sort
    /// column. Out-of-range columns are ignored. The selected entry keeps
    /// its selection across the reorder.
    pub fn sort_by_column(&mut self, column: i32) {
        if self.found.is_some() || self.all_repositories.is_some() {
            return;
        }
        let Some(key) = SortKey::from_index(column) else {
            return;
        };
        if self.sort_key == key {
            self.sort_ascending = !self.sort_ascending;
        } else {
            self.sort_key = key;
            self.sort_ascending = true;
        }
        // A selection is a set of files, not a set of row numbers. The
        // lead row followed its file across the reorder from the start;
        // the set behind it did not, so the highlight, and every
        // operation that reads it, kept pointing at whatever landed on
        // the old numbers. Delete after a sort named a file the reader
        // had never picked.
        let selected = self
            .contents
            .get(self.content_selected)
            .map(|entry| entry.name.clone());
        let anchored = self
            .contents
            .get(self.anchor)
            .map(|entry| entry.name.clone());
        let held: Vec<String> = self
            .selection
            .iter()
            .filter_map(|index| self.contents.get(*index))
            .map(|entry| entry.name.clone())
            .collect();

        self.sort_contents();
        self.recompute_contents();

        let row_of = |name: &str, contents: &[DirectoryEntry]| {
            contents.iter().position(|entry| entry.name == name)
        };
        self.content_selected = selected
            .as_deref()
            .and_then(|name| row_of(name, &self.contents))
            .unwrap_or(0);
        self.anchor = anchored
            .as_deref()
            .and_then(|name| row_of(name, &self.contents))
            .unwrap_or(self.content_selected);
        self.selection = held
            .iter()
            .filter_map(|name| row_of(name, &self.contents))
            .collect();
    }

    /// The column currently sorted on, as a UI column index.
    #[must_use]
    pub const fn sort_column(&self) -> i32 {
        self.sort_key.index()
    }

    /// Whether the current sort is ascending.
    #[must_use]
    pub const fn sort_ascending(&self) -> bool {
        self.sort_ascending
    }

    /// The contents pane's rows, one per entry, with a cell per column.
    #[must_use]
    pub fn content_rows(&self) -> Vec<ContentRow> {
        if let Some(found) = &self.found {
            return found.matches.iter().map(found_row).collect();
        }
        if let Some(view) = &self.all_repositories {
            let now = now_epoch_seconds();
            return view
                .entries
                .iter()
                .map(|entry| self.all_repository_row(now, entry))
                .collect();
        }
        let now = now_epoch_seconds();
        self.contents
            .iter()
            .map(|entry| {
                let marker = if entry.repository.is_some() {
                    self.marker_for(&entry.name)
                } else {
                    ""
                };
                let stale = entry.repository.as_ref().is_some_and(|repository| {
                    fetch_is_stale(repository.last_fetch, repository.remote.is_some(), now)
                });
                let stale_tooltip = if stale {
                    stale_fetch_tooltip(entry.repository.as_ref().and_then(|r| r.last_fetch), now)
                } else {
                    String::new()
                };
                ContentRow {
                    icon: icon_for(&entry.name, entry.is_dir),
                    is_dir: entry.is_dir,
                    name: if entry.is_dir {
                        format!("{}/", entry.name)
                    } else {
                        entry.name.clone()
                    },
                    size: if entry.is_dir {
                        String::new()
                    } else {
                        format_size(entry.size)
                    },
                    kind: format_kind_of(&entry.name, entry.is_dir, entry.repository.as_ref()),
                    modified: format_timestamp(effective_modified(entry)),
                    is_repository: entry.repository.is_some(),
                    mark: repository_mark(
                        entry.repository.is_some(),
                        entry
                            .repository
                            .as_ref()
                            .and_then(|r| r.provider.as_deref()),
                    ),
                    branch: entry
                        .repository
                        .as_ref()
                        .map_or_else(String::new, |repository| {
                            repository
                                .branch
                                .clone()
                                .unwrap_or_else(|| "detached".to_owned())
                        }),
                    marker: marker.to_owned(),
                    marker_tooltip: marker_tooltip(marker).to_owned(),
                    marker_warning: marker == CHANGED_MARKER,
                    stale_marker: if stale {
                        STALE_FETCH_MARKER.to_owned()
                    } else {
                        String::new()
                    },
                    stale_tooltip,
                }
            })
            .collect()
    }

    /// Ctrl+Shift+F: opens the address bar as a prompt for a name to find
    /// across every repository.
    pub fn begin_find(&mut self) {
        if !matches!(self.mode, Mode::Normal) || self.editing_file.is_some() {
            return;
        }
        let input = self
            .found
            .as_ref()
            .map(|found| found.query.clone())
            .unwrap_or_default();
        self.mode = Mode::FindInput { input };
    }

    /// Ctrl+F, or a click on the Contents pane's filter field: gives it the
    /// keyboard, so a typed character narrows the listing instead of
    /// jumping to a name (#582). Refused while another prompt or the
    /// editor already has the keyboard, and while a cross-repository
    /// search is showing - its results are not this folder's listing to
    /// filter.
    pub fn begin_filter(&mut self) {
        if !matches!(self.mode, Mode::Normal) || self.editing_file.is_some() || self.found.is_some()
        {
            return;
        }
        self.filter.focused = true;
        self.focus = Pane::Contents;
    }

    /// Ctrl+P / Cmd+P: opens the Go to Repository switcher (#590). Refused
    /// while another prompt, the editor or a cross-repository search
    /// already has the keyboard, the same guard [`Self::begin_filter`]
    /// uses.
    pub fn begin_switcher(&mut self) {
        if !matches!(self.mode, Mode::Normal) || self.editing_file.is_some() || self.found.is_some()
        {
            return;
        }
        self.switcher_selected = 0;
        self.mode = Mode::Switcher {
            query: String::new(),
        };
    }

    /// The Repos Directory's direct children that are working copies -
    /// nested ones wait on #591 - matched against `query` and ranked by
    /// [`switcher::score`], word-start and contiguous runs first. An empty
    /// query keeps every repository, in name order.
    fn switcher_matches(&self, query: &str) -> Vec<&FolderNode> {
        let repositories = self
            .root
            .children
            .iter()
            .flatten()
            .filter(|node| node.is_repository);
        if query.is_empty() {
            // The Contents pane's order, as far as a repository has the
            // fields for it: they are all folders, so they have no size and
            // one kind between them, and the tree the switcher reads knows
            // no modified time. What is left is the name - and the
            // direction, which the reader did choose and which the pane is
            // showing right now.
            let mut repositories: Vec<&FolderNode> = repositories.collect();
            repositories.sort_by(|a, b| {
                let ordering = a.name.to_lowercase().cmp(&b.name.to_lowercase());
                if self.sort_ascending {
                    ordering
                } else {
                    ordering.reverse()
                }
            });
            return repositories;
        }
        let mut scored: Vec<(i32, &FolderNode)> = repositories
            .filter_map(|node| switcher::score(query, &node.name).map(|score| (score, node)))
            .collect();
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()))
        });
        scored.into_iter().map(|(_, node)| node).collect()
    }

    /// Whether the Go to Repository switcher is open.
    #[must_use]
    pub const fn switcher_open(&self) -> bool {
        matches!(self.mode, Mode::Switcher { .. })
    }

    /// The switcher's typed query, or an empty string while it is closed.
    #[must_use]
    pub fn switcher_query(&self) -> String {
        match &self.mode {
            Mode::Switcher { query } => query.clone(),
            _ => String::new(),
        }
    }

    /// The switcher's matches for its current query, for the overlay to
    /// draw: each result's name, branch and change marker, the way
    /// Contents shows them. The change marker is only ever real when the
    /// Repos Directory itself is the folder on screen - the listing
    /// [`Self::row_statuses`] answers for - and is the "not known yet"
    /// glyph otherwise, same as a Contents row not yet asked about.
    #[must_use]
    pub fn switcher_rows(&self) -> Vec<SwitcherRow> {
        let query = self.switcher_query();
        let at_root = self.selected_dir_path() == self.root.path;
        self.switcher_matches(&query)
            .into_iter()
            .enumerate()
            .map(|(index, node)| {
                let marker = if at_root {
                    self.marker_for(&node.name)
                } else {
                    NOT_KNOWN_YET_MARKER
                };
                SwitcherRow {
                    name: node.name.clone(),
                    // Every result is a direct child of the Repos Directory
                    // until #591 lands, so it has no path of its own to show
                    // beside its name.
                    path: String::new(),
                    branch: node.branch.clone().unwrap_or_else(|| "detached".to_owned()),
                    marker: marker.to_owned(),
                    marker_tooltip: marker_tooltip(marker).to_owned(),
                    marker_warning: marker == CHANGED_MARKER,
                    selected: index == self.switcher_selected,
                }
            })
            .collect()
    }

    /// Appends one typed character to the switcher's query (#590),
    /// resetting the highlight to the new query's first match.
    fn type_into_switcher(&mut self, text: &str) {
        let Some(c) = text.chars().next() else {
            return;
        };
        if !typeable(c) {
            return;
        }
        if let Mode::Switcher { query } = &mut self.mode {
            query.push(c);
        }
        self.switcher_selected = 0;
    }

    /// Removes the last character of the switcher's query (#590).
    fn switcher_backspace(&mut self) {
        if let Mode::Switcher { query } = &mut self.mode {
            query.pop();
        }
        self.switcher_selected = 0;
    }

    /// Return in the switcher: goes to the highlighted match, exactly as
    /// clicking it would.
    fn confirm_switcher(&mut self) {
        self.activate_switcher_result(self.switcher_selected);
    }

    /// Clicking a switcher result, or Return while it is highlighted:
    /// selects that repository in Contents, and its parent - the Repos
    /// Directory itself - in Folders, the same as clicking the repository
    /// there would. Every result is already a direct child of the root
    /// (#591 is what would make that not so), so this reselects the root
    /// row rather than re-rooting the whole tree the way Find's jump to an
    /// arbitrary folder has to.
    pub fn activate_switcher_result(&mut self, index: usize) {
        let Mode::Switcher { query } = std::mem::replace(&mut self.mode, Mode::Normal) else {
            return;
        };
        if let Some(node) = self.switcher_matches(&query).get(index) {
            self.reselect = Some(node.name.clone());
            self.select_folder(0);
        }
    }

    /// Clicking "N with uncommitted changes" in the status bar (#582):
    /// narrows the Contents pane to just those rows.
    pub fn filter_to_changed(&mut self) {
        if !matches!(self.mode, Mode::Normal) || self.found.is_some() {
            return;
        }
        self.filter.changed_only = true;
        self.focus = Pane::Contents;
        self.recompute_contents();
        self.reset_selection_after_filter();
    }

    /// "clear", or Escape in the Contents pane: drops every active filter
    /// (#582) and restores the listing the service last sent.
    pub fn clear_filters(&mut self) {
        self.filter = Filter::default();
        self.recompute_contents();
        self.reset_selection_after_filter();
    }

    /// Whether a filter (#582) is narrowing the Contents pane, or has the
    /// keyboard to type one.
    fn filtering(&self) -> bool {
        self.filter.focused || !self.filter.text.is_empty() || self.filter.changed_only
    }

    /// The Contents pane filter field's current text (#582).
    #[must_use]
    pub fn filter_text(&self) -> String {
        self.filter.text.clone()
    }

    /// Whether the Contents pane filter field has the keyboard (#582).
    #[must_use]
    pub const fn filter_focused(&self) -> bool {
        self.filter.focused
    }

    /// "3 with uncommitted changes" - the status bar's clickable link
    /// (#582), shown whenever [`Self::repositories_summary`] would have a
    /// repository to count, even when none of them has changed: 0 is still
    /// worth clicking through to confirm. Empty while a transient message
    /// is showing, while the changed-only filter it activates has already
    /// narrowed the pane, or while the listing holds no repository.
    #[must_use]
    pub fn status_changed_label(&self) -> String {
        let has_repository = self.contents.iter().any(|entry| entry.repository.is_some());
        if !matches!(self.mode, Mode::Normal)
            || self.status.is_some()
            || self.filter.changed_only
            || !has_repository
        {
            return String::new();
        }
        format!("{} with uncommitted changes", self.changed_marker_count())
    }

    /// Whether the status bar's "clear" link is showing: only while the
    /// changed-only filter (#582) is narrowing the Contents pane.
    #[must_use]
    pub fn status_show_clear_link(&self) -> bool {
        matches!(self.mode, Mode::Normal) && self.status.is_none() && self.filter.changed_only
    }

    /// The Contents pane's centred message (#592) - what is wrong with
    /// the Repos Directory itself, that it has nothing in it yet, or
    /// that a filter (#582) has narrowed a real listing to nothing -
    /// `None` while the pane is just drawing its listing.
    fn contents_message(&self) -> Option<ContentsMessage> {
        if !matches!(self.mode, Mode::Normal)
            || self.found.is_some()
            || self.all_repositories.is_some()
            || !self.contents.is_empty()
        {
            return None;
        }
        if !self.all_contents.is_empty() {
            let title = if self.filter.text.is_empty() {
                "No repositories with uncommitted changes".to_owned()
            } else {
                format!("No name matches \"{}\"", self.filter.text)
            };
            return Some(ContentsMessage::FilterEmpty { title });
        }
        if self.selected_dir_path() != self.root.path {
            return None;
        }
        let path = self.root.path.display();
        Some(match &self.root_problem {
            Some(RootProblem::NotThere { cause }) => ContentsMessage::NotThere {
                title: format!("The Repos Directory {path} is not available"),
                cause: cause.describe(),
            },
            Some(RootProblem::NotReadable { message }) => ContentsMessage::NotReadable {
                title: format!("Repos Explorer cannot read {path}"),
                detail: message.clone(),
            },
            None => ContentsMessage::Empty {
                title: format!("{path} has no repositories yet"),
            },
        })
    }

    /// The buttons [`Self::contents_message`] offers, in the order Tab
    /// reaches them. Empty while no message is showing.
    fn message_buttons(&self) -> Vec<MessageButton> {
        match self.contents_message() {
            Some(ContentsMessage::NotThere { .. } | ContentsMessage::NotReadable { .. }) => {
                vec![MessageButton::Retry, MessageButton::Choose]
            }
            Some(ContentsMessage::Empty { .. }) => vec![MessageButton::Choose],
            Some(ContentsMessage::FilterEmpty { .. }) => vec![MessageButton::ClearFilter],
            None => Vec::new(),
        }
    }

    /// The Contents pane message's title, or empty while none is
    /// showing.
    #[must_use]
    pub fn contents_message_title(&self) -> String {
        match self.contents_message() {
            Some(
                ContentsMessage::NotThere { title, .. }
                | ContentsMessage::NotReadable { title, .. }
                | ContentsMessage::Empty { title }
                | ContentsMessage::FilterEmpty { title },
            ) => title,
            None => String::new(),
        }
    }

    /// The second line under the title: the likely cause, the read
    /// error's own message, what an empty Repos Directory will show once
    /// something is cloned into it, or empty while no message is
    /// showing.
    #[must_use]
    pub fn contents_message_detail(&self) -> String {
        match self.contents_message() {
            Some(ContentsMessage::NotThere { cause, .. }) => cause,
            Some(ContentsMessage::NotReadable { detail, .. }) => detail,
            Some(ContentsMessage::Empty { .. }) => {
                "Working copies cloned into it will appear here.".to_owned()
            }
            Some(ContentsMessage::FilterEmpty { .. }) | None => String::new(),
        }
    }

    /// Whether the message offers Retry - the "not there" and "not
    /// readable" cases only.
    #[must_use]
    pub fn contents_message_show_retry(&self) -> bool {
        self.message_buttons().contains(&MessageButton::Retry)
    }

    /// Whether the message offers "Choose Repos Directory..." - every
    /// case but the filter narrowing one, which has its own way back.
    #[must_use]
    pub fn contents_message_show_choose(&self) -> bool {
        self.message_buttons().contains(&MessageButton::Choose)
    }

    /// Whether the message offers a way to clear the filter (#582) that
    /// narrowed the listing to nothing.
    #[must_use]
    pub fn contents_message_show_clear_filter(&self) -> bool {
        self.message_buttons().contains(&MessageButton::ClearFilter)
    }

    /// Which button Tab has highlighted, as an index into
    /// [`Self::message_buttons`] in the order they are drawn - `-1` while
    /// no message is showing.
    #[must_use]
    pub fn contents_message_focus(&self) -> i32 {
        let count = self.message_buttons().len();
        if count == 0 {
            -1
        } else {
            i32::try_from(self.message_focus % count).unwrap_or(0)
        }
    }

    /// Tab (`delta` 1) or Shift+Tab (`delta` -1) while the Contents
    /// pane's message (#592) is showing: moves the highlighted button
    /// among whichever ones the current case offers.
    pub fn move_message_focus(&mut self, delta: i32) {
        let count = self.message_buttons().len();
        if count == 0 {
            return;
        }
        let len = i32::try_from(count).unwrap_or(1);
        let current = i32::try_from(self.message_focus % count).unwrap_or(0);
        self.message_focus = usize::try_from((current + delta).rem_euclid(len)).unwrap_or(0);
    }

    /// Return while the Contents pane's message (#592) is showing:
    /// activates whichever button Tab last highlighted, the first one
    /// when nothing has moved it yet.
    fn activate_focused_message_button(&mut self) {
        let buttons = self.message_buttons();
        let Some(button) = buttons.get(self.message_focus).or_else(|| buttons.first()) else {
            return;
        };
        match button {
            MessageButton::Retry => self.refresh(),
            MessageButton::Choose => {
                let current = self.root.path.to_string_lossy().into_owned();
                self.begin_repos_root_edit(&current);
            }
            MessageButton::ClearFilter => self.clear_filters(),
        }
    }

    /// Whether the Contents pane is showing a search's results rather
    /// than the browsed folder, so its column headings can say so.
    #[must_use]
    pub const fn showing_found(&self) -> bool {
        self.found.is_some()
    }

    /// Whether the Contents pane is showing the All Repositories view
    /// (#591) rather than the browsed folder, so its column headings can
    /// say so.
    #[must_use]
    pub const fn showing_all_repositories(&self) -> bool {
        self.all_repositories.is_some()
    }

    /// Whether the Contents pane should draw its Size column: a search's
    /// results keep their own columns regardless (#578), and an ordinary
    /// listing draws it only once it holds a file - every row is a folder
    /// in the common case of browsing a Repos Directory, where an empty,
    /// fixed-width Size column only crowds out the Name and Type columns
    /// for a fact no row has. An All Repositories row is always a folder,
    /// so it never earns the column either.
    #[must_use]
    pub fn content_size_column_visible(&self) -> bool {
        self.found.is_some()
            || (self.all_repositories.is_none() && self.contents.iter().any(|entry| !entry.is_dir))
    }

    /// Whether the Contents pane's Modified column should read "Last
    /// activity" instead: once the listing holds any repository row, since
    /// that column shows last activity rather than the folder's own time
    /// for that row (#588). A search's results, and the All Repositories
    /// view (#591), draw their own Type-column replacement instead and are
    /// never this.
    #[must_use]
    pub fn content_holds_repository(&self) -> bool {
        self.found.is_none()
            && self.all_repositories.is_none()
            && self.contents.iter().any(|entry| entry.repository.is_some())
    }

    /// Plants a search's answer, for a test that has one without a service.
    pub fn apply_find_result_for_test(&mut self, query: &str, response: Response) {
        self.apply_find_result(query, Ok(response));
    }

    fn apply_find_result(&mut self, query: &str, result: io::Result<Response>) {
        match result {
            Ok(Response::Names {
                root,
                matches,
                cut_short,
            }) => {
                let previous_selected = self
                    .found
                    .as_ref()
                    .map_or(self.content_selected, |found| found.previous_selected);
                self.status = Some(if matches.is_empty() {
                    format!("nothing named like \"{query}\" in any repository")
                } else if cut_short {
                    format!(
                        "the first {} names like \"{query}\" - there are more; Esc for the folder",
                        matches.len()
                    )
                } else {
                    format!(
                        "{} named like \"{query}\"; Enter opens one, Esc for the folder",
                        matches.len()
                    )
                });
                self.found = Some(Found {
                    query: query.to_owned(),
                    root: PathBuf::from(root),
                    matches,
                    previous_selected,
                });
                self.content_selected = 0;
                self.anchor = 0;
                self.selection.clear();
                self.selection.insert(0);
                self.focus = Pane::Contents;
                self.load_file_view();
            }
            Ok(Response::Error { message }) => self.status = Some(message),
            Ok(_) => self.status = Some("unexpected response to a search".to_owned()),
            Err(err) => self.status = Some(err.to_string()),
        }
    }

    /// Goes to the folder holding result `index`, with it selected.
    fn open_found(&mut self, index: usize) {
        let Some(found) = &self.found else {
            return;
        };
        let Some(found_match) = found.matches.get(index) else {
            return;
        };
        let path = found.root.join(&found_match.path);
        let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
            return;
        };
        let (folder, name) = (folder.to_path_buf(), name.to_string_lossy().into_owned());
        self.found = None;
        self.remember_current();
        self.push_history(folder.clone());
        self.reselect = Some(name);
        self.browse(folder);
    }

    /// View > All Repositories, and the Folders tree's own entry for it
    /// (#591): every working copy up to three folder levels below the
    /// Repos Directory, found by the service in the background. Replaces
    /// the Contents pane's listing in place, the way a search's results
    /// do, rather than navigating anywhere - the browsed folder is still
    /// there for Escape to put back.
    pub fn open_all_repositories(&mut self) {
        if !matches!(self.mode, Mode::Normal) || self.editing_file.is_some() {
            return;
        }
        self.found = None;
        let root = self.root.path.clone();
        self.all_repositories = Some(AllRepositoriesView {
            root: root.clone(),
            entries: Vec::new(),
            done: false,
            previous_selected: self.content_selected,
        });
        self.content_selected = 0;
        self.anchor = 0;
        self.selection.clear();
        self.selection.insert(0);
        self.focus = Pane::Contents;
        self.status = Some("Looking for repositories…".to_owned());
        self.pending_all_repositories = Some(spawn_request(Request::AllRepositories {
            root: root.to_string_lossy().into_owned(),
            refresh: false,
        }));
    }

    /// Plants an All Repositories answer, for a test that has one without a
    /// service.
    pub fn apply_all_repositories_result_for_test(&mut self, response: Response) {
        self.apply_all_repositories_result(Ok(response));
    }

    fn apply_all_repositories_result(&mut self, result: io::Result<Response>) {
        let Some(view) = &mut self.all_repositories else {
            return;
        };
        match result {
            Ok(Response::AllRepositories { entries, done }) => {
                view.entries = entries;
                view.done = done;
                self.status = if done {
                    None
                } else {
                    let root = view.root.clone();
                    let found_so_far = view.entries.len();
                    self.pending_all_repositories = Some(spawn_request(Request::AllRepositories {
                        root: root.to_string_lossy().into_owned(),
                        refresh: false,
                    }));
                    Some(format!("Looking for repositories… {found_so_far} found"))
                };
            }
            Ok(Response::Error { message }) => self.status = Some(message),
            Ok(_) => self.status = Some("unexpected response to All Repositories".to_owned()),
            Err(err) => self.status = Some(err.to_string()),
        }
    }

    /// Return or double-click on an All Repositories row (#591): goes to
    /// that repository in its real folder, the same way [`Self::open_found`]
    /// goes to a search result's.
    fn open_all_repositories_entry(&mut self, index: usize) {
        let Some(view) = &self.all_repositories else {
            return;
        };
        let Some(entry) = view.entries.get(index) else {
            return;
        };
        let path = all_repository_path(&view.root, entry);
        let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
            return;
        };
        let (folder, name) = (folder.to_path_buf(), name.to_string_lossy().into_owned());
        self.all_repositories = None;
        self.pending_all_repositories = None;
        self.remember_current();
        self.push_history(folder.clone());
        self.reselect = Some(name);
        self.browse(folder);
    }

    fn listed_len(&self) -> usize {
        if let Some(found) = &self.found {
            return found.matches.len();
        }
        if let Some(view) = &self.all_repositories {
            return view.entries.len();
        }
        self.contents.len()
    }

    /// Index of the selected row in [`Self::content_labels`].
    #[must_use]
    pub fn content_selected(&self) -> usize {
        self.content_selected
    }

    /// The picture the previewed file's plugin offers, if its type is one.
    /// The pane draws this above the text, so an image reads as the image
    /// rather than as three lines describing it.
    #[must_use]
    pub fn file_graphic(&self) -> Option<Graphic> {
        // The picture belongs to the Preview: it is what the plugin
        // renders. The Text tab is the file read plainly, and a fixed
        // band of rendered drawing above it is the Preview intruding on
        // the one view that exists to get away from it. An SVG is a
        // picture and text at once, so it has both tabs and this is
        // reachable - read the file's markup and the drawing of it was
        // still there, taking a fifth of the pane, with no way to dismiss
        // it.
        if self.file_views().get(self.file_view_index) != Some(&PREVIEW_VIEW) {
            return None;
        }
        match &self.file_view {
            Some(Response::FileView { plugin, data, .. }) => present_graphic(plugin, data),
            _ => None,
        }
    }

    /// The views the previewed file's type offers, in the plugin's order.
    /// Empty when nothing is previewed, or when the preview is an error
    /// rather than a file - there is nothing there to look at two ways.
    #[must_use]
    pub fn file_views(&self) -> Vec<&'static str> {
        match &self.file_view {
            Some(Response::FileView { plugin, data, .. }) => PRESENTATION_PLUGINS
                .iter()
                .find(|candidate| candidate.name() == plugin)
                .map(|candidate| candidate.views(data))
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// The Text view's lines, each split into coloured runs, or empty when
    /// there is nothing to colour.
    ///
    /// Empty covers three cases and the pane treats them alike, falling
    /// back to the plain text it always drew: the pane is showing the
    /// plugin's Preview rather than the file, the plugin describes no
    /// language, or the file carries no text. None of them is a failure,
    /// and a reader should not be able to tell them apart.
    ///
    /// Lines rather than one list of runs because a line is what gets
    /// laid out: runs sit side by side across a line and lines stack, and
    /// flattening that would leave the pane unable to place either.
    #[must_use]
    pub fn file_lines(&self) -> Vec<Vec<ColouredRun>> {
        let Some(Response::FileView { plugin, data, .. }) = &self.file_view else {
            return Vec::new();
        };
        let showing = self.file_views().get(self.file_view_index).copied();
        if showing != Some(TEXT_VIEW) && showing != Some(PREVIEW_VIEW) {
            return Vec::new();
        }
        let Some(text) = data.get("content").and_then(serde_json::Value::as_str) else {
            return Vec::new();
        };
        let Some(presentation) = PRESENTATION_PLUGINS
            .iter()
            .find(|candidate| candidate.name() == plugin)
        else {
            return Vec::new();
        };
        let spans = presentation.classify(text);
        if spans.is_empty() {
            return Vec::new();
        }
        let coloured = colour_lines(text, &spans);
        if showing == Some(TEXT_VIEW) {
            return coloured;
        }

        // The Preview is the plugin's summary and then, for a source
        // language, the file itself - the same bytes the Text tab
        // colours. Leaving one plain and the other coloured reads as the
        // colouring being broken rather than as two views doing
        // different jobs.
        let preview = present(plugin, data);
        let Some(from) = file_starts_in_preview(&preview, text) else {
            // A Preview that is all summary - `msbuild` prints no file -
            // is left exactly as it was.
            return Vec::new();
        };
        let mut lines: Vec<Vec<ColouredRun>> = preview[..from]
            .iter()
            .map(|line| colour_summary_line(line).unwrap_or_else(|| plain_line(line)))
            .collect();
        lines.extend(coloured);
        lines
    }

    /// The tab strip above the File pane: the plugin's views, and then
    /// the pane's own way in to the editor.
    ///
    /// Editing is not a plugin's idea of the file - it is something the
    /// application offers - so it is appended here rather than added to
    /// `PluginPresentation::views`, which would make every plugin
    /// responsible for a thing none of them does.
    ///
    /// While the editor is open the plugin's views are not offered at
    /// all. They would be one stray click away from discarding what
    /// somebody has typed, and the way out is Save or Escape, which say
    /// what they did.
    #[must_use]
    pub fn file_tabs(&self) -> Vec<String> {
        if self.editing_file() {
            return vec![EDITING_TAB.to_owned()];
        }
        let mut tabs: Vec<String> = self.file_views().into_iter().map(str::to_owned).collect();
        if self.can_edit() {
            tabs.push(EDIT_TAB.to_owned());
        }
        tabs
    }

    /// Which tab is active, as an index into [`Self::file_tabs`].
    #[must_use]
    pub fn file_tab_index(&self) -> usize {
        if self.editing_file() {
            0
        } else {
            self.file_view_index
        }
    }

    /// Chooses the tab at `index`: a view, or the editor.
    pub fn select_file_tab(&mut self, index: usize) {
        if self.editing_file() {
            // Only the one tab while editing, and it is already active.
            return;
        }
        let views = self.file_views().len();
        if index < views {
            self.select_file_view(index);
        } else if index == views {
            self.begin_file_edit();
        }
    }

    /// Which view the pane is showing, as an index into [`Self::file_views`].
    #[must_use]
    pub const fn file_view_index(&self) -> usize {
        self.file_view_index
    }

    /// Shows the view at `index`, ignoring an index the type does not offer:
    /// a switcher click can only arrive for a view that was on screen, so a
    /// stale one is worth ignoring rather than reporting.
    pub fn select_file_view(&mut self, index: usize) {
        if index < self.file_views().len() {
            self.file_view_index = index;
        }
    }

    /// The File pane's fact table for the selected file: the label/value
    /// pairs [`Self::file_text`] draws as sentences instead, when the
    /// plugin has nothing tabular to offer.
    #[must_use]
    pub fn file_facts(&self) -> Vec<FactRow> {
        let Some(Response::FileView { plugin, data, .. }) = &self.file_view else {
            return Vec::new();
        };
        facts(plugin, data)
            .into_iter()
            .map(|fact| FactRow {
                label: fact.label,
                display_value: middle_elide(&fact.value, FACT_VALUE_BUDGET),
                full_value: fact.value,
                dim: fact.dim,
            })
            .collect()
    }

    /// Display text for the file pane, in whichever view is selected.
    #[must_use]
    pub fn file_text(&self) -> String {
        match &self.file_view {
            Some(Response::FileView { plugin, data, also }) => {
                // A plugin offering a fact table (#576) draws it there
                // instead of these lines - drawing both would say the same
                // thing twice. What is never covered by the table, a
                // stacked folder plugin's own lines (D12), still belongs
                // here.
                let mut lines = if facts(plugin, data).is_empty() {
                    let views = self.file_views();
                    match views.get(self.file_view_index) {
                        Some(view) => present_view(plugin, view, data),
                        None => present(plugin, data),
                    }
                } else {
                    Vec::new()
                };
                // A folder is several things at once, and each folder
                // plugin that recognises it adds its lines below the
                // folder's own rather than in place of them.
                for extra in also {
                    lines.push(String::new());
                    lines.extend(present_folder(&extra.plugin, &extra.data));
                }
                lines.join("\n")
            }
            Some(Response::Error { message }) => message.clone(),
            Some(
                Response::Directory { .. }
                | Response::Done
                | Response::ReposRoots { .. }
                | Response::Names { .. }
                | Response::WorkingTree { .. }
                | Response::AllRepositories { .. },
            )
            | None => String::new(),
        }
    }

    /// The selected working copy's README (#584), read straight from the
    /// `directory` plugin's own view data rather than through [`facts`] or
    /// [`present`]: the fact table only ever holds label/value pairs, and
    /// `directory` always has at least the entry-count fact, so its own
    /// `present` lines - which do carry the README - never reach the pane
    /// (`facts(plugin, data).is_empty()` is never true for it).
    fn readme_excerpt(&self) -> Option<plugin_directory::readme::ReadmeExcerpt> {
        let Some(Response::FileView { plugin, data, .. }) = &self.file_view else {
            return None;
        };
        if plugin != "directory" {
            return None;
        }
        serde_json::from_value::<plugin_directory::DirectoryView>(data.clone())
            .ok()
            .and_then(|view| view.readme)
    }

    /// The README's file name, for the File pane's Open README link - empty
    /// when the selected folder has none, which both hides the section and
    /// gives [`Self::open_readme`] nothing to act on.
    #[must_use]
    pub fn file_readme_name(&self) -> String {
        self.readme_excerpt()
            .map(|readme| readme.name)
            .unwrap_or_default()
    }

    /// The README section's own title: its first heading, or the literal
    /// word "README" when it has none (or could not be read - #584
    /// requirement 5). Empty exactly when [`Self::file_readme_name`] is,
    /// which is what hides the section.
    #[must_use]
    pub fn file_readme_title(&self) -> String {
        self.readme_excerpt()
            .map(|readme| readme.title.unwrap_or_else(|| "README".to_owned()))
            .unwrap_or_default()
    }

    /// The README's opening excerpt, its paragraphs joined by blank lines
    /// so the pane's own word-wrap reads it as prose - empty when there is
    /// none to show (a bare `README`, or one past the read cap).
    #[must_use]
    pub fn file_readme_excerpt(&self) -> String {
        self.readme_excerpt()
            .map(|readme| readme.excerpt.join("\n\n"))
            .unwrap_or_default()
    }

    /// Opens the selected working copy and selects its README once its
    /// listing arrives, so the README's own plugin shows it in full - the
    /// File pane's Open README link (#584). The README lives a level below
    /// what the File pane is previewing: the folder the reader selected,
    /// not the folder currently on screen in Contents.
    pub fn open_readme(&mut self) {
        let name = self.file_readme_name();
        if name.is_empty() || self.found.is_some() {
            return;
        }
        self.reselect = Some(name);
        self.open_content(self.content_selected);
    }

    /// What the File pane's "Worktree of"/"Submodule of" line (#587) should
    /// say, and whether it is a link - read the same way [`Self::readme_excerpt`]
    /// reads the README, straight from the `directory` plugin's own view
    /// data. `None` for an ordinary clone, which has no such thing to say.
    fn related_repository(&self) -> Option<RelatedRepository> {
        let Some(Response::FileView { plugin, data, .. }) = &self.file_view else {
            return None;
        };
        if plugin != "directory" {
            return None;
        }
        let view = serde_json::from_value::<plugin_directory::DirectoryView>(data.clone()).ok()?;
        let repository = view.repository?;
        match repository.kind {
            plugin_directory::repository::Kind::Clone => None,
            plugin_directory::repository::Kind::Worktree {
                clone,
                clone_exists,
            } => Some(if clone_exists {
                RelatedRepository::Link {
                    label: format!("Worktree of {}", self.related_repository_name(&clone)),
                    path: clone,
                }
            } else {
                RelatedRepository::Gone {
                    label: format!(
                        "Worktree of a clone that is no longer at {}",
                        clone.display()
                    ),
                }
            }),
            plugin_directory::repository::Kind::Submodule { outer } => {
                Some(RelatedRepository::Link {
                    label: format!("Submodule of {}", self.related_repository_name(&outer)),
                    path: outer,
                })
            }
        }
    }

    /// `path`'s folder name, with where it is in parentheses when that says
    /// more than the name alone does: its path relative to the Repos
    /// Directory when it is inside `self.root`, the full path otherwise -
    /// left off when `path` is a direct child of the Repos Directory, where
    /// the name already says where it is (#587).
    fn related_repository_name(&self, path: &Path) -> String {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let located = path.strip_prefix(&self.root.path).map_or_else(
            |_| path.display().to_string(),
            |relative| relative.to_string_lossy().replace('\\', "/"),
        );
        if located == name {
            name
        } else {
            format!("{name} (at {located})")
        }
    }

    /// The File pane's "Worktree of"/"Submodule of" line (#587), or empty
    /// when the selected folder is not one - which hides the line.
    #[must_use]
    pub fn file_related_repository_label(&self) -> String {
        match self.related_repository() {
            Some(RelatedRepository::Link { label, .. } | RelatedRepository::Gone { label }) => {
                label
            }
            None => String::new(),
        }
    }

    /// Whether [`Self::file_related_repository_label`] is a link: true for
    /// a worktree whose clone is still there, or a submodule, and false
    /// for a worktree whose clone is gone or an ordinary clone with
    /// nothing to say.
    #[must_use]
    pub fn file_related_repository_linked(&self) -> bool {
        matches!(
            self.related_repository(),
            Some(RelatedRepository::Link { .. })
        )
    }

    /// Follows the File pane's "Worktree of"/"Submodule of" link (#587):
    /// goes to the folder holding the clone or outer working copy, with it
    /// selected - matching [`Self::open_found`]'s own shape, since the
    /// target is not guaranteed to be inside the folder on screen.
    pub fn open_related_repository(&mut self) {
        let Some(RelatedRepository::Link { path, .. }) = self.related_repository() else {
            return;
        };
        let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
            return;
        };
        let (folder, name) = (folder.to_path_buf(), name.to_string_lossy().into_owned());
        self.remember_current();
        self.push_history(folder.clone());
        self.reselect = Some(name);
        self.browse(folder);
    }

    /// Display text for the status bar.
    #[must_use]
    pub fn status_text(&self) -> String {
        match &self.mode {
            Mode::ConfirmDelete { name, .. } => format!("Delete {name}? y/n"),
            Mode::RenameInput { input, .. } => format!("Rename to: {input}_  (Enter/Esc)"),
            Mode::CopyInput { input, .. } => format!("Copy to: {input}_  (Enter/Esc)"),
            Mode::ExtractInput { input, .. } => format!("Extract to: {input}_  (Enter/Esc)"),
            Mode::PathInput { input } => format!("Go to: {input}_  (Enter/Esc)"),
            Mode::FindInput { input } => {
                format!("Find in every repository: {input}_  (Enter/Esc)")
            }
            Mode::ReposRootInput { input } => {
                format!(
                    "Repos Directory: {input}_  (Enter to open there from now on, Esc to cancel)"
                )
            }
            Mode::Switcher { query } => format!("Go to repository: {query}_  (Enter/Esc)"),
            Mode::Normal => self
                .status
                .clone()
                .unwrap_or_else(|| self.contents_summary()),
        }
    }

    /// The prompt a pending operation is waiting on, or an empty string
    /// when there is none. The status bar carries this too, but a line at
    /// the very bottom of the window is easy to miss entirely: every menu
    /// action except Open answers only there, so choosing one looks like
    /// nothing happened. The contents pane shows this over the row the
    /// operation applies to, where the user is already looking.
    #[must_use]
    pub fn prompt_text(&self) -> String {
        match &self.mode {
            Mode::ConfirmDelete { name, .. } => format!("Delete {name}?  (y / n)"),
            Mode::RenameInput { input, .. } => format!("Rename to:  {input}"),
            Mode::CopyInput { input, .. } => format!("Copy to:  {input}"),
            Mode::ExtractInput { input, .. } => format!("Extract into:  {input}"),
            // A first run has no rows to point at, and an empty pane with a
            // status line nobody reads is how somebody decides the
            // application is broken. So this prompt is shown in the pane.
            Mode::ReposRootInput { input } => {
                format!("Where are your repositories?  {input}")
            }
            // The address bar shows its own text field; the contents pane
            // has nothing to say about a path being typed. The switcher
            // (#590) draws its own overlay, over every pane, so it has
            // nothing to say about one row either.
            Mode::PathInput { .. }
            | Mode::FindInput { .. }
            | Mode::Normal
            | Mode::Switcher { .. } => String::new(),
        }
    }

    /// The contents row [`Self::prompt_text`] applies to, or `-1` when no
    /// prompt is pending.
    #[must_use]
    pub fn prompt_row(&self) -> i32 {
        // The row comes from the path the prompt will act on, not from
        // wherever the selection happens to be now. Reading the live
        // selection meant a click on another row carried the prompt onto
        // it while the text, and the file it would act on, stayed behind:
        // "Delete target.txt?" drawn over keep.txt, and `y` taking
        // target.txt. The words were right and the position was wrong,
        // and the position is what the eye reads.
        let path = match &self.mode {
            Mode::ConfirmDelete { paths, .. } => match paths.first() {
                Some(path) => path,
                None => return -1,
            },
            Mode::RenameInput { path, .. }
            | Mode::CopyInput { path, .. }
            | Mode::ExtractInput { path, .. } => path,
            // Nothing else is drawn on a row: the address bar's two
            // prompts live in the address bar, Normal has no prompt, and
            // the switcher (#590) draws over every pane rather than one row.
            Mode::Normal
            | Mode::PathInput { .. }
            | Mode::FindInput { .. }
            | Mode::ReposRootInput { .. }
            | Mode::Switcher { .. } => return -1,
        };
        let dir = self.selected_dir_path();
        self.contents
            .iter()
            .position(|entry| dir.join(&entry.name) == *path)
            .and_then(|index| i32::try_from(index).ok())
            .unwrap_or(-1)
    }

    /// Whether the pending prompt takes typed text, as opposed to the
    /// delete confirmation's single y/n keypress. The pane draws a caret
    /// only for the former.
    #[must_use]
    pub fn prompt_is_editable(&self) -> bool {
        matches!(
            self.mode,
            Mode::RenameInput { .. } | Mode::CopyInput { .. } | Mode::ExtractInput { .. }
        )
    }

    /// Item count, total size, and current selection for the browsed
    /// folder, e.g. `"42 items, 1.2 MB — selected: notes.txt (3 of 42)"`.
    /// Falls back to a usage hint when the folder hasn't loaded any
    /// contents yet, and to a short line naming just the changed-only
    /// filter (#582) while that is narrowing the pane - the size and
    /// selection a full summary carries say nothing a filtered view needs.
    fn contents_summary(&self) -> String {
        if let Some(view) = &self.all_repositories {
            let count = view.entries.len();
            let noun = if count == 1 {
                "repository"
            } else {
                "repositories"
            };
            return format!("{count} {noun} found");
        }
        if self.filter.changed_only {
            return format!("showing {} with uncommitted changes", self.contents.len());
        }
        if self.contents.is_empty() {
            return "Click a folder or file. Double-click to open. Delete/r/c/x on a file. \
                    Esc cancels."
                .to_owned();
        }
        let count = self.contents.len();
        let noun = if count == 1 { "item" } else { "items" };
        // The Size column, and this size, are worth drawing only once the
        // listing holds a file: every row is a folder in the common case
        // of browsing a Repos Directory, where an empty size is noise
        // (#582).
        let header = if self.content_size_column_visible() {
            let total_size: u64 = self.contents.iter().map(|entry| entry.size).sum();
            format!(
                "{count} {noun}, {}{}",
                format_size(total_size),
                self.repositories_summary()
            )
        } else {
            format!("{count} {noun}{}", self.repositories_summary())
        };
        let selected = self.selected_count();
        if selected > 1 {
            return format!("{header} — {selected} selected");
        }
        match self.contents.get(self.content_selected) {
            Some(entry) => format!(
                "{header} — selected: {} ({} of {count})",
                entry.name,
                self.content_selected + 1
            ),
            None => header,
        }
    }

    /// Which pane is currently focused, as an index (0/1/2) matching the
    /// UI's `focus-pane` property.
    #[must_use]
    pub fn focus_index(&self) -> i32 {
        match self.focus {
            Pane::Folders => 0,
            Pane::Contents => 1,
            Pane::File => 2,
        }
    }

    /// Everything that says what the reader is looking at, in one value.
    /// `sync_ui` reads the same state through the narrower accessors above;
    /// this exists for what wants it as a whole - a test proving that an
    /// intent method left every part of the selection consistent, and a
    /// window test proving that every pane agrees with it.
    #[must_use]
    pub fn selection(&self) -> Selection {
        Selection {
            folder: self.folder_selected,
            content: self.content_selected,
            contents: self.selection.clone(),
            anchor: self.anchor,
            file_view_index: self.file_view_index,
            editing: self.editing_file.is_some(),
            focus: self.focus,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        Class, ColouredRun, EditCommand, Pane, colour_summary_line, file_starts_in_preview,
        middle_elide, summary_label,
    };

    use super::{
        App, CANNOT_TELL_MARKER, CHANGED_MARKER, NOT_KNOWN_YET_MARKER, PathBuf, RepositoryMark,
        STALE_FETCH_MARKER, Selection, UNKNOWN_ICON, chevron_hit, fetch_is_stale, format_kind,
        format_kind_of, format_timestamp, icon_for, now_epoch_seconds, repository_mark,
        strip_verbatim_prefix,
    };
    use plugin_api::{PREVIEW_VIEW, TEXT_VIEW};
    use protocol::{DirectoryEntry, RepositoryInfo, RepositoryKind, Response};

    fn entries(names: &[(&str, bool)]) -> Vec<DirectoryEntry> {
        names
            .iter()
            .map(|(name, is_dir)| DirectoryEntry {
                name: (*name).to_owned(),
                is_dir: *is_dir,
                size: 0,
                modified: None,
                repository: None,
            })
            .collect()
    }

    /// Entries with sizes and modified times, for the details columns.
    fn detailed_entries(rows: &[(&str, bool, u64, Option<u64>)]) -> Vec<DirectoryEntry> {
        rows.iter()
            .map(|(name, is_dir, size, modified)| DirectoryEntry {
                name: (*name).to_owned(),
                is_dir: *is_dir,
                size: *size,
                modified: *modified,
                repository: None,
            })
            .collect()
    }

    /// Four rows, with the second selected.
    fn app_with_four_rows() -> App {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[
                    ("a.txt", false),
                    ("b.txt", false),
                    ("c.txt", false),
                    ("d.txt", false),
                ]),
            }),
        );
        app.select_content(1);
        app
    }

    #[test]
    fn ctrl_click_adds_a_row_to_the_selection_and_clicking_it_again_removes_it() {
        let mut app = app_with_four_rows();
        assert_eq!(app.selected_count(), 1);

        app.toggle_content(3);
        assert_eq!(app.selected_count(), 2);
        assert!(app.is_selected(1));
        assert!(app.is_selected(3));

        app.toggle_content(1);
        assert_eq!(app.selected_count(), 1, "the first row drops out again");
        assert!(!app.is_selected(1));
        assert!(app.is_selected(3));
    }

    #[test]
    fn shift_click_selects_the_range_from_the_anchor() {
        let mut app = app_with_four_rows();

        app.extend_selection_to(3);

        assert_eq!(app.selected_count(), 3);
        assert!(!app.is_selected(0));
        for index in 1..=3 {
            assert!(app.is_selected(index), "row {index} is in the range");
        }
    }

    #[test]
    fn shift_click_backwards_selects_the_range_the_other_way() {
        let mut app = app_with_four_rows();
        app.select_content(2);

        app.extend_selection_to(0);

        assert_eq!(app.selected_count(), 3);
        for index in 0..=2 {
            assert!(app.is_selected(index));
        }
        assert!(!app.is_selected(3));
    }

    #[test]
    fn a_plain_click_drops_back_to_a_single_row() {
        let mut app = app_with_four_rows();
        app.extend_selection_to(3);
        assert_eq!(app.selected_count(), 3);

        app.select_content(0);

        assert_eq!(app.selected_count(), 1);
        assert!(app.is_selected(0));
    }

    #[test]
    fn a_marquee_selects_every_row_it_covered() {
        let mut app = app_with_four_rows();

        app.select_range(1, 3);

        assert_eq!(app.selected_count(), 3);
        assert!(!app.is_selected(0));
        for index in 1..=3 {
            assert!(app.is_selected(index));
        }
        assert_eq!(
            app.content_selected(),
            3,
            "the lead row is where the drag ended"
        );
    }

    #[test]
    fn a_marquee_dragged_upwards_covers_the_same_rows() {
        let mut app = app_with_four_rows();

        app.select_range(3, 1);

        assert_eq!(app.selected_count(), 3);
        for index in 1..=3 {
            assert!(app.is_selected(index));
        }
        assert_eq!(
            app.content_selected(),
            1,
            "the lead row is still where the drag ended"
        );
    }

    #[test]
    fn a_marquee_past_the_last_row_stops_at_it() {
        let mut app = app_with_four_rows();

        app.select_range(2, 99);

        assert_eq!(app.selected_count(), 2);
        assert!(app.is_selected(2));
        assert!(app.is_selected(3));
    }

    #[test]
    fn ctrl_a_selects_every_row_and_the_status_bar_counts_them() {
        let mut app = app_with_four_rows();

        app.select_all();

        assert_eq!(app.selected_count(), 4);
        assert!(
            app.status_text().ends_with("4 selected"),
            "{}",
            app.status_text()
        );
    }

    // Each intent method below is asserted against the whole `Selection`
    // it leaves behind (#614), rather than one getter at a time - what a
    // pane reads through `App::selection()` is exactly this value, so a
    // method that gets one field right and another wrong would pass a
    // narrower test and still leave two panes disagreeing.

    #[test]
    fn select_folder_moves_the_folder_and_gives_the_tree_focus() {
        let mut app = App::new(std::env::temp_dir().join("repos"));
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("src", true)]),
            }),
        );

        app.select_folder(1);

        assert_eq!(
            app.selection(),
            Selection {
                folder: 1,
                // The listing this kicks off never lands in a unit test,
                // so Contents keeps showing the root's own single row.
                content: 0,
                contents: [0].into_iter().collect(),
                anchor: 0,
                file_view_index: 0,
                editing: false,
                focus: Pane::Folders,
            }
        );
    }

    #[test]
    fn select_content_replaces_the_selection_with_one_row_and_gives_contents_focus() {
        let mut app = app_with_four_rows();

        app.select_content(2);

        assert_eq!(
            app.selection(),
            Selection {
                folder: 0,
                content: 2,
                contents: [2].into_iter().collect(),
                anchor: 2,
                file_view_index: 0,
                editing: false,
                focus: Pane::Contents,
            }
        );
    }

    #[test]
    fn toggle_content_adds_to_the_selection_without_moving_the_anchor() {
        let mut app = app_with_four_rows(); // row 1 already selected.

        app.toggle_content(3);

        let selection = app.selection();
        assert_eq!(selection.contents, [1, 3].into_iter().collect());
        assert_eq!(selection.content, 3, "the row just clicked leads");
        assert_eq!(selection.anchor, 3);
        assert_eq!(selection.focus, Pane::Contents);
    }

    #[test]
    fn extend_selection_to_grows_the_range_from_the_anchor() {
        let mut app = app_with_four_rows(); // anchor is row 1.

        app.extend_selection_to(3);

        assert_eq!(
            app.selection(),
            Selection {
                folder: 0,
                content: 3,
                contents: [1, 2, 3].into_iter().collect(),
                anchor: 1,
                file_view_index: 0,
                editing: false,
                focus: Pane::Contents,
            }
        );
    }

    #[test]
    fn select_range_selects_the_marqueed_rows_and_leads_from_where_it_ended() {
        let mut app = app_with_four_rows();

        app.select_range(3, 1);

        assert_eq!(
            app.selection(),
            Selection {
                folder: 0,
                content: 1,
                contents: [1, 2, 3].into_iter().collect(),
                anchor: 3,
                file_view_index: 0,
                editing: false,
                focus: Pane::Contents,
            }
        );
    }

    #[test]
    fn select_all_selects_every_row_without_moving_the_lead() {
        let mut app = app_with_four_rows(); // row 1 is the lead.

        app.select_all();

        let selection = app.selection();
        assert_eq!(selection.contents, (0..4).collect());
        assert_eq!(selection.content, 1, "select_all does not move the lead");
        assert_eq!(selection.focus, Pane::Contents);
    }

    #[test]
    fn select_file_view_updates_only_the_file_view_index() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view(
            "rust",
            serde_json::json!({
                "content": "fn main() {}",
                "truncated": false,
                "functions": ["main"],
                "structs": [],
                "traits": [],
            }),
        );
        let before = app.selection();

        app.select_file_view(1);

        assert_eq!(
            app.selection(),
            Selection {
                file_view_index: 1,
                ..before
            }
        );
    }

    #[test]
    fn begin_file_edit_marks_the_selection_as_editing() {
        let mut app = app_with_editable_file();
        let before = app.selection();

        app.begin_file_edit();

        assert_eq!(
            app.selection(),
            Selection {
                editing: true,
                focus: Pane::File,
                ..before.clone()
            }
        );

        app.cancel_file_edit();

        assert_eq!(
            app.selection(),
            Selection {
                focus: Pane::File,
                ..before
            }
        );
    }

    #[test]
    fn deleting_a_multi_row_selection_confirms_once_and_sends_every_path() {
        let mut app = app_with_four_rows();
        app.extend_selection_to(3);

        app.request_delete();
        assert_eq!(app.prompt_text(), "Delete 3 items?  (y / n)");

        app.confirm_delete();

        assert!(app.pending_operation.is_some());
        assert_eq!(app.status_text(), "deleting...");
    }

    #[test]
    fn deleting_a_single_row_still_names_it() {
        let mut app = app_with_four_rows();

        app.request_delete();

        assert_eq!(app.prompt_text(), "Delete b.txt?  (y / n)");
    }

    #[test]
    fn typing_a_letter_jumps_to_the_next_name_starting_with_it() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[
                    ("apple.txt", false),
                    ("Banana.txt", false),
                    ("blueberry.txt", false),
                    ("cherry.txt", false),
                ]),
            }),
        );
        app.select_content(0);

        app.handle_key_text("b");
        assert_eq!(app.content_selected(), 1, "matched without regard to case");

        app.handle_key_text("b");
        assert_eq!(
            app.content_selected(),
            2,
            "a repeat cycles to the next match"
        );

        app.handle_key_text("z");
        assert_eq!(
            app.content_selected(),
            2,
            "no match leaves the selection put"
        );
    }

    #[test]
    fn copying_then_pasting_in_the_same_folder_asks_for_a_name_beside_the_original() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);

        app.copy_to_clipboard();
        assert_eq!(app.status_text(), "doomed.txt copied");

        app.paste_from_clipboard();

        assert!(app.pending_operation.is_some(), "a copy request went out");
        assert_eq!(app.status_text(), "working...");
    }

    #[test]
    fn cutting_clears_the_clipboard_once_it_has_been_pasted() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.cut_to_clipboard();
        assert_eq!(app.status_text(), "doomed.txt cut");

        app.paste_from_clipboard();
        assert!(app.pending_operation.is_some());

        app.cancel_pending();
        app.paste_from_clipboard();
        assert!(
            app.pending_operation.is_none(),
            "a cut is spent by the paste that moved it"
        );
    }

    #[test]
    fn pasting_with_an_empty_clipboard_does_nothing() {
        let mut app = app_with_one_content_entry();

        app.paste_from_clipboard();

        assert!(app.pending_operation.is_none());
    }

    #[test]
    fn home_and_end_jump_to_the_first_and_last_row() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("a.txt", false), ("b.txt", false), ("c.txt", false)]),
            }),
        );
        app.select_content(1);

        app.select_edge(true);
        assert_eq!(app.content_selected(), 2);

        app.select_edge(false);
        assert_eq!(app.content_selected(), 0);
    }

    #[test]
    fn page_movement_clamps_at_the_ends_of_the_list() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("a.txt", false), ("b.txt", false), ("c.txt", false)]),
            }),
        );
        app.select_content(0);

        app.move_selection(10);
        assert_eq!(app.content_selected(), 2);

        app.move_selection(-10);
        assert_eq!(app.content_selected(), 0);
    }

    /// An app with one selected entry whose preview is an editable text
    /// view, as a text plugin would produce.
    fn app_with_editable_file() -> App {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view(
            "text",
            serde_json::json!({ "content": "first
second", "truncated": false }),
        );
        app
    }

    #[test]
    fn a_text_type_offers_its_own_text_as_a_second_view() {
        // A source plugin, whose preview prepends an outline to the text -
        // which is the commentary the second view exists to leave out.
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view(
            "rust",
            serde_json::json!({
                "content": "fn main() {}",
                "truncated": false,
                "functions": ["main"],
                "structs": [],
                "traits": [],
            }),
        );

        assert_eq!(app.file_views(), vec![PREVIEW_VIEW, TEXT_VIEW]);
        assert_eq!(app.file_view_index(), 0);
        let preview = app.file_text();
        assert!(
            preview.contains("functions: main"),
            "the preview is the plugin's own rendering: {preview}"
        );

        app.select_file_view(1);
        assert_eq!(
            app.file_text(),
            "fn main() {}",
            "the file's own text, with none of the outline the preview adds"
        );
    }

    #[test]
    fn middle_elide_leaves_a_short_value_alone() {
        assert_eq!(middle_elide("short", 10), "short");
    }

    #[test]
    fn middle_elide_keeps_both_ends_of_a_long_value() {
        assert_eq!(
            middle_elide("abcdefghijklmnopqrstuvwxyz", 10),
            "abcdef…xyz",
            "both the start and the end should still be readable"
        );
    }

    #[test]
    fn a_repositorys_facts_are_a_table_and_left_out_of_the_plain_text() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        let long_branch = "chore/solution-drift-model-refresh-agenttools-and-then-some-more";
        let data = serde_json::to_value(plugin_directory::DirectoryView {
            entry_count: 3,
            total_size: 4096,
            repository: Some(plugin_directory::repository::Repository {
                provider: Some("github.com".to_owned()),
                branch: Some(long_branch.to_owned()),
                remote: None,
                kind: plugin_directory::repository::Kind::Clone,
                tracking: None,
                status: None,
                last_activity: None,
                last_fetch: None,
            }),
            readme: None,
        })
        .unwrap();
        app.set_file_view("directory", data);

        let facts = app.file_facts();
        let branch = facts
            .iter()
            .find(|fact| fact.label == "Branch")
            .expect("a branch row");
        assert_eq!(branch.full_value, long_branch);
        assert_ne!(
            branch.display_value, long_branch,
            "a branch this long should elide in the value column"
        );
        assert!(branch.display_value.contains('…'));
        assert!(
            facts.iter().any(|fact| fact.label == "Provider"),
            "{facts:?}"
        );

        assert_eq!(
            app.file_text(),
            "",
            "the table already says everything the plain text used to; \
             showing both would say it twice"
        );
    }

    // ---- the "Worktree of"/"Submodule of" line (#587) -------------------

    fn repository_view(kind: plugin_directory::repository::Kind) -> serde_json::Value {
        serde_json::to_value(plugin_directory::DirectoryView {
            entry_count: 1,
            total_size: 0,
            repository: Some(plugin_directory::repository::Repository {
                provider: Some("github.com".to_owned()),
                branch: Some("main".to_owned()),
                remote: None,
                kind,
                tracking: None,
                status: None,
                last_activity: None,
                last_fetch: None,
            }),
            readme: None,
        })
        .unwrap()
    }

    #[test]
    fn an_ordinary_clone_has_no_related_repository_line() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view(
            "directory",
            repository_view(plugin_directory::repository::Kind::Clone),
        );

        assert_eq!(app.file_related_repository_label(), "");
        assert!(!app.file_related_repository_linked());
    }

    #[test]
    fn a_worktree_beside_its_clone_names_it_without_repeating_the_path() {
        let root = std::env::temp_dir().join("rse-related-repository-sibling");
        let mut app = App::new(root.clone());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: Vec::new(),
            }),
        );
        app.set_file_view(
            "directory",
            repository_view(plugin_directory::repository::Kind::Worktree {
                clone: root.join("clone"),
                clone_exists: true,
            }),
        );

        assert_eq!(app.file_related_repository_label(), "Worktree of clone");
        assert!(app.file_related_repository_linked());
    }

    #[test]
    fn a_submodule_nested_deeper_names_its_path_too() {
        let root = std::env::temp_dir().join("rse-related-repository-nested");
        let mut app = App::new(root.clone());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: Vec::new(),
            }),
        );
        app.set_file_view(
            "directory",
            repository_view(plugin_directory::repository::Kind::Submodule {
                outer: root.join("vendor").join("forge"),
            }),
        );

        assert_eq!(
            app.file_related_repository_label(),
            "Submodule of forge (at vendor/forge)"
        );
        assert!(app.file_related_repository_linked());
    }

    #[test]
    fn a_worktree_whose_clone_is_gone_names_where_it_was_and_is_not_a_link() {
        let root = std::env::temp_dir().join("rse-related-repository-gone");
        let clone = root.join("clone");
        let mut app = App::new(root.clone());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: Vec::new(),
            }),
        );
        app.set_file_view(
            "directory",
            repository_view(plugin_directory::repository::Kind::Worktree {
                clone: clone.clone(),
                clone_exists: false,
            }),
        );

        assert_eq!(
            app.file_related_repository_label(),
            format!(
                "Worktree of a clone that is no longer at {}",
                clone.display()
            )
        );
        assert!(!app.file_related_repository_linked());
    }

    #[test]
    fn following_the_related_repository_link_goes_to_the_clone_and_selects_it() {
        let root = std::env::temp_dir().join("rse-related-repository-follow");
        let mut app = App::new(root.clone());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: Vec::new(),
            }),
        );
        app.set_file_view(
            "directory",
            repository_view(plugin_directory::repository::Kind::Worktree {
                clone: root.join("clone"),
                clone_exists: true,
            }),
        );

        app.open_related_repository();

        assert_eq!(app.root.path, root, "the clone's parent is the new root");
        assert_eq!(app.reselect.as_deref(), Some("clone"));
    }

    #[test]
    fn a_type_carrying_no_text_offers_a_single_view() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view("image", serde_json::json!({ "width": 4, "height": 4 }));

        assert_eq!(app.file_views(), vec![PREVIEW_VIEW]);
    }

    #[test]
    fn a_truncated_view_still_reads_as_text() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view(
            "text",
            serde_json::json!({ "content": "the first half", "truncated": true }),
        );

        assert_eq!(app.file_views(), vec![PREVIEW_VIEW, TEXT_VIEW]);
        app.select_file_view(1);
        assert_eq!(app.file_text(), "the first half");
        assert_eq!(
            app.editable_text(),
            None,
            "reading part of a long file is the point; saving part of one back is not"
        );
    }

    #[test]
    fn switching_views_leaves_the_contents_selection_alone() {
        let mut app = app_with_editable_file();
        let before = (app.content_selected(), app.selected_entry_path());

        app.select_file_view(1);

        assert_eq!(app.content_selected(), before.0);
        assert_eq!(app.selected_entry_path(), before.1);
    }

    #[test]
    fn a_new_preview_goes_back_to_the_types_first_view() {
        let mut app = app_with_editable_file();
        app.select_file_view(1);

        app.set_file_view("image", serde_json::json!({ "width": 4, "height": 4 }));

        assert_eq!(
            app.file_view_index(),
            0,
            "a view chosen for one file may not exist for the next"
        );
    }

    #[test]
    fn a_view_the_type_does_not_offer_is_ignored() {
        let mut app = app_with_editable_file();

        app.select_file_view(7);

        assert_eq!(app.file_view_index(), 0);
    }

    #[test]
    fn a_text_view_offers_its_text_for_editing() {
        let app = app_with_editable_file();

        assert_eq!(
            app.editable_text().as_deref(),
            Some(
                "first
second"
            )
        );
        assert!(app.can_edit());
    }

    #[test]
    fn a_truncated_view_is_never_editable() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view(
            "text",
            serde_json::json!({ "content": "the first half", "truncated": true }),
        );

        assert_eq!(
            app.editable_text(),
            None,
            "saving part of a file back would discard the rest"
        );
        assert!(!app.can_edit());
    }

    #[test]
    fn a_view_that_is_not_text_is_not_editable() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view("image", serde_json::json!({ "width": 4, "height": 4 }));

        assert_eq!(app.editable_text(), None);
        assert!(!app.can_edit());
    }

    #[test]
    fn saving_an_edit_sends_the_typed_text_to_the_service() {
        let mut app = app_with_editable_file();

        app.begin_file_edit();
        assert!(app.editing_file());
        assert_eq!(
            app.edit_text(),
            "first
second"
        );
        assert!(!app.can_edit(), "already open, so Edit has nothing to do");

        app.set_edit_text(
            "first
second
third",
        );
        app.save_file_edit();

        assert!(!app.editing_file(), "the editor closes on save");
        assert!(app.pending_operation.is_some(), "a write went out");
        assert_eq!(app.status_text(), "saving...");
    }

    #[test]
    fn discarding_an_edit_writes_nothing_and_says_so() {
        let mut app = app_with_editable_file();
        app.begin_file_edit();
        app.set_edit_text("changed");

        app.cancel_file_edit();

        assert!(!app.editing_file());
        assert!(app.pending_operation.is_none(), "nothing was written");
        assert_eq!(
            app.status_text(),
            "edit discarded",
            "the one place text is lost, so it is not lost silently"
        );
    }

    #[test]
    fn escape_closes_the_editor_rather_than_leaving_it_open() {
        let mut app = app_with_editable_file();
        app.begin_file_edit();

        app.cancel_pending();

        assert!(!app.editing_file());
    }

    #[test]
    fn a_file_that_cannot_be_edited_does_not_open_an_editor() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view("image", serde_json::json!({ "width": 4 }));

        app.begin_file_edit();

        assert!(!app.editing_file());
    }

    #[test]
    fn opening_the_address_bar_prefills_the_folder_being_browsed() {
        let mut app = App::new(PathBuf::from("/one/two"));
        assert!(!app.editing_path());
        assert_eq!(app.path_input(), "");

        app.begin_path_edit();

        assert!(app.editing_path());
        assert_eq!(
            app.path_input(),
            app.current_path(),
            "the field starts as the path you are on, ready to be edited"
        );
    }

    #[test]
    fn a_typed_path_is_navigated_to_and_recorded_in_history() {
        let mut app = App::new(PathBuf::from("/one/two"));
        app.begin_path_edit();
        for _ in 0..app.path_input().len() {
            app.backspace();
        }
        for c in "/three/four".chars() {
            app.handle_key_text(&c.to_string());
        }

        app.handle_return();

        assert!(!app.editing_path(), "the field closes");
        assert_eq!(
            app.breadcrumbs().last().map(String::as_str),
            Some("four"),
            "and the typed folder is the one being browsed"
        );
        assert!(app.can_go_back(), "with Back able to return");
    }

    #[test]
    fn escaping_the_address_bar_leaves_the_folder_alone() {
        let mut app = App::new(PathBuf::from("/one/two"));
        let before = app.breadcrumbs();
        app.begin_path_edit();
        for c in "/somewhere/else".chars() {
            app.handle_key_text(&c.to_string());
        }

        app.cancel_pending();

        assert!(!app.editing_path());
        assert_eq!(app.breadcrumbs(), before, "nowhere was navigated to");
    }

    #[test]
    fn an_empty_path_navigates_nowhere() {
        let mut app = App::new(PathBuf::from("/one/two"));
        let before = app.breadcrumbs();
        app.begin_path_edit();
        for _ in 0..app.path_input().len() {
            app.backspace();
        }

        app.handle_return();

        assert_eq!(app.breadcrumbs(), before);
    }

    #[test]
    fn the_address_bar_does_not_open_over_another_prompt() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.request_rename();

        app.begin_path_edit();

        assert!(
            !app.editing_path(),
            "a rename in progress keeps the address bar closed"
        );
    }

    #[test]
    fn breadcrumbs_name_each_folder_on_the_way_down_with_the_root_as_one() {
        let app = App::new(PathBuf::from("/one/two/three"));

        let crumbs = app.breadcrumbs();

        assert_eq!(crumbs.last().map(String::as_str), Some("three"));
        assert!(
            crumbs.len() >= 3,
            "root, then a segment per folder: {crumbs:?}"
        );
        assert!(
            !crumbs.iter().any(String::is_empty),
            "no blank segment for the root separator: {crumbs:?}"
        );
    }

    #[test]
    fn back_and_forward_walk_the_folders_that_were_visited() {
        let mut app = App::new(PathBuf::from("/one/two/three"));
        assert!(!app.can_go_back(), "nowhere to go back to yet");
        assert!(!app.can_go_forward());

        app.navigate_to_parent();
        assert_eq!(app.breadcrumbs().last().map(String::as_str), Some("two"));
        assert!(app.can_go_back());
        assert!(!app.can_go_forward());

        app.go_back();
        assert_eq!(app.breadcrumbs().last().map(String::as_str), Some("three"));
        assert!(app.can_go_forward(), "and forward returns");

        app.go_forward();
        assert_eq!(app.breadcrumbs().last().map(String::as_str), Some("two"));
    }

    #[test]
    fn navigating_somewhere_new_discards_the_forward_stack() {
        let mut app = App::new(PathBuf::from("/one/two/three"));
        app.navigate_to_parent();
        app.go_back();
        assert!(app.can_go_forward());

        app.navigate_to_breadcrumb(0);

        assert!(
            !app.can_go_forward(),
            "a fresh navigation drops what was ahead"
        );
        assert!(app.can_go_back());
    }

    #[test]
    fn clicking_the_folder_already_shown_in_the_address_bar_does_nothing() {
        let mut app = App::new(PathBuf::from("/one/two/three"));
        let before = app.breadcrumbs();

        app.navigate_to_breadcrumb(i32::try_from(before.len()).unwrap() - 1);

        assert_eq!(app.breadcrumbs(), before);
        assert!(!app.can_go_back(), "and records no history for it");
    }

    #[test]
    fn each_file_type_takes_its_icon_from_its_own_plugin() {
        assert_eq!(icon_for("main.rs", false).label, "RS");
        assert_eq!(icon_for("photo.PNG", false).label, "IMG");
        assert_eq!(icon_for("bundle.zip", false).label, "ZIP");
        assert_eq!(icon_for("report.pdf", false).label, "PDF");
        assert_eq!(icon_for("index.html", false).label, "HTML");
    }

    #[test]
    fn a_format_named_rather_than_suffixed_is_matched_on_the_whole_name() {
        assert_eq!(icon_for("Dockerfile", false).label, "DOCK");
        assert_eq!(icon_for("Makefile", false).label, "MAKE");
    }

    #[test]
    fn a_directory_takes_the_directory_plugin_s_icon() {
        let folder = icon_for("anything", true);
        assert_eq!(folder.label, "DIR");
        assert_ne!(folder, icon_for("main.rs", false));
    }

    #[test]
    fn an_unclaimed_extension_falls_back_to_the_unknown_icon() {
        assert_eq!(icon_for("mystery.xyz123", false), UNKNOWN_ICON);
        assert_eq!(icon_for("no_extension_at_all", false), UNKNOWN_ICON);
    }

    #[test]
    fn no_two_plugins_claim_the_same_extension() {
        let mut seen: std::collections::BTreeMap<&str, &str> = std::collections::BTreeMap::new();
        let mut clashes = Vec::new();
        for plugin in super::PRESENTATION_PLUGINS {
            for extension in plugin.extensions() {
                assert_eq!(
                    *extension,
                    extension.to_lowercase(),
                    "{}'s {extension:?} should be lowercase",
                    plugin.name()
                );
                if let Some(other) = seen.insert(extension, plugin.name()) {
                    clashes.push(format!("{extension:?}: {other} and {}", plugin.name()));
                }
            }
        }
        assert!(clashes.is_empty(), "{clashes:?}");
        assert!(seen.len() > 100, "only {} extensions claimed", seen.len());
    }

    #[test]
    fn every_plugin_states_an_icon_of_its_own() {
        for plugin in super::PRESENTATION_PLUGINS {
            let icon = plugin.icon();
            assert_ne!(
                icon,
                UNKNOWN_ICON,
                "{} still has the fallback icon",
                plugin.name()
            );
            assert!(
                !icon.label.is_empty() && icon.label.chars().count() <= 4,
                "{}'s label {:?} should be one to four characters",
                plugin.name(),
                icon.label
            );
        }
    }

    #[test]
    fn the_type_column_names_folders_and_extensions_the_way_explorer_does() {
        assert_eq!(format_kind("src", true), "File folder");
        assert_eq!(format_kind("main.rs", false), "RS file");
        assert_eq!(format_kind("archive.TAR", false), "TAR file");
        assert_eq!(format_kind("LICENSE", false), "File");
    }

    #[test]
    fn the_type_column_names_a_working_copy_and_its_provider() {
        let with_provider = RepositoryInfo {
            provider: Some("github.com".to_owned()),
            branch: None,
            remote: None,
            kind: RepositoryKind::Clone,
            last_activity: None,
            last_fetch: None,
        };
        assert_eq!(
            format_kind_of("repo", true, Some(&with_provider)),
            "Repository · github.com",
            "the full \"Git repository · github.com\" still overruns the column, \
             so the shorter form keeps the provider"
        );

        let without_provider = RepositoryInfo {
            provider: None,
            branch: None,
            remote: None,
            kind: RepositoryKind::Clone,
            last_activity: None,
            last_fetch: None,
        };
        assert_eq!(
            format_kind_of("repo", true, Some(&without_provider)),
            "Git repository"
        );

        assert_eq!(format_kind_of("plain", true, None), "File folder");
    }

    #[test]
    fn the_type_column_names_a_worktree_and_a_submodule_apart_from_a_clone() {
        let worktree = RepositoryInfo {
            provider: Some("github.com".to_owned()),
            branch: None,
            remote: None,
            kind: RepositoryKind::Worktree {
                clone: "/repos/clone".to_owned(),
                clone_exists: true,
            },
            last_activity: None,
            last_fetch: None,
        };
        assert_eq!(
            format_kind_of("linked", true, Some(&worktree)),
            "Worktree · github.com"
        );

        let submodule = RepositoryInfo {
            provider: Some("gitlab.com".to_owned()),
            branch: None,
            remote: None,
            kind: RepositoryKind::Submodule {
                outer: "/repos/outer".to_owned(),
            },
            last_activity: None,
            last_fetch: None,
        };
        assert_eq!(
            format_kind_of("inner", true, Some(&submodule)),
            "Submodule · gitlab.com"
        );

        let worktree_with_no_remote = RepositoryInfo {
            provider: None,
            branch: None,
            remote: None,
            kind: RepositoryKind::Worktree {
                clone: "/repos/clone".to_owned(),
                clone_exists: false,
            },
            last_activity: None,
            last_fetch: None,
        };
        assert_eq!(
            format_kind_of("linked", true, Some(&worktree_with_no_remote)),
            "Worktree"
        );
    }

    #[test]
    fn the_modified_column_formats_epoch_seconds_as_a_date_and_time() {
        assert_eq!(format_timestamp(Some(0)), "1970-01-01 00:00");
        // 2026-09-07T14:31:00Z.
        assert_eq!(format_timestamp(Some(1_788_791_460)), "2026-09-07 14:31");
        assert_eq!(
            format_timestamp(None),
            "",
            "a filesystem that reports no time leaves the cell blank"
        );
    }

    #[test]
    fn content_rows_carry_a_cell_per_column() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: detailed_entries(&[
                    ("notes.txt", false, 2048, Some(0)),
                    ("src", true, 0, Some(0)),
                ]),
            }),
        );

        let rows = app.content_rows();

        assert_eq!(rows[0].name, "src/", "folders sort ahead of files");
        assert_eq!(rows[0].kind, "File folder");
        assert_eq!(rows[0].size, "", "a folder shows no size");
        assert_eq!(rows[1].name, "notes.txt");
        assert_eq!(rows[1].size, "2.0 KB");
        assert_eq!(rows[1].kind, "TXT file");
        assert_eq!(rows[1].modified, "1970-01-01 00:00");
    }

    #[test]
    fn clicking_a_column_sorts_by_it_and_clicking_again_reverses() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: detailed_entries(&[
                    ("big.bin", false, 900, None),
                    ("small.bin", false, 10, None),
                    ("mid.bin", false, 100, None),
                ]),
            }),
        );
        assert_eq!(app.sort_column(), 0, "name to begin with");

        app.sort_by_column(1);
        assert_eq!(app.sort_column(), 1);
        assert!(app.sort_ascending());
        let names: Vec<_> = app.content_rows().into_iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["small.bin", "mid.bin", "big.bin"]);

        app.sort_by_column(1);
        assert!(!app.sort_ascending(), "the same column reverses");
        let names: Vec<_> = app.content_rows().into_iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["big.bin", "mid.bin", "small.bin"]);

        app.sort_by_column(9);
        assert_eq!(app.sort_column(), 1, "an unknown column is ignored");
    }

    #[test]
    fn sorting_keeps_folders_first_and_holds_the_selection() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: detailed_entries(&[
                    ("zeta.txt", false, 10, None),
                    ("alpha", true, 0, None),
                    ("beta.txt", false, 900, None),
                ]),
            }),
        );
        app.select_content(1); // beta.txt, after the folder.
        assert_eq!(app.content_rows()[1].name, "beta.txt");

        app.sort_by_column(1);

        assert_eq!(
            app.content_rows()[0].name,
            "alpha/",
            "the folder stays at the top whichever column is sorted on"
        );
        assert_eq!(
            app.content_rows()[app.content_selected()].name,
            "beta.txt",
            "the selection follows its entry"
        );
    }

    #[test]
    fn sorting_by_modified_orders_repositories_by_last_activity_not_folder_time() {
        // The folder's own modification time only moves when an entry
        // directly inside it changes, so it is a poor answer to "which was
        // I in most recently"; last activity is read from the checkout's
        // own files instead (#588).
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![
                    DirectoryEntry {
                        name: "older-folder-newer-activity".to_owned(),
                        is_dir: true,
                        size: 0,
                        modified: Some(100),
                        repository: Some(RepositoryInfo {
                            provider: None,
                            branch: None,
                            remote: None,
                            kind: RepositoryKind::Clone,
                            last_activity: Some(900),
                            last_fetch: None,
                        }),
                    },
                    DirectoryEntry {
                        name: "newer-folder-older-activity".to_owned(),
                        is_dir: true,
                        size: 0,
                        modified: Some(800),
                        repository: Some(RepositoryInfo {
                            provider: None,
                            branch: None,
                            remote: None,
                            kind: RepositoryKind::Clone,
                            last_activity: Some(200),
                            last_fetch: None,
                        }),
                    },
                    DirectoryEntry {
                        name: "plain-folder".to_owned(),
                        is_dir: true,
                        size: 0,
                        modified: Some(500),
                        repository: None,
                    },
                ],
            }),
        );

        app.sort_by_column(3);

        assert_eq!(
            app.content_rows()
                .iter()
                .map(|row| row.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "newer-folder-older-activity/",
                "plain-folder/",
                "older-folder-newer-activity/",
            ],
            "the repositories order by last activity (200, 900), the plain \
             folder by its own time (500) - not by folder time throughout"
        );
        let newest_row = app
            .content_rows()
            .into_iter()
            .find(|row| row.name == "older-folder-newer-activity/")
            .unwrap();
        assert_eq!(
            newest_row.modified,
            format_timestamp(Some(900)),
            "the Modified cell shows last activity, not the folder's own time"
        );
    }

    #[test]
    fn content_holds_repository_is_true_once_any_row_is_a_working_copy() {
        let mut app = App::new(std::env::temp_dir());
        assert!(
            !app.content_holds_repository(),
            "an empty listing holds no repository"
        );

        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("plain", true), ("also-plain.txt", false)]),
            }),
        );
        assert!(!app.content_holds_repository());

        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![
                    DirectoryEntry {
                        name: "plain".to_owned(),
                        is_dir: true,
                        size: 0,
                        modified: None,
                        repository: None,
                    },
                    DirectoryEntry {
                        name: "checkout".to_owned(),
                        is_dir: true,
                        size: 0,
                        modified: None,
                        repository: Some(RepositoryInfo {
                            provider: None,
                            branch: None,
                            remote: None,
                            kind: RepositoryKind::Clone,
                            last_activity: None,
                            last_fetch: None,
                        }),
                    },
                ],
            }),
        );
        assert!(app.content_holds_repository());
    }

    #[test]
    fn applying_a_directory_result_populates_contents_and_tree() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("sub", true), ("note.txt", false)]),
            }),
        );

        assert_eq!(app.content_labels(), vec!["sub/", "note.txt"]);
        assert_eq!(app.folder_rows().len(), 2); // root + "sub"
    }

    #[test]
    fn status_text_reports_the_folder_item_count() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("sub", true), ("a.txt", false), ("b.txt", false)]),
            }),
        );

        assert!(app.status_text().starts_with("3 items"));
    }

    #[test]
    fn status_text_reports_total_size_of_the_folder_contents() {
        let mut app = App::new(std::env::temp_dir());
        let mut entries = entries(&[("a.txt", false), ("b.txt", false)]);
        entries[0].size = 1000;
        entries[1].size = 500;
        app.apply_contents_result(&[], Ok(Response::Directory { entries }));

        assert!(app.status_text().contains("1.5 KB"));
    }

    #[test]
    fn selecting_a_content_row_updates_the_status_with_selection() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("a.txt", false), ("b.txt", false)]),
            }),
        );

        app.select_content(1);

        let status = app.status_text();
        assert!(status.starts_with("2 items"));
        assert!(status.contains("selected: b.txt (2 of 2)"));
    }

    #[test]
    fn selecting_a_file_triggers_a_preview_request() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("a.txt", false), ("b.txt", false)]),
            }),
        );

        app.select_content(1);
        assert_eq!(app.content_selected(), 1);
    }

    #[test]
    fn cancelling_a_pending_request_makes_a_late_result_harmless() {
        let mut app = App::new(std::env::temp_dir());
        let (tx, rx) = std::sync::mpsc::channel();
        app.pending_contents = Some((vec![], rx));

        app.cancel_pending();
        assert!(app.pending_contents.is_none());
        assert_eq!(app.status_text(), "cancelled");

        assert!(
            tx.send(Ok(Response::Directory { entries: vec![] }))
                .is_err()
        );
    }

    #[test]
    fn tick_applies_a_completed_pending_contents_request() {
        let mut app = App::new(std::env::temp_dir());
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Ok(Response::Directory {
            entries: entries(&[("only.txt", false)]),
        }))
        .unwrap();
        app.pending_contents = Some((vec![], rx));

        app.tick();

        assert!(app.pending_contents.is_none());
        assert_eq!(app.content_labels(), vec!["only.txt"]);
    }

    #[test]
    fn return_drills_into_the_selected_folder() {
        // The keyboard way to do what double-clicking does. Before this
        // Return did nothing at all off macOS, so a folder in the
        // contents pane could only be opened with the mouse.
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("sub", true)]),
            }),
        );
        app.select_content(0);

        app.handle_return_for_os("windows");

        assert_eq!(app.folder_selected(), 1, "it drilled in");
        assert!(app.status_text().starts_with("loading"));
    }

    #[test]
    fn return_on_a_file_does_nothing() {
        // This application reads files; it does not launch them, and
        // selecting one has already shown it in the File pane.
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("readings.csv", false)]),
            }),
        );
        app.select_content(0);
        let before = app.folder_selected();

        app.handle_return_for_os("windows");

        assert_eq!(app.folder_selected(), before);
    }

    #[test]
    fn return_still_renames_on_macos() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("sub", true)]),
            }),
        );
        app.select_content(0);

        app.handle_return_for_os("macos");

        assert_eq!(
            app.folder_selected(),
            0,
            "Return is rename on macOS, and drilling in is the mouse's job there"
        );
    }

    #[test]
    fn return_in_the_folders_pane_collapses_that_row() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("sub", true)]),
            }),
        );
        app.select_folder(0);
        assert!(app.folder_rows()[0].expanded, "the root starts expanded");

        app.handle_return_for_os("windows");

        assert!(
            !app.folder_rows()[0].expanded,
            "Return acts on whichever pane has focus, and in the tree that              is expanding or collapsing - the same as double-clicking it"
        );
    }

    #[test]
    fn drilling_in_leaves_the_keyboard_in_the_contents_pane() {
        // It used to move focus to the tree, so the arrow keys then
        // moved the tree rather than the listing being looked at, and a
        // second Return collapsed the folder just opened.
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("sub", true)]),
            }),
        );
        app.select_content(0);

        app.open_content(0);

        assert_eq!(app.focus_index(), 1, "the contents pane");
    }

    #[test]
    fn opening_a_directory_expands_and_selects_it_in_the_tree() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("sub", true)]),
            }),
        );

        app.open_content(0);

        assert_eq!(app.folder_selected(), 1);
        assert!(app.status_text().starts_with("loading"));
    }

    #[test]
    fn selecting_an_out_of_range_folder_row_is_a_no_op() {
        let mut app = App::new(std::env::temp_dir());
        let before = app.folder_selected();
        app.select_folder(999);
        assert_eq!(app.folder_selected(), before);
    }

    #[test]
    fn navigating_to_parent_moves_up_from_a_subdirectory() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("sub", true)]),
            }),
        );
        app.open_content(0);
        assert_eq!(app.folder_selected(), 1);

        app.navigate_to_parent();

        assert_eq!(app.folder_selected(), 0);
        assert!(app.status_text().starts_with("loading"));
    }

    #[test]
    fn navigating_to_parent_beyond_the_tree_root_reroots_the_tree() {
        let mut app = App::new(std::env::temp_dir());

        app.navigate_to_parent();

        assert_eq!(app.folder_selected(), 0);
        assert!(app.status_text().starts_with("loading"));
    }

    #[test]
    fn navigating_to_parent_at_the_filesystem_root_is_a_no_op() {
        let mut app = App::new(std::path::PathBuf::from("/"));
        app.cancel_pending();
        let status_before = app.status_text();

        app.navigate_to_parent();

        assert!(app.pending_contents.is_none());
        assert_eq!(app.status_text(), status_before);
    }

    #[test]
    fn strips_a_windows_verbatim_prefix_but_leaves_other_paths_alone() {
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"\\?\C:\dir\file.txt")),
            PathBuf::from(r"C:\dir\file.txt")
        );
        // A verbatim UNC path needs its prefix to keep resolving, and a
        // path from any other platform never carries one.
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"\\?\UNC\server\share")),
            PathBuf::from(r"\\?\UNC\server\share")
        );
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from("/home/user/file.txt")),
            PathBuf::from("/home/user/file.txt")
        );
    }

    #[test]
    fn new_canonicalizes_a_relative_root() {
        let app = App::new(std::path::PathBuf::from("."));

        assert!(app.root.path.is_absolute());
    }

    #[test]
    fn navigating_to_parent_from_a_relative_root_never_targets_an_empty_path() {
        // `Path::new(".").parent()` is `Some("")` (an empty path), not
        // `None` - without canonicalizing the root first, this used to
        // re-root the tree at "" and leave every future request targeting
        // a path that resolves to nothing, freezing all further input.
        let mut app = App::new(std::path::PathBuf::from("."));

        app.navigate_to_parent();

        assert!(!app.root.path.as_os_str().is_empty());
    }

    fn app_with_one_content_entry() -> App {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("doomed.txt", false)]),
            }),
        );
        app
    }

    #[test]
    fn delete_requested_asks_for_confirmation() {
        let mut app = app_with_one_content_entry();
        app.request_delete();
        assert_eq!(app.status_text(), "Delete doomed.txt? y/n");
    }

    #[test]
    fn declining_the_delete_confirmation_returns_to_normal_without_a_request() {
        let mut app = app_with_one_content_entry();
        app.request_delete();
        app.decline_delete();
        assert!(app.pending_operation.is_none());
        assert_ne!(app.status_text(), "Delete doomed.txt? y/n");
    }

    #[test]
    fn cancel_pending_during_delete_confirmation_returns_to_normal() {
        let mut app = app_with_one_content_entry();
        app.request_delete();
        app.cancel_pending();
        assert_ne!(app.status_text(), "Delete doomed.txt? y/n");
    }

    #[test]
    fn confirming_the_delete_sends_a_request_for_exactly_that_path() {
        let mut app = app_with_one_content_entry();
        app.request_delete();
        app.confirm_delete();
        assert!(app.pending_operation.is_some());
        assert_eq!(app.status_text(), "deleting...");
    }

    #[test]
    fn a_successful_delete_result_reloads_contents() {
        let mut app = app_with_one_content_entry();
        app.apply_operation_result(Ok(Response::Done));
        assert!(app.pending_contents.is_some());
        assert_ne!(app.status_text(), "deleting...");
    }

    #[test]
    fn a_failed_delete_result_surfaces_the_error() {
        let mut app = app_with_one_content_entry();
        app.apply_operation_result(Ok(Response::Error {
            message: "permission denied".to_owned(),
        }));
        assert_eq!(app.status_text(), "permission denied");
    }

    #[test]
    fn requesting_a_rename_prefills_the_input_with_the_current_name() {
        let mut app = app_with_one_content_entry();
        app.request_rename();
        assert_eq!(app.status_text(), "Rename to: doomed.txt_  (Enter/Esc)");
    }

    #[test]
    fn f2_prefills_the_rename_input_with_the_current_name() {
        // app.slint's key-scope wires F2 straight to `request_rename`, the
        // same App method the "r" hotkey calls in Mode::Normal.
        let mut app = app_with_one_content_entry();
        app.request_rename();
        assert_eq!(app.status_text(), "Rename to: doomed.txt_  (Enter/Esc)");
    }

    #[test]
    fn macos_return_in_normal_mode_prefills_the_rename_input() {
        let mut app = app_with_one_content_entry();
        app.handle_return_for_os("macos");
        assert_eq!(app.status_text(), "Rename to: doomed.txt_  (Enter/Esc)");
    }

    #[test]
    fn non_macos_return_in_normal_mode_is_a_no_op() {
        let mut app = app_with_one_content_entry();
        let before = app.status_text();

        app.handle_return_for_os("windows");
        assert_eq!(app.status_text(), before);

        app.handle_return_for_os("linux");
        assert_eq!(app.status_text(), before);
    }

    #[test]
    fn editing_the_rename_input_appends_and_backspaces() {
        let mut app = app_with_one_content_entry();
        app.request_rename();

        app.backspace();
        app.handle_key_text("!");

        assert_eq!(app.status_text(), "Rename to: doomed.tx!_  (Enter/Esc)");
    }

    #[test]
    fn returning_confirms_a_rename_and_sends_a_request() {
        let mut app = app_with_one_content_entry();
        app.request_rename();

        app.handle_return();

        assert!(app.pending_operation.is_some());
        assert_eq!(app.status_text(), "working...");
    }

    #[test]
    fn escaping_a_rename_input_cancels_without_a_request() {
        let mut app = app_with_one_content_entry();
        app.request_rename();

        app.cancel_pending();

        assert!(app.pending_operation.is_none());
        assert_ne!(app.status_text(), "Rename to: doomed.txt_  (Enter/Esc)");
    }

    /// A copy that loses its extension loses the plugin that made the
    /// original readable - the File pane cannot preview it and the listing
    /// cannot type it. The count goes in front of the extension, the way
    /// every file manager does it.
    #[test]
    fn a_copy_keeps_the_extension_that_makes_it_readable() {
        let taken = vec!["notes.txt".to_owned()];

        assert_eq!(super::dedup_name(&taken, "notes.txt"), "notes (2).txt");
    }

    /// Names that have no extension to sit in front of keep the old shape.
    #[test]
    fn a_name_with_nothing_to_protect_is_numbered_at_the_end() {
        let makefile = vec!["Makefile".to_owned()];
        assert_eq!(super::dedup_name(&makefile, "Makefile"), "Makefile (2)");

        let folder = vec!["src".to_owned()];
        assert_eq!(super::dedup_name(&folder, "src"), "src (2)");
    }

    /// A leading dot is the name, not a separator: `.gitignore` is not a
    /// file called nothing with a `gitignore` extension.
    #[test]
    fn a_dotfile_is_numbered_without_growing_a_stray_dot() {
        let taken = vec![".gitignore".to_owned()];

        assert_eq!(super::dedup_name(&taken, ".gitignore"), ".gitignore (2)");
    }

    /// Only the last dot separates, so a doubled extension keeps the half
    /// that names the type.
    #[test]
    fn only_the_last_dot_separates_the_extension() {
        let taken = vec!["archive.tar.gz".to_owned()];

        assert_eq!(
            super::dedup_name(&taken, "archive.tar.gz"),
            "archive.tar (2).gz"
        );
    }

    /// Counting past the first free name still keeps the extension.
    #[test]
    fn the_second_copy_of_a_file_is_numbered_three() {
        let taken = vec![
            "notes.txt".to_owned(),
            "notes (2).txt".to_owned(),
            "notes (3).txt".to_owned(),
        ];

        assert_eq!(super::dedup_name(&taken, "notes.txt"), "notes (4).txt");
    }

    /// The point of the change, said in the application's own terms: the
    /// copy is recognised by the same plugin as the original.
    #[test]
    fn a_pasted_copy_is_still_the_type_it_was_copied_from() {
        let taken = vec!["notes.txt".to_owned()];

        let copy = super::dedup_name(&taken, "notes.txt");

        let original = super::PRESENTATION_PLUGINS
            .iter()
            .find(|plugin| plugin.extensions().contains(&"txt"));
        assert!(
            std::path::Path::new(&copy)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("txt")),
            "the copy is named {copy}, which no longer says what it is"
        );
        assert!(
            original.is_some(),
            "the fixture only means something while a plugin claims .txt"
        );
    }

    #[test]
    fn requesting_a_copy_prefills_a_name_that_does_not_collide() {
        let mut app = app_with_one_content_entry();
        app.request_copy();
        // Before the extension, so the copy is still a text file. See
        // `a_copy_keeps_the_extension_that_makes_it_readable`.
        assert_eq!(app.status_text(), "Copy to: doomed (2).txt_  (Enter/Esc)");
    }

    #[test]
    fn returning_confirms_a_copy_and_sends_a_request() {
        let mut app = app_with_one_content_entry();
        app.request_copy();

        app.handle_return();

        assert!(app.pending_operation.is_some());
        assert_eq!(app.status_text(), "working...");
    }

    fn app_with_one_archive_entry() -> App {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("bundle.zip", false)]),
            }),
        );
        app
    }

    #[test]
    fn requesting_an_extract_prefills_the_archive_stem() {
        let mut app = app_with_one_archive_entry();
        app.request_extract();
        assert_eq!(app.status_text(), "Extract to: bundle_  (Enter/Esc)");
    }

    #[test]
    fn returning_confirms_an_extract_and_sends_a_request() {
        let mut app = app_with_one_archive_entry();
        app.request_extract();

        app.handle_return();

        assert!(app.pending_operation.is_some());
        assert_eq!(app.status_text(), "working...");
    }

    #[test]
    fn returning_with_an_emptied_rename_input_does_not_send_a_request() {
        let mut app = app_with_one_content_entry();
        app.request_rename();
        for _ in 0.."doomed.txt".len() {
            app.backspace();
        }

        app.handle_return();

        assert!(app.pending_operation.is_none());
    }

    #[test]
    fn typed_letters_are_appended_during_text_input() {
        let mut app = app_with_one_content_entry();
        app.request_rename();

        app.handle_key_text("x");

        assert_eq!(app.status_text(), "Rename to: doomed.txtx_  (Enter/Esc)");
    }

    #[test]
    fn arrow_keys_move_the_selection_within_the_focused_pane() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("a.txt", false), ("b.txt", false), ("c.txt", false)]),
            }),
        );
        app.select_content(0);

        app.move_selection(1);
        assert_eq!(app.content_selected(), 1);

        app.move_selection(1);
        app.move_selection(1);
        assert_eq!(app.content_selected(), 2, "stops at the last row");

        app.move_selection(-1);
        assert_eq!(app.content_selected(), 1);

        app.move_selection(-5);
        assert_eq!(app.content_selected(), 0, "stops at the first row");
    }

    #[test]
    fn arrow_keys_are_ignored_while_a_name_is_being_typed() {
        let mut app = app_with_one_content_entry();
        app.request_rename();

        app.move_selection(1);
        app.cycle_focus(1);

        assert_eq!(app.status_text(), "Rename to: doomed.txt_  (Enter/Esc)");
        assert_eq!(app.focus_index(), 0, "focus did not move either");
    }

    #[test]
    fn left_and_right_cycle_focus_through_the_three_panes() {
        let mut app = App::new(std::env::temp_dir());
        assert_eq!(app.focus_index(), 0);

        app.cycle_focus(1);
        assert_eq!(app.focus_index(), 1);

        app.cycle_focus(1);
        assert_eq!(app.focus_index(), 2);

        app.cycle_focus(1);
        assert_eq!(app.focus_index(), 0, "wraps past the last pane");

        app.cycle_focus(-1);
        assert_eq!(app.focus_index(), 2, "and wraps back the other way");

        app.cycle_focus(-1);
        assert_eq!(app.focus_index(), 1);
    }

    #[test]
    fn a_reload_after_an_operation_keeps_the_selection_on_its_entry() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("a.txt", false), ("b.txt", false)]),
            }),
        );
        app.select_content(1);
        app.request_copy();
        for _ in 0.."b.txt (2)".len() {
            app.backspace();
        }
        for c in "copy.txt".chars() {
            app.handle_key_text(&c.to_string());
        }
        app.handle_return();

        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("a.txt", false), ("b.txt", false), ("copy.txt", false)]),
            }),
        );

        assert_eq!(app.content_selected(), 2, "the copy, not the first row");
    }

    #[test]
    fn a_pending_operation_offers_a_prompt_for_the_row_it_applies_to() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        assert_eq!(app.prompt_text(), "", "nothing pending to begin with");
        assert_eq!(app.prompt_row(), -1);

        app.request_rename();
        assert_eq!(app.prompt_text(), "Rename to:  doomed.txt");
        assert_eq!(app.prompt_row(), 0);
        assert!(app.prompt_is_editable());

        app.cancel_pending();
        app.request_delete();
        assert_eq!(app.prompt_text(), "Delete doomed.txt?  (y / n)");
        assert_eq!(app.prompt_row(), 0);
        assert!(
            !app.prompt_is_editable(),
            "a y/n confirmation takes no typing, so it draws no caret"
        );

        app.decline_delete();
        assert_eq!(app.prompt_text(), "");
        assert_eq!(app.prompt_row(), -1);
    }

    #[test]
    fn focus_index_reflects_which_pane_was_last_interacted_with() {
        let mut app = app_with_one_content_entry();
        assert_eq!(app.focus_index(), 0); // App::new leaves focus on Folders.

        app.select_content(0);
        assert_eq!(app.focus_index(), 1);

        app.select_folder(0);
        assert_eq!(app.focus_index(), 0);
    }

    #[test]
    fn toggle_folder_flips_expanded_state() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("sub", true)]),
            }),
        );
        assert!(app.folder_rows()[0].expanded); // root starts expanded.

        app.toggle_folder(0);
        assert!(!app.folder_rows()[0].expanded);

        app.toggle_folder(0);
        assert!(app.folder_rows()[0].expanded);
    }

    #[test]
    fn selecting_an_out_of_range_content_row_is_a_no_op() {
        let mut app = app_with_one_content_entry();
        let before = app.content_selected();

        app.select_content(999);

        assert_eq!(app.content_selected(), before);
    }

    #[test]
    fn opening_a_file_row_does_not_drill_into_the_tree() {
        let mut app = app_with_one_content_entry();
        let folder_before = app.folder_selected();

        app.open_content(0); // "doomed.txt" is a file, not a directory.

        assert_eq!(app.folder_selected(), folder_before);
        assert!(!app.status_text().starts_with("loading"));
    }

    #[test]
    fn returning_with_an_emptied_copy_input_does_not_send_a_request() {
        let mut app = app_with_one_content_entry();
        app.request_copy();
        for _ in 0.."doomed.txt (2)".len() {
            app.backspace();
        }

        app.handle_return();

        assert!(app.pending_operation.is_none());
    }

    #[test]
    fn returning_with_an_emptied_extract_input_does_not_send_a_request() {
        let mut app = app_with_one_archive_entry();
        app.request_extract();
        for _ in 0.."bundle".len() {
            app.backspace();
        }

        app.handle_return();

        assert!(app.pending_operation.is_none());
    }

    #[test]
    fn cancel_pending_during_copy_input_returns_to_normal() {
        let mut app = app_with_one_content_entry();
        app.request_copy();

        app.cancel_pending();

        assert!(app.pending_operation.is_none());
        assert_ne!(app.status_text(), "Copy to: doomed.txt_  (Enter/Esc)");
    }

    #[test]
    fn cancel_pending_during_extract_input_returns_to_normal() {
        let mut app = app_with_one_archive_entry();
        app.request_extract();

        app.cancel_pending();

        assert!(app.pending_operation.is_none());
        assert_ne!(app.status_text(), "Extract to: bundle_  (Enter/Esc)");
    }

    #[test]
    fn an_unexpected_response_to_a_directory_listing_surfaces_a_message() {
        let mut app = App::new(std::env::temp_dir());

        app.apply_contents_result(
            &[],
            Ok(Response::FileView {
                plugin: "text".to_owned(),
                data: serde_json::json!({}),
                also: Vec::new(),
            }),
        );

        assert_eq!(app.status_text(), "expected a directory listing");
    }

    #[test]
    fn unrecognized_characters_are_ignored_in_normal_mode_and_during_delete_confirmation() {
        let mut app = app_with_one_content_entry();

        app.handle_key_text("q");
        assert!(app.pending_operation.is_none());
        assert!(
            !app.status_text().starts_with("Rename")
                && !app.status_text().starts_with("Copy")
                && !app.status_text().starts_with("Extract")
        );

        app.request_delete();
        app.handle_key_text("q");
        assert_eq!(app.status_text(), "Delete doomed.txt? y/n");
    }

    #[test]
    fn drilling_in_works_with_the_tree_above_it_collapsed() {
        // The reader tidied the tree away, which is the obvious thing to
        // do when the tree is not what they are working in. Return in the
        // listing then did nothing at all, and said nothing about it.
        let mut app = App::new(std::env::temp_dir().join("repos"));
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("src", true), ("notes.txt", false)]),
            }),
        );
        app.toggle_folder(0);
        assert!(!app.folder_rows()[0].expanded, "the tree is collapsed");

        app.select_content(0);
        app.open_content(0);

        assert_eq!(
            app.folder_rows().len(),
            2,
            "drilling in shows where the reader went: the tree follows              them rather than staying shut"
        );
        assert_eq!(app.folder_rows()[1].name, "src");
        assert_eq!(
            app.folder_selected(),
            1,
            "and the selected folder is the one just opened"
        );
    }

    #[test]
    fn collapsing_a_folder_does_not_leave_the_selection_past_the_end() {
        // A selection pointing past the last row selects nothing, and
        // every rule that reads it - which folder to list, which to drill
        // into - then quietly does nothing.
        let mut app = App::new(std::env::temp_dir().join("repos"));
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("src", true)]),
            }),
        );
        app.select_folder(1);
        assert_eq!(app.folder_selected(), 1, "the child is selected");

        app.toggle_folder(0);

        assert!(
            app.folder_selected() < app.folder_rows().len(),
            "the selection has to name a row that exists; it is {} of {}",
            app.folder_selected(),
            app.folder_rows().len()
        );
        assert_eq!(
            app.folder_selected(),
            0,
            "and the folder just collapsed is where a reader expects to be"
        );
    }

    #[test]
    fn folder_rows_carry_the_name_and_the_depth_the_pane_indents_by() {
        let mut app = App::new(std::env::temp_dir().join("repos"));
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("sub", true)]),
            }),
        );

        let rows = app.folder_rows();
        assert_eq!(rows.len(), 2);
        // The name alone: File Explorer's navigation pane draws a folder
        // icon, not a trailing separator, and the depth is a number the
        // pane indents by rather than spaces baked into the text.
        assert_eq!(rows[0].name, "repos");
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].name, "sub");
        assert_eq!(rows[1].depth, 1);
    }

    #[test]
    fn the_chevron_is_the_first_column_and_moves_right_with_the_depth() {
        // Padding 4, chevron 16, indent 16. Depth 0 owns 4..20, depth 1
        // owns 20..36 - the boundaries are what a misplaced click lands on.
        assert!(
            !chevron_hit(3.9, 0, 1.0),
            "left of the padding is not the chevron"
        );
        assert!(
            chevron_hit(4.0, 0, 1.0),
            "the chevron starts where the padding ends"
        );
        assert!(chevron_hit(19.9, 0, 1.0));
        assert!(!chevron_hit(20.0, 0, 1.0), "at 20 the icon has started");

        assert!(
            !chevron_hit(19.9, 1, 1.0),
            "a child's chevron is one indent in"
        );
        assert!(chevron_hit(20.0, 1, 1.0));
        assert!(chevron_hit(35.9, 1, 1.0));
        assert!(!chevron_hit(36.0, 1, 1.0));
    }

    /// #586: `app.slint` scales the padding, indent and chevron column by
    /// the zoom factor, so the same boundaries as
    /// [`the_chevron_is_the_first_column_and_moves_right_with_the_depth`]
    /// move with it rather than staying at their 100% pixels.
    #[test]
    fn the_chevron_hit_test_scales_with_zoom() {
        assert!(!chevron_hit(7.9, 0, 2.0), "left of the doubled padding");
        assert!(chevron_hit(8.0, 0, 2.0), "the doubled padding ends at 8");
        assert!(chevron_hit(39.9, 0, 2.0));
        assert!(!chevron_hit(40.0, 0, 2.0), "the doubled icon has started");
    }

    #[test]
    fn zooming_in_and_out_reports_the_new_level() {
        let mut app = App::new(std::env::temp_dir());
        assert_eq!(app.zoom_percent(), 100);

        app.zoom_in();
        assert_eq!(app.zoom_percent(), 110);
        assert_eq!(app.status_text(), "Zoom 110%");

        app.zoom_out();
        assert_eq!(app.zoom_percent(), 100);
        assert_eq!(app.status_text(), "Zoom 100%");
    }

    #[test]
    fn zooming_out_below_the_lowest_step_stays_there_and_says_nothing() {
        let mut app = App::new(std::env::temp_dir());
        for _ in 0..8 {
            app.zoom_out();
        }
        assert_eq!(app.zoom_percent(), 80);
        let status_at_the_floor = app.status_text();

        app.zoom_out();
        assert_eq!(app.zoom_percent(), 80);
        assert_eq!(
            app.status_text(),
            status_at_the_floor,
            "zooming out at the floor should not overwrite the status bar"
        );
    }

    #[test]
    fn resetting_the_zoom_returns_to_100_percent() {
        let mut app = App::new(std::env::temp_dir());
        app.zoom_in();
        app.zoom_in();
        assert_eq!(app.zoom_percent(), 125);

        app.zoom_reset();
        assert_eq!(app.zoom_percent(), 100);
        assert_eq!(app.status_text(), "Zoom 100%");
    }

    #[test]
    fn clicking_the_chevron_opens_the_row_and_clicking_the_name_selects_it() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("sub", true)]),
            }),
        );
        assert!(app.folder_rows()[0].expanded, "the root starts expanded");

        // On the chevron: closes it, and the selection does not move.
        app.select_folder(1);
        app.click_folder(0, 8.0);
        assert!(!app.folder_rows()[0].expanded);

        // On the name: selects the row and leaves it closed.
        app.click_folder(0, 60.0);
        assert!(!app.folder_rows()[0].expanded, "the name does not toggle");
        assert_eq!(app.folder_selected(), 0);
    }

    #[test]
    fn the_chevron_column_of_a_row_that_cannot_expand_just_selects_it() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("sub", true)]),
            }),
        );
        // "sub" has never been opened, so it has no chevron to click.
        assert!(!app.folder_rows()[1].expandable);

        app.click_folder(1, 24.0);

        assert_eq!(app.folder_selected(), 1);
        assert!(
            !app.folder_rows()[1].expanded,
            "an empty chevron column is part of the row, not a control"
        );
    }

    #[test]
    fn folder_rows_mark_collapsed_and_leaf_rows_distinctly() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("sub", true)]),
            }),
        );
        // The root is expanded by default but "sub"'s own children have
        // never been fetched, so it gets no chevron: the tree learns a
        // folder has subfolders by being opened.
        assert!(!app.folder_rows()[1].expandable);

        app.toggle_folder(0);
        assert_eq!(app.folder_rows().len(), 1);
        let root = &app.folder_rows()[0];
        assert!(root.expandable && !root.expanded);
    }

    #[test]
    fn requesting_a_new_folder_sends_a_create_directory_request_and_prefills_rename() {
        let mut app = App::new(std::env::temp_dir());

        app.request_new_folder();

        assert!(app.pending_operation.is_some());
        assert_eq!(app.status_text(), "working...");

        app.apply_operation_result(Ok(Response::Done));
        assert_eq!(app.status_text(), "Rename to: New folder_  (Enter/Esc)");
    }

    #[test]
    fn requesting_a_new_file_sends_a_create_file_request_and_prefills_rename() {
        let mut app = App::new(std::env::temp_dir());

        app.request_new_file();

        assert!(app.pending_operation.is_some());
        assert_eq!(app.status_text(), "working...");

        app.apply_operation_result(Ok(Response::Done));
        assert_eq!(app.status_text(), "Rename to: New file_  (Enter/Esc)");
    }

    #[test]
    fn a_failed_create_does_not_enter_rename_mode() {
        let mut app = App::new(std::env::temp_dir());

        app.request_new_folder();
        app.apply_operation_result(Ok(Response::Error {
            message: "already exists".to_owned(),
        }));

        assert_eq!(app.status_text(), "already exists");
    }

    #[test]
    fn a_new_folder_default_name_is_deduplicated_against_the_current_listing() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("New folder", true)]),
            }),
        );

        app.request_new_folder();
        app.apply_operation_result(Ok(Response::Done));

        assert_eq!(app.status_text(), "Rename to: New folder (2)_  (Enter/Esc)");
    }

    /// A listing holding one working copy and one ordinary folder, as the
    /// service reports it.
    fn workspace_entries() -> Vec<DirectoryEntry> {
        vec![
            DirectoryEntry {
                name: "explorer".to_owned(),
                is_dir: true,
                size: 0,
                modified: None,
                repository: Some(RepositoryInfo {
                    provider: Some("github.com".to_owned()),
                    branch: Some("main".to_owned()),
                    remote: Some("https://github.com/owner/explorer.git".to_owned()),
                    kind: RepositoryKind::Clone,
                    last_activity: None,
                    last_fetch: None,
                }),
            },
            DirectoryEntry {
                name: "scratch".to_owned(),
                is_dir: true,
                size: 0,
                modified: None,
                repository: None,
            },
        ]
    }

    #[test]
    fn a_working_copy_is_marked_and_names_its_provider() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: workspace_entries(),
            }),
        );

        let rows = app.content_rows();

        assert!(rows[0].is_repository, "a checkout should be marked as one");
        assert_eq!(rows[0].kind, "Repository · github.com");
        assert!(
            !rows[1].is_repository,
            "an ordinary folder is not a checkout"
        );
        assert_eq!(
            rows[1].kind, "File folder",
            "and it stays listed, plainly, rather than being hidden"
        );
    }

    #[test]
    fn a_working_copy_with_no_remote_is_still_marked() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![DirectoryEntry {
                    name: "local-only".to_owned(),
                    is_dir: true,
                    size: 0,
                    modified: None,
                    repository: Some(RepositoryInfo::default()),
                }],
            }),
        );

        let rows = app.content_rows();

        assert!(rows[0].is_repository);
        assert_eq!(rows[0].kind, "Git repository");
    }

    /// A working copy's icon carries the mark for each known provider, the
    /// generic mark for one with no remote (or a host this project does
    /// not name), and no mark at all for a plain folder (#579).
    #[test]
    fn the_mark_names_the_provider_or_falls_back_to_generic() {
        assert_eq!(
            repository_mark(true, Some("github.com")),
            Some(RepositoryMark::GitHub)
        );
        assert_eq!(
            repository_mark(true, Some("gitlab.com")),
            Some(RepositoryMark::GitLab)
        );
        assert_eq!(
            repository_mark(true, Some("bitbucket.org")),
            Some(RepositoryMark::Bitbucket)
        );
        assert_eq!(
            repository_mark(true, Some("dev.azure.com")),
            Some(RepositoryMark::AzureDevOps)
        );
        assert_eq!(
            repository_mark(true, Some("git.example.com")),
            Some(RepositoryMark::Generic),
            "a host this project does not name still reads as a working copy"
        );
        assert_eq!(
            repository_mark(true, None),
            Some(RepositoryMark::Generic),
            "a checkout with no remote is still a working copy"
        );
        assert_eq!(
            repository_mark(false, None),
            None,
            "a plain folder carries no mark"
        );
    }

    #[test]
    fn the_repos_directory_prompt_holds_what_it_was_seeded_with() {
        let mut app = App::new(std::env::temp_dir());

        app.begin_repos_root_edit(r"Z:\repos");

        assert!(app.choosing_repos_root());
        assert!(
            app.status_text().contains(r"Z:\repos"),
            "the prompt shows the suggestion: {}",
            app.status_text()
        );
        assert!(
            app.prompt_text().contains("Where are your repositories?"),
            "and asks in the pane, where a first run is looking: {}",
            app.prompt_text()
        );
    }

    #[test]
    fn cancelling_the_repos_directory_prompt_leaves_the_configuration_alone() {
        let mut app = App::new(std::env::temp_dir());
        app.begin_repos_root_edit("/somewhere");

        app.cancel_pending();

        assert!(!app.choosing_repos_root());
    }
    #[test]
    fn a_folder_shows_its_project_lines_below_its_own() {
        // Settled with the requester: a project view adds to a folder's
        // details, it does not replace them. A folder is still a folder.
        let mut app = App::new(std::env::temp_dir());
        let folder = serde_json::json!({ "entries": ["src"], "total": 1 });
        let project = plugin_api::FolderCore::view(
            &plugin_project_cargo::CargoProjectCore,
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/project-cargo"),
        )
        .unwrap();

        app.set_folder_view(
            "directory",
            folder.clone(),
            vec![protocol::PluginView {
                plugin: "project-cargo".to_owned(),
                data: project,
            }],
        );

        let text = app.file_text();

        assert!(
            text.starts_with(&super::present("directory", &folder).join("\n")),
            "the folder keeps its own lines, and keeps them first: {text}"
        );
        assert!(
            text.contains("Package: instrument-log 2.3.0"),
            "and the project lines follow: {text}"
        );
        assert!(
            text.contains("Edition: 2024"),
            "with everything the plugin found: {text}"
        );
    }

    #[test]
    fn a_file_view_is_unchanged_by_the_folder_machinery() {
        let mut app = App::new(std::env::temp_dir());

        app.set_file_view(
            "text",
            serde_json::json!({ "content": "hello", "truncated": false }),
        );

        assert_eq!(app.file_text(), "hello");
    }
    #[test]
    fn a_file_the_surface_cannot_serve_opens_in_the_plain_editor_and_says_so() {
        // Acceptance checks 2 and 4 of #480: neither input method
        // composition nor right-to-left text is handled, so rather than
        // letting either fail quietly the file goes to Slint's own box.
        let name_text = "let name = \"\u{5f20}\u{4e09}\";";
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("names.txt", false)]),
            }),
        );
        app.select_content(0);
        app.set_file_view("text", serde_json::json!({ "content": name_text }));
        app.begin_file_edit();

        assert!(app.editing_file(), "it still opens for editing");
        assert!(
            !app.editing_in_colour(),
            "but not in the surface that cannot place a caret in it"
        );
        assert!(
            app.status_text().contains("plain text"),
            "and the reader is told why rather than left wondering: {:?}",
            app.status_text()
        );
        assert!(
            app.edit_lines().is_empty(),
            "so the coloured surface is handed nothing to draw"
        );
    }

    #[test]
    fn an_ordinary_file_opens_in_the_coloured_surface() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("notes.txt", false)]),
            }),
        );
        app.select_content(0);
        app.set_file_view(
            "text",
            serde_json::json!({ "content": "one\ntwo\nthree\n" }),
        );
        app.begin_file_edit();

        assert!(app.editing_in_colour());
        assert_eq!(app.edit_lines().len(), 3, "a line per line");
        assert_eq!(app.edit_caret(), (0, 0));
        assert_eq!(app.edit_longest_line(), 5, "the width of \"three\"");
    }
    /// A clipboard holding nothing. These tests press no clipboard key,
    /// and reaching for the machine's would make them fight whatever
    /// else on it is using it.
    struct NoClipboard;

    impl crate::editor::Clipboard for NoClipboard {
        fn read(&mut self) -> Option<String> {
            None
        }
        fn write(&mut self, _text: &str) {}
    }

    /// A directory of this test's own, so two runs cannot tread on each
    /// other and neither treads on a fixture.
    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("repos-explorer-round-trip-{name}"));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a scratch directory");
        directory
    }

    /// An app looking at `directory`, with its one file open in the
    /// editor - through `service`, so the view is the real one.
    fn editing(directory: &Path, name: &str) -> App {
        let mut app = App::new(directory.to_path_buf());
        let entries = service::list_directory(directory).expect("the scratch directory lists");
        app.apply_contents_result(&[], Ok(Response::Directory { entries }));
        app.select_content(0);
        let view = service::view_file(&directory.join(name)).expect("the file opens");
        app.show_file_view(Some(view));
        app.begin_file_edit();
        app
    }

    /// A real file, opened through the service, typed into, and asked
    /// for its text back.
    ///
    /// The save itself crosses a process boundary on purpose - the
    /// service is the only thing that touches the filesystem - so this
    /// stops at the text the save would carry. What that text does when
    /// it gets there is `service`'s own
    /// `writing_a_file_puts_the_bytes_on_disk`.
    #[test]
    fn typing_into_a_real_file_gives_the_text_a_save_would_carry() {
        let directory = scratch("save");
        let path = directory.join("demo.rs");
        std::fs::write(&path, "fn main() {}\n").expect("the fixture is written");

        let mut app = editing(&directory, "demo.rs");
        assert!(
            app.editing_in_colour(),
            "an ASCII file gets the coloured surface"
        );

        let mut clipboard = NoClipboard;
        for letter in ["/", "/", " ", "h", "i"] {
            app.edit_key(&mut clipboard, letter, false, false, 20);
        }

        assert_eq!(
            app.edit_text(),
            "// hifn main() {}\n",
            "what was typed is there, where the caret was - which starts \
             at the beginning of the file"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("the file is still there"),
            "fn main() {}\n",
            "and nothing has reached the file yet, because nothing has \
             been saved"
        );
    }

    #[test]
    fn discarding_an_edit_leaves_the_file_exactly_as_it_was() {
        let directory = scratch("discard");
        let path = directory.join("demo.rs");
        let original = "fn main() {}\n";
        std::fs::write(&path, original).expect("the fixture is written");

        let mut app = editing(&directory, "demo.rs");
        let mut clipboard = NoClipboard;
        app.edit_key(&mut clipboard, "x", false, false, 20);
        assert!(app.edit_text().starts_with('x'), "it was typed into");

        app.cancel_file_edit();

        assert!(!app.editing_file(), "the editor closes");
        assert_eq!(
            app.status_text(),
            "edit discarded",
            "and says so, because this is the one place the text is lost"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("the file is still there"),
            original,
            "and the file is untouched"
        );
    }
    /// The rule that finds the file inside a Preview, on the shapes it
    /// meets. It is the only judgement in #492; everything else draws.
    #[test]
    fn the_file_is_found_at_the_end_of_a_preview_or_not_at_all() {
        let owned = |lines: &[&str]| -> Vec<String> {
            lines.iter().map(|line| (*line).to_owned()).collect()
        };

        // Summary, then the file.
        let preview = owned(&["functions: main", "fn main() {}", "// done"]);
        assert_eq!(
            file_starts_in_preview(&preview, "fn main() {}\n// done\n"),
            Some(1)
        );

        // All file and no summary.
        let preview = owned(&["fn main() {}"]);
        assert_eq!(file_starts_in_preview(&preview, "fn main() {}\n"), Some(0));

        // All summary and no file, which is what msbuild prints.
        let preview = owned(&["Software development kit: Microsoft.NET.Sdk"]);
        assert_eq!(file_starts_in_preview(&preview, "fn main() {}\n"), None);

        // A summary line repeating a line of the file changes nothing,
        // because the tail either matches whole or it does not.
        let preview = owned(&["fn main() {}", "functions: main", "fn main() {}"]);
        assert_eq!(
            file_starts_in_preview(&preview, "fn main() {}\n"),
            Some(2),
            "the last one is the file, not the first"
        );

        // A Preview shorter than the file cannot contain it.
        let preview = owned(&["fn main() {}"]);
        assert_eq!(file_starts_in_preview(&preview, "one\ntwo\nthree\n"), None);

        // Nothing to find.
        assert!(file_starts_in_preview(&preview, "").is_none());
    }

    #[test]
    fn a_previews_file_half_is_coloured_and_its_summary_is_not() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view(
            "rust",
            serde_json::json!({
                "content": "fn main() {}\n",
                "truncated": false,
                "functions": ["main"],
                "structs": [],
                "traits": [],
            }),
        );

        // The Preview is what a plugin shows first, so this is the view
        // already selected.
        let lines = app.file_lines();
        assert!(
            !lines.is_empty(),
            "the Preview should now be coloured at all"
        );

        // `functions: main` is a label this knows, so it is drawn as a
        // label and a name rather than as one run. A line it does not
        // know would be the single plain run this used to assert, and
        // `a_label_nobody_has_mapped_leaves_its_line_completely_alone`
        // is where that is checked.
        let summary = &lines[0];
        assert_eq!(summary[0].class, Class::Keyword, "the label");
        assert!(
            summary.iter().any(|run| run.class == Class::Function),
            "and the name it says is a function: {summary:?}"
        );

        let file = lines.last().expect("the file half is there");
        assert!(
            file.iter().any(|run| run.class == Class::Keyword),
            "and the file half has the colours the Text tab gives it: {file:?}"
        );
    }

    #[test]
    fn a_preview_that_is_all_summary_is_left_exactly_as_it_was() {
        // `msbuild` prints what it found and never the file, so there is
        // nothing here to colour and nothing should change.
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view(
            "msbuild",
            serde_json::json!({
                "content": "<Project><PropertyGroup/></Project>",
                "sdk": "Microsoft.NET.Sdk",
                "target_frameworks": ["net9.0"],
                "output_type": "Library",
                "properties": [],
                "packages": [],
                "project_references": [],
                "imports": [],
                "unversioned": [],
                "truncated": false,
            }),
        );

        assert!(
            app.file_lines().is_empty(),
            "no coloured lines means the pane draws the plain text it \
             always drew"
        );
    }
    /// What a coloured line is made of, as (class, text) pairs.
    fn runs_of(line: &[ColouredRun]) -> Vec<(Class, &str)> {
        line.iter()
            .map(|run| (run.class, run.text.as_str()))
            .collect()
    }

    #[test]
    fn a_summary_line_is_its_label_and_then_the_names_it_says_they_are() {
        let line = colour_summary_line("functions: index, spawn, main")
            .expect("a mapped label is coloured");
        assert_eq!(
            runs_of(&line),
            vec![
                (Class::Keyword, "functions"),
                (Class::Punctuation, ":"),
                (Class::Plain, " "),
                (Class::Function, "index"),
                (Class::Punctuation, ","),
                (Class::Plain, " "),
                (Class::Function, "spawn"),
                (Class::Punctuation, ","),
                (Class::Plain, " "),
                (Class::Function, "main"),
            ]
        );
    }

    #[test]
    fn a_type_label_gives_its_names_the_colour_a_type_has_in_the_file() {
        let line = colour_summary_line("structs: Entry, Progress").expect("mapped");
        assert!(
            runs_of(&line)
                .iter()
                .filter(|(class, _)| *class == Class::Type)
                .map(|(_, text)| *text)
                .eq(["Entry", "Progress"]),
            "so a type looks the same wherever a reader meets it: {:?}",
            runs_of(&line)
        );
    }

    /// The rule that made a whitelist necessary rather than tidy.
    ///
    /// Colouring whatever comes before the first colon was tried against
    /// every fixture in the repository. It produced labels like
    /// `10.0.0.10 - - [08/Sep/2026`, `{"id"` and `[0]` - an access log's
    /// address, a JSON key, an array index - because a plugin that
    /// pretty-prints its content rather than appending it has no summary
    /// to speak of, and nothing in the shape of a line tells the two
    /// apart.
    #[test]
    fn a_label_nobody_has_mapped_leaves_its_line_completely_alone() {
        for line in [
            "10.0.0.10 - - [08/Sep/2026:04:11:22 +0000] \"GET / HTTP/1.1\" 200",
            "{\"id\": 4, \"name\": \"ada\"}",
            "[0]: the first element",
            "Title: A Markdown fixture",
            "Comments: 14",
            "Produces: Library",
        ] {
            assert!(
                colour_summary_line(line).is_none(),
                "nothing in {line:?} is a label this knows, so it should be \
                 left as it is"
            );
        }
    }

    #[test]
    fn the_shapes_a_summary_line_comes_in() {
        // A value continued on an indented line below its label is not a
        // label of its own, whatever punctuation it holds.
        assert!(summary_label("  Microsoft.NET.Test.Sdk 17.13.0").is_none());
        assert!(summary_label("  nested: thing").is_none());

        // No colon at all.
        assert!(summary_label("Packages").is_none());

        // A colon inside a value: the first one is the label's.
        assert_eq!(
            summary_label("Remote: https://github.com/a/b.git"),
            Some(("Remote", " https://github.com/a/b.git"))
        );

        // A label with a count after it, which several plugins print.
        assert_eq!(summary_label("Packages (4):"), Some(("Packages (4)", "")));
        assert!(
            colour_summary_line("Packages (4):").is_none(),
            "and it is not one of the mapped labels, so it stays plain"
        );
    }

    #[test]
    fn one_name_and_no_comma_is_still_one_name() {
        let line = colour_summary_line("functions: main").expect("mapped");
        assert_eq!(
            runs_of(&line),
            vec![
                (Class::Keyword, "functions"),
                (Class::Punctuation, ":"),
                (Class::Plain, " "),
                (Class::Function, "main"),
            ]
        );
    }

    #[test]
    fn three_plugins_of_different_shapes_get_what_they_should() {
        // rust names its items, python names classes and functions, and
        // msbuild names none of them - it prints what it found about a
        // project, and every line of it should be left alone.
        let rust = colour_summary_line("structs: Entry, Progress");
        let python = colour_summary_line("classes: State, Task");
        let also_python = colour_summary_line("functions: retrying, draining");
        let msbuild = colour_summary_line("Software development kit: Microsoft.NET.Sdk");

        assert!(rust.is_some() && python.is_some() && also_python.is_some());
        assert!(
            msbuild.is_none(),
            "a project's summary is prose, not a list of names"
        );
    }
    /// An editor nobody can find is an editor nobody has.
    ///
    /// The only way in used to be a toolbar button called Edit, sitting
    /// beside Cut, Copy, Rename and Delete - where it reads as another
    /// thing done *to* a file - and greyed out on every fresh launch,
    /// because the first row of a repos listing is a folder.
    #[test]
    fn an_editable_file_offers_a_way_into_the_editor_in_the_pane_itself() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view(
            "text",
            serde_json::json!({ "content": "hello", "truncated": false }),
        );

        assert_eq!(
            app.file_tabs(),
            vec!["Preview", "Text", "Edit"],
            "the plugin's views, and then the pane's own"
        );
    }

    #[test]
    fn a_file_that_cannot_be_edited_offers_no_way_in_rather_than_a_dead_one() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view("image", serde_json::json!({ "width": 4 }));

        assert_eq!(
            app.file_tabs(),
            vec!["Preview"],
            "nothing to wonder about: the affordance is absent rather \
             than greyed"
        );
    }

    #[test]
    fn choosing_the_edit_tab_opens_the_editor() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view(
            "text",
            serde_json::json!({ "content": "hello", "truncated": false }),
        );
        assert!(!app.editing_file());

        let edit = app.file_tabs().len() - 1;
        app.select_file_tab(edit);

        assert!(app.editing_file(), "the same thing the toolbar button does");
    }

    #[test]
    fn the_editor_offers_only_itself_while_it_is_open() {
        // The plugin's views would be one stray click away from
        // discarding what somebody has typed. The way out is Save or
        // Escape, both of which say what they did.
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view(
            "text",
            serde_json::json!({ "content": "hello", "truncated": false }),
        );
        app.begin_file_edit();

        assert_eq!(app.file_tabs(), vec!["Editing"]);
        assert_eq!(app.file_tab_index(), 0);

        app.select_file_tab(0);
        assert!(app.editing_file(), "and choosing it again changes nothing");
    }

    #[test]
    fn leaving_the_editor_returns_to_the_view_that_was_showing() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view(
            "text",
            serde_json::json!({ "content": "hello", "truncated": false }),
        );
        app.select_file_tab(1);
        assert_eq!(app.file_tab_index(), 1, "the Text view");

        app.begin_file_edit();
        app.cancel_file_edit();

        assert_eq!(
            app.file_tabs(),
            vec!["Preview", "Text", "Edit"],
            "the views are offered again"
        );
        assert_eq!(
            app.file_tab_index(),
            1,
            "and the Text view is still the one"
        );
    }

    #[test]
    fn a_type_with_one_view_still_gets_its_edit_tab() {
        // The strip is hidden when there is only one thing to choose, so
        // without this the only way in would be hidden for exactly the
        // types that offer a single view.
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view(
            "dotenv",
            serde_json::json!({ "content": "A=1", "truncated": false, "keys": [] }),
        );

        let tabs = app.file_tabs();
        assert!(
            tabs.len() > 1 && tabs.last().map(String::as_str) == Some("Edit"),
            "a single-view type still shows a strip, because of the Edit \
             tab: {tabs:?}"
        );
    }
    /// An app editing a small text file, for the command row's tests.
    fn app_editing(text: &str) -> App {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.set_file_view(
            "text",
            serde_json::json!({ "content": text, "truncated": false }),
        );
        app.begin_file_edit();
        app
    }

    /// **The defect this work order is named for.**
    ///
    /// `Undo` in the window's toolbar sent `Request::Undo` to the
    /// service, which undoes the last *file operation* - a rename, a
    /// delete, a copy. While somebody was typing, the button marked Undo
    /// reached past their text and put back a file they had deleted
    /// earlier, and nothing told the two apart.
    #[test]
    fn the_windows_undo_undoes_typing_while_the_editor_is_open() {
        let mut app = app_editing("hello");
        let mut clipboard = NoClipboard;
        app.edit_key(&mut clipboard, "x", false, false, 20);
        assert_eq!(app.edit_text(), "xhello");

        app.undo();

        assert_eq!(app.edit_text(), "hello", "it undid the typing");
        assert_ne!(
            app.status_text(),
            "undoing...",
            "and did not send an undo to the filesystem"
        );
    }

    #[test]
    fn the_windows_undo_is_still_the_filesystems_when_no_editor_is_open() {
        let mut app = app_with_one_content_entry();
        app.undo();
        assert_eq!(
            app.status_text(),
            "undoing...",
            "outside the editor it is what it always was"
        );
    }

    #[test]
    fn the_row_says_whether_there_is_anything_to_save() {
        let mut app = app_editing("hello");
        assert!(!app.edit_modified(), "nothing typed yet");

        let mut clipboard = NoClipboard;
        app.edit_key(&mut clipboard, "x", false, false, 20);
        assert!(app.edit_modified());

        // Typed and taken out again is unchanged, which is what a reader
        // means by it - counting undo steps would say otherwise.
        app.undo();
        assert!(!app.edit_modified());
    }

    #[test]
    fn undo_and_redo_grey_when_there_is_nothing_to_do() {
        let mut app = app_editing("hello");
        assert!(!app.edit_can_undo(), "nothing typed, nothing to undo");
        assert!(!app.edit_can_redo());

        let mut clipboard = NoClipboard;
        app.edit_key(&mut clipboard, "x", false, false, 20);
        assert!(app.edit_can_undo());
        assert!(!app.edit_can_redo(), "nothing undone yet");

        app.edit_command(EditCommand::Undo, &mut clipboard);
        assert!(app.edit_can_redo(), "and now there is");
    }

    #[test]
    fn cut_and_copy_grey_until_something_is_selected() {
        let mut app = app_editing("hello world");
        assert!(!app.edit_has_selection());

        let mut clipboard = NoClipboard;
        app.edit_key(&mut clipboard, "a", false, true, 20);
        assert!(app.edit_has_selection(), "control-a selected it all");
    }

    #[test]
    fn each_command_in_the_row_does_what_its_shortcut_does() {
        // Routed through the same handler, so a button and its shortcut
        // cannot come to mean different things.
        let mut app = app_editing("alpha beta");
        let mut clipboard = Board::default();

        app.edit_key(&mut clipboard, "a", false, true, 20);
        app.edit_command(EditCommand::Copy, &mut clipboard);
        assert_eq!(clipboard.0.as_deref(), Some("alpha beta"), "Copy");

        app.edit_command(EditCommand::Cut, &mut clipboard);
        assert_eq!(app.edit_text(), "", "Cut");

        app.edit_command(EditCommand::Paste, &mut clipboard);
        assert_eq!(app.edit_text(), "alpha beta", "Paste");

        app.edit_command(EditCommand::Undo, &mut clipboard);
        assert_eq!(app.edit_text(), "", "Undo");

        app.edit_command(EditCommand::Redo, &mut clipboard);
        assert_eq!(app.edit_text(), "alpha beta", "Redo");
    }

    #[test]
    fn the_caret_is_reported_from_one_as_every_editor_shows_it() {
        let mut app = app_editing("one\ntwo\nthree");
        assert_eq!(app.edit_position(), (1, 1), "the very start is Ln 1, Col 1");

        let mut clipboard = NoClipboard;
        app.edit_key(
            &mut clipboard,
            &char::from(slint::platform::Key::DownArrow).to_string(),
            false,
            false,
            20,
        );
        app.edit_key(
            &mut clipboard,
            &char::from(slint::platform::Key::RightArrow).to_string(),
            false,
            false,
            20,
        );
        assert_eq!(app.edit_position(), (2, 2));
    }

    /// A clipboard that remembers, for the commands that use one.
    #[derive(Default)]
    struct Board(Option<String>);

    impl crate::editor::Clipboard for Board {
        fn read(&mut self) -> Option<String> {
            self.0.clone()
        }
        fn write(&mut self, text: &str) {
            self.0 = Some(text.to_owned());
        }
    }

    // ---- Ctrl+V into a typed prompt (#530) -----------------------------

    /// The case this exists for: a path copied from somewhere else goes into
    /// the address bar, and Enter then has it to go to.
    #[test]
    fn a_path_on_the_clipboard_pastes_into_the_address_bar() {
        let mut app = app_with_one_content_entry();
        app.begin_path_edit();
        // Clear what Ctrl+L prefilled, the way a reader selects and types
        // over it.
        while !app.path_input().is_empty() {
            app.backspace();
        }
        let mut clipboard = Board(Some("/somewhere/else".to_owned()));

        app.paste(&mut clipboard);

        assert_eq!(app.path_input(), "/somewhere/else");
    }

    /// Pasted text is added where typing would add it, after what is there.
    #[test]
    fn pasted_text_is_appended_to_what_the_prompt_already_holds() {
        let mut app = app_with_one_content_entry();
        app.request_rename();
        let before = app.status_text();
        let mut clipboard = Board(Some("-copy".to_owned()));

        app.paste(&mut clipboard);

        assert_ne!(app.status_text(), before);
        assert!(
            app.status_text().contains("doomed.txt-copy"),
            "the rename box should now read doomed.txt-copy: {}",
            app.status_text()
        );
    }

    /// A trailing newline - which a path copied from a terminal nearly
    /// always has - must not reach a prompt that confirms on Enter.
    #[test]
    fn only_the_first_line_of_the_clipboard_is_pasted() {
        for text in ["notes\n", "notes\r\n", "notes\nsecond line"] {
            let mut app = app_with_one_content_entry();
            app.request_rename();
            let mut clipboard = Board(Some(text.to_owned()));

            app.paste(&mut clipboard);

            assert!(
                app.status_text().contains("doomed.txtnotes_"),
                "{text:?} should paste as one line: {}",
                app.status_text()
            );
        }
    }

    /// An empty clipboard changes nothing, rather than failing.
    #[test]
    fn an_empty_clipboard_leaves_the_prompt_as_it_was() {
        let mut app = app_with_one_content_entry();
        app.request_rename();
        let before = app.status_text();

        app.paste(&mut Board(None));

        assert_eq!(app.status_text(), before);
    }

    /// With no prompt open, Paste is the file paste it always was - and it
    /// never reads the system clipboard's text to do it.
    #[test]
    fn with_no_prompt_open_paste_is_still_the_file_paste() {
        let mut app = app_with_one_content_entry();
        app.select_content(0);
        app.copy_to_clipboard();
        let mut clipboard = Board(Some("text that must not become a file".to_owned()));

        app.paste(&mut clipboard);

        assert!(
            app.pending_operation.is_some(),
            "the copied file should be on its way into the folder"
        );
    }

    /// Pasted text is held to the same rule as typed text: a tab or other
    /// control character on the clipboard does not reach a name.
    #[test]
    fn a_control_character_on_the_clipboard_is_not_pasted_into_a_prompt() {
        let mut app = app_with_one_content_entry();
        app.request_rename();

        app.paste(&mut Board(Some("a\tb\u{7}c".to_owned())));

        assert!(
            app.status_text().contains("doomed.txtabc_"),
            "only the characters should arrive: {:?}",
            app.status_text()
        );
    }

    // ---- Open on the web (#534) ----------------------------------------

    /// An application whose one row is a GitHub checkout on `branch`.
    fn app_with_a_checkout(branch: Option<&str>, remote: Option<&str>) -> App {
        let mut app = App::new(std::env::temp_dir().join("rse-notional-open-web"));
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![DirectoryEntry {
                    name: "name".to_owned(),
                    is_dir: true,
                    size: 0,
                    modified: None,
                    repository: Some(RepositoryInfo {
                        provider: remote.map(|_| "github.com".to_owned()),
                        branch: branch.map(str::to_owned),
                        remote: remote.map(str::to_owned),
                        kind: RepositoryKind::Clone,
                        last_activity: None,
                        last_fetch: None,
                    }),
                }],
            }),
        );
        app.select_content(0);
        app
    }

    #[test]
    fn opening_a_repository_on_the_web_hands_the_launcher_its_page_at_the_branch() {
        let mut app = app_with_a_checkout(Some("main"), Some("git@github.com:owner/name.git"));
        let mut launched = Vec::new();

        app.open_on_the_web(|address| {
            launched.push(address.to_owned());
            Ok(())
        });

        assert_eq!(launched, vec!["https://github.com/owner/name/tree/main"]);
        assert_eq!(
            app.web_provider().as_deref(),
            Some("github.com"),
            "and the menus are told where it goes"
        );
        assert!(
            app.status_text()
                .contains("opened https://github.com/owner/name")
        );
    }

    #[test]
    fn a_checkout_with_no_remote_has_nothing_to_open() {
        let mut app = app_with_a_checkout(Some("main"), None);
        let mut launched = false;

        app.open_on_the_web(|_| {
            launched = true;
            Ok(())
        });

        assert!(!launched);
        assert_eq!(app.web_provider(), None, "so the menus draw it refused");
    }

    /// A browser that will not start is said out loud, not swallowed.
    #[test]
    fn a_browser_that_will_not_start_is_reported() {
        let mut app = app_with_a_checkout(Some("main"), Some("https://github.com/owner/name"));

        app.open_on_the_web(|_| Err(std::io::Error::other("no browser here")));

        assert!(
            app.status_text()
                .contains("could not open a browser: no browser here"),
            "{}",
            app.status_text()
        );
    }

    // ---- Open a repository in the tools you work on it with (#581) ------

    #[test]
    fn opening_a_terminal_hands_the_launcher_the_selected_folders_path() {
        let mut app = app_with_a_checkout(Some("main"), Some("git@github.com:owner/name.git"));
        let mut launched = None;

        app.open_terminal_here(|command| {
            launched = Some(command.clone());
            Ok(())
        });

        let command = launched.expect("a folder was selected");
        // The folder reaches the terminal as its working directory, and on
        // some platforms as an argument too; a terminal that takes it only
        // one way still opens in the right place.
        assert!(
            command
                .current_dir
                .as_deref()
                .is_some_and(|dir| dir.contains("name"))
                || command.args.iter().any(|arg| arg.contains("name")),
            "the selected folder's path should reach the terminal: {command:?}"
        );
        assert!(app.status_text().contains("opened a terminal"));
    }

    #[test]
    fn opening_a_terminal_on_a_selected_file_does_nothing() {
        let mut app = App::new(std::env::temp_dir().join("rse-notional-terminal"));
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("readme.md", false)]),
            }),
        );
        app.select_content(0);
        let mut launched = false;

        app.open_terminal_here(|_| {
            launched = true;
            Ok(())
        });

        assert!(!launched, "a file has no terminal to open");
    }

    #[test]
    fn opening_a_terminal_at_the_folders_pane_selection_hands_off_its_path() {
        let mut app = App::new(std::env::temp_dir().join("rse-notional-folder-terminal"));
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("src", true)]),
            }),
        );
        app.select_folder(1);
        let mut launched = None;

        app.open_terminal_at_folder(|command| {
            launched = Some(command.clone());
            Ok(())
        });

        let command = launched.expect("the folders pane always has a folder selected");
        assert!(
            command
                .current_dir
                .as_deref()
                .is_some_and(|dir| dir.contains("src"))
                || command.args.iter().any(|arg| arg.contains("src")),
            "the right-clicked folder's path should reach the terminal: {command:?}"
        );
    }

    #[test]
    fn a_terminal_that_will_not_start_is_reported() {
        let mut app = app_with_a_checkout(Some("main"), Some("git@github.com:owner/name.git"));

        app.open_terminal_here(|_| Err(std::io::Error::other("no terminal here")));

        assert!(
            app.status_text()
                .contains("could not open a terminal: no terminal here"),
            "{}",
            app.status_text()
        );
    }

    #[test]
    fn opening_in_an_editor_is_refused_with_nothing_configured() {
        let mut app = app_with_a_checkout(Some("main"), Some("git@github.com:owner/name.git"));
        assert!(!app.can_open_selected_in_editor());
        let mut launched = false;

        app.open_selected_in_editor(|_| {
            launched = true;
            Ok(())
        });

        assert!(!launched, "there is nothing to open it with");
    }

    #[test]
    fn a_configured_editor_is_handed_the_selected_folder() {
        let mut app = app_with_a_checkout(Some("main"), Some("git@github.com:owner/name.git"));
        app.set_editor(Some("subl".to_owned()), false);
        assert!(app.can_open_selected_in_editor());
        let mut launched = None;

        app.open_selected_in_editor(|command| {
            launched = Some(command.clone());
            Ok(())
        });

        let command = launched.expect("an editor was configured");
        assert_eq!(command.program, "subl");
    }

    #[test]
    fn visual_studio_code_is_used_when_nothing_is_configured_but_it_is_on_the_path() {
        let mut app = app_with_a_checkout(Some("main"), Some("git@github.com:owner/name.git"));
        app.set_editor(None, true);
        let mut launched = None;

        app.open_selected_in_editor(|command| {
            launched = Some(command.clone());
            Ok(())
        });

        assert_eq!(launched.expect("code is on the PATH").program, "code");
    }

    #[test]
    fn a_folder_with_no_remote_has_no_address_to_copy() {
        let mut app = app_with_a_checkout(Some("main"), None);
        assert!(!app.can_copy_selected_remote_address());
        let mut copied = None;

        app.copy_selected_remote_address(|text| copied = Some(text.to_owned()));

        assert_eq!(copied, None);
    }

    #[test]
    fn a_working_copys_remote_address_is_copied_as_the_checkout_wrote_it() {
        let mut app = app_with_a_checkout(Some("main"), Some("git@github.com:owner/name.git"));
        assert!(app.can_copy_selected_remote_address());
        let mut copied = None;

        app.copy_selected_remote_address(|text| copied = Some(text.to_owned()));

        assert_eq!(copied.as_deref(), Some("git@github.com:owner/name.git"));
    }

    #[test]
    fn the_folders_pane_selections_remote_address_is_copied_too() {
        let mut app = App::new(std::env::temp_dir().join("rse-notional-folder-remote"));
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![DirectoryEntry {
                    name: "name".to_owned(),
                    is_dir: true,
                    size: 0,
                    modified: None,
                    repository: Some(RepositoryInfo {
                        provider: Some("github.com".to_owned()),
                        branch: Some("main".to_owned()),
                        remote: Some("git@github.com:owner/name.git".to_owned()),
                        kind: RepositoryKind::Clone,
                        last_activity: None,
                        last_fetch: None,
                    }),
                }],
            }),
        );
        app.select_folder(1);
        assert!(app.can_copy_folder_remote_address());
        let mut copied = None;

        app.copy_folder_remote_address(|text| copied = Some(text.to_owned()));

        assert_eq!(copied.as_deref(), Some("git@github.com:owner/name.git"));
    }

    #[test]
    fn copying_the_selected_folders_path_copies_its_full_path() {
        let mut app = app_with_a_checkout(Some("main"), Some("git@github.com:owner/name.git"));
        let mut copied = None;

        app.copy_selected_path(|text| copied = Some(text.to_owned()));

        let copied = copied.expect("a folder was selected");
        assert!(copied.ends_with("name"), "{copied}");
    }

    #[test]
    fn showing_the_selected_folder_in_the_file_manager_hands_off_its_path() {
        let mut app = app_with_a_checkout(Some("main"), Some("git@github.com:owner/name.git"));
        let mut launched = None;

        app.show_selected_in_file_manager(|command| {
            launched = Some(command.clone());
            Ok(())
        });

        // What each platform's command looks like is `launch`'s own unit
        // tests (`launch::tests`); this only proves the join - that the
        // selected folder's path reaches it at all.
        let command = launched.expect("a folder was selected");
        assert!(!command.program.is_empty());
        assert!(app.status_text().contains("opened the file manager"));
    }

    // ---- finding a name across every repository (#536) ------------------

    fn found(path: &str, is_dir: bool, repository: Option<&str>) -> protocol::NameMatch {
        protocol::NameMatch {
            path: path.to_owned(),
            is_dir,
            repository: repository.map(str::to_owned),
        }
    }

    /// An application showing three results for "notes".
    fn app_showing_results() -> App {
        let mut app = app_with_one_content_entry();
        app.apply_find_result_for_test(
            "notes",
            Response::Names {
                root: "/repos".to_owned(),
                matches: vec![
                    found("alpha/docs/notes.md", false, Some("alpha")),
                    found("beta/notes.txt", false, Some("beta")),
                    found("loose/notes", true, None),
                ],
                cut_short: false,
            },
        );
        app
    }

    #[test]
    fn ctrl_shift_f_opens_the_address_bar_as_a_find_prompt() {
        let mut app = app_with_one_content_entry();

        app.begin_find();
        app.type_char("n");
        app.type_char("o");

        assert!(app.editing_path(), "the address bar is the prompt");
        assert_eq!(app.path_input(), "Find: no");
        assert!(app.status_text().contains("Find in every repository: no"));
    }

    #[test]
    fn enter_in_the_find_prompt_asks_the_service_and_closes_the_prompt() {
        let mut app = app_with_one_content_entry();
        app.begin_find();
        for c in "notes".chars() {
            app.type_char(&c.to_string());
        }

        app.handle_return();

        assert!(app.is_busy(), "a search is on its way");
        assert!(!app.editing_path(), "and the prompt has gone");
    }

    #[test]
    fn results_are_drawn_as_their_path_within_their_repository() {
        let app = app_showing_results();

        let rows = app.content_rows();
        assert!(app.showing_found());
        assert_eq!(
            rows.iter()
                .map(|row| (row.name.as_str(), row.kind.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("docs/notes.md", "alpha"),
                ("notes.txt", "beta"),
                ("loose/notes/", "-"),
            ],
            "the path within each repository is what tells one result from the next"
        );
        assert!(app.status_text().contains("3 named like \"notes\""));
    }

    /// A result lives in some other folder, so every command that joins a
    /// row's name onto the browsed folder is refused while results show.
    #[test]
    fn nothing_acts_on_a_result_as_if_it_were_in_the_browsed_folder() {
        let mut app = app_showing_results();

        app.request_delete();
        app.request_rename();
        app.request_copy();
        app.copy_to_clipboard();
        app.select_all();
        app.sort_by_column(1);
        app.type_ahead("b");

        assert!(
            !app.status_text().contains("Delete"),
            "{}",
            app.status_text()
        );
        assert_eq!(app.prompt_row(), -1, "no prompt was opened over a result");
        assert_eq!(
            app.selected_count(),
            1,
            "no multiple selection over results"
        );
        assert_eq!(app.content_selected(), 0, "type-ahead did not move");
    }

    #[test]
    fn the_arrows_and_home_and_end_walk_the_results() {
        let mut app = app_showing_results();

        app.move_selection(1);
        assert_eq!(app.content_selected(), 1);
        app.select_edge(true);
        assert_eq!(app.content_selected(), 2);
        app.move_selection(5);
        assert_eq!(app.content_selected(), 2, "stops at the last result");
    }

    #[test]
    fn escape_puts_the_listing_back_where_the_reader_left_it() {
        let mut app = app_showing_results();

        app.cancel_pending();

        assert!(!app.showing_found());
        assert_eq!(app.content_rows().len(), 1);
        assert_eq!(app.content_rows()[0].name, "doomed.txt");
    }

    #[test]
    fn opening_a_result_goes_to_its_folder_with_it_selected() {
        let mut app = app_showing_results();
        app.move_selection(1);

        app.activate_selection();

        assert!(!app.showing_found(), "the listing is back");
        assert_eq!(
            PathBuf::from(app.current_path()),
            PathBuf::from("/repos").join("beta")
        );
    }

    #[test]
    fn a_search_that_was_cut_short_says_so() {
        let mut app = app_with_one_content_entry();
        app.apply_find_result_for_test(
            "a",
            Response::Names {
                root: "/repos".to_owned(),
                matches: vec![found("a", false, None)],
                cut_short: true,
            },
        );

        assert!(
            app.status_text().contains("there are more"),
            "{}",
            app.status_text()
        );
    }

    #[test]
    fn a_search_that_finds_nothing_says_so_and_a_failure_is_reported() {
        let mut app = app_with_one_content_entry();
        app.apply_find_result_for_test(
            "zzz",
            Response::Names {
                root: "/repos".to_owned(),
                matches: Vec::new(),
                cut_short: false,
            },
        );
        assert!(app.status_text().contains("nothing named like \"zzz\""));

        let mut app = app_with_one_content_entry();
        app.apply_find_result_for_test(
            "x",
            Response::Error {
                message: "no Repos Directory is configured to search".to_owned(),
            },
        );
        assert!(!app.showing_found());
        assert!(
            app.status_text()
                .contains("no Repos Directory is configured")
        );
    }

    // ---- branch and uncommitted changes on every repository row (#535) ----

    /// An application listing `names`, each a checkout on branch `main`, and
    /// `plain.txt`, which is not.
    fn app_listing_checkouts(names: &[&str]) -> App {
        let mut app = App::new(std::env::temp_dir());
        let mut listed: Vec<DirectoryEntry> = names
            .iter()
            .map(|name| DirectoryEntry {
                name: (*name).to_owned(),
                is_dir: true,
                size: 0,
                modified: None,
                repository: Some(RepositoryInfo {
                    provider: None,
                    branch: Some("main".to_owned()),
                    remote: None,
                    kind: RepositoryKind::Clone,
                    last_activity: None,
                    last_fetch: None,
                }),
            })
            .collect();
        listed.push(DirectoryEntry {
            name: "plain.txt".to_owned(),
            is_dir: false,
            size: 1,
            modified: None,
            repository: None,
        });
        app.apply_contents_result(&[], Ok(Response::Directory { entries: listed }));
        app
    }

    fn marker_of(app: &App, name: &str) -> String {
        app.content_rows()
            .into_iter()
            .find(|row| row.name.trim_end_matches('/') == name)
            .map_or_else(|| panic!("no row named {name}"), |row| row.marker)
    }

    fn working_tree(changed: usize, partial: bool) -> Response {
        Response::WorkingTree {
            path: String::new(),
            status: Some(protocol::WorkingTreeSummary {
                changed,
                partial,
                summary: String::new(),
            }),
        }
    }

    #[test]
    fn a_listing_draws_branches_before_any_status_is_known() {
        let app = app_listing_checkouts(&["alpha", "beta"]);

        let rows = app.content_rows();
        assert_eq!(rows[0].branch, "main");
        assert_eq!(rows[0].marker, NOT_KNOWN_YET_MARKER, "unknown, not clean");
        assert_eq!(rows[2].branch, "", "a plain file has no branch");
        assert_eq!(rows[2].marker, "");
        assert!(
            app.status_text().contains("2 repositories (2 not known)"),
            "{}",
            app.status_text()
        );
        assert_eq!(app.status_changed_label(), "0 with uncommitted changes");
    }

    #[test]
    fn each_answer_marks_its_own_row() {
        let mut app = app_listing_checkouts(&["alpha", "beta", "gamma", "delta"]);
        app.ask_for_statuses(0..4);

        app.apply_status_result_for_test("alpha", working_tree(0, false));
        app.apply_status_result_for_test("beta", working_tree(2, false));
        app.apply_status_result_for_test(
            "gamma",
            Response::WorkingTree {
                path: String::new(),
                status: None,
            },
        );
        app.apply_status_result_for_test("delta", working_tree(0, true));

        assert_eq!(marker_of(&app, "alpha"), "", "no changes");
        assert_eq!(marker_of(&app, "beta"), CHANGED_MARKER);
        assert_eq!(
            marker_of(&app, "gamma"),
            CANNOT_TELL_MARKER,
            "unreadable is not clean"
        );
        assert_eq!(
            marker_of(&app, "delta"),
            CANNOT_TELL_MARKER,
            "a count that stopped short without a change cannot say clean"
        );
        assert!(
            app.status_text().contains("4 repositories (2 not known)"),
            "{}",
            app.status_text()
        );
        assert_eq!(app.status_changed_label(), "1 with uncommitted changes");
    }

    // ---- a last fetch too old to trust (#589) ---------------------------

    #[test]
    fn a_fetch_thirty_one_days_old_is_stale_but_twenty_nine_is_not() {
        const DAY: u64 = 24 * 60 * 60;
        let now = 1_000_000_000;
        assert!(
            fetch_is_stale(Some(now - 31 * DAY), true, now),
            "31 days old is past the threshold"
        );
        assert!(
            !fetch_is_stale(Some(now - 29 * DAY), true, now),
            "29 days old is still within it"
        );
    }

    #[test]
    fn a_remote_never_fetched_is_stale_and_no_remote_never_is() {
        let now = 1_000_000_000;
        assert!(
            fetch_is_stale(None, true, now),
            "a remote configured but never fetched cannot be trusted"
        );
        assert!(
            !fetch_is_stale(None, false, now),
            "nothing to fetch means nothing to call stale"
        );
        assert!(
            !fetch_is_stale(Some(now - 1_000 * 24 * 60 * 60), false, now),
            "a repository with no remote is never stale, however old the field"
        );
    }

    /// An application listing one checkout with the given `remote` and
    /// `last_fetch`, for the stale-marker tests below.
    fn app_listing_one_checkout(remote: Option<&str>, last_fetch: Option<u64>) -> App {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![DirectoryEntry {
                    name: "widgets".to_owned(),
                    is_dir: true,
                    size: 0,
                    modified: None,
                    repository: Some(RepositoryInfo {
                        provider: None,
                        branch: Some("main".to_owned()),
                        remote: remote.map(str::to_owned),
                        kind: RepositoryKind::Clone,
                        last_activity: None,
                        last_fetch,
                    }),
                }],
            }),
        );
        app
    }

    #[test]
    fn a_stale_repositorys_row_carries_the_marker_and_tooltip() {
        let now = now_epoch_seconds();
        let app = app_listing_one_checkout(
            Some("https://github.com/owner/widgets.git"),
            Some(now - 61 * 24 * 60 * 60),
        );

        let row = app
            .content_rows()
            .into_iter()
            .next()
            .expect("one repository row");
        assert_eq!(row.stale_marker, STALE_FETCH_MARKER);
        assert_eq!(
            row.stale_tooltip,
            "Last fetched 61 days ago; ahead and behind counts may be out of date"
        );
        assert!(
            app.status_text().contains(", 1 not fetched in 30 days"),
            "{}",
            app.status_text()
        );
    }

    #[test]
    fn a_never_fetched_remote_reads_never_fetched_in_its_tooltip() {
        let app = app_listing_one_checkout(Some("https://github.com/owner/widgets.git"), None);

        let row = app
            .content_rows()
            .into_iter()
            .next()
            .expect("one repository row");
        assert_eq!(row.stale_marker, STALE_FETCH_MARKER);
        assert_eq!(row.stale_tooltip, "Never fetched");
    }

    #[test]
    fn a_freshly_fetched_or_remote_less_repository_carries_no_stale_marker() {
        let now = now_epoch_seconds();
        let fresh = app_listing_one_checkout(
            Some("https://github.com/owner/widgets.git"),
            Some(now - 5 * 24 * 60 * 60),
        );
        let row = fresh
            .content_rows()
            .into_iter()
            .next()
            .expect("one repository row");
        assert_eq!(row.stale_marker, "", "a recent fetch is not stale");
        assert!(!fresh.status_text().contains("not fetched in 30 days"));

        let no_remote = app_listing_one_checkout(None, None);
        let row = no_remote
            .content_rows()
            .into_iter()
            .next()
            .expect("one repository row");
        assert_eq!(
            row.stale_marker, "",
            "nothing to fetch means nothing to mark stale"
        );
    }

    // ---- status bar filters (#582) ----

    #[test]
    fn contents_summary_omits_size_for_a_folders_only_listing() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("alpha", true), ("beta", true)]),
            }),
        );
        let status = app.status_text();
        assert!(status.starts_with("2 items"), "{status}");
        assert!(!status.contains("0 B"), "{status}");
    }

    #[test]
    fn typing_into_the_filter_narrows_the_listing_by_name() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[
                    ("alpha.txt", false),
                    ("beta.txt", false),
                    ("gamma.txt", false),
                ]),
            }),
        );

        app.begin_filter();
        assert!(app.filter_focused());
        // Case-insensitive, per #582.
        app.handle_key_text("A");
        app.handle_key_text("L");

        let names: Vec<_> = app.content_rows().into_iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["alpha.txt".to_owned()]);

        app.clear_filters();
        assert_eq!(app.content_rows().len(), 3);
    }

    #[test]
    fn the_switchers_own_order_follows_the_way_contents_is_sorted() {
        // #590 asks for "Contents' current sort order". Sorting the pane
        // the other way round turns the switcher's list round with it; the
        // review of #634 found it always alphabetical.
        let mut app = app_listing_checkouts(&["alpha", "beta", "gamma"]);

        app.begin_switcher();
        let ascending: Vec<String> = app
            .switcher_rows()
            .into_iter()
            .map(|row| row.name)
            .collect();
        assert_eq!(ascending, vec!["alpha", "beta", "gamma"]);
        app.cancel_pending();

        // Column 0 is Name: clicking it again reverses the direction.
        app.sort_by_column(0);
        app.begin_switcher();
        let descending: Vec<String> = app
            .switcher_rows()
            .into_iter()
            .map(|row| row.name)
            .collect();

        assert_eq!(
            descending,
            vec!["gamma", "beta", "alpha"],
            "the switcher should list them the way the pane is listing them"
        );
    }

    #[test]
    fn opening_the_switcher_lists_every_repository_in_name_order() {
        let mut app = app_listing_checkouts(&["zulu", "alpha", "mike"]);

        app.begin_switcher();

        assert!(app.switcher_open());
        let names: Vec<_> = app.switcher_rows().into_iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["alpha", "mike", "zulu"]);
    }

    #[test]
    fn typing_narrows_to_fuzzy_matches_and_resets_the_highlight() {
        let mut app = app_listing_checkouts(&["TankSwarmCode", "other-repo"]);

        app.begin_switcher();
        app.handle_key_text("t");
        app.handle_key_text("s");
        app.handle_key_text("c");

        let rows = app.switcher_rows();
        assert_eq!(rows[0].name, "TankSwarmCode");
        assert!(rows[0].selected, "the first match is highlighted");
    }

    #[test]
    fn typing_into_the_switcher_sends_no_directory_or_find_request() {
        let mut app = app_listing_checkouts(&["alpha", "beta"]);
        // Discard the listing `App::new` already asked for at startup,
        // which has nothing to do with the switcher.
        app.pending_contents = None;

        app.begin_switcher();
        app.handle_key_text("a");
        app.handle_key_text("l");
        app.backspace();

        assert!(
            app.pending_contents.is_none(),
            "a typed query must not walk the filesystem (#590)"
        );
        assert!(app.pending_find.is_none());
    }

    #[test]
    fn escape_closes_the_switcher_without_changing_the_selection() {
        let mut app = app_listing_checkouts(&["alpha", "beta"]);
        app.select_content(1);

        app.begin_switcher();
        app.handle_key_text("a");
        app.cancel_pending();

        assert!(!app.switcher_open());
        assert_eq!(app.content_selected(), 1);
    }

    #[test]
    fn return_on_the_highlighted_match_goes_to_it_once_the_listing_lands() {
        let mut app = app_listing_checkouts(&["alpha", "beta", "gamma"]);
        app.begin_switcher();
        app.handle_key_text("b");

        app.handle_return();

        assert!(!app.switcher_open(), "Return closes the switcher");
        assert_eq!(
            app.folder_selected(),
            0,
            "the repository's parent, the Repos Directory, is selected in Folders"
        );

        // The fresh listing `select_folder` asked for, landing the way it
        // would after a real click on the Repos Directory's row.
        app.apply_contents_result_for_test(
            &[],
            Response::Directory {
                entries: entries(&[
                    ("alpha", true),
                    ("beta", true),
                    ("gamma", true),
                    ("plain.txt", false),
                ]),
            },
        );
        let selected = app.content_rows()[app.content_selected()].name.clone();
        assert_eq!(selected.trim_end_matches('/'), "beta");
    }

    #[test]
    fn clicking_the_changed_count_narrows_to_repositories_with_uncommitted_changes() {
        let mut app = app_listing_checkouts(&["alpha", "beta", "gamma"]);
        app.ask_for_statuses(0..4);
        app.apply_status_result_for_test("alpha", working_tree(0, false));
        app.apply_status_result_for_test("beta", working_tree(2, false));
        app.apply_status_result_for_test("gamma", working_tree(0, false));

        app.filter_to_changed();

        let names: Vec<_> = app.content_rows().into_iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["beta/".to_owned()]);
        assert!(
            app.status_text()
                .contains("showing 1 with uncommitted changes"),
            "{}",
            app.status_text()
        );

        app.clear_filters();
        assert_eq!(app.content_rows().len(), 4, "the plain file is back too");
    }

    #[test]
    fn the_two_filters_combine() {
        let mut app = app_listing_checkouts(&["alpha", "alberta", "beta"]);
        app.ask_for_statuses(0..4);
        app.apply_status_result_for_test("alpha", working_tree(1, false));
        app.apply_status_result_for_test("alberta", working_tree(0, false));
        app.apply_status_result_for_test("beta", working_tree(1, false));

        app.filter_to_changed();
        app.begin_filter();
        app.handle_key_text("a");
        app.handle_key_text("l");

        // "alberta" matches the typed text but has nothing changed, and
        // "beta" has changed but does not match the typed text - only
        // "alpha" passes both.
        let names: Vec<_> = app.content_rows().into_iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["alpha/".to_owned()]);
    }

    #[test]
    fn escape_in_contents_clears_an_active_filter() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("alpha.txt", false), ("test.txt", false)]),
            }),
        );
        app.begin_filter();
        app.handle_key_text("a");
        assert_eq!(app.content_rows().len(), 1);

        app.cancel_pending();

        assert_eq!(app.content_rows().len(), 2);
        assert!(!app.filter_focused());
    }

    /// An operation that reloads the folder somebody is looking at keeps
    /// their filter. Delete and undo were left out when the other four
    /// were wired up (#582's review of #623): filtering to the changed
    /// repositories and deleting one of them threw the filter away and
    /// put every row back, which is the opposite of what the rest of the
    /// operations do.
    #[test]
    fn deleting_a_row_keeps_the_filter_it_was_deleted_from() {
        let mut app = App::new(std::env::temp_dir());
        let listing = || {
            Ok(Response::Directory {
                entries: entries(&[("alpha.txt", false), ("test.txt", false)]),
            })
        };
        app.apply_contents_result(&[], listing());
        app.begin_filter();
        app.handle_key_text("a");
        assert_eq!(app.content_rows().len(), 1);

        app.request_delete();
        app.confirm_delete();
        app.apply_contents_result(&[], listing());

        assert_eq!(
            app.content_rows().len(),
            1,
            "deleting a row is the same folder reloaded, so the filter stays"
        );
    }

    #[test]
    fn undoing_an_operation_keeps_the_filter() {
        let mut app = App::new(std::env::temp_dir());
        let listing = || {
            Ok(Response::Directory {
                entries: entries(&[("alpha.txt", false), ("test.txt", false)]),
            })
        };
        app.apply_contents_result(&[], listing());
        app.begin_filter();
        app.handle_key_text("a");

        app.undo();
        app.apply_contents_result(&[], listing());

        assert_eq!(
            app.content_rows().len(),
            1,
            "undo puts something back into the same folder, so the filter stays"
        );
    }

    #[test]
    fn navigating_to_a_different_folder_drops_the_filter() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("alpha.txt", false), ("test.txt", false)]),
            }),
        );
        app.begin_filter();
        app.handle_key_text("a");
        assert_eq!(app.content_rows().len(), 1);

        // A real navigation, unlike a refresh, is not the same folder
        // reloaded (#582).
        app.navigate_to_parent();
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("alpha.txt", false), ("test.txt", false)]),
            }),
        );

        assert_eq!(app.content_rows().len(), 2);
        assert!(!app.filter_focused());
    }

    /// What the row named `name` draws its marker's tooltip and warning
    /// flag as - the words and the colour a mouse or screen reader user
    /// gets when the glyph alone would tell them nothing (#574).
    fn marker_tooltip_and_warning_of(app: &App, name: &str) -> (String, bool) {
        app.content_rows()
            .into_iter()
            .find(|row| row.name.trim_end_matches('/') == name)
            .map_or_else(
                || panic!("no row named {name}"),
                |row| (row.marker_tooltip, row.marker_warning),
            )
    }

    #[test]
    fn each_marker_carries_the_words_it_means_and_only_the_changed_one_warns() {
        let mut app = app_listing_checkouts(&["alpha", "beta", "gamma", "delta"]);
        app.ask_for_statuses(0..4);

        app.apply_status_result_for_test("alpha", working_tree(0, false));
        app.apply_status_result_for_test("beta", working_tree(2, false));
        app.apply_status_result_for_test(
            "gamma",
            Response::WorkingTree {
                path: String::new(),
                status: None,
            },
        );
        // "delta" is left waiting, so it still carries the not-known-yet
        // marker.

        assert_eq!(
            marker_tooltip_and_warning_of(&app, "alpha"),
            (String::new(), false),
            "no marker, no words and no warning"
        );
        assert_eq!(
            marker_tooltip_and_warning_of(&app, "beta"),
            ("Uncommitted changes to tracked files".to_owned(), true)
        );
        assert_eq!(
            marker_tooltip_and_warning_of(&app, "gamma"),
            ("Could not tell whether there are changes".to_owned(), false)
        );
        assert_eq!(
            marker_tooltip_and_warning_of(&app, "delta"),
            ("Checking for changes…".to_owned(), false)
        );
        assert_eq!(
            marker_tooltip_and_warning_of(&app, "plain.txt"),
            (String::new(), false),
            "a plain file has no marker to explain"
        );
    }

    #[test]
    fn a_row_detached_from_any_branch_says_detached() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![DirectoryEntry {
                    name: "loose".to_owned(),
                    is_dir: true,
                    size: 0,
                    modified: None,
                    repository: Some(RepositoryInfo {
                        provider: None,
                        branch: None,
                        remote: None,
                        kind: RepositoryKind::Clone,
                        last_activity: None,
                        last_fetch: None,
                    }),
                }],
            }),
        );

        assert_eq!(app.content_rows()[0].branch, "detached");
    }

    #[test]
    fn statuses_are_asked_for_the_rows_on_screen_and_only_once() {
        let mut app = app_listing_checkouts(&["alpha", "beta", "gamma", "delta"]);

        app.ask_for_statuses(0..2);
        assert_eq!(
            app.pending_statuses.len(),
            2,
            "one request per row on screen"
        );
        // An answer for a row that was never on screen has nowhere to go.
        app.apply_status_result_for_test("delta", working_tree(5, false));
        assert_eq!(marker_of(&app, "delta"), NOT_KNOWN_YET_MARKER);

        app.apply_status_result_for_test("alpha", working_tree(1, false));
        app.ask_for_statuses(0..3);
        assert_eq!(
            app.pending_statuses.len(),
            3,
            "only gamma is new; alpha and beta were asked already"
        );
        assert_eq!(
            marker_of(&app, "alpha"),
            CHANGED_MARKER,
            "and alpha keeps its answer"
        );

        app.ask_for_statuses(1..40);
        assert_eq!(
            app.pending_statuses.len(),
            4,
            "a range past the end stops at it"
        );
    }

    #[test]
    fn a_new_listing_abandons_the_statuses_of_the_old_one() {
        let mut app = app_listing_checkouts(&["alpha"]);
        app.ask_for_statuses(0..1);

        // Somewhere else, which has an `alpha` of its own.
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: app.contents.clone(),
            }),
        );
        app.apply_status_result_for_test("alpha", working_tree(3, false));

        assert!(
            app.pending_statuses.is_empty(),
            "the outstanding request is dropped"
        );
        assert_eq!(
            marker_of(&app, "alpha"),
            NOT_KNOWN_YET_MARKER,
            "a late answer about the old folder is not drawn on the new one"
        );
    }

    #[test]
    fn search_results_ask_for_no_statuses() {
        let mut app = app_listing_checkouts(&["alpha"]);
        app.apply_find_result_for_test(
            "a",
            Response::Names {
                root: "/repos".to_owned(),
                matches: Vec::new(),
                cut_short: false,
            },
        );

        app.ask_for_statuses(0..10);

        assert!(app.pending_statuses.is_empty());
    }

    // ---- All Repositories (#591) -----------------------------------------

    fn all_repository_entry(name: &str, location: &str) -> protocol::AllRepositoryEntry {
        protocol::AllRepositoryEntry {
            name: name.to_owned(),
            location: location.to_owned(),
            repository: RepositoryInfo {
                provider: None,
                branch: Some("main".to_owned()),
                remote: None,
                kind: RepositoryKind::Clone,
                last_activity: None,
                last_fetch: None,
            },
        }
    }

    #[test]
    fn opening_all_repositories_asks_the_service_and_shows_a_looking_status() {
        let mut app = app_with_one_content_entry();

        app.open_all_repositories();

        assert!(app.pending_all_repositories.is_some());
        assert!(app.showing_all_repositories());
        assert!(app.status_text().contains("Looking for repositories"));
    }

    #[test]
    fn results_land_in_the_contents_pane_with_a_location_column() {
        let mut app = app_with_one_content_entry();
        app.open_all_repositories();

        app.apply_all_repositories_result_for_test(Response::AllRepositories {
            entries: vec![
                all_repository_entry("direct", ""),
                all_repository_entry("project", "github/owner"),
            ],
            done: true,
        });

        let rows = app.content_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "direct/");
        assert_eq!(rows[0].kind, ".", "a direct child names the root itself");
        assert_eq!(rows[1].name, "project/");
        assert_eq!(rows[1].kind, "github/owner");
        assert!(rows[1].is_repository);
        assert_eq!(rows[1].branch, "main");
    }

    #[test]
    fn an_unfinished_scan_keeps_polling_and_says_how_many_are_found_so_far() {
        let mut app = app_with_one_content_entry();
        app.open_all_repositories();

        app.apply_all_repositories_result_for_test(Response::AllRepositories {
            entries: vec![all_repository_entry("one", "")],
            done: false,
        });

        assert!(
            app.pending_all_repositories.is_some(),
            "still scanning, so another poll is on its way"
        );
        assert!(app.status_text().contains("1 found"));

        app.apply_all_repositories_result_for_test(Response::AllRepositories {
            entries: vec![
                all_repository_entry("one", ""),
                all_repository_entry("two", ""),
            ],
            done: true,
        });

        assert_eq!(app.content_rows().len(), 2);
        assert!(
            !app.status_text().contains("Looking for repositories"),
            "the scan is done: {}",
            app.status_text()
        );
    }

    #[test]
    fn escape_restores_the_listing_that_was_on_screen_before() {
        let mut app = app_listing_checkouts(&["alpha", "beta"]);
        app.select_content(1);
        app.open_all_repositories();

        app.cancel_pending();

        assert!(!app.showing_all_repositories());
        assert_eq!(app.content_selected(), 1);
    }

    #[test]
    fn f5_discards_the_cached_scan_and_asks_again() {
        let mut app = app_with_one_content_entry();
        app.open_all_repositories();
        app.apply_all_repositories_result_for_test(Response::AllRepositories {
            entries: vec![all_repository_entry("one", "")],
            done: true,
        });

        app.refresh();

        assert!(app.content_rows().is_empty(), "the stale list is cleared");
        assert!(app.pending_all_repositories.is_some());
        assert!(app.status_text().contains("Looking for repositories"));
    }

    #[test]
    fn opening_a_nested_entry_goes_to_its_real_folder_with_it_selected() {
        let mut app = app_with_one_content_entry();
        app.open_all_repositories();
        app.apply_all_repositories_result_for_test(Response::AllRepositories {
            entries: vec![all_repository_entry("project", "github/owner")],
            done: true,
        });

        app.open_content(0);

        assert!(!app.showing_all_repositories());
        assert_eq!(
            PathBuf::from(app.current_path()),
            std::env::temp_dir().join("github").join("owner")
        );
    }

    #[test]
    fn navigating_a_folder_leaves_all_repositories() {
        let mut app = app_with_one_content_entry();
        app.open_all_repositories();

        app.refresh();
        app.select_folder(0);

        assert!(!app.showing_all_repositories());
    }

    // ---- #592: what is wrong with the Repos Directory itself ----

    #[test]
    fn classify_root_problem_reports_a_missing_folder() {
        let missing = std::env::temp_dir().join("repos-explorer-592-missing-folder");
        let _ = std::fs::remove_dir_all(&missing);

        let problem = super::classify_root_problem(&missing, "not found");

        assert_eq!(
            problem,
            super::RootProblem::NotThere {
                cause: super::NotThereCause::FolderMissing
            }
        );
    }

    #[test]
    fn classify_root_problem_reports_a_drive_that_is_not_connected() {
        // A drive-letter path parses from its text alone (see
        // `drive_letter`), so this runs on any host - but the letter has to
        // be one this host has *not* got, or the classifier rightly reports
        // something else. It was written as "Z:\repos", which on a Windows
        // machine with a Z: drive is a real folder: the test passed on the
        // Linux runner and failed on the developer's own machine.
        // Not merely "cannot read": a drive that is there but not ready,
        // such as an empty optical drive, answers with something else and
        // is classified as unreadable rather than missing, correctly.
        let absent = |path: String| {
            std::fs::metadata(path)
                .err()
                .is_some_and(|err| err.kind() == std::io::ErrorKind::NotFound)
        };
        let Some(letter) = ('D'..='Z')
            .map(|letter| format!("{letter}:"))
            .find(|drive| absent(format!("{drive}\\")) && absent(format!("{drive}\\repos")))
        else {
            // Every drive letter answers: nothing to tell apart here.
            return;
        };

        let problem =
            super::classify_root_problem(Path::new(&format!("{letter}\\repos")), "not found");

        assert_eq!(
            problem,
            super::RootProblem::NotThere {
                cause: super::NotThereCause::DriveNotConnected(letter)
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn classify_root_problem_reports_permission_denied() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = scratch("592-permission-denied");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).unwrap();
        let err = std::fs::read_dir(&dir).unwrap_err();

        let problem = super::classify_root_problem(&dir, &err.to_string());

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            problem,
            super::RootProblem::NotReadable {
                message: err.to_string(),
            }
        );
    }

    #[test]
    fn a_missing_repos_directory_shows_the_not_there_message() {
        let missing = std::env::temp_dir().join("repos-explorer-592-app-missing");
        let _ = std::fs::remove_dir_all(&missing);
        let mut app = App::new(missing);

        app.apply_contents_result(
            &[],
            Ok(Response::Error {
                message: "not found".to_owned(),
            }),
        );

        assert!(app.contents_message_title().contains("is not available"));
        assert_eq!(app.contents_message_detail(), "The folder does not exist");
        assert!(app.contents_message_show_retry());
        assert!(app.contents_message_show_choose());
        assert!(!app.contents_message_show_clear_filter());
        assert!(app.content_rows().is_empty());
        assert_eq!(app.folder_rows().len(), 1, "the root, with no children");
        assert!(app.file_views().is_empty());
    }

    #[test]
    fn an_empty_repos_directory_shows_the_empty_message_with_only_choose() {
        let dir = scratch("592-app-empty");
        let mut app = App::new(dir.clone());

        app.apply_contents_result(&[], Ok(Response::Directory { entries: vec![] }));

        assert!(
            app.contents_message_title()
                .contains("has no repositories yet")
        );
        assert!(app.contents_message_detail().contains("will appear here"));
        assert!(!app.contents_message_show_retry());
        assert!(app.contents_message_show_choose());
        assert!(!app.contents_message_show_clear_filter());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_listable_repos_directory_shows_no_message() {
        let dir = scratch("592-app-listable");
        let mut app = App::new(dir.clone());

        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("a.txt", false)]),
            }),
        );

        assert_eq!(app.contents_message_title(), "");
        assert!(!app.contents_message_show_retry());
        assert!(!app.contents_message_show_choose());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_filter_matching_nothing_offers_a_way_to_clear_it_not_the_empty_message() {
        let mut app = app_with_four_rows();
        app.begin_filter();
        for c in "zzz".chars() {
            app.handle_key_text(&c.to_string());
        }

        assert_eq!(app.contents_message_title(), "No name matches \"zzz\"");
        assert!(!app.contents_message_show_retry());
        assert!(!app.contents_message_show_choose());
        assert!(app.contents_message_show_clear_filter());

        app.clear_filters();

        assert_eq!(app.contents_message_title(), "");
        assert_eq!(app.content_rows().len(), 4);
    }

    #[test]
    fn tab_moves_the_message_focus_between_retry_and_choose() {
        let missing = std::env::temp_dir().join("repos-explorer-592-tab-focus");
        let _ = std::fs::remove_dir_all(&missing);
        let mut app = App::new(missing);
        app.apply_contents_result(
            &[],
            Ok(Response::Error {
                message: "not found".to_owned(),
            }),
        );
        assert_eq!(
            app.contents_message_focus(),
            0,
            "Retry is highlighted first"
        );

        app.move_message_focus(1);
        assert_eq!(app.contents_message_focus(), 1, "Tab moved to Choose");

        app.handle_return();
        assert!(
            app.choosing_repos_root(),
            "Return activated the highlighted Choose button"
        );
    }

    #[test]
    fn return_with_no_tab_activates_retry_by_default() {
        let missing = std::env::temp_dir().join("repos-explorer-592-return-default");
        let _ = std::fs::remove_dir_all(&missing);
        let mut app = App::new(missing);
        app.apply_contents_result(
            &[],
            Ok(Response::Error {
                message: "not found".to_owned(),
            }),
        );
        app.pending_contents = None;

        app.handle_return();

        assert!(
            app.pending_contents.is_some(),
            "Retry re-issued the listing request"
        );
    }

    #[test]
    fn the_message_re_lists_automatically_after_ten_seconds_of_ticks() {
        let missing = std::env::temp_dir().join("repos-explorer-592-auto-retry");
        let _ = std::fs::remove_dir_all(&missing);
        let mut app = App::new(missing);
        app.apply_contents_result(
            &[],
            Ok(Response::Error {
                message: "not found".to_owned(),
            }),
        );
        app.pending_contents = None;

        for _ in 0..super::LISTING_MESSAGE_RETRY_TICKS - 1 {
            app.tick();
        }
        assert!(
            app.pending_contents.is_none(),
            "not yet - fewer than ten seconds of ticks"
        );

        app.tick();
        assert!(
            app.pending_contents.is_some(),
            "ten seconds of ticks re-listed on its own"
        );
    }
}
