//! The right-hand pane's tool slot (#616): a `Tool` applies to a selection
//! or does not, the registry lists the compiled-in tools in a fixed order,
//! and the editor - today's File pane, unchanged - is the first one.

use crate::app::Selection;

/// A tool the slot can show for the current selection: what decides whether
/// it applies, and what identifies and labels it once it does.
///
/// How a tool's own view is drawn and updated is not part of this trait -
/// with the editor the only one compiled in, the slot's markup shows its
/// content directly, the way it always has; a second tool with a view of
/// its own is #622's to add.
pub trait Tool {
    /// A stable name, used to tell tools apart. Not shown to the reader.
    fn id(&self) -> &'static str;

    /// The pane frame's title while this is the active tool, and the
    /// picker's label for it.
    fn title(&self) -> &'static str;

    /// Whether this tool has anything to offer for `selection`.
    fn applies_to(&self, selection: &Selection) -> bool;
}

/// The editor: today's File pane. It applies to any file and to any
/// folder - only an empty selection leaves it with nothing to show - and
/// its title stays "File", so a window with only this tool registered
/// looks exactly as it always did.
pub struct EditorTool;

impl Tool for EditorTool {
    fn id(&self) -> &'static str {
        "editor"
    }

    fn title(&self) -> &'static str {
        "File"
    }

    fn applies_to(&self, selection: &Selection) -> bool {
        selection.target.is_some()
    }
}

/// The compiled-in tools, in the fixed order the picker lists them and the
/// order [`Self::default_index`] searches for the first one that applies.
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self {
            tools: vec![Box::new(EditorTool)],
        }
    }
}

impl ToolRegistry {
    /// Every registered tool, in order.
    #[must_use]
    pub fn tools(&self) -> &[Box<dyn Tool>] {
        &self.tools
    }

    /// Adds `tool` after the ones already registered. A real window never
    /// calls this beyond [`Self::default`]'s own compiled-in list; see
    /// [`crate::app::App::register_tool_for_test`].
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    /// The index of every tool that applies to `selection`, in order.
    #[must_use]
    pub fn applicable(&self, selection: &Selection) -> Vec<usize> {
        self.tools
            .iter()
            .enumerate()
            .filter(|(_, tool)| tool.applies_to(selection))
            .map(|(index, _)| index)
            .collect()
    }

    /// The first tool that applies to `selection`, or the first tool of
    /// all when none does - a selection has to show something.
    #[must_use]
    pub fn default_index(&self, selection: &Selection) -> usize {
        self.applicable(selection).first().copied().unwrap_or(0)
    }

    /// `current` if it still applies to `selection`, otherwise
    /// [`Self::default_index`] - a tool that stops applying hands back to
    /// the default.
    #[must_use]
    pub fn active_index(&self, current: usize, selection: &Selection) -> usize {
        if self
            .tools
            .get(current)
            .is_some_and(|tool| tool.applies_to(selection))
        {
            current
        } else {
            self.default_index(selection)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EditorTool, Tool, ToolRegistry};
    use crate::app::{Pane, Selection, Target};

    fn selection(target: Option<Target>) -> Selection {
        Selection {
            folder: 0,
            content: 0,
            contents: std::collections::BTreeSet::new(),
            anchor: 0,
            file_view_index: 0,
            editing: false,
            focus: Pane::Contents,
            target,
        }
    }

    fn file(name: &str) -> Target {
        Target::File {
            name: name.to_owned(),
        }
    }

    /// A tool that applies only to a `.txt` file - the shape #616's own
    /// acceptance checks ask a window test to register.
    struct TxtTool;

    impl Tool for TxtTool {
        fn id(&self) -> &'static str {
            "txt"
        }

        fn title(&self) -> &'static str {
            "Txt"
        }

        fn applies_to(&self, selection: &Selection) -> bool {
            matches!(
                &selection.target,
                Some(Target::File { name })
                    if std::path::Path::new(name).extension().is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
            )
        }
    }

    #[test]
    fn the_editor_is_the_default_for_a_file_and_for_a_folder() {
        let registry = ToolRegistry::default();
        assert_eq!(registry.default_index(&selection(Some(file("a.rs")))), 0);
        assert_eq!(registry.default_index(&selection(Some(Target::Folder))), 0);
    }

    #[test]
    fn nothing_selected_still_names_a_default() {
        let registry = ToolRegistry::default();
        assert_eq!(registry.default_index(&selection(None)), 0);
    }

    #[test]
    fn the_editor_does_not_apply_once_nothing_is_selected() {
        assert!(!EditorTool.applies_to(&selection(None)));
        assert!(EditorTool.applies_to(&selection(Some(file("a.rs")))));
        assert!(EditorTool.applies_to(&selection(Some(Target::Folder))));
    }

    #[test]
    fn a_tool_that_stops_applying_hands_back_to_the_default() {
        let mut registry = ToolRegistry::default();
        registry.register(Box::new(TxtTool));

        // Chosen while a `.txt` file is selected: it applies, and stays
        // active.
        assert_eq!(
            registry.active_index(1, &selection(Some(file("notes.txt")))),
            1
        );

        // The selection moves to a file `TxtTool` has nothing to say
        // about: the slot falls back to the editor rather than keeping a
        // tool that no longer applies.
        assert_eq!(
            registry.active_index(1, &selection(Some(file("main.rs")))),
            0
        );
        assert_eq!(
            registry.active_index(1, &selection(Some(Target::Folder))),
            0
        );
    }
}
