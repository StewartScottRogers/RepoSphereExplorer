//! JavaScript file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`JavaScriptCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaScriptView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `function` declarations found in the content.
    pub functions: Vec<String>,
    /// Names of top-level `class` declarations found in the content.
    pub classes: Vec<String>,
}

/// Extracts the identifier following `keyword` (`"function"` or `"class"`) at
/// the start of `line`, if present.
fn top_level_name<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(keyword)?.strip_prefix(' ')?;
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level function and class names out of `content`, in source
/// order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut classes = Vec::new();
    for line in content.lines() {
        if let Some(name) = top_level_name(line, "function") {
            functions.push(name.to_owned());
        } else if let Some(name) = top_level_name(line, "class") {
            classes.push(name.to_owned());
        }
    }
    (functions, classes)
}

/// Whether `text`'s first line is a `node`-flavoured shebang.
fn has_node_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with("#!") && line.contains("node"))
}

/// Whether `line` is an ES module import: a `from '...'`/`from "..."` clause
/// (default/named/namespace imports), or a bare side-effect import
/// terminated with a semicolon (`import './foo';`). Both shapes are absent
/// from Go's `import "pkg"`/`import (` and Swift's `import Foundation`: those
/// have no `from` clause, and neither language terminates statements with a
/// semicolon by convention, so a bare `import <ident-or-string>` line with
/// no `from` and no trailing `;` does not count.
fn is_es_import_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.contains(" from '") || trimmed.contains(" from \"") {
        return true;
    }
    (trimmed.starts_with("import '") && trimmed.ends_with("';"))
        || (trimmed.starts_with("import \"") && trimmed.ends_with("\";"))
}

/// Whether `text` looks like JavaScript source: a top-level `function` or
/// `class` declaration, a `CommonJS` `require(`/`module.exports` marker, an
/// ES module `import`/`export` line, or an arrow function.
fn has_javascript_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        top_level_name(line, "function").is_some() || top_level_name(line, "class").is_some()
    }) || text.contains("require(")
        || text.contains("module.exports")
        || text.contains("=>")
        || text
            .lines()
            .any(|line| is_es_import_line(line) || line.starts_with("export "))
}

/// Whether `text` is a Vue single-file component: a top-level `<template>`
/// block paired with a top-level `<script>` block. A `.vue` SFC's `<script>`
/// block is genuinely JavaScript (or TypeScript), so its `export default {`
/// would otherwise satisfy [`has_javascript_syntax`]'s `export` marker; the
/// paired `<template>` block is the one piece of that file no standalone
/// JavaScript file carries, so it is what rules the SFC out here.
fn looks_like_vue_sfc(text: &str) -> bool {
    let has_template = text.contains("<template>") || text.contains("<template ");
    let has_script = text.contains("<script>") || text.contains("<script ");
    has_template && has_script
}

/// The JavaScript plugin's core half.
#[derive(Debug, Default)]
pub struct JavaScriptCore;

impl PluginCore for JavaScriptCore {
    fn name(&self) -> &'static str {
        "javascript"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        !looks_like_vue_sfc(text) && (has_node_shebang(text) || has_javascript_syntax(text))
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, classes) = parse_definitions(&content);
        let view = JavaScriptView {
            content,
            truncated,
            functions,
            classes,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The JavaScript plugin's presentation half.
#[derive(Debug, Default)]
pub struct JavaScriptPresentation;

impl PluginPresentation for JavaScriptPresentation {
    fn name(&self) -> &'static str {
        "javascript"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: JavaScriptView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.classes.is_empty() {
            lines.push(format!("classes: {}", view.classes.join(", ")));
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
    use super::{JavaScriptCore, JavaScriptPresentation, JavaScriptView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-javascript-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_node_shebang_line_as_javascript() {
        assert!(JavaScriptCore.sniff(b"#!/usr/bin/env node\nconsole.log('hi');\n"));
    }

    #[test]
    fn sniffs_top_level_function_and_class_as_javascript() {
        assert!(JavaScriptCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(JavaScriptCore.sniff(b"class Greeter {\n  constructor() {}\n}\n"));
    }

    #[test]
    fn sniffs_common_js_and_es_module_markers_as_javascript() {
        assert!(JavaScriptCore.sniff(b"const fs = require('fs');\n"));
        assert!(JavaScriptCore.sniff(b"module.exports = { a: 1 };\n"));
        assert!(JavaScriptCore.sniff(b"import fs from 'fs';\n"));
        assert!(JavaScriptCore.sniff(b"export const a = 1;\n"));
        assert!(JavaScriptCore.sniff(b"const add = (a, b) => a + b;\n"));
    }

    #[test]
    fn does_not_sniff_plain_text_as_javascript() {
        assert!(!JavaScriptCore.sniff(b"just a regular line of text\n"));
        assert!(!JavaScriptCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn sniffs_a_bare_side_effect_import_terminated_with_a_semicolon() {
        assert!(JavaScriptCore.sniff(b"import './polyfill';\n"));
        assert!(JavaScriptCore.sniff(b"import \"./polyfill\";\n"));
    }

    #[test]
    fn does_not_sniff_a_go_file_with_an_import_block_as_javascript() {
        assert!(!JavaScriptCore.sniff(
            b"package main\n\nimport (\n\t\"fmt\"\n)\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"
        ));
    }

    #[test]
    fn does_not_sniff_a_swift_file_with_a_bare_import_as_javascript() {
        assert!(!JavaScriptCore.sniff(
            b"import Foundation\n\nfunc greet(name: String) -> String {\n    return \"Hi, \\(name)\"\n}\n"
        ));
    }

    #[test]
    fn does_not_sniff_a_vue_sfc_as_javascript() {
        assert!(!JavaScriptCore.sniff(
            b"<template>\n  <p>{{ greeting }}</p>\n</template>\n\n<script>\nexport default {\n  data() {\n    return { greeting: \"Hello, World\" };\n  },\n};\n</script>\n"
        ));
    }

    #[test]
    fn views_a_real_javascript_file_and_extracts_definitions() {
        let path = unique_temp_file("greet.js");
        std::fs::write(
            &path,
            "'use strict';\n\n\nclass Greeter {\n  constructor() {}\n}\n\n\nfunction greet(name) {\n  return `Hello, ${name}!`;\n}\n",
        )
        .unwrap();

        let data = JavaScriptCore.view(&path).unwrap();
        let view: JavaScriptView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.classes, vec!["Greeter"]);
        assert_eq!(view.functions, vec!["greet"]);
        assert!(view.content.contains("Hello, ${name}!"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.js");
        let mut content = "function pad() {\n".to_owned();
        content.push_str(&"/".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = JavaScriptCore.view(&path).unwrap();
        let view: JavaScriptView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_classes_functions_and_content() {
        let data = serde_json::to_value(JavaScriptView {
            content: "class A {\n}".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            classes: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = JavaScriptPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["classes: A", "functions: greet", "class A {", "}"]
        );
    }
}
