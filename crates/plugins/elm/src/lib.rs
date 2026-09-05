//! Elm file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`ElmCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElmView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level declarations found via their type signature
    /// (`name : Type`), in source order.
    pub declarations: Vec<String>,
}

/// Whether `line` is a top-level Elm type signature, e.g. `view : Model ->
/// Html Msg`. Elm type signatures sit at column zero, start with a
/// lowercase identifier, and use a single colon surrounded by spaces
/// (unlike Haskell's `::`).
fn is_type_signature(line: &str) -> bool {
    if line != line.trim_start() {
        return false;
    }
    let Some((name, rest)) = line.split_once(" : ") else {
        return false;
    };
    !name.is_empty()
        && name.starts_with(|ch: char| ch.is_ascii_lowercase() || ch == '_')
        && name
            .chars()
            .all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '\'')
        && !rest.trim().is_empty()
}

/// Parses top-level type-signature names out of `content`, in source order.
fn parse_declarations(content: &str) -> Vec<String> {
    content
        .lines()
        .filter(|line| is_type_signature(line))
        .map(|line| line.split_once(" : ").unwrap().0.to_owned())
        .collect()
}

/// Whether `text` looks like Elm source: markers not used by this project's
/// other source-language plugins. `type alias ` names a type synonym in
/// Elm's own vocabulary (Haskell's equivalent type-synonym syntax has no
/// `alias` keyword); ` exposing (` is how both `module` headers and
/// `import` statements restrict their export/import list in Elm, a keyword
/// no sibling plugin's own module syntax uses; and a top-level type
/// signature with a single ` : ` (see [`is_type_signature`]) is Elm's own
/// convention, distinct from Haskell's double-colon `::`.
fn has_elm_syntax(text: &str) -> bool {
    text.contains("type alias ")
        || text.contains(" exposing (")
        || text.lines().any(is_type_signature)
}

/// The Elm plugin's core half.
#[derive(Debug, Default)]
pub struct ElmCore;

impl PluginCore for ElmCore {
    fn name(&self) -> &'static str {
        "elm"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_elm_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let declarations = parse_declarations(&content);
        let view = ElmView {
            content,
            truncated,
            declarations,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Elm plugin's presentation half.
#[derive(Debug, Default)]
pub struct ElmPresentation;

impl PluginPresentation for ElmPresentation {
    fn name(&self) -> &'static str {
        "elm"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: ElmView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.declarations.is_empty() {
            lines.push(format!("declarations: {}", view.declarations.join(", ")));
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
    use super::{ElmCore, ElmPresentation, ElmView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-elm-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_elm_markers_as_elm() {
        assert!(ElmCore.sniff(b"module Main exposing (main)\n"));
        assert!(ElmCore.sniff(b"import Html exposing (text)\n"));
        assert!(ElmCore.sniff(b"type alias Model =\n    Int\n"));
        assert!(ElmCore.sniff(b"view : Model -> Html Msg\nview model =\n    text \"hi\"\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_elm() {
        assert!(!ElmCore.sniff(b"greet :: String -> String\ngreet name = name\n"));
        assert!(!ElmCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!ElmCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!ElmCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!ElmCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!ElmCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!ElmCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!ElmCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!ElmCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!ElmCore.sniff(b"let x : int = 5\n"));
        assert!(!ElmCore.sniff(b"just a regular line of text\n"));
        assert!(!ElmCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_elm_file_and_extracts_declarations() {
        let path = unique_temp_file("main.elm");
        std::fs::write(
            &path,
            "module Main exposing (main)\n\nimport Html exposing (text)\n\ntype alias Model =\n    Int\n\nview : Model -> Html.Html msg\nview model =\n    text (String.fromInt model)\n\nmain : Html.Html msg\nmain =\n    view 0\n",
        )
        .unwrap();

        let data = ElmCore.view(&path).unwrap();
        let view: ElmView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.declarations, vec!["view", "main"]);
        assert!(view.content.contains("Html.Html"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.elm");
        let mut content = "main : Int\n".to_owned();
        content.push_str(&"-- ".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = ElmCore.view(&path).unwrap();
        let view: ElmView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_declarations_and_content() {
        let data = serde_json::to_value(ElmView {
            content: "main : Int\nmain =\n    0".to_owned(),
            truncated: false,
            declarations: vec!["main".to_owned()],
        })
        .unwrap();

        let lines = ElmPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["declarations: main", "main : Int", "main =", "    0"]
        );
    }
}
