//! What a keystroke means to a [`Document`], for the terminal File pane
//! (#645, D16).
//!
//! Mirrors `crates/gui/src/editor.rs`'s split between deciding and
//! drawing, against `ratatui::crossterm::event::{KeyCode, KeyModifiers}`
//! rather than Slint's `Key`, since a terminal reports a key as a code and
//! a modifier set rather than as text. No clipboard: the issue this module
//! answers (#645) scopes the terminal editor to movement, selection,
//! insertion, deletion and undo/redo, and cut, copy and paste are not
//! among them.
//!
//! GUIDANCE.md §3.6 excuses the terminal editor from
//! `editor::code_editor_suits`'s plainer-editor fallback: the host
//! terminal, not this application, already carries input method editor
//! (IME) composition, screen-reader accessibility and right-to-left text,
//! which is exactly what that fallback exists to work around in the
//! graphical front end's own hand-drawn surface.

use crate::document::Document;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

/// Applies `code` (held with `modifiers`) to `document`, and says whether
/// it meant anything.
///
/// `rows` is how many lines the pane is showing, which is what a page up
/// or down moves by. The pane knows it; this does not, and guessing would
/// make a page a different distance on every window size.
///
/// Returns `false` for a key the editor has nothing to do with, so the
/// caller can leave it to whatever else is listening rather than
/// swallowing every keystroke in the window.
pub fn handle_key(
    document: &mut Document,
    code: KeyCode,
    modifiers: KeyModifiers,
    rows: usize,
) -> bool {
    let shift = modifiers.contains(KeyModifiers::SHIFT);
    let control = modifiers.contains(KeyModifiers::CONTROL);

    if control {
        return handle_control_key(document, code, shift);
    }

    match code {
        KeyCode::Left => document.move_left(shift),
        KeyCode::Right => document.move_right(shift),
        KeyCode::Up => document.move_up(shift),
        KeyCode::Down => document.move_down(shift),
        KeyCode::Home => document.move_line_start(shift),
        KeyCode::End => document.move_line_end(shift),
        KeyCode::PageUp => document.move_page_up(rows, shift),
        KeyCode::PageDown => document.move_page_down(rows, shift),
        KeyCode::Backspace => document.delete_back(),
        KeyCode::Delete => document.delete_forward(),
        KeyCode::Enter => document.insert_newline(),
        KeyCode::Tab => document.insert("    "),
        KeyCode::Char(c) => document.insert(&c.to_string()),
        _ => return false,
    }
    true
}

/// The control-held half, kept apart so the plain half above reads as a
/// list of keys rather than a list of conditions.
fn handle_control_key(document: &mut Document, code: KeyCode, shift: bool) -> bool {
    match code {
        KeyCode::Left => document.move_word_left(shift),
        KeyCode::Right => document.move_word_right(shift),
        KeyCode::Home => document.move_document_start(shift),
        KeyCode::End => document.move_document_end(shift),
        KeyCode::Backspace => document.delete_word_back(),
        // Both spellings of redo, because both are in use and a reader
        // should not have to find out which this one took.
        KeyCode::Char('z' | 'Z') if shift => document.redo(),
        KeyCode::Char('z' | 'Z') => document.undo(),
        KeyCode::Char('y' | 'Y') => document.redo(),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::handle_key;
    use crate::document::Document;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    fn document() -> Document {
        Document::new("alpha beta\nsecond line\nthird")
    }

    #[test]
    fn the_arrows_move_and_shift_selects() {
        let mut it = document();
        assert!(handle_key(&mut it, KeyCode::Right, KeyModifiers::NONE, 10));
        assert_eq!(it.caret(), 1);
        assert!(handle_key(&mut it, KeyCode::Down, KeyModifiers::NONE, 10));
        assert_eq!(it.line_of(it.caret()), 1);

        assert!(handle_key(&mut it, KeyCode::Right, KeyModifiers::SHIFT, 10));
        assert!(it.selection().is_some(), "shift extends");
    }

    #[test]
    fn home_and_end_and_their_control_forms_go_where_they_should() {
        let mut it = document();
        handle_key(&mut it, KeyCode::Down, KeyModifiers::NONE, 10);
        handle_key(&mut it, KeyCode::End, KeyModifiers::NONE, 10);
        assert_eq!(it.column_of(it.caret()), 11, "the end of the second line");

        handle_key(&mut it, KeyCode::Home, KeyModifiers::NONE, 10);
        assert_eq!(it.column_of(it.caret()), 0);

        handle_key(&mut it, KeyCode::End, KeyModifiers::CONTROL, 10);
        assert_eq!(it.caret(), it.text().len(), "control-end is the document");
        handle_key(&mut it, KeyCode::Home, KeyModifiers::CONTROL, 10);
        assert_eq!(it.caret(), 0);
    }

    #[test]
    fn a_page_moves_by_what_the_pane_says_it_is_showing() {
        let mut it = Document::new("a\nb\nc\nd\ne\nf\ng\nh");
        handle_key(&mut it, KeyCode::PageDown, KeyModifiers::NONE, 3);
        assert_eq!(it.line_of(it.caret()), 3);
    }

    #[test]
    fn typing_inserts_and_a_tab_inserts_spaces() {
        let mut it = Document::new("");
        assert!(handle_key(
            &mut it,
            KeyCode::Char('h'),
            KeyModifiers::NONE,
            10
        ));
        assert!(handle_key(
            &mut it,
            KeyCode::Char('i'),
            KeyModifiers::NONE,
            10
        ));
        assert_eq!(it.text(), "hi");

        handle_key(&mut it, KeyCode::Tab, KeyModifiers::NONE, 10);
        assert_eq!(
            it.text(),
            "hi    ",
            "a tab is spaces, as the file will hold"
        );
    }

    #[test]
    fn a_multibyte_character_types_and_moves_as_one() {
        let mut it = Document::new("");
        handle_key(&mut it, KeyCode::Char('\u{1f600}'), KeyModifiers::NONE, 10);
        assert_eq!(it.text(), "\u{1f600}");
        assert_eq!(
            it.caret(),
            it.text().len(),
            "the caret lands after it, on a boundary"
        );
        handle_key(&mut it, KeyCode::Left, KeyModifiers::NONE, 10);
        assert_eq!(it.caret(), 0, "and one grapheme left is the whole thing");
    }

    #[test]
    fn enter_takes_the_indentation_with_it() {
        let mut it = Document::new("    indented");
        handle_key(&mut it, KeyCode::End, KeyModifiers::NONE, 10);
        handle_key(&mut it, KeyCode::Enter, KeyModifiers::NONE, 10);
        handle_key(&mut it, KeyCode::Char('x'), KeyModifiers::NONE, 10);
        assert_eq!(it.text(), "    indented\n    x");
    }

    #[test]
    fn backspace_and_delete_remove_from_either_side() {
        let mut it = Document::new("abc");
        handle_key(&mut it, KeyCode::Right, KeyModifiers::NONE, 10);
        handle_key(&mut it, KeyCode::Backspace, KeyModifiers::NONE, 10);
        assert_eq!(it.text(), "bc");
        handle_key(&mut it, KeyCode::Delete, KeyModifiers::NONE, 10);
        assert_eq!(it.text(), "c");
    }

    #[test]
    fn control_takes_a_word_at_a_time() {
        let mut it = Document::new("alpha beta gamma");
        handle_key(&mut it, KeyCode::End, KeyModifiers::CONTROL, 10);
        handle_key(&mut it, KeyCode::Left, KeyModifiers::CONTROL, 10);
        assert_eq!(it.column_of(it.caret()), 11, "the start of the last word");

        let mut fresh = Document::new("alpha beta gamma");
        handle_key(&mut fresh, KeyCode::End, KeyModifiers::CONTROL, 10);
        handle_key(&mut fresh, KeyCode::Backspace, KeyModifiers::CONTROL, 10);
        assert_eq!(fresh.text(), "alpha beta ");
    }

    #[test]
    fn undo_and_both_spellings_of_redo() {
        let mut it = Document::new("abc");
        handle_key(&mut it, KeyCode::End, KeyModifiers::CONTROL, 10);
        handle_key(&mut it, KeyCode::Char('x'), KeyModifiers::NONE, 10);
        assert_eq!(it.text(), "abcx");

        handle_key(&mut it, KeyCode::Char('z'), KeyModifiers::CONTROL, 10);
        assert_eq!(it.text(), "abc", "control-z undoes");
        handle_key(
            &mut it,
            KeyCode::Char('z'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            10,
        );
        assert_eq!(it.text(), "abcx", "control-shift-z redoes");
        handle_key(&mut it, KeyCode::Char('z'), KeyModifiers::CONTROL, 10);
        handle_key(&mut it, KeyCode::Char('y'), KeyModifiers::CONTROL, 10);
        assert_eq!(it.text(), "abcx", "and so does control-y");
    }

    #[test]
    fn a_key_the_editor_has_nothing_to_do_with_is_left_alone() {
        let mut it = document();
        let before = it.text().to_owned();
        assert!(
            !handle_key(&mut it, KeyCode::Esc, KeyModifiers::NONE, 10),
            "Escape belongs to whatever else is listening"
        );
        assert!(!handle_key(&mut it, KeyCode::F(5), KeyModifiers::NONE, 10));
        assert!(
            !handle_key(&mut it, KeyCode::Char('q'), KeyModifiers::CONTROL, 10),
            "control-q is not ours"
        );
        assert_eq!(it.text(), before, "and none of them changed the file");
    }
}
