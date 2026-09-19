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
use crate::render_with_block;
use protocol::{DirectoryEntry, ReposRoot, Request, Response};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

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
    status: Option<String>,
    mode: Mode,
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
    /// Set once the user has asked to quit.
    pub should_quit: bool,
}

impl App {
    /// Starts a new explorer rooted at `root`, and kicks off loading its
    /// contents in the background.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        let mut app = Self {
            root: FolderNode::root(root),
            tree_selected: 0,
            contents: Vec::new(),
            contents_dir: PathBuf::new(),
            contents_selected: 0,
            focus: Focus::Folders,
            file_view: None,
            status: None,
            mode: Mode::Normal,
            pending_contents: None,
            pending_contents_dir: None,
            pending_file: None,
            pending_file_path: None,
            pending_operation: None,
            should_quit: false,
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
            return;
        };
        let path = self.contents_dir.join(&entry.name);
        let request = Request::ViewFile {
            path: path.to_string_lossy().into_owned(),
        };
        self.pending_file_path = Some(path);
        self.pending_file = Some(spawn_request(request));
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
        }
        if let Some(rx) = &self.pending_operation
            && let Ok(result) = rx.try_recv()
        {
            self.pending_operation = None;
            self.apply_operation_result(result);
        }
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
                self.contents_selected = 0;
                self.load_file_view();
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
            Mode::Normal => {}
        }
        if let Some(action) = bindings::find(key, self.focus) {
            self.dispatch(action);
        }
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
        }
    }

    fn cancel_or_quit(&mut self) {
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
            Mode::Normal | Mode::ConfirmDelete { .. } => None,
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
    /// text input is pending, otherwise the current status or the default
    /// help text, in full - see [`App::status_line_at`] for the version
    /// that fits a given terminal width.
    fn status_line(&self) -> String {
        match &self.mode {
            Mode::ConfirmDelete { name, .. } => format!("Delete {name}? y/n"),
            Mode::RenameInput { input, .. } => format!("Rename to: {input}_  (Enter/Esc)"),
            Mode::CopyInput { input, .. } => format!("Copy to: {input}_  (Enter/Esc)"),
            Mode::ExtractInput { input, .. } => format!("Extract to: {input}_  (Enter/Esc)"),
            Mode::Normal => self
                .status
                .clone()
                .unwrap_or_else(|| HELP_SEGMENTS.join("  ")),
        }
    }

    /// As [`App::status_line`], but the default help text is shortened to
    /// fit `width` columns rather than being cut off wherever the terminal
    /// happens to end - the bug #638 reported: a truncated help line on an
    /// 80-column terminal, hiding three of its own bindings. A prompt or a
    /// custom status message is returned exactly as `status_line` gives it,
    /// since neither is this front end's to shorten (a full reference for
    /// the help text is #649's job).
    fn status_line_at(&self, width: usize) -> String {
        match &self.mode {
            Mode::Normal if self.status.is_none() => fit_help_line(width),
            Mode::ConfirmDelete { .. }
            | Mode::RenameInput { .. }
            | Mode::CopyInput { .. }
            | Mode::ExtractInput { .. }
            | Mode::Normal => self.status_line(),
        }
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
        }
    }

    fn move_up_in_contents(&mut self) {
        if self.contents_selected > 0 {
            self.contents_selected -= 1;
            self.load_file_view();
        }
    }

    fn move_down_in_contents(&mut self) {
        if self.contents_selected + 1 < self.contents.len() {
            self.contents_selected += 1;
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

fn pane_block(title: &str, focused: bool) -> Block<'_> {
    let style = if focused {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };
    Block::bordered().title(title).border_style(style)
}

/// Renders the three-pane explorer into `area` of `frame`.
pub fn render_app(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(35),
            Constraint::Percentage(40),
        ])
        .split(rows[0]);

    render_folders(frame, columns[0], app);
    render_contents(frame, columns[1], app);
    render_file(frame, columns[2], app);

    frame.render_widget(
        Paragraph::new(app.status_line_at(rows[1].width.into())),
        rows[1],
    );
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

/// How one entry reads in the contents pane.
///
/// A working copy is what somebody opening their workspace is looking for,
/// so it says it is one and names the provider it came from. A folder that
/// is not one stays listed, with the trailing slash it always had - visible,
/// and plainly different (GUIDANCE.md 2.5).
fn contents_label(entry: &DirectoryEntry) -> String {
    match &entry.repository {
        Some(repository) => match &repository.provider {
            Some(provider) => format!("{}/  [{provider}]", entry.name),
            None => format!("{}/  [repository]", entry.name),
        },
        None if entry.is_dir => format!("{}/", entry.name),
        None => entry.name.clone(),
    }
}

fn render_contents(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let items: Vec<ListItem<'_>> = app
        .contents
        .iter()
        .map(|entry| ListItem::new(contents_label(entry)))
        .collect();

    let mut state = ListState::default();
    if !app.contents.is_empty() {
        state.select(Some(app.contents_selected));
    }

    let list = List::new(items)
        .block(pane_block("Contents", app.focus == Focus::Contents))
        .highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black));
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_file(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = pane_block("File", app.focus == Focus::File);
    match &app.file_view {
        Some(response) => render_with_block(frame, area, response, block),
        None => frame.render_widget(Paragraph::new("(no file selected)").block(block), area),
    }
}

#[cfg(test)]
mod tests {
    use super::{App, Focus, FolderNode, Mode, render_app};
    use protocol::{DirectoryEntry, ReposRoot, Response};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
    fn a_working_copy_is_labelled_apart_from_a_plain_folder() {
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

        assert_eq!(super::contents_label(&checkout), "explorer/  [github.com]");
        assert_eq!(
            super::contents_label(&folder),
            "scratch/",
            "a folder that is not a checkout stays listed, and stays plain"
        );
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

        assert_eq!(
            super::contents_label(&checkout),
            "local-only/  [repository]"
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
        }
    }

    /// The path the open prompt is about, whichever prompt it is.
    fn prompt_path(app: &App) -> Option<&Path> {
        match &app.mode {
            Mode::ConfirmDelete { path, .. }
            | Mode::RenameInput { path, .. }
            | Mode::CopyInput { path, .. }
            | Mode::ExtractInput { path, .. } => Some(path.as_path()),
            Mode::Normal => None,
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
    fn a_collapsed_root_hides_everything_below_it_and_left_again_does_nothing() {
        let root = notional_root("collapse-the-root");
        let mut app = app_showing(&root, &[("alpha", true)]);
        assert_eq!(app.root.flatten().len(), 2);

        app.handle_key(KeyCode::Left);
        assert_eq!(app.root.flatten().len(), 1);
        assert_eq!(app.tree_selected, 0);

        app.handle_key(KeyCode::Left);
        assert_eq!(
            app.tree_selected, 0,
            "there is nothing above the root to step out to"
        );
        assert!(
            app.pending_contents.is_none(),
            "and nothing was fetched for a move that did not happen"
        );
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

        assert_ne!(
            buffer[(0, 0)].fg,
            Color::Yellow,
            "the folders pane does not have focus"
        );
        assert_eq!(
            buffer[(10, 0)].fg,
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
        let mut terminal = Terminal::new(TestBackend::new(40, 6)).expect("a test terminal");
        terminal
            .draw(|frame| render_app(frame, frame.area(), &app))
            .expect("a draw into the test backend");
        let buffer = terminal.backend().buffer().clone();

        assert_eq!(
            buffer[(11, 2)].bg,
            Color::Cyan,
            "the second row is the selected one"
        );
        assert_ne!(
            buffer[(11, 1)].bg,
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

        assert_eq!(super::contents_label(&listing[0]), "notes.txt");
        assert_eq!(super::contents_label(&listing[1]), "src/");
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
}
