//! Yacc grammar file type plugin: core and presentation halves.
//!
//! The `%%` separators that divide a Yacc file into its three sections,
//! with `%token` or `%%` declarations above the first - a shape nothing
//! else has.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["y", "yy", "ypp"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One grammar rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// The non-terminal it defines.
    pub name: String,
    /// How many alternatives it has.
    pub alternatives: usize,
    /// Whether any alternative carries an action block.
    pub has_action: bool,
    /// Whether any alternative is empty, which makes the rule optional
    /// and is the usual source of a shift/reduce conflict.
    pub has_empty: bool,
}

/// View data produced by [`YaccCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YaccView {
    /// The tokens declared with `%token`.
    pub tokens: Vec<String>,
    /// The start symbol, when `%start` names one.
    pub start: Option<String>,
    /// The precedence declarations, as `%left '+' '-'`.
    pub precedence: Vec<String>,
    /// The `%union` members, as declared.
    pub union_members: Vec<String>,
    /// The `%type` declarations.
    pub types: Vec<String>,
    /// The rules, in file order.
    pub rules: Vec<Rule>,
    /// Whether it declares an `error` rule, without which a parser stops
    /// at the first mistake.
    pub has_error_rule: bool,
    /// Non-terminals used on the right and never defined on the left.
    pub undefined: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The three sections a `%%` divides a file into.
fn sections(text: &str) -> (String, String, String) {
    let mut parts: Vec<Vec<&str>> = vec![Vec::new()];
    for line in text.lines() {
        if line.trim() == "%%" && parts.len() < 3 {
            parts.push(Vec::new());
        } else {
            parts
                .last_mut()
                .expect("there is always a current section")
                .push(line);
        }
    }
    while parts.len() < 3 {
        parts.push(Vec::new());
    }
    (
        parts[0].join("\n"),
        parts[1].join("\n"),
        parts[2].join("\n"),
    )
}

/// `text` with `{ ... }` action blocks, `'c'` literals and comments
/// removed, so what is left is grammar.
fn without_actions(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            '\'' => {
                for inner in chars.by_ref() {
                    if inner == '\'' {
                        break;
                    }
                }
                if depth == 0 {
                    out.push(' ');
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                let mut last = ' ';
                for inner in chars.by_ref() {
                    if last == '*' && inner == '/' {
                        break;
                    }
                    last = inner;
                }
            }
            other if depth == 0 => out.push(other),
            _ => {}
        }
    }
    out
}

/// The identifiers in `text`.
fn identifiers(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for word in text.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if !word.is_empty()
            && word
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
            && !found.contains(&word.to_owned())
        {
            found.push(word.to_owned());
        }
    }
    found
}

/// Everything [`YaccView`] holds, read from `text`.
fn parse(text: &str) -> YaccView {
    let (declarations, grammar, _epilogue) = sections(text);
    let mut view = YaccView {
        tokens: Vec::new(),
        start: None,
        precedence: Vec::new(),
        union_members: Vec::new(),
        types: Vec::new(),
        rules: Vec::new(),
        has_error_rule: false,
        undefined: Vec::new(),
        content: String::new(),
        truncated: false,
    };

    let mut in_union = false;
    for raw in declarations.lines() {
        let line = raw.trim();
        if line.starts_with("%union") {
            in_union = true;
            continue;
        }
        if in_union {
            if line.starts_with('}') {
                in_union = false;
            } else if !line.is_empty() && !line.starts_with('{') {
                view.union_members
                    .push(line.trim_end_matches(';').trim().to_owned());
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("%token") {
            // `%token <type> NAME NAME` - the angle-bracketed type is not
            // a token name.
            let rest = rest.split('>').next_back().unwrap_or(rest);
            view.tokens.extend(identifiers(rest));
        } else if let Some(rest) = line.strip_prefix("%start ") {
            view.start = Some(rest.trim().to_owned());
        } else if line.starts_with("%left")
            || line.starts_with("%right")
            || line.starts_with("%nonassoc")
        {
            view.precedence.push(line.to_owned());
        } else if let Some(rest) = line.strip_prefix("%type") {
            let rest = rest.split('>').next_back().unwrap_or(rest);
            view.types.extend(identifiers(rest));
        }
    }

    let body = without_actions(&grammar);
    view.has_error_rule = grammar.contains("error");

    // A rule is `name : alternatives ;`.
    let mut used: Vec<String> = Vec::new();
    let mut from = 0usize;
    while let Some(at) = body[from..].find(':') {
        let colon = from + at;
        let Some(end) = body[colon..].find(';') else {
            break;
        };
        let end = colon + end;
        let name = body[from..colon]
            .rsplit(|c: char| !c.is_alphanumeric() && c != '_')
            .find(|word| !word.is_empty())
            .unwrap_or("")
            .to_owned();
        let alternatives = &body[colon + 1..end];
        from = end + 1;

        if name.is_empty()
            || !name
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
        {
            continue;
        }
        // An alternative with nothing in it makes the rule optional.
        let has_empty = alternatives
            .split('|')
            .any(|one| one.split_whitespace().next().is_none());
        view.rules.push(Rule {
            name: name.clone(),
            alternatives: alternatives.matches('|').count() + 1,
            has_action: grammar.contains('{'),
            has_empty,
        });
        for word in identifiers(alternatives) {
            if !used.contains(&word) {
                used.push(word);
            }
        }
    }

    let defined: Vec<&str> = view.rules.iter().map(|rule| rule.name.as_str()).collect();
    for name in used {
        if !defined.contains(&name.as_str()) && !view.tokens.contains(&name) && name != "error" {
            view.undefined.push(name);
        }
    }
    view
}

/// Whether `text` is a Yacc grammar.
fn looks_like_it(text: &str) -> bool {
    let separators = text.lines().filter(|line| line.trim() == "%%").count();
    separators >= 1
        && (text.contains("%token") || text.contains("%start") || text.contains("%union"))
}

/// The Yacc grammar plugin's core half.
#[derive(Debug, Default)]
pub struct YaccCore;

impl PluginCore for YaccCore {
    fn name(&self) -> &'static str {
        "yacc"
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

/// The Yacc grammar plugin's presentation half.
#[derive(Debug, Default)]
pub struct YaccPresentation;

impl PluginPresentation for YaccPresentation {
    fn name(&self) -> &'static str {
        "yacc"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "YACC",
            tint: 0x0079_5548,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: YaccView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(start) = &view.start {
            lines.push(format!("Start symbol: {start}"));
        }
        if !view.tokens.is_empty() {
            lines.push(format!(
                "Tokens ({}): {}",
                view.tokens.len(),
                view.tokens.join(", ")
            ));
        }
        if !view.precedence.is_empty() {
            lines.push(format!(
                "Precedence, loosest first ({}):",
                view.precedence.len()
            ));
            for line in &view.precedence {
                lines.push(format!("  {line}"));
            }
        }
        if !view.union_members.is_empty() {
            lines.push(format!("Union members: {}", view.union_members.join("; ")));
        }
        if !view.types.is_empty() {
            lines.push(format!("Typed non-terminals: {}", view.types.join(", ")));
        }
        lines.push(format!("{} rule(s):", view.rules.len()));
        for rule in &view.rules {
            let empty = if rule.has_empty {
                ", one of them empty"
            } else {
                ""
            };
            lines.push(format!(
                "  {}  ({} alternative(s){empty})",
                rule.name, rule.alternatives
            ));
        }
        if !view.has_error_rule {
            lines.push(
                "No `error` rule, so the parser stops at the first mistake it meets.".to_owned(),
            );
        }
        if !view.undefined.is_empty() {
            lines.push("Used but never defined nor declared as a token:".to_owned());
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
    use super::{YaccCore, YaccPresentation, YaccView, parse, sections, without_actions};
    use plugin_api::{PluginCore, PluginPresentation};

    const GRAMMAR: &str = "%{\n#include <stdio.h>\n%}\n\
        %union {\n  int number;\n  char *name;\n}\n\
        %token <number> INT\n%token <name> ID\n%token PLUS MINUS\n\
        %left PLUS MINUS\n%left TIMES\n\
        %type <number> expr\n\
        %start prog\n\
        %%\n\
        prog: stmts ;\n\
        stmts: stmts stmt | ;\n\
        stmt: expr ';' { printf(\"%d\\n\", $1); }\n    | error ';'\n    ;\n\
        expr: expr PLUS expr | INT | ID ;\n\
        %%\n\
        int main(void) { return yyparse(); }\n";

    #[test]
    fn sniffs_a_separator_with_declarations() {
        assert!(YaccCore.sniff(GRAMMAR.as_bytes()));
    }

    #[test]
    fn does_not_claim_a_file_that_merely_has_percent_signs() {
        assert!(!YaccCore.sniff(b"100% done\n50% left\n"));
        assert!(!YaccCore.sniff(b""));
    }

    #[test]
    fn splits_the_three_sections() {
        let (declarations, grammar, epilogue) = sections(GRAMMAR);

        assert!(declarations.contains("%token"));
        assert!(grammar.contains("prog:"));
        assert!(epilogue.contains("yyparse"));
    }

    #[test]
    fn an_action_block_is_not_grammar() {
        // `printf` and `%d` are C, and would otherwise read as symbols
        // the grammar uses and never defines.
        let cleaned = without_actions("stmt: expr { printf(\"%d\", $1); } ;");

        assert!(!cleaned.contains("printf"));
        assert!(cleaned.contains("expr"));
    }

    #[test]
    fn reads_the_declarations() {
        let view = parse(GRAMMAR);

        assert_eq!(view.start.as_deref(), Some("prog"));
        assert!(view.tokens.contains(&"INT".to_owned()));
        assert!(
            !view.tokens.contains(&"number".to_owned()),
            "the angle-bracketed type is not a token name"
        );
        assert_eq!(view.precedence.len(), 2);
        assert_eq!(view.union_members.len(), 2);
        assert_eq!(view.types, vec!["expr".to_owned()]);
    }

    #[test]
    fn an_empty_alternative_makes_a_rule_optional() {
        let view = parse(GRAMMAR);

        let stmts = view.rules.iter().find(|rule| rule.name == "stmts").unwrap();
        assert!(stmts.has_empty, "`stmts: stmts stmt | ;` can match nothing");

        let expr = view.rules.iter().find(|rule| rule.name == "expr").unwrap();
        assert!(!expr.has_empty);
    }

    #[test]
    fn notices_the_error_rule() {
        assert!(parse(GRAMMAR).has_error_rule);

        let without = parse("%token A\n%%\nprog: A ;\n");
        assert!(!without.has_error_rule);

        let data = serde_json::to_value(without).unwrap();
        let lines = YaccPresentation.present(&data);
        assert!(lines.iter().any(|line| line.contains("first mistake")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/yacc/expression.y");

        let data = YaccCore.view(&path).unwrap();
        let view: YaccView = serde_json::from_value(data).unwrap();

        assert!(view.start.is_some());
        assert!(view.tokens.len() >= 4);
        assert!(view.precedence.len() >= 2);
        assert!(view.union_members.len() >= 2);
        assert!(!view.types.is_empty());
        assert!(view.rules.len() >= 4);
        assert!(view.rules.iter().any(|rule| rule.has_empty));
        assert!(view.rules.iter().any(|rule| rule.alternatives > 1));
        assert!(view.has_error_rule);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::YaccCore),
            plugin_api::PluginPresentation::extensions(&crate::YaccPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
