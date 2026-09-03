//! PHP file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`PhpCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhpView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `function` declarations found in the content.
    pub functions: Vec<String>,
    /// Names of top-level `class` declarations found in the content.
    pub classes: Vec<String>,
}

/// Extracts the identifier that follows `keyword` at the start of `line`,
/// e.g. `top_level_name("function greet(", "function")` returns
/// `Some("greet")`.
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

/// Whether `text` contains a PHP opening tag (`<?php` or the short echo tag
/// `<?=`), the one marker unique to PHP source and carried by essentially
/// every real `.php` file. This project's other source-language plugins
/// sniff bare `function `/`class ` or `require '`/`require "` lines at the
/// start of a line, all of which ordinary PHP code also produces (a
/// top-level `function greet($name) {`, a top-level `class Greeter {`, or a
/// Composer `require 'vendor/autoload.php';`), so this plugin is placed
/// first in `CORE_PLUGINS` to claim PHP files before ruby, python, or
/// javascript can steal them on those shared substrings.
fn has_php_open_tag(text: &str) -> bool {
    text.contains("<?php") || text.contains("<?=")
}

/// The PHP plugin's core half.
#[derive(Debug, Default)]
pub struct PhpCore;

impl PluginCore for PhpCore {
    fn name(&self) -> &'static str {
        "php"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_php_open_tag(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, classes) = parse_definitions(&content);
        let view = PhpView {
            content,
            truncated,
            functions,
            classes,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The PHP plugin's presentation half.
#[derive(Debug, Default)]
pub struct PhpPresentation;

impl PluginPresentation for PhpPresentation {
    fn name(&self) -> &'static str {
        "php"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: PhpView = match serde_json::from_value(data.clone()) {
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
    use super::{MAX_VIEW_BYTES, PhpCore, PhpPresentation, PhpView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-php-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_php_open_tag_as_php() {
        assert!(PhpCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(PhpCore.sniff(b"<html>\n<?php echo 'hi'; ?>\n</html>\n"));
        assert!(PhpCore.sniff(b"<?= $name ?>\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_php() {
        assert!(!PhpCore.sniff(b"function greet($name) {\n  return 1;\n}\n"));
        assert!(!PhpCore.sniff(b"class Greeter {\n  public function greet() {}\n}\n"));
        assert!(!PhpCore.sniff(b"require 'json'\n"));
        assert!(!PhpCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!PhpCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!PhpCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!PhpCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!PhpCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!PhpCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!PhpCore.sniff(b"fun greet() {\n    println(\"hi\")\n}\n"));
        assert!(!PhpCore.sniff(b"just a regular line of text\n"));
        assert!(!PhpCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_php_file_and_extracts_definitions() {
        let path = unique_temp_file("greeter.php");
        std::fs::write(
            &path,
            "<?php\n\nclass Greeter {\n}\n\nfunction greet($name) {\n    return \"Hello, $name!\";\n}\n\necho greet('world');\n",
        )
        .unwrap();

        let data = PhpCore.view(&path).unwrap();
        let view: PhpView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.classes, vec!["Greeter"]);
        assert_eq!(view.functions, vec!["greet"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.php");
        let mut content = "<?php\n".to_owned();
        content.push_str(&"#".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = PhpCore.view(&path).unwrap();
        let view: PhpView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_classes_functions_and_content() {
        let data = serde_json::to_value(PhpView {
            content: "<?php\nclass A {\n}".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            classes: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = PhpPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["classes: A", "functions: greet", "<?php", "class A {", "}"]
        );
    }
}
