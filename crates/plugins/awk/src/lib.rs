//! AWK file type plugin: core and presentation halves.
//!
//! An awk script is a BEGIN block, some pattern-action rules and an END
//! block. This reads those, the functions with their parameters, the
//! field separator, the highest field number the script reads, the
//! arrays - and the names used inside a function that are not among its
//! parameters, which in awk means they are global.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["awk"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One pattern-action rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// The pattern, or `every line` when the rule has none.
    pub pattern: String,
    /// Whether the rule has an action of its own, or falls back to
    /// printing the line.
    pub has_action: bool,
}

/// View data produced by [`AwkCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwkView {
    /// How many `BEGIN` blocks the script has.
    pub begin_blocks: usize,
    /// How many `END` blocks it has.
    pub end_blocks: usize,
    /// Every pattern-action rule, in order.
    pub rules: Vec<Rule>,
    /// The functions defined, with their parameters.
    pub functions: Vec<String>,
    /// The field separator, when the script sets one.
    pub field_separator: Option<String>,
    /// The highest field number the script refers to, which says how many
    /// columns it expects.
    pub highest_field: Option<usize>,
    /// The arrays it uses, by name.
    pub arrays: Vec<String>,
    /// Names used in a function body that the function does not declare,
    /// and so are global whether that was meant or not.
    pub globals_in_functions: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The variables awk sets itself, which are global on purpose.
const BUILT_IN: &[&str] = &[
    "NR", "NF", "FS", "OFS", "RS", "ORS", "FILENAME", "FNR", "SUBSEP", "RSTART", "RLENGTH",
    "CONVFMT", "OFMT", "ENVIRON", "ARGC", "ARGV",
];

/// `line` with its comment stripped.
///
/// A `#` inside a string or a regular expression is not a comment, and a
/// script that splits on `#` would lose half of `print "#" $1`.
fn cleaned(line: &str) -> &str {
    let mut in_string = false;
    let mut in_regex = false;
    let mut escaped = false;
    for (at, letter) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match letter {
            '\\' => escaped = true,
            '"' if !in_regex => in_string = !in_string,
            '/' if !in_string => in_regex = !in_regex,
            '#' if !in_string && !in_regex => return &line[..at],
            _ => {}
        }
    }
    line
}

/// The parameters of a `function name(a, b)` line.
fn function_of(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("function ")?;
    let name = rest.split(['(', ' ']).next()?.trim();
    if name.is_empty() {
        return None;
    }
    let parameters = rest
        .split_once('(')
        .and_then(|(_, tail)| tail.split(')').next())
        .unwrap_or("")
        .trim()
        .to_owned();
    Some(format!("{name}({parameters})"))
}

/// The rule `line` opens, if it opens one.
fn rule_of(line: &str) -> Option<Rule> {
    let trimmed = line.trim();
    if trimmed.starts_with("function ") || trimmed.starts_with('}') {
        return None;
    }
    let (pattern, has_action) = match trimmed.find('{') {
        Some(at) => (trimmed[..at].trim(), true),
        None => (trimmed, false),
    };
    if pattern.is_empty() && !has_action {
        return None;
    }
    // A bare `{ ... }` acts on every line; `BEGIN` and `END` are not
    // rules and are counted separately.
    if pattern == "BEGIN" || pattern == "END" {
        return None;
    }
    // An action's body is not a rule: a rule's pattern has to look like
    // one, which means it is empty, a regular expression, or an
    // expression mentioning a field or a built-in.
    let looks_like_a_pattern = pattern.is_empty()
        || pattern.starts_with('/')
        || pattern.contains('$')
        || BUILT_IN.iter().any(|name| pattern.contains(name));
    if !looks_like_a_pattern {
        return None;
    }
    Some(Rule {
        pattern: if pattern.is_empty() {
            "every line".to_owned()
        } else {
            pattern.to_owned()
        },
        has_action,
    })
}

/// The highest `$n` field number `text` mentions.
fn highest_field(text: &str) -> Option<usize> {
    let mut highest = None;
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        let digits: String = rest[at + 1..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if let Ok(number) = digits.parse::<usize>() {
            highest = Some(highest.map_or(number, |seen: usize| seen.max(number)));
        }
        rest = &rest[at + 1..];
    }
    highest
}

/// Reads a field separator, and any array subscript, off `line`.
fn read_separator_and_arrays(line: &str, view: &mut AwkView) {
    if view.field_separator.is_none()
        && let Some(at) = line.find("FS")
        && let Some(value) = line[at..].split('=').nth(1)
    {
        let value = value.trim().trim_end_matches(';').trim();
        if !value.is_empty() && value != "=" {
            view.field_separator = Some(value.to_owned());
        }
    }
    for opened in line.match_indices('[') {
        let before = line[..opened.0].trim_end();
        let Some(name) = before
            .rsplit(|letter: char| !letter.is_alphanumeric() && letter != '_')
            .next()
        else {
            continue;
        };
        if !name.is_empty()
            && !name
                .chars()
                .next()
                .is_some_and(|first| first.is_ascii_digit())
            && !BUILT_IN.contains(&name)
            && !view.arrays.contains(&name.to_owned())
        {
            view.arrays.push(name.to_owned());
        }
    }
}

/// Everything [`AwkView`] holds, read from `text`.
fn parse(text: &str) -> AwkView {
    let mut view = AwkView {
        begin_blocks: 0,
        end_blocks: 0,
        rules: Vec::new(),
        functions: Vec::new(),
        field_separator: None,
        highest_field: None,
        arrays: Vec::new(),
        globals_in_functions: Vec::new(),
        truncated: false,
    };
    // Brace depth, so the body of a rule is not read as more rules, and
    // the parameters of the function currently open.
    let mut depth = 0usize;
    let mut declared: Option<Vec<String>> = None;
    let mut used_in_function: Vec<String> = Vec::new();
    let mut whole = String::new();

    for raw in text.lines() {
        let line = cleaned(raw).trim();
        if line.is_empty() {
            continue;
        }
        whole.push_str(line);
        whole.push('\n');

        read_separator_and_arrays(line, &mut view);

        if depth == 0 {
            if line.starts_with("BEGIN") {
                view.begin_blocks += 1;
            } else if line.starts_with("END") {
                view.end_blocks += 1;
            } else if let Some(function) = function_of(line) {
                view.functions.push(function.clone());
                declared = Some(
                    function
                        .split_once('(')
                        .map(|(_, tail)| {
                            tail.trim_end_matches(')')
                                .split(',')
                                .map(|name| name.trim().to_owned())
                                .filter(|name| !name.is_empty())
                                .collect()
                        })
                        .unwrap_or_default(),
                );
                used_in_function.clear();
            } else if let Some(rule) = rule_of(line) {
                view.rules.push(rule);
            }
        } else if declared.is_some() {
            for word in line.split(|letter: char| !letter.is_alphanumeric() && letter != '_') {
                if !word.is_empty()
                    && !word
                        .chars()
                        .next()
                        .is_some_and(|first| first.is_ascii_digit())
                    && !used_in_function.contains(&word.to_owned())
                {
                    used_in_function.push(word.to_owned());
                }
            }
        }

        let was = depth;
        depth = depth + line.matches('{').count()
            - line
                .matches('}')
                .count()
                .min(depth + line.matches('{').count());
        if was > 0
            && depth == 0
            && let Some(parameters) = declared.take()
        {
            for name in &used_in_function {
                if !parameters.contains(name)
                    && !BUILT_IN.contains(&name.as_str())
                    && !KEYWORDS.contains(&name.as_str())
                    && !view
                        .functions
                        .iter()
                        .any(|f| f.starts_with(&format!("{name}(")))
                    && !view.globals_in_functions.contains(name)
                {
                    view.globals_in_functions.push(name.clone());
                }
            }
            used_in_function.clear();
        }
    }
    view.highest_field = highest_field(&whole);
    view
}

/// The words awk has of its own, which are not variable names.
const KEYWORDS: &[&str] = &[
    "print", "printf", "if", "else", "while", "for", "do", "break", "continue", "next", "nextfile",
    "exit", "return", "delete", "getline", "in", "function", "length", "substr", "index", "split",
    "sub", "gsub", "match", "sprintf", "sin", "cos", "atan2", "exp", "log", "sqrt", "int", "rand",
    "srand", "tolower", "toupper", "system", "close", "fflush", "asort", "asorti",
];

/// Whether `text` is an awk script.
fn looks_like_it(text: &str) -> bool {
    if text.starts_with("#!") && text.lines().next().is_some_and(|line| line.contains("awk")) {
        return true;
    }
    let view = parse(text);
    // A BEGIN or END block, or a pattern-action rule with a field
    // reference in it, is awk and nothing else in this registry.
    (view.begin_blocks > 0 || view.end_blocks > 0)
        && (!view.rules.is_empty() || !view.functions.is_empty() || view.highest_field.is_some())
}

/// The AWK plugin's core half.
#[derive(Debug, Default)]
pub struct AwkCore;

impl PluginCore for AwkCore {
    fn name(&self) -> &'static str {
        "awk"
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
        // The rules and functions are the whole of the script's shape,
        // and each is on the view already.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The AWK plugin's presentation half.
#[derive(Debug, Default)]
pub struct AwkPresentation;

impl PluginPresentation for AwkPresentation {
    fn name(&self) -> &'static str {
        "awk"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "AWK",
            tint: 0x0044_7a3f,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: AwkView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "awk script: {} BEGIN, {} END, {} rule(s)",
            view.begin_blocks,
            view.end_blocks,
            view.rules.len()
        ));
        if let Some(separator) = &view.field_separator {
            lines.push(format!("Field separator: {separator}"));
        }
        if let Some(highest) = view.highest_field {
            lines.push(format!("Reads up to field ${highest}"));
        }
        for rule in &view.rules {
            let action = if rule.has_action {
                String::new()
            } else {
                " - prints the line".to_owned()
            };
            lines.push(format!("  {}{action}", rule.pattern));
        }
        if !view.functions.is_empty() {
            lines.push(format!("{} function(s):", view.functions.len()));
            for function in &view.functions {
                lines.push(format!("  {function}"));
            }
        }
        if !view.arrays.is_empty() {
            lines.push(format!("Arrays: {}", view.arrays.join(", ")));
        }
        if !view.globals_in_functions.is_empty() {
            lines.push("Used inside a function without being one of its".to_owned());
            lines.push("parameters, so these are global and survive the call:".to_owned());
            for name in &view.globals_in_functions {
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
    use super::{AwkCore, AwkPresentation, AwkView, cleaned, highest_field, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const SCRIPT: &str = concat!(
        "#!/usr/bin/awk -f\n",
        "# Summary statistics for a column.\n",
        "\n",
        "BEGIN {\n",
        "    FS = \",\"\n",
        "    count = 0\n",
        "}\n",
        "\n",
        "/^#/ { next }\n",
        "\n",
        "NF >= 3 {\n",
        "    total += $2\n",
        "    seen[$1] = $3\n",
        "    count++\n",
        "}\n",
        "\n",
        "$1 == \"skip\"\n",
        "\n",
        "function average(sum, n,    result) {\n",
        "    result = sum / n\n",
        "    leaked = result\n",
        "    return result\n",
        "}\n",
        "\n",
        "END {\n",
        "    printf \"%d rows, mean %.3f\\n\", count, average(total, count)\n",
        "}\n",
    );

    #[test]
    fn sniffs_a_script() {
        assert!(AwkCore.sniff(SCRIPT.as_bytes()));
    }

    #[test]
    fn does_not_claim_a_shell_script() {
        assert!(!AwkCore.sniff(b"#!/bin/sh\nfor f in *; do echo $f; done\n"));
        assert!(!AwkCore.sniff(b""));
    }

    #[test]
    fn a_hash_inside_a_string_or_a_pattern_is_not_a_comment() {
        assert_eq!(
            cleaned("print \"#\" $1  # a real comment").trim(),
            "print \"#\" $1"
        );
        assert_eq!(cleaned("/^#/ { next }").trim(), "/^#/ { next }");
    }

    #[test]
    fn counts_the_begin_and_end_blocks() {
        let view = parse(SCRIPT);

        assert_eq!(view.begin_blocks, 1);
        assert_eq!(view.end_blocks, 1);
        assert_eq!(view.field_separator.as_deref(), Some("\",\""));
    }

    #[test]
    fn a_rule_body_is_not_another_rule() {
        let view = parse(SCRIPT);

        assert_eq!(
            view.rules.len(),
            3,
            "`total += $2` and `count++` are inside a rule, not rules of their own"
        );
        assert!(view.rules.iter().any(|rule| rule.pattern == "/^#/"));
        assert!(view.rules.iter().any(|rule| !rule.has_action));
    }

    #[test]
    fn reads_the_function_and_its_array() {
        let view = parse(SCRIPT);

        assert_eq!(
            view.functions,
            vec!["average(sum, n,    result)".to_owned()]
        );
        assert_eq!(view.arrays, vec!["seen".to_owned()]);
    }

    #[test]
    fn finds_the_highest_field_the_script_reads() {
        assert_eq!(highest_field("$1 $3 $2"), Some(3));
        assert_eq!(highest_field("no fields here"), None);
        assert_eq!(parse(SCRIPT).highest_field, Some(3));
    }

    #[test]
    fn names_what_leaks_out_of_a_function() {
        let view = parse(SCRIPT);

        assert_eq!(
            view.globals_in_functions,
            vec!["leaked".to_owned()],
            "`result` is a parameter, which is how awk spells a local"
        );
    }

    #[test]
    fn presents_the_leak_with_its_reason() {
        let data = serde_json::to_value(parse(SCRIPT)).unwrap();

        let lines = AwkPresentation.present(&data);

        assert!(lines[0].starts_with("awk script: 1 BEGIN, 1 END"));
        assert!(lines.iter().any(|line| line.contains("survive the call")));
        assert!(lines.iter().any(|line| line.contains("prints the line")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/awk/csvstats.awk");

        let data = AwkCore.view(&path).unwrap();
        let view: AwkView = serde_json::from_value(data).unwrap();

        assert_eq!(view.begin_blocks, 1);
        assert_eq!(view.end_blocks, 1);
        assert!(view.rules.len() >= 3);
        assert!(view.rules.iter().any(|rule| !rule.has_action));
        assert!(view.functions.len() >= 2);
        assert!(view.field_separator.is_some());
        assert!(view.highest_field.is_some_and(|field| field >= 3));
        assert!(!view.arrays.is_empty());
        assert!(!view.globals_in_functions.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::AwkCore),
            plugin_api::PluginPresentation::extensions(&crate::AwkPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
