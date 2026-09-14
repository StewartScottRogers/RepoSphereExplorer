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

/// Somewhere to cut to and paste from.
///
/// A trait rather than the platform call directly, because Slint 1.17.1
/// keeps the clipboard on its `Platform` trait where an application
/// cannot reach it, and because a test that used the real one would
/// fight whatever else on the machine is holding it.
pub trait Clipboard {
    /// What is on the clipboard, if anything.
    fn read(&mut self) -> Option<String>;
    /// Puts `text` on the clipboard.
    fn write(&mut self, text: &str);
}

/// Whether the hand-written surface can be trusted with `text`.
///
/// Two gaps, both recorded in GUIDANCE.md §3.6. The surface places its
/// caret by arithmetic on a fixed cell width, which assumes text runs
/// left to right and one character to a cell; and it reads keys through
/// a `FocusScope`, which receives no input method editor (IME)
/// composition, so the scripts that need one cannot be typed at all.
///
/// Rather than let either fail quietly, a file holding such text opens
/// in the plain editor instead and the pane says why. That is a smaller
/// answer than handling them, and it is a visible one.
#[must_use]
pub fn code_editor_suits(text: &str) -> bool {
    !text.chars().any(needs_more_than_arithmetic)
}

/// Whether `c` is from a script the surface cannot place or cannot
/// receive.
fn needs_more_than_arithmetic(c: char) -> bool {
    matches!(c,
        // Hebrew, Arabic, Syriac, Thaana and the Arabic presentation
        // forms: written right to left.
        '\u{0590}'..='\u{08ff}'
        | '\u{fb1d}'..='\u{fdff}'
        // Stopping at FEFC rather than FEFF. The last of that block is
        // U+FEFF, the byte-order mark, which is not Arabic and not
        // written in any direction. Including it sent every file saved
        // as UTF-8 with a mark to the plain editor on the strength of an
        // invisible character - which is what happened the first time
        // this was driven in the application.
        | '\u{fe70}'..='\u{fefc}'
        // Chinese, Japanese and Korean, which need an input method the
        // surface never sees - and whose characters are two cells wide
        // rather than one, which the arithmetic also assumes away.
        | '\u{1100}'..='\u{11ff}'
        | '\u{2e80}'..='\u{9fff}'
        | '\u{a960}'..='\u{a97f}'
        | '\u{ac00}'..='\u{d7ff}'
        | '\u{f900}'..='\u{faff}'
        | '\u{ff00}'..='\u{ff60}'
    )
}

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
    clipboard: &mut dyn Clipboard,
    text: &str,
    shift: bool,
    control: bool,
    rows: usize,
) -> bool {
    if control {
        return handle_control_key(document, clipboard, text, shift, rows);
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
fn handle_control_key(
    document: &mut Document,
    clipboard: &mut dyn Clipboard,
    text: &str,
    shift: bool,
    rows: usize,
) -> bool {
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
            "c" => copy(document, clipboard),
            "x" => {
                copy(document, clipboard);
                document.delete_back();
            }
            "v" => {
                if let Some(pasted) = clipboard.read() {
                    document.insert(&pasted);
                }
            }
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

/// Puts the selection on the clipboard, or leaves it alone when there
/// is none - copying nothing should not empty what is already there.
fn copy(document: &Document, clipboard: &mut dyn Clipboard) {
    if let Some(range) = document.selection()
        && let Some(selected) = document.text().get(range)
    {
        clipboard.write(selected);
    }
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
    use super::{Clipboard, code_editor_suits, handle_click, handle_key};
    use crate::document::Document;
    use slint::platform::Key;

    /// A clipboard of its own, so a test never touches the machine's.
    #[derive(Default)]
    struct Board(Option<String>);
    impl Clipboard for Board {
        fn read(&mut self) -> Option<String> {
            self.0.clone()
        }
        fn write(&mut self, text: &str) {
            self.0 = Some(text.to_owned());
        }
    }

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
        assert!(handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::RightArrow),
            false,
            false,
            10
        ));
        assert_eq!(it.caret(), 1);
        assert!(handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::DownArrow),
            false,
            false,
            10
        ));
        assert_eq!(it.line_of(it.caret()), 1);

        assert!(handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::RightArrow),
            true,
            false,
            10
        ));
        assert!(it.selection().is_some(), "shift extends");
    }

    #[test]
    fn home_and_end_and_their_control_forms_go_where_they_should() {
        let mut it = document();
        handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::DownArrow),
            false,
            false,
            10,
        );
        handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::End),
            false,
            false,
            10,
        );
        assert_eq!(it.column_of(it.caret()), 11, "the end of the second line");

        handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::Home),
            false,
            false,
            10,
        );
        assert_eq!(it.column_of(it.caret()), 0);

        handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::End),
            false,
            true,
            10,
        );
        assert_eq!(it.caret(), it.text().len(), "control-end is the document");
        handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::Home),
            false,
            true,
            10,
        );
        assert_eq!(it.caret(), 0);
    }

    #[test]
    fn a_page_moves_by_what_the_pane_says_it_is_showing() {
        let mut it = Document::new("a\nb\nc\nd\ne\nf\ng\nh");
        handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::PageDown),
            false,
            false,
            3,
        );
        assert_eq!(it.line_of(it.caret()), 3);
    }

    #[test]
    fn typing_inserts_and_a_tab_inserts_spaces() {
        let mut it = Document::new("");
        assert!(handle_key(
            &mut it,
            &mut Board::default(),
            "h",
            false,
            false,
            10
        ));
        assert!(handle_key(
            &mut it,
            &mut Board::default(),
            "i",
            false,
            false,
            10
        ));
        assert_eq!(it.text(), "hi");

        handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::Tab),
            false,
            false,
            10,
        );
        assert_eq!(
            it.text(),
            "hi    ",
            "a tab is spaces, as the file will hold"
        );
    }

    #[test]
    fn return_takes_the_indentation_with_it() {
        let mut it = Document::new("    indented");
        handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::End),
            false,
            false,
            10,
        );
        handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::Return),
            false,
            false,
            10,
        );
        handle_key(&mut it, &mut Board::default(), "x", false, false, 10);
        assert_eq!(it.text(), "    indented\n    x");
    }

    #[test]
    fn backspace_and_delete_remove_from_either_side() {
        let mut it = Document::new("abc");
        handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::RightArrow),
            false,
            false,
            10,
        );
        handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::Backspace),
            false,
            false,
            10,
        );
        assert_eq!(it.text(), "bc");
        handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::Delete),
            false,
            false,
            10,
        );
        assert_eq!(it.text(), "c");
    }

    #[test]
    fn control_takes_a_word_at_a_time() {
        let mut it = Document::new("alpha beta gamma");
        handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::End),
            false,
            true,
            10,
        );
        handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::LeftArrow),
            false,
            true,
            10,
        );
        assert_eq!(it.column_of(it.caret()), 11, "the start of the last word");

        // From the end, so what it takes is the last word rather than
        // the one the caret had already stepped back over.
        let mut fresh = Document::new("alpha beta gamma");
        handle_key(
            &mut fresh,
            &mut Board::default(),
            &key(Key::End),
            false,
            true,
            10,
        );
        handle_key(
            &mut fresh,
            &mut Board::default(),
            &key(Key::Backspace),
            false,
            true,
            10,
        );
        assert_eq!(fresh.text(), "alpha beta ");
    }

    #[test]
    fn select_all_undo_and_both_spellings_of_redo() {
        let mut it = Document::new("abc");
        assert!(handle_key(
            &mut it,
            &mut Board::default(),
            "a",
            false,
            true,
            10
        ));
        assert_eq!(it.selection(), Some(0..3), "control-a takes everything");

        handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::End),
            false,
            true,
            10,
        );
        handle_key(&mut it, &mut Board::default(), "x", false, false, 10);
        assert_eq!(it.text(), "abcx");

        handle_key(&mut it, &mut Board::default(), "z", false, true, 10);
        assert_eq!(it.text(), "abc", "control-z undoes");
        handle_key(&mut it, &mut Board::default(), "z", true, true, 10);
        assert_eq!(it.text(), "abcx", "control-shift-z redoes");
        handle_key(&mut it, &mut Board::default(), "z", false, true, 10);
        handle_key(&mut it, &mut Board::default(), "y", false, true, 10);
        assert_eq!(it.text(), "abcx", "and so does control-y");
    }

    #[test]
    fn a_key_the_editor_has_nothing_to_do_with_is_left_alone() {
        let mut it = document();
        let before = it.text().to_owned();
        assert!(
            !handle_key(
                &mut it,
                &mut Board::default(),
                &key(Key::Escape),
                false,
                false,
                10
            ),
            "Escape belongs to whatever else is listening"
        );
        assert!(!handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::F5),
            false,
            false,
            10
        ));
        assert!(
            !handle_key(&mut it, &mut Board::default(), "q", false, true, 10),
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
        handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::Escape),
            false,
            false,
            10,
        );
        handle_key(
            &mut it,
            &mut Board::default(),
            &key(Key::F1),
            false,
            false,
            10,
        );
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
    #[test]
    fn copy_cut_and_paste_go_through_the_clipboard() {
        let mut board = Board::default();
        let mut it = Document::new("alpha beta");
        it.select_word_at(0);

        handle_key(&mut it, &mut board, "c", false, true, 10);
        assert_eq!(board.0.as_deref(), Some("alpha"), "control-c copies");
        assert_eq!(it.text(), "alpha beta", "and changes nothing");

        handle_key(&mut it, &mut board, "x", false, true, 10);
        assert_eq!(it.text(), " beta", "control-x takes it out");
        assert_eq!(board.0.as_deref(), Some("alpha"));

        handle_key(&mut it, &mut board, "v", false, true, 10);
        assert_eq!(it.text(), "alpha beta", "and control-v puts it back");
    }

    #[test]
    fn copying_nothing_leaves_the_clipboard_as_it_was() {
        // Pressing control-c with no selection is a common miss, and
        // emptying the clipboard over it loses whatever was there.
        let mut board = Board(Some("kept".to_owned()));
        let mut it = Document::new("abc");
        handle_key(&mut it, &mut board, "c", false, true, 10);
        assert_eq!(board.0.as_deref(), Some("kept"));
    }

    #[test]
    fn a_multi_line_paste_arrives_whole_and_undoes_in_one_step() {
        let mut board = Board(Some("one\ntwo\nthree".to_owned()));
        let mut it = Document::new("");
        handle_key(&mut it, &mut board, "v", false, true, 10);
        assert_eq!(it.text(), "one\ntwo\nthree");
        handle_key(&mut it, &mut board, "z", false, true, 10);
        assert_eq!(it.text(), "", "one step, not three lines of them");
    }

    #[test]
    fn a_paste_replaces_what_is_selected() {
        let mut board = Board(Some("new".to_owned()));
        let mut it = Document::new("old text");
        it.select_word_at(0);
        handle_key(&mut it, &mut board, "v", false, true, 10);
        assert_eq!(it.text(), "new text");
    }

    #[test]
    fn ordinary_text_suits_the_surface_and_two_scripts_do_not() {
        assert!(code_editor_suits("fn main() { println!(\"hello\"); }"));
        assert!(code_editor_suits(
            "accented: caf\u{e9} na\u{ef}ve \u{fc}ber"
        ));
        assert!(
            code_editor_suits("an emoji \u{1f600} is wide but is not a script"),
            "an emoji needs no input method and is not written right to left"
        );

        assert!(
            !code_editor_suits("\u{5e2}\u{5d1}\u{5e8}\u{5d9}\u{5ea}"),
            "Hebrew runs right to left, which the caret arithmetic assumes away"
        );
        assert!(
            !code_editor_suits("\u{627}\u{644}\u{639}\u{631}\u{628}\u{64a}\u{629}"),
            "and so does Arabic"
        );
        assert!(
            !code_editor_suits("\u{65e5}\u{672c}\u{8a9e}"),
            "Japanese needs an input method the surface never sees"
        );
        assert!(
            !code_editor_suits("\u{d55c}\u{ad6d}\u{c5b4}"),
            "and so does Korean"
        );
    }

    #[test]
    fn a_byte_order_mark_is_not_a_script() {
        // Found by driving it: a file saved as UTF-8 with a mark went to
        // the plain editor, because U+FEFF sits at the end of the Arabic
        // presentation forms block and was being read as Arabic. It is
        // invisible, so the only symptom was colour disappearing for no
        // reason a reader could see.
        assert!(code_editor_suits("\u{feff}fn main() {}"));
    }

    #[test]
    fn one_character_of_a_script_is_enough_to_send_the_file_elsewhere() {
        // A comment in one language inside a file of another is the
        // common case, and it is exactly the case that would otherwise
        // put an unreachable caret in the middle of somebody's file.
        assert!(!code_editor_suits(
            "let name = \"\u{5f20}\u{4e09}\"; // a name"
        ));
    }
}
