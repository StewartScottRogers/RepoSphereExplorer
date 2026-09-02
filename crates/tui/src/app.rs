//! Application state and rendering for the three-pane explorer.
//!
//! Directory listings are not streamed by the service (see `service`'s crate
//! docs): each pane's request runs on a background thread instead, so the
//! event loop never blocks on the network round trip. Cancelling means no
//! longer waiting on that thread's result, not aborting the walk in
//! progress — a smaller, real slice of §3.3's "every long operation is
//! cancellable from the UI", with true early-abort deferred alongside
//! streaming itself.

use crate::render_with_block;
use protocol::{DirectoryEntry, Request, Response};
use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
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

/// The three-pane explorer's state: a folders tree, the selected folder's
/// contents, and the selected file's preview.
pub struct App {
    root: FolderNode,
    tree_selected: usize,
    contents: Vec<DirectoryEntry>,
    contents_selected: usize,
    focus: Focus,
    file_view: Option<Response>,
    status: Option<String>,
    mode: Mode,
    pending_contents: Option<(Vec<usize>, Receiver<io::Result<Response>>)>,
    pending_file: Option<Receiver<io::Result<Response>>>,
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
            contents_selected: 0,
            focus: Focus::Folders,
            file_view: None,
            status: None,
            mode: Mode::Normal,
            pending_contents: None,
            pending_file: None,
            pending_operation: None,
            should_quit: false,
        };
        app.load_contents_for_selected();
        app
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
        self.status = Some(format!("loading {}...", path.display()));
    }

    fn load_file_view(&mut self) {
        let Some(entry) = self.contents.get(self.contents_selected) else {
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
                self.contents_selected = 0;
                self.load_file_view();
            }
            Ok(Response::Error { message }) => self.status = Some(message),
            Ok(Response::FileView { .. } | Response::Done) => {
                self.status = Some("expected a directory listing".to_owned());
            }
            Err(err) => self.status = Some(err.to_string()),
        }
    }

    /// Handles one key press.
    pub fn handle_key(&mut self, code: KeyCode) {
        match self.mode {
            Mode::ConfirmDelete { .. } => {
                self.handle_confirm_delete_key(code);
                return;
            }
            Mode::RenameInput { .. } | Mode::CopyInput { .. } | Mode::ExtractInput { .. } => {
                self.handle_text_input_key(code);
                return;
            }
            Mode::Normal => {}
        }
        match code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc => self.cancel_or_quit(),
            KeyCode::Tab => self.focus = self.focus.next(),
            KeyCode::BackTab => self.focus = self.focus.previous(),
            KeyCode::Delete if self.focus == Focus::Contents => self.start_delete_confirmation(),
            KeyCode::Char('r') if self.focus == Focus::Contents => self.start_rename_input(),
            KeyCode::Char('c') if self.focus == Focus::Contents => self.start_copy_input(),
            KeyCode::Char('x') if self.focus == Focus::Contents => self.start_extract_input(),
            _ => match self.focus {
                Focus::Folders => self.handle_folders_key(code),
                Focus::Contents => self.handle_contents_key(code),
                Focus::File => {}
            },
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
        let path = self.selected_dir_path().join(&entry.name);
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
        let path = self.selected_dir_path().join(&entry.name);
        self.mode = Mode::RenameInput {
            path,
            input: entry.name.clone(),
        };
    }

    fn start_copy_input(&mut self) {
        let Some(entry) = self.contents.get(self.contents_selected) else {
            return;
        };
        let path = self.selected_dir_path().join(&entry.name);
        self.mode = Mode::CopyInput {
            path,
            input: entry.name.clone(),
        };
    }

    fn start_extract_input(&mut self) {
        let Some(entry) = self.contents.get(self.contents_selected) else {
            return;
        };
        let path = self.selected_dir_path().join(&entry.name);
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
                from: path.to_string_lossy().into_owned(),
                to: sibling_path(&path, &input),
            }),
            Mode::CopyInput { path, input } if !input.is_empty() => Some(Request::Copy {
                from: path.to_string_lossy().into_owned(),
                to: sibling_path(&path, &input),
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
    /// help text.
    fn status_line(&self) -> String {
        match &self.mode {
            Mode::ConfirmDelete { name, .. } => format!("Delete {name}? y/n"),
            Mode::RenameInput { input, .. } => format!("Rename to: {input}_  (Enter/Esc)"),
            Mode::CopyInput { input, .. } => format!("Copy to: {input}_  (Enter/Esc)"),
            Mode::ExtractInput { input, .. } => format!("Extract to: {input}_  (Enter/Esc)"),
            Mode::Normal => self.status.clone().unwrap_or_else(|| {
                "Tab: switch pane  Up/Down: move  Enter/Right: open  Left: collapse  \
                 Delete: delete  r: rename  c: copy  x: extract  Esc: cancel/quit  q: quit"
                    .to_owned()
            }),
        }
    }

    fn handle_folders_key(&mut self, code: KeyCode) {
        let len = self.root.flatten().len();
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.tree_selected > 0 {
                    self.tree_selected -= 1;
                    self.load_contents_for_selected();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.tree_selected + 1 < len {
                    self.tree_selected += 1;
                    self.load_contents_for_selected();
                }
            }
            KeyCode::Right | KeyCode::Enter => self.expand_selected(),
            KeyCode::Left => self.collapse_selected(),
            _ => {}
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

    fn handle_contents_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.contents_selected > 0 {
                    self.contents_selected -= 1;
                    self.load_file_view();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.contents_selected + 1 < self.contents.len() {
                    self.contents_selected += 1;
                    self.load_file_view();
                }
            }
            KeyCode::Enter | KeyCode::Right => self.drill_into_selected(),
            _ => {}
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

    frame.render_widget(Paragraph::new(app.status_line()), rows[1]);
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

fn render_contents(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let items: Vec<ListItem<'_>> = app
        .contents
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
    use super::{App, Focus, FolderNode};
    use protocol::{DirectoryEntry, Response};
    use ratatui::crossterm::event::KeyCode;

    fn entries(names: &[(&str, bool)]) -> Vec<DirectoryEntry> {
        names
            .iter()
            .map(|(name, is_dir)| DirectoryEntry {
                name: (*name).to_owned(),
                is_dir: *is_dir,
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
        let mut app = App::new(std::env::temp_dir());
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
}
