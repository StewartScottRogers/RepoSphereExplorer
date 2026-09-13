//! One error-tolerant tokeniser, driven by a small description of a
//! language.
//!
//! A hundred and eighty plugins already parse their formats. None of them
//! should have to grow a highlighter as well, so each describes its
//! language once - comment markers, quotes, keywords - and this does the
//! work. GUIDANCE.md §3.6.
//!
//! **Tolerant, always.** The text handed here is a file somebody is part
//! way through editing: an unterminated string, an unclosed comment, a
//! stray brace. A classifier that gave up on those would go blank the
//! moment a reader typed a quote, which is exactly when they are looking
//! at it. So there is no error path. Every input yields spans covering it,
//! and an unterminated construct simply runs to the end of the text, which
//! is also what the reader wants to see: everything after the quote turns
//! into string, and the missing quote becomes obvious.
//!
//! This is the lexical layer of the Roslyn C# compiler platform's
//! classification - what a token looks like, without knowing what it
//! means. The layer above, colouring a name by what it refers to, is a
//! separate and later job.

use plugin_api::{Class, Span};

/// How one kind of quoted run opens, closes and escapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quote {
    /// The character that opens the run.
    pub open: char,
    /// The character that closes it, which is usually `open` again.
    pub close: char,
    /// The character that makes the next one literal, if the language has
    /// one. A raw string such as Python's `r'...'` has none.
    pub escape: Option<char>,
    /// Whether the run may cross a line. Most may not, and saying so is
    /// what stops one unclosed quote colouring the rest of the file.
    pub multiline: bool,
}

impl Quote {
    /// The common case: same character both ends, backslash escapes, one
    /// line only.
    #[must_use]
    pub const fn simple(delimiter: char) -> Self {
        Self {
            open: delimiter,
            close: delimiter,
            escape: Some('\\'),
            multiline: false,
        }
    }
}

/// Everything the tokeniser needs to know about one language.
///
/// Small on purpose. A description that grew a field per language would
/// be a parser with extra steps, and the plugin that wants a parser
/// already has one - it can override `classify` itself.
#[derive(Debug, Clone, Copy)]
pub struct Language {
    /// Markers that comment out the rest of the line: `//`, `#`, `--`.
    pub line_comment: &'static [&'static str],
    /// Pairs that open and close a comment spanning lines.
    pub block_comment: &'static [(&'static str, &'static str)],
    /// The kinds of quoted run the language has.
    pub quotes: &'static [Quote],
    /// Words the language reserves. Matched whole, and case-sensitively
    /// unless [`Self::ignore_case`] says otherwise.
    pub keywords: &'static [&'static str],
    /// Words naming a type, drawn apart from other keywords because
    /// telling them apart is most of what makes code skimmable.
    pub types: &'static [&'static str],
    /// Whether a name directly before an opening bracket is a call, and
    /// should be drawn as one. False for a data format, where `key (` is
    /// not a call and colouring it as one would mislead.
    pub calls: bool,
    /// Whether keywords match regardless of case, as in `SQL` and some
    /// configuration formats.
    pub ignore_case: bool,
}

impl Language {
    /// A language with nothing filled in, to be built on by the plugins.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            line_comment: &[],
            block_comment: &[],
            quotes: &[],
            keywords: &[],
            types: &[],
            calls: false,
            ignore_case: false,
        }
    }
}

impl Default for Language {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether `c` may start a name.
fn starts_name(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

/// Whether `c` may continue a name. Hyphens and dots are in because the
/// configuration and markup formats treat them as part of one word, and
/// splitting `font-size` into three runs helps nobody.
fn continues_name(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
}

/// A tokeniser walking `text` and emitting spans that cover it exactly.
struct Scan<'a> {
    text: &'a str,
    language: &'a Language,
    /// Where the run of unclassified text that has not been emitted yet
    /// begins. Emitting anything else flushes it first, which is what
    /// keeps the spans gapless without tracking gaps.
    plain_from: usize,
    spans: Vec<Span>,
}

impl<'a> Scan<'a> {
    fn new(text: &'a str, language: &'a Language) -> Self {
        Self {
            text,
            language,
            plain_from: 0,
            spans: Vec::new(),
        }
    }

    /// Emits everything from [`Self::plain_from`] up to `at` as plain.
    fn flush_plain(&mut self, at: usize) {
        if at > self.plain_from {
            self.spans.push(Span::new(
                self.plain_from,
                at - self.plain_from,
                Class::Plain,
            ));
        }
        self.plain_from = at;
    }

    fn emit(&mut self, start: usize, end: usize, class: Class) {
        self.flush_plain(start);
        if end > start {
            self.spans.push(Span::new(start, end - start, class));
        }
        self.plain_from = end;
    }

    /// The rest of the text from `at`.
    fn rest(&self, at: usize) -> &'a str {
        &self.text[at..]
    }

    /// The end of a line comment opened at `at`, which is the end of the
    /// line or of the text.
    fn line_comment_end(&self, at: usize) -> usize {
        self.rest(at)
            .find('\n')
            .map_or(self.text.len(), |offset| at + offset)
    }

    /// The end of a block comment opened at `at` with `open`.
    ///
    /// An unclosed one runs to the end of the text rather than being an
    /// error: that is both the tolerant answer and the useful one, since
    /// the whole tail turning into comment is how a reader sees they left
    /// one open.
    fn block_comment_end(&self, at: usize, open: &str, close: &str) -> usize {
        let after = at + open.len();
        self.rest(after)
            .find(close)
            .map_or(self.text.len(), |offset| after + offset + close.len())
    }

    /// The end of a quoted run opened at `at`.
    ///
    /// Stops at the closing delimiter, at the end of the line for a quote
    /// that may not cross one, or at the end of the text.
    fn quote_end(&self, at: usize, quote: Quote) -> usize {
        let mut chars = self.rest(at).char_indices();
        chars.next(); // the opening delimiter
        let mut escaped = false;
        for (offset, c) in chars {
            if escaped {
                escaped = false;
                continue;
            }
            if Some(c) == quote.escape {
                escaped = true;
                continue;
            }
            if c == quote.close {
                return at + offset + c.len_utf8();
            }
            if c == '\n' && !quote.multiline {
                return at + offset;
            }
        }
        self.text.len()
    }

    /// The end of a number starting at `at`.
    ///
    /// Deliberately loose: a radix prefix, digits, separators, a decimal
    /// point, an exponent and a suffix all read as one run. Getting the
    /// exact grammar of every language's numeric literal right would be a
    /// parser, and drawing `0xFF_u8` as one number is what a reader wants
    /// either way.
    fn number_end(&self, at: usize) -> usize {
        let mut end = at;
        let mut previous = '0';
        for (offset, c) in self.rest(at).char_indices() {
            let exponent_sign = (c == '+' || c == '-') && matches!(previous, 'e' | 'E');
            if c.is_alphanumeric() || c == '.' || c == '_' || exponent_sign {
                end = at + offset + c.len_utf8();
                previous = c;
            } else {
                break;
            }
        }
        end
    }

    /// Whether `word` is in `list`, respecting the language's case rule.
    fn listed(&self, word: &str, list: &[&str]) -> bool {
        if self.language.ignore_case {
            list.iter().any(|entry| entry.eq_ignore_ascii_case(word))
        } else {
            list.contains(&word)
        }
    }

    /// Whether the first thing after `at`, ignoring spaces, is an opening
    /// bracket - which is what makes the name before it a call.
    fn call_follows(&self, at: usize) -> bool {
        self.rest(at)
            .chars()
            .find(|c| !matches!(c, ' ' | '\t'))
            .is_some_and(|c| c == '(')
    }

    fn run(mut self) -> Vec<Span> {
        let mut at = 0usize;
        while at < self.text.len() {
            let rest = self.rest(at);
            let Some(c) = rest.chars().next() else {
                break;
            };

            if let Some(marker) = self
                .language
                .line_comment
                .iter()
                .find(|marker| rest.starts_with(**marker))
            {
                let _ = marker;
                let end = self.line_comment_end(at);
                self.emit(at, end, Class::Comment);
                at = end;
                continue;
            }

            if let Some((open, close)) = self
                .language
                .block_comment
                .iter()
                .find(|(open, _)| rest.starts_with(*open))
                .copied()
            {
                let end = self.block_comment_end(at, open, close);
                self.emit(at, end, Class::Comment);
                at = end;
                continue;
            }

            if let Some(quote) = self
                .language
                .quotes
                .iter()
                .find(|quote| c == quote.open)
                .copied()
            {
                let end = self.quote_end(at, quote);
                self.emit(at, end, Class::Text);
                at = end;
                continue;
            }

            if c.is_ascii_digit() {
                let end = self.number_end(at);
                self.emit(at, end, Class::Number);
                at = end;
                continue;
            }

            if starts_name(c) {
                let end = at
                    + rest
                        .char_indices()
                        .find(|(_, c)| !continues_name(*c))
                        .map_or(rest.len(), |(offset, _)| offset);
                let word = &self.text[at..end];
                let class = if self.listed(word, self.language.types) {
                    Class::Type
                } else if self.listed(word, self.language.keywords) {
                    Class::Keyword
                } else if self.language.calls && self.call_follows(end) {
                    Class::Function
                } else {
                    Class::Plain
                };
                if class != Class::Plain {
                    self.emit(at, end, class);
                }
                at = end;
                continue;
            }

            if !c.is_whitespace() && !c.is_alphanumeric() {
                let end = at + c.len_utf8();
                self.emit(at, end, Class::Punctuation);
                at = end;
                continue;
            }

            at += c.len_utf8();
        }

        self.flush_plain(self.text.len());
        self.spans
    }
}

/// What each run of `text` is, read as `language`.
///
/// The spans come back in order and cover `text` exactly once, with no
/// gap and no overlap, so a front end can draw the text by walking them
/// and nothing else. [`spans_cover`] is the check, and it runs over every
/// fixture in the repository.
#[must_use]
pub fn classify(text: &str, language: &Language) -> Vec<Span> {
    Scan::new(text, language).run()
}

/// Whether `spans` cover `text` exactly once, in order.
///
/// Public because it is the contract, not an implementation detail: a
/// plugin that classifies its own format some other way has to hold to
/// the same rule, and this is how its tests say so.
#[must_use]
pub fn spans_cover(text: &str, spans: &[Span]) -> bool {
    let mut at = 0usize;
    for span in spans {
        if span.start != at || !text.is_char_boundary(span.start) {
            return false;
        }
        at = span.start + span.len;
        if at > text.len() || !text.is_char_boundary(at) {
            return false;
        }
    }
    at == text.len()
}

#[cfg(test)]
mod tests {
    use super::{Language, Quote, classify, spans_cover};
    use plugin_api::Class;

    /// A C-shaped language, which is most of them.
    const C_LIKE: Language = Language {
        line_comment: &["//"],
        block_comment: &[("/*", "*/")],
        quotes: &[Quote::simple('"'), Quote::simple('\'')],
        keywords: &["if", "else", "return", "fn", "let"],
        types: &["int", "String"],
        calls: true,
        ignore_case: false,
    };

    /// The classes `text` is made of, in order, with the text of each.
    fn runs(text: &str, language: &Language) -> Vec<(Class, String)> {
        classify(text, language)
            .into_iter()
            .map(|span| {
                (
                    span.class,
                    text[span.start..span.start + span.len].to_owned(),
                )
            })
            .collect()
    }

    fn classes_of(text: &str, language: &Language) -> Vec<Class> {
        classify(text, language)
            .into_iter()
            .map(|span| span.class)
            .collect()
    }

    #[test]
    fn a_keyword_a_type_and_a_call_are_told_apart() {
        let seen = runs("let x: int = size(1);", &C_LIKE);
        assert!(seen.contains(&(Class::Keyword, "let".to_owned())));
        assert!(seen.contains(&(Class::Type, "int".to_owned())));
        assert!(seen.contains(&(Class::Function, "size".to_owned())));
        assert!(seen.contains(&(Class::Number, "1".to_owned())));
    }

    #[test]
    fn a_word_that_is_not_reserved_and_not_called_stays_plain() {
        let seen = runs("total = other", &C_LIKE);
        assert!(
            seen.iter()
                .all(|(class, _)| *class != Class::Keyword && *class != Class::Function),
            "an ordinary name is not a keyword and not a call: {seen:?}"
        );
    }

    #[test]
    fn a_keyword_inside_a_longer_word_is_not_a_keyword() {
        // `iffy` starts with `if`, and colouring the first two letters of
        // it would be worse than colouring none.
        let seen = runs("iffy", &C_LIKE);
        assert_eq!(seen, vec![(Class::Plain, "iffy".to_owned())]);
    }

    #[test]
    fn a_comment_runs_to_the_end_of_its_line_and_no_further() {
        let seen = runs("let // note\nlet", &C_LIKE);
        assert!(seen.contains(&(Class::Comment, "// note".to_owned())));
        assert_eq!(
            seen.iter()
                .filter(|(class, _)| *class == Class::Keyword)
                .count(),
            2,
            "the keyword on the next line is not part of the comment"
        );
    }

    #[test]
    fn a_string_swallows_its_escaped_delimiter() {
        let seen = runs(r#"let s = "a\"b";"#, &C_LIKE);
        assert!(
            seen.contains(&(Class::Text, r#""a\"b""#.to_owned())),
            "the escaped quote is inside the string, not the end of it: {seen:?}"
        );
    }

    #[test]
    fn an_unterminated_string_stops_at_the_line_and_does_not_eat_the_file() {
        // The tolerant case, and the common one: a reader has just typed
        // the opening quote. Everything after it on that line reads as
        // string, and the next line goes back to normal.
        let seen = runs("let s = \"open\nlet t = 1;", &C_LIKE);
        assert!(seen.contains(&(Class::Text, "\"open".to_owned())));
        assert!(
            seen.contains(&(Class::Number, "1".to_owned())),
            "the line after an unterminated string is still read normally: {seen:?}"
        );
    }

    #[test]
    fn an_unclosed_block_comment_runs_to_the_end_of_the_text() {
        // Deliberate: the tail turning into comment is how a reader sees
        // they left one open.
        let seen = runs("let\n/* open\nstill open", &C_LIKE);
        assert!(
            seen.iter()
                .any(|(class, text)| *class == Class::Comment && text.ends_with("still open")),
            "{seen:?}"
        );
    }

    #[test]
    fn the_spans_cover_the_text_exactly_once() {
        for text in [
            "",
            "\n",
            "   ",
            "let x: int = size(1); // done\n/* and */ \"a\"",
            "\"unterminated",
            "/* unclosed",
            "iffy",
            "0xFF_u8 1.5e-3 42",
            "// only a comment",
        ] {
            let spans = classify(text, &C_LIKE);
            assert!(
                spans_cover(text, &spans),
                "spans must cover {text:?} exactly once: {spans:?}"
            );
        }
    }

    #[test]
    fn text_that_is_one_enormous_token_is_still_one_span() {
        let huge = "a".repeat(200_000);
        let spans = classify(&huge, &C_LIKE);
        assert!(spans_cover(&huge, &spans));
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn an_empty_language_colours_only_what_needs_no_description() {
        // A bracket is a bracket and a digit is a digit in every language,
        // so those stay. Everything a description would have named -
        // keywords, types, strings, comments - does not, which is what a
        // plugin that has said nothing about its syntax should get.
        let plain = Language::new();
        let text = "if (x) { return 1; } // nothing is described\n";
        assert!(spans_cover(text, &classify(text, &plain)));

        let described = [
            Class::Keyword,
            Class::Type,
            Class::Function,
            Class::Text,
            Class::Comment,
        ];
        let seen = classes_of(text, &plain);
        assert!(
            seen.iter().all(|class| !described.contains(class)),
            "nothing that needs describing should be coloured: {seen:?}"
        );
        assert!(
            seen.contains(&Class::Punctuation) && seen.contains(&Class::Number),
            "a bracket and a digit need no description: {seen:?}"
        );
    }

    #[test]
    fn a_multibyte_character_never_splits_a_span() {
        // Byte offsets that land inside a character would panic the
        // moment a front end sliced the text with them.
        let text = "let s = \"héllo → wörld\"; // ünicöde\n";
        let spans = classify(text, &C_LIKE);
        assert!(spans_cover(text, &spans));
        for span in spans {
            assert!(text.is_char_boundary(span.start));
            assert!(text.is_char_boundary(span.start + span.len));
        }
    }

    #[test]
    fn a_case_insensitive_language_matches_a_keyword_in_any_case() {
        const SQL: Language = Language {
            line_comment: &["--"],
            block_comment: &[],
            quotes: &[Quote::simple('\'')],
            keywords: &["select", "from"],
            types: &[],
            calls: false,
            ignore_case: true,
        };
        let seen = runs("SELECT a From b", &SQL);
        assert!(seen.contains(&(Class::Keyword, "SELECT".to_owned())));
        assert!(seen.contains(&(Class::Keyword, "From".to_owned())));
    }

    #[test]
    fn a_language_without_calls_does_not_invent_them() {
        const DATA: Language = Language {
            line_comment: &["#"],
            block_comment: &[],
            quotes: &[Quote::simple('"')],
            keywords: &["true", "false"],
            types: &[],
            calls: false,
            ignore_case: false,
        };
        let seen = runs("key (value)", &DATA);
        assert!(
            seen.iter().all(|(class, _)| *class != Class::Function),
            "a data format has no calls to colour: {seen:?}"
        );
    }
}
