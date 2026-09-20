//! The terminal front end's keyboard bindings, in one table (#638).
//!
//! Before this, a binding's keys and the pane it answered in were spread
//! through `App::handle_key`'s own match arms, matched on `KeyCode` alone -
//! so every modifier was thrown away before it got there, and Ctrl+C read
//! the same as a plain `c`. Reading here instead, on the full key event
//! (code and modifiers together), means a binding cannot exist in the
//! dispatch code and be missing from the table a future keyboard reference
//! (#649) would read. Modelled on `crates/gui/src/shortcuts.rs`.

use crate::app::Focus;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Which pane a binding answers in - a `Folders`-, `Contents`- or
/// `File`-owned binding only when that pane has focus, a `Global` one
/// regardless.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// Answers regardless of which pane is focused.
    Global,
    /// The folders tree pane.
    Folders,
    /// The current folder's contents pane.
    Contents,
    /// The selected file's preview pane.
    File,
}

impl Owner {
    /// The heading the keyboard reference (#649) shows above this owner's
    /// entries.
    #[must_use]
    pub fn heading(self) -> &'static str {
        match self {
            Owner::Global => "Global",
            Owner::Folders => "Folders pane",
            Owner::Contents => "Contents pane",
            Owner::File => "File pane",
        }
    }
}

/// What a matched binding does to the [`App`](crate::app::App).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Quits immediately.
    Quit,
    /// Cancels whatever request is in flight, or quits if nothing is.
    CancelOrQuit,
    /// Moves keyboard focus to the next pane.
    FocusNext,
    /// Moves keyboard focus to the previous pane.
    FocusPrevious,
    /// Asks whether to delete the selected row.
    StartDelete,
    /// Starts renaming the selected row.
    StartRename,
    /// Starts copying the selected row.
    StartCopy,
    /// Starts extracting the selected archive.
    StartExtract,
    /// Starts naming a new, empty directory (#646).
    StartCreateDirectory,
    /// Starts naming a new, empty file (#646).
    StartCreateFile,
    /// Sends `Undo` for the immediately preceding operation (#646).
    StartUndo,
    /// Sends `Open` for the selected row (#646).
    StartOpen,
    /// Copies the selection onto the clipboard (#646).
    ClipboardCopy,
    /// Cuts the selection onto the clipboard (#646).
    ClipboardCut,
    /// Pastes the clipboard into the folder currently shown (#646).
    ClipboardPaste,
    /// Moves the folders tree cursor up.
    FoldersUp,
    /// Moves the folders tree cursor down.
    FoldersDown,
    /// Expands the selected folder.
    FoldersExpand,
    /// Collapses the selected folder, or steps out to its parent.
    FoldersCollapse,
    /// Moves the contents cursor up.
    ContentsUp,
    /// Moves the contents cursor down.
    ContentsDown,
    /// Extends the contents selection to the row above the cursor (#676).
    ExtendContentsUp,
    /// Extends the contents selection to the row below the cursor (#676).
    ExtendContentsDown,
    /// Extends the contents selection to the first row shown (#676).
    ExtendContentsHome,
    /// Extends the contents selection to the last row shown (#676).
    ExtendContentsEnd,
    /// Toggles the cursor row in or out of the contents selection and
    /// moves down one (#676).
    ToggleContentsSelected,
    /// Inverts the contents selection (#676).
    InvertContentsSelection,
    /// Opens the selected row if it is a folder.
    ContentsOpen,
    /// Sorts the contents by Name, reversing direction if already sorted
    /// by it.
    ContentsSortName,
    /// Sorts the contents by Type, reversing direction if already sorted
    /// by it.
    ContentsSortType,
    /// Sorts the contents by Size, reversing direction if already sorted
    /// by it.
    ContentsSortSize,
    /// Sorts the contents by Modified, reversing direction if already
    /// sorted by it.
    ContentsSortModified,
    /// Steps above the tree's own root, to its parent (#642).
    NavigateAboveRoot,
    /// Returns to the previously visited root.
    GoBack,
    /// Returns to the root Back moved away from.
    GoForward,
    /// Scrolls the File pane's text up one line.
    FileScrollUp,
    /// Scrolls the File pane's text down one line.
    FileScrollDown,
    /// Scrolls the File pane's text up one page.
    FileScrollPageUp,
    /// Scrolls the File pane's text down one page.
    FileScrollPageDown,
    /// Scrolls the File pane's text to its start.
    FileScrollHome,
    /// Scrolls the File pane's text to its end.
    FileScrollEnd,
    /// Switches the File pane to the view before the one it is showing.
    FileViewPrevious,
    /// Switches the File pane to the view after the one it is showing.
    FileViewNext,
    /// Activates the File pane's currently shown view - starts editing
    /// when it is the `"Edit"` view (#645).
    FileActivateView,
    /// Starts typing into the Contents pane's filter field (#650).
    StartFilter,
    /// Narrows the Contents pane to repositories with uncommitted changes,
    /// or lifts that narrowing if it is already applied (#650).
    ToggleChangedFilter,
    /// Widens the focused pane by one column, narrowing its neighbour
    /// (#650).
    WidenPane,
    /// Narrows the focused pane by one column, widening its neighbour
    /// (#650).
    NarrowPane,
    /// Maximises the focused pane to the whole terminal, or restores the
    /// three-pane layout if it is already maximised (#650).
    ToggleMaximize,
    /// Opens the "Go to Repository" switcher (#647).
    OpenSwitcher,
    /// Opens the cross-repository Find prompt (#647).
    StartFind,
    /// Opens the All Repositories view (#647/#591).
    OpenAllRepositories,
    /// Opens the Repos Directory roots view (#648).
    OpenReposRoots,
    /// Opens the Certificates view (#621/#681).
    OpenCertificates,
    /// Opens the Contents pane cursor's folder in an editor (#674).
    ContentsOpenInEditor,
    /// Opens the Folders pane's selected folder in an editor (#674).
    FoldersOpenInEditor,
    /// Copies the Contents pane cursor's folder's full path (#674).
    ContentsCopyPath,
    /// Copies the Folders pane's selected folder's full path (#674).
    FoldersCopyPath,
    /// Copies the Contents pane cursor's folder's remote address (#674).
    ContentsCopyRemoteAddress,
    /// Copies the Folders pane's selected folder's remote address (#674).
    FoldersCopyRemoteAddress,
    /// Shows the Contents pane cursor's folder in the platform's file
    /// manager (#674).
    ContentsShowInFileManager,
    /// Shows the Folders pane's selected folder in the platform's file
    /// manager (#674).
    FoldersShowInFileManager,
    /// Opens the Contents pane cursor's repository's web page in the
    /// reader's browser (#679).
    ContentsOpenOnTheWeb,
    /// Opens the keyboard reference (#649).
    OpenKeyboardReference,
    /// Opens the command palette (#649).
    OpenCommandPalette,
    /// Re-reads the selected folder's listing, the tree beneath the root,
    /// and the facts the File pane is showing (#675).
    Refresh,
}

/// One row of the table: the keys that trigger it, the pane it answers in,
/// what it does in plain words, and the action dispatching it runs.
pub struct Binding {
    /// The pane this binding answers in.
    pub owner: Owner,
    /// The key itself.
    pub code: KeyCode,
    /// The modifiers that must be held, exactly - so a plain `c` and a
    /// Ctrl+C are never the same binding.
    pub modifiers: KeyModifiers,
    /// What the binding does, in plain words.
    pub description: &'static str,
    /// What dispatching this binding runs.
    pub action: Action,
}

impl Binding {
    fn matches(&self, key: KeyEvent, focus: Focus) -> bool {
        self.code == key.code
            && self.modifiers == key.modifiers
            && match self.owner {
                Owner::Global => true,
                Owner::Folders => focus == Focus::Folders,
                Owner::Contents => focus == Focus::Contents,
                Owner::File => focus == Focus::File,
            }
    }

    /// The keys as a reader sees them, e.g. `"Ctrl+P"` or `"Shift+Tab"`
    /// (#649). Mirrors `crates/gui/src/shortcuts.rs`'s own `Binding::label`,
    /// without its macOS "Cmd" substitution - a terminal's Ctrl key is
    /// never relabelled (D16).
    #[must_use]
    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push("Ctrl".to_owned());
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push("Alt".to_owned());
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            parts.push("Shift".to_owned());
        }
        // A Ctrl or Alt chord's letter is typed without Shift - crossterm
        // reports it lowercase - but the conventional written form is the
        // capital, `"Ctrl+P"` rather than `"Ctrl+p"`; a plain letter with
        // no modifier keeps whatever case the table itself gives it.
        let held_with_control_or_alt = self
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        parts.push(match self.code {
            KeyCode::Char(c) if held_with_control_or_alt => c.to_ascii_uppercase().to_string(),
            code => key_text(code),
        });
        parts.join("+")
    }
}

/// [`Binding::label`]'s text for the key itself, ignoring modifiers -
/// `BackTab` is crossterm's own name for Shift+Tab, carrying no modifier
/// of its own, so it is spelled out here rather than left to read as a
/// bare `"Tab"`.
fn key_text(code: KeyCode) -> String {
    match code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::F(n) => format!("F{n}"),
        KeyCode::Up => "Up".to_owned(),
        KeyCode::Down => "Down".to_owned(),
        KeyCode::Left => "Left".to_owned(),
        KeyCode::Right => "Right".to_owned(),
        KeyCode::Home => "Home".to_owned(),
        KeyCode::End => "End".to_owned(),
        KeyCode::PageUp => "PageUp".to_owned(),
        KeyCode::PageDown => "PageDown".to_owned(),
        KeyCode::Delete => "Delete".to_owned(),
        KeyCode::Insert => "Insert".to_owned(),
        KeyCode::Enter => "Enter".to_owned(),
        KeyCode::Esc => "Esc".to_owned(),
        KeyCode::Tab => "Tab".to_owned(),
        KeyCode::BackTab => "Shift+Tab".to_owned(),
        other => format!("{other:?}"),
    }
}

/// One command the command palette (#649) can run: every distinct
/// [`Action`] in [`BINDINGS`], with every key bound to it joined for the
/// "shows each one's keys beside it" requirement.
#[derive(Clone)]
pub struct PaletteEntry {
    /// The pane this command answers in - decides whether it currently
    /// applies (`crates/tui/src/app.rs`'s `App::palette_entry_applies`).
    pub owner: Owner,
    /// Every key bound to this command, joined with `"/"`.
    pub keys: String,
    /// What the command does, in plain words - what a typed query is
    /// matched against.
    pub description: &'static str,
    /// What running the command does.
    pub action: Action,
}

/// Every distinct [`Action`] in [`BINDINGS`], in the order it first
/// appears there, each with every key bound to it collected onto one
/// entry - so the palette lists a command once no matter how many keys
/// reach it.
#[must_use]
pub fn palette_entries() -> Vec<PaletteEntry> {
    let mut entries: Vec<PaletteEntry> = Vec::new();
    for binding in BINDINGS {
        match entries
            .iter_mut()
            .find(|entry| entry.action == binding.action)
        {
            Some(entry) => {
                entry.keys.push('/');
                entry.keys.push_str(&binding.label());
            }
            None => entries.push(PaletteEntry {
                owner: binding.owner,
                keys: binding.label(),
                description: binding.description,
                action: binding.action,
            }),
        }
    }
    entries
}

/// Every [`BINDINGS`] entry, as a line for the keyboard reference (#649):
/// a heading naming the pane a group of entries answers in, followed by
/// "key  description" lines for the entries themselves - built from the
/// table so a binding cannot exist without a line here. Grouped as
/// [`Owner::Global`], [`Owner::Folders`], [`Owner::Contents`], then
/// [`Owner::File`], regardless of the table's own declaration order -
/// unlike `crates/gui/src/shortcuts.rs`'s table, this one's owners are not
/// declared contiguously, so each group is collected by filtering rather
/// than by merely watching for a change from the previous row.
#[must_use]
pub fn reference_lines() -> Vec<String> {
    let mut lines = Vec::new();
    for owner in [Owner::Global, Owner::Folders, Owner::Contents, Owner::File] {
        lines.push(owner.heading().to_owned());
        for binding in BINDINGS.iter().filter(|binding| binding.owner == owner) {
            lines.push(format!("{}  {}", binding.label(), binding.description));
        }
    }
    lines
}

/// Every keyboard binding `App::handle_key` answers while no prompt is
/// open.
///
/// Nothing here binds Ctrl+A or Ctrl+B, which a host multiplexer needs
/// (GUIDANCE.md §2.2, D16) - `no_binding_takes_a_host_multiplexer_s_key`
/// below asserts it.
pub const BINDINGS: &[Binding] = &[
    Binding {
        owner: Owner::Global,
        code: KeyCode::Char('q'),
        modifiers: KeyModifiers::NONE,
        description: "Quit",
        action: Action::Quit,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::Char('q'),
        modifiers: KeyModifiers::CONTROL,
        description: "Quit",
        action: Action::Quit,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::Esc,
        modifiers: KeyModifiers::NONE,
        description: "Cancel, or quit",
        action: Action::CancelOrQuit,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::Tab,
        modifiers: KeyModifiers::NONE,
        description: "Switch pane",
        action: Action::FocusNext,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::BackTab,
        modifiers: KeyModifiers::NONE,
        description: "Switch pane backwards",
        action: Action::FocusPrevious,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Delete,
        modifiers: KeyModifiers::NONE,
        description: "Delete",
        action: Action::StartDelete,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('r'),
        modifiers: KeyModifiers::NONE,
        description: "Rename",
        action: Action::StartRename,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('c'),
        modifiers: KeyModifiers::NONE,
        description: "Copy",
        action: Action::StartCopy,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('x'),
        modifiers: KeyModifiers::NONE,
        description: "Extract",
        action: Action::StartExtract,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('D'),
        modifiers: KeyModifiers::NONE,
        description: "New folder",
        action: Action::StartCreateDirectory,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('F'),
        modifiers: KeyModifiers::NONE,
        description: "New file",
        action: Action::StartCreateFile,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('O'),
        modifiers: KeyModifiers::NONE,
        description: "Open with the platform",
        action: Action::StartOpen,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::Char('z'),
        modifiers: KeyModifiers::CONTROL,
        description: "Undo",
        action: Action::StartUndo,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('c'),
        modifiers: KeyModifiers::CONTROL,
        description: "Copy to clipboard",
        action: Action::ClipboardCopy,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('x'),
        modifiers: KeyModifiers::CONTROL,
        description: "Cut to clipboard",
        action: Action::ClipboardCut,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('v'),
        modifiers: KeyModifiers::CONTROL,
        description: "Paste",
        action: Action::ClipboardPaste,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Up,
        modifiers: KeyModifiers::NONE,
        description: "Move up",
        action: Action::ContentsUp,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('k'),
        modifiers: KeyModifiers::NONE,
        description: "Move up",
        action: Action::ContentsUp,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Down,
        modifiers: KeyModifiers::NONE,
        description: "Move down",
        action: Action::ContentsDown,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('j'),
        modifiers: KeyModifiers::NONE,
        description: "Move down",
        action: Action::ContentsDown,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Up,
        modifiers: KeyModifiers::SHIFT,
        description: "Extend selection up",
        action: Action::ExtendContentsUp,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Down,
        modifiers: KeyModifiers::SHIFT,
        description: "Extend selection down",
        action: Action::ExtendContentsDown,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Home,
        modifiers: KeyModifiers::SHIFT,
        description: "Extend selection to the top",
        action: Action::ExtendContentsHome,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::End,
        modifiers: KeyModifiers::SHIFT,
        description: "Extend selection to the bottom",
        action: Action::ExtendContentsEnd,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Insert,
        modifiers: KeyModifiers::NONE,
        description: "Select and move down",
        action: Action::ToggleContentsSelected,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('*'),
        modifiers: KeyModifiers::NONE,
        description: "Invert selection",
        action: Action::InvertContentsSelection,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Enter,
        modifiers: KeyModifiers::NONE,
        description: "Open",
        action: Action::ContentsOpen,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Right,
        modifiers: KeyModifiers::NONE,
        description: "Open",
        action: Action::ContentsOpen,
    },
    Binding {
        owner: Owner::Folders,
        code: KeyCode::Up,
        modifiers: KeyModifiers::NONE,
        description: "Move up",
        action: Action::FoldersUp,
    },
    Binding {
        owner: Owner::Folders,
        code: KeyCode::Char('k'),
        modifiers: KeyModifiers::NONE,
        description: "Move up",
        action: Action::FoldersUp,
    },
    Binding {
        owner: Owner::Folders,
        code: KeyCode::Down,
        modifiers: KeyModifiers::NONE,
        description: "Move down",
        action: Action::FoldersDown,
    },
    Binding {
        owner: Owner::Folders,
        code: KeyCode::Char('j'),
        modifiers: KeyModifiers::NONE,
        description: "Move down",
        action: Action::FoldersDown,
    },
    Binding {
        owner: Owner::Folders,
        code: KeyCode::Right,
        modifiers: KeyModifiers::NONE,
        description: "Expand",
        action: Action::FoldersExpand,
    },
    Binding {
        owner: Owner::Folders,
        code: KeyCode::Enter,
        modifiers: KeyModifiers::NONE,
        description: "Expand",
        action: Action::FoldersExpand,
    },
    Binding {
        owner: Owner::Folders,
        code: KeyCode::Left,
        modifiers: KeyModifiers::NONE,
        description: "Collapse",
        action: Action::FoldersCollapse,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('n'),
        modifiers: KeyModifiers::NONE,
        description: "Sort by name",
        action: Action::ContentsSortName,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('t'),
        modifiers: KeyModifiers::NONE,
        description: "Sort by type",
        action: Action::ContentsSortType,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('s'),
        modifiers: KeyModifiers::NONE,
        description: "Sort by size",
        action: Action::ContentsSortSize,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('m'),
        modifiers: KeyModifiers::NONE,
        description: "Sort by modified",
        action: Action::ContentsSortModified,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::Up,
        modifiers: KeyModifiers::ALT,
        description: "Go to the parent folder",
        action: Action::NavigateAboveRoot,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::Left,
        modifiers: KeyModifiers::ALT,
        description: "Go back",
        action: Action::GoBack,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::Right,
        modifiers: KeyModifiers::ALT,
        description: "Go forward",
        action: Action::GoForward,
    },
    Binding {
        owner: Owner::File,
        code: KeyCode::Up,
        modifiers: KeyModifiers::NONE,
        description: "Scroll up",
        action: Action::FileScrollUp,
    },
    Binding {
        owner: Owner::File,
        code: KeyCode::Char('k'),
        modifiers: KeyModifiers::NONE,
        description: "Scroll up",
        action: Action::FileScrollUp,
    },
    Binding {
        owner: Owner::File,
        code: KeyCode::Down,
        modifiers: KeyModifiers::NONE,
        description: "Scroll down",
        action: Action::FileScrollDown,
    },
    Binding {
        owner: Owner::File,
        code: KeyCode::Char('j'),
        modifiers: KeyModifiers::NONE,
        description: "Scroll down",
        action: Action::FileScrollDown,
    },
    Binding {
        owner: Owner::File,
        code: KeyCode::PageUp,
        modifiers: KeyModifiers::NONE,
        description: "Scroll up a page",
        action: Action::FileScrollPageUp,
    },
    Binding {
        owner: Owner::File,
        code: KeyCode::PageDown,
        modifiers: KeyModifiers::NONE,
        description: "Scroll down a page",
        action: Action::FileScrollPageDown,
    },
    Binding {
        owner: Owner::File,
        code: KeyCode::Home,
        modifiers: KeyModifiers::NONE,
        description: "Scroll to the start",
        action: Action::FileScrollHome,
    },
    Binding {
        owner: Owner::File,
        code: KeyCode::End,
        modifiers: KeyModifiers::NONE,
        description: "Scroll to the end",
        action: Action::FileScrollEnd,
    },
    Binding {
        owner: Owner::File,
        code: KeyCode::Left,
        modifiers: KeyModifiers::NONE,
        description: "Previous view",
        action: Action::FileViewPrevious,
    },
    Binding {
        owner: Owner::File,
        code: KeyCode::Right,
        modifiers: KeyModifiers::NONE,
        description: "Next view",
        action: Action::FileViewNext,
    },
    Binding {
        owner: Owner::File,
        code: KeyCode::Enter,
        modifiers: KeyModifiers::NONE,
        description: "Edit",
        action: Action::FileActivateView,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('/'),
        modifiers: KeyModifiers::NONE,
        description: "Filter",
        action: Action::StartFilter,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('u'),
        modifiers: KeyModifiers::NONE,
        description: "Filter to uncommitted changes",
        action: Action::ToggleChangedFilter,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::Char(']'),
        modifiers: KeyModifiers::NONE,
        description: "Widen the focused pane",
        action: Action::WidenPane,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::Char('['),
        modifiers: KeyModifiers::NONE,
        description: "Narrow the focused pane",
        action: Action::NarrowPane,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::Char('z'),
        modifiers: KeyModifiers::NONE,
        description: "Maximise/restore the focused pane",
        action: Action::ToggleMaximize,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::Char('p'),
        modifiers: KeyModifiers::CONTROL,
        description: "Go to a repository",
        action: Action::OpenSwitcher,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::Char('f'),
        modifiers: KeyModifiers::CONTROL,
        description: "Find by name",
        action: Action::StartFind,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::Char('r'),
        modifiers: KeyModifiers::CONTROL,
        description: "All repositories",
        action: Action::OpenAllRepositories,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::Char('d'),
        modifiers: KeyModifiers::CONTROL,
        description: "Repos Directory",
        action: Action::OpenReposRoots,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::Char('t'),
        modifiers: KeyModifiers::CONTROL,
        description: "Certificates",
        action: Action::OpenCertificates,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('e'),
        modifiers: KeyModifiers::NONE,
        description: "Open in editor",
        action: Action::ContentsOpenInEditor,
    },
    Binding {
        owner: Owner::Folders,
        code: KeyCode::Char('e'),
        modifiers: KeyModifiers::NONE,
        description: "Open in editor",
        action: Action::FoldersOpenInEditor,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('y'),
        modifiers: KeyModifiers::NONE,
        description: "Copy path",
        action: Action::ContentsCopyPath,
    },
    Binding {
        owner: Owner::Folders,
        code: KeyCode::Char('y'),
        modifiers: KeyModifiers::NONE,
        description: "Copy path",
        action: Action::FoldersCopyPath,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('R'),
        modifiers: KeyModifiers::NONE,
        description: "Copy remote address",
        action: Action::ContentsCopyRemoteAddress,
    },
    Binding {
        owner: Owner::Folders,
        code: KeyCode::Char('R'),
        modifiers: KeyModifiers::NONE,
        description: "Copy remote address",
        action: Action::FoldersCopyRemoteAddress,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('f'),
        modifiers: KeyModifiers::NONE,
        description: "Show in file manager",
        action: Action::ContentsShowInFileManager,
    },
    Binding {
        owner: Owner::Folders,
        code: KeyCode::Char('f'),
        modifiers: KeyModifiers::NONE,
        description: "Show in file manager",
        action: Action::FoldersShowInFileManager,
    },
    Binding {
        owner: Owner::Contents,
        code: KeyCode::Char('w'),
        modifiers: KeyModifiers::NONE,
        description: "Open on the web",
        action: Action::ContentsOpenOnTheWeb,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::F(1),
        modifiers: KeyModifiers::NONE,
        description: "Keyboard reference",
        action: Action::OpenKeyboardReference,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::F(5),
        modifiers: KeyModifiers::NONE,
        description: "Refresh the listing",
        action: Action::Refresh,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::Char('?'),
        modifiers: KeyModifiers::NONE,
        description: "Keyboard reference",
        action: Action::OpenKeyboardReference,
    },
    Binding {
        owner: Owner::Global,
        code: KeyCode::Char('k'),
        modifiers: KeyModifiers::CONTROL,
        description: "Command palette",
        action: Action::OpenCommandPalette,
    },
];

/// The action `key` runs with `focus` currently held, if any binding
/// answers it.
#[must_use]
pub fn find(key: KeyEvent, focus: Focus) -> Option<Action> {
    BINDINGS
        .iter()
        .find(|binding| binding.matches(key, focus))
        .map(|binding| binding.action)
}

#[cfg(test)]
mod tests {
    use super::{Action, BINDINGS, find, palette_entries, reference_lines};
    use crate::app::Focus;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn no_binding_takes_a_host_multiplexer_s_key() {
        for binding in BINDINGS {
            let steals_it = binding.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(binding.code, KeyCode::Char('a' | 'b' | 'A' | 'B'));
            assert!(
                !steals_it,
                "a binding on Ctrl+{:?} would steal a key a host multiplexer needs \
                 (GUIDANCE.md §2.2)",
                binding.code
            );
        }
    }

    #[test]
    fn ctrl_c_does_not_collide_with_the_plain_copy_binding() {
        assert_eq!(
            find(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                Focus::Contents
            ),
            Some(Action::ClipboardCopy),
            "Ctrl+C must not read as the plain `c` copy binding - it copies to the \
             clipboard instead (#646)"
        );
        assert_eq!(
            find(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
                Focus::Contents
            ),
            Some(Action::StartCopy)
        );
    }

    #[test]
    fn ctrl_q_and_plain_q_both_quit() {
        assert_eq!(
            find(
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
                Focus::Folders
            ),
            Some(Action::Quit)
        );
        assert_eq!(
            find(
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
                Focus::Folders
            ),
            Some(Action::Quit)
        );
    }

    #[test]
    fn a_contents_owned_binding_does_not_answer_while_folders_has_focus() {
        assert_eq!(
            find(
                KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
                Focus::Folders
            ),
            None
        );
    }

    #[test]
    fn a_binding_s_label_names_its_modifiers_and_key() {
        let ctrl_p = BINDINGS
            .iter()
            .find(|binding| binding.action == Action::OpenSwitcher)
            .expect("Ctrl+P opens the switcher");
        assert_eq!(ctrl_p.label(), "Ctrl+P");

        let shift_up = BINDINGS
            .iter()
            .find(|binding| binding.action == Action::ExtendContentsUp)
            .expect("Shift+Up extends the contents selection");
        assert_eq!(shift_up.label(), "Shift+Up");

        let back_tab = BINDINGS
            .iter()
            .find(|binding| binding.action == Action::FocusPrevious)
            .expect("BackTab switches panes backwards");
        assert_eq!(back_tab.label(), "Shift+Tab");
    }

    #[test]
    fn every_binding_appears_in_the_keyboard_reference() {
        let lines = reference_lines();
        for binding in BINDINGS {
            let line = format!("{}  {}", binding.label(), binding.description);
            assert!(
                lines.contains(&line),
                "{line:?} from the binding table is missing from the reference"
            );
        }
    }

    #[test]
    fn the_reference_groups_each_owner_under_its_own_heading() {
        let lines = reference_lines();
        for owner_heading in ["Global", "Folders pane", "Contents pane", "File pane"] {
            assert_eq!(
                lines.iter().filter(|line| *line == owner_heading).count(),
                1,
                "{owner_heading:?} should appear exactly once, as a heading"
            );
        }
    }

    #[test]
    fn the_palette_lists_a_command_once_with_every_key_bound_to_it() {
        let entries = palette_entries();
        let quit = entries
            .iter()
            .find(|entry| entry.action == Action::Quit)
            .expect("Quit is bound and so appears once in the palette");
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.action == Action::Quit)
                .count(),
            1,
            "Quit is bound to both `q` and Ctrl+Q, but should appear once"
        );
        assert!(
            quit.keys.contains('Q'),
            "{:?} should name its keys",
            quit.keys
        );
    }
}
