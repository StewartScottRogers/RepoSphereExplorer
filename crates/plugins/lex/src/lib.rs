//! Lex scanner file type plugin: core and presentation halves.
//!
//! The same `%%` structure Yacc has, told apart by what fills it: a Lex
//! file's rules section is pattern-action pairs with no `:` and no `;`,
//! and it carries `%option` or `%x` declarations Yacc never writes.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["l", "ll", "lpp"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One pattern and what it does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pattern {
    /// The pattern, as written.
    pub pattern: String,
    /// The action, with its braces removed. Empty when the rule falls
    /// through to the next one.
    pub action: String,
    /// The start condition it applies in, when it names one.
    pub condition: Option<String>,
}

/// View data produced by [`LexCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LexView {
    /// The named definitions, as `name pattern`.
    pub definitions: Vec<String>,
    /// The `%option` settings.
    pub options: Vec<String>,
    /// The start conditions, whether exclusive or inclusive.
    pub start_conditions: Vec<String>,
    /// The pattern-action rules, in order. The first match of the longest
    /// length wins, which is why order is worth showing.
    pub patterns: Vec<Pattern>,
    /// Whether it has a catch-all rule. Without one, an unmatched
    /// character is echoed to standard output rather than reported.
    pub has_catch_all: bool,
    /// Definitions declared and never used, which are usually a rename
    /// that did not finish.
    pub unused_definitions: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The three sections a `%%` divides a file into.
fn sections(text: &str) -> (String, String) {
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
    while parts.len() < 2 {
        parts.push(Vec::new());
    }
    (parts[0].join("\n"), parts[1].join("\n"))
}

/// Splits a rule line into its pattern and its action.
///
/// The pattern runs to the first unescaped whitespace that is not inside
/// a bracket expression or a quoted string - which is the whole
/// difficulty, since `[ \t]+` contains a space that does not end it.
fn split_rule(line: &str) -> Option<(String, String)> {
    let chars: Vec<char> = line.chars().collect();
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for (index, &c) in chars.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '"' => quoted = !quoted,
            '[' if !quoted => depth += 1,
            ']' if !quoted => depth = depth.saturating_sub(1),
            c if c.is_whitespace() && depth == 0 && !quoted => {
                let pattern: String = chars[..index].iter().collect();
                let action: String = chars[index..].iter().collect();
                if pattern.is_empty() {
                    return None;
                }
                return Some((pattern, action.trim().to_owned()));
            }
            _ => {}
        }
    }
    // A pattern with no action at all still counts.
    let pattern: String = chars.iter().collect();
    (!pattern.trim().is_empty()).then(|| (pattern.trim().to_owned(), String::new()))
}

/// Everything [`LexView`] holds, read from `text`.
fn parse(text: &str) -> LexView {
    let (definitions_section, rules_section) = sections(text);
    let mut view = LexView {
        definitions: Vec::new(),
        options: Vec::new(),
        start_conditions: Vec::new(),
        patterns: Vec::new(),
        has_catch_all: false,
        unused_definitions: Vec::new(),
        content: String::new(),
        truncated: false,
    };

    let mut in_prologue = false;
    for raw in definitions_section.lines() {
        let line = raw.trim();
        if line == "%{" {
            in_prologue = true;
            continue;
        }
        if line == "%}" {
            in_prologue = false;
            continue;
        }
        if in_prologue || line.is_empty() || line.starts_with("/*") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("%option ") {
            view.options.extend(
                rest.split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
            );
            continue;
        }
        if let Some(rest) = line
            .strip_prefix("%x ")
            .or_else(|| line.strip_prefix("%s "))
        {
            view.start_conditions.extend(
                rest.split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
            );
            continue;
        }
        if line.starts_with('%') {
            continue;
        }
        // A definition is `NAME pattern`.
        if let Some((name, pattern)) = line.split_once(char::is_whitespace) {
            let name = name.trim();
            if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                view.definitions.push(format!("{name} {}", pattern.trim()));
            }
        }
    }

    let mut condition: Option<String> = None;
    for raw in rules_section.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("/*") || line == "%{" || line == "%}" {
            continue;
        }
        // `<CONDITION>pattern action`, or a `<CONDITION>{` block.
        let (here, rest) = match line.strip_prefix('<').and_then(|r| r.split_once('>')) {
            Some((name, rest)) => (Some(name.to_owned()), rest),
            None => (condition.clone(), line),
        };
        if rest.trim() == "{" {
            condition = here;
            continue;
        }
        if rest.trim() == "}" {
            condition = None;
            continue;
        }
        let Some((pattern, action)) = split_rule(rest.trim()) else {
            continue;
        };
        if pattern == "." || pattern == ".|\\n" {
            view.has_catch_all = true;
        }
        view.patterns.push(Pattern {
            pattern,
            action: action.trim_matches(['{', '}']).trim().to_owned(),
            condition: here,
        });
    }

    for definition in &view.definitions {
        let name = definition.split_whitespace().next().unwrap_or_default();
        let used = view
            .patterns
            .iter()
            .any(|rule| rule.pattern.contains(&format!("{{{name}}}")));
        if !used && !name.is_empty() {
            view.unused_definitions.push(name.to_owned());
        }
    }
    view
}

/// Whether `text` is a Lex scanner.
fn looks_like_it(text: &str) -> bool {
    let separators = text.lines().filter(|line| line.trim() == "%%").count();
    if separators == 0 {
        return false;
    }
    // Yacc's own markers, which this must not claim.
    if text.contains("%token") || text.contains("%start") || text.contains("%union") {
        return false;
    }
    text.contains("%option") || text.contains("yytext") || text.contains("%x ")
}

/// The Lex scanner plugin's core half.
#[derive(Debug, Default)]
pub struct LexCore;

impl PluginCore for LexCore {
    fn name(&self) -> &'static str {
        "lex"
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

/// The Lex scanner plugin's presentation half.
#[derive(Debug, Default)]
pub struct LexPresentation;

impl PluginPresentation for LexPresentation {
    fn name(&self) -> &'static str {
        "lex"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "LEX",
            tint: 0x0079_5548,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: LexView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.options.is_empty() {
            lines.push(format!("Options: {}", view.options.join(", ")));
        }
        if !view.start_conditions.is_empty() {
            lines.push(format!(
                "Start conditions: {}",
                view.start_conditions.join(", ")
            ));
        }
        if !view.definitions.is_empty() {
            lines.push(format!("Definitions ({}):", view.definitions.len()));
            for definition in &view.definitions {
                lines.push(format!("  {definition}"));
            }
        }
        lines.push(format!(
            "{} rule(s), the longest match winning and ties going to the first:",
            view.patterns.len()
        ));
        for rule in &view.patterns {
            let condition = rule
                .condition
                .as_ref()
                .map_or_else(String::new, |name| format!("<{name}> "));
            let action = if rule.action.is_empty() {
                "(falls through)".to_owned()
            } else {
                rule.action.clone()
            };
            lines.push(format!("  {condition}{}  ->  {action}", rule.pattern));
        }
        if !view.has_catch_all {
            lines.push(
                "No catch-all rule, so an unmatched character is echoed to standard".to_owned(),
            );
            lines.push("output rather than reported.".to_owned());
        }
        if !view.unused_definitions.is_empty() {
            lines.push(format!(
                "Defined and never used: {}",
                view.unused_definitions.join(", ")
            ));
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{LexCore, LexPresentation, LexView, parse, split_rule};
    use plugin_api::{PluginCore, PluginPresentation};

    const SCANNER: &str = "%{\n#include \"parser.h\"\n%}\n\
        %option noyywrap\n%option yylineno\n\
        %x COMMENT\n\
        DIGIT   [0-9]\n\
        LETTER  [a-zA-Z]\n\
        UNUSED  [!?]\n\
        %%\n\
        {DIGIT}+          { return NUMBER; }\n\
        {LETTER}+         { return IDENTIFIER; }\n\
        [ \\t\\r\\n]+       ;\n\
        \"/*\"             { BEGIN(COMMENT); }\n\
        <COMMENT>\"*/\"    { BEGIN(INITIAL); }\n\
        <COMMENT>.        ;\n\
        .                 { return yytext[0]; }\n\
        %%\n\
        int yywrap(void) { return 1; }\n";

    #[test]
    fn sniffs_a_scanner() {
        assert!(LexCore.sniff(SCANNER.as_bytes()));
    }

    #[test]
    fn leaves_a_yacc_grammar_to_its_own_plugin() {
        assert!(!LexCore.sniff(b"%token A\n%%\nprog: A ;\n%%\n"));
        assert!(!LexCore.sniff(b""));
    }

    #[test]
    fn a_space_inside_a_bracket_expression_does_not_end_the_pattern() {
        // `[ \t]+` holds a space, and splitting on the first whitespace
        // would cut the pattern in half.
        let (pattern, action) = split_rule("[ \\t]+       ;").unwrap();

        assert_eq!(pattern, "[ \\t]+");
        assert_eq!(action, ";");
    }

    #[test]
    fn reads_options_conditions_and_definitions() {
        let view = parse(SCANNER);

        assert_eq!(
            view.options,
            vec!["noyywrap".to_owned(), "yylineno".to_owned()]
        );
        assert_eq!(view.start_conditions, vec!["COMMENT".to_owned()]);
        assert_eq!(view.definitions.len(), 3);
    }

    #[test]
    fn a_rule_carries_the_start_condition_it_applies_in() {
        let view = parse(SCANNER);

        let conditional: Vec<_> = view
            .patterns
            .iter()
            .filter(|rule| rule.condition.as_deref() == Some("COMMENT"))
            .collect();

        assert_eq!(conditional.len(), 2);
    }

    #[test]
    fn notices_the_catch_all_rule() {
        assert!(parse(SCANNER).has_catch_all);

        let without = parse("%option noyywrap\n%%\n[0-9]+ { return NUMBER; }\n");
        assert!(!without.has_catch_all);

        let data = serde_json::to_value(without).unwrap();
        let lines = LexPresentation.present(&data);
        assert!(lines.iter().any(|line| line.contains("echoed to standard")));
    }

    #[test]
    fn names_a_definition_nothing_uses() {
        let view = parse(SCANNER);

        assert_eq!(view.unused_definitions, vec!["UNUSED".to_owned()]);
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/lex/tokens.l");

        let data = LexCore.view(&path).unwrap();
        let view: LexView = serde_json::from_value(data).unwrap();

        assert!(view.options.len() >= 2);
        assert!(!view.start_conditions.is_empty());
        assert!(view.definitions.len() >= 3);
        assert!(view.patterns.len() >= 6);
        assert!(view.patterns.iter().any(|rule| rule.condition.is_some()));
        assert!(view.has_catch_all);
        assert!(!view.unused_definitions.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::LexCore),
            plugin_api::PluginPresentation::extensions(&crate::LexPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
