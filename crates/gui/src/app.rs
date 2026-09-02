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
/// caller to justify it while five entries can still be read at a glance
/// (see `plugin-api`'s crate docs).
const PRESENTATION_PLUGINS: &[&dyn PluginPresentation] = &[
    &plugin_text::TextPresentation,
    &plugin_image::ImagePresentation,
    &plugin_archive::ArchivePresentation,
    &plugin_pdf::PdfPresentation,
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

/// The three-pane explorer's state.
pub struct App {
    root: FolderNode,
    folder_selected: usize,
    contents: Vec<DirectoryEntry>,
    content_selected: usize,
    file_view: Option<Response>,
    status: Option<String>,
    focus: Pane,
    pending_contents: Option<(Vec<usize>, Receiver<io::Result<Response>>)>,
    pending_file: Option<Receiver<io::Result<Response>>>,
}

impl App {
    /// Starts a new explorer rooted at `root`, and kicks off loading its
    /// contents in the background.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        let mut app = Self {
            root: FolderNode::root(root),
            folder_selected: 0,
            contents: Vec::new(),
            content_selected: 0,
            file_view: None,
            status: None,
            focus: Pane::Folders,
            pending_contents: None,
            pending_file: None,
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
    }

    fn apply_contents_result(&mut self, indices: &[usize], result: io::Result<Response>) {
        self.status = None;
        match result {
            Ok(Response::Directory { entries }) => {
                if let Some(node) = self.root.node_at_mut(indices) {
                    node.set_children_from(&entries);
                }
                self.contents = entries;
                self.content_selected = 0;
                self.load_file_view();
            }
            Ok(Response::Error { message }) => self.status = Some(message),
            Ok(Response::FileView { .. }) => {
                self.status = Some("expected a directory listing".to_owned());
            }
            Err(err) => self.status = Some(err.to_string()),
        }
    }

    /// Cancels any pending request; a late result is simply discarded when
    /// it arrives, since its receiver is dropped.
    pub fn cancel_pending(&mut self) {
        let cancelled = self.pending_contents.take().is_some() | self.pending_file.take().is_some();
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
            Some(Response::Directory { .. }) | None => String::new(),
        }
    }

    /// Display text for the status bar.
    #[must_use]
    pub fn status_text(&self) -> String {
        self.status.clone().unwrap_or_else(|| {
            "Click a folder or file. Double-click to open. Esc cancels a pending load.".to_owned()
        })
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
    use super::App;
    use protocol::{DirectoryEntry, Response};

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
}
