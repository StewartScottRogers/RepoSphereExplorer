//! CSS file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`CssCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CssView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Selectors and at-rule preludes of top-level rule blocks found in the
    /// content, in source order (e.g. `body`, `.container > .item:hover`,
    /// `@media (max-width: 600px)`).
    pub selectors: Vec<String>,
}

/// At-rule keywords whose presence marks `text` as CSS/SCSS/Less — not used
/// by any sibling plugin.
const AT_RULE_KEYWORDS: &[&str] = &[
    "@media",
    "@import",
    "@font-face",
    "@keyframes",
    "@supports",
    "@charset",
    "@page",
];

/// Whether `line` is a CSS/SCSS/Less declaration, e.g. `color: red;`,
/// `--main-color: #336699;`, `$primary: blue;`, or `@link-color: blue;`. The
/// property name must be a single space-free token, which is what excludes
/// unrelated `key: value;`-shaped lines from other languages (e.g. a
/// TypeScript `let x: string;` has a space in `let x`).
fn is_css_declaration(line: &str) -> bool {
    let Some(without_semi) = line.trim().strip_suffix(';') else {
        return false;
    };
    let Some(colon_idx) = without_semi.find(':') else {
        return false;
    };
    let (prop, value) = without_semi.split_at(colon_idx);
    let prop = prop.trim();
    let value = value[1..].trim();
    if prop.is_empty() || value.is_empty() {
        return false;
    }
    let first = prop.chars().next().unwrap_or(' ');
    if !(first.is_ascii_alphabetic() || first == '-' || first == '$' || first == '@') {
        return false;
    }
    prop.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '$' || c == '@')
}

/// Whether `text` looks like CSS/SCSS/Less: an `@media`/`@import`/
/// `@font-face`/`@keyframes`/`@supports`/`@charset`/`@page` at-rule, an
/// `!important` declaration, or a `property: value;`-shaped declaration line
/// — markers not used by any sibling plugin.
fn has_css_syntax(text: &str) -> bool {
    AT_RULE_KEYWORDS.iter().any(|kw| text.contains(kw))
        || text.contains("!important")
        || text.lines().any(is_css_declaration)
}

/// Extracts the selector or at-rule prelude from a top-level rule-opening
/// line, e.g. `body {` or `.container > .item:hover {`.
fn parse_selector(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let prelude = trimmed.strip_suffix('{')?.trim();
    (!prelude.is_empty()).then(|| prelude.to_owned())
}

/// Parses the selectors and at-rule preludes of rule-opening lines out of
/// `content`, in source order.
fn parse_selectors(content: &str) -> Vec<String> {
    content.lines().filter_map(parse_selector).collect()
}

/// The CSS plugin's core half.
#[derive(Debug, Default)]
pub struct CssCore;

impl PluginCore for CssCore {
    fn name(&self) -> &'static str {
        "css"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_css_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let selectors = parse_selectors(&content);
        let view = CssView {
            content,
            truncated,
            selectors,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The CSS plugin's presentation half.
#[derive(Debug, Default)]
pub struct CssPresentation;

impl PluginPresentation for CssPresentation {
    fn name(&self) -> &'static str {
        "css"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: CssView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.selectors.is_empty() {
            lines.push(format!("selectors: {}", view.selectors.join(", ")));
        }
        lines.extend(view.content.lines().map(str::to_owned));
        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{CssCore, CssPresentation, CssView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-css-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_css_syntax_as_css() {
        assert!(CssCore.sniff(b"body {\n  color: red;\n}\n"));
        assert!(CssCore.sniff(b"@media (max-width: 600px) {\n  .a { display: none; }\n}\n"));
        assert!(CssCore.sniff(b"@import url('reset.css');\n"));
        assert!(CssCore.sniff(b"@font-face {\n  font-family: 'Foo';\n}\n"));
        assert!(CssCore.sniff(b"@keyframes spin {\n  to { transform: rotate(360deg); }\n}\n"));
        assert!(CssCore.sniff(b":root {\n  --main-color: #336699;\n}\n"));
        assert!(CssCore.sniff(b"$primary-color: #336699;\n"));
        assert!(CssCore.sniff(b".a {\n  color: red !important;\n}\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_css() {
        assert!(!CssCore.sniff(b"interface Named {\n  let x: string;\n}\n"));
        assert!(!CssCore.sniff(b"package main\n\nfunc main() {}\n"));
        assert!(!CssCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!CssCore.sniff(b"struct Point {\n    x: i32,\n    y: i32,\n}\n"));
        assert!(!CssCore.sniff(b"just a regular line of text\n"));
        assert!(!CssCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_css_file_and_extracts_selectors() {
        let path = unique_temp_file("styles.css");
        std::fs::write(
            &path,
            ":root {\n  --main-color: #336699;\n}\n\nbody {\n  margin: 0;\n}\n\n.container > .item:hover {\n  color: var(--main-color);\n}\n\n@media (max-width: 600px) {\n  .container {\n    display: block;\n  }\n}\n",
        )
        .unwrap();

        let data = CssCore.view(&path).unwrap();
        let view: CssView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(
            view.selectors,
            vec![
                ":root",
                "body",
                ".container > .item:hover",
                "@media (max-width: 600px)",
                ".container",
            ]
        );
        assert!(view.content.contains("--main-color: #336699;"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.css");
        let mut content = "body {\n  color: red;\n}\n".to_owned();
        content.push_str(&"/* padding */ ".repeat(MAX_VIEW_BYTES));
        std::fs::write(&path, content).unwrap();

        let data = CssCore.view(&path).unwrap();
        let view: CssView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_selectors_and_content() {
        let data = serde_json::to_value(CssView {
            content: "body {\n  color: red;\n}".to_owned(),
            truncated: false,
            selectors: vec!["body".to_owned()],
        })
        .unwrap();

        let lines = CssPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["selectors: body", "body {", "  color: red;", "}"]
        );
    }
}
