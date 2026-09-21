//! The rule that turns a test's own name into a directory name (#741).
//!
//! `binding_table.rs` names each scratch directory after the binding it is
//! exercising, and those names read like prose: "Delete", "Sort by name",
//! "? for keys". A question mark, a colon, a slash or a quotation mark is an
//! ordinary character in a POSIX file name and is forbidden in a Windows one,
//! so the suite passed on the Linux runner and, on a Windows desktop, panicked
//! creating the directory with `InvalidFilename` before reaching a single
//! assertion.
//!
//! The rule is tested here rather than only through the suites that use it:
//! exercised in passing, it only ever meets the one forbidden character that
//! happens to be in a binding's description today, and would stop proving
//! anything the moment that description changed.

// Every integration file is its own binary and compiles the whole of
// `common` into it. This one needs a single function from it, so the rest -
// the service harness, the key queue - is dead code here and nowhere else.
#[allow(dead_code)]
mod common;

use common::usable_as_a_name;

/// Every character Windows reserves in a file name, one at a time, so a new
/// one cannot be added to the rule without a test that says what it does.
#[test]
fn every_character_windows_forbids_is_replaced() {
    for forbidden in ['<', '>', ':', '"', '/', '\\', '|', '?', '*'] {
        let name = format!("before{forbidden}after");
        let cleaned = usable_as_a_name(&name);
        assert!(
            !cleaned.contains(forbidden),
            "{forbidden:?} must not survive into a file name: {cleaned:?}"
        );
        assert_eq!(
            cleaned, "before-after",
            "and it is replaced rather than dropped, so what surrounds it stays apart"
        );
    }
}

/// Control characters are forbidden too, and are easy to introduce by
/// accident from a description that spans lines.
#[test]
fn control_characters_are_replaced() {
    assert_eq!(usable_as_a_name("one\ttwo\nthree"), "one-two-three");
    assert_eq!(usable_as_a_name("bell\u{7}rings"), "bell-rings");
}

/// A run collapses to one separator, so a name stays readable.
#[test]
fn a_run_of_forbidden_characters_collapses() {
    assert_eq!(usable_as_a_name("a<>:\"|?*b"), "a-b");
}

/// But two names that differ only in punctuation still get directories of
/// their own - otherwise two cases would share one scratch directory and
/// each would delete the other's fixture.
#[test]
fn names_differing_only_in_punctuation_stay_different() {
    let one = usable_as_a_name("Find: by name");
    let two = usable_as_a_name("Find by name");
    assert_ne!(
        one, two,
        "two cases must not collide on one scratch directory"
    );
}

/// A trailing dot or space is legal to create on Windows and impossible to
/// open again, so it is trimmed.
#[test]
fn a_trailing_dot_or_space_is_trimmed() {
    assert_eq!(usable_as_a_name("ends with a space "), "ends with a space");
    assert_eq!(usable_as_a_name("ends with a dot."), "ends with a dot");
    assert_eq!(usable_as_a_name("both. "), "both");
}

/// A name that was always usable is left exactly as it was, so the
/// directories every other suite already makes do not move.
#[test]
fn a_name_that_needs_nothing_is_untouched() {
    for name in ["opening", "folders-relist", "Sort by name", "quit-on-q"] {
        assert_eq!(usable_as_a_name(name), name);
    }
}

/// The description that actually broke the suite.
#[test]
fn the_binding_description_that_broke_the_suite_is_usable() {
    let cleaned = usable_as_a_name("? for keys");
    assert_eq!(cleaned, "- for keys");
    assert!(!cleaned.contains('?'));
}
