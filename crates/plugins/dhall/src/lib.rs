//! Dhall file type plugin: core and presentation halves.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["dhall"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// A `https://` import and, when present, the `sha256:` integrity hash
/// pinned to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DhallImport {
    /// The imported address.
    pub url: String,
    /// The hex-encoded SHA-256 hash pinned to the import, when present.
    pub sha256: Option<String>,
}

/// View data produced by [`DhallCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DhallView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Each top-level `let <name> = <value>` binding's name (with its type
    /// annotation, when the binding carries one), in source order.
    pub let_bindings: Vec<String>,
    /// Every `https://` import found, with its `sha256:` integrity hash
    /// where one is pinned to it.
    pub imports: Vec<DhallImport>,
    /// Each lambda parameter's `name : Type` annotation, in source order.
    pub type_annotations: Vec<String>,
    /// The field names (and, for a record type, their types) of the
    /// file's first `{ ... }` record.
    pub record_fields: Vec<String>,
    /// The alternative names (and, for a typed alternative, its type) of
    /// the file's first `< ... >` union.
    pub union_alternatives: Vec<String>,
}

/// The `name : Type` annotations inside every `\(name : Type)` lambda
/// parameter list on `line`, Dhall's own lambda syntax (e.g. `\(x :
/// Natural) -> x`). A marker not used by any sibling plugin.
fn lambda_params(line: &str) -> Vec<String> {
    let mut params = Vec::new();
    let mut search_from = 0;
    while let Some(rel_idx) = line[search_from..].find("\\(") {
        let idx = search_from + rel_idx;
        let after = &line[idx + 2..];
        let Some(close_rel) = after.find(')') else {
            break;
        };
        let inside = after[..close_rel].trim();
        let tail = after[close_rel + 1..].trim_start();
        if inside.contains(':') && tail.starts_with("->") {
            params.push(inside.to_owned());
        }
        search_from = idx + 2;
    }
    params
}

/// Whether `line` opens a Dhall lambda (see [`lambda_params`]).
fn dhall_lambda(line: &str) -> bool {
    !lambda_params(line).is_empty()
}

/// Whether `text` carries a `https://` import with a `sha256:` integrity
/// hash pinned to it - Dhall's own way of pinning an import, and a marker
/// not used by any sibling plugin.
fn has_hashed_import(text: &str) -> bool {
    text.contains("https://") && text.contains("sha256:")
}

/// Whether `text` contains a top-level `let ... in` binding: a line
/// opening `let <name> = ...` followed, inline or on a later bare line, by
/// `in`. Dhall's own `let` requires a matching `in`, unlike a bare
/// assignment statement in most of this project's other sniffed
/// languages.
///
/// The later-line case requires a *bare* `in` line rather than merely one
/// starting with `in`, since a bare two-character line is a much rarer
/// coincidence than a line that merely opens with those two letters -
/// keeping this weaker, shared-vocabulary marker (`let`/`in` are also
/// Haskell's, OCaml's, F#'s and Elm's own `let ... in` expression syntax)
/// from over-claiming a sibling's file. This plugin is placed just after
/// `elm` in `CORE_PLUGINS`, right after `haskell`, `fsharp`, `ocaml` and
/// `nim`, so those plugins' own stronger markers claim a real file of
/// theirs first.
fn has_let_in(text: &str) -> bool {
    let mut saw_let = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("let ") && trimmed.contains('=') {
            saw_let = true;
            if let Some((_, tail)) = trimmed.rsplit_once('=') {
                let tail = tail.trim();
                if tail == "in" || tail.starts_with("in ") || tail.contains(" in ") {
                    return true;
                }
            }
            continue;
        }
        if saw_let && trimmed == "in" {
            return true;
        }
    }
    false
}

/// Whether `text` looks like a Dhall configuration: a lambda (see
/// [`lambda_params`]), a hashed import (see [`has_hashed_import`]), or a
/// `let ... in` binding (see [`has_let_in`]).
fn has_dhall_syntax(text: &str) -> bool {
    text.lines().any(dhall_lambda) || has_hashed_import(text) || has_let_in(text)
}

/// Each top-level `let <name> = <value>` binding's descriptor - everything
/// between `let` and the assigning `=`, which includes a type annotation
/// when the binding carries one (`let x : Natural = 5` yields `x :
/// Natural`).
fn parse_let_bindings(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let rest = trimmed.strip_prefix("let ")?;
            let (name, _) = rest.split_once('=')?;
            let name = name.trim();
            (!name.is_empty()).then(|| name.to_owned())
        })
        .collect()
}

/// Every `https://` import in `text`, paired with the `sha256:` hash
/// immediately following it, when present. Dhall writes the hash as its
/// own whitespace-separated token, on the same line as the import or
/// indented on the next one, so splitting `text` on whitespace finds both
/// regardless of which.
fn parse_imports(text: &str) -> Vec<DhallImport> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| token.starts_with("https://"))
        .map(|(idx, token)| {
            let url = token.trim_end_matches(['.', ',', ')']).to_owned();
            let sha256 = tokens
                .get(idx + 1)
                .and_then(|next| next.strip_prefix("sha256:"))
                .map(|hash| {
                    hash.trim_end_matches(|c: char| !c.is_ascii_hexdigit())
                        .to_owned()
                })
                .filter(|hash| !hash.is_empty());
            DhallImport { url, sha256 }
        })
        .collect()
}

/// The entries of the first `open ... close` block in `text`, split on
/// `separator` - used for a record's fields (`{`/`}`/`,`) and a union's
/// alternatives (`<`/`>`/`|`). Dhall reserves both bracket pairs for
/// exactly these two constructs, so the first occurrence is unambiguous;
/// a *second* record or union elsewhere in the file is not read, an
/// accepted simplification for this hand-rolled parser shared with this
/// project's other structurally-read formats.
fn braced_entries(text: &str, open: char, close: char, separator: char) -> Vec<String> {
    let Some(start) = text.find(open) else {
        return Vec::new();
    };
    let after = &text[start + open.len_utf8()..];
    let Some(end) = after.find(close) else {
        return Vec::new();
    };
    after[..end]
        .split(separator)
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The Dhall plugin's core half.
#[derive(Debug, Default)]
pub struct DhallCore;

impl PluginCore for DhallCore {
    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn name(&self) -> &'static str {
        "dhall"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_dhall_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let view = DhallView {
            let_bindings: parse_let_bindings(&content),
            imports: parse_imports(&content),
            type_annotations: content.lines().flat_map(lambda_params).collect(),
            record_fields: braced_entries(&content, '{', '}', ','),
            union_alternatives: braced_entries(&content, '<', '>', '|'),
            content,
            truncated,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Dhall plugin's presentation half.
#[derive(Debug, Default)]
pub struct DhallPresentation;

impl PluginPresentation for DhallPresentation {
    fn name(&self) -> &'static str {
        "dhall"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "DH",
            tint: 0x0021_a692,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: DhallView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.let_bindings.is_empty() {
            lines.push(format!("let bindings: {}", view.let_bindings.join(", ")));
        }
        if !view.imports.is_empty() {
            let imports: Vec<String> = view
                .imports
                .iter()
                .map(|import| match &import.sha256 {
                    Some(hash) => format!("{} (sha256:{hash})", import.url),
                    None => import.url.clone(),
                })
                .collect();
            lines.push(format!("imports: {}", imports.join(", ")));
        }
        if !view.type_annotations.is_empty() {
            lines.push(format!(
                "type annotations: {}",
                view.type_annotations.join(", ")
            ));
        }
        if !view.record_fields.is_empty() {
            lines.push(format!("record fields: {}", view.record_fields.join(", ")));
        }
        if !view.union_alternatives.is_empty() {
            lines.push(format!(
                "union alternatives: {}",
                view.union_alternatives.join(", ")
            ));
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
    use super::{DhallCore, DhallImport, DhallPresentation, DhallView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-dhall-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_dhall_syntax() {
        assert!(DhallCore.sniff(b"let x = 1 in x\n"));
        assert!(DhallCore.sniff(b"\\(x : Natural) -> x + 1\n"));
        assert!(DhallCore.sniff(
            b"https://prelude.dhall-lang.org/Prelude.dhall\n  sha256:10db3c919c25e9046833df897a096ecad6d17b2ada6ed3ac4ded27f0ceb0f24\n"
        ));
        assert!(DhallCore.sniff(b"let x =\n  1\nin\nx\n"));
    }

    #[test]
    fn does_not_sniff_near_misses_or_other_languages_as_dhall() {
        // An untyped, unparenthesized lambda is not Dhall's own syntax.
        assert!(!DhallCore.sniff(b"\\x -> x\n"));
        // A plain link, with no integrity hash pinned to it.
        assert!(!DhallCore.sniff(b"See https://example.com for details.\n"));
        // English prose using both words, but no assignment and no bare
        // `in` line.
        assert!(!DhallCore.sniff(b"let us go in and see\n"));
        // A Rust `let` binding with no matching `in`.
        assert!(!DhallCore.sniff(b"let mut x = 5;\nprintln!(\"{x}\");\n"));
        assert!(!DhallCore.sniff(b""));
        assert!(!DhallCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_dhall_file_and_extracts_every_field() {
        let path = unique_temp_file("config.dhall");
        std::fs::write(
            &path,
            "let Prelude =\n      https://prelude.dhall-lang.org/v21.1.0/package.dhall\n        sha256:6b90326dc39ab738d7ed87b970ba675c496bed0194071b332840a87261649dc\n\nlet Environment = < Development | Staging | Production : Text >\n\nlet ServiceConfig = { name : Text, port : Natural, environment : Environment }\n\nlet describeEnvironment : Environment -> Text = \\(environment : Environment) -> merge { Development = \"development\", Staging = \"staging\", Production = \\(region : Text) -> \"production (${region})\" } environment\n\nlet service : ServiceConfig = { name = \"repos-explorer-api\", port = 8080, environment = Environment.Production \"us-east-1\" }\n\nin  { service = service, summary = describeEnvironment service.environment }\n",
        )
        .unwrap();

        let data = DhallCore.view(&path).unwrap();
        let view: DhallView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(
            view.let_bindings,
            vec![
                "Prelude",
                "Environment",
                "ServiceConfig",
                "describeEnvironment : Environment -> Text",
                "service : ServiceConfig",
            ]
        );
        assert_eq!(
            view.imports,
            vec![DhallImport {
                url: "https://prelude.dhall-lang.org/v21.1.0/package.dhall".to_owned(),
                sha256: Some(
                    "6b90326dc39ab738d7ed87b970ba675c496bed0194071b332840a87261649dc".to_owned()
                ),
            }]
        );
        assert_eq!(
            view.type_annotations,
            vec!["environment : Environment", "region : Text"]
        );
        assert_eq!(
            view.record_fields,
            vec!["name : Text", "port : Natural", "environment : Environment"]
        );
        assert_eq!(
            view.union_alternatives,
            vec!["Development", "Staging", "Production : Text"]
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.dhall");
        let mut content = "let x =\n".to_owned();
        content.push_str(&"  -- padding\n".repeat(MAX_VIEW_BYTES));
        content.push_str("  1\nin x\n");
        std::fs::write(&path, content).unwrap();

        let data = DhallCore.view(&path).unwrap();
        let view: DhallView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_every_category_and_content() {
        let data = serde_json::to_value(DhallView {
            content: "let x = 1 in x".to_owned(),
            truncated: false,
            let_bindings: vec!["x".to_owned()],
            imports: vec![DhallImport {
                url: "https://example.com/pkg.dhall".to_owned(),
                sha256: Some("abc123".to_owned()),
            }],
            type_annotations: vec!["y : Natural".to_owned()],
            record_fields: vec!["a : Text".to_owned()],
            union_alternatives: vec!["Left".to_owned()],
        })
        .unwrap();

        let lines = DhallPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "let bindings: x",
                "imports: https://example.com/pkg.dhall (sha256:abc123)",
                "type annotations: y : Natural",
                "record fields: a : Text",
                "union alternatives: Left",
                "let x = 1 in x",
            ]
        );
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::DhallCore),
            plugin_api::PluginPresentation::extensions(&crate::DhallPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
