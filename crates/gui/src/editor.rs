//! What a keystroke and a click mean to a [`Document`].
//!
//! The `CodeEditor` component in `app.slint` draws and reports; this
//! decides. Nothing here draws and nothing there judges, which is the
//! split `scroll_offset_for` and `chevron_hit` already use: a rule
//! written in that file cannot be exercised without an event loop, and
//! every layout rule this project has got wrong was one that lived
//! there.
//!
//! A key arrives as the text Slint gives it, which for an arrow or a
//! function key is a private-use character rather than a letter.
//! [`slint::platform::Key`] names the same characters on this side, so
//! the two agree without either spelling the code point out.

use crate::document::Document;
use slint::platform::Key;

/// Whether `text` is the key `wanted`.
fn is(text: &str, wanted: Key) -> bool {
    let mut characters = text.chars();
    characters.next() == Some(char::from(wanted)) && characters.next().is_none()
}

/// Applies `text` to `document`, and says whether it meant anything.
///
/// `rows` is how many lines the pane is showing, which is what a page up
/// or down moves by. The pane knows it; this does not, and guessing
/// would make a page a different distance on every window size.
///
/// Returns `false` for a key the editor has nothing to do with, so the
/// caller can leave it to whatever else is listening rather than
/// swallowing every keystroke in the window.
pub fn handle_key(
    document: &mut Document,
    text: &str,
    shift: bool,
    control: bool,
    rows: usize,
) -> bool {
    if control {
        return handle_control_key(document, text, shift, rows);
    }

    if is(text, Key::LeftArrow) {
        document.move_left(shift);
    } else if is(text, Key::RightArrow) {
        document.move_right(shift);
    } else if is(text, Key::UpArrow) {
        document.move_up(shift);
    } else if is(text, Key::DownArrow) {
        document.move_down(shift);
    } else if is(text, Key::Home) {
        document.move_line_start(shift);
    } else if is(text, Key::End) {
        document.move_line_end(shift);
    } else if is(text, Key::PageUp) {
        document.move_page_up(rows, shift);
    } else if is(text, Key::PageDown) {
        document.move_page_down(rows, shift);
    } else if is(text, Key::Backspace) {
        document.delete_back();
    } else if is(text, Key::Delete) {
        document.delete_forward();
    } else if is(text, Key::Return) {
        document.insert_newline();
    } else if is(text, Key::Tab) {
        document.insert("    ");
    } else if is_typed(text) {
        document.insert(text);
    } else {
        return false;
    }
    true
}

/// The control-held half, kept apart so the plain half above reads as a
/// list of keys rather than a list of conditions.
fn handle_control_key(document: &mut Document, text: &str, shift: bool, rows: usize) -> bool {
    let _ = rows;
    if is(text, Key::LeftArrow) {
        document.move_word_left(shift);
    } else if is(text, Key::RightArrow) {
        document.move_word_right(shift);
    } else if is(text, Key::Home) {
        document.move_document_start(shift);
    } else if is(text, Key::End) {
        document.move_document_end(shift);
    } else if is(text, Key::Backspace) {
        document.delete_word_back();
    } else {
        match text.to_ascii_lowercase().as_str() {
            "a" => document.select_all(),
            // Both spellings of redo, because both are in use and a
            // reader should not have to find out which this one took.
            "z" if shift => document.redo(),
            "z" => document.undo(),
            "y" => document.redo(),
            _ => return false,
        }
    }
    true
}

/// Whether `text` is something a reader typed rather than a key that
/// happens to carry a character.
///
/// Slint hands every key through as text, so Escape and the function
/// keys arrive looking like one-character strings. They are in Unicode's
/// private use area, which nothing anybody types ever is.
fn is_typed(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| !c.is_control() && !('\u{e000}'..='\u{f8ff}').contains(&c))
}

/// Where a click at `line` and `column` puts the caret.
///
/// Separate from [`handle_key`] only because a click carries a position
/// and a key does not.
pub fn handle_click(document: &mut Document, line: usize, column: usize, extend: bool) {
    let offset = document.offset_at(line.min(document.line_count() - 1), column);
    if extend {
        document.select_to(offset);
    } else {
        document.place_caret(offset);
    }
}

#[cfg(test)]
mod tests {
    use super::{handle_click, handle_key};
    use crate::document::Document;
    use slint::platform::Key;

    /// A key as Slint delivers it.
    fn key(which: Key) -> String {
        char::from(which).to_string()
    }

    fn document() -> Document {
        Document::new("alpha beta\nsecond line\nthird")
    }

    #[test]
    fn the_arrows_move_and_shift_selects() {
        let mut it = document();
        assert!(handle_key(&mut it, &key(Key::RightArrow), false, false, 10));
        assert_eq!(it.caret(), 1);
        assert!(handle_key(&mut it, &key(Key::DownArrow), false, false, 10));
        assert_eq!(it.line_of(it.caret()), 1);

        assert!(handle_key(&mut it, &key(Key::RightArrow), true, false, 10));
        assert!(it.selection().is_some(), "shift extends");
    }

    #[test]
    fn home_and_end_and_their_control_forms_go_where_they_should() {
        let mut it = document();
        handle_key(&mut it, &key(Key::DownArrow), false, false, 10);
        handle_key(&mut it, &key(Key::End), false, false, 10);
        assert_eq!(it.column_of(it.caret()), 11, "the end of the second line");

        handle_key(&mut it, &key(Key::Home), false, false, 10);
        assert_eq!(it.column_of(it.caret()), 0);

        handle_key(&mut it, &key(Key::End), false, true, 10);
        assert_eq!(it.caret(), it.text().len(), "control-end is the document");
        handle_key(&mut it, &key(Key::Home), false, true, 10);
        assert_eq!(it.caret(), 0);
    }

    #[test]
    fn a_page_moves_by_what_the_pane_says_it_is_showing() {
        let mut it = Document::new("a\nb\nc\nd\ne\nf\ng\nh");
        handle_key(&mut it, &key(Key::PageDown), false, false, 3);
        assert_eq!(it.line_of(it.caret()), 3);
    }

    #[test]
    fn typing_inserts_and_a_tab_inserts_spaces() {
        let mut it = Document::new("");
        assert!(handle_key(&mut it, "h", false, false, 10));
        assert!(handle_key(&mut it, "i", false, false, 10));
        assert_eq!(it.text(), "hi");

        handle_key(&mut it, &key(Key::Tab), false, false, 10);
        assert_eq!(
            it.text(),
            "hi    ",
            "a tab is spaces, as the file will hold"
        );
    }

    #[test]
    fn return_takes_the_indentation_with_it() {
        let mut it = Document::new("    indented");
        handle_key(&mut it, &key(Key::End), false, false, 10);
        handle_key(&mut it, &key(Key::Return), false, false, 10);
        handle_key(&mut it, "x", false, false, 10);
        assert_eq!(it.text(), "    indented\n    x");
    }

    #[test]
    fn backspace_and_delete_remove_from_either_side() {
        let mut it = Document::new("abc");
        handle_key(&mut it, &key(Key::RightArrow), false, false, 10);
        handle_key(&mut it, &key(Key::Backspace), false, false, 10);
        assert_eq!(it.text(), "bc");
        handle_key(&mut it, &key(Key::Delete), false, false, 10);
        assert_eq!(it.text(), "c");
    }

    #[test]
    fn control_takes_a_word_at_a_time() {
        let mut it = Document::new("alpha beta gamma");
        handle_key(&mut it, &key(Key::End), false, true, 10);
        handle_key(&mut it, &key(Key::LeftArrow), false, true, 10);
        assert_eq!(it.column_of(it.caret()), 11, "the start of the last word");

        // From the end, so what it takes is the last word rather than
        // the one the caret had already stepped back over.
        let mut fresh = Document::new("alpha beta gamma");
        handle_key(&mut fresh, &key(Key::End), false, true, 10);
        handle_key(&mut fresh, &key(Key::Backspace), false, true, 10);
        assert_eq!(fresh.text(), "alpha beta ");
    }

    #[test]
    fn select_all_undo_and_both_spellings_of_redo() {
        let mut it = Document::new("abc");
        assert!(handle_key(&mut it, "a", false, true, 10));
        assert_eq!(it.selection(), Some(0..3), "control-a takes everything");

        handle_key(&mut it, &key(Key::End), false, true, 10);
        handle_key(&mut it, "x", false, false, 10);
        assert_eq!(it.text(), "abcx");

        handle_key(&mut it, "z", false, true, 10);
        assert_eq!(it.text(), "abc", "control-z undoes");
        handle_key(&mut it, "z", true, true, 10);
        assert_eq!(it.text(), "abcx", "control-shift-z redoes");
        handle_key(&mut it, "z", false, true, 10);
        handle_key(&mut it, "y", false, true, 10);
        assert_eq!(it.text(), "abcx", "and so does control-y");
    }

    #[test]
    fn a_key_the_editor_has_nothing_to_do_with_is_left_alone() {
        let mut it = document();
        let before = it.text().to_owned();
        assert!(
            !handle_key(&mut it, &key(Key::Escape), false, false, 10),
            "Escape belongs to whatever else is listening"
        );
        assert!(!handle_key(&mut it, &key(Key::F5), false, false, 10));
        assert!(
            !handle_key(&mut it, "q", false, true, 10),
            "control-q is not ours"
        );
        assert_eq!(it.text(), before, "and none of them changed the file");
    }

    #[test]
    fn a_function_key_is_not_typed_into_the_file() {
        // Slint hands every key through as text, so Escape arrives as a
        // one-character string. Inserting it would put a character in the
        // file that the reader cannot see and cannot delete by eye.
        let mut it = Document::new("");
        handle_key(&mut it, &key(Key::Escape), false, false, 10);
        handle_key(&mut it, &key(Key::F1), false, false, 10);
        assert_eq!(it.text(), "");
    }

    #[test]
    fn a_click_puts_the_caret_where_it_landed_and_shift_selects_to_it() {
        let mut it = document();
        handle_click(&mut it, 1, 3, false);
        assert_eq!(it.line_of(it.caret()), 1);
        assert_eq!(it.column_of(it.caret()), 3);
        assert!(it.selection().is_none());

        handle_click(&mut it, 2, 2, true);
        assert_eq!(
            it.selection().map(|range| range.len()),
            Some(11),
            "shift-clicking selects from where the caret was - three into \
             the second line - to where the pointer landed, two into the \
             third"
        );
    }

    #[test]
    fn a_click_past_the_last_line_lands_on_the_last_line() {
        let mut it = document();
        handle_click(&mut it, 99, 0, false);
        assert_eq!(it.line_of(it.caret()), 2);
    }
}
