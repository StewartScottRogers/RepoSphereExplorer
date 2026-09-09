//! Sass file type plugin: core and presentation halves.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["scss", "sass"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// Which of Sass's two surface syntaxes a file uses: the braced, CSS-like
/// `.scss` syntax, or the indentation-based `.sass` one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Syntax {
    /// The braced, semicolon-terminated syntax (`.scss`).
    Scss,
    /// The indentation-based syntax with no braces or semicolons (`.sass`).
    Sass,
}

/// A `@mixin` or `@function` declaration, with the parameters it takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    /// The mixin's or function's name.
    pub name: String,
    /// Its parameters, in declaration order, with any default value
    /// dropped - just the `$name` each is called with.
    pub parameters: Vec<String>,
}

/// View data produced by [`SassCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SassView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Which surface syntax the file is written in.
    pub syntax: Syntax,
    /// Names of top-level `$variable:` declarations found in the content.
    pub variables: Vec<String>,
    /// `@mixin` declarations found in the content, with their parameters.
    pub mixins: Vec<Definition>,
    /// `@function` declarations found in the content, with their
    /// parameters.
    pub functions: Vec<Definition>,
    /// The module paths named by `@use` rules.
    pub uses: Vec<String>,
    /// The module paths named by `@forward` rules.
    pub forwards: Vec<String>,
    /// Names of `%placeholder` selectors declared in the content, without
    /// their leading `%`.
    pub placeholders: Vec<String>,
    /// The deepest level of selector nesting the content reaches: brace
    /// depth in the braced syntax, indent steps of two spaces - the
    /// syntax's own documented indent width - in the indented one.
    pub nesting_depth: usize,
}

/// Whether a trimmed line is a `$variable:` declaration, e.g. `$size: 16px;`
/// or, in the indented syntax, `$size: 16px`.
fn parse_variable_declaration(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix('$')?;
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '-'))
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    rest[end..]
        .trim_start()
        .starts_with(':')
        .then(|| rest[..end].to_owned())
}

/// Parses top-level `$variable:` declarations out of `content`, in source
/// order.
fn parse_variables(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(parse_variable_declaration)
        .collect()
}

/// Parses a `@mixin name(...)` or `@function name(...)` declaration line,
/// dropping each parameter's default value and keeping just its `$name`.
fn parse_definition_line(line: &str, keyword: &str) -> Option<Definition> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix(keyword)?.trim_start();
    let name_end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '-'))
        .unwrap_or(rest.len());
    if name_end == 0 {
        return None;
    }
    let name = rest[..name_end].to_owned();
    let after_name = &rest[name_end..];
    let parameters = after_name
        .find('(')
        .and_then(|open| {
            let after_open = &after_name[open + 1..];
            after_open.find(')').map(|close| {
                after_open[..close]
                    .split(',')
                    .map(str::trim)
                    .filter(|param| !param.is_empty())
                    .map(|param| param.split(':').next().unwrap_or(param).trim().to_owned())
                    .collect()
            })
        })
        .unwrap_or_default();
    Some(Definition { name, parameters })
}

/// Parses `@mixin` and `@function` declarations out of `content`, in source
/// order.
fn parse_mixins_and_functions(content: &str) -> (Vec<Definition>, Vec<Definition>) {
    let mut mixins = Vec::new();
    let mut functions = Vec::new();
    for line in content.lines() {
        if let Some(def) = parse_definition_line(line, "@mixin") {
            mixins.push(def);
        } else if let Some(def) = parse_definition_line(line, "@function") {
            functions.push(def);
        }
    }
    (mixins, functions)
}

/// Parses the quoted target of an `@use "..."` or `@forward '...'` line,
/// if `line` is one.
fn parse_quoted_target(line: &str, keyword: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix(keyword)?.trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &rest[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_owned())
}

/// Parses every `@use`/`@forward` target out of `content`, in source order.
fn parse_quoted_targets(content: &str, keyword: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| parse_quoted_target(line, keyword))
        .collect()
}

/// Whether a trimmed line declares a `%placeholder` selector, e.g.
/// `%button-base {` or, in the indented syntax, `%button-base`.
fn parse_placeholder(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix('%')?;
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '-'))
        .unwrap_or(rest.len());
    (end > 0).then(|| rest[..end].to_owned())
}

/// Parses `%placeholder` selector declarations out of `content`, in source
/// order.
fn parse_placeholders(content: &str) -> Vec<String> {
    content.lines().filter_map(parse_placeholder).collect()
}

/// The deepest brace nesting reached in the braced syntax.
fn brace_nesting_depth(content: &str) -> usize {
    let mut depth = 0usize;
    let mut max_depth = 0usize;
    for ch in content.chars() {
        match ch {
            '{' => {
                depth += 1;
                max_depth = max_depth.max(depth);
            }
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max_depth
}

/// The deepest indent step reached in the indented syntax, counting each
/// two spaces of leading whitespace as one level.
fn indented_nesting_depth(content: &str) -> usize {
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.chars().take_while(|ch| *ch == ' ').count() / 2)
        .max()
        .unwrap_or(0)
}

/// The deepest level of selector nesting `content` reaches, in whichever
/// of the two syntaxes `syntax` names.
fn nesting_depth(content: &str, syntax: Syntax) -> usize {
    match syntax {
        Syntax::Scss => brace_nesting_depth(content),
        Syntax::Sass => indented_nesting_depth(content),
    }
}

/// Which surface syntax `content` is written in. The braced syntax always
/// has at least one `{`; the indented syntax never does.
fn detect_syntax(content: &str) -> Syntax {
    if content.contains('{') {
        Syntax::Scss
    } else {
        Syntax::Sass
    }
}

/// Whether a trimmed line opens with a parent-selector reference in a form
/// that only Sass uses - `&:hover`, `&.active`, `&--modifier`, `&,` - as
/// opposed to a bare `&` or `&&`, which plenty of other languages also
/// write at a line's start.
fn has_parent_selector_reference(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("&:")
            || trimmed.starts_with("&.")
            || trimmed.starts_with("&-")
            || trimmed.starts_with("&,")
    })
}

/// Whether `text` looks like Sass source: `$variable:` declarations,
/// `@mixin`/`@include`, `@use`/`@forward`, or a Sass-only parent-selector
/// reference. None of these are markers this project's other
/// source-language plugins sniff for, so this plugin has no ordering
/// dependency on a sibling.
fn has_sass_syntax(text: &str) -> bool {
    text.lines()
        .any(|line| parse_variable_declaration(line).is_some())
        || text.contains("@mixin ")
        || text.contains("@include ")
        || text.contains("@use \"")
        || text.contains("@use '")
        || text.contains("@forward \"")
        || text.contains("@forward '")
        || has_parent_selector_reference(text)
}

/// The Sass plugin's core half.
#[derive(Debug, Default)]
pub struct SassCore;

impl PluginCore for SassCore {
    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn name(&self) -> &'static str {
        "sass"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_sass_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let syntax = detect_syntax(&content);
        let variables = parse_variables(&content);
        let (mixins, functions) = parse_mixins_and_functions(&content);
        let uses = parse_quoted_targets(&content, "@use");
        let forwards = parse_quoted_targets(&content, "@forward");
        let placeholders = parse_placeholders(&content);
        let nesting_depth = nesting_depth(&content, syntax);
        let view = SassView {
            content,
            truncated,
            syntax,
            variables,
            mixins,
            functions,
            uses,
            forwards,
            placeholders,
            nesting_depth,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// Renders a [`Definition`] as `name` or `name(params, ...)`.
fn format_definition(definition: &Definition) -> String {
    if definition.parameters.is_empty() {
        definition.name.clone()
    } else {
        format!("{}({})", definition.name, definition.parameters.join(", "))
    }
}

/// The Sass plugin's presentation half.
#[derive(Debug, Default)]
pub struct SassPresentation;

impl PluginPresentation for SassPresentation {
    fn name(&self) -> &'static str {
        "sass"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "SASS",
            tint: 0x00cf_649a,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: SassView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "syntax: {}",
            match view.syntax {
                Syntax::Scss => "scss",
                Syntax::Sass => "sass (indented)",
            }
        ));
        if !view.variables.is_empty() {
            lines.push(format!("variables: {}", view.variables.join(", ")));
        }
        if !view.mixins.is_empty() {
            lines.push(format!(
                "mixins: {}",
                view.mixins
                    .iter()
                    .map(format_definition)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !view.functions.is_empty() {
            lines.push(format!(
                "functions: {}",
                view.functions
                    .iter()
                    .map(format_definition)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !view.uses.is_empty() {
            lines.push(format!("@use: {}", view.uses.join(", ")));
        }
        if !view.forwards.is_empty() {
            lines.push(format!("@forward: {}", view.forwards.join(", ")));
        }
        if !view.placeholders.is_empty() {
            lines.push(format!(
                "placeholders: {}",
                view.placeholders
                    .iter()
                    .map(|name| format!("%{name}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        lines.push(format!("nesting depth: {}", view.nesting_depth));
        lines.extend(view.content.lines().map(str::to_owned));
        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Definition, MAX_VIEW_BYTES, SassCore, SassPresentation, SassView, Syntax,
        parse_definition_line, parse_variable_declaration,
    };
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-sass-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_scss_markers_as_sass() {
        assert!(SassCore.sniff(b"$size: 16px;\n\n.box {\n  width: $size;\n}\n"));
        assert!(SassCore.sniff(b"@mixin center {\n  display: flex;\n}\n"));
        assert!(SassCore.sniff(b".box {\n  @include center;\n}\n"));
        assert!(SassCore.sniff(b"@use \"sass:math\";\n"));
        assert!(SassCore.sniff(b"@forward 'buttons';\n"));
        assert!(SassCore.sniff(b".button {\n  &:hover {\n    color: red;\n  }\n}\n"));
        assert!(SassCore.sniff(b".button {\n  &.active {\n    color: blue;\n  }\n}\n"));
    }

    #[test]
    fn sniffs_indented_syntax_markers_as_sass() {
        assert!(SassCore.sniff(b"$size: 16px\n\n.box\n  width: $size\n"));
        assert!(SassCore.sniff(b"@mixin center\n  display: flex\n"));
        assert!(SassCore.sniff(b".button\n  &:hover\n    color: red\n"));
    }

    #[test]
    fn does_not_sniff_near_miss_languages_as_sass() {
        // Plain CSS: no variables, mixins, `@use`/`@forward`, or parent
        // selectors - just what every plugin in this project's registry
        // would otherwise be free to claim if the markers above were any
        // looser.
        assert!(!SassCore.sniff(b".box {\n  width: 16px;\n  color: red;\n}\n"));
        // PHP assigns with `=`, not `$name:`.
        assert!(!SassCore.sniff(b"<?php\n$name = 'eve';\necho $name;\n"));
        // Perl's ternary places `:` after an operand, never right after the
        // variable name.
        assert!(!SassCore.sniff(b"my $label = $ok ? 'yes' : 'no';\n"));
        // Dart interpolates a variable but never declares one with `$name:`.
        assert!(!SassCore.sniff(b"var name = 'eve';\nprint('hi $name');\n"));
        // Rust's `&` is a reference, never a parent-selector line.
        assert!(!SassCore.sniff(b"pub fn greet(name: &str) -> String {\n  name.to_string()\n}\n"));
        assert!(!SassCore.sniff(b"just a regular line of text\n"));
        assert!(!SassCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn parses_a_variable_declaration_but_not_a_lookalike() {
        assert_eq!(
            parse_variable_declaration("$primary-color: #333;"),
            Some("primary-color".to_owned())
        );
        assert_eq!(parse_variable_declaration("$name = 'eve';"), None);
        assert_eq!(parse_variable_declaration("width: $size;"), None);
    }

    #[test]
    fn parses_a_definition_line_with_and_without_parameters() {
        assert_eq!(
            parse_definition_line("@mixin center {", "@mixin"),
            Some(Definition {
                name: "center".to_owned(),
                parameters: Vec::new(),
            })
        );
        assert_eq!(
            parse_definition_line("@mixin button($color, $radius: 4px) {", "@mixin"),
            Some(Definition {
                name: "button".to_owned(),
                parameters: vec!["$color".to_owned(), "$radius".to_owned()],
            })
        );
        assert_eq!(
            parse_definition_line("@include button($color);", "@mixin"),
            None
        );
    }

    #[test]
    fn views_a_real_scss_file_and_extracts_every_field() {
        let path = unique_temp_file("_buttons.scss");
        std::fs::write(
            &path,
            concat!(
                "@use \"sass:math\";\n",
                "@forward \"tokens\";\n\n",
                "$spacing: 8px;\n\n",
                "@function double($value) {\n",
                "  @return $value * 2;\n",
                "}\n\n",
                "%reset {\n",
                "  margin: 0;\n",
                "}\n\n",
                "@mixin button($color, $radius: 4px) {\n",
                "  color: $color;\n",
                "  border-radius: $radius;\n",
                "}\n\n",
                ".button {\n",
                "  @extend %reset;\n",
                "  @include button($color: blue);\n\n",
                "  &:hover {\n",
                "    .icon {\n",
                "      &.spin {\n",
                "        transform: rotate(1turn);\n",
                "      }\n",
                "    }\n",
                "  }\n",
                "}\n",
            ),
        )
        .unwrap();

        let data = SassCore.view(&path).unwrap();
        let view: SassView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.syntax, Syntax::Scss);
        assert_eq!(view.variables, vec!["spacing"]);
        assert_eq!(
            view.mixins,
            vec![Definition {
                name: "button".to_owned(),
                parameters: vec!["$color".to_owned(), "$radius".to_owned()],
            }]
        );
        assert_eq!(
            view.functions,
            vec![Definition {
                name: "double".to_owned(),
                parameters: vec!["$value".to_owned()],
            }]
        );
        assert_eq!(view.uses, vec!["sass:math"]);
        assert_eq!(view.forwards, vec!["tokens"]);
        assert_eq!(view.placeholders, vec!["reset"]);
        assert_eq!(view.nesting_depth, 4);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.scss");
        let mut content = "$pad: 1;\n".to_owned();
        content.push_str(&"/* pad */\n".repeat((MAX_VIEW_BYTES / 10) + 10));
        std::fs::write(&path, &content).unwrap();

        let data = SassCore.view(&path).unwrap();
        let view: SassView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_every_section_and_the_content() {
        let data = serde_json::to_value(SassView {
            content: ".button {\n}".to_owned(),
            truncated: false,
            syntax: Syntax::Scss,
            variables: vec!["spacing".to_owned()],
            mixins: vec![Definition {
                name: "button".to_owned(),
                parameters: vec!["$color".to_owned()],
            }],
            functions: Vec::new(),
            uses: vec!["sass:math".to_owned()],
            forwards: Vec::new(),
            placeholders: vec!["reset".to_owned()],
            nesting_depth: 1,
        })
        .unwrap();

        let lines = SassPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "syntax: scss".to_owned(),
                "variables: spacing".to_owned(),
                "mixins: button($color)".to_owned(),
                "@use: sass:math".to_owned(),
                "placeholders: %reset".to_owned(),
                "nesting depth: 1".to_owned(),
                ".button {".to_owned(),
                "}".to_owned(),
            ]
        );
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::SassCore),
            plugin_api::PluginPresentation::extensions(&crate::SassPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
