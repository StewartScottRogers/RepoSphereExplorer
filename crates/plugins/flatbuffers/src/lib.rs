//! `FlatBuffers` schema file type plugin: core and presentation halves.
//!
//! `table`, `root_type` and `namespace` declarations, and the
//! `file_identifier` a schema sets - markers no sibling has.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["fbs"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One field of a table or struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// Its name.
    pub name: String,
    /// Its declared type.
    pub kind: String,
    /// Its default, when it states one.
    pub default: Option<String>,
    /// Whether it is marked deprecated - a slot that must be kept for
    /// ever so the ones after it keep their identifiers.
    pub deprecated: bool,
}

/// One table, struct or union.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Declaration {
    /// `table`, `struct`, `enum` or `union`.
    pub keyword: String,
    /// Its name.
    pub name: String,
    /// Its fields, for the kinds that have them.
    pub fields: Vec<Field>,
}

/// View data produced by [`FlatbuffersCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlatbuffersView {
    /// The namespace the types live in.
    pub namespace: Option<String>,
    /// The type a buffer's root is.
    pub root_type: Option<String>,
    /// The four-character identifier written into every buffer.
    pub file_identifier: Option<String>,
    /// The extension the tooling gives a written buffer.
    pub file_extension: Option<String>,
    /// The schemas it includes.
    pub includes: Vec<String>,
    /// The tables, structs, enumerations and unions.
    pub declarations: Vec<Declaration>,
    /// The fields marked deprecated, whose slots can never be reused.
    pub deprecated: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// `line` with any trailing `//` comment removed.
fn uncommented(line: &str) -> &str {
    match line.find("//") {
        Some(at) => &line[..at],
        None => line,
    }
}

/// The name a `keyword Name {` line declares.
fn declaration<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.trim().strip_prefix(keyword)?.strip_prefix(' ')?;
    let name = rest.split([' ', '{', ':']).next()?.trim();
    (!name.is_empty()).then_some(name)
}

/// A `name: type = default;` field, if `line` is one.
fn field(line: &str) -> Option<Field> {
    let trimmed = uncommented(line).trim().trim_end_matches(';').trim();
    let (name, rest) = trimmed.split_once(':')?;
    let name = name.trim();
    if name.is_empty() || name.contains(' ') {
        return None;
    }
    let deprecated = rest.contains("(deprecated");
    // Attributes come after the type in parentheses.
    let rest = rest.split('(').next().unwrap_or(rest).trim();
    let (kind, default) = match rest.split_once('=') {
        Some((kind, value)) => (kind.trim(), Some(value.trim().to_owned())),
        None => (rest, None),
    };
    if kind.is_empty() {
        return None;
    }
    Some(Field {
        name: name.to_owned(),
        kind: kind.to_owned(),
        default,
        deprecated,
    })
}

/// Everything [`FlatbuffersView`] holds, read from `text`.
fn parse(text: &str) -> FlatbuffersView {
    let mut view = FlatbuffersView {
        namespace: None,
        root_type: None,
        file_identifier: None,
        file_extension: None,
        includes: Vec::new(),
        declarations: Vec::new(),
        deprecated: Vec::new(),
        content: String::new(),
        truncated: false,
    };
    let mut open: Option<String> = None;

    for raw in text.lines() {
        let line = uncommented(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('}') {
            open = None;
            continue;
        }
        if let Some(rest) = line.strip_prefix("namespace ") {
            view.namespace = Some(rest.trim_end_matches(';').trim().to_owned());
            continue;
        }
        if let Some(rest) = line.strip_prefix("root_type ") {
            view.root_type = Some(rest.trim_end_matches(';').trim().to_owned());
            continue;
        }
        if line.starts_with("file_identifier ") {
            view.file_identifier = line.split('"').nth(1).map(str::to_owned);
            continue;
        }
        if line.starts_with("file_extension ") {
            view.file_extension = line.split('"').nth(1).map(str::to_owned);
            continue;
        }
        if line.starts_with("include ") {
            if let Some(file) = line.split('"').nth(1) {
                view.includes.push(file.to_owned());
            }
            continue;
        }

        let mut opened = false;
        for keyword in ["table", "struct", "enum", "union"] {
            if let Some(name) = declaration(line, keyword) {
                open = Some(name.to_owned());
                view.declarations.push(Declaration {
                    keyword: keyword.to_owned(),
                    name: name.to_owned(),
                    fields: Vec::new(),
                });
                opened = true;
                break;
            }
        }
        if opened {
            continue;
        }

        if let Some(name) = open.clone()
            && let Some(found) = field(line)
        {
            if found.deprecated {
                view.deprecated.push(format!("{name}.{}", found.name));
            }
            if let Some(entry) = view
                .declarations
                .iter_mut()
                .rev()
                .find(|entry| entry.name == name)
            {
                entry.fields.push(found);
            }
        }
    }
    view
}

/// Whether `text` is a `FlatBuffers` schema.
fn looks_like_it(text: &str) -> bool {
    let has = |keyword: &str| {
        text.lines()
            .any(|line| declaration(uncommented(line), keyword).is_some())
    };
    text.contains("root_type ")
        || text.contains("file_identifier ")
        || (has("table") && text.contains("namespace "))
}

/// The `FlatBuffers` schema plugin's core half.
#[derive(Debug, Default)]
pub struct FlatbuffersCore;

impl PluginCore for FlatbuffersCore {
    fn name(&self) -> &'static str {
        "flatbuffers"
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

/// The `FlatBuffers` schema plugin's presentation half.
#[derive(Debug, Default)]
pub struct FlatbuffersPresentation;

impl PluginPresentation for FlatbuffersPresentation {
    fn name(&self) -> &'static str {
        "flatbuffers"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "FBS",
            tint: 0x0000_bcd4,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: FlatbuffersView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(namespace) = &view.namespace {
            lines.push(format!("Namespace: {namespace}"));
        }
        if let Some(root) = &view.root_type {
            lines.push(format!("A buffer's root is a {root}"));
        }
        match (&view.file_identifier, &view.file_extension) {
            (Some(id), Some(extension)) => {
                lines.push(format!("Written as .{extension}, identified by \"{id}\""));
            }
            (Some(id), None) => lines.push(format!("Identified by \"{id}\"")),
            (None, Some(extension)) => lines.push(format!("Written as .{extension}")),
            (None, None) => {}
        }
        if !view.includes.is_empty() {
            lines.push(format!("Includes: {}", view.includes.join(", ")));
        }
        for entry in &view.declarations {
            lines.push(format!(
                "{} {} ({} field(s))",
                entry.keyword,
                entry.name,
                entry.fields.len()
            ));
            for field in &entry.fields {
                let default = field
                    .default
                    .as_ref()
                    .map_or_else(String::new, |value| format!(" = {value}"));
                let deprecated = if field.deprecated {
                    "  (deprecated)"
                } else {
                    ""
                };
                lines.push(format!(
                    "  {}: {}{default}{deprecated}",
                    field.name, field.kind
                ));
            }
        }
        if !view.deprecated.is_empty() {
            lines.push(
                "Deprecated, and their slots kept for ever so the fields after them".to_owned(),
            );
            lines.push("keep the identifiers they were written with:".to_owned());
            for name in &view.deprecated {
                lines.push(format!("  {name}"));
            }
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{FlatbuffersCore, FlatbuffersPresentation, FlatbuffersView, field, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const SCHEMA: &str = "namespace Example.Orders;\n\
        include \"common.fbs\";\n\
        enum State : byte { Draft = 0, Placed = 1 }\n\
        struct Point { x: float; y: float; }\n\
        table Line {\n  sku: string;\n  quantity: int = 1;\n  price: long (deprecated);\n}\n\
        table Order {\n  id: string (required);\n  lines: [Line];\n  state: State = Draft;\n}\n\
        union Payment { Card, Account }\n\
        root_type Order;\n\
        file_identifier \"ORDR\";\n\
        file_extension \"ord\";\n";

    #[test]
    fn sniffs_a_root_type_or_a_file_identifier() {
        assert!(FlatbuffersCore.sniff(SCHEMA.as_bytes()));
        assert!(FlatbuffersCore.sniff(b"table A { a: int; }\nnamespace X;\n"));
    }

    #[test]
    fn does_not_claim_other_curly_brace_languages() {
        assert!(!FlatbuffersCore.sniff(b"class A { void go() {} }\n"));
        assert!(!FlatbuffersCore.sniff(b""));
    }

    #[test]
    fn reads_a_field_with_its_default_and_attributes() {
        assert_eq!(
            field("  quantity: int = 1;").unwrap().default.as_deref(),
            Some("1")
        );
        assert!(field("  price: long (deprecated);").unwrap().deprecated);
        assert_eq!(
            field("  id: string (required);").unwrap().kind,
            "string",
            "an attribute is not part of the type"
        );
    }

    #[test]
    fn reads_the_root_type_identifier_and_extension() {
        let view = parse(SCHEMA);

        assert_eq!(view.namespace.as_deref(), Some("Example.Orders"));
        assert_eq!(view.root_type.as_deref(), Some("Order"));
        assert_eq!(view.file_identifier.as_deref(), Some("ORDR"));
        assert_eq!(view.file_extension.as_deref(), Some("ord"));
        assert_eq!(view.includes, vec!["common.fbs".to_owned()]);
    }

    #[test]
    fn fields_land_on_the_declaration_that_holds_them() {
        let view = parse(SCHEMA);

        let line = view.declarations.iter().find(|d| d.name == "Line").unwrap();
        let order = view
            .declarations
            .iter()
            .find(|d| d.name == "Order")
            .unwrap();

        assert_eq!(line.fields.len(), 3);
        assert_eq!(order.fields.len(), 3);
        assert!(view.declarations.iter().any(|d| d.keyword == "union"));
        assert!(view.declarations.iter().any(|d| d.keyword == "struct"));
    }

    #[test]
    fn names_the_deprecated_slots() {
        let view = parse(SCHEMA);

        assert_eq!(view.deprecated, vec!["Line.price".to_owned()]);
    }

    #[test]
    fn presents_why_a_deprecated_slot_stays() {
        let data = serde_json::to_value(parse(SCHEMA)).unwrap();

        let lines = FlatbuffersPresentation.present(&data);

        assert!(lines.iter().any(|line| line.contains("kept for ever")));
        assert!(lines.iter().any(|line| line.contains("root is a Order")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/flatbuffers/orders.fbs");

        let data = FlatbuffersCore.view(&path).unwrap();
        let view: FlatbuffersView = serde_json::from_value(data).unwrap();

        assert!(view.namespace.is_some());
        assert!(view.root_type.is_some());
        assert!(view.file_identifier.is_some());
        assert!(view.file_extension.is_some());
        assert!(!view.includes.is_empty());
        assert!(view.declarations.len() >= 5);
        assert!(view.declarations.iter().any(|d| d.keyword == "union"));
        assert!(view.declarations.iter().any(|d| d.keyword == "enum"));
        assert!(view.declarations.iter().any(|d| d.keyword == "struct"));
        assert!(!view.deprecated.is_empty());
        assert!(
            view.declarations
                .iter()
                .any(|d| d.fields.iter().any(|f| f.default.is_some()))
        );
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::FlatbuffersCore),
            plugin_api::PluginPresentation::extensions(&crate::FlatbuffersPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
