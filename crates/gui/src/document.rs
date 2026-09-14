//! The text an editor holds, and everything a caret needs, as functions
//! over it.
//!
//! Nothing here draws. That is deliberate and is the same split the rest
//! of this crate already uses: `scroll_offset_for` and `chevron_hit` are
//! rules that live in Rust rather than in `app.slint`, because a rule in
//! that file cannot be exercised without an event loop, and every layout
//! rule this project has got wrong was one that lived there. An editor is
//! that problem many times over - a caret that goes to the wrong column
//! on the third line of a file is not something a person notices by
//! looking once.
//!
//! **Offsets are byte offsets** into [`Document::text`], because that is
//! what `plugin_api::Span` uses and the two have to agree for a coloured
//! line to be drawn with a caret in it. **Movement is by grapheme
//! cluster**, because an accented letter or an emoji is one thing to a
//! reader and two to five bytes to a computer, and a caret that stops in
//! the middle of one is a caret in a place that is not on the screen.
//!
//! Undo keeps whole snapshots rather than diffs. At the 64KB ceiling
//! GUIDANCE.md §3.6 puts on an editable file that is cheap, and it cannot
//! be subtly wrong the way a diff that has to be inverted can.

use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// What kind of change the last edit was, so that a run of the same kind
/// collapses into one undo step.
///
/// Typing a word and pressing undo should give back the line as it was,
/// not the word missing its last letter, eight times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    /// Characters inserted one at a time.
    Typing,
    /// Characters removed one at a time.
    Deleting,
    /// A paste, a newline, a word deleted whole: its own step either way.
    Whole,
}

/// The document as it was, to go back to.
#[derive(Debug, Clone)]
struct Snapshot {
    text: String,
    caret: usize,
    anchor: usize,
}

/// A file open in the editor.
#[derive(Debug, Clone)]
pub struct Document {
    text: String,
    /// Where the caret is, as a byte offset on a character boundary.
    caret: usize,
    /// The other end of the selection. Equal to `caret` when there is
    /// none, which is why there is no `Option` here: a selection of
    /// nothing and no selection are the same thing to every caller.
    anchor: usize,
    /// The column an up-or-down run started in, in graphemes.
    ///
    /// Without it, moving down through a short line and out the other
    /// side leaves the caret at the end of the short one - so a reader
    /// holding the down arrow watches their column melt away. Set by
    /// vertical movement and cleared by everything else.
    desired_column: Option<usize>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// What the last change was, for coalescing.
    last_change: Option<Change>,
}

impl Document {
    /// Opens `text` with the caret at the start and nothing selected.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            caret: 0,
            anchor: 0,
            desired_column: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last_change: None,
        }
    }

    /// The whole text, which is what a save writes.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Where the caret is, as a byte offset.
    #[must_use]
    pub const fn caret(&self) -> usize {
        self.caret
    }

    /// The selected range, or `None` when nothing is selected.
    #[must_use]
    pub fn selection(&self) -> Option<Range<usize>> {
        if self.caret == self.anchor {
            None
        } else {
            Some(self.caret.min(self.anchor)..self.caret.max(self.anchor))
        }
    }

    /// Whether there is a step to go back to.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether there is a step that was undone to put back.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    // -- reading the text ------------------------------------------------

    /// Which line `offset` is on, counting from zero.
    #[must_use]
    pub fn line_of(&self, offset: usize) -> usize {
        self.text[..offset.min(self.text.len())]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
    }

    /// How many lines there are. A document ending in a newline does not
    /// gain an empty one: the newline ends the last line rather than
    /// starting another, which is what the pane draws.
    #[must_use]
    pub fn line_count(&self) -> usize {
        if self.text.is_empty() {
            return 1;
        }
        self.text.bytes().filter(|byte| *byte == b'\n').count()
            + usize::from(!self.text.ends_with('\n'))
    }

    /// Where `line` starts, or the end of the text for a line past the
    /// last.
    #[must_use]
    pub fn line_start(&self, line: usize) -> usize {
        if line == 0 {
            return 0;
        }
        let mut seen = 0usize;
        for (offset, byte) in self.text.bytes().enumerate() {
            if byte == b'\n' {
                seen += 1;
                if seen == line {
                    return offset + 1;
                }
            }
        }
        self.text.len()
    }

    /// Where `line` ends, before its newline.
    #[must_use]
    pub fn line_end(&self, line: usize) -> usize {
        let start = self.line_start(line);
        self.text[start..]
            .find('\n')
            .map_or(self.text.len(), |at| start + at)
    }

    /// How many graphemes into its line `offset` is.
    #[must_use]
    pub fn column_of(&self, offset: usize) -> usize {
        let start = self.line_start(self.line_of(offset));
        self.text[start..offset.min(self.text.len())]
            .graphemes(true)
            .count()
    }

    /// The offset `column` graphemes into `line`, clamped to its end.
    #[must_use]
    pub fn offset_at(&self, line: usize, column: usize) -> usize {
        let start = self.line_start(line);
        let end = self.line_end(line);
        self.text[start..end]
            .grapheme_indices(true)
            .nth(column)
            .map_or(end, |(at, _)| start + at)
    }

    // -- moving ----------------------------------------------------------

    /// Puts the caret at `offset`, extending the selection or dropping it.
    fn place(&mut self, offset: usize, extend: bool) {
        self.caret = offset.min(self.text.len());
        if !extend {
            self.anchor = self.caret;
        }
    }

    /// One grapheme left. With a selection and no shift, this goes to the
    /// selection's start rather than one back from the caret - which is
    /// what every editor does, and what makes left-after-selecting feel
    /// like cancelling rather than deleting one more.
    pub fn move_left(&mut self, extend: bool) {
        self.desired_column = None;
        if !extend && let Some(range) = self.selection() {
            self.place(range.start, false);
            return;
        }
        let at = self.text[..self.caret]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(offset, _)| offset);
        self.place(at, extend);
    }

    /// One grapheme right, or to the end of a selection.
    pub fn move_right(&mut self, extend: bool) {
        self.desired_column = None;
        if !extend && let Some(range) = self.selection() {
            self.place(range.end, false);
            return;
        }
        let at = self.text[self.caret..]
            .graphemes(true)
            .next()
            .map_or(self.caret, |first| self.caret + first.len());
        self.place(at, extend);
    }

    /// Up or down `rows` lines, keeping the column the run started in.
    fn move_vertically(&mut self, rows: isize, extend: bool) {
        let column = self
            .desired_column
            .unwrap_or_else(|| self.column_of(self.caret));
        let line = self.line_of(self.caret);
        let last = self.line_count().saturating_sub(1);
        let target = line.saturating_add_signed(rows).min(last);
        self.place(self.offset_at(target, column), extend);
        self.desired_column = Some(column);
    }

    /// One line up.
    pub fn move_up(&mut self, extend: bool) {
        self.move_vertically(-1, extend);
    }

    /// One line down.
    pub fn move_down(&mut self, extend: bool) {
        self.move_vertically(1, extend);
    }

    /// A screenful up. The pane knows how many rows that is; this does
    /// not, and should not have to guess.
    ///
    /// A pane taller than `isize::MAX` rows is not a pane, so `rows` is
    /// narrowed rather than cast: an absurd number moves to the end of
    /// the document, which is where a page that large would land anyway.
    pub fn move_page_up(&mut self, rows: usize, extend: bool) {
        self.move_vertically(-Self::rows_as_steps(rows), extend);
    }

    /// A screenful down.
    pub fn move_page_down(&mut self, rows: usize, extend: bool) {
        self.move_vertically(Self::rows_as_steps(rows), extend);
    }

    /// `rows` as a signed step count, saturating rather than wrapping.
    fn rows_as_steps(rows: usize) -> isize {
        isize::try_from(rows).unwrap_or(isize::MAX)
    }

    /// To the start of the current line.
    pub fn move_line_start(&mut self, extend: bool) {
        self.desired_column = None;
        let at = self.line_start(self.line_of(self.caret));
        self.place(at, extend);
    }

    /// To the end of the current line, before its newline.
    pub fn move_line_end(&mut self, extend: bool) {
        self.desired_column = None;
        let at = self.line_end(self.line_of(self.caret));
        self.place(at, extend);
    }

    /// To the very start of the document.
    pub fn move_document_start(&mut self, extend: bool) {
        self.desired_column = None;
        self.place(0, extend);
    }

    /// To the very end.
    pub fn move_document_end(&mut self, extend: bool) {
        self.desired_column = None;
        self.place(self.text.len(), extend);
    }

    /// Where each word starts, in order, and the end of the text.
    ///
    /// Runs of whitespace are not words. Without that, a word at a time
    /// stops twice between every pair of them - once on the space and
    /// once on the word - and holding the key moves at half the rate a
    /// reader expects.
    fn word_bounds(&self) -> Vec<usize> {
        let mut bounds: Vec<usize> = self
            .text
            .split_word_bound_indices()
            .filter(|(_, word)| !word.chars().all(char::is_whitespace))
            .map(|(at, _)| at)
            .collect();
        bounds.push(self.text.len());
        bounds
    }

    /// To the start of the word before the caret.
    pub fn move_word_left(&mut self, extend: bool) {
        self.desired_column = None;
        let at = self
            .word_bounds()
            .into_iter()
            .rfind(|bound| *bound < self.caret)
            .unwrap_or(0);
        self.place(at, extend);
    }

    /// To the start of the word after the caret.
    pub fn move_word_right(&mut self, extend: bool) {
        self.desired_column = None;
        let at = self
            .word_bounds()
            .into_iter()
            .find(|bound| *bound > self.caret)
            .unwrap_or(self.text.len());
        self.place(at, extend);
    }

    /// Puts the caret at `offset`, dropping any selection. What a click
    /// does.
    pub fn place_caret(&mut self, offset: usize) {
        self.desired_column = None;
        self.place(offset, false);
    }

    /// Extends the selection to `offset`, keeping the anchor. What a
    /// shift-click and a drag do.
    pub fn select_to(&mut self, offset: usize) {
        self.desired_column = None;
        self.place(offset, true);
    }

    // -- selecting -------------------------------------------------------

    /// Selects everything.
    pub fn select_all(&mut self) {
        self.desired_column = None;
        self.anchor = 0;
        self.caret = self.text.len();
    }

    /// Selects the word `offset` falls in, for a double click.
    pub fn select_word_at(&mut self, offset: usize) {
        self.desired_column = None;
        let offset = offset.min(self.text.len());
        for (at, word) in self.text.split_word_bound_indices() {
            if offset >= at && offset < at + word.len() {
                self.anchor = at;
                self.caret = at + word.len();
                return;
            }
        }
        self.anchor = offset;
        self.caret = offset;
    }

    /// Selects the whole line `offset` falls on, for a triple click.
    ///
    /// Including its newline, so that deleting the selection removes the
    /// line rather than leaving a blank one behind.
    pub fn select_line_at(&mut self, offset: usize) {
        self.desired_column = None;
        let line = self.line_of(offset.min(self.text.len()));
        self.anchor = self.line_start(line);
        let end = self.line_end(line);
        self.caret = if end < self.text.len() { end + 1 } else { end };
    }

    // -- changing --------------------------------------------------------

    /// Remembers the document as it is, unless this change continues the
    /// last one.
    fn remember(&mut self, change: Change) {
        let continues = change != Change::Whole && self.last_change == Some(change);
        if !continues {
            self.undo.push(Snapshot {
                text: self.text.clone(),
                caret: self.caret,
                anchor: self.anchor,
            });
        }
        self.last_change = Some(change);
        // Anything new makes the redo stack a history that did not happen.
        self.redo.clear();
    }

    /// Replaces the selection, or inserts at the caret when there is none.
    fn replace_selection(&mut self, with: &str) {
        let range = self.selection().unwrap_or(self.caret..self.caret);
        self.text.replace_range(range.clone(), with);
        self.caret = range.start + with.len();
        self.anchor = self.caret;
    }

    /// Types `text` at the caret, replacing any selection.
    ///
    /// One character is typing and coalesces; anything longer is a paste
    /// and is its own undo step.
    pub fn insert(&mut self, text: &str) {
        self.desired_column = None;
        let single = text.chars().count() == 1 && text != "\n";
        self.remember(if single {
            Change::Typing
        } else {
            Change::Whole
        });
        self.replace_selection(text);
    }

    /// Enter: a newline, and the indentation of the line it was pressed
    /// on, so that a reader does not have to retype it every line.
    pub fn insert_newline(&mut self) {
        self.desired_column = None;
        self.remember(Change::Whole);
        let line = self.line_of(self.caret);
        let start = self.line_start(line);
        let indent: String = self.text[start..self.caret]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        self.replace_selection(&format!("\n{indent}"));
    }

    /// Backspace: the selection, or one grapheme behind the caret.
    pub fn delete_back(&mut self) {
        self.desired_column = None;
        if self.selection().is_some() {
            self.remember(Change::Whole);
            self.replace_selection("");
            return;
        }
        if self.caret == 0 {
            return;
        }
        self.remember(Change::Deleting);
        let from = self.text[..self.caret]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(offset, _)| offset);
        self.text.replace_range(from..self.caret, "");
        self.caret = from;
        self.anchor = from;
    }

    /// Delete: the selection, or one grapheme ahead of the caret.
    pub fn delete_forward(&mut self) {
        self.desired_column = None;
        if self.selection().is_some() {
            self.remember(Change::Whole);
            self.replace_selection("");
            return;
        }
        let Some(width) = self.text[self.caret..].graphemes(true).next().map(str::len) else {
            return;
        };
        self.remember(Change::Deleting);
        let to = self.caret + width;
        self.text.replace_range(self.caret..to, "");
    }

    /// Control-Backspace: back to the start of the word behind the caret,
    /// as one step whatever it removes.
    pub fn delete_word_back(&mut self) {
        self.desired_column = None;
        if self.selection().is_some() {
            self.delete_back();
            return;
        }
        if self.caret == 0 {
            return;
        }
        self.remember(Change::Whole);
        let from = self
            .word_bounds()
            .into_iter()
            .rfind(|bound| *bound < self.caret)
            .unwrap_or(0);
        self.text.replace_range(from..self.caret, "");
        self.caret = from;
        self.anchor = from;
    }

    // -- going back ------------------------------------------------------

    /// Undoes the last step, returning the text to exactly what it was.
    pub fn undo(&mut self) {
        let Some(previous) = self.undo.pop() else {
            return;
        };
        self.redo.push(Snapshot {
            text: std::mem::replace(&mut self.text, previous.text),
            caret: self.caret,
            anchor: self.anchor,
        });
        self.caret = previous.caret.min(self.text.len());
        self.anchor = previous.anchor.min(self.text.len());
        self.desired_column = None;
        // The next keystroke starts a new step rather than joining the one
        // that was just undone.
        self.last_change = None;
    }

    /// Redoes the last undone step.
    pub fn redo(&mut self) {
        let Some(next) = self.redo.pop() else {
            return;
        };
        self.undo.push(Snapshot {
            text: std::mem::replace(&mut self.text, next.text),
            caret: self.caret,
            anchor: self.anchor,
        });
        self.caret = next.caret.min(self.text.len());
        self.anchor = next.anchor.min(self.text.len());
        self.desired_column = None;
        self.last_change = None;
    }
}

#[cfg(test)]
mod tests {
    use super::Document;

    /// A document with the caret placed by a marker, which reads better
    /// in a test than a byte offset nobody can count.
    fn at(text: &str) -> Document {
        let caret = text.find('|').expect("the fixture marks the caret");
        let mut document = Document::new(text.replace('|', ""));
        document.move_document_start(false);
        for _ in 0..document.text()[..caret].graphemes_for_test() {
            document.move_right(false);
        }
        document
    }

    trait GraphemeCount {
        fn graphemes_for_test(&self) -> usize;
    }
    impl GraphemeCount for str {
        fn graphemes_for_test(&self) -> usize {
            unicode_segmentation::UnicodeSegmentation::graphemes(self, true).count()
        }
    }

    /// The text with `|` where the caret is, for comparing in one line.
    fn shown(document: &Document) -> String {
        let mut text = document.text().to_owned();
        text.insert(document.caret(), '|');
        text
    }

    #[test]
    fn left_and_right_move_one_visible_character_at_a_time() {
        // Four graphemes, eleven bytes: a letter, an accented letter
        // written as two code points, an emoji, and a tab.
        let mut document = Document::new("ae\u{301}\u{1f600}\tz");
        let mut seen = vec![document.caret()];
        for _ in 0..5 {
            document.move_right(false);
            seen.push(document.caret());
        }
        assert_eq!(
            seen,
            vec![0, 1, 4, 8, 9, 10],
            "each step is one grapheme: the accent goes with its letter \
             and the emoji moves as one"
        );

        for _ in 0..5 {
            document.move_left(false);
        }
        assert_eq!(document.caret(), 0, "and back again, the same way");
    }

    #[test]
    fn moving_past_either_end_stays_there() {
        let mut document = Document::new("ab");
        document.move_left(false);
        assert_eq!(document.caret(), 0);
        document.move_document_end(false);
        document.move_right(false);
        assert_eq!(document.caret(), 2);
    }

    #[test]
    fn an_empty_document_has_one_line_and_nowhere_to_go() {
        let mut document = Document::new("");
        assert_eq!(document.line_count(), 1);
        document.move_right(false);
        document.move_down(false);
        document.move_word_right(false);
        assert_eq!(document.caret(), 0);
        assert_eq!(document.text(), "");
    }

    #[test]
    fn a_trailing_newline_does_not_add_a_line_that_is_not_there() {
        assert_eq!(Document::new("a\nb\n").line_count(), 2);
        assert_eq!(Document::new("a\nb").line_count(), 2);
        assert_eq!(Document::new("\n").line_count(), 1);
    }

    #[test]
    fn a_line_that_is_only_a_newline_can_be_moved_through() {
        let mut document = at("a|\n\nb");
        document.move_down(false);
        assert_eq!(shown(&document), "a\n|\nb", "the empty line has one place");
        document.move_down(false);
        assert_eq!(
            shown(&document),
            "a\n\nb|",
            "and out the other side to the column it started in, which is \
             one in, not the start of the line it was clamped to"
        );
    }

    #[test]
    fn going_down_through_a_short_line_comes_back_to_the_column() {
        // The one that makes holding the down arrow feel right.
        let mut document = at("longest|_line\nab\nlongest_line");
        document.move_down(false);
        assert_eq!(
            shown(&document),
            "longest_line\nab|\nlongest_line",
            "clamped"
        );
        document.move_down(false);
        assert_eq!(
            shown(&document),
            "longest_line\nab\nlongest|_line",
            "and back to the column it started in, not the column it was \
             clamped to"
        );
    }

    #[test]
    fn any_other_movement_forgets_the_column() {
        let mut document = at("longest|_line\nab\nlongest_line");
        document.move_down(false);
        document.move_left(false);
        document.move_down(false);
        assert_eq!(
            shown(&document),
            "longest_line\nab\nl|ongest_line",
            "after moving sideways, down keeps the column it moved to - \
             one in - rather than the seven the run started from"
        );
    }

    #[test]
    fn home_and_end_reach_the_ends_of_the_line_and_no_further() {
        let mut document = at("one\ntw|o\nthree");
        document.move_line_start(false);
        assert_eq!(shown(&document), "one\n|two\nthree");
        document.move_line_end(false);
        assert_eq!(shown(&document), "one\ntwo|\nthree", "before the newline");
    }

    #[test]
    fn a_word_at_a_time_stops_where_words_start() {
        let mut document = Document::new("alpha beta gamma");
        document.move_document_end(false);
        document.move_word_left(false);
        assert_eq!(shown(&document), "alpha beta |gamma");
        document.move_word_left(false);
        assert_eq!(shown(&document), "alpha |beta gamma");
    }

    #[test]
    fn shift_extends_and_a_plain_move_drops_it() {
        let mut document = Document::new("abcdef");
        document.move_right(true);
        document.move_right(true);
        assert_eq!(document.selection(), Some(0..2));
        document.move_right(false);
        assert_eq!(document.selection(), None, "a plain move drops it");
        assert_eq!(
            document.caret(),
            2,
            "and lands at the end of what was selected, not one past it"
        );
    }

    #[test]
    fn a_double_click_takes_the_word_and_a_triple_click_the_line() {
        let mut document = Document::new("alpha beta\ngamma");
        document.select_word_at(7);
        assert_eq!(document.selection(), Some(6..10));

        document.select_line_at(7);
        assert_eq!(
            document.selection(),
            Some(0..11),
            "the line takes its newline, so deleting it removes the line \
             rather than leaving a blank one"
        );
    }

    #[test]
    fn select_all_takes_everything_including_the_last_newline() {
        let mut document = Document::new("a\nb\n");
        document.select_all();
        assert_eq!(document.selection(), Some(0..4));
    }

    #[test]
    fn typing_replaces_what_is_selected() {
        let mut document = Document::new("hello world");
        document.select_word_at(0);
        document.insert("goodbye");
        assert_eq!(document.text(), "goodbye world");
        assert_eq!(shown(&document), "goodbye| world");
    }

    #[test]
    fn a_paste_arrives_whole_and_keeps_the_caret_after_it() {
        let mut document = at("a|b");
        document.insert("one\ntwo");
        assert_eq!(shown(&document), "aone\ntwo|b");
    }

    #[test]
    fn a_new_line_keeps_the_indentation_of_the_one_it_was_typed_on() {
        let mut document = at("    indented|");
        document.insert_newline();
        assert_eq!(shown(&document), "    indented\n    |");
    }

    #[test]
    fn backspace_and_delete_take_one_visible_character() {
        let mut document = at("ae\u{301}|z");
        document.delete_back();
        assert_eq!(document.text(), "az", "the accent goes with its letter");
        document.delete_forward();
        assert_eq!(document.text(), "a");
        document.delete_forward();
        assert_eq!(document.text(), "a", "at the end there is nothing to take");
    }

    #[test]
    fn backspace_at_the_very_start_does_nothing_at_all() {
        let mut document = Document::new("abc");
        document.delete_back();
        assert_eq!(document.text(), "abc");
        document.undo();
        assert_eq!(document.text(), "abc", "and left nothing to undo");
    }

    #[test]
    fn deleting_a_word_takes_it_in_one_step() {
        let mut document = Document::new("alpha beta");
        document.move_document_end(false);
        document.delete_word_back();
        assert_eq!(document.text(), "alpha ");
        document.undo();
        assert_eq!(document.text(), "alpha beta", "one step, not four");
    }

    #[test]
    fn a_word_typed_is_one_undo_step() {
        let mut document = Document::new("");
        for letter in ["h", "e", "l", "l", "o"] {
            document.insert(letter);
        }
        assert_eq!(document.text(), "hello");
        document.undo();
        assert_eq!(document.text(), "", "one step, not five");
    }

    #[test]
    fn a_deletion_run_is_one_step_and_typing_after_it_is_another() {
        let mut document = Document::new("hello");
        document.move_document_end(false);
        document.delete_back();
        document.delete_back();
        document.insert("p");
        assert_eq!(document.text(), "help");

        document.undo();
        assert_eq!(document.text(), "hel", "the typing undoes on its own");
        document.undo();
        assert_eq!(document.text(), "hello", "and then the whole deletion run");
    }

    #[test]
    fn undo_returns_the_bytes_exactly_after_a_mixed_run() {
        let original = "one\n  two\nthree";
        let mut document = Document::new(original);
        document.move_document_end(false);
        document.insert("x");
        document.insert_newline();
        document.insert("four");
        document.move_document_start(false);
        document.select_word_at(0);
        document.insert("ONE");
        document.delete_word_back();
        document.move_document_end(false);
        document.delete_back();
        document.delete_back();

        while document.text() != original {
            let before = document.text().to_owned();
            document.undo();
            assert_ne!(
                document.text(),
                before,
                "undo ran out before reaching the original: {:?}",
                document.text()
            );
        }
        assert_eq!(document.text(), original);
    }

    #[test]
    fn redo_puts_back_what_undo_took_and_a_new_change_forgets_it() {
        let mut document = Document::new("a");
        document.move_document_end(false);
        document.insert("b");
        document.undo();
        assert_eq!(document.text(), "a");
        document.redo();
        assert_eq!(document.text(), "ab");

        document.undo();
        document.insert("c");
        document.redo();
        assert_eq!(
            document.text(),
            "ac",
            "redo after a new change would be putting back a history that \
             did not happen"
        );
    }

    #[test]
    fn a_page_is_as_many_rows_as_the_pane_says() {
        let text = (0..10)
            .map(|n| format!("line{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut document = Document::new(text);
        document.move_page_down(4, false);
        assert_eq!(document.line_of(document.caret()), 4);
        document.move_page_down(100, false);
        assert_eq!(
            document.line_of(document.caret()),
            9,
            "a page past the end stops at the last line"
        );
        document.move_page_up(100, false);
        assert_eq!(document.line_of(document.caret()), 0);
    }

    #[test]
    fn a_document_with_no_trailing_newline_still_ends_somewhere() {
        let mut document = Document::new("last line");
        document.move_document_end(false);
        assert_eq!(document.caret(), 9);
        document.move_down(false);
        assert_eq!(document.caret(), 9, "there is no line below to go to");
        document.insert("!");
        assert_eq!(document.text(), "last line!");
    }
}
