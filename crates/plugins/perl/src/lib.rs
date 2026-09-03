//! Perl file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`PerlCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerlView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `sub` declarations found in the content.
    pub functions: Vec<String>,
    /// Names of top-level `package` declarations found in the content.
    pub packages: Vec<String>,
}

/// Extracts the identifier that follows `keyword` at the start of `line`,
/// e.g. `top_level_name("sub greet {", "sub", is_name_char)` returns
/// `Some("greet")`.
fn top_level_name<'a>(
    line: &'a str,
    keyword: &str,
    is_name_char: impl Fn(char) -> bool,
) -> Option<&'a str> {
    let rest = line.strip_prefix(keyword)?.strip_prefix(' ')?;
    let end = rest.find(|ch| !is_name_char(ch)).unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level sub and package names out of `content`, in source
/// order. Package names may contain `::` namespace separators (e.g.
/// `Data::Greeter`), which sub names may not.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut packages = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        if let Some(name) = top_level_name(trimmed, "package", |ch| {
            ch.is_alphanumeric() || ch == '_' || ch == ':'
        }) {
            packages.push(name.to_owned());
        } else if let Some(name) =
            top_level_name(trimmed, "sub", |ch| ch.is_alphanumeric() || ch == '_')
        {
            functions.push(name.to_owned());
        }
    }
    (functions, packages)
}

/// Whether `text`'s first line is a shebang naming the `perl` interpreter.
fn has_perl_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with("#!") && line.contains("perl"))
}

/// Whether `text` looks like Perl source: markers not used by this
/// project's other source-language plugins. `use strict`/`use warnings` are
/// Perl idioms not sniffed elsewhere; a `package Name;` declaration (with a
/// trailing semicolon) is distinct from the Go plugin's bare `package main`
/// (no semicolon); `sub name {` is Perl's subroutine keyword, unused by any
/// sibling plugin's `function `/`def `/`fn ` checks; `my $` declares a
/// lexical scalar variable and `=~` is Perl's regex-binding operator,
/// neither of which any other sniffed language here uses. This project has
/// no path/extension-based dispatch (per the C plugin's note), so a future
/// Prolog plugin sniffing `.pl` files must avoid these same markers, or be
/// ordered after `perl`.
fn has_perl_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("use strict")
            || trimmed.starts_with("use warnings")
            || trimmed.starts_with("sub ")
            || (trimmed.starts_with("package ") && trimmed.trim_end().ends_with(';'))
    }) || text.contains("my $")
        || text.contains("=~")
}

/// The Perl plugin's core half.
#[derive(Debug, Default)]
pub struct PerlCore;

impl PluginCore for PerlCore {
    fn name(&self) -> &'static str {
        "perl"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_perl_shebang(text) || has_perl_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, packages) = parse_definitions(&content);
        let view = PerlView {
            content,
            truncated,
            functions,
            packages,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Perl plugin's presentation half.
#[derive(Debug, Default)]
pub struct PerlPresentation;

impl PluginPresentation for PerlPresentation {
    fn name(&self) -> &'static str {
        "perl"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: PerlView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.packages.is_empty() {
            lines.push(format!("packages: {}", view.packages.join(", ")));
        }
        if !view.functions.is_empty() {
            lines.push(format!("functions: {}", view.functions.join(", ")));
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
    use super::{MAX_VIEW_BYTES, PerlCore, PerlPresentation, PerlView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-perl-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_perl_shebang_line_as_perl() {
        assert!(PerlCore.sniff(b"#!/usr/bin/env perl\nprint \"hi\\n\";\n"));
        assert!(PerlCore.sniff(b"#!/usr/bin/perl\nprint \"hi\\n\";\n"));
    }

    #[test]
    fn sniffs_common_perl_markers_as_perl() {
        assert!(PerlCore.sniff(b"use strict;\nuse warnings;\n"));
        assert!(PerlCore.sniff(b"package Greeter;\n\nsub greet {\n    return 1;\n}\n"));
        assert!(PerlCore.sniff(b"my $name = 'world';\n"));
        assert!(PerlCore.sniff(b"if ($line =~ /^\\d+$/) {\n    print \"number\\n\";\n}\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_perl() {
        assert!(!PerlCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!PerlCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!PerlCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!PerlCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!PerlCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!PerlCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!PerlCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!PerlCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!PerlCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!PerlCore.sniff(b"fun greet() {\n    println(\"hi\")\n}\n"));
        assert!(!PerlCore.sniff(b"just a regular line of text\n"));
        assert!(!PerlCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_perl_file_and_extracts_definitions() {
        let path = unique_temp_file("greeter.pl");
        std::fs::write(
            &path,
            "use strict;\nuse warnings;\n\npackage Greeter;\n\nsub greet {\n    my $name = shift;\n    return \"Hello, $name!\";\n}\n\nprint greet('world');\n",
        )
        .unwrap();

        let data = PerlCore.view(&path).unwrap();
        let view: PerlView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.packages, vec!["Greeter"]);
        assert_eq!(view.functions, vec!["greet"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.pl");
        let mut content = "use strict;\n".to_owned();
        content.push_str(&"#".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = PerlCore.view(&path).unwrap();
        let view: PerlView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_packages_functions_and_content() {
        let data = serde_json::to_value(PerlView {
            content: "package A;\nsub greet {\n}".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            packages: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = PerlPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "packages: A",
                "functions: greet",
                "package A;",
                "sub greet {",
                "}"
            ]
        );
    }
}
