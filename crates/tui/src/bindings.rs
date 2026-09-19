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
    use super::{Action, BINDINGS, find};
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
}
