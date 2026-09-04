//! LaTeX file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// Substrings that mark a file as LaTeX source — a `\documentclass`
/// preamble declaration, a `\begin{document}`/`\end{document}` body
/// delimiter, a `\usepackage` import, a `\newcommand` definition, a
/// `\maketitle` command, or a `\section{` sectioning command — none used
/// by any sibling plugin.
const MARKERS: &[&str] = &[
    "\\documentclass",
    "\\begin{document}",
    "\\end{document}",
    "\\usepackage",
    "\\newcommand",
    "\\maketitle",
    "\\section{",
];

/// View data produced by [`LatexCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatexView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Titles from `\chapter`/`\section`/`\subsection`/`\subsubsection`
    /// commands found in the content, in source order.
    pub sections: Vec<String>,
    /// Names from `\usepackage` imports found in the content, in source
    /// order.
    pub packages: Vec<String>,
}

/// Extracts the title from a `\chapter{...}`, `\section{...}`,
/// `\subsection{...}`, or `\subsubsection{...}` sectioning command, or
/// `None` if `trimmed` is not such a command.
fn parse_section_title(trimmed: &str) -> Option<&str> {
    const COMMANDS: &[&str] = &[
        "\\chapter{",
        "\\subsubsection{",
        "\\subsection{",
        "\\section{",
    ];
    for command in COMMANDS {
        if let Some(rest) = trimmed.strip_prefix(command) {
            let end = rest.find('}')?;
            let title = &rest[..end];
            return (!title.is_empty()).then_some(title);
        }
    }
    None
}

/// Extracts the package names from a `\usepackage{...}` or
/// `\usepackage[options]{...}` import line, or `None` if `trimmed` is not
/// such an import.
fn parse_package_names(trimmed: &str) -> Option<Vec<String>> {
    let rest = trimmed.strip_prefix("\\usepackage")?;
    let rest = match rest.strip_prefix('[') {
        Some(after_bracket) => {
            let end = after_bracket.find(']')?;
            &after_bracket[end + 1..]
        }
        None => rest,
    };
    let rest = rest.strip_prefix('{')?;
    let end = rest.find('}')?;
    Some(
        rest[..end]
            .split(',')
            .map(|name| name.trim().to_owned())
            .collect(),
    )
}

/// Parses sectioning and `\usepackage` commands out of `content`, in
/// source order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut sections = Vec::new();
    let mut packages = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(title) = parse_section_title(trimmed) {
            sections.push(title.to_owned());
        } else if let Some(names) = parse_package_names(trimmed) {
            packages.extend(names);
        }
    }
    (sections, packages)
}

/// Whether `text` looks like LaTeX source, per [`MARKERS`].
fn has_latex_syntax(text: &str) -> bool {
    MARKERS.iter().any(|marker| text.contains(marker))
}

/// The LaTeX plugin's core half.
#[derive(Debug, Default)]
pub struct LatexCore;

impl PluginCore for LatexCore {
    fn name(&self) -> &'static str {
        "latex"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_latex_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (sections, packages) = parse_definitions(&content);
        let view = LatexView {
            content,
            truncated,
            sections,
            packages,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The LaTeX plugin's presentation half.
#[derive(Debug, Default)]
pub struct LatexPresentation;

impl PluginPresentation for LatexPresentation {
    fn name(&self) -> &'static str {
        "latex"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: LatexView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.packages.is_empty() {
            lines.push(format!("packages: {}", view.packages.join(", ")));
        }
        if !view.sections.is_empty() {
            lines.push(format!("sections: {}", view.sections.join(", ")));
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
    use super::{LatexCore, LatexPresentation, LatexView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-latex-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_latex_markers_as_latex() {
        assert!(LatexCore.sniff(b"\\documentclass{article}\n"));
        assert!(LatexCore.sniff(b"\\begin{document}\nHello\n\\end{document}\n"));
        assert!(LatexCore.sniff(b"\\usepackage{amsmath}\n"));
        assert!(LatexCore.sniff(b"\\newcommand{\\vect}[1]{\\mathbf{#1}}\n"));
        assert!(LatexCore.sniff(b"\\maketitle\n"));
        assert!(LatexCore.sniff(b"\\section{Introduction}\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_latex() {
        assert!(!LatexCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!LatexCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!LatexCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!LatexCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!LatexCore.sniff(b"vim9script\n\nvar x = 1\n"));
        assert!(!LatexCore.sniff(b"just a regular line of text\n"));
        assert!(!LatexCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_latex_file_and_extracts_definitions() {
        let path = unique_temp_file("paper.tex");
        std::fs::write(
            &path,
            "\\documentclass{article}\n\\usepackage{amsmath}\n\\usepackage[utf8]{inputenc}\n\\begin{document}\n\\section{Introduction}\nSome text.\n\\subsection{Background}\nMore text.\n\\end{document}\n",
        )
        .unwrap();

        let data = LatexCore.view(&path).unwrap();
        let view: LatexView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.packages, vec!["amsmath", "inputenc"]);
        assert_eq!(view.sections, vec!["Introduction", "Background"]);
        assert!(view.content.contains("Some text."));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.tex");
        let mut content = "\\documentclass{article}\n".to_owned();
        content.push_str(&"% a comment line\n".repeat(MAX_VIEW_BYTES));
        std::fs::write(&path, content).unwrap();

        let data = LatexCore.view(&path).unwrap();
        let view: LatexView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_packages_sections_and_content() {
        let data = serde_json::to_value(LatexView {
            content: "\\section{Intro}\nHello".to_owned(),
            truncated: false,
            sections: vec!["Intro".to_owned()],
            packages: vec!["amsmath".to_owned()],
        })
        .unwrap();

        let lines = LatexPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "packages: amsmath",
                "sections: Intro",
                "\\section{Intro}",
                "Hello"
            ]
        );
    }
}
