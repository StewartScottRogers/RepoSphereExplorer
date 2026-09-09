//! Git attributes file type plugin: core and presentation halves.
//!
//! Lines of `pattern attr=value`, with attributes git defines: `text`,
//! `binary`, `eol`, `diff`, `merge`, `filter`, `linguist-*`. Those names
//! are the marker; the shape alone would read as an ignore file.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["gitattributes"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One pattern and the attributes it sets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// The path pattern.
    pub pattern: String,
    /// The attributes, as written: `text`, `-text`, `eol=lf`.
    pub attributes: Vec<String>,
    /// Whether it marks the pattern binary, either directly or through
    /// `-text -diff`.
    pub binary: bool,
}

/// View data produced by [`GitattributesCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitattributesView {
    /// Every rule, in file order. The last match wins.
    pub rules: Vec<Rule>,
    /// The patterns marked binary, which is what stops a checkout
    /// rewriting bytes inside them.
    pub binary_patterns: Vec<String>,
    /// The custom diff drivers named.
    pub diff_drivers: Vec<String>,
    /// The custom merge drivers named.
    pub merge_drivers: Vec<String>,
    /// The clean/smudge filters named.
    pub filters: Vec<String>,
    /// The `linguist-*` overrides, which change what the host counts.
    pub linguist: Vec<String>,
    /// How many comment lines the file carries.
    pub comments: usize,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The attribute names git itself defines.
const KNOWN: &[&str] = &[
    "text",
    "eol",
    "diff",
    "merge",
    "filter",
    "binary",
    "ident",
    "working-tree-encoding",
    "export-ignore",
    "export-subst",
    "delta",
    "encoding",
    "whitespace",
];

/// Everything [`GitattributesView`] holds, read from `text`.
fn parse(text: &str) -> GitattributesView {
    let mut view = GitattributesView {
        rules: Vec::new(),
        binary_patterns: Vec::new(),
        diff_drivers: Vec::new(),
        merge_drivers: Vec::new(),
        filters: Vec::new(),
        linguist: Vec::new(),
        comments: 0,
        content: String::new(),
        truncated: false,
    };

    for raw in text.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            view.comments += 1;
            continue;
        }
        let mut fields = trimmed.split_whitespace();
        let Some(pattern) = fields.next() else {
            continue;
        };
        let attributes: Vec<String> = fields.map(str::to_owned).collect();
        if attributes.is_empty() {
            continue;
        }

        let mut binary = attributes.iter().any(|attribute| attribute == "binary");
        let unset_text = attributes.iter().any(|attribute| attribute == "-text");
        let unset_diff = attributes.iter().any(|attribute| attribute == "-diff");
        if unset_text && unset_diff {
            binary = true;
        }
        if binary {
            view.binary_patterns.push(pattern.to_owned());
        }

        for attribute in &attributes {
            if let Some(driver) = attribute.strip_prefix("diff=") {
                view.diff_drivers.push(driver.to_owned());
            } else if let Some(driver) = attribute.strip_prefix("merge=") {
                view.merge_drivers.push(driver.to_owned());
            } else if let Some(name) = attribute.strip_prefix("filter=") {
                view.filters.push(name.to_owned());
            } else if attribute.starts_with("linguist-") {
                view.linguist.push(format!("{pattern} {attribute}"));
            }
        }

        view.rules.push(Rule {
            pattern: pattern.to_owned(),
            attributes,
            binary,
        });
    }
    view
}

/// Whether `text` looks like a git attributes file.
fn looks_like_it(text: &str) -> bool {
    let mut recognised = 0usize;
    let mut rules = 0usize;
    for raw in text.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut fields = trimmed.split_whitespace();
        if fields.next().is_none() {
            continue;
        }
        let attributes: Vec<&str> = fields.collect();
        if attributes.is_empty() {
            // A bare pattern with no attribute is an ignore file's shape,
            // not this one's.
            return false;
        }
        rules += 1;
        for attribute in attributes {
            let name = attribute
                .trim_start_matches(['-', '!'])
                .split('=')
                .next()
                .unwrap_or_default();
            if KNOWN.contains(&name) || name.starts_with("linguist-") {
                recognised += 1;
            }
        }
    }
    rules >= 2 && recognised >= 2
}

/// The Git attributes plugin's core half.
#[derive(Debug, Default)]
pub struct GitattributesCore;

impl PluginCore for GitattributesCore {
    fn name(&self) -> &'static str {
        "gitattributes"
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

/// The Git attributes plugin's presentation half.
#[derive(Debug, Default)]
pub struct GitattributesPresentation;

impl PluginPresentation for GitattributesPresentation {
    fn name(&self) -> &'static str {
        "gitattributes"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "ATTR",
            tint: 0x00f0_5033,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: GitattributesView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "{} rule(s), the last match winning",
            view.rules.len()
        ));
        for rule in &view.rules {
            lines.push(format!("  {}  {}", rule.pattern, rule.attributes.join(" ")));
        }
        if !view.binary_patterns.is_empty() {
            lines.push(format!(
                "Marked binary, so a checkout rewrites no bytes inside them: {}",
                view.binary_patterns.join(", ")
            ));
        }
        if !view.diff_drivers.is_empty() {
            lines.push(format!("Diff drivers: {}", view.diff_drivers.join(", ")));
        }
        if !view.merge_drivers.is_empty() {
            lines.push(format!("Merge drivers: {}", view.merge_drivers.join(", ")));
        }
        if !view.filters.is_empty() {
            lines.push(format!("Filters: {}", view.filters.join(", ")));
        }
        if !view.linguist.is_empty() {
            lines.push(format!("Language overrides: {}", view.linguist.join(", ")));
        }
        lines.push(format!("Comments: {}", view.comments));

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{GitattributesCore, GitattributesPresentation, GitattributesView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_patterns_carrying_attributes_git_defines() {
        assert!(GitattributesCore.sniff(b"*.png binary\n*.rs text eol=lf\n"));
        assert!(GitattributesCore.sniff(b"*.md text diff=markdown\n*.lock -diff\n"));
    }

    #[test]
    fn does_not_claim_an_ignore_file() {
        // Bare patterns with no attributes are an ignore file's shape.
        assert!(!GitattributesCore.sniff(b"target/\n*.log\n!keep.log\n"));
        assert!(!GitattributesCore.sniff(b""));
    }

    #[test]
    fn reads_each_rule_with_its_attributes() {
        let view = parse("*.rs text eol=lf\n*.png binary\n");

        assert_eq!(view.rules.len(), 2);
        assert_eq!(
            view.rules[0].attributes,
            vec!["text".to_owned(), "eol=lf".to_owned()]
        );
        assert!(view.rules[1].binary);
    }

    #[test]
    fn minus_text_and_minus_diff_together_mean_binary() {
        // Which is how most files are actually marked binary in the wild -
        // and the reason a PDF's cross-reference table survived a checkout.
        let view = parse("*.pdf -text -diff\n");

        assert!(view.rules[0].binary);
        assert_eq!(view.binary_patterns, vec!["*.pdf".to_owned()]);
    }

    #[test]
    fn minus_text_alone_is_not_binary() {
        let view = parse("*.txt -text\n*.a binary\n");

        assert!(!view.rules[0].binary);
    }

    #[test]
    fn reads_drivers_filters_and_language_overrides() {
        let view = parse(
            "*.md diff=markdown\n*.lock merge=ours\n*.enc filter=crypt\n\
             samples/* linguist-vendored\n",
        );

        assert_eq!(view.diff_drivers, vec!["markdown".to_owned()]);
        assert_eq!(view.merge_drivers, vec!["ours".to_owned()]);
        assert_eq!(view.filters, vec!["crypt".to_owned()]);
        assert_eq!(view.linguist.len(), 1);
    }

    #[test]
    fn presents_why_binary_matters() {
        let data = serde_json::to_value(parse("*.pdf binary\n*.rs text\n")).unwrap();

        let lines = GitattributesPresentation.present(&data);

        assert!(lines.iter().any(|line| line.contains("rewrites no bytes")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/gitattributes/repository.gitattributes");

        let data = GitattributesCore.view(&path).unwrap();
        let view: GitattributesView = serde_json::from_value(data).unwrap();

        assert!(view.rules.len() >= 8);
        assert!(view.binary_patterns.len() >= 3);
        assert!(!view.diff_drivers.is_empty());
        assert!(!view.merge_drivers.is_empty());
        assert!(!view.filters.is_empty());
        assert!(!view.linguist.is_empty());
        assert!(view.comments >= 3);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::GitattributesCore),
            plugin_api::PluginPresentation::extensions(&crate::GitattributesPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
