//! Application state for the three-pane explorer, independent of Slint so
//! it is unit-testable without a display. See `tui::app` for the sibling
//! Ratatui implementation: per §3.1 each front end owns its own
//! presentation half, so the two are separate, not shared, despite the
//! similar shape.

use crate::document::Document;
use crate::editor;
use plugin_api::{
    Class, FolderPresentation, Graphic, Icon, PREVIEW_VIEW, PluginPresentation, Span, TEXT_VIEW,
    UNKNOWN_ICON,
};
use protocol::{DirectoryEntry, ReposRoot, RepositoryInfo, Request, Response};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
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
                    previous.remove(&entry.name).unwrap_or_else(|| FolderNode {
                        path: path.join(&entry.name),
                        name: entry.name.clone(),
                        expanded: false,
                        children: None,
                    })
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

/// Explorer's "Type" column: `"File folder"` for a directory, otherwise the
/// uppercased extension as `"RS file"`, or plain `"File"` when there is none.
fn format_kind(name: &str, is_dir: bool) -> String {
    format_kind_of(name, is_dir, None)
}

/// As [`format_kind`], but for an entry the service has told us something
/// about as a source control working copy.
///
/// A repository says so, and names the provider it came from, because that
/// is the fact somebody opening their workspace is looking for. An ordinary
/// folder is still an ordinary folder - visible, and plainly different
/// (GUIDANCE.md 2.5).
fn format_kind_of(name: &str, is_dir: bool, repository: Option<&RepositoryInfo>) -> String {
    if let Some(repository) = repository {
        // The provider alone, because the Type column is narrow and
        // "Repository (github.com)" elides to "Repository (git..." - which
        // keeps the half a reader already knows from the row's own styling
        // and throws away the half they do not. A checkout with no remote
        // has no provider to name, and says what it is instead.
        return repository
            .provider
            .clone()
            .unwrap_or_else(|| "Repository".to_owned());
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
}

/// How far one level of the tree indents, in pixels.
pub const FOLDER_INDENT: f32 = 16.0;

/// The width of the chevron column in front of a folder's icon.
pub const FOLDER_CHEVRON: f32 = 16.0;

/// The padding inside a pane's left edge, before the first row content.
pub const FOLDER_PADDING: f32 = 4.0;

/// Whether `x` pixels in from a folders-pane row's left edge falls on the
/// chevron of a row at `depth`.
///
/// In Rust rather than in `app.slint` for the reason given on
/// [`crate::scroll_offset_for`]: a rule written in that file cannot be
/// exercised without an event loop, and every layout rule this project has
/// got wrong was one that lived there.
#[must_use]
pub fn chevron_hit(x: f32, depth: usize) -> bool {
    // `depth` is a tree level, and a tree deep enough to overflow this has
    // long since run out of pane to indent into.
    let Ok(level) = u16::try_from(depth) else {
        return false;
    };
    let start = FOLDER_PADDING + f32::from(level) * FOLDER_INDENT;
    x >= start && x < start + FOLDER_CHEVRON
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
}

/// The three-pane explorer's state.
pub struct App {
    root: FolderNode,
    folder_selected: usize,
    contents: Vec<DirectoryEntry>,
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
    focus: Pane,
    pending_contents: Option<(Vec<usize>, Receiver<io::Result<Response>>)>,
    pending_file: Option<Receiver<io::Result<Response>>>,
    /// The file the outstanding preview was asked for, and the file the
    /// pane is showing now. Compared so that re-reading the file already
    /// on screen keeps the view the reader chose.
    pending_file_path: Option<PathBuf>,
    shown_file_path: Option<PathBuf>,
    mode: Mode,
    pending_operation: Option<Receiver<io::Result<Response>>>,
    after_operation: Option<AfterOperation>,
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
            contents: Vec::new(),
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
            focus: Pane::Folders,
            pending_contents: None,
            pending_file: None,
            pending_file_path: None,
            shown_file_path: None,
            mode: Mode::Normal,
            pending_operation: None,
            after_operation: None,
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
    }

    fn apply_contents_result(&mut self, indices: &[usize], result: io::Result<Response>) {
        self.status = None;
        match result {
            Ok(Response::Directory { entries }) => {
                if let Some(node) = self.root.node_at_mut(indices) {
                    node.set_children_from(&entries);
                }
                self.contents = entries;
                self.sort_contents();
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
            Ok(Response::Error { message }) => self.status = Some(message),
            Ok(Response::FileView { .. } | Response::Done | Response::ReposRoots { .. }) => {
                self.status = Some("expected a directory listing".to_owned());
            }
            Err(err) => self.status = Some(err.to_string()),
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
        self.after_operation = Some(AfterOperation::EnterRename { path, input: name });
        self.status = Some("working...".to_owned());
    }

    fn input_mut(&mut self) -> Option<&mut String> {
        match &mut self.mode {
            Mode::RenameInput { input, .. }
            | Mode::CopyInput { input, .. }
            | Mode::ExtractInput { input, .. }
            | Mode::PathInput { input }
            | Mode::ReposRootInput { input } => Some(input),
            Mode::Normal | Mode::ConfirmDelete { .. } => None,
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

    /// Removes the last character of a pending rename/copy/extract input;
    /// a no-op otherwise.
    pub fn backspace(&mut self) {
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
            | Mode::ReposRootInput { .. } => {
                self.confirm_text_input();
            }
            // Return renames on macOS, which is that platform's
            // convention and the reason this is parameterised at all.
            Mode::Normal if os == "macos" => self.request_rename(),
            Mode::Normal => self.activate_selection(),
            Mode::ConfirmDelete { .. } => {}
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
            | Mode::ReposRootInput { .. } => {
                self.type_char(text);
            }
            // Explorer's type-ahead: a typed letter jumps to a name, it is
            // not a command. Rename, copy and extract are on F2, Ctrl+C and
            // the context menu.
            Mode::Normal => self.type_ahead(text),
        }
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
        if !matches!(self.mode, Mode::Normal) {
            return;
        }
        let (len, current) = match self.focus {
            Pane::Folders => (self.root.flatten().len(), self.folder_selected),
            Pane::Contents | Pane::File => (self.contents.len(), self.content_selected),
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
        if index < self.contents.len() {
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
            _ => String::new(),
        }
    }

    /// Whether the address bar is currently a text field.
    #[must_use]
    pub const fn editing_path(&self) -> bool {
        matches!(self.mode, Mode::PathInput { .. })
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
        self.status = Some("undoing...".to_owned());
    }

    /// The editor's undo, with no clipboard to reach for.
    pub fn undo_edit(&mut self) {
        if let Some(edit) = self.editing_file.as_mut() {
            edit.document.undo();
        }
    }

    /// F5: re-reads the folder being browsed.
    pub fn refresh(&mut self) {
        self.reselect = self
            .contents
            .get(self.content_selected)
            .map(|entry| entry.name.clone());
        self.load_contents_for_selected();
    }

    /// Moves the contents selection to the first or last row.
    pub fn select_edge(&mut self, last: bool) {
        if !matches!(self.mode, Mode::Normal) || self.contents.is_empty() {
            return;
        }
        let index = if last { self.contents.len() - 1 } else { 0 };
        self.select_content(index);
    }

    /// Jumps to the first entry whose name starts with `prefix`, matched
    /// without regard to case, the way Explorer's type-ahead does. Search
    /// starts after the current row so repeated presses cycle through the
    /// matches.
    pub fn type_ahead(&mut self, prefix: &str) {
        if !matches!(self.mode, Mode::Normal) || prefix.is_empty() {
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
        matches!(self.mode, Mode::Normal) && self.editing_file.is_none()
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
            Mode::PathInput { .. } => {
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
        if row.is_some_and(|row| row.expandable && chevron_hit(x, row.depth)) {
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

    /// Orders `contents` by the current sort column. Directories come
    /// first whichever column is chosen, the way Explorer groups them, and
    /// the name is the tiebreak so the order is total and stable.
    fn sort_contents(&mut self) {
        let key = self.sort_key;
        let ascending = self.sort_ascending;
        self.contents.sort_by(|a, b| {
            let ordering = match key {
                SortKey::Name => std::cmp::Ordering::Equal,
                SortKey::Size => a.size.cmp(&b.size),
                SortKey::Kind => {
                    format_kind(&a.name, a.is_dir).cmp(&format_kind(&b.name, b.is_dir))
                }
                SortKey::Modified => a.modified.cmp(&b.modified),
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
        self.contents
            .iter()
            .map(|entry| ContentRow {
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
                modified: format_timestamp(entry.modified),
                is_repository: entry.repository.is_some(),
            })
            .collect()
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

    /// Display text for the file pane, in whichever view is selected.
    #[must_use]
    pub fn file_text(&self) -> String {
        match &self.file_view {
            Some(Response::FileView { plugin, data, also }) => {
                let views = self.file_views();
                let mut lines = match views.get(self.file_view_index) {
                    Some(view) => present_view(plugin, view, data),
                    None => present(plugin, data),
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
            Some(Response::Directory { .. } | Response::Done | Response::ReposRoots { .. })
            | None => String::new(),
        }
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
            Mode::ReposRootInput { input } => {
                format!(
                    "Repos Directory: {input}_  (Enter to open there from now on, Esc to cancel)"
                )
            }
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
            // has nothing to say about a path being typed.
            Mode::PathInput { .. } | Mode::Normal => String::new(),
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
            // prompts live in the address bar, and Normal has no prompt.
            Mode::Normal | Mode::PathInput { .. } | Mode::ReposRootInput { .. } => return -1,
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
    /// contents yet.
    fn contents_summary(&self) -> String {
        if self.contents.is_empty() {
            return "Click a folder or file. Double-click to open. Delete/r/c/x on a file. \
                    Esc cancels."
                .to_owned();
        }
        let count = self.contents.len();
        let noun = if count == 1 { "item" } else { "items" };
        let total_size: u64 = self.contents.iter().map(|entry| entry.size).sum();
        let header = format!("{count} {noun}, {}", format_size(total_size));
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
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        Class, ColouredRun, EditCommand, colour_summary_line, file_starts_in_preview, summary_label,
    };

    use super::{
        App, PathBuf, UNKNOWN_ICON, chevron_hit, format_kind, format_timestamp, icon_for,
        strip_verbatim_prefix,
    };
    use plugin_api::{PREVIEW_VIEW, TEXT_VIEW};
    use protocol::{DirectoryEntry, RepositoryInfo, Response};

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
            !chevron_hit(3.9, 0),
            "left of the padding is not the chevron"
        );
        assert!(
            chevron_hit(4.0, 0),
            "the chevron starts where the padding ends"
        );
        assert!(chevron_hit(19.9, 0));
        assert!(!chevron_hit(20.0, 0), "at 20 the icon has started");

        assert!(!chevron_hit(19.9, 1), "a child's chevron is one indent in");
        assert!(chevron_hit(20.0, 1));
        assert!(chevron_hit(35.9, 1));
        assert!(!chevron_hit(36.0, 1));
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
        assert_eq!(rows[0].kind, "github.com");
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
        assert_eq!(rows[0].kind, "Repository");
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
}
