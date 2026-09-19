//! Application state and rendering for the three-pane explorer.
//!
//! Directory listings are not streamed by the service (see `service`'s crate
//! docs): each pane's request runs on a background thread instead, so the
//! event loop never blocks on the network round trip. Cancelling means no
//! longer waiting on that thread's result, not aborting the walk in
//! progress — a smaller, real slice of §3.3's "every long operation is
//! cancellable from the UI", with true early-abort deferred alongside
//! streaming itself.

use crate::bindings::{self, Action};
use crate::colour::{self, Run};
use crate::document::Document;
use crate::settings;
use crate::{
    classify, editable_text, editor, facts, graphic, present, present_folder, present_view,
    render_with_block, views,
};
use plugin_api::{Class, Fact, Span as PluginSpan};
use protocol::{DirectoryEntry, ReposRoot, Request, Response};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Cell, List, ListItem, ListState, Paragraph, Row, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Table, TableState, Tabs, Wrap,
};
use std::collections::HashMap;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

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

    /// Replaces this node's children with the directories found in
    /// `entries`, reusing already-fetched state for names that persist.
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

/// Which pane currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The folders tree pane.
    Folders,
    /// The current folder's contents pane.
    Contents,
    /// The selected file's preview pane.
    File,
}

impl Focus {
    #[must_use]
    fn next(self) -> Self {
        match self {
            Focus::Folders => Focus::Contents,
            Focus::Contents => Focus::File,
            Focus::File => Focus::Folders,
        }
    }

    #[must_use]
    fn previous(self) -> Self {
        match self {
            Focus::Folders => Focus::File,
            Focus::Contents => Focus::Folders,
            Focus::File => Focus::Contents,
        }
    }
}

/// Where the terminal front end opens, and what it should say about it.
///
/// Decision D7 puts every launch at the configured Repos Directory, and
/// that applies to both front ends: this asks the service the same question
/// the graphical one does, rather than defaulting to wherever the shell
/// happened to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opening {
    /// The directory to open at.
    pub root: PathBuf,
    /// A line for the status bar when the Repos Directory is not set, so
    /// the reader knows this is the platform's suggestion rather than their
    /// choice. `None` once one is configured.
    pub notice: Option<String>,
}

/// Decides where to open, given what the service reports and what the
/// command line asked for.
///
/// Split from the asking so the rule is testable without a service: an
/// explicit path wins, then the configured root, then the platform default
/// with a notice.
#[must_use]
pub fn opening_from(explicit: Option<PathBuf>, reply: Option<(Vec<ReposRoot>, String)>) -> Opening {
    if let Some(root) = explicit {
        // An explicit instruction now, not a memory of where somebody was.
        return Opening { root, notice: None };
    }

    match reply {
        Some((roots, default)) => match roots.into_iter().find(|root| root.active) {
            Some(active) => Opening {
                root: PathBuf::from(active.path),
                notice: None,
            },
            None => Opening {
                root: PathBuf::from(&default),
                notice: Some(format!(
                    "No Repos Directory set - showing {default}. Set one in the graphical front end, under File."
                )),
            },
        },
        None => Opening {
            root: PathBuf::from("."),
            notice: Some(
                "Could not ask the service where the Repos Directory is - showing the current directory."
                    .to_owned(),
            ),
        },
    }
}

/// Asks the service where this machine's Repos Directory is, and decides
/// where to open.
#[must_use]
pub fn opening(explicit: Option<PathBuf>) -> Opening {
    let reply = protocol::socket_name()
        .and_then(|name| crate::send_request(name, &Request::ReposRoots))
        .ok()
        .and_then(|response| match response {
            Response::ReposRoots { roots, default } => Some((roots, default)),
            _ => None,
        });
    opening_from(explicit, reply)
}

/// Sends `request` to the service on a background thread, returning a
/// receiver for its eventual result. Dropping the receiver without reading
/// it discards the result when it arrives: that is what cancelling a
/// pending request means here.
fn spawn_request(request: Request) -> Receiver<io::Result<Response>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = protocol::socket_name().and_then(|name| crate::send_request(name, &request));
        let _ = tx.send(result);
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

/// A modal interaction awaiting the user's response, on top of the normal
/// three-pane navigation.
enum Mode {
    /// Nothing pending; keys navigate the panes as usual.
    Normal,
    /// Asking whether to delete `path`, per GUIDANCE.md §2.1.5: destructive
    /// operations need an explicit confirmed intent before they run.
    ConfirmDelete {
        /// The path that would be deleted.
        path: PathBuf,
        /// Its display name, for the confirmation prompt.
        name: String,
    },
    /// Editing a new name to rename `path` to, within its own directory.
    RenameInput {
        /// The path being renamed.
        path: PathBuf,
        /// The new name typed so far.
        input: String,
    },
    /// Editing a destination name to copy `path` to, within its own
    /// directory.
    CopyInput {
        /// The path being copied.
        path: PathBuf,
        /// The destination name typed so far.
        input: String,
    },
    /// Editing a destination directory name to extract the archive at
    /// `path` into, within its own directory.
    ExtractInput {
        /// The archive being extracted.
        path: PathBuf,
        /// The destination directory name typed so far.
        input: String,
    },
    /// Asking whether to discard unsaved changes to the file open in the
    /// editor, named `name`, before leaving it (#645).
    ConfirmDiscardEdit {
        /// Its display name, for the confirmation prompt.
        name: String,
    },
    /// Typing into the Contents pane's filter field (#650). Unlike the
    /// other prompts above, each keystroke here narrows the listing at
    /// once rather than waiting for Enter, so the text lives on
    /// [`App::filter`] itself rather than in this variant.
    FilterInput,
}

/// A file open in the terminal File pane's editor (#645), on the same
/// terms as the graphical front end's own `Edit` (`crates/gui/src/app.rs`):
/// the path it saves to, the document a reader is typing into, and the
/// text it started from, so leaving can tell whether anything changed.
struct Editing {
    /// The file being edited.
    path: PathBuf,
    /// The caret, selection and undo history typing has built up.
    document: Document,
    /// The text the file held when editing began - compared against
    /// `document.text()` to decide whether leaving needs to ask first.
    original: String,
}

/// What the File pane says when `path`, the row it just asked to view,
/// failed - a reader-facing sentence in place of the operating system's
/// own wording (#625), when `path` is confirmed gone. Checked directly
/// against the filesystem rather than by parsing `message`: a service
/// error's text is not something a front end can reliably tell "gone"
/// apart from "unreadable" by parsing.
fn classify_file_problem(path: &Path, message: &str) -> String {
    if !matches!(std::fs::metadata(path), Err(err) if err.kind() == io::ErrorKind::NotFound) {
        return message.to_owned();
    }
    let name = path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    format!("{name} is no longer there - press F5 to reload the folder")
}

/// Which column the Contents pane is sorted by (#640). Branch is not a
/// sort key, the way the graphical front end's own `SortKey` has none
/// either: a working copy's branch says nothing about the folder's own
/// identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortKey {
    /// Entry name, case-insensitively.
    Name,
    /// The Type column's text.
    Kind,
    /// Size in bytes.
    Size,
    /// Last modified time - a repository's last activity when it has one,
    /// the folder's or file's own time otherwise.
    Modified,
}

/// What the Contents pane knows about one repository row's working tree,
/// asked for only after the listing lands, and only once per row - a
/// listing of forty checkouts must not wait on forty passes over their
/// tracked files (GUIDANCE.md §3.5, #640).
#[derive(Debug, Clone, PartialEq, Eq)]
enum RowStatus {
    /// Asked for, not answered yet.
    Waiting,
    /// Answered: the summary, or `None` when the service could not tell.
    Answered(Option<protocol::WorkingTreeSummary>),
}

/// Drawn beside a repository row's branch when its tracked files have
/// uncommitted changes - the shape itself reads "modified", the way `git
/// status` already does, rather than relying on the colour it is also
/// given (#574, #640).
const CHANGED_MARKER: &str = "M";
/// Drawn beside a repository row's branch until its status has been asked
/// for and answered.
const NOT_KNOWN_YET_MARKER: &str = "\u{2026}";
/// Drawn beside a repository row's branch when the answer came back and
/// cannot say: the index could not be read, or the count stopped short
/// without finding a change.
const CANNOT_TELL_MARKER: &str = "?";

/// Drawn after a repository row's branch when its last fetch is too old to
/// trust, or it has a remote and has never been fetched at all (#589,
/// #641). Matches the graphical front end's own glyph for the same mark.
const STALE_FETCH_MARKER: &str = "\u{23F0}";

/// How old a fetch has to be, in seconds, before [`STALE_FETCH_MARKER`] is
/// drawn - 30 days, the same threshold the graphical front end's File pane
/// already reads as stale (#576).
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

/// [`STALE_FETCH_MARKER`]'s words for the status line: `Last fetched 61
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

/// Strips Windows' `\\?\` verbatim prefix from a canonicalized path.
/// `fs::canonicalize` adds one there, and it then travels through every
/// request into the messages the status bar shows, where `\\?\C:\dir\file`
/// is noise the reader has to look past. Only a drive path is unwrapped: a
/// verbatim UNC path (`\\?\UNC\server\share`) needs its prefix to keep
/// resolving, and paths on other platforms never carry one. Mirrors the
/// graphical front end's own `strip_verbatim_prefix`.
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

/// Canonicalizes `root` for [`App::new`], so its ancestors are well-formed:
/// a relative root such as "." has `Path::parent()` return `Some("")` (an
/// empty path, not `None`), which stepping above the root (#642) would
/// otherwise treat as a real, requestable directory - re-rooting the tree
/// at "" and leaving every future request targeting a path that resolves to
/// nothing. Falls back to the given root if it doesn't exist yet or
/// canonicalization otherwise fails. Mirrors the graphical front end's own
/// canonicalizing of its root in `App::new`.
fn canonicalize_root(root: PathBuf) -> PathBuf {
    strip_verbatim_prefix(std::fs::canonicalize(&root).unwrap_or(root))
}

/// How long a type-ahead search (#642) stays open for another keystroke to
/// extend it before the next letter starts a fresh one.
const TYPE_AHEAD_TIMEOUT: Duration = Duration::from_secs(1);

/// The Contents pane's filter state (#650): a name typed into the filter
/// field, and/or narrowed to the repositories with uncommitted changes.
/// Mirrors the graphical front end's own `Filter`
/// (`crates/gui/src/app.rs`), without its keyboard-focus bookkeeping - a
/// terminal binding either is or is not in [`Mode::FilterInput`], so there
/// is no separate "focused" flag to keep in step with it.
#[derive(Default)]
struct ContentsFilter {
    /// Typed into the filter field, matched case-insensitively as a
    /// substring of the entry's name. Empty when nothing is typed.
    text: String,
    /// Whether the listing is narrowed to just the repositories with
    /// uncommitted changes.
    changed_only: bool,
}

impl ContentsFilter {
    /// Whether either half of the filter is currently narrowing the
    /// listing - what Escape has to undo before it cancels or quits.
    fn is_active(&self) -> bool {
        !self.text.is_empty() || self.changed_only
    }
}

/// The three-pane explorer's state: a folders tree, the selected folder's
/// contents, and the selected file's preview.
pub struct App {
    root: FolderNode,
    tree_selected: usize,
    contents: Vec<DirectoryEntry>,
    /// The folder `contents` was listed from.
    ///
    /// Not the same as `selected_dir_path()`, which reads the tree cursor:
    /// between a move in the Folders pane and the reply arriving, the two
    /// name different folders, and every command that joined a row's name
    /// onto the tree cursor's path was addressing a file the reader could
    /// not see.
    contents_dir: PathBuf,
    contents_selected: usize,
    focus: Focus,
    file_view: Option<Response>,
    /// Which of the selected file's [`views`] the File pane is showing
    /// (#643). Reset to `0` whenever `file_view` changes to a fresh
    /// selection, so a reader never lands on a view a different file
    /// happened to leave selected.
    file_view_index: usize,
    /// The File pane's vertical scroll offset, in the wrapped lines
    /// [`render_file`] last drew (#643). Reset alongside `file_view_index`.
    file_scroll: usize,
    /// The width [`render_file`] last wrapped the File pane's text at, read
    /// back after each draw so the scroll keys can work out how many
    /// wrapped lines the current text holds without laying it out
    /// themselves. `0` until the first real draw, which [`App::file_total_lines`]
    /// reads as "not drawn yet" rather than a pane with no room at all.
    drawn_file_content_width: std::cell::Cell<u16>,
    /// How many rows of wrapped text the File pane's scrollable area last
    /// drew, read back the same way as `drawn_file_content_width`.
    drawn_file_viewport_rows: std::cell::Cell<usize>,
    status: Option<String>,
    mode: Mode,
    /// The file open in the File pane's editor, if any (#645). `None`
    /// means the File pane shows a plugin's own views, the way it always
    /// has; entering the editor is what appends `"Edit"` to
    /// [`App::file_views`] and leaves the plugin's own views hidden while
    /// it holds `Some`, the way the graphical front end's `EDIT_TAB`/
    /// `EDITING_TAB` convention already does.
    editing: Option<Editing>,
    pending_contents: Option<(Vec<usize>, Receiver<io::Result<Response>>)>,
    /// The folder the outstanding listing was asked for, so `contents_dir`
    /// can follow the entries rather than the cursor.
    pending_contents_dir: Option<PathBuf>,
    pending_file: Option<Receiver<io::Result<Response>>>,
    /// The path the outstanding file view was asked for, so a failed
    /// answer can be checked against the filesystem under its own name
    /// rather than whatever row the cursor has moved to since (#625).
    pending_file_path: Option<PathBuf>,
    pending_operation: Option<Receiver<io::Result<Response>>>,
    /// Which column the Contents pane is sorted by.
    sort_key: SortKey,
    /// Whether the current sort is ascending.
    sort_ascending: bool,
    /// What each repository row's tracked files look like, by entry name -
    /// cleared and asked for again on every fresh listing (#640).
    row_statuses: HashMap<String, RowStatus>,
    /// Working-tree status requests in flight, by the entry name they were
    /// asked about.
    pending_statuses: Vec<(String, Receiver<io::Result<Response>>)>,
    /// The index of the first Contents row the pane currently shows -
    /// follows `contents_selected` so a status is asked for only once its
    /// row scrolls into view (#641).
    contents_scroll: usize,
    /// The offset the Contents table was last drawn at, read back from
    /// its own state after each draw and handed to the next one.
    ///
    /// `App` used to keep a scroll model of its own and hope it matched:
    /// it tracked the window stickily, while a freshly defaulted
    /// `TableState` made the widget re-derive its window from the selected
    /// row alone on every frame. The two agree while a reader scrolls
    /// steadily downward and part company the moment anything jumps - a
    /// sort, for instance - after which a row plainly on screen could
    /// never be asked about and kept the not-known marker for ever (the
    /// review of #672). What is drawn is now the one answer, and the
    /// status requests are scoped by it.
    drawn_contents_offset: std::cell::Cell<Option<usize>>,
    /// How many Contents rows fit in the pane the terminal last drew, set
    /// by [`App::set_contents_viewport_rows`]. `usize::MAX` until the first
    /// real draw reports one, so a listing asked about before any terminal
    /// exists - as most of this module's own tests do - still gets every
    /// row's status asked for, matching the behaviour before scrolling was
    /// scoped at all.
    contents_viewport_rows: usize,
    /// Roots visited by stepping above the tree's own root (#642), oldest
    /// first, with [`App::history_index`] naming the one currently shown -
    /// what Back and Forward walk. Empty until the first step above the
    /// root; ordinary movement within the tree is never recorded here, the
    /// same asymmetry the graphical front end's own history keeps.
    history: Vec<PathBuf>,
    /// Which entry of [`App::history`] is currently shown.
    history_index: usize,
    /// Letters typed since [`App::type_ahead_at`], narrowing a jump to the
    /// first Folders or Contents row whose name starts with them (#642).
    /// Cleared once [`TYPE_AHEAD_TIMEOUT`] passes without another letter,
    /// or Escape is pressed.
    type_ahead_buffer: String,
    /// When the last letter was added to [`App::type_ahead_buffer`], so the
    /// next one can tell whether it extends that search or starts a fresh
    /// one.
    type_ahead_at: Option<Instant>,
    /// Set once the user has asked to quit.
    pub should_quit: bool,
    /// Whether the File pane may draw a classified [`Run`] in its class's
    /// colour (#644). Read once from the environment at construction -
    /// `NO_COLOR` set, or `TERM` reporting a terminal with none
    /// ([`colour::terminal_colour_enabled`]) turns it off - rather than on
    /// every draw, so a test can set it directly instead of mutating a
    /// process-global environment variable another test reads at the same
    /// time.
    colour_enabled: bool,
    /// The Contents pane's filter (#650): a typed name and/or the
    /// changed-only narrowing, combined.
    filter: ContentsFilter,
    /// The Folders and Contents pane widths, in columns, once the reader
    /// has resized them this session or a settings file supplied them
    /// (#650). `None` draws [`render_app`]'s original 25%/35%/40% split -
    /// the same default as before this existed.
    pane_widths: Option<settings::PaneWidths>,
    /// The width [`render_app`] last split the three panes across, read
    /// back so a resize has a starting point to nudge even before
    /// `pane_widths` holds one of its own - the same "read back after a
    /// draw" shape [`App::drawn_file_content_width`] already uses.
    drawn_panes_width: std::cell::Cell<u16>,
    /// The pane drawn alone, filling the whole terminal, if any (#650).
    /// Not a `bool` (`clippy::struct_excessive_bools`, this struct's
    /// fourth): which pane is the one worth naming here.
    maximized: Option<Focus>,
}

impl App {
    /// Starts a new explorer rooted at `root`, and kicks off loading its
    /// contents in the background.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        let mut app = Self {
            root: FolderNode::root(canonicalize_root(root)),
            tree_selected: 0,
            contents: Vec::new(),
            contents_dir: PathBuf::new(),
            contents_selected: 0,
            focus: Focus::Folders,
            file_view: None,
            file_view_index: 0,
            file_scroll: 0,
            drawn_file_content_width: std::cell::Cell::new(0),
            drawn_file_viewport_rows: std::cell::Cell::new(0),
            status: None,
            mode: Mode::Normal,
            editing: None,
            pending_contents: None,
            pending_contents_dir: None,
            pending_file: None,
            pending_file_path: None,
            pending_operation: None,
            sort_key: SortKey::Name,
            sort_ascending: true,
            row_statuses: HashMap::new(),
            pending_statuses: Vec::new(),
            contents_scroll: 0,
            drawn_contents_offset: std::cell::Cell::new(None),
            contents_viewport_rows: usize::MAX,
            history: Vec::new(),
            history_index: 0,
            type_ahead_buffer: String::new(),
            type_ahead_at: None,
            should_quit: false,
            colour_enabled: colour::terminal_colour_enabled(),
            filter: ContentsFilter::default(),
            pane_widths: None,
            drawn_panes_width: std::cell::Cell::new(0),
            maximized: None,
        };
        app.load_contents_for_selected();
        app
    }

    /// As [`App::new`], but starting with something on the status line -
    /// used to say that the Repos Directory is not set and the platform's
    /// default is being shown instead.
    #[must_use]
    pub fn new_with_notice(root: PathBuf, notice: Option<String>) -> Self {
        let mut app = Self::new(root);
        app.status = notice;
        app
    }

    /// Which pane currently has keyboard focus - so a test outside this
    /// module can tell a binding that moves it apart from one that does
    /// not.
    #[must_use]
    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// The browsed folder as one string - so a test outside this module
    /// can tell that stepping above the root (#642) actually moved it,
    /// without scraping a rendered pane a long enough name would scroll
    /// out of. Mirrors the graphical front end's own `address_path`.
    #[must_use]
    pub fn address_path(&self) -> String {
        self.selected_dir_path().to_string_lossy().into_owned()
    }

    /// The Folders and Contents pane widths, in columns, if the reader has
    /// resized them this session or a settings file supplied them - what
    /// `main` saves for the next launch (#650). `None` before either has
    /// happened, the same as a settings file with nothing usable in it.
    #[must_use]
    pub fn pane_widths(&self) -> Option<settings::PaneWidths> {
        self.pane_widths
    }

    /// Applies pane widths read from a settings file, before the first
    /// draw - what `main` calls in place of leaving [`App::pane_widths`]
    /// at its default (#650).
    pub fn set_pane_widths(&mut self, widths: settings::PaneWidths) {
        self.pane_widths = Some(widths);
    }

    fn selected_dir_path(&self) -> PathBuf {
        let rows = self.root.flatten();
        rows.get(self.tree_selected)
            .and_then(|(_, indices)| self.root.node_at(indices))
            .map_or_else(|| self.root.path.clone(), |node| node.path.clone())
    }

    fn load_contents_for_selected(&mut self) {
        let rows = self.root.flatten();
        let Some((_, indices)) = rows.get(self.tree_selected).cloned() else {
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
        self.pending_contents_dir = Some(path.clone());
        self.status = Some(format!("loading {}...", path.display()));
    }

    fn load_file_view(&mut self) {
        let Some(entry) = self.contents.get(self.contents_selected) else {
            self.file_view = None;
            self.pending_file = None;
            self.pending_file_path = None;
            self.file_view_index = 0;
            self.file_scroll = 0;
            return;
        };
        let path = self.contents_dir.join(&entry.name);
        let request = Request::ViewFile {
            path: path.to_string_lossy().into_owned(),
        };
        self.pending_file_path = Some(path);
        self.pending_file = Some(spawn_request(request));
    }

    /// Orders `contents` by the current sort column. A directory sorts
    /// before a file whichever column is chosen, the way a file explorer
    /// groups them, and the name is the tiebreak so the order is total -
    /// mirrors the graphical front end's own `sort_contents` (#640).
    fn sort_contents(&mut self) {
        let key = self.sort_key;
        let ascending = self.sort_ascending;
        self.contents.sort_by(|a, b| {
            let ordering = match key {
                SortKey::Name => std::cmp::Ordering::Equal,
                SortKey::Size => a.size.cmp(&b.size),
                SortKey::Kind => format_kind(a).cmp(&format_kind(b)),
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

    /// Sorts the Contents pane by `key`, reversing direction if it is
    /// already the sort key. The selected row keeps its selection across
    /// the reorder by following its name, not its old position (#640).
    fn set_sort_key(&mut self, key: SortKey) {
        if self.sort_key == key {
            self.sort_ascending = !self.sort_ascending;
        } else {
            self.sort_key = key;
            self.sort_ascending = true;
        }
        let selected = self
            .contents
            .get(self.contents_selected)
            .map(|entry| entry.name.clone());
        self.sort_contents();
        self.contents_selected = selected
            .as_deref()
            .and_then(|name| self.contents.iter().position(|entry| entry.name == name))
            .unwrap_or(0);
        self.clamp_contents_scroll();
        self.load_file_view();
    }

    /// Whether `entry` passes the Contents pane's current filter (#650):
    /// its name contains the typed text, case-insensitively, and/or it is
    /// a repository with uncommitted changes when the changed-only filter
    /// is on - both at once when both are set, so the two combine rather
    /// than one overriding the other.
    fn passes_filter(&self, entry: &DirectoryEntry) -> bool {
        let name_matches = self.filter.text.is_empty()
            || entry
                .name
                .to_lowercase()
                .contains(&self.filter.text.to_lowercase());
        let changed_matches =
            !self.filter.changed_only || self.marker_for(&entry.name) == CHANGED_MARKER;
        name_matches && changed_matches
    }

    /// The indices into [`App::contents`] that pass the current filter, in
    /// the listing's own (already sorted) order. Identical to every index
    /// in order when no filter is active, so nothing downstream needs a
    /// separate unfiltered path (#650).
    fn filtered_indices(&self) -> Vec<usize> {
        self.contents
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.passes_filter(entry))
            .map(|(index, _)| index)
            .collect()
    }

    /// Where [`App::contents_selected`] sits within [`App::filtered_indices`],
    /// or `None` on the one step between the filter changing and
    /// [`App::resync_filtered_selection`] moving the cursor off a row it
    /// just hid.
    fn selected_filtered_position(&self) -> Option<usize> {
        self.filtered_indices()
            .iter()
            .position(|&index| index == self.contents_selected)
    }

    /// Keeps [`App::contents_selected`] on a row the filter still shows,
    /// moving it to the first match when the row it held is no longer one.
    /// Called after anything that changes what the filter matches (#650).
    fn resync_filtered_selection(&mut self) {
        if self.selected_filtered_position().is_none()
            && let Some(&first) = self.filtered_indices().first()
        {
            self.contents_selected = first;
            self.load_file_view();
        }
        self.clamp_contents_scroll();
    }

    /// What the Contents pane shows instead of a table when the current
    /// filter matches nothing (#650): which half of the filter is
    /// responsible, so it is never mistaken for an empty Repos Directory
    /// (GUIDANCE.md §3.5). `None` when the listing is short for any other
    /// reason - nothing to filter, or nothing filtered out.
    fn filter_empty_message(&self) -> Option<String> {
        if self.contents.is_empty() || !self.filtered_indices().is_empty() {
            return None;
        }
        if !self.filter.text.is_empty() {
            Some(format!("No name matches `{}`", self.filter.text))
        } else if self.filter.changed_only {
            Some("No repositories with uncommitted changes".to_owned())
        } else {
            None
        }
    }

    /// Starts typing into the filter field (#650).
    fn start_filter(&mut self) {
        self.mode = Mode::FilterInput;
    }

    /// Handles one key while [`Mode::FilterInput`] is open: Enter keeps
    /// what has been typed and returns to browsing the narrowed listing;
    /// Escape clears the filter entirely, the way GUIDANCE.md expects a
    /// reader to always have one key back to where they started (#650).
    fn handle_filter_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Enter => self.mode = Mode::Normal,
            KeyCode::Esc => {
                self.filter = ContentsFilter::default();
                self.resync_filtered_selection();
                self.mode = Mode::Normal;
            }
            KeyCode::Backspace => {
                self.filter.text.pop();
                self.resync_filtered_selection();
            }
            KeyCode::Char(c) => {
                self.filter.text.push(c);
                self.resync_filtered_selection();
            }
            _ => {}
        }
    }

    /// Narrows the listing to repositories with uncommitted changes, or
    /// lifts that narrowing if it is already applied (#650).
    fn toggle_changed_filter(&mut self) {
        self.filter.changed_only = !self.filter.changed_only;
        self.resync_filtered_selection();
    }

    /// Asks the service for the working-tree status of each repository row
    /// in `range` that has not been asked about since the listing landed -
    /// one request per row, after the listing is already on screen: the
    /// branch is in the listing already, and the changes cost a pass over
    /// a checkout's tracked files that a listing of forty checkouts must
    /// not wait for (GUIDANCE.md §3.5, #640). `range` is the rows currently
    /// on screen (#641): a row outside it is never asked about until it
    /// scrolls into view. Positions within [`App::filtered_indices`]
    /// (#650), identical to raw listing indices while no filter narrows it.
    fn ask_for_statuses(&mut self, range: Range<usize>) {
        let filtered = self.filtered_indices();
        let end = range.end.min(filtered.len());
        let start = range.start.min(end);
        for &index in filtered.get(start..end).unwrap_or_default() {
            let entry = &self.contents[index];
            if entry.repository.is_none() || self.row_statuses.contains_key(&entry.name) {
                continue;
            }
            self.row_statuses
                .insert(entry.name.clone(), RowStatus::Waiting);
            let request = Request::WorkingTreeStatus {
                path: self
                    .contents_dir
                    .join(&entry.name)
                    .to_string_lossy()
                    .into_owned(),
            };
            self.pending_statuses
                .push((entry.name.clone(), spawn_request(request)));
        }
    }

    /// The Contents rows currently on screen, by index - [`App::tick`] asks
    /// for their statuses every iteration, and a fresh listing asks for
    /// them as soon as it lands (#641).
    /// The name the Contents pane draws for row `index`, or `None` past
    /// the end of the listing.
    #[must_use]
    pub fn content_name_at(&self, index: usize) -> Option<&str> {
        self.contents.get(index).map(|entry| entry.name.as_str())
    }

    /// The rows the front end believes are on screen: what scopes its
    /// status requests (#641), and what a test can hold against the rows
    /// actually drawn. The two were separate models once, agreeing only
    /// while a reader scrolled steadily downward (the review of #672).
    /// Positions within [`App::filtered_indices`] (#650) - identical to raw
    /// listing indices while no filter narrows the listing, which is why
    /// nothing written against this before #650 needed to change.
    #[must_use]
    pub fn visible_rows(&self) -> Range<usize> {
        self.visible_content_range()
    }

    fn visible_content_range(&self) -> Range<usize> {
        let filtered_len = self.filtered_indices().len();
        // What the table drew, when it has drawn: the clamp below is only
        // the answer before the first frame, and in a unit test that never
        // renders one.
        let start = self
            .drawn_contents_offset
            .get()
            .unwrap_or(self.contents_scroll)
            .min(filtered_len);
        let end = start
            .saturating_add(self.contents_viewport_rows)
            .min(filtered_len);
        start..end
    }

    /// Keeps `contents_selected` inside the rows [`App::visible_content_range`]
    /// reports, the way a reader expects the cursor's own row to always be
    /// on screen: scrolling up when it moves above the top, and down when
    /// it moves past the bottom. A no-op when the filter (#650) currently
    /// hides the selected row - [`App::resync_filtered_selection`] is what
    /// moves it back onto a shown one.
    fn clamp_contents_scroll(&mut self) {
        let Some(position) = self.selected_filtered_position() else {
            return;
        };
        if position < self.contents_scroll {
            self.contents_scroll = position;
        } else if self.contents_viewport_rows > 0
            && position
                >= self
                    .contents_scroll
                    .saturating_add(self.contents_viewport_rows)
        {
            self.contents_scroll = position + 1 - self.contents_viewport_rows;
        }
    }

    /// Records how many Contents rows the terminal's last draw had room
    /// for, so [`App::visible_content_range`] can scope status requests to
    /// what a reader can actually see (#641). Called from the event loop
    /// with [`crate::contents_visible_rows_for`], never from a test that
    /// draws nothing - which is why the viewport defaults to every row
    /// until this has run at least once.
    pub(crate) fn set_contents_viewport_rows(&mut self, rows: usize) {
        self.contents_viewport_rows = rows;
        self.clamp_contents_scroll();
    }

    /// How many wrapped lines the File pane's current text holds at the
    /// width it was last drawn at (#643). `usize::MAX` before the first
    /// real draw, since there is no width yet to wrap at - harmless, as
    /// nothing is on screen yet for a scroll key to move either.
    fn file_total_lines(&self) -> usize {
        let width = self.drawn_file_content_width.get();
        if width == 0 {
            return usize::MAX;
        }
        wrap_lines(&file_scrollable_lines(self), usize::from(width)).len()
    }

    /// Keeps [`App::file_scroll`] from scrolling past the last line the
    /// File pane's text can show, the way [`App::clamp_contents_scroll`]
    /// keeps the Contents cursor on screen.
    fn clamp_file_scroll(&mut self) {
        let max = self
            .file_total_lines()
            .saturating_sub(self.drawn_file_viewport_rows.get());
        self.file_scroll = self.file_scroll.min(max);
    }

    fn scroll_file_up(&mut self) {
        self.file_scroll = self.file_scroll.saturating_sub(1);
    }

    fn scroll_file_down(&mut self) {
        self.file_scroll = self.file_scroll.saturating_add(1);
        self.clamp_file_scroll();
    }

    fn scroll_file_page_up(&mut self) {
        let step = self.drawn_file_viewport_rows.get().max(1);
        self.file_scroll = self.file_scroll.saturating_sub(step);
    }

    fn scroll_file_page_down(&mut self) {
        let step = self.drawn_file_viewport_rows.get().max(1);
        self.file_scroll = self.file_scroll.saturating_add(step);
        self.clamp_file_scroll();
    }

    fn scroll_file_home(&mut self) {
        self.file_scroll = 0;
    }

    fn scroll_file_end(&mut self) {
        self.file_scroll = self
            .file_total_lines()
            .saturating_sub(self.drawn_file_viewport_rows.get());
    }

    /// The views the selected file's plugin offers, for the tab strip and
    /// for [`App::select_next_file_view`]/[`App::select_previous_file_view`].
    /// Empty when nothing is selected or its plugin is not registered here.
    ///
    /// Editing is not a plugin's idea of the file, the same as the
    /// graphical front end's own comment on `EDIT_TAB` has it: whenever the
    /// plugin offers editable text, `"Edit"` is appended here for the
    /// reader to switch to and press Enter on (#645). While
    /// [`App::editing`] holds `Some`, the plugin's own views are hidden and
    /// `"Editing"` is the only entry - one stray Left or Right away from
    /// discarding what somebody has typed is not a risk worth keeping.
    fn file_views(&self) -> Vec<&'static str> {
        if self.editing.is_some() {
            return vec!["Editing"];
        }
        match &self.file_view {
            Some(Response::FileView { plugin, data, .. }) => {
                let mut views = views(plugin, data);
                if editable_text(plugin, data).is_some() {
                    views.push("Edit");
                }
                views
            }
            _ => Vec::new(),
        }
    }

    /// Switches the File pane to the view after the one it is showing,
    /// wrapping around. Does nothing when there is only one, or none.
    fn select_next_file_view(&mut self) {
        let count = self.file_views().len();
        if count > 1 {
            self.file_view_index = (self.file_view_index + 1) % count;
            self.file_scroll = 0;
        }
    }

    /// As [`App::select_next_file_view`], the other way.
    fn select_previous_file_view(&mut self) {
        let count = self.file_views().len();
        if count > 1 {
            self.file_view_index = (self.file_view_index + count - 1) % count;
            self.file_scroll = 0;
        }
    }

    /// What Enter does in the File pane: starts editing when the tab
    /// currently shown is `"Edit"`, and does nothing otherwise (#645).
    fn activate_file_view(&mut self) {
        if self.file_views().get(self.file_view_index) == Some(&"Edit") {
            self.start_edit();
        }
    }

    /// Opens the selected file's editable text in the File pane's editor,
    /// if its plugin offers any (#645).
    fn start_edit(&mut self) {
        let Some(Response::FileView { plugin, data, .. }) = &self.file_view else {
            return;
        };
        let Some(text) = editable_text(plugin, data) else {
            return;
        };
        let Some(entry) = self.contents.get(self.contents_selected) else {
            return;
        };
        self.editing = Some(Editing {
            path: self.contents_dir.join(&entry.name),
            document: Document::new(text.clone()),
            original: text,
        });
        self.file_view_index = 0;
        self.file_scroll = 0;
    }

    /// Leaves the editor: at once if nothing has changed, or by asking
    /// first, naming the file, when it has (#645, item 4).
    fn leave_edit(&mut self) {
        let Some(editing) = &self.editing else {
            return;
        };
        if editing.document.text() == editing.original {
            self.editing = None;
            self.file_view_index = 0;
            return;
        }
        let name = editing.path.file_name().map_or_else(
            || editing.path.to_string_lossy().into_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        self.mode = Mode::ConfirmDiscardEdit { name };
    }

    fn handle_confirm_discard_edit_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('y' | 'Y') => {
                self.editing = None;
                self.file_view_index = 0;
                self.mode = Mode::Normal;
                self.status = Some("edit discarded".to_owned());
            }
            KeyCode::Char('n' | 'N') | KeyCode::Esc => self.mode = Mode::Normal,
            _ => {}
        }
    }

    /// Saves the file being edited through the service (`WriteFile`), the
    /// only process that touches the filesystem, and leaves the editor -
    /// the same "the way out is Save or Escape" shape the graphical front
    /// end's own `save_file_edit` uses. A refusal - a file that stopped
    /// being valid UTF-8 underneath the edit, say - comes back exactly as
    /// the service worded it, through the same [`App::apply_operation_result`]
    /// every other operation's errors already go through (#645, item 3).
    fn save_edit(&mut self) {
        let Some(editing) = self.editing.take() else {
            return;
        };
        self.file_view_index = 0;
        self.pending_operation = Some(spawn_request(Request::WriteFile {
            path: editing.path.to_string_lossy().into_owned(),
            content: editing.document.text().to_owned(),
        }));
        self.status = Some("saving...".to_owned());
    }

    /// Keeps the caret's line inside the File pane's editor viewport,
    /// scrolling to follow it the way [`App::clamp_contents_scroll`] keeps
    /// the Contents cursor on screen. Reuses [`App::file_scroll`] as the
    /// editor's own top-line offset - the two are never in play at once,
    /// since [`App::file_views`] shows the plugin's scrollable text or the
    /// editor, never both.
    fn follow_caret_in_editor(&mut self) {
        let Some(editing) = &self.editing else {
            return;
        };
        let line = editing.document.line_of(editing.document.caret());
        let rows = self.drawn_file_viewport_rows.get().max(1);
        if line < self.file_scroll {
            self.file_scroll = line;
        } else if line >= self.file_scroll + rows {
            self.file_scroll = line + 1 - rows;
        }
    }

    /// Handles one key while [`App::editing`] holds `Some`: Escape leaves
    /// (asking first if anything changed), Ctrl+S saves through the
    /// service, and everything else goes to [`editor::handle_key`] - which
    /// is why this is checked ahead of the binding table in
    /// [`App::handle_key`] rather than added to it: an editor has to
    /// consume every printable key, not opt into a table of them (#645).
    fn handle_editing_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            self.leave_edit();
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('s' | 'S'))
        {
            self.save_edit();
            return;
        }
        let Some(editing) = &mut self.editing else {
            return;
        };
        let rows = self.drawn_file_viewport_rows.get().max(1);
        editor::handle_key(&mut editing.document, key.code, key.modifiers, rows);
        self.follow_caret_in_editor();
    }

    /// Records the answer for the row named `name`, if it was asked about
    /// in the listing on screen.
    fn apply_status_result(&mut self, name: &str, result: io::Result<Response>) {
        let status = match result {
            Ok(Response::WorkingTree { status, .. }) => status,
            _ => None,
        };
        if let Some(row) = self.row_statuses.get_mut(name) {
            *row = RowStatus::Answered(status);
        }
    }

    /// The marker drawn beside a repository row's branch: [`CHANGED_MARKER`]
    /// for uncommitted changes, [`CANNOT_TELL_MARKER`] when the answer came
    /// back and could not say, [`NOT_KNOWN_YET_MARKER`] while still
    /// waiting, or empty for a clean working tree (#574, #640).
    fn marker_for(&self, name: &str) -> &'static str {
        match self.row_statuses.get(name) {
            None | Some(RowStatus::Waiting) => NOT_KNOWN_YET_MARKER,
            Some(RowStatus::Answered(Some(status))) if status.changed > 0 => CHANGED_MARKER,
            Some(RowStatus::Answered(Some(status))) if !status.partial => "",
            Some(RowStatus::Answered(_)) => CANNOT_TELL_MARKER,
        }
    }

    /// Applies any background request results that have arrived since the
    /// last call. Call this once per event-loop iteration.
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
            let mut view = result.unwrap_or_else(|err| Response::Error {
                message: err.to_string(),
            });
            if let (Response::Error { message }, Some(path)) = (&mut view, &asked_for) {
                *message = classify_file_problem(path, message);
            }
            self.file_view = Some(view);
            self.file_view_index = 0;
            self.file_scroll = 0;
        }
        if let Some(rx) = &self.pending_operation
            && let Ok(result) = rx.try_recv()
        {
            self.pending_operation = None;
            self.apply_operation_result(result);
        }
        let mut still_pending = Vec::with_capacity(self.pending_statuses.len());
        let mut answered = Vec::new();
        for (name, rx) in self.pending_statuses.drain(..) {
            match rx.try_recv() {
                Ok(result) => answered.push((name, result)),
                Err(mpsc::TryRecvError::Empty) => still_pending.push((name, rx)),
                Err(mpsc::TryRecvError::Disconnected) => {}
            }
        }
        self.pending_statuses = still_pending;
        for (name, result) in answered {
            self.apply_status_result(&name, result);
        }
        // Scrolling since the last tick may have brought new rows into
        // view; already-asked rows are skipped, so this costs nothing on a
        // tick where nothing moved (#641).
        self.ask_for_statuses(self.visible_content_range());
    }

    fn apply_operation_result(&mut self, result: io::Result<Response>) {
        match result {
            Ok(Response::Done) => {
                self.status = None;
                self.load_contents_for_selected();
            }
            Ok(Response::Error { message }) => self.status = Some(message),
            Ok(_) => self.status = Some("unexpected response to operation".to_owned()),
            Err(err) => self.status = Some(err.to_string()),
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
                // A listing that arrived without a recorded request - a
                // test planting one, or a reload path that did not go
                // through `load_contents_for_selected` - falls back to the
                // cursor, which is what every caller used to do.
                self.contents_dir = self
                    .pending_contents_dir
                    .take()
                    .unwrap_or_else(|| self.selected_dir_path());
                // A new listing, even of the same folder, starts its
                // statuses again: what was known belonged to the rows it
                // replaces.
                self.row_statuses.clear();
                self.pending_statuses.clear();
                self.sort_contents();
                self.contents_selected = 0;
                self.contents_scroll = 0;
                // A filter narrows one folder's listing; a different one is
                // not what it was narrowing (#650).
                self.filter = ContentsFilter::default();
                self.load_file_view();
                self.ask_for_statuses(self.visible_content_range());
            }
            Ok(Response::Error { message }) => self.status = Some(message),
            Ok(
                Response::FileView { .. }
                | Response::Done
                | Response::ReposRoots { .. }
                | Response::Names { .. }
                | Response::WorkingTree { .. }
                | Response::AllRepositories { .. }
                | Response::Certificates { .. },
            ) => {
                self.status = Some("expected a directory listing".to_owned());
            }
            Err(err) => self.status = Some(err.to_string()),
        }
    }

    /// Handles one key press.
    ///
    /// Takes anything that converts to a full [`KeyEvent`] - the real event
    /// loop hands it one straight from the terminal, modifiers and all; a
    /// bare [`KeyCode`], as most of this module's own tests still pass,
    /// converts to one with no modifiers held. Reading the whole event
    /// rather than only its code is what lets a binding tell a plain `c`
    /// apart from Ctrl+C (#638).
    pub fn handle_key(&mut self, key: impl Into<KeyEvent>) {
        let key = key.into();
        match self.mode {
            Mode::ConfirmDelete { .. } => {
                self.handle_confirm_delete_key(key.code);
                return;
            }
            Mode::RenameInput { .. } | Mode::CopyInput { .. } | Mode::ExtractInput { .. } => {
                self.handle_text_input_key(key.code);
                return;
            }
            Mode::ConfirmDiscardEdit { .. } => {
                self.handle_confirm_discard_edit_key(key.code);
                return;
            }
            Mode::FilterInput => {
                self.handle_filter_key(key.code);
                return;
            }
            Mode::Normal => {}
        }
        if self.editing.is_some() {
            self.handle_editing_key(key);
            return;
        }
        if let Some(action) = bindings::find(key, self.focus) {
            if matches!(action, Action::CancelOrQuit) {
                self.end_type_ahead();
            }
            self.dispatch(action);
            return;
        }
        // A letter the binding table does not already claim in this pane
        // starts or extends a type-ahead search (#642); one it does claim
        // keeps running that command instead, so a reader typing "readme"
        // in the Contents pane still gets Rename out of the leading `r`
        // rather than having it swallowed by the search.
        if let KeyCode::Char(c) = key.code
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            && matches!(self.focus, Focus::Folders | Focus::Contents)
        {
            self.type_ahead_key(c);
        }
    }

    /// Clears any type-ahead search in progress, without touching anything
    /// else - what Escape does to it (#642), alongside whatever the key
    /// also cancels or quits.
    fn end_type_ahead(&mut self) {
        self.type_ahead_buffer.clear();
        self.type_ahead_at = None;
    }

    /// Runs what a matched [`Action`] from the binding table means for this
    /// app.
    fn dispatch(&mut self, action: Action) {
        match action {
            Action::Quit => self.should_quit = true,
            Action::CancelOrQuit => self.cancel_or_quit(),
            Action::FocusNext => self.focus = self.focus.next(),
            Action::FocusPrevious => self.focus = self.focus.previous(),
            Action::StartDelete => self.start_delete_confirmation(),
            Action::StartRename => self.start_rename_input(),
            Action::StartCopy => self.start_copy_input(),
            Action::StartExtract => self.start_extract_input(),
            Action::FoldersUp => self.move_up_in_tree(),
            Action::FoldersDown => self.move_down_in_tree(),
            Action::FoldersExpand => self.expand_selected(),
            Action::FoldersCollapse => self.collapse_selected(),
            Action::ContentsUp => self.move_up_in_contents(),
            Action::ContentsDown => self.move_down_in_contents(),
            Action::ContentsOpen => self.drill_into_selected(),
            Action::ContentsSortName => self.set_sort_key(SortKey::Name),
            Action::ContentsSortType => self.set_sort_key(SortKey::Kind),
            Action::ContentsSortSize => self.set_sort_key(SortKey::Size),
            Action::ContentsSortModified => self.set_sort_key(SortKey::Modified),
            Action::NavigateAboveRoot => self.navigate_above_root(),
            Action::GoBack => self.go_back(),
            Action::GoForward => self.go_forward(),
            Action::FileScrollUp => self.scroll_file_up(),
            Action::FileScrollDown => self.scroll_file_down(),
            Action::FileScrollPageUp => self.scroll_file_page_up(),
            Action::FileScrollPageDown => self.scroll_file_page_down(),
            Action::FileScrollHome => self.scroll_file_home(),
            Action::FileScrollEnd => self.scroll_file_end(),
            Action::FileViewPrevious => self.select_previous_file_view(),
            Action::FileViewNext => self.select_next_file_view(),
            Action::FileActivateView => self.activate_file_view(),
            Action::StartFilter => self.start_filter(),
            Action::ToggleChangedFilter => self.toggle_changed_filter(),
            Action::WidenPane => self.widen_focused_pane(),
            Action::NarrowPane => self.narrow_focused_pane(),
            Action::ToggleMaximize => {
                self.maximized = if self.maximized.is_some() {
                    None
                } else {
                    Some(self.focus)
                };
            }
        }
    }

    fn cancel_or_quit(&mut self) {
        if self.filter.is_active() {
            self.filter = ContentsFilter::default();
            self.resync_filtered_selection();
            self.status = Some("filter cleared".to_owned());
            return;
        }
        let cancelled = self.pending_contents.take().is_some()
            | self.pending_file.take().is_some()
            | self.pending_operation.take().is_some();
        self.mode = Mode::Normal;
        if cancelled {
            self.status = Some("cancelled".to_owned());
        } else {
            self.should_quit = true;
        }
    }

    /// The Folders and Contents pane widths a resize starts nudging from
    /// when neither a settings file nor an earlier resize this session has
    /// set one (#650): [`render_app`]'s own default 25%/35% split, scaled
    /// to the last width it actually drew - or, before that has happened
    /// even once (most of this module's own tests), a fallback wide enough
    /// for both panes to clear [`settings::MIN_WIDTH`].
    fn default_pane_widths(&self) -> settings::PaneWidths {
        const FALLBACK_TOTAL_WIDTH: u16 = 80;
        let total = match self.drawn_panes_width.get() {
            0 => FALLBACK_TOTAL_WIDTH,
            width => width,
        };
        settings::PaneWidths {
            folders: (total.saturating_mul(25) / 100).max(settings::MIN_WIDTH),
            contents: (total.saturating_mul(35) / 100).max(settings::MIN_WIDTH),
        }
    }

    /// Widens the focused pane by one column, narrowing its neighbour to
    /// make room (#650): Folders takes from Contents, Contents takes from
    /// Folders, and File - which has no stored width of its own; it is
    /// whatever [`render_app`] has left - takes from Contents.
    fn widen_focused_pane(&mut self) {
        let mut widths = self
            .pane_widths
            .unwrap_or_else(|| self.default_pane_widths());
        match self.focus {
            Focus::Folders => widths.folders = widths.folders.saturating_add(1),
            Focus::Contents => widths.contents = widths.contents.saturating_add(1),
            Focus::File => {
                widths.contents = widths.contents.saturating_sub(1).max(settings::MIN_WIDTH);
            }
        }
        self.pane_widths = Some(widths);
    }

    /// As [`App::widen_focused_pane`], the other way - never below
    /// [`settings::MIN_WIDTH`].
    fn narrow_focused_pane(&mut self) {
        let mut widths = self
            .pane_widths
            .unwrap_or_else(|| self.default_pane_widths());
        match self.focus {
            Focus::Folders => {
                widths.folders = widths.folders.saturating_sub(1).max(settings::MIN_WIDTH);
            }
            Focus::Contents => {
                widths.contents = widths.contents.saturating_sub(1).max(settings::MIN_WIDTH);
            }
            Focus::File => widths.contents = widths.contents.saturating_add(1),
        }
        self.pane_widths = Some(widths);
    }

    fn start_delete_confirmation(&mut self) {
        let Some(entry) = self.contents.get(self.contents_selected) else {
            return;
        };
        let path = self.contents_dir.join(&entry.name);
        self.mode = Mode::ConfirmDelete {
            path,
            name: entry.name.clone(),
        };
    }

    fn handle_confirm_delete_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('y' | 'Y') => self.confirm_delete(),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => self.mode = Mode::Normal,
            _ => {}
        }
    }

    fn confirm_delete(&mut self) {
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

    fn start_rename_input(&mut self) {
        let Some(entry) = self.contents.get(self.contents_selected) else {
            return;
        };
        let path = self.contents_dir.join(&entry.name);
        self.mode = Mode::RenameInput {
            path,
            input: entry.name.clone(),
        };
    }

    fn start_copy_input(&mut self) {
        let Some(entry) = self.contents.get(self.contents_selected) else {
            return;
        };
        let path = self.contents_dir.join(&entry.name);
        self.mode = Mode::CopyInput {
            path,
            input: entry.name.clone(),
        };
    }

    fn start_extract_input(&mut self) {
        let Some(entry) = self.contents.get(self.contents_selected) else {
            return;
        };
        let path = self.contents_dir.join(&entry.name);
        let suggested = std::path::Path::new(&entry.name).file_stem().map_or_else(
            || entry.name.clone(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        self.mode = Mode::ExtractInput {
            path,
            input: suggested,
        };
    }

    fn handle_text_input_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Enter => self.confirm_text_input(),
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Backspace => {
                if let Some(input) = self.input_mut() {
                    input.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(input) = self.input_mut() {
                    input.push(c);
                }
            }
            _ => {}
        }
    }

    fn input_mut(&mut self) -> Option<&mut String> {
        match &mut self.mode {
            Mode::RenameInput { input, .. }
            | Mode::CopyInput { input, .. }
            | Mode::ExtractInput { input, .. } => Some(input),
            Mode::Normal
            | Mode::ConfirmDelete { .. }
            | Mode::ConfirmDiscardEdit { .. }
            | Mode::FilterInput => None,
        }
    }

    fn confirm_text_input(&mut self) {
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        let request = match mode {
            Mode::RenameInput { path, input } if !input.is_empty() => Some(Request::Rename {
                items: vec![(
                    path.to_string_lossy().into_owned(),
                    sibling_path(&path, &input),
                )],
            }),
            Mode::CopyInput { path, input } if !input.is_empty() => Some(Request::Copy {
                items: vec![(
                    path.to_string_lossy().into_owned(),
                    sibling_path(&path, &input),
                )],
            }),
            Mode::ExtractInput { path, input } if !input.is_empty() => Some(Request::Extract {
                archive: path.to_string_lossy().into_owned(),
                destination: sibling_path(&path, &input),
            }),
            _ => None,
        };
        if let Some(request) = request {
            self.pending_operation = Some(spawn_request(request));
            self.status = Some("working...".to_owned());
        }
    }

    /// The text shown on the status line: a prompt if a confirmation or
    /// text input is pending, otherwise the current status or the ambient
    /// one - the Contents pane's counts, or the default help text when it
    /// holds no repository - in full. See [`App::status_line_at`] for the
    /// version that fits a given terminal width.
    fn status_line(&self) -> String {
        match &self.mode {
            Mode::ConfirmDelete { name, .. } => format!("Delete {name}? y/n"),
            Mode::RenameInput { input, .. } => format!("Rename to: {input}_  (Enter/Esc)"),
            Mode::CopyInput { input, .. } => format!("Copy to: {input}_  (Enter/Esc)"),
            Mode::ExtractInput { input, .. } => format!("Extract to: {input}_  (Enter/Esc)"),
            Mode::ConfirmDiscardEdit { name } => format!("Discard changes to {name}? y/n"),
            Mode::FilterInput => format!("Filter: {}_  (Enter/Esc)", self.filter.text),
            Mode::Normal => self.status.clone().unwrap_or_else(|| self.ambient_status()),
        }
    }

    /// As [`App::status_line`], but the default help text is shortened to
    /// fit `width` columns rather than being cut off wherever the terminal
    /// happens to end - the bug #638 reported: a truncated help line on an
    /// 80-column terminal, hiding three of its own bindings. A prompt, a
    /// custom status message, or the Contents pane's counts (#641) is
    /// returned exactly as `status_line` gives it, since none of those is
    /// this front end's to shorten (a full reference for the help text is
    /// #649's job).
    fn status_line_at(&self, width: usize) -> String {
        match &self.mode {
            Mode::Normal if self.status.is_none() && self.contents_summary().is_empty() => {
                fit_help_line(width)
            }
            Mode::ConfirmDelete { .. }
            | Mode::RenameInput { .. }
            | Mode::CopyInput { .. }
            | Mode::ExtractInput { .. }
            | Mode::ConfirmDiscardEdit { .. }
            | Mode::FilterInput
            | Mode::Normal => self.status_line(),
        }
    }

    /// The breadcrumb line drawn above the panes (#642): the folder
    /// currently shown, fitted to `width` columns.
    fn breadcrumb_text(&self, width: usize) -> String {
        breadcrumb_line(&self.selected_dir_path(), width)
    }

    /// What [`App::status_line`] shows in [`Mode::Normal`] with nothing
    /// else to say: the Contents pane's counts (#641) when it holds a
    /// repository, with the selected row's stale-fetch words appended when
    /// it has one; the keyboard help text for a folder with no repository
    /// rows, which has nothing of that kind to say.
    fn ambient_status(&self) -> String {
        let summary = self.contents_summary();
        if summary.is_empty() {
            return HELP_SEGMENTS.join("  ");
        }
        match self.stale_fetch_note() {
            Some(note) => format!("{summary} - {note}"),
            None => summary,
        }
    }

    /// `{N} item(s), {M} repositor(y|ies){not known}{with uncommitted
    /// changes}` - what the Contents pane's listing holds, e.g. `17 items,
    /// 16 repositories, 3 with uncommitted changes` (#641). Counts the
    /// filtered listing (#650), identical to the raw one while no filter
    /// narrows it. Empty when the listing holds no repository row, so a
    /// plain folder's status line keeps its keyboard hints instead of a
    /// bare item count nobody asked for.
    fn contents_summary(&self) -> String {
        let filtered = self.filtered_indices();
        let repository_names: Vec<&str> = filtered
            .iter()
            .map(|&index| &self.contents[index])
            .filter(|entry| entry.repository.is_some())
            .map(|entry| entry.name.as_str())
            .collect();
        if repository_names.is_empty() {
            return String::new();
        }

        let items = filtered.len();
        let item_noun = if items == 1 { "item" } else { "items" };
        let repos = repository_names.len();
        let repo_noun = if repos == 1 {
            "repository"
        } else {
            "repositories"
        };
        let markers: Vec<&str> = repository_names
            .iter()
            .map(|name| self.marker_for(name))
            .collect();
        let not_known = markers
            .iter()
            .filter(|marker| matches!(**marker, NOT_KNOWN_YET_MARKER | CANNOT_TELL_MARKER))
            .count();
        let not_known = if not_known == 0 {
            String::new()
        } else {
            format!(" ({not_known} not known)")
        };
        let changed = markers
            .iter()
            .filter(|marker| **marker == CHANGED_MARKER)
            .count();
        let changed = if changed == 0 {
            String::new()
        } else {
            format!(", {changed} with uncommitted changes")
        };
        format!("{items} {item_noun}, {repos} {repo_noun}{not_known}{changed}")
    }

    /// The status line's words for the selected row's stale fetch, when it
    /// has one (#589, #641) - `None` for a clean fetch, a row that is not a
    /// repository, or no selection at all.
    fn stale_fetch_note(&self) -> Option<String> {
        let repository = self
            .contents
            .get(self.contents_selected)?
            .repository
            .as_ref()?;
        let now = now_epoch_seconds();
        fetch_is_stale(repository.last_fetch, repository.remote.is_some(), now)
            .then(|| stale_fetch_tooltip(repository.last_fetch, now))
    }

    fn move_up_in_tree(&mut self) {
        if self.tree_selected > 0 {
            self.tree_selected -= 1;
            self.load_contents_for_selected();
        }
    }

    fn move_down_in_tree(&mut self) {
        let len = self.root.flatten().len();
        if self.tree_selected + 1 < len {
            self.tree_selected += 1;
            self.load_contents_for_selected();
        }
    }

    fn expand_selected(&mut self) {
        let rows = self.root.flatten();
        let Some((_, indices)) = rows.get(self.tree_selected).cloned() else {
            return;
        };
        if let Some(node) = self.root.node_at_mut(&indices) {
            node.expanded = true;
        }
    }

    fn collapse_selected(&mut self) {
        let rows = self.root.flatten();
        let Some((_, indices)) = rows.get(self.tree_selected).cloned() else {
            return;
        };
        if let Some(node) = self.root.node_at_mut(&indices)
            && node.expanded
        {
            node.expanded = false;
            return;
        }
        if let Some((_, parent_slice)) = indices.split_last() {
            let parent_indices = parent_slice.to_vec();
            if let Some(row) = rows.iter().position(|(_, idx)| idx == &parent_indices) {
                self.tree_selected = row;
                self.load_contents_for_selected();
            }
            return;
        }
        // At the tree's own root, with nothing left to collapse: the old
        // dead end (#642) - step above it instead, exactly as the
        // dedicated parent key does.
        self.navigate_above_root();
    }

    /// Steps above the tree's own root, to its filesystem parent - "Above
    /// the root" (#642). D8's soft boundary means the Repos Directory is
    /// one configuration point, not a hard ceiling the reader cannot pass;
    /// nothing about where a fresh launch opens changes (D7), because
    /// nothing here is persisted between runs. A no-op at the filesystem
    /// root, which has no parent to step to.
    fn navigate_above_root(&mut self) {
        let Some(parent) = self.root.path.parent().map(PathBuf::from) else {
            return;
        };
        self.remember_current();
        self.push_history(parent.clone());
        self.browse(parent);
    }

    /// Re-roots the tree at `path` and browses it, without touching
    /// history - what [`App::go_back`] and [`App::go_forward`] land on,
    /// and what [`App::navigate_above_root`] does once history has
    /// recorded where it came from.
    fn browse(&mut self, path: PathBuf) {
        self.root = FolderNode::root(path);
        self.tree_selected = 0;
        self.load_contents_for_selected();
    }

    /// Records the root currently shown as the newest history entry -
    /// unless it is already there, which it is whenever nothing has moved
    /// the root since the last time this ran.
    fn remember_current(&mut self) {
        let current = self.root.path.clone();
        if self.history.is_empty() {
            self.history.push(current);
            self.history_index = 0;
        } else {
            self.push_history(current);
        }
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

    /// Whether Back has an earlier root to return to.
    fn can_go_back(&self) -> bool {
        self.history_index > 0
    }

    /// Whether Forward has a root to return to.
    fn can_go_forward(&self) -> bool {
        self.history_index + 1 < self.history.len()
    }

    /// Goes back one root in history, noting on the status line when there
    /// is nowhere to go (#642).
    fn go_back(&mut self) {
        if !self.can_go_back() {
            self.status = Some("nowhere to go back to".to_owned());
            return;
        }
        self.remember_current();
        self.history_index -= 1;
        if let Some(path) = self.history.get(self.history_index).cloned() {
            self.browse(path);
        }
    }

    /// Goes forward one root in history, noting on the status line when
    /// there is nowhere to go (#642).
    fn go_forward(&mut self) {
        if !self.can_go_forward() {
            self.status = Some("nowhere to go forward to".to_owned());
            return;
        }
        self.history_index += 1;
        if let Some(path) = self.history.get(self.history_index).cloned() {
            self.browse(path);
        }
    }

    /// Adds `c` to the type-ahead search, starting a fresh one if more
    /// than [`TYPE_AHEAD_TIMEOUT`] has passed since the last letter, then
    /// jumps the focused pane to the first row matching it (#642). Split
    /// from [`App::type_ahead_key`] so a test can supply `now` instead of
    /// waiting a real second for the timeout to matter.
    fn type_ahead_key_at(&mut self, c: char, now: Instant) {
        let stale = match self.type_ahead_at {
            Some(at) => now.saturating_duration_since(at) >= TYPE_AHEAD_TIMEOUT,
            None => true,
        };
        if stale {
            self.type_ahead_buffer.clear();
        }
        self.type_ahead_buffer.push(c.to_ascii_lowercase());
        self.type_ahead_at = Some(now);
        let prefix = self.type_ahead_buffer.clone();
        match self.focus {
            Focus::Folders => self.jump_folders_to(&prefix),
            Focus::Contents => self.jump_contents_to(&prefix),
            Focus::File => {}
        }
    }

    /// As [`App::type_ahead_key_at`], timed against the real clock.
    fn type_ahead_key(&mut self, c: char) {
        self.type_ahead_key_at(c, Instant::now());
    }

    /// Moves the Folders tree cursor to the first visible row whose name
    /// starts with `prefix`, matched without regard to case. Does nothing
    /// when no row matches, or the match is the row already selected.
    fn jump_folders_to(&mut self, prefix: &str) {
        let rows = self.root.flatten();
        let Some(index) = rows.iter().position(|(_, indices)| {
            self.root
                .node_at(indices)
                .is_some_and(|node| node.name.to_lowercase().starts_with(prefix))
        }) else {
            return;
        };
        if index != self.tree_selected {
            self.tree_selected = index;
            self.load_contents_for_selected();
        }
    }

    /// As [`App::jump_folders_to`], for the Contents pane's listing -
    /// among the rows the current filter shows (#650); identical to every
    /// row while no filter narrows it.
    fn jump_contents_to(&mut self, prefix: &str) {
        let Some(&index) = self
            .filtered_indices()
            .iter()
            .find(|&&index| self.contents[index].name.to_lowercase().starts_with(prefix))
        else {
            return;
        };
        if index != self.contents_selected {
            self.contents_selected = index;
            self.clamp_contents_scroll();
            self.load_file_view();
        }
    }

    /// Moves the cursor to the previous row the current filter shows
    /// (#650) - the previous row outright while no filter narrows the
    /// listing.
    fn move_up_in_contents(&mut self) {
        let filtered = self.filtered_indices();
        let Some(position) = filtered
            .iter()
            .position(|&index| index == self.contents_selected)
        else {
            return;
        };
        if position > 0 {
            self.contents_selected = filtered[position - 1];
            self.clamp_contents_scroll();
            self.load_file_view();
        }
    }

    /// As [`App::move_up_in_contents`], the other way.
    fn move_down_in_contents(&mut self) {
        let filtered = self.filtered_indices();
        let Some(position) = filtered
            .iter()
            .position(|&index| index == self.contents_selected)
        else {
            return;
        };
        if position + 1 < filtered.len() {
            self.contents_selected = filtered[position + 1];
            self.clamp_contents_scroll();
            self.load_file_view();
        }
    }

    fn drill_into_selected(&mut self) {
        let Some(entry) = self.contents.get(self.contents_selected) else {
            return;
        };
        if !entry.is_dir {
            return;
        }
        let entry_name = entry.name.clone();
        let rows = self.root.flatten();
        let Some((_, parent_indices)) = rows.get(self.tree_selected).cloned() else {
            return;
        };
        let Some(child_index) = self
            .root
            .node_at(&parent_indices)
            .and_then(|parent| parent.children.as_ref())
            .and_then(|children| children.iter().position(|node| node.name == entry_name))
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
            self.tree_selected = row;
        }
        self.focus = Focus::Folders;
        self.load_contents_for_selected();
    }
}

/// The default help text's pieces, in the order they are shown, joined with
/// two spaces for [`App::status_line`]'s unbounded form.
const HELP_SEGMENTS: &[&str] = &[
    "Tab: switch pane",
    "Up/Down: move",
    "Enter/Right: open",
    "Left: collapse",
    "Delete: delete",
    "r: rename",
    "c: copy",
    "x: extract",
    "Esc: cancel/quit",
    "q: quit",
];

/// Shortens [`HELP_SEGMENTS`] to fit `width` columns, keeping the first
/// segment (what a reader sees first) and the last (how to quit) and
/// dropping whole segments from the middle - never a single character off
/// a segment's end - until what remains fits.
fn fit_help_line(width: usize) -> String {
    let Some((first, rest)) = HELP_SEGMENTS.split_first() else {
        return String::new();
    };
    let Some((last, middle)) = rest.split_last() else {
        return (*first).to_owned();
    };

    let mut budget = width.saturating_sub(first.chars().count());
    let suffix_cost = 2 + last.chars().count();
    let show_suffix = budget >= suffix_cost;
    if show_suffix {
        budget -= suffix_cost;
    }

    let mut line = (*first).to_owned();
    for segment in middle {
        let cost = 2 + segment.chars().count();
        if cost > budget {
            break;
        }
        line.push_str("  ");
        line.push_str(segment);
        budget -= cost;
    }
    if show_suffix {
        line.push_str("  ");
        line.push_str(last);
    }
    line
}

/// The breadcrumb line above the panes (#642): `path`, fitted to `width`
/// columns. Shortened from the left with a leading ellipsis when it does
/// not fit, so the folder currently shown stays visible - the root that
/// anchors it is what a reader needs least once the path has grown long.
fn breadcrumb_line(path: &Path, width: usize) -> String {
    const ELLIPSIS: char = '\u{2026}';
    if width == 0 {
        return String::new();
    }
    let full = path.display().to_string();
    if full.chars().count() <= width {
        return full;
    }
    let budget = width.saturating_sub(1);
    let tail: String = full
        .chars()
        .rev()
        .take(budget)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{ELLIPSIS}{tail}")
}

fn pane_block(title: &str, focused: bool) -> Block<'_> {
    let style = if focused {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };
    Block::bordered().title(title).border_style(style)
}

/// Renders the three-pane explorer into `area` of `frame`.
///
/// A one-row-tall terminal keeps its one row for the status line, matching
/// the behaviour before #642's breadcrumb line existed: below two rows
/// there is no room to spare it one.
pub fn render_app(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let show_breadcrumb = area.height >= 2;
    let mut constraints = Vec::with_capacity(3);
    if show_breadcrumb {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Min(0));
    constraints.push(Constraint::Length(1));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);
    let (breadcrumb_area, panes_area, status_area) = if show_breadcrumb {
        (Some(rows[0]), rows[1], rows[2])
    } else {
        (None, rows[0], rows[1])
    };

    if let Some(breadcrumb_area) = breadcrumb_area {
        frame.render_widget(
            Paragraph::new(app.breadcrumb_text(breadcrumb_area.width.into())),
            breadcrumb_area,
        );
    }

    app.drawn_panes_width.set(panes_area.width);

    if let Some(pane) = app.maximized {
        render_pane(frame, panes_area, app, pane);
    } else {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(pane_constraints(app))
            .split(panes_area);

        render_folders(frame, columns[0], app);
        render_contents(frame, columns[1], app);
        render_file(frame, columns[2], app);
    }

    frame.render_widget(
        Paragraph::new(app.status_line_at(status_area.width.into())),
        status_area,
    );
}

/// The Folders/Contents/File column constraints [`render_app`] splits the
/// panes area with: the reader's own chosen widths (#650) once there are
/// any, in columns; the original 25%/35%/40% split otherwise, unchanged
/// from before this existed.
fn pane_constraints(app: &App) -> [Constraint; 3] {
    match app.pane_widths {
        Some(widths) => [
            Constraint::Length(widths.folders),
            Constraint::Length(widths.contents),
            Constraint::Min(0),
        ],
        None => [
            Constraint::Percentage(25),
            Constraint::Percentage(35),
            Constraint::Percentage(40),
        ],
    }
}

/// Renders only `pane`, filling `area` - what a maximised pane draws into
/// instead of its usual third of [`render_app`]'s own layout (#650).
fn render_pane(frame: &mut Frame<'_>, area: Rect, app: &App, pane: Focus) {
    match pane {
        Focus::Folders => render_folders(frame, area, app),
        Focus::Contents => render_contents(frame, area, app),
        Focus::File => render_file(frame, area, app),
    }
}

fn render_folders(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let rows = app.root.flatten();
    let items: Vec<ListItem<'_>> = rows
        .iter()
        .map(|(depth, indices)| {
            let node = app.root.node_at(indices);
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
            ListItem::new(format!("{}{marker} {name}/", "  ".repeat(*depth)))
        })
        .collect();

    let mut state = ListState::default();
    if !rows.is_empty() {
        state.select(Some(app.tree_selected));
    }

    let list = List::new(items)
        .block(pane_block("Folders", app.focus == Focus::Folders))
        .highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black));
    frame.render_stateful_widget(list, area, &mut state);
}

/// The entry's own name, with a trailing `/` for a directory.
fn entry_display_name(entry: &DirectoryEntry) -> String {
    if entry.is_dir {
        format!("{}/", entry.name)
    } else {
        entry.name.clone()
    }
}

/// How many characters the Type column shows before shortening a working
/// copy's own text - a character-cell budget, playing the same role the
/// graphical front end's pixel one does (#578, #640).
const KIND_COLUMN_BUDGET: usize = 18;

/// The Contents pane's "Type" column: `"File folder"` for a directory,
/// `"Git repository"` (with its provider, budget allowing) for a working
/// copy - a worktree or submodule says so instead (#587) - or the
/// uppercased extension as `"RS file"`, plain `"File"` when there is none.
/// Mirrors the graphical front end's `format_kind_of`.
fn format_kind(entry: &DirectoryEntry) -> String {
    if let Some(repository) = &entry.repository {
        return match &repository.kind {
            protocol::RepositoryKind::Worktree { .. } => {
                with_provider("Worktree", repository.provider.as_deref())
            }
            protocol::RepositoryKind::Submodule { .. } => {
                with_provider("Submodule", repository.provider.as_deref())
            }
            protocol::RepositoryKind::Clone => match &repository.provider {
                Some(provider) => {
                    let named = format!("Git repository · {provider}");
                    if named.chars().count() <= KIND_COLUMN_BUDGET {
                        named
                    } else {
                        format!("Repository · {provider}")
                    }
                }
                None => "Git repository".to_owned(),
            },
        };
    }
    if entry.is_dir {
        return "File folder".to_owned();
    }
    Path::new(&entry.name)
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

/// Formats a byte count for the Size column, e.g. `1.2 MB`.
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

/// A modified time as `YYYY-MM-DD HH:MM`, from seconds since the Unix
/// epoch. Rendered in UTC: the service reports the timestamp in epoch
/// seconds and this front end has no timezone database to convert it
/// with, so a label that is unambiguous beats one that is quietly wrong
/// by an offset.
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

/// Which optional columns the Contents pane draws this frame, and how
/// wide each shown column is - decided once from the pane's own width,
/// never from a directory read that would make a listing crawl
/// (GUIDANCE.md §3.5).
struct ContentsColumns {
    name: u16,
    branch: Option<u16>,
    kind: Option<u16>,
    size: Option<u16>,
    modified: Option<u16>,
}

const BRANCH_COLUMN_WIDTH: u16 = 16;
const KIND_COLUMN_WIDTH: u16 = 18;
const SIZE_COLUMN_WIDTH: u16 = 9;
const MODIFIED_COLUMN_WIDTH: u16 = 16;
/// The gap Ratatui's `Table` draws between two columns.
const COLUMN_SPACING: u16 = 1;
/// Below this many columns left for Name, an optional column gives up its
/// own room instead (#573, #640): Modified, then Size, then Type, then
/// Branch disappear - dropped from the right, in that order - rather than
/// Name ever being the one elided.
const NAME_FLOOR: u16 = 20;

/// Fits the Contents pane's columns to `width` (the pane's inner width,
/// borders already excluded). Size is never offered while `holds_file` is
/// false (#578), and Branch is never offered while `holds_repository` is
/// false, the same rule applied to the fact neither column would have
/// anything to show. Whichever of Modified, Size, Type and Branch remain
/// candidates give up their column in that order - right to left - before
/// the Name column is ever squeezed below [`NAME_FLOOR`] (#573).
fn plan_contents_columns(width: u16, holds_file: bool, holds_repository: bool) -> ContentsColumns {
    let mut show_modified = true;
    let mut show_size = holds_file;
    let mut show_kind = true;
    let mut show_branch = holds_repository;

    loop {
        let shown = u16::from(show_branch)
            + u16::from(show_kind)
            + u16::from(show_size)
            + u16::from(show_modified);
        let fixed = shown * COLUMN_SPACING
            + if show_branch { BRANCH_COLUMN_WIDTH } else { 0 }
            + if show_kind { KIND_COLUMN_WIDTH } else { 0 }
            + if show_size { SIZE_COLUMN_WIDTH } else { 0 }
            + if show_modified {
                MODIFIED_COLUMN_WIDTH
            } else {
                0
            };

        if width.saturating_sub(fixed) >= NAME_FLOOR || shown == 0 {
            return ContentsColumns {
                name: width.saturating_sub(fixed),
                branch: show_branch.then_some(BRANCH_COLUMN_WIDTH),
                kind: show_kind.then_some(KIND_COLUMN_WIDTH),
                size: show_size.then_some(SIZE_COLUMN_WIDTH),
                modified: show_modified.then_some(MODIFIED_COLUMN_WIDTH),
            };
        }
        if show_modified {
            show_modified = false;
        } else if show_size {
            show_size = false;
        } else if show_kind {
            show_kind = false;
        } else {
            show_branch = false;
        }
    }
}

/// `title`, with an American Standard Code for Information Interchange
/// (ASCII) arrow appended when the Contents pane is currently sorted by
/// `key` - `^` ascending, `v` descending.
fn sort_header(title: &'static str, app: &App, key: SortKey) -> Cell<'static> {
    if app.sort_key == key {
        let arrow = if app.sort_ascending { '^' } else { 'v' };
        Cell::from(format!("{title} {arrow}"))
    } else {
        Cell::from(title)
    }
}

/// The Branch column's cell: the branch name, then its change marker in
/// the warning colour when it means uncommitted changes - a glyph, not
/// colour alone (#574, #640) - and [`STALE_FETCH_MARKER`] after that when
/// the last fetch is too old to trust (#589, #641).
fn branch_cell(app: &App, entry: &DirectoryEntry) -> Cell<'static> {
    let Some(repository) = &entry.repository else {
        return Cell::from("");
    };
    let branch = repository
        .branch
        .clone()
        .unwrap_or_else(|| "detached".to_owned());
    let marker = app.marker_for(&entry.name);
    let stale = fetch_is_stale(
        repository.last_fetch,
        repository.remote.is_some(),
        now_epoch_seconds(),
    );
    if marker.is_empty() && !stale {
        return Cell::from(branch);
    }
    let mut spans = vec![Span::raw(format!("{branch} "))];
    if !marker.is_empty() {
        let marker_style = if marker == CHANGED_MARKER {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        spans.push(Span::styled(marker, marker_style));
    }
    if stale {
        if !marker.is_empty() {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::raw(STALE_FETCH_MARKER));
    }
    Cell::from(Line::from(spans))
}

/// One Contents row's cells, fitted to `columns`.
fn contents_row(app: &App, entry: &DirectoryEntry, columns: &ContentsColumns) -> Row<'static> {
    let mut cells = vec![Cell::from(entry_display_name(entry))];
    if columns.branch.is_some() {
        cells.push(branch_cell(app, entry));
    }
    if columns.kind.is_some() {
        cells.push(Cell::from(format_kind(entry)));
    }
    if columns.size.is_some() {
        let size = if entry.is_dir {
            String::new()
        } else {
            format_size(entry.size)
        };
        cells.push(Cell::from(size));
    }
    if columns.modified.is_some() {
        cells.push(Cell::from(format_timestamp(effective_modified(entry))));
    }
    Row::new(cells)
}

/// Renders the Contents pane as a table fitted to what the listing holds
/// (#578, #640): Name, then Branch, Type, Size and Modified as room and
/// the listing's own contents allow, sorted by [`App::sort_key`], and
/// narrowed to what the current filter shows (#650) - a message in place
/// of the table when that filter matches nothing.
fn render_contents(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = pane_block("Contents", app.focus == Focus::Contents);
    if let Some(message) = app.filter_empty_message() {
        frame.render_widget(
            Paragraph::new(message)
                .wrap(Wrap { trim: false })
                .block(block),
            area,
        );
        return;
    }

    let holds_file = app.contents.iter().any(|entry| !entry.is_dir);
    let holds_repository = app.contents.iter().any(|entry| entry.repository.is_some());
    let inner_width = area.width.saturating_sub(2);
    let columns = plan_contents_columns(inner_width, holds_file, holds_repository);

    let mut widths = vec![Constraint::Length(columns.name)];
    let mut header_cells = vec![sort_header("Name", app, SortKey::Name)];
    if let Some(width) = columns.branch {
        widths.push(Constraint::Length(width));
        header_cells.push(Cell::from("Branch"));
    }
    if let Some(width) = columns.kind {
        widths.push(Constraint::Length(width));
        header_cells.push(sort_header("Type", app, SortKey::Kind));
    }
    if let Some(width) = columns.size {
        widths.push(Constraint::Length(width));
        header_cells.push(sort_header("Size", app, SortKey::Size));
    }
    if let Some(width) = columns.modified {
        widths.push(Constraint::Length(width));
        header_cells.push(sort_header("Modified", app, SortKey::Modified));
    }

    let filtered = app.filtered_indices();
    let rows: Vec<Row<'static>> = filtered
        .iter()
        .map(|&index| contents_row(app, &app.contents[index], &columns))
        .collect();

    let mut state = TableState::default();
    // Carried from the last frame, so the table scrolls the way a reader
    // expects rather than re-deriving its window from the selected row
    // each time - and so what it draws can be read back below.
    if let Some(offset) = app.drawn_contents_offset.get() {
        *state.offset_mut() = offset.min(filtered.len());
    }
    if let Some(position) = filtered
        .iter()
        .position(|&index| index == app.contents_selected)
    {
        state.select(Some(position));
    }

    let table = Table::new(rows, widths)
        .header(Row::new(header_cells))
        .column_spacing(COLUMN_SPACING)
        .block(block)
        .row_highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black));
    frame.render_stateful_widget(table, area, &mut state);
    // The widget has just decided which rows fit; that decision is what
    // scopes the status requests (#641).
    app.drawn_contents_offset.set(Some(state.offset()));
}

/// What the File pane says above the selected repository's own view when
/// it is a worktree or a submodule (#587, #641): the clone or outer
/// working copy it belongs to, or that the clone is no longer there.
/// `None` for an ordinary clone, or a selection that is not a repository
/// row at all.
fn related_repository_line(app: &App) -> Option<String> {
    let repository = app
        .contents
        .get(app.contents_selected)?
        .repository
        .as_ref()?;
    match &repository.kind {
        protocol::RepositoryKind::Clone => None,
        protocol::RepositoryKind::Worktree {
            clone,
            clone_exists,
        } => Some(if *clone_exists {
            format!(
                "Worktree of {}",
                related_repository_name(Path::new(clone), &app.root.path)
            )
        } else {
            format!("Worktree of a clone that is no longer at {clone}")
        }),
        protocol::RepositoryKind::Submodule { outer } => Some(format!(
            "Submodule of {}",
            related_repository_name(Path::new(outer), &app.root.path)
        )),
    }
}

/// `path`'s folder name, with where it is in parentheses when that says
/// more than the name alone does: its path relative to `root` when it is
/// inside it, the full path otherwise - left off when `path` is a direct
/// child of `root`, where the name already says where it is (#587).
/// Mirrors the graphical front end's own `related_repository_name_of`.
fn related_repository_name(path: &Path, root: &Path) -> String {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let located = path.strip_prefix(root).map_or_else(
        |_| path.display().to_string(),
        |relative| relative.to_string_lossy().replace('\\', "/"),
    );
    if located == name {
        name
    } else {
        format!("{name} (at {located})")
    }
}

/// The selected repository's README (#584), read straight from the
/// `directory` plugin's own view data rather than through [`facts`] or
/// [`present`]: the fact table only ever holds label/value pairs, and
/// `directory` always has at least the entry-count fact, so its own
/// `present` lines - which do carry the README - never reach the pane.
/// Mirrors the graphical front end's own `readme_excerpt_of`.
fn directory_readme(
    plugin: &str,
    data: &serde_json::Value,
) -> Option<plugin_directory::readme::ReadmeExcerpt> {
    if plugin != "directory" {
        return None;
    }
    serde_json::from_value::<plugin_directory::DirectoryView>(data.clone())
        .ok()
        .and_then(|view| view.readme)
}

/// The plugin's own lines for the view named `showing` (or its default
/// view, for `None`), classified into [`Run`]s where the plugin has
/// something to say about its syntax (#644).
///
/// The lines themselves are exactly [`present_view`]'s (or [`present`]'s):
/// classifying never changes what is drawn, only how - what has no
/// `content` to classify, or whose plugin classifies nothing, comes back
/// as one plain run per line. A view that is not the file's own text
/// (`TEXT_VIEW`) is only partly classified: `coloured_lines` is spliced in
/// under whatever summary a plugin like `rust`'s prepends to it, found by
/// [`colour::file_starts_in`], and left alone entirely when that summary
/// does not simply end with the file (a `Table` view, say).
fn coloured_view_lines(
    plugin: &str,
    showing: Option<&str>,
    data: &serde_json::Value,
) -> Vec<Vec<Run>> {
    let plain = match showing {
        Some(view) => present_view(plugin, view, data),
        None => present(plugin, data),
    };
    let plain_runs = || plain.iter().map(|line| colour::plain_line(line)).collect();
    let Some(text) = data.get("content").and_then(serde_json::Value::as_str) else {
        return plain_runs();
    };
    let spans = classify(plugin, text);
    if spans.is_empty() {
        return plain_runs();
    }
    let coloured = colour::coloured_lines(text, &spans);
    if showing == Some(plugin_api::TEXT_VIEW) {
        return coloured;
    }
    let Some(from) = colour::file_starts_in(&plain, text) else {
        return plain_runs();
    };
    let mut lines: Vec<Vec<Run>> = plain[..from]
        .iter()
        .map(|line| colour::plain_line(line))
        .collect();
    lines.extend(coloured);
    lines
}

/// The File pane's scrollable text, as classified [`Run`]s (#643, #644):
/// the plugin's own lines for the view currently shown, when it offers no
/// fact table (`facts` says that instead); a note that a picture is not
/// shown here, for a type whose view is one; a folder plugin's stacked
/// lines below that, never in place of what is already there (D12); and a
/// repository's README below those (#584). Everything but the plugin's own
/// lines is plain - only its own text is the plugin's to classify.
fn file_scrollable_runs(app: &App) -> Vec<Vec<Run>> {
    let Some(Response::FileView { plugin, data, also }) = &app.file_view else {
        return Vec::new();
    };
    let plugin_views = app.file_views();
    let view_index = app
        .file_view_index
        .min(plugin_views.len().saturating_sub(1));
    let showing = plugin_views.get(view_index).copied();

    let mut lines = if facts(plugin, data).is_empty() {
        coloured_view_lines(plugin, showing, data)
    } else {
        Vec::new()
    };
    if showing == Some(plugin_api::PREVIEW_VIEW) && graphic(plugin, data).is_some() {
        lines.insert(
            0,
            colour::plain_line("Picture - shown in the graphical front end, not here."),
        );
    }
    for extra in also {
        lines.push(Vec::new());
        lines.extend(
            present_folder(&extra.plugin, &extra.data)
                .iter()
                .map(|line| colour::plain_line(line)),
        );
    }
    if let Some(readme) = directory_readme(plugin, data) {
        lines.push(Vec::new());
        lines.push(colour::plain_line(
            &readme.title.unwrap_or_else(|| "README".to_owned()),
        ));
        lines.extend(readme.excerpt.iter().map(|line| colour::plain_line(line)));
    }
    lines
}

/// The File pane's scrollable text, as plain strings - [`file_scrollable_runs`]
/// with each line's runs joined back together, so a scroll bound computed
/// from this and a row drawn from the runs never disagree on how many
/// rows a line wraps into (#643).
fn file_scrollable_lines(app: &App) -> Vec<String> {
    file_scrollable_runs(app)
        .iter()
        .map(|line| line.iter().map(|run| run.text.as_str()).collect())
        .collect()
}

/// How wide the File pane's fact table gives its label column - enough for
/// "Last fetched" without leaving a wide gutter beside a short one like
/// "Branch" in a narrow pane.
const FACT_LABEL_WIDTH: u16 = 14;

/// The File pane's fact table (#576, #643): a label and its value per row,
/// dimmed per [`plugin_api::Fact::dim`] - the folder-count divider a
/// repository's own facts end with, for instance.
fn facts_table(rows: &[Fact], width: u16) -> Table<'static> {
    let label_width = FACT_LABEL_WIDTH.min(width / 2);
    let table_rows: Vec<Row<'static>> = rows
        .iter()
        .map(|fact| {
            let style = if fact.dim {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(fact.label.clone()),
                Cell::from(fact.value.clone()),
            ])
            .style(style)
        })
        .collect();
    Table::new(
        table_rows,
        [Constraint::Length(label_width), Constraint::Min(0)],
    )
    .column_spacing(1)
}

/// Wraps `line` at `width` characters, so a line wider than the pane
/// continues on the row below it rather than being cut (#643). Character
/// count, not measured display width - the same per-cell simplification
/// `format_kind`'s column budget already makes, with no grapheme-width
/// table to consult.
fn wrap_line(line: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![line.to_owned()];
    }
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    chars
        .chunks(width)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

/// [`wrap_line`], over every line of text.
fn wrap_lines(lines: &[String], width: usize) -> Vec<String> {
    lines
        .iter()
        .flat_map(|line| wrap_line(line, width))
        .collect()
}

/// A [`Run`] drawn in its class's colour (#644), or in none at all when
/// `enabled` is false - `NO_COLOR` set, or a terminal that reports it
/// supports none (`colour::terminal_colour_enabled`) - so a class is never
/// stood in for by a colour a terminal does not have.
fn run_span(run: &Run, enabled: bool) -> Span<'static> {
    let style = if enabled {
        colour::class_colour(run.class).map_or_else(Style::default, |fg| Style::default().fg(fg))
    } else {
        Style::default()
    };
    Span::styled(run.text.clone(), style)
}

/// Draws the File pane's scrollable text into `area`, wrapped rather than
/// cut at the pane's width, with a [`Scrollbar`] beside it (#643), coloured
/// the way the file's own plugin classifies it (#644). Reads back the
/// width and row count it drew at into `app`'s cells, so the scroll keys -
/// handled long after this returns - know what is on screen.
fn render_file_text(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let lines = file_scrollable_runs(app);
    let scrollbar_width = u16::from(!lines.is_empty());
    let text_width = area.width.saturating_sub(scrollbar_width);
    let text_area = Rect {
        width: text_width,
        ..area
    };

    let wrapped: Vec<Vec<Run>> = lines
        .iter()
        .flat_map(|line| colour::wrap_runs(line, usize::from(text_width)))
        .collect();
    let total_lines = wrapped.len();
    let viewport_rows = usize::from(area.height);
    app.drawn_file_content_width.set(text_width);
    app.drawn_file_viewport_rows.set(viewport_rows);
    let scroll = app
        .file_scroll
        .min(total_lines.saturating_sub(viewport_rows));

    let visible: Vec<Line<'static>> = wrapped
        .get(scroll..)
        .unwrap_or_default()
        .iter()
        .map(|runs| {
            Line::from(
                runs.iter()
                    .map(|run| run_span(run, app.colour_enabled))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(visible), text_area);

    if scrollbar_width > 0 {
        let scrollbar_area = Rect {
            x: area.x + text_width,
            width: scrollbar_width,
            ..area
        };
        let mut scrollbar_state = ScrollbarState::new(total_lines).position(scroll);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            scrollbar_area,
            &mut scrollbar_state,
        );
    }
}

/// The runs `classify` gives for the byte range `start..end` of `text` -
/// one document line - clipped to that line's own bounds, or the whole
/// range as one plain run when `spans` is empty (a plugin with nothing to
/// say about its syntax, the same as [`colour::plain_line`] for a view
/// that is not being edited).
///
/// A span may run past a line's end (a block comment usually does): the
/// clip is what keeps it from bleeding into a row it does not belong to,
/// since the editor draws one document line per row rather than the
/// wrapped rows [`render_file_text`] does.
fn line_runs(spans: &[PluginSpan], start: usize, end: usize) -> Vec<(Range<usize>, Class)> {
    if spans.is_empty() {
        return if start == end {
            Vec::new()
        } else {
            vec![(start..end, Class::Plain)]
        };
    }
    spans
        .iter()
        .filter_map(|span| {
            let clip_start = span.start.max(start);
            let clip_end = (span.start + span.len).min(end);
            (clip_start < clip_end).then_some((clip_start..clip_end, span.class))
        })
        .collect()
}

/// Splits `range` into the parts of it outside `selection` and the part
/// inside, each tagged with whether it is selected - up to three pieces,
/// in order, empty ones dropped.
fn split_by_selection(
    range: Range<usize>,
    selection: Option<&Range<usize>>,
) -> Vec<(Range<usize>, bool)> {
    let Some(selection) = selection else {
        return vec![(range, false)];
    };
    let sel_start = selection.start.clamp(range.start, range.end);
    let sel_end = selection.end.clamp(range.start, range.end);
    [
        (range.start..sel_start, false),
        (sel_start..sel_end, true),
        (sel_end..range.end, false),
    ]
    .into_iter()
    .filter(|(part, _)| !part.is_empty())
    .collect()
}

/// One run of the editor's text as a styled [`Span`], coloured by `class`
/// the way [`run_span`] colours a read-only run, and drawn reversed when
/// `selected` - the caret's own selection, the terminal's ordinary way of
/// showing one, in place of the background colour Slint's `TextInput`
/// gives the graphical front end's plain editor.
fn editing_span(text: &str, class: Class, selected: bool, colour_enabled: bool) -> Span<'static> {
    let mut style = if colour_enabled {
        colour::class_colour(class).map_or_else(Style::default, |fg| Style::default().fg(fg))
    } else {
        Style::default()
    };
    if selected {
        style = style.add_modifier(ratatui::style::Modifier::REVERSED);
    }
    Span::styled(text.to_owned(), style)
}

/// One document line, `start..end` of `text`, as the styled spans a
/// [`Line`] draws: [`line_runs`]'s classified runs, each split by
/// `selection` into up to three pieces so a selection that starts or ends
/// mid-run still highlights exactly the bytes it covers.
fn editing_line_spans(
    text: &str,
    document_spans: &[PluginSpan],
    start: usize,
    end: usize,
    selection: Option<&Range<usize>>,
    colour_enabled: bool,
) -> Vec<Span<'static>> {
    line_runs(document_spans, start, end)
        .into_iter()
        .flat_map(|(range, class)| {
            split_by_selection(range, selection)
                .into_iter()
                .map(move |(part, selected)| (part, class, selected))
        })
        .map(|(part, class, selected)| editing_span(&text[part], class, selected, colour_enabled))
        .collect()
}

/// Draws the File pane's editor into `area`: the document being edited,
/// one document line per row, scrolled to keep the caret's line in view
/// (#645). Coloured the way a read-only view already is - `classify`
/// knows nothing of a caret, so the terminal's own cursor is what shows
/// where it is, placed by [`Frame::set_cursor_position`] rather than drawn
/// by hand, the same reasoning GUIDANCE.md §3.6 gives for leaving input
/// method editor (IME) composition, accessibility and right-to-left text
/// to the host terminal here.
fn render_file_editing(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(editing) = &app.editing else {
        return;
    };
    let plugin = match &app.file_view {
        Some(Response::FileView { plugin, .. }) => plugin.as_str(),
        _ => "",
    };
    let text = editing.document.text();
    let document_spans = classify(plugin, text);
    let selection = editing.document.selection();
    let caret = editing.document.caret();
    let caret_line = editing.document.line_of(caret);
    let caret_column = editing.document.column_of(caret);

    let viewport_rows = usize::from(area.height);
    app.drawn_file_content_width.set(area.width);
    app.drawn_file_viewport_rows.set(viewport_rows);

    let line_count = editing.document.line_count();
    let scroll = app.file_scroll.min(line_count.saturating_sub(1));

    let visible: Vec<Line<'static>> = (scroll..line_count.min(scroll + viewport_rows))
        .map(|index| {
            let start = editing.document.line_start(index);
            let end = editing.document.line_end(index);
            Line::from(editing_line_spans(
                text,
                &document_spans,
                start,
                end,
                selection.as_ref(),
                app.colour_enabled,
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(visible), area);

    let cursor_row = caret_line.saturating_sub(scroll);
    if cursor_row < viewport_rows {
        let x = area.x
            + u16::try_from(caret_column)
                .unwrap_or(u16::MAX)
                .min(area.width.saturating_sub(1));
        let y = area.y + u16::try_from(cursor_row).unwrap_or(0);
        frame.set_cursor_position((x, y));
    }
}

fn render_file(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = pane_block("File", app.focus == Focus::File);
    if app.editing.is_some() {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(inner);
        let tabs = Tabs::new(app.file_views())
            .select(0)
            .highlight_style(Style::default().fg(Color::Yellow));
        frame.render_widget(tabs, rows[0]);
        render_file_editing(frame, rows[1], app);
        return;
    }
    let Some(response) = &app.file_view else {
        frame.render_widget(Paragraph::new("(no file selected)").block(block), area);
        return;
    };
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let related = related_repository_line(app);

    let Response::FileView { plugin, data, .. } = response else {
        // Every other reply - an error, or a kind this front end never
        // asks the File pane to show - keeps the plain path it always had,
        // still led by the related-repository line when there is one.
        match related {
            Some(line) => {
                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(0)])
                    .split(inner);
                frame.render_widget(Paragraph::new(line), rows[0]);
                render_with_block(frame, rows[1], response, Block::default());
            }
            None => render_with_block(frame, inner, response, Block::default()),
        }
        return;
    };

    let plugin_views = app.file_views();
    let show_tabs = plugin_views.len() > 1;
    let repository_facts = facts(plugin, data);

    let mut constraints = Vec::with_capacity(4);
    if show_tabs {
        constraints.push(Constraint::Length(1));
    }
    if related.is_some() {
        constraints.push(Constraint::Length(1));
    }
    if !repository_facts.is_empty() {
        let rows = u16::try_from(repository_facts.len()).unwrap_or(u16::MAX);
        constraints.push(Constraint::Length(rows));
    }
    constraints.push(Constraint::Min(0));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let mut row = 0;
    if show_tabs {
        let index = app
            .file_view_index
            .min(plugin_views.len().saturating_sub(1));
        let tabs = Tabs::new(plugin_views.iter().copied())
            .select(index)
            .highlight_style(Style::default().fg(Color::Yellow));
        frame.render_widget(tabs, rows[row]);
        row += 1;
    }
    if let Some(line) = related {
        frame.render_widget(Paragraph::new(line), rows[row]);
        row += 1;
    }
    if !repository_facts.is_empty() {
        frame.render_widget(facts_table(&repository_facts, rows[row].width), rows[row]);
        row += 1;
    }
    render_file_text(frame, rows[row], app);
}

#[cfg(test)]
mod tests {
    use super::{
        App, CHANGED_MARKER, Document, Editing, Focus, FolderNode, Mode, NOT_KNOWN_YET_MARKER,
        RowStatus, STALE_FETCH_MARKER, breadcrumb_line, render_app, render_contents,
        render_file_editing, settings,
    };
    use protocol::{DirectoryEntry, ReposRoot, Response};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::layout::Rect;
    use ratatui::style::Color;
    use std::path::{Path, PathBuf};

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

    #[test]
    fn flattens_only_expanded_directories() {
        let mut root = FolderNode::root("/root".into());
        root.set_children_from(&entries(&[("a", true), ("b.txt", false), ("c", true)]));
        // Root is expanded by default, so its two directory children ("a"
        // and "c") are already visible; "b.txt" is filtered out entirely.
        assert_eq!(root.flatten().len(), 3);

        // "a"'s own children stay hidden until "a" itself is expanded.
        root.children.as_mut().unwrap()[0].set_children_from(&entries(&[("grandchild", true)]));
        assert_eq!(root.flatten().len(), 3);

        root.children.as_mut().unwrap()[0].expanded = true;
        let rows = root.flatten();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0], (0, vec![]));
        assert_eq!(rows[1], (1, vec![0]));
        assert_eq!(rows[2], (2, vec![0, 0]));
        assert_eq!(rows[3], (1, vec![1]));
    }

    #[test]
    fn focus_cycles_forward_and_backward() {
        assert_eq!(Focus::Folders.next(), Focus::Contents);
        assert_eq!(Focus::Contents.next(), Focus::File);
        assert_eq!(Focus::File.next(), Focus::Folders);
        assert_eq!(Focus::Folders.previous(), Focus::File);
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

        assert_eq!(app.contents.len(), 2);
        assert_eq!(app.contents_selected, 0);
        assert_eq!(app.root.children.as_ref().unwrap().len(), 1);
        assert_eq!(app.root.children.as_ref().unwrap()[0].name, "sub");
    }

    #[test]
    fn applying_an_error_sets_status_and_leaves_contents_empty() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Error {
                message: "boom".to_owned(),
            }),
        );

        assert!(app.contents.is_empty());
        assert_eq!(app.status.as_deref(), Some("boom"));
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
        assert_eq!(app.contents.len(), 1);
    }

    #[test]
    fn cancelling_a_pending_request_makes_a_late_result_harmless() {
        let mut app = App::new(std::env::temp_dir());
        let (tx, rx) = std::sync::mpsc::channel();
        app.pending_contents = Some((vec![], rx));

        app.handle_key(KeyCode::Esc);
        assert!(app.pending_contents.is_none());
        assert_eq!(app.status.as_deref(), Some("cancelled"));

        // The sender outlives the (now dropped) receiver; sending must not
        // panic, and the late result is simply lost.
        let send_result = tx.send(Ok(Response::Directory { entries: vec![] }));
        assert!(send_result.is_err());
    }

    #[test]
    fn esc_quits_when_nothing_is_pending() {
        let mut app = App::new(std::env::temp_dir());
        app.pending_contents = None;
        app.pending_file = None;

        app.handle_key(KeyCode::Esc);

        assert!(app.should_quit);
    }

    #[test]
    fn contents_selection_does_not_move_past_the_edges() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("a", false)]),
            }),
        );

        app.handle_key(KeyCode::Up);
        assert_eq!(app.contents_selected, 0);

        app.handle_key(KeyCode::Down);
        assert_eq!(app.contents_selected, 0);
    }

    fn app_with_one_content_entry() -> App {
        // A notional folder that does not exist, and is named per process
        // so two runs cannot meet. Rooting this at `temp_dir()` itself
        // meant confirming a delete sent a real `Request::Delete` for
        // `<temp>/note.txt` - so on a machine with the service running and
        // such a file present, running the tests recycled it.
        let mut app =
            App::new(std::env::temp_dir().join(format!("rse-tui-notional-{}", std::process::id())));
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("note.txt", false)]),
            }),
        );
        app.focus = Focus::Contents;
        app
    }

    #[test]
    fn delete_key_on_contents_asks_for_confirmation() {
        let mut app = app_with_one_content_entry();

        app.handle_key(KeyCode::Delete);

        assert_eq!(app.status_line(), "Delete note.txt? y/n");
        assert!(app.pending_operation.is_none());
    }

    #[test]
    fn declining_the_delete_confirmation_returns_to_normal_without_a_request() {
        let mut app = app_with_one_content_entry();
        app.handle_key(KeyCode::Delete);

        app.handle_key(KeyCode::Char('n'));

        assert!(app.pending_operation.is_none());
        assert_ne!(app.status_line(), "Delete note.txt? y/n");
    }

    #[test]
    fn esc_during_delete_confirmation_cancels_without_quitting() {
        let mut app = app_with_one_content_entry();
        app.handle_key(KeyCode::Delete);

        app.handle_key(KeyCode::Esc);

        assert!(!app.should_quit);
        assert_ne!(app.status_line(), "Delete note.txt? y/n");
    }

    #[test]
    fn confirming_the_delete_sends_a_request_for_exactly_that_path() {
        let mut app = app_with_one_content_entry();
        app.handle_key(KeyCode::Delete);

        app.handle_key(KeyCode::Char('y'));

        assert!(app.pending_operation.is_some());
        assert_eq!(app.status_line(), "deleting...");
    }

    #[test]
    fn a_successful_delete_result_reloads_contents() {
        let mut app = app_with_one_content_entry();
        app.status = Some("deleting...".to_owned());

        app.apply_operation_result(Ok(Response::Done));

        // load_contents_for_selected() ran again: a fresh request is in
        // flight, and its own "loading..." status has replaced "deleting...".
        assert!(app.pending_contents.is_some());
        assert_ne!(app.status.as_deref(), Some("deleting..."));
    }

    #[test]
    fn a_failed_delete_result_surfaces_the_error() {
        let mut app = app_with_one_content_entry();

        app.apply_operation_result(Ok(Response::Error {
            message: "permission denied".to_owned(),
        }));

        assert_eq!(app.status.as_deref(), Some("permission denied"));
    }

    #[test]
    fn rename_key_prefills_the_input_with_the_current_name() {
        let mut app = app_with_one_content_entry();

        app.handle_key(KeyCode::Char('r'));

        assert_eq!(app.status_line(), "Rename to: note.txt_  (Enter/Esc)");
    }

    #[test]
    fn editing_the_rename_input_appends_and_backspaces() {
        let mut app = app_with_one_content_entry();
        app.handle_key(KeyCode::Char('r'));

        app.handle_key(KeyCode::Backspace);
        app.handle_key(KeyCode::Char('!'));

        assert_eq!(app.status_line(), "Rename to: note.tx!_  (Enter/Esc)");
    }

    #[test]
    fn confirming_a_rename_sends_a_request_for_the_sibling_path() {
        let mut app = app_with_one_content_entry();
        app.handle_key(KeyCode::Char('r'));
        for _ in 0..8 {
            app.handle_key(KeyCode::Backspace);
        }
        for c in "renamed.txt".chars() {
            app.handle_key(KeyCode::Char(c));
        }

        app.handle_key(KeyCode::Enter);

        assert!(app.pending_operation.is_some());
        assert_eq!(app.status_line(), "working...");
    }

    #[test]
    fn esc_during_rename_input_cancels_without_a_request() {
        let mut app = app_with_one_content_entry();
        app.handle_key(KeyCode::Char('r'));

        app.handle_key(KeyCode::Esc);

        assert!(app.pending_operation.is_none());
        assert_ne!(app.status_line(), "Rename to: note.txt_  (Enter/Esc)");
    }

    #[test]
    fn copy_key_prefills_the_input_with_the_current_name() {
        let mut app = app_with_one_content_entry();

        app.handle_key(KeyCode::Char('c'));

        assert_eq!(app.status_line(), "Copy to: note.txt_  (Enter/Esc)");
    }

    #[test]
    fn confirming_a_copy_sends_a_request() {
        let mut app = app_with_one_content_entry();
        app.handle_key(KeyCode::Char('c'));

        app.handle_key(KeyCode::Enter);

        assert!(app.pending_operation.is_some());
        assert_eq!(app.status_line(), "working...");
    }

    #[test]
    fn extract_key_prefills_the_input_with_the_archive_stem() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("bundle.zip", false)]),
            }),
        );
        app.focus = Focus::Contents;

        app.handle_key(KeyCode::Char('x'));

        assert_eq!(app.status_line(), "Extract to: bundle_  (Enter/Esc)");
    }

    #[test]
    fn confirming_an_extract_sends_a_request() {
        let mut app = App::new(std::env::temp_dir());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("bundle.zip", false)]),
            }),
        );
        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Char('x'));

        app.handle_key(KeyCode::Enter);

        assert!(app.pending_operation.is_some());
        assert_eq!(app.status_line(), "working...");
    }

    #[test]
    fn confirming_an_empty_rename_input_does_not_send_a_request() {
        let mut app = app_with_one_content_entry();
        app.handle_key(KeyCode::Char('r'));
        for _ in 0.."note.txt".len() {
            app.handle_key(KeyCode::Backspace);
        }

        app.handle_key(KeyCode::Enter);

        assert!(app.pending_operation.is_none());
    }

    #[test]
    fn a_path_on_the_command_line_wins() {
        let opening = super::opening_from(
            Some(std::path::PathBuf::from("/somewhere/else")),
            Some((
                vec![ReposRoot {
                    path: "/home/ada/repos".to_owned(),
                    active: true,
                }],
                "/home/ada/repos".to_owned(),
            )),
        );

        assert_eq!(opening.root, std::path::PathBuf::from("/somewhere/else"));
        assert_eq!(
            opening.notice, None,
            "an explicit instruction needs no explaining"
        );
    }

    #[test]
    fn with_nothing_on_the_command_line_it_opens_at_the_configured_root() {
        let opening = super::opening_from(
            None,
            Some((
                vec![
                    ReposRoot {
                        path: "/mnt/work/repos".to_owned(),
                        active: false,
                    },
                    ReposRoot {
                        path: "/home/ada/repos".to_owned(),
                        active: true,
                    },
                ],
                "/home/ada/repos".to_owned(),
            )),
        );

        assert_eq!(opening.root, std::path::PathBuf::from("/home/ada/repos"));
        assert_eq!(opening.notice, None);
    }

    #[test]
    fn with_no_root_configured_it_offers_the_default_and_says_so() {
        let opening = super::opening_from(None, Some((Vec::new(), "/home/ada/repos".to_owned())));

        assert_eq!(opening.root, std::path::PathBuf::from("/home/ada/repos"));
        let notice = opening.notice.expect("an unset root should be explained");
        assert!(notice.contains("No Repos Directory set"), "{notice}");
        assert!(
            notice.contains("/home/ada/repos"),
            "and it should name what it is showing instead: {notice}"
        );
    }

    #[test]
    fn with_no_service_it_falls_back_and_says_why() {
        let opening = super::opening_from(None, None);

        assert_eq!(opening.root, std::path::PathBuf::from("."));
        assert!(
            opening
                .notice
                .expect("a front end that cannot ask should say so")
                .contains("Could not ask the service")
        );
    }

    #[test]
    fn the_type_column_names_a_working_copy_and_its_provider() {
        let checkout = DirectoryEntry {
            name: "explorer".to_owned(),
            is_dir: true,
            size: 0,
            modified: None,
            repository: Some(protocol::RepositoryInfo {
                provider: Some("github.com".to_owned()),
                branch: Some("main".to_owned()),
                remote: Some("https://github.com/owner/explorer.git".to_owned()),
                kind: protocol::RepositoryKind::Clone,
                last_activity: None,
                last_fetch: None,
            }),
        };
        let folder = DirectoryEntry {
            name: "scratch".to_owned(),
            is_dir: true,
            size: 0,
            modified: None,
            repository: None,
        };

        assert_eq!(
            super::format_kind(&checkout),
            "Repository · github.com",
            "the full \"Git repository · provider\" already overruns an 18-character budget"
        );
        assert_eq!(
            super::format_kind(&folder),
            "File folder",
            "a folder that is not a checkout stays listed, and stays plain"
        );
        assert_eq!(super::entry_display_name(&checkout), "explorer/");
        assert_eq!(super::entry_display_name(&folder), "scratch/");
    }

    #[test]
    fn a_working_copy_with_no_remote_still_says_it_is_one() {
        let checkout = DirectoryEntry {
            name: "local-only".to_owned(),
            is_dir: true,
            size: 0,
            modified: None,
            repository: Some(protocol::RepositoryInfo::default()),
        };

        assert_eq!(super::format_kind(&checkout), "Git repository");
    }

    #[test]
    fn a_long_provider_shortens_to_keep_the_type_column_readable() {
        let checkout = DirectoryEntry {
            name: "explorer".to_owned(),
            is_dir: true,
            size: 0,
            modified: None,
            repository: Some(protocol::RepositoryInfo {
                provider: Some("an-extremely-long-self-hosted-git-provider.example.com".to_owned()),
                branch: None,
                remote: None,
                kind: protocol::RepositoryKind::Clone,
                last_activity: None,
                last_fetch: None,
            }),
        };

        assert_eq!(
            super::format_kind(&checkout),
            "Repository · an-extremely-long-self-hosted-git-provider.example.com",
            "the full \"Git repository · provider\" still overruns the column, \
             so it gives way to the shorter form that keeps the provider"
        );
    }

    // ---------------------------------------------------------------------
    // Helpers for the tests below.
    // ---------------------------------------------------------------------

    /// A directory under the temp directory that is never created and never
    /// written to.
    ///
    /// `App` touches no file itself - every path it holds is a string it
    /// hands to the service - so naming a directory nothing answers to keeps
    /// these tests off any real file even on a machine where a service
    /// happens to be listening.
    fn notional_root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rse-tui-tests-{}-{tag}", std::process::id()))
    }

    /// An app showing `listing` as the contents of `root`, with nothing left
    /// in flight, so a test starts from a settled screen.
    fn app_showing(root: &Path, listing: &[(&str, bool)]) -> App {
        let mut app = App::new(root.to_path_buf());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(listing),
            }),
        );
        app.pending_contents = None;
        app.pending_file = None;
        app
    }

    /// Which mode the app is in, as a word a failure message can carry.
    fn mode_of(app: &App) -> &'static str {
        match app.mode {
            Mode::Normal => "normal",
            Mode::ConfirmDelete { .. } => "confirm-delete",
            Mode::RenameInput { .. } => "rename",
            Mode::CopyInput { .. } => "copy",
            Mode::ExtractInput { .. } => "extract",
            Mode::ConfirmDiscardEdit { .. } => "confirm-discard-edit",
            Mode::FilterInput => "filter",
        }
    }

    /// The Contents pane's entry names, in the order it currently shows
    /// them - what a sort test compares against.
    fn content_names(app: &App) -> Vec<&str> {
        app.contents
            .iter()
            .map(|entry| entry.name.as_str())
            .collect()
    }

    /// The entry names the current filter (#650) actually leaves standing,
    /// in listing order - what a filter test compares against, as opposed
    /// to [`content_names`]'s unfiltered raw listing.
    fn filtered_content_names(app: &App) -> Vec<&str> {
        app.filtered_indices()
            .iter()
            .map(|&index| app.contents[index].name.as_str())
            .collect()
    }

    /// The path the open prompt is about, whichever prompt it is.
    fn prompt_path(app: &App) -> Option<&Path> {
        match &app.mode {
            Mode::ConfirmDelete { path, .. }
            | Mode::RenameInput { path, .. }
            | Mode::CopyInput { path, .. }
            | Mode::ExtractInput { path, .. } => Some(path.as_path()),
            Mode::Normal | Mode::ConfirmDiscardEdit { .. } | Mode::FilterInput => None,
        }
    }

    /// What `render_app` puts on a `width` x `height` terminal, one string
    /// per row.
    fn drawn_rows(width: u16, height: u16, app: &App) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        terminal
            .draw(|frame| render_app(frame, frame.area(), app))
            .expect("a draw into the test backend");
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    /// As [`drawn_rows`], run together into one string.
    fn drawn(width: u16, height: u16, app: &App) -> String {
        drawn_rows(width, height, app).concat()
    }

    /// As [`drawn_rows`], but each cell's foreground colour rather than its
    /// glyph (#644) - what a colour assertion reads, at the same
    /// coordinates `drawn_rows` finds a needle at.
    fn drawn_colours(width: u16, height: u16, app: &App) -> Vec<Vec<Color>> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        terminal
            .draw(|frame| render_app(frame, frame.area(), app))
            .expect("a draw into the test backend");
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].fg).collect())
            .collect()
    }

    /// What [`render_contents`] alone draws into a `width` x `height`
    /// rect, one string per row - the Contents pane's own table, without
    /// the rest of the three-pane layout narrowing it further (#640).
    fn drawn_contents(width: u16, height: u16, app: &App) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        terminal
            .draw(|frame| render_contents(frame, frame.area(), app))
            .expect("a draw into the test backend");
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    /// A repository row for the table tests below: a checkout on `branch`,
    /// with a provider so the Type column has something to shorten.
    fn repository_entry(name: &str, branch: &str) -> DirectoryEntry {
        DirectoryEntry {
            name: name.to_owned(),
            is_dir: true,
            size: 0,
            modified: None,
            repository: Some(protocol::RepositoryInfo {
                provider: Some("github.com".to_owned()),
                branch: Some(branch.to_owned()),
                remote: Some("https://github.com/owner/repo.git".to_owned()),
                kind: protocol::RepositoryKind::Clone,
                last_activity: None,
                last_fetch: None,
            }),
        }
    }

    /// As [`repository_entry`], but with no remote configured - so it is
    /// never stale (#589) - for a test whose own subject is something
    /// other than the stale-fetch mark.
    fn repository_entry_without_remote(name: &str, branch: &str) -> DirectoryEntry {
        DirectoryEntry {
            repository: Some(protocol::RepositoryInfo {
                remote: None,
                ..repository_entry(name, branch).repository.unwrap()
            }),
            ..repository_entry(name, branch)
        }
    }

    // ---------------------------------------------------------------------
    // The Contents pane as a table fitted to a Repos Directory, and it
    // sorts (#640).
    // ---------------------------------------------------------------------

    #[test]
    fn every_column_shows_at_120_columns_and_some_drop_as_it_narrows() {
        let root = notional_root("column-widths");
        let mut app = App::new(root);
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![
                    repository_entry("checkout", "main"),
                    DirectoryEntry {
                        name: "notes.txt".to_owned(),
                        is_dir: false,
                        size: 42,
                        modified: Some(0),
                        repository: None,
                    },
                ],
            }),
        );

        let text_at = |width: u16| drawn_contents(width, 6, &app).concat();

        let wide = text_at(120);
        assert!(wide.contains("Branch"), "{wide}");
        assert!(wide.contains("Type"), "{wide}");
        assert!(wide.contains("Size"), "{wide}");
        assert!(wide.contains("Modified"), "{wide}");

        let medium = text_at(80);
        assert!(
            !medium.contains("Modified"),
            "80 columns should already have dropped the least essential column: {medium}"
        );

        let narrow = text_at(40);
        assert!(
            !narrow.contains("Modified") && !narrow.contains("Size") && !narrow.contains("Type"),
            "40 columns should hold only Name and Branch: {narrow}"
        );
        assert!(narrow.contains("checkout"), "{narrow}");
    }

    #[test]
    fn a_long_branch_gives_up_its_room_before_the_name_is_touched() {
        let root = notional_root("long-branch");
        let mut app = App::new(root);
        let long_branch = "an-extremely-long-branch-name-that-does-not-fit-in-any-column";
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![repository_entry("project", long_branch)],
            }),
        );

        let text = drawn_contents(60, 6, &app).concat();

        assert!(text.contains("project"), "the name must be whole: {text}");
        assert!(
            !text.contains(long_branch),
            "the branch must have given up its room before the name was touched: {text}"
        );
    }

    #[test]
    fn the_size_column_shows_only_once_a_file_is_in_the_listing() {
        let root = notional_root("size-column");
        let mut app = app_showing(&root, &[("alpha", true), ("beta", true)]);
        let folders_only = drawn_contents(60, 6, &app).concat();
        assert!(!folders_only.contains("Size"), "{folders_only}");

        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("alpha", true), ("notes.txt", false)]),
            }),
        );
        let with_a_file = drawn_contents(60, 6, &app).concat();
        assert!(with_a_file.contains("Size"), "{with_a_file}");
    }

    #[test]
    fn sorting_by_size_orders_files_and_keeps_folders_first_both_directions() {
        let root = notional_root("sort-by-size");
        let mut app = App::new(root);
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![
                    DirectoryEntry {
                        name: "z-folder".to_owned(),
                        is_dir: true,
                        size: 0,
                        modified: None,
                        repository: None,
                    },
                    DirectoryEntry {
                        name: "small.txt".to_owned(),
                        is_dir: false,
                        size: 10,
                        modified: None,
                        repository: None,
                    },
                    DirectoryEntry {
                        name: "large.txt".to_owned(),
                        is_dir: false,
                        size: 1000,
                        modified: None,
                        repository: None,
                    },
                ],
            }),
        );
        app.focus = Focus::Contents;

        app.handle_key(KeyCode::Char('s'));
        assert_eq!(
            app.contents
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["z-folder", "small.txt", "large.txt"],
            "folders first, then ascending by size"
        );

        app.handle_key(KeyCode::Char('s'));
        assert_eq!(
            app.contents
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["z-folder", "large.txt", "small.txt"],
            "pressing the same sort key again reverses direction, folders still first"
        );
    }

    #[test]
    fn sorting_by_name_type_and_modified_all_keep_folders_first_both_directions() {
        let root = notional_root("sort-by-every-key");
        let mut app = App::new(root);
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![
                    DirectoryEntry {
                        name: "z-folder".to_owned(),
                        is_dir: true,
                        size: 0,
                        modified: Some(1),
                        repository: None,
                    },
                    DirectoryEntry {
                        name: "a.md".to_owned(),
                        is_dir: false,
                        size: 0,
                        modified: Some(200),
                        repository: None,
                    },
                    DirectoryEntry {
                        name: "b.txt".to_owned(),
                        is_dir: false,
                        size: 0,
                        modified: Some(100),
                        repository: None,
                    },
                ],
            }),
        );
        app.focus = Focus::Contents;

        // The listing is already sorted ascending by name (the default),
        // so the first press of its own key reverses it to descending.
        assert_eq!(content_names(&app), vec!["z-folder", "a.md", "b.txt"]);
        app.handle_key(KeyCode::Char('n'));
        assert_eq!(content_names(&app), vec!["z-folder", "b.txt", "a.md"]);
        app.handle_key(KeyCode::Char('n'));
        assert_eq!(content_names(&app), vec!["z-folder", "a.md", "b.txt"]);

        // Type: "MD file" sorts before "TXT file".
        app.handle_key(KeyCode::Char('t'));
        assert_eq!(content_names(&app), vec!["z-folder", "a.md", "b.txt"]);
        app.handle_key(KeyCode::Char('t'));
        assert_eq!(content_names(&app), vec!["z-folder", "b.txt", "a.md"]);

        // Modified: b.txt (100) is older than a.md (200).
        app.handle_key(KeyCode::Char('m'));
        assert_eq!(content_names(&app), vec!["z-folder", "b.txt", "a.md"]);
        app.handle_key(KeyCode::Char('m'));
        assert_eq!(content_names(&app), vec!["z-folder", "a.md", "b.txt"]);
    }

    #[test]
    fn the_change_marker_reads_as_a_glyph_not_only_a_colour() {
        let root = notional_root("change-marker");
        let mut app = App::new(root);
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![repository_entry("checkout", "main")],
            }),
        );
        app.row_statuses.insert(
            "checkout".to_owned(),
            RowStatus::Answered(Some(protocol::WorkingTreeSummary {
                changed: 3,
                partial: false,
                summary: "3 tracked files changed".to_owned(),
            })),
        );

        let text = drawn_contents(60, 6, &app).concat();

        assert!(
            text.contains(CHANGED_MARKER),
            "the marker must be a glyph a reader can see even with no colour drawn at all: {text}"
        );
    }

    // ---------------------------------------------------------------------
    // Which file an operation is about.
    // ---------------------------------------------------------------------

    #[test]
    fn delete_names_the_file_in_the_listing_the_reader_can_see() {
        let root = notional_root("stale-listing-delete");
        let mut app = app_showing(
            &root,
            &[("alpha", true), ("beta", true), ("notes.txt", false)],
        );

        // Walk the folders cursor down to "beta". Its listing has been asked
        // for but has not arrived, so the contents pane is still showing the
        // root's - which is what the reader is looking at and picking from.
        app.handle_key(KeyCode::Down);
        app.handle_key(KeyCode::Down);
        assert_eq!(
            app.contents.len(),
            3,
            "the pane is still showing the root's listing while beta's is fetched"
        );

        app.handle_key(KeyCode::Tab);
        app.handle_key(KeyCode::Delete);

        assert_eq!(
            prompt_path(&app),
            Some(root.join("alpha").as_path()),
            "the question must be about the row the reader picked, in the folder \
             they can see - not about a namesake in whichever folder the tree \
             cursor has since landed on"
        );
    }

    #[test]
    fn a_listing_that_lands_under_an_open_prompt_does_not_change_which_file_it_names() {
        let root = notional_root("prompt-holds-its-file");
        let mut app = app_showing(&root, &[("alpha", true), ("notes.txt", false)]);
        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Down);
        app.handle_key(KeyCode::Delete);

        // A listing asked for earlier arrives while the question stands.
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("zebra.txt", false)]),
            }),
        );

        assert_eq!(
            app.contents_selected, 0,
            "the listing under the question has moved out from under it"
        );
        assert_eq!(
            prompt_path(&app),
            Some(root.join("notes.txt").as_path()),
            "but the question is still about the file it was asked about"
        );
        assert_eq!(app.status_line(), "Delete notes.txt? y/n");
    }

    #[test]
    fn the_prompt_sits_on_the_last_row_and_names_the_file_it_holds() {
        let root = notional_root("prompt-on-the-status-row");
        let mut app = app_showing(&root, &[("alpha", true), ("notes.txt", false)]);
        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Down);
        app.handle_key(KeyCode::Delete);

        let rows = drawn_rows(40, 8, &app);

        assert!(
            rows[7].starts_with("Delete notes.txt? y/n"),
            "the question belongs on the status row, naming its own file: {:?}",
            rows[7]
        );
    }

    // ---------------------------------------------------------------------
    // Key dispatch: no key means one thing in two modes at once.
    // ---------------------------------------------------------------------

    #[test]
    fn no_command_key_runs_while_a_delete_confirmation_is_open() {
        let root = notional_root("delete-question-is-modal");
        let mut app = app_showing(&root, &[("alpha", true), ("notes.txt", false)]);
        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Down);
        app.handle_key(KeyCode::Delete);
        assert_eq!(prompt_path(&app), Some(root.join("notes.txt").as_path()));

        for code in [
            KeyCode::Char('q'),
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Delete,
            KeyCode::Char('r'),
            KeyCode::Char('c'),
            KeyCode::Char('x'),
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Enter,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Backspace,
        ] {
            app.handle_key(code);
            assert_eq!(
                mode_of(&app),
                "confirm-delete",
                "{code:?} got out from under an unanswered question"
            );
        }

        assert!(
            !app.should_quit,
            "q must not quit out from under a question"
        );
        assert_eq!(
            app.focus,
            Focus::Contents,
            "Tab must not move focus away from the pane the question is about"
        );
        assert_eq!(
            app.contents_selected, 1,
            "and the listing must not move under it"
        );
        assert!(
            app.pending_operation.is_none(),
            "no command ran while the question stood"
        );
        assert_eq!(app.status_line(), "Delete notes.txt? y/n");
    }

    #[test]
    fn delete_pressed_inside_a_rename_prompt_does_not_arm_a_deletion() {
        let root = notional_root("delete-inside-a-rename");
        let mut app = app_showing(&root, &[("notes.txt", false)]);
        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Char('r'));

        app.handle_key(KeyCode::Delete);

        assert_eq!(
            mode_of(&app),
            "rename",
            "Delete inside a rename turned it into a deletion"
        );
        assert_eq!(
            app.status_line(),
            "Rename to: notes.txt_  (Enter/Esc)",
            "and it must not have edited the name either"
        );

        // The next keystroke belongs to the rename, not to a question that
        // was never asked.
        app.handle_key(KeyCode::Char('y'));
        assert_eq!(app.status_line(), "Rename to: notes.txty_  (Enter/Esc)");
        assert!(app.pending_operation.is_none());
    }

    #[test]
    fn q_typed_into_a_rename_prompt_is_a_letter_not_a_quit() {
        let root = notional_root("q-inside-a-rename");
        let mut app = app_showing(&root, &[("notes.txt", false)]);
        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Char('r'));

        app.handle_key(KeyCode::Char('q'));

        assert!(!app.should_quit, "q while typing a name must not quit");
        assert_eq!(app.status_line(), "Rename to: notes.txtq_  (Enter/Esc)");
    }

    #[test]
    fn a_prompt_holds_the_listing_still_while_it_is_open() {
        let root = notional_root("listing-held-still");
        let mut app = app_showing(&root, &[("alpha", true), ("notes.txt", false)]);
        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Down);
        app.handle_key(KeyCode::Char('c'));

        app.handle_key(KeyCode::Up);
        app.handle_key(KeyCode::Down);
        app.handle_key(KeyCode::Tab);

        assert_eq!(
            app.contents_selected, 1,
            "movement keys must not move the listing an open prompt is about"
        );
        assert_eq!(app.focus, Focus::Contents);
        assert_eq!(mode_of(&app), "copy");
        assert_eq!(
            app.status_line(),
            "Copy to: notes.txt_  (Enter/Esc)",
            "and they must not have been typed into the name either"
        );
    }

    #[test]
    fn the_operation_keys_only_answer_in_the_pane_that_holds_files() {
        let root = notional_root("operations-are-per-pane");
        let mut app = app_showing(&root, &[("notes.txt", false)]);

        for focus in [Focus::Folders, Focus::File] {
            app.focus = focus;
            for code in [
                KeyCode::Delete,
                KeyCode::Char('r'),
                KeyCode::Char('c'),
                KeyCode::Char('x'),
            ] {
                app.handle_key(code);
                assert_eq!(
                    mode_of(&app),
                    "normal",
                    "{code:?} acted on the contents pane while {focus:?} had focus"
                );
            }
        }

        assert!(app.pending_operation.is_none());
        assert!(!app.should_quit);
    }

    #[test]
    fn j_and_k_move_the_pane_that_has_focus_and_leave_the_other_where_it_was() {
        let root = notional_root("vim-keys-follow-focus");
        let mut app = app_showing(&root, &[("alpha", true), ("beta", true)]);

        app.handle_key(KeyCode::Char('j'));
        assert_eq!(app.tree_selected, 1, "the tree has focus, so j moved it");
        assert_eq!(
            app.contents_selected, 0,
            "the listing moved on a key meant for the tree"
        );

        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Char('j'));
        assert_eq!(app.contents_selected, 1);
        assert_eq!(
            app.tree_selected, 1,
            "the tree moved on a key meant for the listing"
        );

        app.focus = Focus::File;
        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('k'));
        assert_eq!(
            (app.tree_selected, app.contents_selected),
            (1, 1),
            "the preview pane has no rows, so its movement keys move nothing"
        );
    }

    // ---------------------------------------------------------------------
    // Selection arithmetic at the edges.
    // ---------------------------------------------------------------------

    #[test]
    fn the_listing_stops_at_each_end_however_often_the_key_is_pressed() {
        let root = notional_root("listing-edges");
        let mut app = app_showing(&root, &[("a", false), ("b", false), ("c", false)]);
        app.focus = Focus::Contents;

        for _ in 0..6 {
            app.handle_key(KeyCode::Down);
        }
        assert_eq!(app.contents_selected, 2, "the last row is the last row");

        for _ in 0..6 {
            app.handle_key(KeyCode::Up);
        }
        assert_eq!(app.contents_selected, 0, "and the first is the first");
    }

    #[test]
    fn an_empty_listing_ignores_every_key_that_would_need_a_row() {
        let root = notional_root("empty-listing");
        let mut app = app_showing(&root, &[]);
        app.focus = Focus::Contents;

        for code in [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Enter,
            KeyCode::Right,
            KeyCode::Delete,
            KeyCode::Char('r'),
            KeyCode::Char('c'),
            KeyCode::Char('x'),
        ] {
            app.handle_key(code);
            assert_eq!(
                mode_of(&app),
                "normal",
                "{code:?} opened a prompt over a listing with nothing in it"
            );
        }

        assert_eq!(app.contents_selected, 0);
        assert!(app.pending_operation.is_none());
        assert!(!app.should_quit);
        assert!(
            app.file_view.is_none(),
            "and there is nothing to preview either"
        );
    }

    #[test]
    fn a_tree_with_only_its_root_does_not_move_or_ask_for_anything() {
        let mut app = App::new(notional_root("one-row-tree"));
        app.pending_contents = None;

        app.handle_key(KeyCode::Up);
        app.handle_key(KeyCode::Down);

        assert_eq!(app.tree_selected, 0);
        assert!(
            app.pending_contents.is_none(),
            "nothing moved, so nothing should have been fetched again"
        );
    }

    #[test]
    fn a_tree_cursor_past_the_last_row_falls_back_to_the_root() {
        let root = notional_root("tree-cursor-past-the-end");
        let mut app = app_showing(&root, &[("alpha", true)]);
        app.tree_selected = 99;

        assert_eq!(
            app.selected_dir_path(),
            root,
            "a row that is not there resolves to the root, not to a panic"
        );

        app.load_contents_for_selected();
        assert!(
            app.pending_contents.is_none(),
            "and a row that is not there is not fetched"
        );

        let rows = drawn_rows(40, 8, &app);
        assert_eq!(rows.len(), 8, "and the screen still draws");
    }

    #[test]
    fn a_refreshed_listing_puts_the_selection_back_at_the_top() {
        // Nothing here remembers the row by name across a reload: once an
        // operation finishes and the listing comes back, the first row is
        // selected again wherever the reader had been. Recorded because the
        // graphical front end does put the selection back.
        let root = notional_root("reload-resets-the-row");
        let mut app = app_showing(&root, &[("a", false), ("b", false), ("c", false)]);
        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Down);
        app.handle_key(KeyCode::Down);
        assert_eq!(app.contents_selected, 2);

        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("a", false), ("b", false), ("c", false)]),
            }),
        );

        assert_eq!(app.contents_selected, 0);
    }

    // ---------------------------------------------------------------------
    // Layout against a terminal with no room in it.
    // ---------------------------------------------------------------------

    #[test]
    fn the_three_panes_draw_into_a_terminal_with_no_room_for_them() {
        let app = app_showing(
            &notional_root("tiny-terminal"),
            &[("alpha", true), ("notes.txt", false)],
        );

        for (width, height) in [
            (0, 0),
            (0, 4),
            (4, 0),
            (1, 1),
            (2, 1),
            (3, 2),
            (8, 3),
            (1, 40),
        ] {
            let rows = drawn_rows(width, height, &app);
            assert_eq!(
                rows.len(),
                height as usize,
                "{width}x{height} did not fill the terminal it was given"
            );
        }
    }

    #[test]
    fn a_terminal_one_row_tall_gives_that_row_to_the_status_line() {
        let app = app_showing(&notional_root("one-row-tall"), &[("notes.txt", false)]);

        let rows = drawn_rows(30, 1, &app);

        assert!(
            rows[0].starts_with("Tab: switch pane"),
            "with room for one row it should be the one that says what the keys do: {:?}",
            rows[0]
        );
    }

    #[test]
    fn a_terminal_narrower_than_the_pane_titles_still_fills_every_row() {
        let app = app_showing(&notional_root("narrower-than-a-title"), &[("a.txt", false)]);

        let rows = drawn_rows(6, 3, &app);

        assert_eq!(rows.len(), 3);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(
                row.chars().count(),
                6,
                "row {index} came out {} cells wide: {row:?}",
                row.chars().count()
            );
        }
    }

    #[test]
    fn breadcrumb_line_shows_the_full_path_when_it_fits_a_wide_terminal() {
        let path = Path::new("/repos/project");

        assert_eq!(breadcrumb_line(path, 40), "/repos/project");
    }

    #[test]
    fn breadcrumb_line_is_shortened_from_the_left_on_a_narrow_terminal() {
        let path = Path::new("/very/deeply/nested/repos/project");

        let line = breadcrumb_line(path, 15);

        assert_eq!(line.chars().count(), 15);
        assert!(
            line.starts_with('\u{2026}'),
            "a leading ellipsis marks where it was cut: {line:?}"
        );
        assert!(
            line.ends_with("project"),
            "the folder being shown stays visible: {line:?}"
        );
    }

    #[test]
    fn the_breadcrumb_line_is_drawn_above_the_panes() {
        let root = notional_root("breadcrumb-wired-in");
        let app = app_showing(&root, &[("alpha", true)]);

        let rows = drawn_rows(80, 8, &app);

        assert!(
            rows[0].contains(&root.display().to_string()),
            "the browsed folder's path is the first row, at a wide enough terminal: {:?}",
            rows[0]
        );
    }

    #[test]
    fn a_name_wider_than_its_pane_is_cut_to_the_pane() {
        let long = "an-extremely-long-file-name-that-no-pane-here-could-possibly-hold.txt";
        let app = app_showing(&notional_root("long-names"), &[(long, false)]);

        let rows = drawn_rows(40, 6, &app);

        assert!(
            !rows.iter().any(|row| row.contains(long)),
            "the name spilled out of the pane that is meant to hold it: {rows:?}"
        );
        assert!(
            rows.iter().any(|row| row.contains("an-extr")),
            "and the start of it should still be readable: {rows:?}"
        );
    }

    #[test]
    fn multibyte_names_draw_at_every_width_without_splitting_a_character() {
        let app = app_showing(
            &notional_root("multibyte-listing"),
            &[("日本語のフォルダ", true), ("café-notes.txt", false)],
        );

        for width in 1..24_u16 {
            let text = drawn(width, 6, &app);
            assert!(
                !text.contains('\u{fffd}'),
                "a pane {width} cells wide cut a character in half"
            );
        }
    }

    #[test]
    fn a_multibyte_name_survives_the_whole_prompt_round_trip() {
        let root = notional_root("multibyte-prompt");
        let mut app = app_showing(&root, &[("日本語.tar.gz", false)]);
        app.focus = Focus::Contents;

        app.handle_key(KeyCode::Char('x'));
        assert_eq!(app.status_line(), "Extract to: 日本語.tar_  (Enter/Esc)");

        // Backspace takes a character off, not a byte: taking a byte off the
        // end of "語" is where a name like this panics.
        app.handle_key(KeyCode::Backspace);
        assert_eq!(app.status_line(), "Extract to: 日本語.ta_  (Enter/Esc)");

        app.handle_key(KeyCode::Esc);
        app.handle_key(KeyCode::Char('r'));
        assert_eq!(app.status_line(), "Rename to: 日本語.tar.gz_  (Enter/Esc)");
        for _ in 0..7 {
            app.handle_key(KeyCode::Backspace);
        }
        assert_eq!(app.status_line(), "Rename to: 日本語_  (Enter/Esc)");
    }

    // ---------------------------------------------------------------------
    // What a confirmed prompt asks the service for.
    // ---------------------------------------------------------------------

    #[test]
    fn a_new_name_lands_beside_the_old_one_rather_than_inside_it() {
        let old = PathBuf::from("/repos/project").join("notes.txt");

        assert_eq!(
            super::sibling_path(&old, "read-me.txt"),
            PathBuf::from("/repos/project")
                .join("read-me.txt")
                .to_string_lossy()
        );
    }

    #[test]
    fn a_name_with_nothing_to_sit_beside_is_used_as_it_stands() {
        assert_eq!(
            super::sibling_path(Path::new("/"), "read-me.txt"),
            "read-me.txt",
            "a path with no parent leaves the bare name, not an empty one"
        );
    }

    #[test]
    fn a_typed_name_containing_a_separator_leaves_the_folder_it_was_meant_to_stay_in() {
        // `Mode::RenameInput` is documented as a new name "within its own
        // directory", but nothing holds the typed text to a single
        // component: `..` walks out, and the service takes the destination
        // as given. Recorded so the next reader can see that it is the
        // prompt, not the service, that would have to say no.
        let old = PathBuf::from("/repos/project").join("notes.txt");

        assert_eq!(
            super::sibling_path(&old, "../../escaped.txt"),
            PathBuf::from("/repos/project")
                .join("../../escaped.txt")
                .to_string_lossy()
        );
    }

    #[test]
    fn a_whitespace_only_name_is_accepted_and_sent_as_typed() {
        // Only an *empty* name is refused. A name of one space is sent, and
        // would create a file called " ". Recorded rather than asserted as
        // right: the graphical front end does the same for these three
        // prompts, so it is a shared decision, not a slip in one of them.
        let root = notional_root("whitespace-name");
        let mut app = app_showing(&root, &[("notes.txt", false)]);
        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Char('r'));
        for _ in 0.."notes.txt".len() {
            app.handle_key(KeyCode::Backspace);
        }
        app.handle_key(KeyCode::Char(' '));
        assert_eq!(app.status_line(), "Rename to:  _  (Enter/Esc)");

        app.handle_key(KeyCode::Enter);

        assert!(app.pending_operation.is_some());
        assert_eq!(mode_of(&app), "normal");
    }

    #[test]
    fn an_empty_copy_or_extract_name_sends_nothing_and_closes_the_prompt() {
        let root = notional_root("empty-names");

        for key in [KeyCode::Char('c'), KeyCode::Char('x')] {
            let mut app = app_showing(&root, &[("bundle.zip", false)]);
            app.focus = Focus::Contents;
            app.handle_key(key);
            for _ in 0.."bundle.zip".len() {
                app.handle_key(KeyCode::Backspace);
            }

            app.handle_key(KeyCode::Enter);

            assert!(
                app.pending_operation.is_none(),
                "{key:?} sent a request with no name in it"
            );
            assert_eq!(
                mode_of(&app),
                "normal",
                "{key:?} left its prompt open with nothing to confirm"
            );
            assert!(app.status_line().starts_with("Tab: switch pane"));
        }
    }

    #[test]
    fn the_extract_prompt_suggests_what_is_left_after_the_last_extension() {
        let root = notional_root("extract-stems");

        for (name, suggested) in [
            ("bundle.tar.gz", "bundle.tar"),
            (".zip", ".zip"),
            ("no-extension", "no-extension"),
        ] {
            let mut app = app_showing(&root, &[(name, false)]);
            app.focus = Focus::Contents;

            app.handle_key(KeyCode::Char('x'));

            assert_eq!(
                app.status_line(),
                format!("Extract to: {suggested}_  (Enter/Esc)")
            );
        }
    }

    #[test]
    fn extract_pressed_on_a_folder_suggests_the_folder_itself() {
        // Nothing asks whether the row is an archive, so `x` on a folder
        // prefills that folder's own name - and confirming it would ask the
        // service to extract a directory into the very path it sits at. The
        // service answers; the prompt never questions it.
        let root = notional_root("extract-a-folder");
        let mut app = app_showing(&root, &[("src", true)]);
        app.focus = Focus::Contents;

        app.handle_key(KeyCode::Char('x'));

        assert_eq!(app.status_line(), "Extract to: src_  (Enter/Esc)");
        assert_eq!(
            super::sibling_path(&root.join("src"), "src"),
            root.join("src").to_string_lossy(),
            "the destination it would send is the folder being extracted"
        );
    }

    // ---------------------------------------------------------------------
    // Cancelling, and what arrives afterwards.
    // ---------------------------------------------------------------------

    #[test]
    fn a_prompt_covers_the_status_it_was_opened_over_and_gives_it_back() {
        let root = notional_root("status-under-a-prompt");
        let mut app = app_showing(&root, &[("notes.txt", false)]);
        app.focus = Focus::Contents;
        app.status = Some("permission denied".to_owned());

        app.handle_key(KeyCode::Char('c'));
        assert_eq!(app.status_line(), "Copy to: notes.txt_  (Enter/Esc)");

        app.handle_key(KeyCode::Esc);
        assert_eq!(
            app.status_line(),
            "permission denied",
            "the message the prompt covered is still true underneath it"
        );
    }

    #[test]
    fn esc_cancels_what_is_in_flight_first_and_quits_on_the_next_press() {
        let mut app = App::new(notional_root("esc-twice"));
        assert!(
            app.pending_contents.is_some(),
            "a new app is already asking for its root's listing"
        );

        app.handle_key(KeyCode::Esc);
        assert!(!app.should_quit, "the first Esc had something to cancel");
        assert_eq!(app.status.as_deref(), Some("cancelled"));

        app.handle_key(KeyCode::Esc);
        assert!(app.should_quit, "the second had nothing left, so it quits");
    }

    #[test]
    fn esc_after_a_delete_is_confirmed_stops_the_waiting_but_not_the_deletion() {
        // What "cancel" can mean here is narrow, and this records the edge:
        // dropping the receiver means the reply is never applied, so the
        // listing is not reloaded and the deleted row stays on screen -
        // under a line that says "cancelled" - while the service carries the
        // deletion out regardless.
        let root = notional_root("cancel-after-confirm");
        let mut app = app_showing(&root, &[("notes.txt", false)]);
        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Delete);
        app.handle_key(KeyCode::Char('y'));
        assert!(app.pending_operation.is_some());

        app.handle_key(KeyCode::Esc);

        assert!(app.pending_operation.is_none());
        assert_eq!(app.status.as_deref(), Some("cancelled"));
        assert_eq!(
            app.contents.len(),
            1,
            "the row whose deletion is under way is still listed"
        );
        assert!(
            app.pending_contents.is_none(),
            "and nothing was asked for that would put that right"
        );
    }

    #[test]
    fn tick_turns_a_failed_file_request_into_something_the_pane_can_show() {
        let mut app = App::new(notional_root("failed-file-request"));
        app.pending_contents = None;
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Err(std::io::Error::other("the service went away")))
            .expect("the receiver is still held");
        app.pending_file = Some(rx);

        app.tick();

        assert!(app.pending_file.is_none());
        match &app.file_view {
            Some(Response::Error { message }) => assert!(
                message.contains("the service went away"),
                "the failure should be the one that happened: {message}"
            ),
            other => panic!("a failed request must still leave something to show: {other:?}"),
        }
    }

    #[test]
    fn tick_applies_a_finished_operation_and_leaves_an_unfinished_one_alone() {
        let mut app = App::new(notional_root("operation-tick"));
        app.pending_contents = None;
        let (tx, rx) = std::sync::mpsc::channel();
        app.pending_operation = Some(rx);

        app.tick();
        assert!(
            app.pending_operation.is_some(),
            "a request that has not answered yet is still waited on"
        );

        tx.send(Ok(Response::Error {
            message: "no such file".to_owned(),
        }))
        .expect("the receiver is still held");
        app.tick();

        assert!(app.pending_operation.is_none());
        assert_eq!(app.status.as_deref(), Some("no such file"));
    }

    #[test]
    fn a_reply_that_is_not_a_listing_says_so_rather_than_emptying_the_pane() {
        let root = notional_root("wrong-reply-to-a-listing");
        let mut app = app_showing(&root, &[("notes.txt", false)]);

        app.apply_contents_result(&[], Ok(Response::Done));

        assert_eq!(app.status.as_deref(), Some("expected a directory listing"));
        assert_eq!(
            app.contents.len(),
            1,
            "and what was already listed stays listed"
        );
    }

    #[test]
    fn a_reply_that_is_not_an_operation_result_is_not_taken_for_success() {
        let root = notional_root("wrong-reply-to-an-operation");
        let mut app = app_showing(&root, &[]);

        app.apply_operation_result(Ok(Response::Directory {
            entries: Vec::new(),
        }));

        assert_eq!(
            app.status.as_deref(),
            Some("unexpected response to operation")
        );
        assert!(
            app.pending_contents.is_none(),
            "a reply nobody understands must not trigger the reload that success does"
        );
    }

    #[test]
    fn a_broken_connection_during_a_listing_is_reported_on_the_status_line() {
        let root = notional_root("broken-listing");
        let mut app = app_showing(&root, &[]);

        app.apply_contents_result(&[], Err(std::io::Error::other("connection reset")));

        assert_eq!(app.status.as_deref(), Some("connection reset"));
    }

    // ---------------------------------------------------------------------
    // The folders tree.
    // ---------------------------------------------------------------------

    #[test]
    fn a_refreshed_listing_keeps_the_branches_the_reader_had_opened() {
        let mut root = FolderNode::root("/root".into());
        root.set_children_from(&entries(&[("keep", true), ("gone", true)]));
        let children = root.children.as_mut().expect("children were just set");
        children[0].expanded = true;
        children[0].set_children_from(&entries(&[("deep", true)]));

        root.set_children_from(&entries(&[("fresh", true), ("keep", true)]));

        let children = root.children.as_ref().expect("children were just set");
        let names: Vec<&str> = children.iter().map(|node| node.name.as_str()).collect();
        assert_eq!(
            names,
            ["fresh", "keep"],
            "the order is the listing's, and a folder that has gone is gone"
        );
        assert!(
            children[1].expanded,
            "a refresh must not fold up a branch the reader opened"
        );
        assert_eq!(
            children[1]
                .children
                .as_ref()
                .expect("what was already fetched is kept")
                .len(),
            1
        );
        assert!(
            !children[0].expanded && children[0].children.is_none(),
            "a folder seen for the first time starts closed and unfetched"
        );
    }

    #[test]
    fn a_root_path_with_no_final_component_is_named_by_the_path_itself() {
        let path = PathBuf::from("/");
        let root = FolderNode::root(path.clone());

        assert_eq!(root.name, path.display().to_string());
        assert!(root.expanded, "the root opens showing what it holds");
        assert!(root.children.is_none(), "and has not asked for it yet");
    }

    #[test]
    fn a_row_that_is_not_in_the_tree_resolves_to_nothing() {
        let mut root = FolderNode::root("/root".into());
        assert!(
            root.node_at(&[0]).is_none(),
            "a child of a node with no children yet"
        );

        root.set_children_from(&entries(&[("a", true)]));
        assert!(root.node_at(&[0]).is_some());
        assert!(root.node_at(&[1]).is_none(), "one past the last child");
        assert!(root.node_at(&[0, 0]).is_none(), "a grandchild that is not");
        assert_eq!(
            root.node_at(&[]).map(|node| node.name.clone()),
            Some(root.name.clone()),
            "and the empty path is the root itself"
        );
    }

    #[test]
    fn a_collapsed_root_hides_everything_below_it_and_left_again_steps_above_it() {
        let root = notional_root("collapse-the-root");
        let mut app = app_showing(&root, &[("alpha", true)]);
        assert_eq!(app.root.flatten().len(), 2);

        app.handle_key(KeyCode::Left);
        assert_eq!(app.root.flatten().len(), 1);
        assert_eq!(app.tree_selected, 0);

        app.handle_key(KeyCode::Left);
        assert_eq!(
            app.root.path,
            root.parent().expect("a notional root has a parent"),
            "left again, with nothing left to collapse, steps above the root (#642)"
        );
        assert_eq!(app.tree_selected, 0);
        assert!(
            app.pending_contents.is_some(),
            "and the parent's own listing is fetched"
        );
    }

    #[test]
    fn the_dedicated_parent_key_steps_above_the_root_from_any_pane() {
        let root = notional_root("above-root-parent-key");
        let mut app = app_showing(&root, &[("alpha", true)]);
        app.focus = Focus::Contents;

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT));

        assert_eq!(
            app.root.path,
            root.parent().expect("a notional root has a parent"),
            "the parent key steps above the root even with Contents focused"
        );
    }

    #[test]
    fn navigating_above_the_root_does_not_change_where_a_fresh_launch_opens() {
        // D7: every launch opens at the configured Repos Directory, and
        // nothing about a run's own navigation is remembered between runs.
        let root = notional_root("d7-anchor-holds");
        let mut app = app_showing(&root, &[("alpha", true)]);

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT));
        assert_ne!(app.root.path, root, "sanity: navigation actually moved");

        let fresh = App::new(root.clone());

        assert_eq!(
            fresh.root.path, root,
            "a fresh launch still opens at the configured root"
        );
    }

    #[test]
    fn back_and_forward_have_nowhere_to_go_until_something_steps_above_the_root() {
        let root = notional_root("nowhere-to-go");
        let mut app = app_showing(&root, &[("alpha", true)]);

        app.go_back();
        assert_eq!(app.status_line(), "nowhere to go back to");

        app.go_forward();
        assert_eq!(app.status_line(), "nowhere to go forward to");
    }

    #[test]
    fn back_and_forward_walk_the_roots_that_stepping_above_the_root_visited() {
        let root = notional_root("walk-history");
        let mut app = app_showing(&root, &[("alpha", true)]);
        assert!(!app.can_go_back(), "nowhere to go back to yet");
        assert!(!app.can_go_forward());

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT));
        let parent = app.root.path.clone();
        assert!(app.can_go_back());
        assert!(!app.can_go_forward());

        app.go_back();
        assert_eq!(app.root.path, root, "back returns to where this started");
        assert!(app.can_go_forward(), "and forward returns");

        app.go_forward();
        assert_eq!(app.root.path, parent);
    }

    #[test]
    fn left_on_a_folder_with_nothing_open_steps_out_to_its_parent() {
        let root = notional_root("step-out-to-the-parent");
        let mut app = app_showing(&root, &[("alpha", true), ("beta", true)]);
        app.handle_key(KeyCode::Down);
        app.handle_key(KeyCode::Down);
        assert_eq!(app.tree_selected, 2);
        app.pending_contents = None;

        app.handle_key(KeyCode::Left);

        assert_eq!(app.tree_selected, 0);
        assert!(
            app.pending_contents.is_some(),
            "stepping out is a move, so the parent's listing is fetched"
        );
    }

    #[test]
    fn left_on_an_open_folder_closes_it_and_stays_on_it() {
        let root = notional_root("close-a-branch-in-place");
        let mut app = app_showing(&root, &[("alpha", true)]);
        app.handle_key(KeyCode::Down);
        app.handle_key(KeyCode::Right);
        app.apply_contents_result(
            &[0],
            Ok(Response::Directory {
                entries: entries(&[("deep", true)]),
            }),
        );
        assert_eq!(app.root.flatten().len(), 3);
        app.pending_contents = None;

        app.handle_key(KeyCode::Left);

        assert_eq!(app.root.flatten().len(), 2, "the branch folded up");
        assert_eq!(
            app.tree_selected, 1,
            "and the folder the reader closed is still the selected one"
        );
        assert!(
            app.pending_contents.is_none(),
            "closing a branch is not a move, so nothing is fetched again"
        );
    }

    #[test]
    fn enter_on_a_folder_row_opens_it_in_the_tree_and_hands_focus_back() {
        let root = notional_root("drill-into-a-folder");
        let mut app = app_showing(&root, &[("alpha", true), ("notes.txt", false)]);
        app.focus = Focus::Contents;

        app.handle_key(KeyCode::Enter);

        assert_eq!(app.tree_selected, 1);
        assert_eq!(
            app.focus,
            Focus::Folders,
            "the tree is where the reader now is, so that is where the cursor goes"
        );
        assert_eq!(app.selected_dir_path(), root.join("alpha"));
        assert!(app.pending_contents.is_some());
    }

    #[test]
    fn enter_on_a_file_row_goes_nowhere() {
        let root = notional_root("drill-into-a-file");
        let mut app = app_showing(&root, &[("notes.txt", false)]);
        app.focus = Focus::Contents;

        app.handle_key(KeyCode::Enter);

        assert_eq!(app.focus, Focus::Contents);
        assert_eq!(app.tree_selected, 0);
        assert!(app.pending_contents.is_none());
    }

    #[test]
    fn enter_on_a_row_the_tree_has_never_heard_of_goes_nowhere() {
        let root = notional_root("drill-into-a-ghost");
        let mut app = app_showing(&root, &[("notes.txt", false)]);
        app.contents = entries(&[("ghost", true)]);
        app.focus = Focus::Contents;

        app.handle_key(KeyCode::Enter);

        assert_eq!(
            app.focus,
            Focus::Contents,
            "a row with no folder behind it must not move the tree"
        );
        assert_eq!(app.tree_selected, 0);
        assert!(app.pending_contents.is_none());
    }

    // ---------------------------------------------------------------------
    // What the panes actually show.
    // ---------------------------------------------------------------------

    #[test]
    fn only_the_focused_panes_border_is_drawn_in_the_focus_colour() {
        let root = notional_root("focus-colour");
        let mut app = app_showing(&root, &[("notes.txt", false)]);
        app.focus = Focus::Contents;
        let mut terminal = Terminal::new(TestBackend::new(40, 6)).expect("a test terminal");
        terminal
            .draw(|frame| render_app(frame, frame.area(), &app))
            .expect("a draw into the test backend");
        let buffer = terminal.backend().buffer().clone();

        // Row 0 is the breadcrumb line (#642); the panes' own top border
        // sits on row 1.
        assert_ne!(
            buffer[(0, 1)].fg,
            Color::Yellow,
            "the folders pane does not have focus"
        );
        assert_eq!(
            buffer[(10, 1)].fg,
            Color::Yellow,
            "the contents pane does, and its border is how a reader can tell"
        );
    }

    #[test]
    fn the_highlight_sits_on_the_row_the_listing_says_is_selected() {
        let root = notional_root("highlighted-row");
        let mut app = app_showing(&root, &[("first.txt", false), ("second.txt", false)]);
        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Down);
        assert_eq!(app.contents_selected, 1);
        let mut terminal = Terminal::new(TestBackend::new(40, 7)).expect("a test terminal");
        terminal
            .draw(|frame| render_app(frame, frame.area(), &app))
            .expect("a draw into the test backend");
        let buffer = terminal.backend().buffer().clone();

        // Row 0 is the breadcrumb line (#642), row 1 the pane's top
        // border, row 2 the table header, so the first entry sits at row
        // 3 and the second at row 4.
        assert_eq!(
            buffer[(11, 4)].bg,
            Color::Cyan,
            "the second row is the selected one"
        );
        assert_ne!(
            buffer[(11, 3)].bg,
            Color::Cyan,
            "and the first one is not, or the reader cannot tell them apart"
        );
    }

    #[test]
    fn an_empty_contents_pane_draws_no_highlight_at_all() {
        let root = notional_root("nothing-to-highlight");
        let mut app = app_showing(&root, &[]);
        app.focus = Focus::Contents;
        let mut terminal = Terminal::new(TestBackend::new(40, 6)).expect("a test terminal");
        terminal
            .draw(|frame| render_app(frame, frame.area(), &app))
            .expect("a draw into the test backend");
        let buffer = terminal.backend().buffer().clone();

        for y in 1..5_u16 {
            for x in 11..23_u16 {
                assert_ne!(
                    buffer[(x, y)].bg,
                    Color::Cyan,
                    "a listing with nothing in it must not point at a row ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn a_file_keeps_its_name_exactly_and_a_folder_gains_a_slash() {
        let listing = entries(&[("notes.txt", false), ("src", true)]);

        assert_eq!(super::entry_display_name(&listing[0]), "notes.txt");
        assert_eq!(super::entry_display_name(&listing[1]), "src/");
    }

    #[test]
    fn roots_with_none_of_them_active_fall_back_to_the_default_and_say_so() {
        let opening = super::opening_from(
            None,
            Some((
                vec![ReposRoot {
                    path: "/home/ada/repos".to_owned(),
                    active: false,
                }],
                "/var/tmp".to_owned(),
            )),
        );

        assert_eq!(
            opening.root,
            PathBuf::from("/var/tmp"),
            "a list with nothing marked active is as good as no list"
        );
        assert!(
            opening
                .notice
                .expect("a root nobody chose should be explained")
                .contains("No Repos Directory set")
        );
    }

    #[test]
    fn an_explicit_path_wins_even_when_the_service_cannot_be_asked() {
        let opening = super::opening_from(Some(PathBuf::from("/somewhere/else")), None);

        assert_eq!(opening.root, PathBuf::from("/somewhere/else"));
        assert_eq!(
            opening.notice, None,
            "nothing to explain: the reader said where"
        );
    }

    // ---- #625: a folder or file that has vanished since it was listed ----

    #[test]
    fn classify_file_problem_names_a_row_confirmed_gone() {
        let gone = notional_root("625-gone").join("alpha");

        let message = super::classify_file_problem(&gone, "the operating system's own words");

        assert_eq!(
            message,
            "alpha is no longer there - press F5 to reload the folder"
        );
    }

    #[test]
    fn classify_file_problem_leaves_an_unrelated_error_alone() {
        let dir = std::env::temp_dir();

        let message = super::classify_file_problem(&dir, "permission denied");

        assert_eq!(message, "permission denied");
    }

    // ---- #638: modifiers are read, not thrown away ----

    #[test]
    fn ctrl_c_does_not_open_the_copy_prompt_but_plain_c_does() {
        let mut app = app_with_one_content_entry();

        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(
            mode_of(&app),
            "normal",
            "Ctrl+C must not be read as the plain `c` copy binding"
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert_eq!(
            mode_of(&app),
            "copy",
            "plain `c` should still open the copy prompt"
        );
    }

    #[test]
    fn ctrl_q_quits_the_same_as_plain_q() {
        let mut app = App::new(notional_root("ctrl-q-quits"));

        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));

        assert!(app.should_quit, "Ctrl+Q should quit, the same as plain q");
    }

    // ---- #642: type-ahead does not swallow a command key ----

    #[test]
    fn typing_a_letter_not_bound_to_a_command_jumps_the_contents_selection() {
        let root = notional_root("type-ahead-contents");
        let mut app = app_showing(
            &root,
            &[
                ("alpha.txt", false),
                ("beta.txt", false),
                ("gamma.txt", false),
            ],
        );
        app.focus = Focus::Contents;

        app.handle_key(KeyCode::Char('g'));

        assert_eq!(app.contents_selected, 2, "typing g jumped to gamma.txt");
    }

    #[test]
    fn a_command_key_still_runs_its_command_instead_of_being_swallowed_by_type_ahead() {
        let root = notional_root("type-ahead-does-not-swallow-r");
        let mut app = app_showing(&root, &[("readme.txt", false)]);
        app.focus = Focus::Contents;

        // `r` is bound to Rename in the Contents pane; it must still rename
        // even though a row starts with it (#642's own acceptance check).
        app.handle_key(KeyCode::Char('r'));

        assert_eq!(
            mode_of(&app),
            "rename",
            "a bound command key must not be swallowed by type-ahead"
        );
    }

    #[test]
    fn typing_a_letter_jumps_the_folders_selection_too() {
        let root = notional_root("type-ahead-folders");
        let mut app = app_showing(&root, &[("apple", true), ("banana", true)]);

        app.handle_key(KeyCode::Char('b'));

        assert_eq!(
            app.tree_selected, 2,
            "row 0 is the root, row 1 apple, row 2 banana"
        );
    }

    #[test]
    fn letters_typed_within_a_second_extend_the_search_and_a_pause_starts_fresh() {
        let root = notional_root("type-ahead-timeout");
        let mut app = app_showing(&root, &[("alpha.txt", false), ("apricot.txt", false)]);
        app.focus = Focus::Contents;
        let start = std::time::Instant::now();

        app.type_ahead_key_at('a', start);
        assert_eq!(app.contents_selected, 0, "the first a matches alpha.txt");

        app.type_ahead_key_at('p', start + std::time::Duration::from_millis(200));
        assert_eq!(
            app.contents_selected, 1,
            "ap, typed quickly, narrows to apricot.txt"
        );

        app.type_ahead_key_at('a', start + std::time::Duration::from_secs(2));
        assert_eq!(
            app.contents_selected, 0,
            "typed after the timeout, a starts a fresh search rather than extending \"apa\""
        );
    }

    #[test]
    fn escape_ends_a_type_ahead_search_in_progress() {
        let root = notional_root("type-ahead-escape");
        let mut app = app_showing(&root, &[("alpha.txt", false), ("apricot.txt", false)]);
        app.focus = Focus::Contents;
        let start = std::time::Instant::now();
        app.type_ahead_key_at('a', start);
        app.contents_selected = 0;

        app.handle_key(KeyCode::Esc);
        app.type_ahead_key_at('p', start + std::time::Duration::from_millis(200));

        assert_eq!(
            app.contents_selected, 0,
            "Escape ended the search, so p alone matches nothing starting with it"
        );
    }

    #[test]
    fn the_default_help_line_fits_an_80_and_a_40_column_terminal_without_being_cut() {
        // A fresh app's own "loading..." status would otherwise stand in
        // for the help text this test means to measure.
        let mut app = App::new(notional_root("help-line-width"));
        app.status = None;

        let wide = app.status_line_at(80);
        assert!(
            wide.chars().count() <= 80,
            "an 80-column line must not overflow its own width: {wide:?}"
        );
        assert!(wide.starts_with("Tab: switch pane"));
        assert!(
            wide.ends_with("q: quit"),
            "there is room to say how to quit: {wide:?}"
        );

        let narrow = app.status_line_at(40);
        assert!(
            narrow.chars().count() <= 40,
            "a 40-column line must not overflow its own width: {narrow:?}"
        );
        assert!(narrow.starts_with("Tab: switch pane"));
        assert!(
            narrow.ends_with("q: quit"),
            "even a narrow terminal should still say how to quit: {narrow:?}"
        );
    }

    // ---------------------------------------------------------------------
    // #641: the terminal front end reads what a repository is, for every
    // row on screen.
    // ---------------------------------------------------------------------

    #[test]
    fn two_repositories_show_the_unanswered_mark_before_their_statuses_arrive_and_their_own_marks_after()
     {
        let root = notional_root("two-repos-marks");
        let mut app = App::new(root);
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![
                    repository_entry("dirty", "main"),
                    repository_entry("clean", "main"),
                ],
            }),
        );

        let before = drawn_contents(70, 6, &app).concat();
        assert!(
            before.contains(NOT_KNOWN_YET_MARKER),
            "before an answer arrives, a repository row says it does not know yet: {before}"
        );

        app.row_statuses.insert(
            "dirty".to_owned(),
            RowStatus::Answered(Some(protocol::WorkingTreeSummary {
                changed: 1,
                partial: false,
                summary: "1 tracked file changed".to_owned(),
            })),
        );
        app.row_statuses.insert(
            "clean".to_owned(),
            RowStatus::Answered(Some(protocol::WorkingTreeSummary {
                changed: 0,
                partial: false,
                summary: "clean".to_owned(),
            })),
        );

        let after = drawn_contents(70, 6, &app).concat();
        assert!(
            after.contains(CHANGED_MARKER),
            "the dirty repository should carry its own mark: {after}"
        );
        assert!(
            !after.contains(NOT_KNOWN_YET_MARKER),
            "once both have answered, neither should still say it does not know: {after}"
        );
    }

    #[test]
    fn scrolling_asks_only_for_the_rows_that_have_come_into_view() {
        let root = notional_root("scroll-visible-range");
        let mut app = App::new(root);
        app.set_contents_viewport_rows(2);
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![
                    repository_entry("a", "main"),
                    repository_entry("b", "main"),
                    repository_entry("c", "main"),
                    repository_entry("d", "main"),
                ],
            }),
        );

        // The viewport only fits two rows: only the first two are asked.
        assert!(app.row_statuses.contains_key("a"));
        assert!(app.row_statuses.contains_key("b"));
        assert!(!app.row_statuses.contains_key("c"));
        assert!(!app.row_statuses.contains_key("d"));

        // Moving the cursor past the visible window scrolls it; the next
        // tick asks for the row that came into view, and only that one.
        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Down);
        app.handle_key(KeyCode::Down);
        app.tick();

        assert!(
            app.row_statuses.contains_key("c"),
            "the row that scrolled into view should now be asked about"
        );
        assert!(
            !app.row_statuses.contains_key("d"),
            "a row still off screen should not be"
        );
    }

    #[test]
    fn a_new_listing_abandons_the_statuses_of_the_old_one() {
        let root = notional_root("abandon-old-statuses");
        let mut app = App::new(root);
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![repository_entry("old-repo", "main")],
            }),
        );
        assert!(app.row_statuses.contains_key("old-repo"));
        let (_tx, rx) = std::sync::mpsc::channel::<std::io::Result<Response>>();
        app.pending_statuses.push(("old-repo".to_owned(), rx));

        // Navigating away applies a fresh listing - even an empty one -
        // which must not leave the old folder's statuses, or its
        // outstanding requests, behind for a late answer to land in.
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: Vec::new(),
            }),
        );

        assert!(app.row_statuses.is_empty());
        assert!(app.pending_statuses.is_empty());
        app.tick();
    }

    #[test]
    fn fetch_is_stale_flags_an_old_or_missing_fetch_but_not_a_recent_one_or_no_remote() {
        const DAY: u64 = 24 * 60 * 60;
        let now = 1_000_000_000u64;

        assert!(super::fetch_is_stale(Some(now - 31 * DAY), true, now));
        assert!(!super::fetch_is_stale(Some(now - 29 * DAY), true, now));
        assert!(
            super::fetch_is_stale(None, true, now),
            "a repository with a remote it has never fetched is stale"
        );
        assert!(
            !super::fetch_is_stale(None, false, now),
            "a repository with no remote has nothing to have fetched"
        );
    }

    fn repository_entry_with_last_fetch(name: &str, last_fetch: Option<u64>) -> DirectoryEntry {
        DirectoryEntry {
            name: name.to_owned(),
            is_dir: true,
            size: 0,
            modified: None,
            repository: Some(protocol::RepositoryInfo {
                provider: Some("github.com".to_owned()),
                branch: Some("main".to_owned()),
                remote: Some("https://github.com/owner/repo.git".to_owned()),
                kind: protocol::RepositoryKind::Clone,
                last_activity: None,
                last_fetch,
            }),
        }
    }

    #[test]
    fn a_stale_repositorys_row_carries_the_clock_glyph() {
        let root = notional_root("stale-clock-glyph");
        let now = super::now_epoch_seconds();
        let mut app = App::new(root);
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![repository_entry_with_last_fetch(
                    "checkout",
                    Some(now - 31 * 24 * 60 * 60),
                )],
            }),
        );

        let text = drawn_contents(60, 6, &app).concat();

        assert!(
            text.contains(STALE_FETCH_MARKER),
            "a fetch over 30 days old should carry the clock glyph: {text}"
        );
    }

    #[test]
    fn the_status_line_says_why_the_selected_rows_fetch_is_stale() {
        let root = notional_root("stale-status-line");
        let mut app = App::new(root);
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![repository_entry_with_last_fetch("checkout", None)],
            }),
        );

        assert!(
            app.status_line().contains("Never fetched"),
            "a repository with a remote and no fetch at all should say so: {}",
            app.status_line()
        );
    }

    #[test]
    fn the_type_column_names_a_worktree_and_a_submodule() {
        let worktree = DirectoryEntry {
            name: "feature".to_owned(),
            is_dir: true,
            size: 0,
            modified: None,
            repository: Some(protocol::RepositoryInfo {
                provider: Some("github.com".to_owned()),
                branch: Some("feature".to_owned()),
                remote: Some("https://github.com/owner/repo.git".to_owned()),
                kind: protocol::RepositoryKind::Worktree {
                    clone: "/repos/repo".to_owned(),
                    clone_exists: true,
                },
                last_activity: None,
                last_fetch: None,
            }),
        };
        let submodule = DirectoryEntry {
            name: "vendor".to_owned(),
            is_dir: true,
            size: 0,
            modified: None,
            repository: Some(protocol::RepositoryInfo {
                provider: Some("github.com".to_owned()),
                branch: Some("main".to_owned()),
                remote: Some("https://github.com/owner/vendor.git".to_owned()),
                kind: protocol::RepositoryKind::Submodule {
                    outer: "/repos/outer".to_owned(),
                },
                last_activity: None,
                last_fetch: None,
            }),
        };

        assert_eq!(super::format_kind(&worktree), "Worktree · github.com");
        assert_eq!(super::format_kind(&submodule), "Submodule · github.com");
    }

    fn app_with_worktree(root: &Path, clone: &Path, clone_exists: bool) -> App {
        let mut app = App::new(root.to_path_buf());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![DirectoryEntry {
                    name: "feature".to_owned(),
                    is_dir: true,
                    size: 0,
                    modified: None,
                    repository: Some(protocol::RepositoryInfo {
                        provider: Some("github.com".to_owned()),
                        branch: Some("feature".to_owned()),
                        remote: None,
                        kind: protocol::RepositoryKind::Worktree {
                            clone: clone.to_string_lossy().into_owned(),
                            clone_exists,
                        },
                        last_activity: None,
                        last_fetch: None,
                    }),
                }],
            }),
        );
        app.file_view = Some(Response::Error {
            message: String::new(),
        });
        app
    }

    #[test]
    fn the_file_pane_names_the_clone_a_worktree_belongs_to() {
        let root = notional_root("worktree-file-pane");
        let app = app_with_worktree(&root, &root.join("repo"), true);

        let text = drawn_rows(100, 20, &app).concat();

        assert!(
            text.contains("Worktree of repo"),
            "the File pane should name the clone this worktree belongs to: {text}"
        );
    }

    #[test]
    fn the_file_pane_says_when_a_worktrees_clone_is_gone() {
        let root = notional_root("worktree-clone-gone");
        let app = app_with_worktree(&root, &PathBuf::from("/elsewhere/repo"), false);

        let text = drawn_rows(250, 10, &app).concat();

        assert!(
            text.contains("Worktree of a clone that is no longer at /elsewhere/repo"),
            "a worktree whose clone is gone should say so, not name it as if it were still there: {text}"
        );
    }

    #[test]
    fn the_file_pane_names_the_outer_working_copy_a_submodule_belongs_to() {
        let root = notional_root("submodule-file-pane");
        let mut app = App::new(root.clone());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![DirectoryEntry {
                    name: "vendor".to_owned(),
                    is_dir: true,
                    size: 0,
                    modified: None,
                    repository: Some(protocol::RepositoryInfo {
                        provider: Some("github.com".to_owned()),
                        branch: Some("main".to_owned()),
                        remote: None,
                        kind: protocol::RepositoryKind::Submodule {
                            outer: root.join("outer").to_string_lossy().into_owned(),
                        },
                        last_activity: None,
                        last_fetch: None,
                    }),
                }],
            }),
        );
        app.file_view = Some(Response::Error {
            message: String::new(),
        });

        let text = drawn_rows(100, 20, &app).concat();

        assert!(
            text.contains("Submodule of outer"),
            "the File pane should name the outer working copy this submodule belongs to: {text}"
        );
    }

    /// An app with `root` listing one selected row, and the File pane
    /// already showing `plugin`'s view of `data`, focused so its own
    /// scrolling and tab-switching bindings answer (#643).
    fn app_with_file_view(root: &Path, plugin: &str, data: serde_json::Value) -> App {
        let mut app = App::new(root.to_path_buf());
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("selected", false)]),
            }),
        );
        app.file_view = Some(Response::FileView {
            plugin: plugin.to_owned(),
            data,
            also: Vec::new(),
        });
        app.focus = Focus::File;
        app
    }

    #[test]
    fn the_file_pane_scrolls_and_its_end_is_reachable() {
        let root = notional_root("file-pane-scroll");
        let content = (1..=60)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut app = app_with_file_view(
            &root,
            "text",
            serde_json::json!({ "content": content, "truncated": false }),
        );

        let top = drawn_rows(40, 12, &app).concat();
        assert!(top.contains("line 1"), "{top}");
        assert!(
            !top.contains("line 60"),
            "a pane this short should not already show the file's last line: {top}"
        );

        for _ in 0..200 {
            app.handle_key(KeyCode::Down);
        }
        let scrolled = drawn_rows(40, 12, &app).concat();
        assert!(
            scrolled.contains("line 60"),
            "pressing Down enough times should scroll all the way to the last line: {scrolled}"
        );

        app.handle_key(KeyCode::Home);
        let home = drawn_rows(40, 12, &app).concat();
        assert!(
            home.contains("line 1") && !home.contains("line 60"),
            "Home should scroll back to the start: {home}"
        );

        app.handle_key(KeyCode::End);
        let end = drawn_rows(40, 12, &app).concat();
        assert!(
            end.contains("line 60"),
            "End should reach the file's last line directly: {end}"
        );
    }

    #[test]
    fn the_file_panes_scrollbar_moves_as_the_text_scrolls() {
        let root = notional_root("file-pane-scrollbar");
        let content = (1..=60)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut app = app_with_file_view(
            &root,
            "text",
            serde_json::json!({ "content": content, "truncated": false }),
        );

        let track_at = |app: &App| -> String {
            drawn_rows(40, 20, app)
                .iter()
                .map(|row| row.chars().rev().nth(1).unwrap_or(' '))
                .collect()
        };

        let at_top = track_at(&app);
        for _ in 0..200 {
            app.handle_key(KeyCode::Down);
        }
        let at_bottom = track_at(&app);

        assert_ne!(
            at_top, at_bottom,
            "the scrollbar should draw differently once the text is scrolled to the end"
        );
    }

    #[test]
    fn a_wide_line_in_the_file_pane_wraps_rather_than_being_cut() {
        let root = notional_root("file-pane-wide-line");
        let wide = format!("{}TAIL-MARKER", "x".repeat(80));
        let mut app = app_with_file_view(
            &root,
            "text",
            serde_json::json!({ "content": wide, "truncated": false }),
        );

        // A wide terminal, so the File pane's own share of it is wider
        // than the line - what a narrow pane's Contents column already
        // does not have room to spare a wrapped line, this one does.
        let text = drawn_rows(200, 20, &app).concat();
        app.handle_key(KeyCode::End);
        let scrolled = drawn_rows(200, 20, &app).concat();

        assert!(
            text.contains("TAIL-MARKER") || scrolled.contains("TAIL-MARKER"),
            "a line wider than the pane should wrap onto another row, not be cut short: \
             {text} / {scrolled}"
        );
    }

    #[test]
    fn a_multi_view_plugin_shows_a_tab_strip_and_switching_draws_the_other_view() {
        let root = notional_root("file-pane-tabs-multi");
        let mut app = app_with_file_view(
            &root,
            "rust",
            serde_json::json!({
                "content": "struct Widget;\n",
                "truncated": false,
                "functions": [],
                "structs": ["Widget"],
                "traits": [],
            }),
        );

        let preview = drawn_rows(60, 20, &app).concat();
        assert!(
            preview.contains("Preview") && preview.contains("Text"),
            "a plugin with more than one view should draw a tab strip naming them: {preview}"
        );
        assert!(
            preview.contains("structs: Widget"),
            "the Preview tab should show the plugin's own outline: {preview}"
        );

        app.handle_key(KeyCode::Right);
        let text_view = drawn_rows(60, 20, &app).concat();
        assert!(
            !text_view.contains("structs: Widget"),
            "switching to the Text tab should draw the file's plain content instead: {text_view}"
        );
    }

    /// A rust file's data for the colouring tests below: a comment, a
    /// keyword, and a string, each of the Rust language's own classes.
    fn coloured_rust_file() -> serde_json::Value {
        serde_json::json!({
            "content": "// a comment\nfn go() {\n    let s = \"hi\";\n}\n",
            "truncated": false,
            "functions": ["go"],
            "structs": [],
            "traits": [],
        })
    }

    /// Where `needle` is drawn in `rows`, as a (row, column) pair rather
    /// than the byte offset `str::find` would give - a border or the
    /// scrollbar draws a multi-byte character for a single column, so a
    /// byte offset and a column stop agreeing the moment one appears
    /// earlier in the row (#644).
    fn column_of(rows: &[String], needle: &str) -> (usize, usize) {
        let needle: Vec<char> = needle.chars().collect();
        for (y, row) in rows.iter().enumerate() {
            let chars: Vec<char> = row.chars().collect();
            if let Some(x) = chars
                .windows(needle.len())
                .position(|window| window == needle.as_slice())
            {
                return (y, x);
            }
        }
        panic!("{needle:?} was not drawn: {rows:?}");
    }

    #[test]
    fn a_keyword_a_string_and_a_comment_draw_in_their_classes_colours() {
        let root = notional_root("file-pane-colour");
        let mut app = app_with_file_view(&root, "rust", coloured_rust_file());
        app.colour_enabled = true;
        // Onto the Text tab, so this reads the branch that hands
        // `coloured_lines` straight to the pane rather than splicing it
        // in under the Preview tab's own summary line.
        app.handle_key(KeyCode::Right);

        let rows = drawn_rows(100, 20, &app);
        let colours = drawn_colours(100, 20, &app);
        let colour_of = |needle: &str| -> Color {
            let (y, x) = column_of(&rows, needle);
            colours[y][x]
        };

        assert_eq!(
            colour_of("fn go"),
            Color::Blue,
            "a keyword should draw in the keyword colour"
        );
        assert_eq!(
            colour_of("\"hi\""),
            Color::Red,
            "a string should draw in the string colour"
        );
        assert_eq!(
            colour_of("// a comment"),
            Color::DarkGray,
            "a comment should draw in the comment colour"
        );
    }

    #[test]
    fn no_colour_draws_the_same_characters_with_none_standing_in_for_a_class() {
        let root = notional_root("file-pane-no-colour");

        let mut coloured = app_with_file_view(&root, "rust", coloured_rust_file());
        coloured.colour_enabled = true;
        coloured.handle_key(KeyCode::Right);

        let mut plain = app_with_file_view(&root, "rust", coloured_rust_file());
        plain.colour_enabled = false;
        plain.handle_key(KeyCode::Right);

        assert_eq!(
            drawn_rows(100, 20, &coloured),
            drawn_rows(100, 20, &plain),
            "disabling colour - NO_COLOR, or a terminal that reports it supports \
             none - should never change what is drawn, only how"
        );

        let rows = drawn_rows(100, 20, &plain);
        let plain_colours = drawn_colours(100, 20, &plain);
        let (y, x) = column_of(&rows, "fn go");
        assert_eq!(
            plain_colours[y][x],
            Color::Reset,
            "with colour disabled, no class should stand in a colour it does not have"
        );
    }

    #[test]
    fn a_single_view_plugin_draws_no_tab_strip() {
        let root = notional_root("file-pane-tabs-single");
        let app = app_with_file_view(
            &root,
            "directory",
            serde_json::json!({
                "entry_count": 3,
                "total_size": 42,
                "repository": null,
                "readme": null,
            }),
        );

        let text = drawn_rows(60, 20, &app).concat();

        assert!(
            !text.contains("Preview"),
            "a plugin with only one view should draw no tab strip at all: {text}"
        );
    }

    #[test]
    fn the_file_panes_facts_are_a_table_with_folder_lines_and_a_readme_below_them() {
        let root = notional_root("file-pane-facts-readme");
        let mut app = app_with_file_view(
            &root,
            "directory",
            serde_json::json!({
                "entry_count": 5,
                "total_size": 1024,
                "repository": {
                    "provider": "github.com",
                    "branch": "main",
                    "remote": "https://github.com/example/repo.git",
                    "status": null,
                    "tracking": null,
                },
                "readme": {
                    "name": "README.md",
                    "title": "Widget",
                    "excerpt": ["A small widget library."],
                },
            }),
        );
        let Some(Response::FileView { also, .. }) = &mut app.file_view else {
            unreachable!("just constructed as a FileView");
        };
        *also = vec![protocol::PluginView {
            plugin: "project-cargo".to_owned(),
            data: serde_json::json!({
                "kind": "package",
                "package": {
                    "name": "widget-crate",
                    "version": "1.0.0",
                    "edition": "2024",
                    "rust_version": null,
                    "description": null
                },
                "members": [],
                "dependencies": 0,
                "dev_dependencies": 0,
                "build_dependencies": 0
            }),
        }];

        let rows = drawn_rows(80, 24, &app);
        let text = rows.concat();

        assert!(text.contains("Provider"), "{text}");
        let provider_row = rows
            .iter()
            .find(|row| row.contains("Provider"))
            .expect("a row for the Provider fact");
        assert!(
            provider_row.contains("github.com"),
            "the fact's label and value should sit on the same row, as a table draws them: {provider_row}"
        );

        assert!(
            text.contains("widget-crate"),
            "the folder plugin's own lines should still appear below the facts (D12): {text}"
        );
        assert!(
            text.contains("Widget") && text.contains("A small widget library."),
            "the README's title and excerpt should appear: {text}"
        );

        let facts_row = rows
            .iter()
            .position(|row| row.contains("Provider"))
            .unwrap();
        let readme_row = rows
            .iter()
            .position(|row| row.contains("A small widget library."))
            .unwrap();
        assert!(
            readme_row > facts_row,
            "the README should be drawn below the facts table: facts at row {facts_row}, README at row {readme_row}"
        );
    }

    #[test]
    fn a_repositorys_last_activity_drives_the_modified_column() {
        let root = notional_root("last-activity-modified");
        let mut app = App::new(root);
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![DirectoryEntry {
                    name: "checkout".to_owned(),
                    is_dir: true,
                    // The folder's own modified time - if this wins over
                    // the repository's last activity, the Modified column
                    // shows the Unix epoch instead.
                    size: 0,
                    modified: Some(0),
                    repository: Some(protocol::RepositoryInfo {
                        provider: None,
                        branch: Some("main".to_owned()),
                        remote: None,
                        kind: protocol::RepositoryKind::Clone,
                        last_activity: Some(1_700_000_000),
                        last_fetch: None,
                    }),
                }],
            }),
        );

        let text = drawn_contents(90, 6, &app).concat();

        assert!(
            text.contains(&super::format_timestamp(Some(1_700_000_000))),
            "the Modified column should show the repository's last activity: {text}"
        );
        assert!(
            !text.contains(&super::format_timestamp(Some(0))),
            "the folder's own modified time should not win over last activity: {text}"
        );
    }

    #[test]
    fn the_status_line_counts_items_repositories_and_uncommitted_changes() {
        let root = notional_root("status-line-counts");
        let mut app = App::new(root);
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![
                    repository_entry_without_remote("alpha", "main"),
                    repository_entry_without_remote("beta", "main"),
                    DirectoryEntry {
                        name: "notes.txt".to_owned(),
                        is_dir: false,
                        size: 0,
                        modified: None,
                        repository: None,
                    },
                ],
            }),
        );
        app.row_statuses.insert(
            "alpha".to_owned(),
            RowStatus::Answered(Some(protocol::WorkingTreeSummary {
                changed: 2,
                partial: false,
                summary: "2 tracked files changed".to_owned(),
            })),
        );
        app.row_statuses.insert(
            "beta".to_owned(),
            RowStatus::Answered(Some(protocol::WorkingTreeSummary {
                changed: 0,
                partial: false,
                summary: "clean".to_owned(),
            })),
        );

        assert_eq!(
            app.status_line(),
            "3 items, 2 repositories, 1 with uncommitted changes"
        );
    }

    #[test]
    fn the_status_line_says_how_many_repositories_are_not_known_yet() {
        let root = notional_root("status-line-not-known");
        let mut app = App::new(root);
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![
                    repository_entry_without_remote("alpha", "main"),
                    repository_entry_without_remote("beta", "main"),
                ],
            }),
        );

        assert_eq!(app.status_line(), "2 items, 2 repositories (2 not known)");
    }

    #[test]
    fn the_status_line_falls_back_to_help_text_for_a_folder_with_no_repository() {
        let root = notional_root("status-line-no-repository");
        let app = app_showing(&root, &[("notes.txt", false)]);

        assert!(app.status_line().starts_with("Tab: switch pane"));
    }

    // -- the editor (#645) ------------------------------------------------

    #[test]
    fn the_edit_view_is_appended_only_when_the_plugin_offers_editable_text() {
        let root = notional_root("edit-view-offered");
        let editable = app_with_file_view(
            &root,
            "text",
            serde_json::json!({ "content": "hello", "truncated": false }),
        );
        assert_eq!(editable.file_views(), vec!["Preview", "Text", "Edit"]);

        let truncated = app_with_file_view(
            &root,
            "text",
            serde_json::json!({ "content": "hello", "truncated": true }),
        );
        assert!(
            !truncated.file_views().contains(&"Edit"),
            "a truncated view must not be offered for editing - saving it back would \
             silently drop the rest of the file"
        );
    }

    #[test]
    fn enter_on_the_edit_view_starts_editing_and_hides_the_plugins_own_views() {
        let root = notional_root("edit-view-enter");
        let mut app = app_with_file_view(
            &root,
            "text",
            serde_json::json!({ "content": "hello", "truncated": false }),
        );
        app.file_view_index = 2; // "Edit", the last of ["Preview", "Text", "Edit"]

        app.handle_key(KeyCode::Enter);

        assert!(
            app.editing.is_some(),
            "Enter on the Edit view should start editing"
        );
        assert_eq!(
            app.file_views(),
            vec!["Editing"],
            "the plugin's own views are hidden while editing - one stray Left \
             or Right away from discarding what somebody has typed is not a \
             risk worth keeping"
        );
    }

    #[test]
    fn leaving_with_no_changes_does_not_ask() {
        let root = notional_root("edit-leave-clean");
        let mut app = app_with_file_view(
            &root,
            "text",
            serde_json::json!({ "content": "hello", "truncated": false }),
        );
        app.editing = Some(Editing {
            path: root.join("selected"),
            document: Document::new("hello"),
            original: "hello".to_owned(),
        });

        app.handle_key(KeyCode::Esc);

        assert!(
            app.editing.is_none(),
            "leaving unchanged should not stay in the editor"
        );
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn leaving_with_changes_asks_naming_the_file_and_declining_keeps_them() {
        let root = notional_root("edit-leave-dirty");
        let mut app = app_with_file_view(
            &root,
            "text",
            serde_json::json!({ "content": "hello", "truncated": false }),
        );
        let mut document = Document::new("hello");
        document.insert("X");
        app.editing = Some(Editing {
            path: root.join("selected"),
            document,
            original: "hello".to_owned(),
        });

        app.handle_key(KeyCode::Esc);
        assert_eq!(
            app.status_line(),
            "Discard changes to selected? y/n",
            "the prompt should name the file"
        );
        assert!(app.editing.is_some(), "nothing should be discarded yet");

        app.handle_key(KeyCode::Char('n'));
        assert!(matches!(app.mode, Mode::Normal));
        assert!(app.editing.is_some(), "declining should keep editing");
        assert_eq!(
            app.editing.as_ref().unwrap().document.text(),
            "Xhello",
            "declining should keep exactly what was typed"
        );
    }

    #[test]
    fn confirming_discard_drops_the_typed_changes() {
        let root = notional_root("edit-discard-yes");
        let mut app = app_with_file_view(
            &root,
            "text",
            serde_json::json!({ "content": "hello", "truncated": false }),
        );
        let mut document = Document::new("hello");
        document.insert("X");
        app.editing = Some(Editing {
            path: root.join("selected"),
            document,
            original: "hello".to_owned(),
        });

        app.handle_key(KeyCode::Esc);
        app.handle_key(KeyCode::Char('y'));

        assert!(app.editing.is_none());
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn saving_sends_write_file_and_leaves_the_editor() {
        let root = notional_root("edit-save");
        let mut app = app_with_file_view(
            &root,
            "text",
            serde_json::json!({ "content": "hello", "truncated": false }),
        );
        let mut document = Document::new("hello");
        document.insert("X");
        app.editing = Some(Editing {
            path: root.join("selected"),
            document,
            original: "hello".to_owned(),
        });

        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

        assert!(
            app.editing.is_none(),
            "saving should leave the editor at once, the way Save always does"
        );
        assert!(
            app.pending_operation.is_some(),
            "a WriteFile request should be sent through the service"
        );
    }

    #[test]
    fn a_refused_save_shows_the_services_own_words() {
        let root = notional_root("edit-save-refused");
        let mut app = App::new(root);

        app.apply_operation_result(Ok(Response::Error {
            message: "stream did not contain valid UTF-8".to_owned(),
        }));

        assert_eq!(
            app.status_line(),
            "stream did not contain valid UTF-8",
            "a refusal should reach the reader exactly as the service worded it"
        );
    }

    #[test]
    fn the_editor_places_the_terminal_cursor_on_a_character_boundary_in_a_multibyte_line() {
        let root = notional_root("edit-caret-multibyte");
        let mut app = app_with_file_view(
            &root,
            "text",
            serde_json::json!({ "content": "héllo wörld", "truncated": false }),
        );
        let mut document = Document::new("héllo wörld");
        for _ in 0..7 {
            document.move_right(false);
        }
        app.editing = Some(Editing {
            path: root.join("selected"),
            document,
            original: "héllo wörld".to_owned(),
        });

        let area = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 6,
        };
        let mut terminal = Terminal::new(TestBackend::new(40, 6)).expect("a test terminal");
        terminal
            .draw(|frame| render_file_editing(frame, area, &app))
            .expect("a draw into the test backend");

        let cursor = terminal
            .get_cursor_position()
            .expect("the editor places the terminal's own cursor");
        assert_eq!(
            (cursor.x, cursor.y),
            (7, 0),
            "the caret sits after the seventh grapheme - a letter, an accented \
             letter and a space each count as one - not mid multibyte character"
        );

        let drawn: String = (0..area.width)
            .map(|x| terminal.backend().buffer()[(x, 0)].symbol().to_owned())
            .collect();
        assert!(
            drawn.starts_with("héllo wörld"),
            "the multibyte line should draw whole, without panicking or mangling: {drawn}"
        );
    }

    // ---------------------------------------------------------------------
    // Filtering the listing, and letting the reader choose the shape of
    // the panes (#650).
    // ---------------------------------------------------------------------

    #[test]
    fn typing_into_the_filter_narrows_the_listing() {
        let root = notional_root("filter-narrows");
        let mut app = app_showing(
            &root,
            &[("alpha", false), ("bravo", false), ("charlie", false)],
        );
        app.focus = Focus::Contents;

        app.handle_key(KeyCode::Char('/'));
        assert_eq!(mode_of(&app), "filter");
        app.handle_key(KeyCode::Char('a'));
        app.handle_key(KeyCode::Char('l'));

        assert_eq!(filtered_content_names(&app), vec!["alpha"]);
        assert_eq!(
            app.contents.len(),
            3,
            "the raw listing itself is untouched by the filter"
        );
    }

    #[test]
    fn enter_keeps_the_filter_and_returns_to_browsing() {
        let root = notional_root("filter-enter-commits");
        let mut app = app_showing(
            &root,
            &[("alpha", false), ("bravo", false), ("charlie", false)],
        );
        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Char('/'));
        app.handle_key(KeyCode::Char('a'));
        app.handle_key(KeyCode::Char('l'));

        app.handle_key(KeyCode::Enter);

        assert_eq!(mode_of(&app), "normal");
        assert_eq!(
            filtered_content_names(&app),
            vec!["alpha"],
            "the filter stays applied after Enter"
        );
    }

    #[test]
    fn escape_while_typing_clears_the_filter_entirely() {
        let root = notional_root("filter-escape-typing");
        let mut app = app_showing(
            &root,
            &[("alpha", false), ("bravo", false), ("charlie", false)],
        );
        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Char('/'));
        app.handle_key(KeyCode::Char('a'));
        app.handle_key(KeyCode::Char('l'));

        app.handle_key(KeyCode::Esc);

        assert_eq!(mode_of(&app), "normal");
        assert_eq!(
            filtered_content_names(&app),
            vec!["alpha", "bravo", "charlie"]
        );
    }

    #[test]
    fn escape_after_committing_a_filter_clears_it_without_quitting() {
        let root = notional_root("filter-escape-committed");
        let mut app = app_showing(
            &root,
            &[("alpha", false), ("bravo", false), ("charlie", false)],
        );
        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Char('/'));
        app.handle_key(KeyCode::Char('a'));
        app.handle_key(KeyCode::Char('l'));
        app.handle_key(KeyCode::Enter);

        app.handle_key(KeyCode::Esc);

        assert!(!app.should_quit, "clearing the filter should not also quit");
        assert_eq!(
            filtered_content_names(&app),
            vec!["alpha", "bravo", "charlie"]
        );
    }

    #[test]
    fn changing_folder_drops_the_filter() {
        let root = notional_root("filter-folder-change");
        let mut app = app_showing(&root, &[("alpha", false), ("bravo", false)]);
        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Char('/'));
        app.handle_key(KeyCode::Char('a'));
        app.handle_key(KeyCode::Char('l'));
        app.handle_key(KeyCode::Enter);
        assert_eq!(filtered_content_names(&app), vec!["alpha"]);

        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: entries(&[("delta", false)]),
            }),
        );

        assert_eq!(
            filtered_content_names(&app),
            vec!["delta"],
            "a fresh listing drops the filter the old one had"
        );
    }

    #[test]
    fn a_filter_matching_nothing_says_so_rather_than_reporting_an_empty_repos_directory() {
        let root = notional_root("filter-no-match");
        let mut app = app_showing(&root, &[("alpha", false), ("bravo", false)]);
        app.focus = Focus::Contents;
        app.handle_key(KeyCode::Char('/'));
        for c in "zzz".chars() {
            app.handle_key(KeyCode::Char(c));
        }
        app.handle_key(KeyCode::Enter);

        let text = drawn_contents(40, 6, &app).concat();

        assert!(
            text.contains("No name matches `zzz`"),
            "should name the filter, not read as an empty Repos Directory: {text}"
        );
    }

    #[test]
    fn the_changed_only_filter_and_the_name_filter_combine() {
        let root = notional_root("filter-combine");
        let mut app = App::new(root);
        app.apply_contents_result(
            &[],
            Ok(Response::Directory {
                entries: vec![
                    repository_entry("alpha-repo", "main"),
                    repository_entry("beta-repo", "main"),
                ],
            }),
        );
        app.pending_contents = None;
        app.pending_file = None;
        app.row_statuses.insert(
            "alpha-repo".to_owned(),
            RowStatus::Answered(Some(protocol::WorkingTreeSummary {
                changed: 2,
                partial: false,
                summary: "2 tracked files changed".to_owned(),
            })),
        );
        app.row_statuses.insert(
            "beta-repo".to_owned(),
            RowStatus::Answered(Some(protocol::WorkingTreeSummary {
                changed: 0,
                partial: false,
                summary: "clean".to_owned(),
            })),
        );
        app.focus = Focus::Contents;

        app.handle_key(KeyCode::Char('u'));
        assert_eq!(
            filtered_content_names(&app),
            vec!["alpha-repo"],
            "changed-only alone narrows to the dirty repository"
        );

        app.handle_key(KeyCode::Char('/'));
        app.handle_key(KeyCode::Char('b'));
        app.handle_key(KeyCode::Enter);

        assert!(
            filtered_content_names(&app).is_empty(),
            "combined with a name filter matching only the clean repository, nothing passes both"
        );

        app.handle_key(KeyCode::Char('u'));
        assert_eq!(
            filtered_content_names(&app),
            vec!["beta-repo"],
            "lifting changed-only leaves the name filter alone"
        );
    }

    #[test]
    fn widening_the_focused_pane_grows_it_and_narrows_its_neighbour() {
        let mut app = App::new(std::env::temp_dir());
        app.focus = Focus::Folders;
        app.drawn_panes_width.set(100);

        app.handle_key(KeyCode::Char(']'));

        let widths = app.pane_widths().expect("a resize sets explicit widths");
        assert_eq!(widths.folders, 26, "25% of 100, then widened by one column");
        assert_eq!(
            widths.contents, 35,
            "unaffected - Folders is the one resized"
        );
    }

    #[test]
    fn narrowing_the_focused_pane_stops_at_the_minimum() {
        let mut app = App::new(std::env::temp_dir());
        app.focus = Focus::Folders;
        app.set_pane_widths(settings::PaneWidths {
            folders: settings::MIN_WIDTH,
            contents: 30,
        });

        app.handle_key(KeyCode::Char('['));

        assert_eq!(
            app.pane_widths().unwrap().folders,
            settings::MIN_WIDTH,
            "a pane never narrows past the minimum"
        );
    }

    #[test]
    fn the_widths_a_resize_chose_are_what_a_later_run_would_load_back() {
        // What `main` does with `App::pane_widths`/`App::set_pane_widths`
        // and `settings::save_pane_widths`/`load_pane_widths`, without
        // touching a real settings file (#650).
        let mut app = App::new(std::env::temp_dir());
        app.focus = Focus::Contents;
        app.drawn_panes_width.set(100);
        app.handle_key(KeyCode::Char(']'));
        let chosen = app.pane_widths().expect("a resize sets explicit widths");

        let mut next_run = App::new(std::env::temp_dir());
        next_run.set_pane_widths(chosen);

        assert_eq!(next_run.pane_widths(), Some(chosen));
    }

    #[test]
    fn maximising_draws_only_the_focused_pane_and_restoring_brings_the_others_back() {
        let root = notional_root("maximize");
        let mut app = app_showing(&root, &[("alpha", false)]);
        app.focus = Focus::Contents;

        app.handle_key(KeyCode::Char('z'));
        let maximized_rows = drawn_rows(60, 10, &app);
        assert!(
            maximized_rows.iter().any(|row| row.contains("Contents")),
            "the maximised pane still draws: {maximized_rows:?}"
        );
        assert!(
            !maximized_rows.iter().any(|row| row.contains("Folders")),
            "the other panes are not drawn while maximised: {maximized_rows:?}"
        );

        app.handle_key(KeyCode::Char('z'));
        let restored_rows = drawn_rows(60, 10, &app);
        assert!(
            restored_rows.iter().any(|row| row.contains("Folders")),
            "restoring brings the three-pane layout back: {restored_rows:?}"
        );
    }
}
