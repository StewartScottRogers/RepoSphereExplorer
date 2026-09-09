//! Application state for the three-pane explorer, independent of Slint so
//! it is unit-testable without a display. See `tui::app` for the sibling
//! Ratatui implementation: per §3.1 each front end owns its own
//! presentation half, so the two are separate, not shared, despite the
//! similar shape.

use plugin_api::{FolderPresentation, Graphic, Icon, PluginPresentation, UNKNOWN_ICON};
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
    &plugin_maven::MavenPresentation,
    &plugin_msbuild::MsbuildPresentation,
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
fn dedup_name(existing: &[DirectoryEntry], base: &str) -> String {
    if !existing.iter().any(|entry| entry.name == base) {
        return base.to_owned();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base} ({n})");
        if !existing.iter().any(|entry| entry.name == candidate) {
            return candidate;
        }
        n += 1;
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
    editing_file: Option<(PathBuf, String)>,
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
    clipboard: Option<(PathBuf, ClipboardMode)>,
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
    fn show_file_view(&mut self, view: Option<Response>) {
        self.file_view = view;
        self.file_view_index = 0;
    }

    fn load_file_view(&mut self) {
        let Some(entry) = self.contents.get(self.content_selected) else {
            self.show_file_view(None);
            self.pending_file = None;
            return;
        };
        let path = self.selected_dir_path().join(&entry.name);
        let request = Request::ViewFile {
            path: path.to_string_lossy().into_owned(),
        };
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
            self.show_file_view(Some(result.unwrap_or_else(|err| Response::Error {
                message: err.to_string(),
            })));
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
        if let Some((path, name)) = self.selected_entry_path() {
            self.mode = Mode::RenameInput { path, input: name };
        }
    }

    /// Starts editing a destination name to copy the selected contents row
    /// to, de-duplicated against the current listing: the source's own name
    /// always collides, and accepting it copies the file onto itself.
    pub fn request_copy(&mut self) {
        if let Some((path, name)) = self.selected_entry_path() {
            let input = dedup_name(&self.contents, &name);
            self.mode = Mode::CopyInput { path, input };
        }
    }

    /// Starts editing a destination directory name to extract the selected
    /// contents row (an archive) into.
    pub fn request_extract(&mut self) {
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
        let base = if is_dir { "New folder" } else { "New file" };
        let name = dedup_name(&self.contents, base);
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

    /// Confirms a pending rename/copy/extract input, sending its request.
    pub fn confirm_text_input(&mut self) {
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        // Each arm carries the name the folder will hold afterwards, so the
        // reloaded listing can put the selection back on it.
        let request = match mode {
            Mode::RenameInput { path, input } if !input.is_empty() => Some((
                Request::Rename {
                    from: path.to_string_lossy().into_owned(),
                    to: sibling_path(&path, &input),
                },
                input,
            )),
            Mode::CopyInput { path, input } if !input.is_empty() => Some((
                Request::Copy {
                    from: path.to_string_lossy().into_owned(),
                    to: sibling_path(&path, &input),
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
            Mode::Normal if os == "macos" => self.request_rename(),
            Mode::Normal | Mode::ConfirmDelete { .. } => {}
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
        if !matches!(self.mode, Mode::Normal) || self.contents.is_empty() {
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
        if let Some(node) = self.root.node_at_mut(&child_indices) {
            node.expanded = true;
        }

        let new_rows = self.root.flatten();
        if let Some(row) = new_rows.iter().position(|(_, idx)| idx == &child_indices) {
            self.folder_selected = row;
        }
        self.focus = Pane::Folders;
        self.load_contents_for_selected();
    }

    /// Navigates to the parent of the directory currently shown in the
    /// contents pane. A no-op if that directory has no parent (the
    /// filesystem root).
    pub fn navigate_to_parent(&mut self) {
        let Some(parent) = self.selected_dir_path().parent().map(PathBuf::from) else {
            return;
        };
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
        let Some((path, name)) = self.selected_entry_path() else {
            return;
        };
        let verb = match mode {
            ClipboardMode::Copy => "copied",
            ClipboardMode::Cut => "cut",
        };
        self.clipboard = Some((path, mode));
        self.status = Some(format!("{name} {verb}"));
    }

    /// Ctrl+V: copies or moves whatever the clipboard holds into the folder
    /// being browsed. The destination name is de-duplicated, since both
    /// operations refuse to replace an existing entry.
    pub fn paste_from_clipboard(&mut self) {
        let Some((source, mode)) = self.clipboard.clone() else {
            return;
        };
        let Some(name) = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            return;
        };
        let destination_dir = self.selected_dir_path();
        // Pasting back into the folder it came from has to land beside the
        // original rather than on it.
        let name = if source.parent() == Some(destination_dir.as_path()) {
            dedup_name(&self.contents, &name)
        } else {
            name
        };
        let destination = destination_dir.join(&name);
        let from = source.to_string_lossy().into_owned();
        let to = destination.to_string_lossy().into_owned();
        let request = match mode {
            ClipboardMode::Copy => Request::Copy { from, to },
            ClipboardMode::Cut => Request::Rename { from, to },
        };
        if mode == ClipboardMode::Cut {
            self.clipboard = None;
        }
        self.reselect = Some(name);
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
            .map_or_else(String::new, |(_, text)| text.clone())
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
        self.editing_file = Some((path, text));
        self.focus = Pane::File;
    }

    /// Takes the editor's text as the user has changed it.
    pub fn set_edit_text(&mut self, text: &str) {
        if let Some((_, current)) = self.editing_file.as_mut() {
            text.clone_into(current);
        }
    }

    /// Ctrl+S: writes the editor's text back through the service, which is
    /// the only process that touches the filesystem.
    pub fn save_file_edit(&mut self) {
        let Some((path, text)) = self.editing_file.clone() else {
            return;
        };
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
        self.pending_operation = Some(spawn_request(Request::Undo));
        self.status = Some("undoing...".to_owned());
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

    /// Whether Forward has a folder to return to.
    #[must_use]
    pub fn can_go_forward(&self) -> bool {
        self.history_index + 1 < self.history.len()
    }

    /// Goes back one folder in history.
    pub fn go_back(&mut self) {
        if !self.can_go_back() {
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
        if !self.can_go_forward() {
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

    /// Display labels for the folders pane, one per visible tree row.
    #[must_use]
    pub fn folder_labels(&self) -> Vec<String> {
        self.root
            .flatten()
            .iter()
            .map(|(depth, indices)| {
                let node = self.root.node_at(indices);
                let name = node.map_or("?", |n| n.name.as_str());
                let marker = node.map_or(' ', |n| {
                    if n.children.is_none() {
                        '.'
                    } else if n.expanded {
                        'v'
                    } else {
                        '>'
                    }
                });
                format!("{}{marker} {name}/", "  ".repeat(*depth))
            })
            .collect()
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
        let selected = self
            .contents
            .get(self.content_selected)
            .map(|entry| entry.name.clone());
        self.sort_contents();
        self.content_selected = selected
            .and_then(|name| self.contents.iter().position(|entry| entry.name == name))
            .unwrap_or(0);
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
        match &self.mode {
            Mode::Normal => -1,
            _ => i32::try_from(self.content_selected).unwrap_or(-1),
        }
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
    use super::{
        App, PathBuf, UNKNOWN_ICON, format_kind, format_timestamp, icon_for, strip_verbatim_prefix,
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
        assert_eq!(app.folder_labels().len(), 2); // root + "sub"
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

    #[test]
    fn requesting_a_copy_prefills_a_name_that_does_not_collide() {
        let mut app = app_with_one_content_entry();
        app.request_copy();
        assert_eq!(app.status_text(), "Copy to: doomed.txt (2)_  (Enter/Esc)");
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
        assert!(app.folder_labels()[0].contains('v')); // root starts expanded.

        app.toggle_folder(0);
        assert!(app.folder_labels()[0].contains('>'));

        app.toggle_folder(0);
        assert!(app.folder_labels()[0].contains('v'));
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
    fn folder_labels_mark_collapsed_and_leaf_rows_distinctly() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("sub", true)]),
            }),
        );
        // The root is expanded by default but "sub"'s own children have
        // never been fetched, so it must render as a leaf ('.'), not a
        // collapsed-but-known-nonempty folder ('>').
        assert!(app.folder_labels()[1].contains('.'));

        app.toggle_folder(0);
        assert_eq!(app.folder_labels().len(), 1);
        assert!(app.folder_labels()[0].contains('>'));
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
}
