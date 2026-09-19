//! The keyboard-shortcut table behind Help > Keyboard shortcuts (#585).
//!
//! One list, read by both halves that must not drift apart: the window's
//! `key-scope` in `ui/app.slint`, which a test in
//! `tests/keyboard_shortcuts_in_the_window.rs` dispatches every entry's
//! keys against and checks the named callback fires, and the sheet itself,
//! whose rows [`rows`] builds. A binding removed from the markup, or added
//! there and not here, fails that test rather than going unnoticed.

use slint::platform::Key;

/// The component a binding belongs to - what the sheet groups its rows
/// under. Grouped by owner rather than by feature so the table survives
/// #615's split of the markup into pane components and #617's per-pane
/// key handling, per the correction on #585.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// Chrome that answers regardless of which pane is focused.
    Window,
    /// Row navigation the Folders and Contents panes share.
    FoldersAndContents,
    /// Operations that act on the Contents pane's selection.
    Contents,
    /// The path field above the panes.
    AddressBar,
    /// The File pane and the editor it opens into.
    FilePaneAndEditor,
    /// The cross-repository search and the Contents pane's filter.
    Find,
}

impl Owner {
    /// The heading the sheet shows above this owner's rows.
    #[must_use]
    pub fn heading(self) -> &'static str {
        match self {
            Owner::Window => "Window",
            Owner::FoldersAndContents => "Folders and Contents panes",
            Owner::Contents => "Contents pane",
            Owner::AddressBar => "Address bar",
            Owner::FilePaneAndEditor => "File pane and editor",
            Owner::Find => "Find",
        }
    }
}

/// The physical key a binding presses: a plain letter, dispatched as
/// typed text, or a named key such as `F2` or `Home`, carrying both the
/// [`Key`] a test dispatches and the word the sheet shows for it.
#[derive(Clone, Copy)]
pub enum Physical {
    /// A letter key, always combined with at least one modifier below -
    /// Slint reports it as the typed character, not a named `Key`.
    Letter(char),
    /// A key with no printable character of its own.
    Named(Key, &'static str),
}

impl Physical {
    fn label(self) -> String {
        match self {
            Physical::Letter(c) => c.to_string(),
            Physical::Named(_, name) => name.to_string(),
        }
    }

    fn dispatch_text(self) -> String {
        match self {
            Physical::Letter(c) => c.to_string(),
            Physical::Named(key, _) => char::from(key).to_string(),
        }
    }
}

/// What dispatching a binding's keys is asserted to fire. Named rather
/// than a bare string, so a typo in one place cannot make the walking
/// test compare a string against itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fires {
    /// `back-requested()`.
    BackRequested,
    /// `forward-requested()`.
    ForwardRequested,
    /// `pane-cycled(int)`.
    PaneCycled(i32),
    /// `refresh-requested()`.
    RefreshRequested,
    /// `undo-requested()`.
    UndoRequested,
    /// `save-requested()`.
    SaveRequested,
    /// `edge-requested(int)`.
    EdgeRequested(i32),
    /// `edge-extended(int)`.
    EdgeExtended(i32),
    /// `selection-moved(int)`.
    SelectionMoved(i32),
    /// `selection-extended(int)`.
    SelectionExtended(i32),
    /// `parent-requested()`.
    ParentRequested,
    /// `return-pressed()`.
    ReturnPressed,
    /// `content-rename-requested()`.
    ContentRenameRequested,
    /// `delete-requested()`.
    DeleteRequested,
    /// `backspace-pressed()`.
    BackspacePressed,
    /// `new-folder-requested()`.
    NewFolderRequested,
    /// `new-file-requested()`.
    NewFileRequested,
    /// `select-all-requested()`.
    SelectAllRequested,
    /// `clipboard-copy-requested()`.
    ClipboardCopyRequested,
    /// `clipboard-cut-requested()`.
    ClipboardCutRequested,
    /// `clipboard-paste-requested()`.
    ClipboardPasteRequested,
    /// `key-text(string)`, with the text a binding's key dispatches.
    KeyText(&'static str),
    /// `path-edit-requested()`.
    PathEditRequested,
    /// `edit-requested()`.
    EditRequested,
    /// `find-requested()`.
    FindRequested,
    /// `filter-focus-requested()`.
    FilterFocusRequested,
    /// `switcher-open-requested()`.
    SwitcherOpenRequested,
    /// `cancel-requested()`.
    CancelRequested,
    /// `zoom-in-requested()`.
    ZoomInRequested,
    /// `zoom-out-requested()`.
    ZoomOutRequested,
    /// `zoom-reset-requested()`.
    ZoomResetRequested,
    /// `changed-filter-toggled()`.
    ChangedFilterToggled,
}

impl Fires {
    /// The name a test compares against, including the argument for a
    /// callback that takes one - so a binding wired to the right callback
    /// with the wrong argument still fails.
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Fires::BackRequested => "back-requested".to_owned(),
            Fires::ForwardRequested => "forward-requested".to_owned(),
            Fires::PaneCycled(n) => format!("pane-cycled({n})"),
            Fires::RefreshRequested => "refresh-requested".to_owned(),
            Fires::UndoRequested => "undo-requested".to_owned(),
            Fires::SaveRequested => "save-requested".to_owned(),
            Fires::EdgeRequested(n) => format!("edge-requested({n})"),
            Fires::EdgeExtended(n) => format!("edge-extended({n})"),
            Fires::SelectionMoved(n) => format!("selection-moved({n})"),
            Fires::SelectionExtended(n) => format!("selection-extended({n})"),
            Fires::ParentRequested => "parent-requested".to_owned(),
            Fires::ReturnPressed => "return-pressed".to_owned(),
            Fires::ContentRenameRequested => "content-rename-requested".to_owned(),
            Fires::DeleteRequested => "delete-requested".to_owned(),
            Fires::BackspacePressed => "backspace-pressed".to_owned(),
            Fires::NewFolderRequested => "new-folder-requested".to_owned(),
            Fires::NewFileRequested => "new-file-requested".to_owned(),
            Fires::SelectAllRequested => "select-all-requested".to_owned(),
            Fires::ClipboardCopyRequested => "clipboard-copy-requested".to_owned(),
            Fires::ClipboardCutRequested => "clipboard-cut-requested".to_owned(),
            Fires::ClipboardPasteRequested => "clipboard-paste-requested".to_owned(),
            Fires::KeyText(text) => format!("key-text({text:?})"),
            Fires::PathEditRequested => "path-edit-requested".to_owned(),
            Fires::EditRequested => "edit-requested".to_owned(),
            Fires::FindRequested => "find-requested".to_owned(),
            Fires::FilterFocusRequested => "filter-focus-requested".to_owned(),
            Fires::SwitcherOpenRequested => "switcher-open-requested".to_owned(),
            Fires::CancelRequested => "cancel-requested".to_owned(),
            Fires::ZoomInRequested => "zoom-in-requested".to_owned(),
            Fires::ZoomOutRequested => "zoom-out-requested".to_owned(),
            Fires::ZoomResetRequested => "zoom-reset-requested".to_owned(),
            Fires::ChangedFilterToggled => "changed-filter-toggled".to_owned(),
        }
    }
}

/// One row of the table: a key combination, the pane it belongs to, what
/// it does in plain words, and the callback pressing it fires.
pub struct Binding {
    /// The pane the sheet lists this row under.
    pub owner: Owner,
    /// Whether Control (Command on macOS) is held.
    pub control: bool,
    /// Whether Alt is held.
    pub alt: bool,
    /// Whether Shift is held.
    pub shift: bool,
    /// The key itself.
    pub key: Physical,
    /// What the binding does, in plain words, for the sheet.
    pub description: &'static str,
    /// What a test dispatching this binding's keys asserts fired.
    pub fires: Fires,
}

impl Binding {
    /// The keys as a reader sees them, e.g. `"Ctrl+Shift+N"` - `"Cmd"` for
    /// `"Ctrl"` on macOS, GUIDANCE.md §2.3.
    #[must_use]
    pub fn label(&self, mac: bool) -> String {
        let mut parts = Vec::new();
        if self.control {
            parts.push(if mac { "Cmd" } else { "Ctrl" }.to_owned());
        }
        if self.alt {
            parts.push("Alt".to_owned());
        }
        if self.shift {
            parts.push("Shift".to_owned());
        }
        parts.push(self.key.label());
        parts.join("+")
    }

    /// The text a `KeyPressed` event carries for this binding's own key
    /// (not its modifiers).
    #[must_use]
    pub fn dispatch_text(&self) -> String {
        self.key.dispatch_text()
    }

    /// The modifier keys held for this binding, each to be dispatched as
    /// its own `KeyPressed`/`KeyReleased` pair around the key itself.
    #[must_use]
    pub fn modifiers(&self) -> Vec<Key> {
        let mut keys = Vec::new();
        if self.control {
            keys.push(Key::Control);
        }
        if self.alt {
            keys.push(Key::Alt);
        }
        if self.shift {
            keys.push(Key::Shift);
        }
        keys
    }
}

/// Every keyboard binding `ui/app.slint`'s `key-scope` answers, in the
/// order the sheet shows them - grouped by [`Owner`], so entries sharing
/// one belong together in this list.
pub const BINDINGS: &[Binding] = &[
    // Window: chrome that answers regardless of which pane is focused.
    Binding {
        owner: Owner::Window,
        control: false,
        alt: true,
        shift: false,
        key: Physical::Named(Key::LeftArrow, "Left"),
        description: "Go back",
        fires: Fires::BackRequested,
    },
    Binding {
        owner: Owner::Window,
        control: false,
        alt: true,
        shift: false,
        key: Physical::Named(Key::RightArrow, "Right"),
        description: "Go forward",
        fires: Fires::ForwardRequested,
    },
    Binding {
        owner: Owner::Window,
        control: false,
        alt: false,
        shift: false,
        key: Physical::Named(Key::LeftArrow, "Left"),
        description: "Move the keyboard to the previous pane",
        fires: Fires::PaneCycled(-1),
    },
    Binding {
        owner: Owner::Window,
        control: false,
        alt: false,
        shift: false,
        key: Physical::Named(Key::RightArrow, "Right"),
        description: "Move the keyboard to the next pane",
        fires: Fires::PaneCycled(1),
    },
    Binding {
        owner: Owner::Window,
        control: false,
        alt: false,
        shift: false,
        key: Physical::Named(Key::F5, "F5"),
        description: "Refresh the listing",
        fires: Fires::RefreshRequested,
    },
    Binding {
        owner: Owner::Window,
        control: true,
        alt: false,
        shift: false,
        key: Physical::Letter('Z'),
        description: "Undo the last edit, or the last file operation",
        fires: Fires::UndoRequested,
    },
    Binding {
        owner: Owner::Window,
        control: true,
        alt: false,
        shift: false,
        key: Physical::Letter('S'),
        description: "Save the open file",
        fires: Fires::SaveRequested,
    },
    Binding {
        owner: Owner::Window,
        control: true,
        alt: false,
        shift: false,
        key: Physical::Letter('P'),
        description: "Go to a repository by name",
        fires: Fires::SwitcherOpenRequested,
    },
    Binding {
        owner: Owner::Window,
        control: true,
        alt: false,
        shift: false,
        key: Physical::Letter('+'),
        description: "Zoom in",
        fires: Fires::ZoomInRequested,
    },
    Binding {
        owner: Owner::Window,
        control: true,
        alt: false,
        shift: false,
        key: Physical::Letter('='),
        description: "Zoom in",
        fires: Fires::ZoomInRequested,
    },
    Binding {
        owner: Owner::Window,
        control: true,
        alt: false,
        shift: false,
        key: Physical::Letter('-'),
        description: "Zoom out",
        fires: Fires::ZoomOutRequested,
    },
    Binding {
        owner: Owner::Window,
        control: true,
        alt: false,
        shift: false,
        key: Physical::Letter('0'),
        description: "Reset the zoom to 100%",
        fires: Fires::ZoomResetRequested,
    },
    // Folders and Contents panes: the row navigation both share.
    Binding {
        owner: Owner::FoldersAndContents,
        control: false,
        alt: false,
        shift: false,
        key: Physical::Named(Key::Home, "Home"),
        description: "Go to the first row",
        fires: Fires::EdgeRequested(0),
    },
    Binding {
        owner: Owner::FoldersAndContents,
        control: false,
        alt: false,
        shift: true,
        key: Physical::Named(Key::Home, "Home"),
        description: "Extend the selection to the first row",
        fires: Fires::EdgeExtended(0),
    },
    Binding {
        owner: Owner::FoldersAndContents,
        control: false,
        alt: false,
        shift: false,
        key: Physical::Named(Key::End, "End"),
        description: "Go to the last row",
        fires: Fires::EdgeRequested(1),
    },
    Binding {
        owner: Owner::FoldersAndContents,
        control: false,
        alt: false,
        shift: true,
        key: Physical::Named(Key::End, "End"),
        description: "Extend the selection to the last row",
        fires: Fires::EdgeExtended(1),
    },
    Binding {
        owner: Owner::FoldersAndContents,
        control: false,
        alt: false,
        shift: false,
        key: Physical::Named(Key::UpArrow, "Up"),
        description: "Move up one row",
        fires: Fires::SelectionMoved(-1),
    },
    Binding {
        owner: Owner::FoldersAndContents,
        control: false,
        alt: false,
        shift: true,
        key: Physical::Named(Key::UpArrow, "Up"),
        description: "Extend the selection up one row",
        fires: Fires::SelectionExtended(-1),
    },
    Binding {
        owner: Owner::FoldersAndContents,
        control: false,
        alt: false,
        shift: false,
        key: Physical::Named(Key::DownArrow, "Down"),
        description: "Move down one row",
        fires: Fires::SelectionMoved(1),
    },
    Binding {
        owner: Owner::FoldersAndContents,
        control: false,
        alt: false,
        shift: true,
        key: Physical::Named(Key::DownArrow, "Down"),
        description: "Extend the selection down one row",
        fires: Fires::SelectionExtended(1),
    },
    Binding {
        owner: Owner::FoldersAndContents,
        control: false,
        alt: false,
        shift: false,
        key: Physical::Named(Key::PageUp, "Page Up"),
        description: "Move up one page",
        fires: Fires::SelectionMoved(-10),
    },
    Binding {
        owner: Owner::FoldersAndContents,
        control: false,
        alt: false,
        shift: true,
        key: Physical::Named(Key::PageUp, "Page Up"),
        description: "Extend the selection up one page",
        fires: Fires::SelectionExtended(-10),
    },
    Binding {
        owner: Owner::FoldersAndContents,
        control: false,
        alt: false,
        shift: false,
        key: Physical::Named(Key::PageDown, "Page Down"),
        description: "Move down one page",
        fires: Fires::SelectionMoved(10),
    },
    Binding {
        owner: Owner::FoldersAndContents,
        control: false,
        alt: false,
        shift: true,
        key: Physical::Named(Key::PageDown, "Page Down"),
        description: "Extend the selection down one page",
        fires: Fires::SelectionExtended(10),
    },
    Binding {
        owner: Owner::FoldersAndContents,
        control: false,
        alt: true,
        shift: false,
        key: Physical::Named(Key::UpArrow, "Up"),
        description: "Go to the parent folder",
        fires: Fires::ParentRequested,
    },
    // Contents pane: operations on its selection.
    Binding {
        owner: Owner::Contents,
        control: false,
        alt: false,
        shift: false,
        key: Physical::Named(Key::Return, "Return"),
        description: "Open the selection",
        fires: Fires::ReturnPressed,
    },
    Binding {
        owner: Owner::Contents,
        control: false,
        alt: false,
        shift: false,
        key: Physical::Named(Key::F2, "F2"),
        description: "Rename the selection",
        fires: Fires::ContentRenameRequested,
    },
    Binding {
        owner: Owner::Contents,
        control: false,
        alt: false,
        shift: false,
        key: Physical::Named(Key::Delete, "Del"),
        description: "Delete the selection",
        fires: Fires::DeleteRequested,
    },
    Binding {
        owner: Owner::Contents,
        control: false,
        alt: false,
        shift: false,
        key: Physical::Named(Key::Backspace, "Backspace"),
        description: "Erase the last typed character of a filter or a prompt",
        fires: Fires::BackspacePressed,
    },
    Binding {
        owner: Owner::Contents,
        control: true,
        alt: false,
        shift: true,
        key: Physical::Letter('N'),
        description: "New Folder",
        fires: Fires::NewFolderRequested,
    },
    Binding {
        owner: Owner::Contents,
        control: true,
        alt: false,
        shift: false,
        key: Physical::Letter('N'),
        description: "New File",
        fires: Fires::NewFileRequested,
    },
    Binding {
        owner: Owner::Contents,
        control: true,
        alt: false,
        shift: false,
        key: Physical::Letter('A'),
        description: "Select all rows",
        fires: Fires::SelectAllRequested,
    },
    Binding {
        owner: Owner::Contents,
        control: true,
        alt: false,
        shift: false,
        key: Physical::Letter('C'),
        description: "Copy",
        fires: Fires::ClipboardCopyRequested,
    },
    Binding {
        owner: Owner::Contents,
        control: true,
        alt: false,
        shift: false,
        key: Physical::Letter('X'),
        description: "Cut",
        fires: Fires::ClipboardCutRequested,
    },
    Binding {
        owner: Owner::Contents,
        control: true,
        alt: false,
        shift: false,
        key: Physical::Letter('V'),
        description: "Paste",
        fires: Fires::ClipboardPasteRequested,
    },
    Binding {
        owner: Owner::Contents,
        control: false,
        alt: false,
        shift: false,
        key: Physical::Letter('J'),
        description: "Type a name to jump to the first row starting with it",
        fires: Fires::KeyText("J"),
    },
    // Address bar.
    Binding {
        owner: Owner::AddressBar,
        control: false,
        alt: false,
        shift: false,
        key: Physical::Named(Key::F4, "F4"),
        description: "Edit the address bar",
        fires: Fires::PathEditRequested,
    },
    Binding {
        owner: Owner::AddressBar,
        control: true,
        alt: false,
        shift: false,
        key: Physical::Letter('L'),
        description: "Edit the address bar",
        fires: Fires::PathEditRequested,
    },
    // File pane and editor.
    Binding {
        owner: Owner::FilePaneAndEditor,
        control: true,
        alt: false,
        shift: false,
        key: Physical::Letter('E'),
        description: "Open the editor",
        fires: Fires::EditRequested,
    },
    // Find: the cross-repository search and the Contents pane's filter.
    Binding {
        owner: Owner::Find,
        control: true,
        alt: false,
        shift: true,
        key: Physical::Letter('F'),
        description: "Find across repositories",
        fires: Fires::FindRequested,
    },
    Binding {
        owner: Owner::Find,
        control: true,
        alt: false,
        shift: false,
        key: Physical::Letter('F'),
        description: "Focus the Contents pane's filter field",
        fires: Fires::FilterFocusRequested,
    },
    Binding {
        owner: Owner::Find,
        control: false,
        alt: false,
        shift: false,
        key: Physical::Named(Key::Escape, "Escape"),
        description: "Cancel a pending prompt, or clear the filter",
        fires: Fires::CancelRequested,
    },
    Binding {
        owner: Owner::Find,
        control: true,
        alt: false,
        shift: true,
        key: Physical::Letter('U'),
        description: "Toggle the status bar's changed-only filter",
        fires: Fires::ChangedFilterToggled,
    },
];

/// The sheet's rows for the current platform: `mac` selects the Cmd
/// labels GUIDANCE.md §2.3 asks for. A row's `group` names the heading to
/// draw above it, or is empty to continue the previous row's group -
/// [`Owner::heading`] compared to the previous entry's, computed once
/// here rather than by the markup walking the list itself.
#[must_use]
pub fn rows(mac: bool) -> Vec<crate::ShortcutRow> {
    let mut previous: Option<&'static str> = None;
    BINDINGS
        .iter()
        .map(|binding| {
            let heading = binding.owner.heading();
            let group = if previous == Some(heading) {
                String::new()
            } else {
                heading.to_owned()
            };
            previous = Some(heading);
            crate::ShortcutRow {
                group: group.into(),
                keys: binding.label(mac).into(),
                description: binding.description.into(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_binding_s_label_names_its_keys() {
        let ctrl_n = BINDINGS
            .iter()
            .find(|b| matches!(b.fires, Fires::NewFileRequested))
            .expect("New File is in the table");
        assert_eq!(ctrl_n.label(false), "Ctrl+N");
        assert_eq!(ctrl_n.label(true), "Cmd+N");
    }

    #[test]
    fn a_binding_with_every_modifier_orders_them_control_alt_shift() {
        let binding = Binding {
            owner: Owner::Window,
            control: true,
            alt: true,
            shift: true,
            key: Physical::Letter('Q'),
            description: "test fixture",
            fires: Fires::SaveRequested,
        };
        assert_eq!(binding.label(false), "Ctrl+Alt+Shift+Q");
    }

    #[test]
    fn rows_of_the_same_owner_are_contiguous() {
        let mut seen = std::collections::HashSet::new();
        let mut previous: Option<&'static str> = None;
        for binding in BINDINGS {
            let heading = binding.owner.heading();
            if previous != Some(heading) {
                assert!(
                    seen.insert(heading),
                    "{heading} appears in two separate places in the table"
                );
            }
            previous = Some(heading);
        }
    }
}
