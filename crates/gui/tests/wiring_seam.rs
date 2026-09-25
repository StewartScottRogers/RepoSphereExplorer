//! The window a test builds and the window `main` builds are wired by the
//! same code - the seam, not the halves.
//!
//! `contents_in_the_window.rs`, `folders_in_the_window.rs` and
//! `editor_keyboard.rs` each build a real `MainWindow` against a real
//! `App`, the way a use-case test has to (rule 14). Every one of them
//! used to carry its own hand copy of `main`'s private wiring, which
//! meant a defect in the wiring a reader actually runs was invisible to
//! every test in the crate - the fault the editor's "does not insert at
//! the caret" report turned out to be. This is a source-level guard
//! against that coming back: `main` has to call the library's
//! `wire_callbacks` rather than define its own, and none of the three
//! suites may hold a copy of it.

/// `main.rs`, as shipped.
const MAIN: &str = include_str!("../src/main.rs");

/// The three suites whose window this file was written to prove is wired
/// the same way `main`'s is.
const SUITES: &[(&str, &str)] = &[
    (
        "contents_in_the_window.rs",
        include_str!("contents_in_the_window.rs"),
    ),
    (
        "folders_in_the_window.rs",
        include_str!("folders_in_the_window.rs"),
    ),
    ("editor_keyboard.rs", include_str!("editor_keyboard.rs")),
];

#[test]
fn main_wires_the_window_through_the_librarys_own_function() {
    assert!(
        MAIN.contains("gui::wire_callbacks("),
        "main.rs no longer calls gui::wire_callbacks - the window it \
         ships would be wired by something a test cannot also call"
    );
}

#[test]
fn main_does_not_define_its_own_copy_of_the_wiring() {
    for name in [
        "fn wire_callbacks",
        "fn wire_rows",
        "fn wire_commands",
        "fn wire_content_operations",
    ] {
        assert!(
            !MAIN.contains(name),
            "main.rs defines `{name}`; the wiring belongs in the library \
             (gui::lib.rs), where a test can call the same function"
        );
    }
}

#[test]
fn no_window_suite_holds_a_hand_copy_of_the_wiring() {
    for (path, source) in SUITES {
        for name in [
            "fn wire_rows",
            "fn wire_commands",
            "fn wire_content_operations",
            "fn wire_window(",
        ] {
            assert!(
                !source.contains(name),
                "{path} defines `{name}` - a copy of the library's wiring \
                 rather than a call to gui::wire_callbacks"
            );
        }
    }
}
