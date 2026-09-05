//! Prolog file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`PrologCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrologView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of predicates defined by a clause head in the content, in
    /// source order, without duplicates.
    pub predicates: Vec<String>,
}

/// Extracts the lowercase atom name at the start of `text`, if any.
fn atom_name(text: &str) -> Option<&str> {
    let end = text.find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))?;
    (end > 0 && text.as_bytes()[0].is_ascii_lowercase()).then(|| &text[..end])
}

/// Extracts the predicate name from `line` if it's a directive (`:- name(`)
/// or a clause head (`name(` ... or `name :-` or `name.`) at column zero —
/// an indented line is a clause *body* goal, not a head, since Prolog has
/// no other syntax marking top-level structure the way braces or
/// significant top-level indentation do in other sniffed languages.
fn clause_or_directive_name(line: &str) -> Option<&str> {
    if let Some(rest) = line.strip_prefix(":-") {
        let rest = rest.trim_start();
        let name = atom_name(rest)?;
        return rest[name.len()..].starts_with('(').then_some(name);
    }
    let name = atom_name(line)?;
    let rest = &line[name.len()..];
    (rest.starts_with('(') || rest.trim_start().starts_with(":-") || rest.starts_with('.'))
        .then_some(name)
}

/// Parses the head predicate name out of each top-level clause or directive
/// in `content`, in source order, skipping duplicates.
fn parse_predicates(content: &str) -> Vec<String> {
    let mut predicates = Vec::new();
    for line in content.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        if let Some(name) = clause_or_directive_name(line)
            && !predicates.iter().any(|existing| existing == name)
        {
            predicates.push(name.to_owned());
        }
    }
    predicates
}

/// Whether `text`'s first line is a shebang naming a Prolog interpreter
/// (`swipl`, `gprolog`, or `prolog`).
fn has_prolog_shebang(text: &str) -> bool {
    text.lines().next().is_some_and(|line| {
        line.starts_with("#!")
            && (line.contains("swipl") || line.contains("gprolog") || line.contains("prolog"))
    })
}

/// Whether `text` looks like Prolog source: markers not used by this
/// project's other source-language plugins. The `:-` rule-neck operator
/// (separating a clause head from its body) and the `?-` query prompt are
/// Prolog-specific two-character sequences no sibling plugin sniffs on —
/// notably distinct from the Erlang plugin's own single-dash `-module(`
/// attribute forms. Perl and Prolog both commonly use the `.pl` extension,
/// but this project sniffs by content only, so this plugin is ordered after
/// `perl` in `CORE_PLUGINS` per that plugin's own note, and does not sniff
/// any of Perl's own markers.
fn has_prolog_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with(":-") || trimmed.starts_with("?-")
    }) || text.contains(") :-")
}

/// The Prolog plugin's core half.
#[derive(Debug, Default)]
pub struct PrologCore;

impl PluginCore for PrologCore {
    fn name(&self) -> &'static str {
        "prolog"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_prolog_shebang(text) || has_prolog_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let predicates = parse_predicates(&content);
        let view = PrologView {
            content,
            truncated,
            predicates,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Prolog plugin's presentation half.
#[derive(Debug, Default)]
pub struct PrologPresentation;

impl PluginPresentation for PrologPresentation {
    fn name(&self) -> &'static str {
        "prolog"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: PrologView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.predicates.is_empty() {
            lines.push(format!("predicates: {}", view.predicates.join(", ")));
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
    use super::{MAX_VIEW_BYTES, PrologCore, PrologPresentation, PrologView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-prolog-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_prolog_shebang_line_as_prolog() {
        assert!(PrologCore.sniff(b"#!/usr/bin/env swipl\n:- initialization(main).\n"));
        assert!(PrologCore.sniff(b"#!/usr/bin/gprolog --consult-file\n"));
    }

    #[test]
    fn sniffs_common_prolog_markers_as_prolog() {
        assert!(PrologCore.sniff(b":- module(greeter, [greet/1]).\n"));
        assert!(PrologCore.sniff(b"?- greet(world).\n"));
        assert!(PrologCore.sniff(b"greet(Name) :-\n    format(\"Hello, ~w!~n\", [Name]).\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_prolog() {
        assert!(!PrologCore.sniff(b"use strict;\nuse warnings;\n\npackage Greeter;\n"));
        assert!(!PrologCore.sniff(b"sub greet {\n    my $name = shift;\n}\n"));
        assert!(!PrologCore.sniff(b"-module(greeter).\n-export([greet/1]).\n"));
        assert!(!PrologCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!PrologCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!PrologCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!PrologCore.sniff(b"just a regular line of text\n"));
        assert!(!PrologCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_prolog_file_and_extracts_predicates() {
        let path = unique_temp_file("greeter.pl");
        std::fs::write(
            &path,
            ":- module(greeter, [greet/1]).\n\ngreet(Name) :-\n    format(\"Hello, ~w!~n\", [Name]).\n\nfarewell(Name) :-\n    format(\"Bye, ~w!~n\", [Name]).\n",
        )
        .unwrap();

        let data = PrologCore.view(&path).unwrap();
        let view: PrologView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.predicates, vec!["module", "greet", "farewell"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.pl");
        let mut content = ":- module(large, []).\n".to_owned();
        content.push_str(&"%".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = PrologCore.view(&path).unwrap();
        let view: PrologView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_predicates_and_content() {
        let data = serde_json::to_value(PrologView {
            content: "greet(Name) :-\n    true.".to_owned(),
            truncated: false,
            predicates: vec!["greet".to_owned()],
        })
        .unwrap();

        let lines = PrologPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["predicates: greet", "greet(Name) :-", "    true."]
        );
    }
}
