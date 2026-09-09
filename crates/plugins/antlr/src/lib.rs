//! ANTLR grammar file type plugin: core and presentation halves.
//!
//! A `grammar`, `lexer grammar` or `parser grammar` declaration is the
//! marker, and nothing else writes one.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["g4"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// Its name.
    pub name: String,
    /// How many alternatives it has, counted by the `|` between them.
    pub alternatives: usize,
    /// Whether it is a lexer rule, which `ANTLR` distinguishes by an
    /// initial capital.
    pub lexer: bool,
    /// Whether it is `fragment`, and so only usable from another lexer
    /// rule.
    pub fragment: bool,
}

/// View data produced by [`AntlrCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AntlrView {
    /// The grammar's name.
    pub name: Option<String>,
    /// `combined`, `lexer` or `parser`.
    pub kind: String,
    /// The options set.
    pub options: Vec<String>,
    /// The grammars it imports.
    pub imports: Vec<String>,
    /// The tokens it declares without defining.
    pub tokens: Vec<String>,
    /// Every rule, in file order.
    pub rules: Vec<Rule>,
    /// The channels it declares.
    pub channels: Vec<String>,
    /// Rules named but never defined, which is a grammar that cannot be
    /// generated.
    pub undefined: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// `line` with any trailing `//` comment removed.
fn uncommented(line: &str) -> &str {
    match line.find("//") {
        Some(at) => &line[..at],
        None => line,
    }
}

/// The comma-separated names inside a `{ ... }` block on one line.
fn braced_list(text: &str) -> Vec<String> {
    let Some(inner) = text
        .split_once('{')
        .and_then(|(_, rest)| rest.split('}').next())
    else {
        return Vec::new();
    };
    inner
        .split(',')
        .map(|one| one.trim().trim_end_matches(';').trim().to_owned())
        .filter(|one| !one.is_empty())
        .collect()
}

/// `text` with everything that is not grammar structure removed:
/// comments, quoted literals, and character classes.
///
/// Needed before rules are read. A body like `[a-zA-Z_]` splits into
/// `a`, `zA` and `Z_` if it is treated as identifiers, and each then
/// looks like a rule that was referenced and never defined.
fn cleaned(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for raw in text.lines() {
        let line = uncommented(raw);
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\'' | '"' => {
                    let quote = c;
                    let mut escaped = false;
                    for inner in chars.by_ref() {
                        if escaped {
                            escaped = false;
                        } else if inner == '\\' {
                            escaped = true;
                        } else if inner == quote {
                            break;
                        }
                    }
                    out.push(' ');
                }
                '[' => {
                    let mut escaped = false;
                    for inner in chars.by_ref() {
                        if escaped {
                            escaped = false;
                        } else if inner == '\\' {
                            escaped = true;
                        } else if inner == ']' {
                            break;
                        }
                    }
                    out.push(' ');
                }
                other => out.push(other),
            }
        }
        out.push('\n');
    }
    out
}

/// The identifiers in `body`, which are the rules and tokens it names.
fn identifiers(body: &str) -> Vec<String> {
    let mut found = Vec::new();
    for word in body.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if !word.is_empty()
            && word.chars().next().is_some_and(char::is_alphabetic)
            && !found.contains(&word.to_owned())
        {
            found.push(word.to_owned());
        }
    }
    found
}

/// Everything [`AntlrView`] holds, read from `text`.
fn parse(text: &str) -> AntlrView {
    let mut view = AntlrView {
        name: None,
        kind: "combined".to_owned(),
        options: Vec::new(),
        imports: Vec::new(),
        tokens: Vec::new(),
        rules: Vec::new(),
        channels: Vec::new(),
        undefined: Vec::new(),
        content: String::new(),
        truncated: false,
    };

    // The header is line-oriented; the rules are not, so they are read
    // separately.
    for raw in text.lines() {
        let line = uncommented(raw).trim();
        for (prefix, kind) in [
            ("lexer grammar ", "lexer"),
            ("parser grammar ", "parser"),
            ("grammar ", "combined"),
        ] {
            if let Some(rest) = line.strip_prefix(prefix) {
                view.name = Some(rest.trim_end_matches(';').trim().to_owned());
                kind.clone_into(&mut view.kind);
                break;
            }
        }
        if line.starts_with("options") {
            view.options = braced_list(line);
        } else if let Some(rest) = line.strip_prefix("import ") {
            view.imports = rest
                .trim_end_matches(';')
                .split(',')
                .map(|one| one.trim().to_owned())
                .filter(|one| !one.is_empty())
                .collect();
        } else if line.starts_with("tokens") {
            view.tokens = braced_list(line);
        } else if line.starts_with("channels") {
            view.channels = braced_list(line);
        }
    }

    // A rule is `name : body ;`, and the name may sit on its own line
    // above the colon. So the search is over the whole cleaned text.
    let body = cleaned(text);
    let mut referenced: Vec<String> = Vec::new();
    let mut from = 0usize;
    while let Some(at) = body[from..].find(':') {
        let colon = from + at;
        let Some(end) = body[colon..].find(';') else {
            break;
        };
        let end = colon + end;

        let before = &body[from..colon];
        // The last *non-empty* identifier: a name on its own line above the
        // colon leaves trailing whitespace, and `next_back` on the raw
        // split hands back the empty string after it.
        let name = before
            .rsplit(|c: char| !c.is_alphanumeric() && c != '_')
            .find(|word| !word.is_empty())
            .unwrap_or("");
        from = end + 1;

        if name.is_empty() || !name.chars().next().is_some_and(char::is_alphabetic) {
            continue;
        }
        let alternatives = &body[colon + 1..end];
        view.rules.push(Rule {
            name: name.to_owned(),
            alternatives: alternatives.matches('|').count() + 1,
            lexer: name.chars().next().is_some_and(char::is_uppercase),
            fragment: before.trim_end().ends_with(name)
                && before
                    .trim_end()
                    .trim_end_matches(name)
                    .trim_end()
                    .ends_with("fragment"),
        });
        for word in identifiers(alternatives) {
            if !referenced.contains(&word) {
                referenced.push(word);
            }
        }
    }

    let defined: Vec<&str> = view.rules.iter().map(|rule| rule.name.as_str()).collect();
    for name in referenced {
        // A reference to something neither defined here, declared as a
        // token, nor an `ANTLR` built-in, is a grammar that will not
        // generate.
        if !defined.contains(&name.as_str())
            && !view.tokens.contains(&name)
            && !BUILT_IN.contains(&name.as_str())
        {
            view.undefined.push(name);
        }
    }
    view
}

/// The names `ANTLR` gives meaning to itself, which a grammar need not
/// define.
const BUILT_IN: &[&str] = &[
    "EOF", "skip", "channel", "pushMode", "popMode", "mode", "more", "type", "options", "op",
];

/// Whether `text` is an `ANTLR` grammar.
fn looks_like_it(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = uncommented(line).trim();
        trimmed.starts_with("grammar ")
            || trimmed.starts_with("lexer grammar ")
            || trimmed.starts_with("parser grammar ")
    })
}

/// The ANTLR grammar plugin's core half.
#[derive(Debug, Default)]
pub struct AntlrCore;

impl PluginCore for AntlrCore {
    fn name(&self) -> &'static str {
        "antlr"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let mut view = parse(&content);
        view.content = content;
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The ANTLR grammar plugin's presentation half.
#[derive(Debug, Default)]
pub struct AntlrPresentation;

impl PluginPresentation for AntlrPresentation {
    fn name(&self) -> &'static str {
        "antlr"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "G4",
            tint: 0x00a9_3226,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: AntlrView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(name) = &view.name {
            lines.push(format!("{} grammar {name}", view.kind));
        }
        if !view.options.is_empty() {
            lines.push(format!("Options: {}", view.options.join(", ")));
        }
        if !view.imports.is_empty() {
            lines.push(format!("Imports: {}", view.imports.join(", ")));
        }
        if !view.tokens.is_empty() {
            lines.push(format!("Declared tokens: {}", view.tokens.join(", ")));
        }
        if !view.channels.is_empty() {
            lines.push(format!("Channels: {}", view.channels.join(", ")));
        }

        let parser: Vec<&Rule> = view.rules.iter().filter(|rule| !rule.lexer).collect();
        let lexer: Vec<&Rule> = view.rules.iter().filter(|rule| rule.lexer).collect();
        if !parser.is_empty() {
            lines.push(format!("Parser rules ({}):", parser.len()));
            for rule in parser {
                lines.push(format!(
                    "  {}  ({} alternative(s))",
                    rule.name, rule.alternatives
                ));
            }
        }
        if !lexer.is_empty() {
            lines.push(format!("Lexer rules ({}):", lexer.len()));
            for rule in lexer {
                let fragment = if rule.fragment { "fragment " } else { "" };
                lines.push(format!("  {fragment}{}", rule.name));
            }
        }
        if !view.undefined.is_empty() {
            lines.push(
                "Referenced but never defined, so this grammar will not generate:".to_owned(),
            );
            for name in &view.undefined {
                lines.push(format!("  {name}"));
            }
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{AntlrCore, AntlrPresentation, AntlrView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const GRAMMAR: &str = "grammar Expr;\n\
        options { language = Java; }\n\
        tokens { PLUS, MINUS }\n\
        channels { COMMENTS }\n\
        prog: stat+ EOF;\n\
        stat\n  : expr ';'\n  | ID '=' expr ';'\n  ;\n\
        expr: expr ('*' | '/') expr | INT | ID;\n\
        ID: [a-zA-Z_] [a-zA-Z0-9_]*;\n\
        INT: DIGIT+;\n\
        fragment DIGIT: [0-9];\n\
        WS: [ \\t\\r\\n]+ -> skip;\n";

    #[test]
    fn sniffs_a_grammar_declaration() {
        assert!(AntlrCore.sniff(b"grammar Expr;\n"));
        assert!(AntlrCore.sniff(b"lexer grammar ExprLexer;\n"));
        assert!(AntlrCore.sniff(b"parser grammar ExprParser;\n"));
    }

    #[test]
    fn does_not_claim_prose_or_other_languages() {
        assert!(!AntlrCore.sniff(b"the grammar of the sentence is wrong\n"));
        assert!(!AntlrCore.sniff(b"class A { }\n"));
        assert!(!AntlrCore.sniff(b""));
    }

    #[test]
    fn reads_the_grammar_kind() {
        assert_eq!(parse("grammar A;\n").kind, "combined");
        assert_eq!(parse("lexer grammar A;\n").kind, "lexer");
        assert_eq!(parse("parser grammar A;\n").kind, "parser");
        assert_eq!(parse("parser grammar A;\n").name.as_deref(), Some("A"));
    }

    #[test]
    fn a_rule_spanning_several_lines_is_one_rule() {
        let view = parse(GRAMMAR);

        let stat = view.rules.iter().find(|rule| rule.name == "stat").unwrap();
        assert_eq!(
            stat.alternatives, 2,
            "the `|` between the alternatives is what counts them"
        );
    }

    #[test]
    fn tells_a_lexer_rule_from_a_parser_rule_by_its_initial() {
        let view = parse(GRAMMAR);

        assert!(view.rules.iter().find(|r| r.name == "ID").unwrap().lexer);
        assert!(!view.rules.iter().find(|r| r.name == "prog").unwrap().lexer);
        assert!(
            view.rules
                .iter()
                .find(|r| r.name == "DIGIT")
                .unwrap()
                .fragment
        );
    }

    #[test]
    fn reads_options_tokens_and_channels() {
        let view = parse(GRAMMAR);

        assert_eq!(view.options, vec!["language = Java".to_owned()]);
        assert_eq!(view.tokens, vec!["PLUS".to_owned(), "MINUS".to_owned()]);
        assert_eq!(view.channels, vec!["COMMENTS".to_owned()]);
    }

    #[test]
    fn a_rule_referenced_but_never_defined_is_named() {
        let view = parse("grammar A;\nprog: missing EOF;\n");

        assert_eq!(view.undefined, vec!["missing".to_owned()]);

        let data = serde_json::to_value(&view).unwrap();
        let lines = AntlrPresentation.present(&data);

        assert!(lines.iter().any(|line| line.contains("will not generate")));
    }

    #[test]
    fn a_complete_grammar_names_nothing_undefined() {
        let view = parse(GRAMMAR);

        assert!(
            view.undefined.is_empty(),
            "everything is defined or declared: {:?}",
            view.undefined
        );
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/antlr/Expr.g4");

        let data = AntlrCore.view(&path).unwrap();
        let view: AntlrView = serde_json::from_value(data).unwrap();

        assert!(view.name.is_some());
        assert_eq!(view.kind, "combined");
        assert!(!view.options.is_empty());
        assert!(!view.tokens.is_empty());
        assert!(!view.channels.is_empty());
        assert!(!view.imports.is_empty());
        assert!(view.rules.len() >= 8);
        assert!(view.rules.iter().any(|rule| rule.lexer));
        assert!(view.rules.iter().any(|rule| !rule.lexer));
        assert!(view.rules.iter().any(|rule| rule.fragment));
        assert!(view.rules.iter().any(|rule| rule.alternatives > 1));
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::AntlrCore),
            plugin_api::PluginPresentation::extensions(&crate::AntlrPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
