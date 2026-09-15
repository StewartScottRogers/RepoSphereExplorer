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

            // Whichever marker is longer wins, rather than whichever is
            // checked first. Several languages open a block comment with
            // the line comment's own marker and something after it - Lua's
            // `--` and `--[[`, Julia's `#` and `#=`, Nim's `#` and `#[`,
            // MATLAB's `%` and `%{`. Testing the line marker first matched
            // all four of those block openers as line comments, so a
            // multi-line comment in any of them was coloured only as far
            // as its first newline.
            let line_marker = self
                .language
                .line_comment
                .iter()
                .filter(|marker| rest.starts_with(**marker))
                .max_by_key(|marker| marker.len())
                .copied();
            let block_marker = self
                .language
                .block_comment
                .iter()
                .filter(|(open, _)| rest.starts_with(*open))
                .max_by_key(|(open, _)| open.len())
                .copied();

            let line_wins = match (line_marker, block_marker) {
                (Some(marker), Some((open, _))) => marker.len() >= open.len(),
                (Some(_), None) => true,
                (None, _) => false,
            };
            if line_wins {
                let end = self.line_comment_end(at);
                self.emit(at, end, Class::Comment);
                at = end;
                continue;
            }
            if let Some((open, close)) = block_marker {
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
        // A checked add, because this is the thing that says yes or no
        // about spans it did not produce. A plugin whose own classifier
        // computed a length by a subtraction that went the wrong way hands
        // it a huge one, and a validator that panics on the input it
        // exists to judge is no use at all.
        let Some(end) = span.start.checked_add(span.len) else {
            return false;
        };
        at = end;
        if at > text.len() || !text.is_char_boundary(at) {
            return false;
        }
    }
    at == text.len()
}

#[cfg(test)]
mod tests {
    use super::{Language, Quote, classify, spans_cover};
    use plugin_api::{Class, Span};

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

    /// Go-shaped: a raw run that crosses lines and has no escape, beside an
    /// ordinary one that does neither.
    const RAW_AND_MULTILINE: Language = Language {
        line_comment: &["//"],
        block_comment: &[("/*", "*/")],
        quotes: &[
            Quote::simple('"'),
            Quote {
                open: '`',
                close: '`',
                escape: None,
                multiline: true,
            },
        ],
        keywords: &["func", "package", "return"],
        types: &["string"],
        calls: true,
        ignore_case: false,
    };

    /// Shell-shaped: a hash line comment, and a single-quoted run in which a
    /// backslash means nothing at all.
    const HASH_AND_RAW: Language = Language {
        line_comment: &["#"],
        block_comment: &[],
        quotes: &[
            Quote::simple('"'),
            Quote {
                open: '\'',
                close: '\'',
                escape: None,
                multiline: false,
            },
        ],
        keywords: &["if", "fi", "for"],
        types: &[],
        calls: false,
        ignore_case: false,
    };

    /// Python-shaped: the triple-quoted run is described as a block comment,
    /// so it has to be matched before the single quote it starts with.
    const TRIPLE_QUOTED: Language = Language {
        line_comment: &["#"],
        block_comment: &[("\"\"\"", "\"\"\"")],
        quotes: &[Quote::simple('"'), Quote::simple('\'')],
        keywords: &["def", "return"],
        types: &["str"],
        calls: true,
        ignore_case: false,
    };

    /// Lua-shaped: the block comment opens with the line comment marker,
    /// which is also true of Julia, Nim and MATLAB.
    const SHADOWED_BLOCK: Language = Language {
        line_comment: &["--"],
        block_comment: &[("--[[", "]]")],
        quotes: &[Quote::simple('"'), Quote::simple('\'')],
        keywords: &["local", "function", "end"],
        types: &[],
        calls: true,
        ignore_case: false,
    };

    /// Markup-shaped: a four-character block comment marker, no line comment
    /// at all, and names matched whatever their case.
    const MARKUP: Language = Language {
        line_comment: &[],
        block_comment: &[("<!--", "-->")],
        quotes: &[Quote::simple('"'), Quote::simple('\'')],
        keywords: &["div", "span"],
        types: &[],
        calls: false,
        ignore_case: true,
    };

    /// A quoted run whose two ends are different characters, which no plugin
    /// has yet but the description allows, so it has to work.
    const ANGLED: Language = Language {
        line_comment: &[";"],
        block_comment: &[],
        quotes: &[Quote {
            open: '<',
            close: '>',
            escape: Some('\\'),
            multiline: true,
        }],
        keywords: &["include"],
        types: &[],
        calls: false,
        ignore_case: false,
    };

    /// Query-shaped: words matched whatever their case, and no calls.
    const QUERY: Language = Language {
        line_comment: &["--"],
        block_comment: &[("/*", "*/")],
        quotes: &[Quote::simple('\''), Quote::simple('"')],
        keywords: &["select", "from", "where", "café"],
        types: &["integer"],
        calls: false,
        ignore_case: true,
    };

    /// A language that has said nothing about itself.
    const DESCRIBES_NOTHING: Language = Language::new();

    /// Every description the property tests run over, so that a shape only
    /// one language has - a raw run, a shadowed marker, a four-character
    /// marker - is still held to the covering rule.
    static LANGUAGES: [(&str, Language); 9] = [
        ("c-like", C_LIKE),
        ("raw and multiline", RAW_AND_MULTILINE),
        ("hash and raw", HASH_AND_RAW),
        ("triple quoted", TRIPLE_QUOTED),
        ("shadowed block", SHADOWED_BLOCK),
        ("markup", MARKUP),
        ("angled", ANGLED),
        ("query", QUERY),
        ("describes nothing", DESCRIBES_NOTHING),
    ];

    fn owned(texts: &[&str]) -> Vec<String> {
        texts.iter().copied().map(String::from).collect()
    }

    /// Inputs that are about shape rather than content: nothing, almost
    /// nothing, and every way a line can end.
    fn shape_inputs() -> Vec<String> {
        owned(&[
            "",
            "a",
            "_",
            "1",
            " ",
            "\t",
            "\n",
            "\r",
            "\r\n",
            "\n\n",
            "   ",
            " \t \r\n \t ",
            "x\r\ny",
            "no trailing newline",
            "trailing newline\n",
            "\u{feff}",
            "\u{feff}let x = 1;",
        ])
    }

    /// Inputs where something is left open, which is the tolerance claim and
    /// so the part most worth pushing on.
    fn unterminated_inputs() -> Vec<String> {
        owned(&[
            "\"",
            "'",
            "`",
            "<",
            "\"unterminated",
            "\"unterminated\nback to normal 1",
            "'c",
            "\"ends at the end\"",
            "\"trailing backslash\\",
            "\"\\",
            "\\",
            "/*",
            "/*/",
            "*/",
            "/",
            "//",
            "--",
            "#",
            "<!--",
            "-->",
            "--[[ open",
            "\"\"\"",
            "\"\"\"open",
            "/* open\nand open\nand still open",
        ])
    }

    /// Inputs where one kind of marker sits inside another.
    fn marker_inputs() -> Vec<String> {
        owned(&[
            "\"// not a comment\"",
            "\"/* not a comment */\"",
            "/* \"not a string\" */",
            "// \"not a string\"",
            "/* // still the block */\nafter",
            "/* a /* b */ c */",
            "\"a\\\"b\"",
            "'a\\'b'",
            "\"a\\\\\"",
            "let s = \"a\" + \"b\";",
            "`raw\nacross lines`",
            "don't split this",
            "a--b",
            "a#b",
        ])
    }

    /// Inputs with characters wider than one byte, which is where a byte
    /// offset that is one out stops being a colouring bug and becomes a
    /// panic.
    fn unicode_inputs() -> Vec<String> {
        owned(&[
            "héllo",
            "héllo = wörld",
            "\"héllo\"",
            "'é'",
            "変数 = 1",
            "\"日本語のテキスト\"",
            "// 日本語",
            "/* 日本語 */",
            "🙂",
            "\"🙂🙂\"",
            "x = \"🙂\"; // 🙂",
            "e\u{301}",
            "cafe\u{301} = 1",
            "→",
            "a→b",
            "\u{feff}\"héllo\"",
            "\"日",
            "`日",
            "función(1)",
            "١٢٣",
            "½",
        ])
    }

    /// Numeric literals of every shape the loose rule is meant to swallow.
    fn number_inputs() -> Vec<String> {
        owned(&[
            "0",
            "0xFF",
            "0xFF_u8",
            "0b1010",
            "0o777",
            "1e10",
            "1E+10",
            "1.5e-3",
            "1.",
            ".5",
            "1_000_000",
            "123abc",
            "0x",
            "9.9.9",
            "1-2",
            "1+2",
            "1e+5",
            "007",
            "1..10",
        ])
    }

    fn nasty_inputs() -> Vec<String> {
        let mut inputs = shape_inputs();
        inputs.extend(unterminated_inputs());
        inputs.extend(marker_inputs());
        inputs.extend(unicode_inputs());
        inputs.extend(number_inputs());
        inputs.push("let x: int = size(1); // done\n/* and */ \"a\"".to_owned());
        inputs.push("if(x){return 1;}else{return 0;}".to_owned());
        inputs.push("a".repeat(5_000));
        inputs.push("(".repeat(2_000));
        inputs.push("\"".repeat(1_001));
        inputs.push("é".repeat(2_000));
        inputs
    }

    #[test]
    fn every_language_covers_every_nasty_input_exactly_once() {
        // The spine of this crate's contract. The pane slices the text by
        // these spans to paint it, so a gap loses characters, an overlap
        // draws them twice, and an offset inside a character panics.
        for (name, language) in &LANGUAGES {
            for text in nasty_inputs() {
                let spans = classify(&text, language);
                assert!(
                    spans_cover(&text, &spans),
                    "{name} must cover {text:?} exactly once: {spans:?}"
                );
            }
        }
    }

    #[test]
    fn walking_the_spans_of_any_input_redraws_the_text_it_came_from() {
        // Stronger than the covering rule and closer to what the pane does:
        // slicing by every span and joining the pieces has to give the text
        // back, character for character.
        for (name, language) in &LANGUAGES {
            for text in nasty_inputs() {
                let mut redrawn = String::new();
                for span in classify(&text, language) {
                    redrawn.push_str(&text[span.start..span.start + span.len]);
                }
                assert_eq!(redrawn, text, "{name} must redraw {text:?} from its spans");
            }
        }
    }

    #[test]
    fn no_span_of_any_input_is_empty() {
        // A zero-length span passes the covering rule and still means the
        // pane draws nothing for a run it was told about.
        for (name, language) in &LANGUAGES {
            for text in nasty_inputs() {
                let spans = classify(&text, language);
                assert!(
                    spans.iter().all(|span| span.len > 0),
                    "{name} emitted an empty span for {text:?}: {spans:?}"
                );
            }
        }
    }

    #[test]
    fn empty_text_produces_no_spans_at_all() {
        for (name, language) in &LANGUAGES {
            assert!(
                classify("", language).is_empty(),
                "{name} invented a span for empty text"
            );
        }
    }

    #[test]
    fn a_single_character_is_a_single_span_of_the_class_it_deserves() {
        for (text, class) in [
            ("a", Class::Plain),
            ("_", Class::Plain),
            (" ", Class::Plain),
            ("1", Class::Number),
            ("(", Class::Punctuation),
            ("\"", Class::Text),
        ] {
            let seen = runs(text, &C_LIKE);
            assert_eq!(seen, vec![(class, text.to_owned())], "{text:?}");
        }
    }

    #[test]
    fn text_that_is_only_whitespace_is_one_plain_span() {
        let text = "  \t\n \t  ";
        assert_eq!(runs(text, &C_LIKE), vec![(Class::Plain, text.to_owned())]);
    }

    #[test]
    fn a_lone_carriage_return_is_whitespace_and_does_not_end_a_line() {
        // A file saved on one platform and read on another has these, and
        // treating one as a line ending would end a comment early.
        let text = "a\rb";
        assert_eq!(runs(text, &C_LIKE), vec![(Class::Plain, text.to_owned())]);
    }

    #[test]
    fn a_comment_on_a_windows_line_swallows_the_carriage_return_and_stops() {
        let seen = runs("// note\r\nlet x = 1;", &C_LIKE);
        assert_eq!(
            seen.first(),
            Some(&(Class::Comment, "// note\r".to_owned())),
            "the comment ends at the newline, carriage return and all: {seen:?}"
        );
        assert!(
            seen.contains(&(Class::Keyword, "let".to_owned())),
            "the next line is read normally: {seen:?}"
        );
    }

    #[test]
    fn text_with_no_trailing_newline_still_ends_its_last_span_at_the_end() {
        for text in ["let x = 1", "// note", "\"open", "/* open"] {
            let spans = classify(text, &C_LIKE);
            let last = spans.last().expect("a non-empty text has a last span");
            assert_eq!(
                last.start + last.len,
                text.len(),
                "the last span must reach the end of {text:?}"
            );
        }
    }

    #[test]
    fn an_unclosed_character_literal_stops_at_the_end_of_its_line() {
        let seen = runs("let c = 'a\nlet d = 1;", &C_LIKE);
        assert!(seen.contains(&(Class::Text, "'a".to_owned())), "{seen:?}");
        assert!(
            seen.contains(&(Class::Number, "1".to_owned())),
            "the line after an unclosed character literal is read normally: {seen:?}"
        );
    }

    #[test]
    fn a_string_that_closes_on_the_last_character_of_the_text_is_one_span() {
        let text = "\"ends at the end\"";
        assert_eq!(runs(text, &C_LIKE), vec![(Class::Text, text.to_owned())]);
    }

    #[test]
    fn a_string_whose_last_character_is_a_backslash_runs_to_the_end_of_the_text() {
        // The escape has nothing left to escape, and the scan must stop at
        // the end rather than reading past it.
        for text in ["\"open\\", "\"\\", "\\"] {
            let spans = classify(text, &C_LIKE);
            assert!(spans_cover(text, &spans), "{text:?}: {spans:?}");
        }
        assert_eq!(
            runs("\"open\\", &C_LIKE),
            vec![(Class::Text, "\"open\\".to_owned())]
        );
    }

    #[test]
    fn a_block_comment_marker_with_nothing_after_it_does_not_read_past_the_end() {
        for text in ["/*", "/*/", "<!--", "\"\"\""] {
            let language = if text.starts_with('<') {
                &MARKUP
            } else if text.starts_with('"') {
                &TRIPLE_QUOTED
            } else {
                &C_LIKE
            };
            let seen = runs(text, language);
            assert_eq!(
                seen,
                vec![(Class::Comment, text.to_owned())],
                "an opener with no room to close is one comment: {text:?}"
            );
        }
    }

    #[test]
    fn a_closing_marker_on_its_own_is_only_punctuation() {
        // `*/` with no `/*` before it is two operators, not a comment, and
        // certainly not something that swallows the file.
        let seen = runs("*/ let", &C_LIKE);
        assert_eq!(seen[0], (Class::Punctuation, "*".to_owned()), "{seen:?}");
        assert_eq!(seen[1], (Class::Punctuation, "/".to_owned()), "{seen:?}");
        assert!(seen.contains(&(Class::Keyword, "let".to_owned())));
    }

    #[test]
    fn an_unterminated_run_that_may_cross_lines_runs_to_the_end_of_the_text() {
        let text = "x := `raw\nstill raw\nstill raw";
        let seen = runs(text, &RAW_AND_MULTILINE);
        assert!(
            seen.iter()
                .any(|(class, run)| *class == Class::Text && run.ends_with("still raw")),
            "a run that may cross lines has nothing to stop it but the end: {seen:?}"
        );
    }

    #[test]
    fn a_run_that_may_not_cross_a_line_stops_where_one_that_may_does_not() {
        let text = "\"single\n`multiple\n";
        let seen = runs(text, &RAW_AND_MULTILINE);
        assert!(
            seen.contains(&(Class::Text, "\"single".to_owned())),
            "{seen:?}"
        );
        assert!(
            seen.contains(&(Class::Text, "`multiple\n".to_owned())),
            "{seen:?}"
        );
    }

    #[test]
    fn a_comment_marker_inside_a_string_does_not_open_a_comment() {
        for text in ["\"// not a comment\"", "\"/* not a comment */\""] {
            let seen = runs(text, &C_LIKE);
            assert_eq!(
                seen,
                vec![(Class::Text, text.to_owned())],
                "the whole quoted run is string: {seen:?}"
            );
        }
    }

    #[test]
    fn a_quote_inside_a_block_comment_does_not_open_a_string() {
        let seen = runs("/* \"not a string */ 1", &C_LIKE);
        assert!(
            seen.iter().all(|(class, _)| *class != Class::Text),
            "a quote inside a comment is comment: {seen:?}"
        );
        assert!(
            seen.contains(&(Class::Number, "1".to_owned())),
            "the comment still ends where it says it does: {seen:?}"
        );
    }

    #[test]
    fn a_line_comment_marker_inside_a_block_comment_does_not_end_it() {
        let seen = runs("/* // still open\nstill open */ 1", &C_LIKE);
        assert!(
            seen.contains(&(Class::Comment, "/* // still open\nstill open */".to_owned())),
            "{seen:?}"
        );
        assert!(seen.contains(&(Class::Number, "1".to_owned())), "{seen:?}");
    }

    #[test]
    fn a_block_comment_ends_at_the_first_close_and_does_not_nest() {
        // No language here nests them, so the inner close ends the whole
        // thing and the trailing one is left as operators. Worth pinning:
        // a reader of Rust or D would expect otherwise.
        let seen = runs("/* a /* b */ c */", &C_LIKE);
        assert_eq!(
            seen.first(),
            Some(&(Class::Comment, "/* a /* b */".to_owned())),
            "{seen:?}"
        );
        assert!(
            seen.iter()
                .any(|(class, run)| *class == Class::Punctuation && run == "*"),
            "the outer close is left over as punctuation: {seen:?}"
        );
    }

    #[test]
    fn an_escaped_backslash_before_the_delimiter_still_closes_the_string() {
        // `"a\\"` ends; `"a\"` does not. Getting this backwards colours the
        // rest of the file.
        assert_eq!(
            runs("\"a\\\\\" + 1", &C_LIKE).first(),
            Some(&(Class::Text, "\"a\\\\\"".to_owned()))
        );
        assert_eq!(
            runs("\"a\\\" + 1", &C_LIKE),
            vec![(Class::Text, "\"a\\\" + 1".to_owned())],
            "the delimiter after a single backslash is escaped, not closing"
        );
    }

    #[test]
    fn a_run_with_no_escape_ends_at_a_delimiter_a_backslash_would_have_hidden() {
        // Shell's single quotes: a backslash inside one is just a
        // backslash, so the run ends at the next quote whatever precedes it.
        let seen = runs("'a\\'b'", &HASH_AND_RAW);
        assert_eq!(
            seen.first(),
            Some(&(Class::Text, "'a\\'".to_owned())),
            "a run with no escape is not fooled by a backslash: {seen:?}"
        );
    }

    #[test]
    fn a_backslash_at_the_end_of_a_line_carries_a_single_line_string_onto_the_next() {
        // The escape is applied before the line test, so a trailing
        // backslash continues the run - which is what C and the shell mean
        // by it, and worth stating because the field is called `multiline`.
        let seen = runs("\"one\\\ntwo\"\n3", &C_LIKE);
        assert!(
            seen.contains(&(Class::Text, "\"one\\\ntwo\"".to_owned())),
            "{seen:?}"
        );
        assert!(seen.contains(&(Class::Number, "3".to_owned())), "{seen:?}");
    }

    #[test]
    fn a_longer_marker_described_as_a_block_comment_beats_the_quote_it_starts_with() {
        // Python's docstring: `"""` has to win over `"`, which it does only
        // because block comments are tried before quotes.
        let seen = runs(
            "\"\"\"a doc\nstring\"\"\"\ndef f(): return 1",
            &TRIPLE_QUOTED,
        );
        assert_eq!(
            seen.first(),
            Some(&(Class::Comment, "\"\"\"a doc\nstring\"\"\"".to_owned())),
            "{seen:?}"
        );
        assert!(
            seen.contains(&(Class::Keyword, "def".to_owned())),
            "{seen:?}"
        );
    }

    #[test]
    fn a_block_comment_opened_with_the_line_comment_marker_still_ends_at_its_own_close() {
        // Lua's `--[[ ... ]]`, and the same shape in Julia (`#=`), Nim
        // (`#[`) and MATLAB (`%{`). Line comments are tried before block
        // comments, so the shorter marker matches first and the block form
        // is lost.
        let text = "--[[ note\nstill note ]]\nlocal x = 1";
        let seen = runs(text, &SHADOWED_BLOCK);
        assert!(
            seen.iter()
                .any(|(class, run)| *class == Class::Comment && run.ends_with("]]")),
            "the block comment is shadowed by the line comment marker it starts \
             with, so it ends at the first newline instead of at `]]`: {seen:?}"
        );
    }

    #[test]
    fn an_accented_letter_is_part_of_the_name_around_it() {
        let seen = runs("héllo = wörld", &C_LIKE);
        assert_eq!(seen.first(), Some(&(Class::Plain, "héllo ".to_owned())));
        assert!(
            seen.contains(&(Class::Punctuation, "=".to_owned())),
            "{seen:?}"
        );
    }

    #[test]
    fn a_keyword_made_of_multibyte_letters_is_matched_and_called() {
        // Every offset in here is past a two-byte character, so a keyword
        // measured in characters rather than bytes would be cut short.
        const ACCENTED: Language = Language {
            line_comment: &["//"],
            block_comment: &[],
            quotes: &[Quote::simple('"')],
            keywords: &["función"],
            types: &["año"],
            calls: true,
            ignore_case: false,
        };
        let seen = runs("función año llamada(1)", &ACCENTED);
        assert!(
            seen.contains(&(Class::Keyword, "función".to_owned())),
            "{seen:?}"
        );
        assert!(seen.contains(&(Class::Type, "año".to_owned())), "{seen:?}");
        assert!(
            seen.contains(&(Class::Function, "llamada".to_owned())),
            "{seen:?}"
        );
    }

    #[test]
    fn a_run_of_ideographs_is_one_name_and_not_a_string_of_punctuation() {
        let seen = runs("変数 = 1", &C_LIKE);
        assert_eq!(seen.first(), Some(&(Class::Plain, "変数 ".to_owned())));
        assert!(seen.contains(&(Class::Number, "1".to_owned())), "{seen:?}");
    }

    #[test]
    fn an_emoji_is_one_punctuation_span_of_all_four_of_its_bytes() {
        let seen = runs("🙂", &C_LIKE);
        assert_eq!(seen, vec![(Class::Punctuation, "🙂".to_owned())]);
        let spans = classify("🙂", &C_LIKE);
        assert_eq!(spans[0].len, 4, "a surrogate pair is not two spans");
    }

    #[test]
    fn a_combining_mark_is_classified_apart_from_the_letter_it_follows() {
        // It continues neither a name nor a number, so `e` and the accent
        // over it land in different spans. Harmless to the covering rule,
        // but it means a composed letter can be coloured in two halves.
        let seen = runs("e\u{301}", &C_LIKE);
        assert_eq!(
            seen,
            vec![
                (Class::Plain, "e".to_owned()),
                (Class::Punctuation, "\u{301}".to_owned()),
            ],
            "{seen:?}"
        );
    }

    #[test]
    fn a_byte_order_mark_does_not_disturb_the_token_after_it() {
        let seen = runs("\u{feff}let x = 1;", &C_LIKE);
        assert_eq!(
            seen.first(),
            Some(&(Class::Punctuation, "\u{feff}".to_owned())),
            "{seen:?}"
        );
        assert!(
            seen.contains(&(Class::Keyword, "let".to_owned())),
            "{seen:?}"
        );
    }

    #[test]
    fn a_multibyte_character_beside_a_delimiter_keeps_the_span_on_a_boundary() {
        for text in [
            "\"日\"",
            "\"日",
            "'é'",
            "// é",
            "/* é */",
            "`日",
            "x=\"🙂\";",
        ] {
            let spans = classify(text, &RAW_AND_MULTILINE);
            assert!(spans_cover(text, &spans), "{text:?}: {spans:?}");
            for span in &spans {
                assert!(
                    text.is_char_boundary(span.start)
                        && text.is_char_boundary(span.start + span.len),
                    "{text:?} split a character: {span:?}"
                );
            }
        }
    }

    #[test]
    fn a_text_ending_in_a_multibyte_character_ends_its_last_span_at_the_end() {
        for text in ["let x = 日", "// 日", "\"日", "é"] {
            let spans = classify(text, &C_LIKE);
            let last = spans.last().expect("a non-empty text has a last span");
            assert_eq!(last.start + last.len, text.len(), "{text:?}: {spans:?}");
        }
    }

    #[test]
    fn a_digit_that_is_not_a_plain_one_does_not_start_a_number() {
        // Arabic-Indic digits and a vulgar fraction are numeric without
        // being the digits a literal is made of; reading them as numbers
        // would colour prose.
        for text in ["١٢٣", "½"] {
            let seen = runs(text, &C_LIKE);
            assert_eq!(seen, vec![(Class::Plain, text.to_owned())], "{text:?}");
        }
    }

    #[test]
    fn a_radix_prefix_a_separator_and_a_suffix_all_stay_in_one_number() {
        for text in ["0xFF_u8", "0b1010", "0o777", "1_000_000", "007", "123abc"] {
            let seen = runs(text, &C_LIKE);
            assert_eq!(seen, vec![(Class::Number, text.to_owned())], "{text:?}");
        }
    }

    #[test]
    fn an_exponent_keeps_its_sign_but_an_operator_does_not_join_a_number() {
        assert_eq!(
            runs("1.5e-3", &C_LIKE),
            vec![(Class::Number, "1.5e-3".to_owned())]
        );
        assert_eq!(
            runs("1E+10", &C_LIKE),
            vec![(Class::Number, "1E+10".to_owned())]
        );
        assert_eq!(
            runs("1-2", &C_LIKE),
            vec![
                (Class::Number, "1".to_owned()),
                (Class::Punctuation, "-".to_owned()),
                (Class::Number, "2".to_owned()),
            ],
            "a minus that follows a digit rather than an exponent is an operator"
        );
    }

    #[test]
    fn a_trailing_dot_joins_a_number_and_a_leading_one_does_not() {
        assert_eq!(runs("1.", &C_LIKE), vec![(Class::Number, "1.".to_owned())]);
        assert_eq!(
            runs(".5", &C_LIKE),
            vec![
                (Class::Punctuation, ".".to_owned()),
                (Class::Number, "5".to_owned()),
            ],
            "a number has to start with a digit"
        );
    }

    #[test]
    fn a_hyphen_joins_a_name_though_it_separates_two_numbers() {
        // The hyphen is in a name on purpose, so `font-size` stays whole.
        // The price is that `a-b` is one name where `1-2` is three runs.
        assert_eq!(runs("a-b", &C_LIKE), vec![(Class::Plain, "a-b".to_owned())]);
        assert_eq!(
            runs("font-size.small", &C_LIKE),
            vec![(Class::Plain, "font-size.small".to_owned())]
        );
    }

    #[test]
    fn a_type_is_preferred_to_a_keyword_when_a_word_is_listed_as_both() {
        const BOTH: Language = Language {
            line_comment: &[],
            block_comment: &[],
            quotes: &[],
            keywords: &["record"],
            types: &["record"],
            calls: false,
            ignore_case: false,
        };
        assert_eq!(
            runs("record", &BOTH),
            vec![(Class::Type, "record".to_owned())],
            "the type list is consulted first"
        );
    }

    #[test]
    fn a_case_sensitive_language_leaves_a_keyword_in_the_wrong_case_alone() {
        let seen = runs("LET Return", &C_LIKE);
        assert!(
            seen.iter().all(|(class, _)| *class != Class::Keyword),
            "{seen:?}"
        );
    }

    #[test]
    fn matching_regardless_of_case_folds_only_the_plain_letters() {
        // `eq_ignore_ascii_case` is what does the folding, so a keyword
        // with an accent in it matches only in the case it was written.
        let seen = runs("SELECT café CAFÉ", &QUERY);
        assert!(
            seen.contains(&(Class::Keyword, "SELECT".to_owned())),
            "{seen:?}"
        );
        assert!(
            seen.contains(&(Class::Keyword, "café".to_owned())),
            "{seen:?}"
        );
        assert!(
            !seen.contains(&(Class::Keyword, "CAFÉ".to_owned())),
            "folding stops at the plain letters: {seen:?}"
        );
    }

    #[test]
    fn a_call_is_seen_through_spaces_and_tabs_but_not_across_a_line() {
        assert!(runs("f (1)", &C_LIKE).contains(&(Class::Function, "f".to_owned())));
        assert!(runs("f\t(1)", &C_LIKE).contains(&(Class::Function, "f".to_owned())));
        let across = runs("f\n(1)", &C_LIKE);
        assert!(
            across.iter().all(|(class, _)| *class != Class::Function),
            "a bracket on the next line is not this name's call: {across:?}"
        );
    }

    #[test]
    fn a_reserved_word_before_a_bracket_stays_reserved() {
        // `if (` is not a call, and drawing it as one would make every
        // condition look like a function.
        let seen = runs("if (x) return size(1);", &C_LIKE);
        assert!(
            seen.contains(&(Class::Keyword, "if".to_owned())),
            "{seen:?}"
        );
        assert!(
            seen.contains(&(Class::Function, "size".to_owned())),
            "a name that is not reserved still calls: {seen:?}"
        );
    }

    #[test]
    fn a_quoted_run_whose_two_ends_differ_closes_only_on_its_close() {
        // The opener appearing again inside is not a close, and is not a
        // second opener either.
        let seen = runs("include <a<b>", &ANGLED);
        assert!(
            seen.contains(&(Class::Keyword, "include".to_owned())),
            "{seen:?}"
        );
        assert!(
            seen.contains(&(Class::Text, "<a<b>".to_owned())),
            "{seen:?}"
        );
    }

    #[test]
    fn a_simple_quote_closes_on_its_own_character_escapes_and_holds_to_one_line() {
        const PIPED: Language = Language {
            line_comment: &[],
            block_comment: &[],
            quotes: &[Quote::simple('|')],
            keywords: &[],
            types: &[],
            calls: false,
            ignore_case: false,
        };
        let quote = Quote::simple('|');
        assert_eq!((quote.open, quote.close), ('|', '|'));
        assert_eq!(quote.escape, Some('\\'));
        assert!(!quote.multiline);
        assert_eq!(
            runs("|a\\|b|", &PIPED),
            vec![(Class::Text, "|a\\|b|".to_owned())]
        );
        assert_eq!(
            runs("|a\nb", &PIPED).first(),
            Some(&(Class::Text, "|a".to_owned()))
        );
    }

    #[test]
    fn the_default_language_describes_exactly_as_little_as_the_empty_one() {
        let text = "if (x) { return \"a\"; } // c\n";
        assert_eq!(
            classify(text, &Language::default()),
            classify(text, &Language::new())
        );
    }

    #[test]
    fn a_very_long_single_line_is_covered_exactly() {
        // No newline to break the work up, which is the shape a minified
        // file arrives in.
        let long = "let x = size(1); ".repeat(20_000);
        let spans = classify(&long, &C_LIKE);
        assert!(spans_cover(&long, &spans));
        assert_eq!(
            spans
                .iter()
                .filter(|span| span.class == Class::Keyword)
                .count(),
            20_000
        );
    }

    #[test]
    fn a_great_many_tiny_tokens_are_each_their_own_span() {
        let many = "1,".repeat(50_000);
        let spans = classify(&many, &C_LIKE);
        assert!(spans_cover(&many, &spans));
        assert_eq!(spans.len(), 100_000, "a number and a comma each time");
    }

    #[test]
    fn deeply_nested_brackets_are_walked_rather_than_recursed_into() {
        // Fifty thousand deep would overflow a stack if the scan recursed.
        let deep = format!("{}x{}", "(".repeat(50_000), ")".repeat(50_000));
        let spans = classify(&deep, &C_LIKE);
        assert!(spans_cover(&deep, &spans));
        assert_eq!(spans.len(), 100_001);
    }

    #[test]
    fn spans_cover_rejects_a_gap_an_overlap_and_a_run_past_the_end() {
        let text = "abcd";
        assert!(
            !spans_cover(
                text,
                &[Span::new(0, 1, Class::Plain), Span::new(2, 2, Class::Plain)]
            ),
            "a gap is not a cover"
        );
        assert!(
            !spans_cover(
                text,
                &[Span::new(0, 3, Class::Plain), Span::new(2, 2, Class::Plain)]
            ),
            "an overlap is not a cover"
        );
        assert!(
            !spans_cover(text, &[Span::new(0, 5, Class::Plain)]),
            "a run past the end is not a cover"
        );
        assert!(
            !spans_cover(text, &[Span::new(0, 3, Class::Plain)]),
            "stopping short is not a cover"
        );
        assert!(spans_cover(text, &[Span::new(0, 4, Class::Plain)]));
    }

    #[test]
    fn spans_cover_rejects_a_boundary_inside_a_character() {
        let text = "é";
        assert!(
            !spans_cover(
                text,
                &[Span::new(0, 1, Class::Plain), Span::new(1, 1, Class::Plain)]
            ),
            "half of a character is not a span"
        );
        assert!(spans_cover(text, &[Span::new(0, 2, Class::Plain)]));
    }

    #[test]
    fn spans_cover_accepts_nothing_only_for_empty_text() {
        assert!(spans_cover("", &[]));
        assert!(!spans_cover("a", &[]));
    }

    #[test]
    fn spans_cover_answers_rather_than_bursting_on_an_impossible_length() {
        // A plugin that classifies its own format calls this to check
        // itself, and the length it passes may be the wrong one - a
        // subtraction that went the wrong way gives a huge number rather
        // than a small one. The answer is no; it should not be a panic.
        let text = "ab";
        assert!(!spans_cover(
            text,
            &[
                Span::new(0, 1, Class::Plain),
                Span::new(1, usize::MAX, Class::Plain)
            ]
        ));
    }
}
