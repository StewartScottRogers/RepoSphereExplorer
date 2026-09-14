//! Cascading Style Sheets file type plugin: core and presentation halves.
//!
//! A stylesheet is read for what it declares rather than for what it
//! does: which custom properties exist, which at-rules gate what, which
//! fonts it pulls in, and which colours it uses. Those are the questions
//! somebody opening a stylesheet they did not write actually has.
//!
//! Colours are counted across all the ways of writing one - hex,
//! `rgb()`, `hsl()`, and the named colours - because a reader asking what
//! colours a sheet uses is asking about all of them at once.

use plugin_api::{Icon, PluginCore, PluginPresentation, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;
use syntax::{Language, Quote};

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["css"];

/// How much of a stylesheet is read.
const READ_CAP: usize = 2 * 1024 * 1024;

/// How many of each kind are listed before the rest are only counted.
const SHOWN: usize = 40;

/// One at-rule, and what it applies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtRule {
    /// Its name, without the `@`.
    pub name: String,
    /// The condition or target after the name.
    pub condition: String,
}

/// One custom property.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomProperty {
    /// Its name, including the two leading dashes.
    pub name: String,
    /// What it is set to.
    pub value: String,
}

/// View data produced by [`CssCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CssView {
    /// How many rules the sheet has: a selector and its block.
    pub rules: usize,
    /// The selectors, in order, up to [`SHOWN`].
    pub selectors: Vec<String>,
    /// The at-rules, with what each one is conditional on.
    pub at_rules: Vec<AtRule>,
    /// The custom properties it defines.
    pub custom_properties: Vec<CustomProperty>,
    /// The custom properties it reads with `var()` but never defines -
    /// which is either a typo or a property somebody else has to supply.
    pub undefined_properties: Vec<String>,
    /// The font families it declares with `@font-face`.
    pub fonts: Vec<String>,
    /// What it imports.
    pub imports: Vec<String>,
    /// The colours it uses, however they are written.
    pub colours: Vec<String>,
    /// The animations it defines with `@keyframes`.
    pub animations: Vec<String>,
    /// Whether the sheet uses nesting: a style rule written *inside*
    /// another style rule, which needs a browser new enough to have it
    /// rather than a preprocessor.
    ///
    /// A rule inside an at-rule is not nesting. Every sheet with a
    /// `@media` block would otherwise claim it.
    pub nested: bool,
    /// Whether the sheet was longer than this reads.
    pub truncated: bool,
}

/// Whether `text` reads like a stylesheet.
///
/// A stylesheet has no magic bytes, so this asks for the shape: a
/// selector and a brace-delimited block of `property: value;`, or one of
/// the at-rules and custom properties that only appear in CSS. Asking
/// for the block matters - a bare `@media` line could be anything.
fn looks_like_it(text: &str) -> bool {
    let stripped = without_comments(text);
    let declarations = stripped.matches(';').count();
    let blocks = stripped.matches('{').count();
    if blocks == 0 || declarations == 0 {
        return false;
    }
    let marker = stripped.contains("@media")
        || stripped.contains("@import")
        || stripped.contains("@font-face")
        || stripped.contains("@keyframes")
        || stripped.contains("@supports")
        || stripped.contains("@charset")
        || stripped.contains("--")
        || stripped.contains("var(");
    // A block whose contents look like `name: value;` is the shape, and
    // several of them is not a coincidence. A JSON object has colons too
    // but no semicolons between its members, and a C function body has
    // semicolons but no colons.
    let colons = stripped.matches(':').count();
    marker || (blocks >= 2 && declarations >= 3 && colons >= declarations)
}

/// The text with every `/* ... */` comment taken out.
///
/// Everything downstream counts braces and semicolons, and a comment can
/// hold either.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("/*") {
        out.push_str(&rest[..at]);
        match rest[at + 2..].find("*/") {
            Some(end) => rest = &rest[at + 2 + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The colours in `text`, in the order they first appear.
fn colours_in(text: &str) -> Vec<String> {
    /// The named colours worth recognising: the ones people write.
    const NAMED: &[&str] = &[
        "black",
        "white",
        "red",
        "green",
        "blue",
        "yellow",
        "orange",
        "purple",
        "grey",
        "gray",
        "silver",
        "maroon",
        "navy",
        "teal",
        "olive",
        "lime",
        "aqua",
        "fuchsia",
        "transparent",
        "currentcolor",
    ];
    let mut found: Vec<String> = Vec::new();
    let mut push = |colour: String| {
        if !found.iter().any(|had| had.eq_ignore_ascii_case(&colour)) {
            found.push(colour);
        }
    };

    // Hex, which is a hash and then three to eight hexadecimal digits.
    let bytes = text.as_bytes();
    let mut at = 0usize;
    while let Some(hash) = text[at..].find('#') {
        let start = at + hash;
        let mut end = start + 1;
        while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
            end += 1;
        }
        let digits = end - start - 1;
        if matches!(digits, 3 | 4 | 6 | 8) {
            push(text[start..end].to_owned());
        }
        at = end.max(start + 1);
    }

    // The functional forms, each of which ends at its closing bracket.
    for function in ["rgb(", "rgba(", "hsl(", "hsla(", "oklch(", "lab("] {
        let mut rest = text;
        while let Some(at) = rest.find(function) {
            let after = &rest[at + function.len()..];
            match after.find(')') {
                Some(end) => {
                    push(format!("{function}{})", &after[..end]));
                    rest = &after[end + 1..];
                }
                None => break,
            }
        }
    }

    // The named ones, as whole words: `red` in `border-red` is not a
    // colour, and `green` in `background: green;` is.
    for name in NAMED {
        for (at, _) in text.match_indices(name) {
            let before = text[..at].chars().next_back();
            let after = text[at + name.len()..].chars().next();
            let boundary = |what: Option<char>| {
                what.is_none_or(|character| !character.is_alphanumeric() && character != '-')
            };
            if boundary(before) && boundary(after) {
                push((*name).to_owned());
                break;
            }
        }
    }
    found
}

/// Everything [`CssView`] holds, read from `source`.
fn parse(source: &str, truncated: bool) -> CssView {
    let text = without_comments(source);
    let mut view = CssView {
        rules: 0,
        selectors: Vec::new(),
        at_rules: Vec::new(),
        custom_properties: Vec::new(),
        undefined_properties: Vec::new(),
        fonts: Vec::new(),
        imports: Vec::new(),
        colours: colours_in(&text),
        animations: Vec::new(),
        nested: false,
        truncated,
    };

    // The heading of a block is everything since the last `{`, `}` or
    // `;`, which is how CSS itself decides where a selector begins.
    let mut heading = String::new();
    // One entry per open brace, saying whether it was opened by a style
    // rule. Nesting is a style rule opened while another is still open,
    // and the depth alone cannot tell that from a rule inside a
    // `@media` or a `@layer`.
    let mut open: Vec<bool> = Vec::new();
    let mut in_font_face = false;
    for character in text.chars() {
        match character {
            '{' => {
                let head = heading.split_whitespace().collect::<Vec<&str>>().join(" ");
                heading.clear();
                let is_style_rule = !head.starts_with('@') && !head.is_empty();
                if is_style_rule && open.contains(&true) {
                    view.nested = true;
                }
                open.push(is_style_rule);
                if let Some(rule) = head.strip_prefix('@') {
                    let (name, condition) =
                        rule.split_once(char::is_whitespace).unwrap_or((rule, ""));
                    if name == "font-face" {
                        in_font_face = true;
                    }
                    if name == "keyframes" {
                        view.animations.push(condition.trim().to_owned());
                    }
                    view.at_rules.push(AtRule {
                        name: name.to_owned(),
                        condition: condition.trim().to_owned(),
                    });
                } else if is_style_rule {
                    view.rules += 1;
                    if view.selectors.len() < SHOWN {
                        view.selectors.push(head);
                    }
                }
            }
            '}' => {
                open.pop();
                if open.is_empty() {
                    in_font_face = false;
                }
                heading.clear();
            }
            ';' => {
                let declaration = heading.trim().to_owned();
                heading.clear();
                read_declaration(&declaration, in_font_face, &mut view);
            }
            _ => heading.push(character),
        }
    }
    // An `@import` or `@charset` ends at its semicolon rather than a
    // block, so the last heading may still hold one.
    read_declaration(heading.trim(), in_font_face, &mut view);

    let defined: Vec<String> = view
        .custom_properties
        .iter()
        .map(|property| property.name.clone())
        .collect();
    view.undefined_properties = used_properties(&text)
        .into_iter()
        .filter(|name| !defined.contains(name))
        .collect();
    view
}

/// Reads one `name: value` declaration, or an at-rule that ends at a
/// semicolon rather than a block.
fn read_declaration(declaration: &str, in_font_face: bool, view: &mut CssView) {
    if declaration.is_empty() {
        return;
    }
    if let Some(rest) = declaration.strip_prefix("@import") {
        view.imports.push(rest.trim().to_owned());
        view.at_rules.push(AtRule {
            name: "import".to_owned(),
            condition: rest.trim().to_owned(),
        });
        return;
    }
    if let Some(rest) = declaration.strip_prefix("@charset") {
        view.at_rules.push(AtRule {
            name: "charset".to_owned(),
            condition: rest.trim().trim_matches('"').to_owned(),
        });
        return;
    }
    if let Some(rule) = declaration.strip_prefix('@') {
        let (name, condition) = rule.split_once(char::is_whitespace).unwrap_or((rule, ""));
        view.at_rules.push(AtRule {
            name: name.trim_end_matches(';').to_owned(),
            condition: condition.trim().to_owned(),
        });
        return;
    }
    let Some((name, value)) = declaration.split_once(':') else {
        return;
    };
    let name = name.trim();
    let value = value.trim();
    if name.starts_with("--") {
        if view.custom_properties.len() < SHOWN {
            view.custom_properties.push(CustomProperty {
                name: name.to_owned(),
                value: value.to_owned(),
            });
        }
    } else if in_font_face && name == "font-family" {
        view.fonts.push(value.trim_matches('"').to_owned());
    }
}

/// Every custom property `var()` reads, in first-seen order.
fn used_properties(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("var(") {
        let after = &rest[at + 4..];
        let end = after.find([')', ',']).unwrap_or(after.len());
        let name = after[..end].trim().to_owned();
        if name.starts_with("--") && !found.contains(&name) {
            found.push(name);
        }
        rest = &after[end..];
    }
    found
}

/// Everything [`CssView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<CssView> {
    let source = std::fs::read_to_string(path)?;
    let truncated = source.len() > READ_CAP;
    let source = if truncated {
        let mut end = READ_CAP;
        while end > 0 && !source.is_char_boundary(end) {
            end -= 1;
        }
        &source[..end]
    } else {
        source.as_str()
    };
    if !looks_like_it(source) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a stylesheet",
        ));
    }
    Ok(parse(source, truncated))
}

/// The stylesheet plugin's core half.
#[derive(Debug, Default)]
pub struct CssCore;

/// How this language is coloured, for the shared tokeniser. GUIDANCE.md
/// §3.6: the plugin describes its own format, the pane paints what it is
/// told.
const CSS: Language = Language {
    line_comment: &[],
    block_comment: &[("/*", "*/")],
    quotes: &[Quote::simple('"'), Quote::simple('\'')],
    keywords: &[
        "and",
        "charset",
        "font-face",
        "from",
        "import",
        "important",
        "keyframes",
        "media",
        "not",
        "only",
        "supports",
        "to",
    ],
    types: &["auto", "inherit", "initial", "none", "unset"],
    calls: true,
    ignore_case: false,
};

impl PluginCore for CssCore {
    fn name(&self) -> &'static str {
        "css"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The stylesheet plugin's presentation half.
#[derive(Debug, Default)]
pub struct CssPresentation;

impl PluginPresentation for CssPresentation {
    fn classify(&self, text: &str) -> Vec<Span> {
        syntax::classify(text, &CSS)
    }

    fn name(&self) -> &'static str {
        "css"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "CSS",
            tint: 0x0026_39bf,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: CssView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "Stylesheet: {} rule(s), {} at-rule(s), {} custom propert(ies)",
            view.rules,
            view.at_rules.len(),
            view.custom_properties.len()
        )];
        if view.truncated {
            lines.push("Longer than this reads; what follows is the start.".to_owned());
        }
        lines.push(if view.nested {
            "Uses nesting, so it needs a browser that has it - no \
             preprocessor is involved."
                .to_owned()
        } else {
            "No nesting.".to_owned()
        });
        if !view.imports.is_empty() {
            lines.push("Imports:".to_owned());
            for import in &view.imports {
                lines.push(format!("  {import}"));
            }
        }
        if !view.custom_properties.is_empty() {
            lines.push("Custom properties:".to_owned());
            for property in &view.custom_properties {
                lines.push(format!("  {}: {}", property.name, property.value));
            }
        }
        if !view.undefined_properties.is_empty() {
            lines.push("Read with var() but never defined here:".to_owned());
            for name in &view.undefined_properties {
                lines.push(format!("  {name}"));
            }
        }
        if !view.fonts.is_empty() {
            lines.push(format!("Declares the font {}", view.fonts.join(", ")));
        }
        if !view.animations.is_empty() {
            lines.push(format!("Animations: {}", view.animations.join(", ")));
        }
        if !view.colours.is_empty() {
            lines.push(format!("Colours: {}", view.colours.join(", ")));
        }
        if !view.at_rules.is_empty() {
            lines.push("At-rules:".to_owned());
            for rule in &view.at_rules {
                if rule.condition.is_empty() {
                    lines.push(format!("  @{}", rule.name));
                } else {
                    lines.push(format!("  @{} {}", rule.name, rule.condition));
                }
            }
        }
        if !view.selectors.is_empty() {
            lines.push("Selectors:".to_owned());
            for selector in &view.selectors {
                lines.push(format!("  {selector}"));
            }
            if view.rules > view.selectors.len() {
                lines.push(format!(
                    "  ... and {} more",
                    view.rules - view.selectors.len()
                ));
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{CssCore, CssPresentation, CssView, colours_in, looks_like_it, without_comments};
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        sample("readings.css")
    }

    fn sample(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/css")
            .join(name)
    }

    fn view_of() -> CssView {
        serde_json::from_value(CssCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&CssCore),
            PluginPresentation::extensions(&CssPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn recognises_the_shape_rather_than_a_keyword() {
        assert!(looks_like_it(
            "a { color: red; }\np { margin: 0; }\nb { top: 0; }"
        ));
        assert!(looks_like_it(":root { --ink: #123456; }"));
        assert!(
            !looks_like_it("@media something"),
            "an at-rule with no block is not a stylesheet"
        );
        assert!(
            !looks_like_it("{\"name\": \"a\", \"version\": \"1\"}"),
            "JSON has colons and braces and is not this"
        );
        assert!(
            !looks_like_it("int main(void) { return 0; }"),
            "a C function has braces and semicolons and no declarations"
        );
        assert!(!looks_like_it(""));
    }

    #[test]
    fn a_comment_cannot_change_the_brace_count() {
        assert_eq!(without_comments("a /* { ; } */ b"), "a  b");
        assert_eq!(
            without_comments("a /* unterminated"),
            "a ",
            "an unterminated comment runs to the end, as CSS says it does"
        );
    }

    #[test]
    fn finds_a_colour_however_it_is_written() {
        let found =
            colours_in("a{color:#fff;background:rgb(1 2 3);border:hsl(0 0% 0%);outline:red}");

        assert!(found.contains(&"#fff".to_owned()));
        assert!(found.contains(&"rgb(1 2 3)".to_owned()));
        assert!(found.contains(&"hsl(0 0% 0%)".to_owned()));
        assert!(found.contains(&"red".to_owned()));
    }

    #[test]
    fn a_colour_name_inside_a_word_is_not_a_colour() {
        let found = colours_in(".border-red { --redacted: 1; }");

        assert!(
            !found.iter().any(|colour| colour == "red"),
            "found {found:?}"
        );
    }

    #[test]
    fn reads_the_custom_properties_and_what_they_are_set_to() {
        let view = view_of();

        assert!(view.custom_properties.len() >= 7);
        let ink = view
            .custom_properties
            .iter()
            .find(|property| property.name == "--ink")
            .expect("--ink is defined");
        assert_eq!(ink.value, "#1f2933");
        assert!(
            view.custom_properties
                .iter()
                .any(|property| property.name == "--measure" && property.value == "68ch")
        );
    }

    #[test]
    fn every_property_this_sheet_reads_is_one_it_defines() {
        let view = view_of();

        assert!(
            view.undefined_properties.is_empty(),
            "this sheet supplies its own: {:?}",
            view.undefined_properties
        );
    }

    #[test]
    fn a_property_read_but_never_defined_is_called_out() {
        let view = super::parse(":root { --ink: #000; }\na { color: var(--paper); }", false);

        assert_eq!(view.undefined_properties, vec!["--paper"]);
    }

    #[test]
    fn an_overlay_reads_properties_the_sheet_underneath_supplies() {
        // A theme overlay defines what it overrides and reads the rest
        // from the sheet it is loaded after. Saying which those are is
        // the difference between a typo and a dependency.
        let view: CssView =
            serde_json::from_value(CssCore.view(&sample("high-contrast.css")).unwrap()).unwrap();

        assert_eq!(
            view.undefined_properties,
            vec!["--gutter", "--measure", "--panel-radius", "--danger"]
        );
        assert!(
            view.custom_properties
                .iter()
                .any(|property| property.name == "--ink"),
            "the ones it does override are still its own"
        );
        let layers: Vec<&str> = view
            .at_rules
            .iter()
            .filter(|rule| rule.name == "layer")
            .map(|rule| rule.condition.as_str())
            .collect();
        assert_eq!(
            layers,
            vec!["theme", "theme"],
            "the statement that orders the layer, and the block that fills it"
        );
    }

    #[test]
    fn reads_the_at_rules_with_their_conditions() {
        let view = view_of();

        let named = |name: &str| -> Vec<String> {
            view.at_rules
                .iter()
                .filter(|rule| rule.name == name)
                .map(|rule| rule.condition.clone())
                .collect()
        };
        assert_eq!(named("charset"), vec!["utf-8"]);
        assert_eq!(named("font-face"), vec![""]);
        assert_eq!(named("container"), vec!["panel (min-width: 40rem)"]);
        assert_eq!(named("supports"), vec!["(backdrop-filter: blur(4px))"]);
        assert_eq!(named("keyframes"), vec!["settle"]);
        let media = named("media");
        assert_eq!(
            media.len(),
            3,
            "three `@media` blocks; the `screen` on the second `@import` is              a media query on an import, not one of these"
        );
        assert!(media.contains(&"(prefers-color-scheme: dark)".to_owned()));
        assert!(media.contains(&"print".to_owned()));
        assert!(media.contains(&"(prefers-reduced-motion: reduce)".to_owned()));
    }

    #[test]
    fn reads_the_imports_the_font_and_the_animation() {
        let view = view_of();

        assert_eq!(view.imports.len(), 2);
        assert!(view.imports[0].contains("reset.css"));
        assert!(view.imports[1].contains("typography.css"));
        assert_eq!(view.fonts, vec!["Readings Mono"]);
        assert_eq!(view.animations, vec!["settle"]);
    }

    #[test]
    fn notices_that_the_sheet_nests() {
        let view = view_of();

        assert!(view.nested, "`.panel` opens `& > h2` inside itself");
        assert!(!super::parse("a { color: red; }", false).nested);
    }

    #[test]
    fn a_rule_inside_an_at_rule_is_not_nesting() {
        // Found by looking at the running application: the overlay said
        // it used nesting, and it does not - every rule in it sits
        // inside `@layer`, at the same depth a `@media` block would put
        // one. Depth alone cannot tell those apart.
        let view: CssView =
            serde_json::from_value(CssCore.view(&sample("high-contrast.css")).unwrap()).unwrap();

        assert!(
            !view.nested,
            "its rules are inside `@layer`, which is not one rule inside another"
        );
        assert!(!super::parse("@media print { a { color: red; } }", false).nested);
        assert!(
            super::parse("a { color: red; & b { color: blue; } }", false).nested,
            "this one really is nested"
        );
    }

    #[test]
    fn reads_the_selectors_and_counts_the_rules() {
        let view = view_of();

        assert!(view.rules >= 12, "{}", view.rules);
        assert!(view.selectors.iter().any(|one| one == ":root"));
        assert!(view.selectors.iter().any(|one| one == "table.readings"));
        assert!(
            view.selectors
                .iter()
                .any(|one| one == "table.readings th, table.readings td"),
            "a selector list is one selector, whitespace collapsed"
        );
        assert!(!view.truncated);
    }

    #[test]
    fn finds_the_colours_the_sheet_actually_uses() {
        let view = view_of();

        assert!(view.colours.contains(&"#1f2933".to_owned()));
        assert!(view.colours.contains(&"rgb(11 94 215)".to_owned()));
        assert!(view.colours.contains(&"hsl(38 92% 44%)".to_owned()));
    }

    #[test]
    fn presents_what_a_reader_opens_a_stylesheet_for() {
        let data = CssCore.view(&fixture()).unwrap();

        let lines = CssPresentation.present(&data);

        assert!(lines[0].starts_with("Stylesheet: "));
        assert!(lines.iter().any(|line| line.contains("Uses nesting")));
        assert!(lines.iter().any(|line| line.contains("--ink: #1f2933")));
        assert!(lines.iter().any(|line| line.contains("Readings Mono")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("@container panel (min-width: 40rem)"))
        );
    }

    #[test]
    fn a_file_that_is_not_a_stylesheet_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really-a.css");
        std::fs::write(&path, b"nothing of the sort at all").unwrap();

        assert!(CssCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
