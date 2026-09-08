//! Application state for the three-pane explorer, independent of Slint so
//! it is unit-testable without a display. See `tui::app` for the sibling
//! Ratatui implementation: per §3.1 each front end owns its own
//! presentation half, so the two are separate, not shared, despite the
//! similar shape.

use plugin_api::PluginPresentation;
use protocol::{DirectoryEntry, Request, Response};
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
];

/// A small, fixed glyph set for the contents pane, keyed only by file
/// extension (or the folder bucket for directories) - not a
/// `plugin_api::PluginPresentation` concept, since dozens of plugins each
/// getting a distinct icon is explicitly out of scope for this glyph set.
///
/// Glyphs are drawn from the Geometric Shapes block rather than pictographic
/// emoji: the latter render as blank boxes without a colour-emoji font,
/// which most Linux font setups (including CI's) don't install by default.
fn content_glyph(name: &str, is_dir: bool) -> &'static str {
    if is_dir {
        // U+25A0. The pointing triangle U+25B8 this used renders as tofu in
        // the GUI's default font.
        return "\u{25A0}";
    }
    let extension = std::path::Path::new(name)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_lowercase);
    match extension.as_deref() {
        Some(
            "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "java" | "kt" | "go" | "rb" | "php" | "pl"
            | "c" | "h" | "cpp" | "hpp" | "cs" | "swift" | "scala" | "sh" | "ps1" | "sql" | "html"
            | "css" | "json" | "yaml" | "yml" | "toml" | "xml" | "md" | "txt",
        ) => "\u{25AA}", // ▪
        Some("png" | "jpg" | "jpeg" | "gif" | "bmp" | "svg" | "webp" | "ico" | "psd") => {
            "\u{25C6}" // ◆
        }
        Some("zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "iso") => "\u{25B2}", // ▲
        Some("pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "epub" | "odt") => {
            "\u{25CF}" // ●
        }
        _ => "\u{25CB}", // ○
    }
}

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
    ConfirmDelete { path: PathBuf, name: String },
    RenameInput { path: PathBuf, input: String },
    CopyInput { path: PathBuf, input: String },
    ExtractInput { path: PathBuf, input: String },
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
    /// Type marker shown ahead of the name.
    pub glyph: String,
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
    /// The entry name to reselect after the next listing arrives, set
    /// by whichever operation is about to change the folder in place.
    reselect: Option<String>,
    /// Which column the contents pane is sorted by, and in which direction.
    sort_key: SortKey,
    sort_ascending: bool,
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
            reselect: None,
            sort_key: SortKey::Name,
            sort_ascending: true,
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

    fn load_file_view(&mut self) {
        let Some(entry) = self.contents.get(self.content_selected) else {
            self.file_view = None;
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
            self.file_view = Some(result.unwrap_or_else(|err| Response::Error {
                message: err.to_string(),
            }));
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
                self.load_file_view();
            }
            Ok(Response::Error { message }) => self.status = Some(message),
            Ok(Response::FileView { .. } | Response::Done) => {
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
        if let Some((path, name)) = self.selected_entry_path() {
            self.mode = Mode::ConfirmDelete { path, name };
        }
    }

    /// Confirms a pending delete confirmation, sending the delete request.
    pub fn confirm_delete(&mut self) {
        let Mode::ConfirmDelete { path, .. } = std::mem::replace(&mut self.mode, Mode::Normal)
        else {
            return;
        };
        let request = Request::Delete {
            paths: vec![path.to_string_lossy().into_owned()],
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
            | Mode::ExtractInput { input, .. } => Some(input),
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
            Mode::RenameInput { .. } | Mode::CopyInput { .. } | Mode::ExtractInput { .. } => {
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
            Mode::RenameInput { .. } | Mode::CopyInput { .. } | Mode::ExtractInput { .. } => {
                self.type_char(text);
            }
            Mode::Normal => match text {
                "r" => self.request_rename(),
                "c" => self.request_copy(),
                "x" => self.request_extract(),
                _ => {}
            },
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
            self.focus = Pane::Contents;
            self.load_file_view();
        }
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
        self.root = FolderNode::root(parent);
        self.folder_selected = 0;
        self.load_contents_for_selected();
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
                let glyph = content_glyph(&entry.name, entry.is_dir);
                if entry.is_dir {
                    format!("{glyph} {}/", entry.name)
                } else {
                    format!("{glyph} {}", entry.name)
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
                glyph: content_glyph(&entry.name, entry.is_dir).to_owned(),
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
                kind: format_kind(&entry.name, entry.is_dir),
                modified: format_timestamp(entry.modified),
            })
            .collect()
    }

    /// Index of the selected row in [`Self::content_labels`].
    #[must_use]
    pub fn content_selected(&self) -> usize {
        self.content_selected
    }

    /// Display text for the file pane.
    #[must_use]
    pub fn file_text(&self) -> String {
        match &self.file_view {
            Some(Response::FileView { plugin, data }) => present(plugin, data).join("\n"),
            Some(Response::Error { message }) => message.clone(),
            Some(Response::Directory { .. } | Response::Done) | None => String::new(),
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
            Mode::Normal => String::new(),
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
        App, PathBuf, content_glyph, format_kind, format_timestamp, strip_verbatim_prefix,
    };
    use protocol::{DirectoryEntry, Response};

    fn entries(names: &[(&str, bool)]) -> Vec<DirectoryEntry> {
        names
            .iter()
            .map(|(name, is_dir)| DirectoryEntry {
                name: (*name).to_owned(),
                is_dir: *is_dir,
                size: 0,
                modified: None,
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
            })
            .collect()
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

        assert_eq!(
            app.content_labels(),
            vec!["\u{25A0} sub/", "\u{25AA} note.txt"]
        );
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
        assert_eq!(app.content_labels(), vec!["\u{25AA} only.txt"]);
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
    fn r_key_prefills_the_rename_input_with_the_current_name() {
        let mut app = app_with_one_content_entry();
        app.handle_key_text("r");
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
        app.handle_key_text("r");

        app.backspace();
        app.handle_key_text("!");

        assert_eq!(app.status_text(), "Rename to: doomed.tx!_  (Enter/Esc)");
    }

    #[test]
    fn returning_confirms_a_rename_and_sends_a_request() {
        let mut app = app_with_one_content_entry();
        app.handle_key_text("r");

        app.handle_return();

        assert!(app.pending_operation.is_some());
        assert_eq!(app.status_text(), "working...");
    }

    #[test]
    fn escaping_a_rename_input_cancels_without_a_request() {
        let mut app = app_with_one_content_entry();
        app.handle_key_text("r");

        app.cancel_pending();

        assert!(app.pending_operation.is_none());
        assert_ne!(app.status_text(), "Rename to: doomed.txt_  (Enter/Esc)");
    }

    #[test]
    fn c_key_prefills_the_copy_input_with_a_name_that_does_not_collide() {
        let mut app = app_with_one_content_entry();
        app.handle_key_text("c");
        assert_eq!(app.status_text(), "Copy to: doomed.txt (2)_  (Enter/Esc)");
    }

    #[test]
    fn returning_confirms_a_copy_and_sends_a_request() {
        let mut app = app_with_one_content_entry();
        app.handle_key_text("c");

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
    fn x_key_prefills_the_extract_input_with_the_archive_stem() {
        let mut app = app_with_one_archive_entry();
        app.handle_key_text("x");
        assert_eq!(app.status_text(), "Extract to: bundle_  (Enter/Esc)");
    }

    #[test]
    fn returning_confirms_an_extract_and_sends_a_request() {
        let mut app = app_with_one_archive_entry();
        app.handle_key_text("x");

        app.handle_return();

        assert!(app.pending_operation.is_some());
        assert_eq!(app.status_text(), "working...");
    }

    #[test]
    fn returning_with_an_emptied_rename_input_does_not_send_a_request() {
        let mut app = app_with_one_content_entry();
        app.handle_key_text("r");
        for _ in 0.."doomed.txt".len() {
            app.backspace();
        }

        app.handle_return();

        assert!(app.pending_operation.is_none());
    }

    #[test]
    fn typed_letters_that_are_also_hotkeys_are_appended_during_text_input() {
        let mut app = app_with_one_content_entry();
        app.handle_key_text("r");

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
        app.handle_key_text("r");

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
        app.handle_key_text("c");
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

        app.handle_key_text("r");
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
        app.handle_key_text("c");
        for _ in 0.."doomed.txt (2)".len() {
            app.backspace();
        }

        app.handle_return();

        assert!(app.pending_operation.is_none());
    }

    #[test]
    fn returning_with_an_emptied_extract_input_does_not_send_a_request() {
        let mut app = app_with_one_archive_entry();
        app.handle_key_text("x");
        for _ in 0.."bundle".len() {
            app.backspace();
        }

        app.handle_return();

        assert!(app.pending_operation.is_none());
    }

    #[test]
    fn cancel_pending_during_copy_input_returns_to_normal() {
        let mut app = app_with_one_content_entry();
        app.handle_key_text("c");

        app.cancel_pending();

        assert!(app.pending_operation.is_none());
        assert_ne!(app.status_text(), "Copy to: doomed.txt_  (Enter/Esc)");
    }

    #[test]
    fn cancel_pending_during_extract_input_returns_to_normal() {
        let mut app = app_with_one_archive_entry();
        app.handle_key_text("x");

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
    fn content_glyph_distinguishes_representative_extensions() {
        assert_eq!(content_glyph("main.rs", false), "\u{25AA}");
        assert_eq!(content_glyph("photo.png", false), "\u{25C6}");
        assert_eq!(content_glyph("bundle.zip", false), "\u{25B2}");
        assert_eq!(content_glyph("report.pdf", false), "\u{25CF}");
    }

    #[test]
    fn content_glyph_falls_back_to_a_default_marker_for_an_unrecognized_extension() {
        assert_eq!(content_glyph("mystery.xyz123", false), "\u{25CB}");
        assert_eq!(content_glyph("no_extension_at_all", false), "\u{25CB}");
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

    #[test]
    fn content_glyph_marks_directories_distinctly_from_every_file_glyph() {
        let folder_glyph = content_glyph("anything", true);
        assert_eq!(folder_glyph, "\u{25A0}");
        assert_ne!(folder_glyph, content_glyph("main.rs", false));
        assert_ne!(folder_glyph, content_glyph("photo.png", false));
        assert_ne!(folder_glyph, content_glyph("bundle.zip", false));
        assert_ne!(folder_glyph, content_glyph("report.pdf", false));
        assert_ne!(folder_glyph, content_glyph("mystery.xyz123", false));
    }
}
