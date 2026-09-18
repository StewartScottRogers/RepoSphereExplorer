//! No pane's selection lives in the markup alone (#614).
//!
//! What is selected has one owner: `App`, read back as one
//! `gui::app::Selection`. The markup only ever receives it - a property
//! `app.slint` assigned to itself, rather than binding from Rust, would be
//! a second copy that could silently drift from `App::selection()` and
//! nothing here or in `app.rs` would notice. This is the audit the .slint
//! side of that claim gets: a grep, not a build, so it stays true even
//! though Slint would happily compile the alternative.

/// The window's own markup, checked as text rather than through Slint's
/// compiler - which reports what a property evaluates to, not who is
/// allowed to have written it.
const APP_SLINT: &str = include_str!("../ui/app.slint");

/// The window-level properties that say what the reader is looking at:
/// which folder, which row, which of a file's views, and whether the File
/// pane is editing. Every one of these is a field `App::selection`
/// (crates/gui/src/app.rs) reads off `App`'s own state.
const SELECTION_PROPERTIES: &[&str] = &[
    "folder-selected",
    "content-selected",
    "file-view-index",
    "editing-file",
    "focus-pane",
];

/// Whether `line` assigns to `name`, as opposed to reading it: a binding
/// passed down to a child element (`selected: folder-selected;`), an
/// event handler's arrow (`changed folder-selected => {`), or a comparison
/// (`focus-pane == 0`) are all reads.
fn assigns_to(line: &str, name: &str) -> bool {
    let mut rest = line;
    while let Some(at) = rest.find(name) {
        let after = rest[at + name.len()..].trim_start();
        if let Some(past_equals) = after.strip_prefix('=')
            && !past_equals.starts_with('=')
            && !past_equals.starts_with('>')
        {
            return true;
        }
        rest = &rest[at + name.len()..];
    }
    false
}

#[test]
fn selection_properties_are_declared_in_not_in_out() {
    for name in SELECTION_PROPERTIES {
        let declaration = APP_SLINT
            .lines()
            .find(|line| line.trim_end().ends_with(&format!("> {name};")))
            .unwrap_or_else(|| panic!("`{name}` is not declared in app.slint"));
        assert!(
            declaration.trim_start().starts_with("in property"),
            "`{name}` must be an `in property` - one-way from Rust to the \
             markup - so the markup itself cannot hold a different value; \
             found `{}`",
            declaration.trim()
        );
    }
}

#[test]
fn selection_properties_are_never_assigned_from_the_markup() {
    for name in SELECTION_PROPERTIES {
        for line in APP_SLINT.lines() {
            let trimmed = line.trim();
            if trimmed.ends_with(&format!("> {name};")) {
                continue; // The declaration itself.
            }
            assert!(
                !assigns_to(line, name),
                "`{name}` is assigned imperatively in app.slint: `{trimmed}` \
                 - only `App`'s intent methods may change what a pane shows \
                 (#614)"
            );
        }
    }
}
