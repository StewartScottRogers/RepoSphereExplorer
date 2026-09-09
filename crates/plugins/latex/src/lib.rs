//! `LaTeX` file type plugin: core and presentation halves.
//!
//! Read line by line for the structure a reader wants - the class, the
//! packages, the section outline, the labels - rather than expanded, which
//! would need the whole TeX engine and would still refuse a document that
//! does not compile.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["tex", "sty", "cls", "ltx"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One sectioning command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    /// `part` is 0, `chapter` 1, `section` 2, and so on down.
    pub level: u8,
    /// The title in the command's braces.
    pub title: String,
}

/// View data produced by [`LatexCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatexView {
    /// The argument of `\documentclass`.
    pub class: Option<String>,
    /// Its bracketed options, if any.
    pub class_options: Vec<String>,
    /// Every package named in a `\usepackage`.
    pub packages: Vec<String>,
    /// The sectioning outline, in document order.
    pub sections: Vec<Section>,
    /// The `\label{...}` names defined.
    pub labels: Vec<String>,
    /// The `\ref`/`\eqref`/`\cref` targets used.
    pub references: Vec<String>,
    /// The `\begin{...}` environments used, each once.
    pub environments: Vec<String>,
    /// The bibliography file named by `\bibliography` or `\addbibresource`.
    pub bibliography: Option<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The sectioning commands, deepest last.
const SECTIONS: &[&str] = &[
    "part",
    "chapter",
    "section",
    "subsection",
    "subsubsection",
    "paragraph",
    "subparagraph",
];

/// The text inside the first `{...}` after `at`, honouring nesting.
fn braced(text: &str, at: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let open = chars.iter().skip(at).position(|&c| c == '{')? + at;
    let mut depth = 0usize;
    for (index, &c) in chars.iter().enumerate().skip(open) {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(chars[open + 1..index].iter().collect());
                }
            }
            _ => {}
        }
    }
    None
}

/// The text inside a `[...]` immediately after `at`, if there is one.
fn bracketed(text: &str, at: usize) -> Option<String> {
    let rest = text.get(at..)?.trim_start();
    let inner = rest.strip_prefix('[')?;
    inner.split(']').next().map(str::to_owned)
}

/// Every argument of a command, wherever it appears in `line`.
fn arguments_of(line: &str, command: &str) -> Vec<String> {
    let needle = format!("\\{command}");
    let mut found = Vec::new();
    let mut from = 0usize;
    while let Some(at) = line[from..].find(&needle) {
        let at = from + at;
        // `\ref` must not match inside `\refstepcounter`, and
        // `\section` must not also match the `\section*` the caller
        // asks for separately - or every starred heading is counted twice.
        let after = at + needle.len();
        let next = line[after..].chars().next();
        if next.is_some_and(|c| c.is_alphabetic() || c == '*') {
            from = after;
            continue;
        }
        if let Some(argument) = braced(line, after) {
            found.push(argument);
        }
        from = after;
    }
    found
}

/// Everything [`LatexView`] holds, read from `text`.
fn parse(text: &str) -> LatexView {
    let mut view = LatexView {
        class: None,
        class_options: Vec::new(),
        packages: Vec::new(),
        sections: Vec::new(),
        labels: Vec::new(),
        references: Vec::new(),
        environments: Vec::new(),
        bibliography: None,
        content: String::new(),
        truncated: false,
    };

    for raw in text.lines() {
        // A `%` opens a comment unless it is escaped.
        let line = match raw.find('%') {
            Some(0) => continue,
            Some(at) if !raw[..at].ends_with('\\') => &raw[..at],
            _ => raw,
        };

        if let Some(at) = line.find("\\documentclass") {
            let after = at + "\\documentclass".len();
            view.class_options = bracketed(line, after)
                .map(|options| {
                    options
                        .split(',')
                        .map(|option| option.trim().to_owned())
                        .filter(|option| !option.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            view.class = braced(line, after);
        }
        for package in arguments_of(line, "usepackage") {
            for name in package.split(',') {
                let name = name.trim().to_owned();
                if !name.is_empty() && !view.packages.contains(&name) {
                    view.packages.push(name);
                }
            }
        }
        for (level, command) in SECTIONS.iter().enumerate() {
            for title in arguments_of(line, command) {
                if let Ok(level) = u8::try_from(level) {
                    view.sections.push(Section { level, title });
                }
            }
            // `\section*` is unnumbered and still a section.
            for title in arguments_of(line, &format!("{command}*")) {
                if let Ok(level) = u8::try_from(level) {
                    view.sections.push(Section { level, title });
                }
            }
        }
        view.labels.extend(arguments_of(line, "label"));
        for command in ["ref", "eqref", "cref", "autoref", "cite"] {
            view.references.extend(arguments_of(line, command));
        }
        for environment in arguments_of(line, "begin") {
            if !view.environments.contains(&environment) {
                view.environments.push(environment);
            }
        }
        for command in ["bibliography", "addbibresource"] {
            if let Some(file) = arguments_of(line, command).into_iter().next() {
                view.bibliography = Some(file);
            }
        }
    }
    view
}

/// Whether `text` looks like `LaTeX`.
fn looks_like_it(text: &str) -> bool {
    let mut packages = 0usize;
    for line in text.lines() {
        if line.contains("\\documentclass")
            || line.contains("\\begin{document}")
            || line.contains("\\newcommand")
            || line.contains("\\ProvidesPackage")
        {
            return true;
        }
        if line.contains("\\usepackage") {
            packages += 1;
        }
    }
    packages >= 2
}

/// The `LaTeX` plugin's core half.
#[derive(Debug, Default)]
pub struct LatexCore;

impl PluginCore for LatexCore {
    fn name(&self) -> &'static str {
        "latex"
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

/// The `LaTeX` plugin's presentation half.
#[derive(Debug, Default)]
pub struct LatexPresentation;

impl PluginPresentation for LatexPresentation {
    fn name(&self) -> &'static str {
        "latex"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "TEX",
            tint: 0x0000_8080,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: LatexView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(class) = &view.class {
            let options = if view.class_options.is_empty() {
                String::new()
            } else {
                format!(" [{}]", view.class_options.join(", "))
            };
            lines.push(format!("Document class: {class}{options}"));
        }
        if !view.packages.is_empty() {
            lines.push(format!(
                "Packages ({}): {}",
                view.packages.len(),
                view.packages.join(", ")
            ));
        }
        if !view.sections.is_empty() {
            lines.push(format!("Outline ({}):", view.sections.len()));
            for section in &view.sections {
                lines.push(format!(
                    "{}{}",
                    "  ".repeat(usize::from(section.level) + 1),
                    section.title
                ));
            }
        }
        if !view.environments.is_empty() {
            lines.push(format!("Environments: {}", view.environments.join(", ")));
        }
        if !view.labels.is_empty() {
            lines.push(format!(
                "Labels ({}): {}",
                view.labels.len(),
                view.labels.join(", ")
            ));
        }
        if !view.references.is_empty() {
            lines.push(format!("References: {}", view.references.join(", ")));
        }
        if let Some(bibliography) = &view.bibliography {
            lines.push(format!("Bibliography: {bibliography}"));
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{LatexCore, LatexPresentation, LatexView, braced, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_a_document_class_or_several_packages() {
        assert!(LatexCore.sniff(b"\\documentclass{article}\n"));
        assert!(LatexCore.sniff(b"\\begin{document}\n"));
        assert!(LatexCore.sniff(b"\\usepackage{a}\n\\usepackage{b}\n"));
    }

    #[test]
    fn does_not_claim_a_stray_backslash() {
        assert!(!LatexCore.sniff(b"C:\\Users\\Stewart\n"));
        assert!(!LatexCore.sniff(b"just prose\n"));
        assert!(!LatexCore.sniff(b""));
    }

    #[test]
    fn braces_are_matched_by_depth_not_by_the_first_close() {
        assert_eq!(
            braced("\\title{a {nested} title}", 6).unwrap(),
            "a {nested} title"
        );
    }

    #[test]
    fn reads_the_class_with_its_options() {
        let view = parse("\\documentclass[11pt,a4paper]{article}\n");

        assert_eq!(view.class.as_deref(), Some("article"));
        assert_eq!(
            view.class_options,
            vec!["11pt".to_owned(), "a4paper".to_owned()]
        );
    }

    #[test]
    fn reads_the_outline_including_starred_sections() {
        let view = parse("\\section{One}\n\\subsection{Two}\n\\section*{Unnumbered}\n");

        assert_eq!(view.sections.len(), 3);
        assert_eq!(view.sections[0].level, 2);
        assert_eq!(view.sections[1].level, 3);
    }

    #[test]
    fn a_commented_line_contributes_nothing() {
        let view = parse("% \\section{Not real}\n\\section{Real}\n");

        assert_eq!(view.sections.len(), 1);
        assert_eq!(view.sections[0].title, "Real");
    }

    #[test]
    fn reads_packages_labels_references_and_the_bibliography() {
        let view = parse(
            "\\usepackage{amsmath,graphicx}\n\\begin{figure}\n\\label{fig:one}\n\
             See \\ref{fig:one} and \\cite{knuth}.\n\\bibliography{refs}\n",
        );

        assert_eq!(
            view.packages,
            vec!["amsmath".to_owned(), "graphicx".to_owned()]
        );
        assert_eq!(view.labels, vec!["fig:one".to_owned()]);
        assert!(view.references.contains(&"knuth".to_owned()));
        assert_eq!(view.environments, vec!["figure".to_owned()]);
        assert_eq!(view.bibliography.as_deref(), Some("refs"));
    }

    #[test]
    fn presents_the_class_before_the_outline() {
        let data = serde_json::to_value(parse("\\documentclass{article}\n\\section{S}\n")).unwrap();

        let lines = LatexPresentation.present(&data);

        assert_eq!(lines[0], "Document class: article");
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/latex/paper.tex");

        let data = LatexCore.view(&path).unwrap();
        let view: LatexView = serde_json::from_value(data).unwrap();

        assert!(view.class.is_some());
        assert!(!view.class_options.is_empty());
        assert!(view.packages.len() >= 4);
        assert!(view.sections.len() >= 4);
        assert!(!view.labels.is_empty());
        assert!(!view.references.is_empty());
        assert!(view.environments.len() >= 3);
        assert!(view.bibliography.is_some());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::LatexCore),
            plugin_api::PluginPresentation::extensions(&crate::LatexPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
