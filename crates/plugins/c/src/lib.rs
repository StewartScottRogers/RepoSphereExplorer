//! C file type plugin: core and presentation halves.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["c", "h"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`CCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level function definitions found in the content.
    pub functions: Vec<String>,
    /// Names of top-level `struct X {` declarations found in the content.
    pub structs: Vec<String>,
}

/// Control-flow keywords that can precede a `(...) {` block without that
/// block being a function definition.
fn is_control_keyword(word: &str) -> bool {
    matches!(word, "if" | "for" | "while" | "switch")
}

/// Extracts the function name from a line that looks like a top-level C
/// function definition, e.g. `int main(void) {` or
/// `void greet(const char *name) {`. Prototypes and calls (which do not end
/// the line with `{`) and control-flow statements are not matched.
fn parse_function_name(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let before_brace = trimmed.strip_suffix('{')?.trim_end();
    let before_paren = before_brace.strip_suffix(')')?;
    let open = before_paren.rfind('(')?;
    let head = before_paren[..open].trim_end();
    let name_start = head
        .rfind(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .map_or(0, |i| i + 1);
    let name = &head[name_start..];
    (!name.is_empty() && !is_control_keyword(name)).then_some(name)
}

/// Extracts the type name from a top-level `struct X {` line, if present.
fn parse_struct_name(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("struct ")?.trim_start();
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0 && rest[end..].trim_start().starts_with('{')).then(|| &rest[..end])
}

/// Parses top-level function and struct names out of `content`, in source
/// order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut structs = Vec::new();
    for line in content.lines() {
        if let Some(name) = parse_struct_name(line) {
            structs.push(name.to_owned());
        } else if let Some(name) = parse_function_name(line) {
            functions.push(name.to_owned());
        }
    }
    (functions, structs)
}

/// Whether `line` contains a `*` immediately followed by an identifier
/// character, C's pointer-declaration/dereference idiom (`Type *name`,
/// `*ptr`). SQL's `SELECT * FROM ...` always separates the `*` from the
/// next word with whitespace, so it does not match.
fn has_pointer_star(line: &str) -> bool {
    line.char_indices().any(|(i, ch)| {
        ch == '*'
            && line[i + 1..]
                .chars()
                .next()
                .is_some_and(|next| next.is_alphanumeric() || next == '_')
    })
}

/// Whether `text` contains a NULL pointer usage in C's syntax: `NULL`
/// appearing on a line that also carries a pointer marker (`Type *name`,
/// `*ptr`, or `->` member access). SQL's `NOT NULL` column constraint
/// carries neither marker, so this excludes it while still matching C's
/// `Type *p = NULL;` / `if (p->next == NULL)` idioms.
fn has_pointer_null_usage(text: &str) -> bool {
    text.lines()
        .any(|line| line.contains("NULL") && (has_pointer_star(line) || line.contains("->")))
}

/// Whether `text` looks like a Dart source file, via the package-URI
/// imports (`import 'dart:...'`/`import 'package:...'`) that appear in
/// virtually every real one. Dart's `void main() {` entry point is
/// textually identical to C's `void main(` idiom.
fn looks_like_dart(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("import 'dart:")
            || line.starts_with("import \"dart:")
            || line.starts_with("import 'package:")
            || line.starts_with("import \"package:")
    })
}

/// Whether `text` looks like C source: preprocessor directives and markers
/// not used by this project's other source-language plugins.
///
/// `printf(` excludes a `@printf(` match: that substring also appears
/// inside Julia's `@printf(...)` standard-library macro call, which is not
/// a call to C's `printf` despite the shared substring.
fn has_c_syntax(text: &str) -> bool {
    if looks_like_dart(text) {
        return false;
    }
    text.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("#include <") || line.starts_with("#include \"")
    }) || text.contains("int main(")
        || text.contains("void main(")
        || (text.contains("printf(") && !text.contains("@printf("))
        || text.contains("malloc(")
        || has_pointer_null_usage(text)
}

/// The C plugin's core half.
#[derive(Debug, Default)]
pub struct CCore;

impl PluginCore for CCore {
    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn name(&self) -> &'static str {
        "c"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_c_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, structs) = parse_definitions(&content);
        let view = CView {
            content,
            truncated,
            functions,
            structs,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The C plugin's presentation half.
#[derive(Debug, Default)]
pub struct CPresentation;

impl PluginPresentation for CPresentation {
    fn name(&self) -> &'static str {
        "c"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "C",
            tint: 0x005c_6bc0,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: CView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.structs.is_empty() {
            lines.push(format!("structs: {}", view.structs.join(", ")));
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
    use super::{CCore, CPresentation, CView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-c-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_an_include_directive_as_c() {
        assert!(CCore.sniff(b"#include <stdio.h>\n\nint main(void) {\n    return 0;\n}\n"));
        assert!(CCore.sniff(b"#include \"local.h\"\n"));
    }

    #[test]
    fn sniffs_common_c_markers_as_c() {
        assert!(CCore.sniff(b"int main(int argc, char **argv) {\n    return 0;\n}\n"));
        assert!(CCore.sniff(b"char *buf = malloc(16);\n"));
        assert!(CCore.sniff(b"printf(\"hi\");\n"));
        assert!(CCore.sniff(b"struct Point *p = NULL;\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_c() {
        assert!(!CCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!CCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!CCore.sniff(b"interface Named {\n  name: string;\n}\n"));
        assert!(!CCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!CCore.sniff(b"package main\n\nfunc main() {}\n"));
        assert!(!CCore.sniff(b"just a regular line of text\n"));
        assert!(!CCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn does_not_sniff_a_sql_schema_with_not_null_as_c() {
        assert!(!CCore.sniff(
            b"CREATE TABLE users (\n    id INTEGER PRIMARY KEY,\n    name TEXT NOT NULL,\n    email TEXT NOT NULL\n);\n"
        ));
    }

    #[test]
    fn views_a_real_c_file_and_extracts_definitions() {
        let path = unique_temp_file("greet.c");
        std::fs::write(
            &path,
            "#include <stdio.h>\n\nstruct Greeter {\n    const char *name;\n};\n\nvoid greet(struct Greeter *g) {\n    if (g->name) {\n        printf(\"Hello, %s!\\n\", g->name);\n    }\n}\n\nint main(void) {\n    struct Greeter g = { \"world\" };\n    greet(&g);\n    return 0;\n}\n",
        )
        .unwrap();

        let data = CCore.view(&path).unwrap();
        let view: CView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.structs, vec!["Greeter"]);
        assert_eq!(view.functions, vec!["greet", "main"]);
        assert!(view.content.contains("Hello, %s!"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.c");
        let mut content = "int pad(void) {\n".to_owned();
        content.push_str(&"/".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = CCore.view(&path).unwrap();
        let view: CView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_structs_functions_and_content() {
        let data = serde_json::to_value(CView {
            content: "struct A {\n};".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            structs: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = CPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["structs: A", "functions: greet", "struct A {", "};"]
        );
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::CCore),
            plugin_api::PluginPresentation::extensions(&crate::CPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
