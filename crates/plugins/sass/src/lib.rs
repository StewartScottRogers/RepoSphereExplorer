//! Sass file type plugin: core and presentation halves.
//!
//! Sass has two syntaxes and they do not look alike. The braced one is
//! CSS with more in it; the indented one has no braces and no
//! semicolons, writes a mixin as `=name` and includes it with `+name`.
//! The same compiler reads both, so which one a file is written in is
//! the first thing a reader needs told.
//!
//! What a stylesheet *declares* is the rest: its variables, the mixins
//! and functions it offers, what it pulls in with `@use`, the
//! placeholders other rules extend, and how deeply it nests - because
//! nesting is where a stylesheet becomes hard to follow.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["scss", "sass"];

/// How much of a stylesheet is read.
const READ_CAP: usize = 2 * 1024 * 1024;

/// How many of each kind are listed before the rest are only counted.
const SHOWN: usize = 40;

/// One mixin or function, with what it takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Callable {
    /// Its name.
    pub name: String,
    /// Its parameters, as written, defaults included.
    pub parameters: Vec<String>,
    /// Whether it takes a `@content` block, which only a mixin can.
    pub takes_content: bool,
}

/// One variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Variable {
    /// Its name, with the leading dollar.
    pub name: String,
    /// What it is set to, on the same line.
    pub value: String,
}

/// View data produced by [`SassCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SassView {
    /// Which of the two syntaxes it is written in.
    pub syntax: String,
    /// Whether it is a partial: compiled only through something that
    /// pulls it in, which is what the leading underscore means.
    pub partial: bool,
    /// The variables it declares.
    pub variables: Vec<Variable>,
    /// The mixins it offers.
    pub mixins: Vec<Callable>,
    /// The functions it offers.
    pub functions: Vec<Callable>,
    /// What it pulls in with `@use` or `@import`.
    pub uses: Vec<String>,
    /// What it passes on with `@forward`.
    pub forwards: Vec<String>,
    /// The placeholder selectors other rules can extend.
    pub placeholders: Vec<String>,
    /// What it extends, whether a placeholder or a real selector.
    pub extends: Vec<String>,
    /// The deepest a rule is nested inside another.
    pub nesting_depth: usize,
    /// Whether the sheet was longer than this reads.
    pub truncated: bool,
}

/// Whether `text` reads like Sass.
///
/// A stylesheet that is only CSS is not this - the CSS plugin has that
/// one - so recognition asks for something CSS cannot do: a variable, a
/// mixin, a module rule or a placeholder.
///
/// A `&` parent reference is deliberately not on that list. It used to
/// be Sass's, and is not any more: CSS has nesting of its own now, and
/// counting `&` claimed both of the CSS fixtures as Sass.
fn looks_like_it(text: &str) -> bool {
    let mut markers = 0usize;
    for line in text.lines() {
        let line = line.trim_start();
        if line.starts_with("//") {
            continue;
        }
        // `$name:` is a variable, and CSS has no such thing. A jQuery
        // line reads `$(...)`, which is why the colon is asked for.
        if let Some(rest) = line.strip_prefix('$')
            && rest.split_once(':').is_some_and(|(name, _)| {
                !name.is_empty()
                    && name
                        .chars()
                        .all(|character| character.is_alphanumeric() || character == '-')
            })
        {
            markers += 1;
        }
        for opener in [
            "@mixin ",
            "@include ",
            "@use ",
            "@forward ",
            "@extend ",
            "@return ",
        ] {
            if line.starts_with(opener) {
                markers += 1;
            }
        }
        // The indented syntax's own shorthands: `=name` declares a
        // mixin, `+name` includes one, `%name` is a placeholder. What
        // follows has to be a name - a unified diff's `+added line`
        // and reStructuredText's `====` underline are neither, and
        // counting them claimed both of those fixtures as Sass.
        if let Some(rest) = line.strip_prefix(['=', '+', '%'])
            && is_name(rest)
        {
            markers += 1;
        }
        if markers >= 2 {
            return true;
        }
    }
    false
}

/// Whether `text` opens with a name: what `=`, `+` and `%` are
/// followed by in Sass, and what a diff marker or a rule of characters
/// is not.
fn is_name(text: &str) -> bool {
    let end = text
        .find(|character: char| {
            !character.is_alphanumeric() && character != '-' && character != '_'
        })
        .unwrap_or(text.len());
    // What follows the name is its argument list or nothing at all. A
    // space and more words is not it: a diff's `+use plugin_api::...`
    // read as an include of a mixin called `use` until this said so.
    end > 0
        && text[..end].chars().next().is_some_and(char::is_alphabetic)
        && (text[end..].starts_with('(') || text[end..].trim_end().is_empty())
}

/// Which syntax a stylesheet is written in.
///
/// The braced one ends its declarations with semicolons and wraps its
/// rules in braces; the indented one does neither. Counting is more
/// reliable than looking for the first brace: a `#{...}` interpolation
/// has braces in either syntax.
fn syntax_of(text: &str) -> &'static str {
    let mut braced = 0usize;
    let mut indented = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.ends_with('{') || trimmed.ends_with(';') || trimmed == "}" {
            braced += 1;
        } else if trimmed.starts_with('=')
            || trimmed.starts_with('+')
            // An indented declaration: `color: red` with nothing after
            // it. A trailing comma is a selector list, not a value.
            || (line.starts_with(' ') && trimmed.contains(':') && !trimmed.ends_with(','))
        {
            indented += 1;
        }
    }
    if braced > indented {
        "braced (.scss)"
    } else {
        "indented (.sass)"
    }
}

/// The parameters in `(...)`, split on the commas between them.
///
/// A default can itself hold a comma - `$shadow: 0 0 2px, 0 0 4px` - so
/// the split counts brackets rather than taking every comma.
fn parameters_in(text: &str) -> Vec<String> {
    let Some(open) = text.find('(') else {
        return Vec::new();
    };
    let mut depth = 0usize;
    let mut current = String::new();
    let mut found = Vec::new();
    for character in text[open..].chars() {
        match character {
            '(' => {
                depth += 1;
                if depth == 1 {
                    continue;
                }
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            ',' if depth == 1 => {
                found.push(current.trim().to_owned());
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(character);
    }
    if !current.trim().is_empty() {
        found.push(current.trim().to_owned());
    }
    found
}

/// The name at the start of `text`, up to a bracket or whitespace.
fn name_in(text: &str) -> String {
    let end = text
        .find(|character: char| character == '(' || character.is_whitespace() || character == '{')
        .unwrap_or(text.len());
    text[..end].trim().to_owned()
}

/// What a `@use`, `@forward` or `@import` names, without its quotes.
fn module_in(rest: &str) -> String {
    let said = rest.trim().trim_end_matches([';', ',']).trim();
    let quoted = said.split(['"', '\'']).nth(1).map_or(said, |inside| inside);
    let mut out = quoted.to_owned();
    // `@use "a" as b` and `@forward "a" show c` say more than the path.
    for keyword in [" as ", " show ", " hide ", " with "] {
        if let Some(at) = said.find(keyword) {
            out.push_str(&said[at..]);
            break;
        }
    }
    out
}

/// Everything [`SassView`] holds, read from `source`.
fn parse(source: &str, partial: bool, truncated: bool) -> SassView {
    let mut view = SassView {
        syntax: syntax_of(source).to_owned(),
        partial,
        variables: Vec::new(),
        mixins: Vec::new(),
        functions: Vec::new(),
        uses: Vec::new(),
        forwards: Vec::new(),
        placeholders: Vec::new(),
        extends: Vec::new(),
        nesting_depth: 0,
        truncated,
    };
    let braced = view.syntax.starts_with("braced");
    let mut depth = 0usize;

    for line in source.lines() {
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if braced {
            // Nesting is how deep a rule sits, and the braces say.
            if trimmed.ends_with('{') {
                depth += 1;
                view.nesting_depth = view.nesting_depth.max(depth);
            }
            depth = depth.saturating_sub(trimmed.matches('}').count());
        } else if trimmed.contains(':') || trimmed.starts_with('+') || trimmed.starts_with('@') {
            // The indented syntax has no braces: the indentation is the
            // nesting, and a declaration is as deep as the rule holding
            // it rather than a level of its own.
        } else {
            view.nesting_depth = view.nesting_depth.max(indent / 2 + 1);
        }

        read_line(trimmed, &mut view);
    }
    view
}

/// Reads whatever one trimmed line declares.
fn read_line(line: &str, view: &mut SassView) {
    if let Some(rest) = line.strip_prefix("@use ") {
        view.uses.push(module_in(rest));
    } else if let Some(rest) = line.strip_prefix("@import ") {
        view.uses.push(module_in(rest));
    } else if let Some(rest) = line.strip_prefix("@forward ") {
        view.forwards.push(module_in(rest));
    } else if let Some(rest) = line.strip_prefix("@extend ") {
        view.extends
            .push(rest.trim().trim_end_matches(';').to_owned());
    } else if let Some(rest) = line.strip_prefix("@mixin ") {
        push_callable(&mut view.mixins, rest);
    } else if let Some(rest) = line.strip_prefix("@function ") {
        push_callable(&mut view.functions, rest);
    } else if let Some(rest) = line.strip_prefix('=') {
        // The indented syntax writes a mixin as `=name(...)`.
        push_callable(&mut view.mixins, rest);
    } else if let Some(rest) = line.strip_prefix('%') {
        let name = name_in(rest);
        if !name.is_empty() && view.placeholders.len() < SHOWN {
            view.placeholders.push(format!("%{name}"));
        }
    } else if let Some(rest) = line.strip_prefix('$')
        && let Some((name, value)) = rest.split_once(':')
        && name
            .chars()
            .all(|character| character.is_alphanumeric() || character == '-')
    {
        let value = value.trim().trim_end_matches(';').trim();
        if view.variables.len() < SHOWN {
            view.variables.push(Variable {
                name: format!("${name}"),
                // A map runs over several lines; only the first is here,
                // and saying so beats showing a lone bracket.
                value: if value == "(" {
                    "(a map)".to_owned()
                } else {
                    value.to_owned()
                },
            });
        }
    }
}

/// Adds one mixin or function, read from the text after its keyword.
fn push_callable(into: &mut Vec<Callable>, rest: &str) {
    let name = name_in(rest);
    if name.is_empty() || into.len() >= SHOWN {
        return;
    }
    into.push(Callable {
        name,
        parameters: parameters_in(rest),
        takes_content: false,
    });
}

/// Everything [`SassView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<SassView> {
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
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not Sass"));
    }
    let partial = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('_'));
    let mut view = parse(source, partial, truncated);
    mark_content_mixins(source, &mut view);
    Ok(view)
}

/// Says which mixins take a `@content` block.
///
/// A mixin's body runs to the end of its block, so the marker is looked
/// for after the mixin's own line and before the next one at the same
/// level - which, for both syntaxes, is the next line that is not
/// indented past it.
fn mark_content_mixins(source: &str, view: &mut SassView) {
    let lines: Vec<&str> = source.lines().collect();
    for mixin in &mut view.mixins {
        let Some(start) = lines.iter().position(|line| {
            let trimmed = line.trim_start();
            (trimmed.starts_with("@mixin ") || trimmed.starts_with('='))
                && name_in(
                    trimmed
                        .trim_start_matches("@mixin ")
                        .trim_start_matches('='),
                ) == mixin.name
        }) else {
            continue;
        };
        let opening = lines[start].len() - lines[start].trim_start().len();
        for line in lines.iter().skip(start + 1) {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if line.len() - line.trim_start().len() <= opening && trimmed != "}" {
                break;
            }
            if trimmed.starts_with("@content") {
                mixin.takes_content = true;
                break;
            }
        }
    }
}

/// The Sass plugin's core half.
#[derive(Debug, Default)]
pub struct SassCore;

impl PluginCore for SassCore {
    fn name(&self) -> &'static str {
        "sass"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // The braced syntax is CSS with more in it, so the CSS plugin
        // recognises it too. This is the narrower reading (D13).
        &["css"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
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
            tint: 0x00cd_6799,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: SassView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!("Sass, {} syntax", view.syntax)];
        if view.truncated {
            lines.push("Longer than this reads; what follows is the start.".to_owned());
        }
        lines.push(if view.partial {
            "A partial: compiled only through whatever pulls it in.".to_owned()
        } else {
            "Compiled on its own.".to_owned()
        });
        lines.push(format!("Nested {} level(s) deep", view.nesting_depth));
        for (heading, modules) in [("Uses", &view.uses), ("Forwards", &view.forwards)] {
            if !modules.is_empty() {
                lines.push(format!("{heading}:"));
                for module in modules {
                    lines.push(format!("  {module}"));
                }
            }
        }
        if !view.variables.is_empty() {
            lines.push("Variables:".to_owned());
            for variable in &view.variables {
                lines.push(format!("  {}: {}", variable.name, variable.value));
            }
        }
        for (heading, callables) in [("Mixins", &view.mixins), ("Functions", &view.functions)] {
            if callables.is_empty() {
                continue;
            }
            lines.push(format!("{heading}:"));
            for callable in callables {
                let content = if callable.takes_content {
                    ", taking a block"
                } else {
                    ""
                };
                lines.push(format!(
                    "  {}({}){content}",
                    callable.name,
                    callable.parameters.join(", ")
                ));
            }
        }
        if !view.placeholders.is_empty() {
            lines.push(format!("Placeholders: {}", view.placeholders.join(", ")));
        }
        if !view.extends.is_empty() {
            lines.push(format!("Extends: {}", view.extends.join(", ")));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{SassCore, SassPresentation, SassView, looks_like_it, parameters_in, syntax_of};
    use plugin_api::{PluginCore, PluginPresentation};

    fn sample(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/sass")
            .join(name)
    }

    fn view_of(name: &str) -> SassView {
        serde_json::from_value(SassCore.view(&sample(name)).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&SassCore),
            PluginPresentation::extensions(&SassPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn recognises_what_css_cannot_do() {
        assert!(looks_like_it("$ink: #000;\n@mixin panel { color: $ink; }"));
        assert!(looks_like_it("=bordered\n  border: 1px solid\n+bordered"));
        assert!(
            !looks_like_it("a { color: red; }\nb { color: blue; }"),
            "plain CSS belongs to the CSS plugin"
        );
        assert!(
            !looks_like_it("$(document).ready(function () {});"),
            "a dollar is not a variable without a name and a colon"
        );
        assert!(!looks_like_it(""));
    }

    #[test]
    fn a_diff_and_a_heading_underline_are_not_sass() {
        // `=` and `+` open a mixin and an include in the indented
        // syntax, and open an added line and a heading underline in two
        // formats this repository has fixtures for. Both `samples/diff`
        // and `samples/restructuredtext` were claimed as Sass until
        // what follows the character had to be a name.
        let diff = concat!(
            "--- a/readings.csv\n",
            "+++ b/readings.csv\n",
            "@@ -1,3 +1,3 @@\n",
            " reading\n",
            "-21.4\n",
            "+21.9\n",
        );
        let underlined = concat!(
            "Readings\n",
            "========\n",
            "\n",
            "+------+------+\n",
            "| one  | two  |\n",
            "+------+------+\n",
        );
        let sass = concat!(
            "$ink: #2b2b2b\n",
            "\n",
            "=bordered($colour: $ink)\n",
            "  border: $colour\n",
            "\n",
            ".panel\n",
            "  +bordered\n",
        );

        // A patch of Rust is the harder half: `+use plugin_api::…` and
        // `+pub const …` both open with a plus and a name.
        let patched_rust = concat!(
            "@@ -1,6 +1,9 @@\n",
            " //! The guide plugin.\n",
            "-use plugin_api::PluginCore;\n",
            "+use plugin_api::{Icon, PluginCore};\n",
            "+pub const EXTENSIONS: &[&str] = &[\"guide\"];\n",
        );

        assert!(!looks_like_it(diff));
        assert!(!looks_like_it(underlined));
        assert!(!looks_like_it(patched_rust));
        assert!(looks_like_it(sass), "and the real thing still is");
    }

    #[test]
    fn plain_css_that_nests_is_still_plain_css() {
        // `&` was a Sass marker until CSS grew nesting. Counting it
        // claimed the CSS fixtures, and `sample_coverage` noticed
        // before anybody else could.
        let nested = concat!(
            ".panel {\n",
            "  padding: 1rem;\n",
            "\n",
            "  & > h2 {\n",
            "    margin: 0;\n",
            "  }\n",
            "\n",
            "  &:focus-within {\n",
            "    outline: 2px solid blue;\n",
            "  }\n",
            "}\n",
        );

        assert!(!looks_like_it(nested));
    }

    #[test]
    fn it_says_it_specialises_the_css_reading() {
        assert_eq!(SassCore.specialises(), &["css"]);
    }

    #[test]
    fn tells_the_two_syntaxes_apart() {
        assert_eq!(syntax_of("a {\n  color: red;\n}\n"), "braced (.scss)");
        assert_eq!(syntax_of("a\n  color: red\n"), "indented (.sass)");
    }

    #[test]
    fn a_default_holding_a_comma_is_still_one_parameter() {
        assert_eq!(
            parameters_in("shadow($layers: 0 0 2px, 0 0 4px, $colour: black)"),
            vec!["$layers: 0 0 2px", "0 0 4px", "$colour: black"],
            "a bare comma does split; brackets are what protect one"
        );
        assert_eq!(
            parameters_in("width($of: max(4, 8))"),
            vec!["$of: max(4, 8)"],
            "a comma inside brackets does not"
        );
        assert!(parameters_in("plain").is_empty());
    }

    #[test]
    fn reads_the_braced_partial() {
        let view = view_of("_readings.scss");

        assert_eq!(view.syntax, "braced (.scss)");
        assert!(view.partial, "the leading underscore says so");
        assert!(!view.truncated);
        assert!(view.nesting_depth >= 5, "{}", view.nesting_depth);
    }

    #[test]
    fn reads_what_it_pulls_in_and_what_it_passes_on() {
        let view = view_of("_readings.scss");

        assert!(view.uses.iter().any(|one| one == "sass:math"));
        assert!(
            view.uses
                .iter()
                .any(|one| one.starts_with("../theme/palette") && one.contains("as palette")),
            "a `@use ... as` says more than the path: {:?}",
            view.uses
        );
        assert_eq!(view.forwards.len(), 1);
        assert!(view.forwards[0].contains("show"));
    }

    #[test]
    fn reads_the_variables_including_a_map() {
        let view = view_of("_readings.scss");

        assert!(
            view.variables
                .iter()
                .any(|one| one.name == "$gutter" && one.value == "1.25rem")
        );
        assert!(
            view.variables
                .iter()
                .any(|one| one.name == "$breakpoints" && one.value == "(a map)"),
            "a map runs over several lines, and a lone bracket says nothing"
        );
    }

    #[test]
    fn reads_the_mixins_and_functions_with_their_parameters() {
        let view = view_of("_readings.scss");

        let respond = view
            .mixins
            .iter()
            .find(|one| one.name == "respond-to")
            .expect("the mixin");
        assert_eq!(respond.parameters, vec!["$name", "$edge: min"]);
        assert!(
            respond.takes_content,
            "it wraps `@content` in a media query, which is the whole point of it"
        );

        let numeric = view
            .mixins
            .iter()
            .find(|one| one.name == "numeric-column")
            .expect("the other mixin");
        assert!(!numeric.takes_content);

        let column = view
            .functions
            .iter()
            .find(|one| one.name == "column-width")
            .expect("the function");
        assert_eq!(column.parameters, vec!["$columns", "$of: 12"]);
    }

    #[test]
    fn reads_the_placeholders_and_what_extends_them() {
        let view = view_of("_readings.scss");

        assert_eq!(
            view.placeholders,
            vec!["%panel-surface", "%screen-reader-only"]
        );
        assert_eq!(view.extends, vec!["%panel-surface", "%screen-reader-only"]);
    }

    #[test]
    fn reads_the_indented_syntax_the_same_way() {
        let view = view_of("legacy.sass");

        assert_eq!(view.syntax, "indented (.sass)");
        assert!(!view.partial, "no leading underscore");
        assert!(
            view.mixins.iter().any(|one| one.name == "bordered"),
            "`=bordered` is a mixin: {:?}",
            view.mixins
        );
        assert!(view.mixins.iter().any(|one| one.name == "stacked"));
        assert!(view.functions.iter().any(|one| one.name == "halve"));
        assert_eq!(view.placeholders, vec!["%boxed"]);
        assert!(view.uses.iter().any(|one| one.contains("readings")));
    }

    #[test]
    fn presents_the_syntax_first_because_it_decides_everything_else() {
        let data = SassCore.view(&sample("legacy.sass")).unwrap();

        let lines = SassPresentation.present(&data);

        assert_eq!(lines[0], "Sass, indented (.sass) syntax");
        assert!(lines.iter().any(|line| line.contains("bordered(")));
        assert!(lines.iter().any(|line| line.contains("%boxed")));
    }

    #[test]
    fn a_file_that_is_not_sass_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.scss");
        std::fs::write(&path, b"nothing of the sort at all").unwrap();

        assert!(SassCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
