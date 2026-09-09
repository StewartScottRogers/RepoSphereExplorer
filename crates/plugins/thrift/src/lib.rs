//! Apache Thrift IDL file type plugin: core and presentation halves.
//!
//! `namespace` lines with `struct`, `service` or `exception`
//! declarations, whose fields carry explicit numbers - a shape no sibling
//! has.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["thrift"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One numbered field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// Its wire number.
    pub number: i32,
    /// `required`, `optional`, or `default` when it says neither - and
    /// the default is the one that surprises people.
    pub requiredness: String,
    /// Its declared type.
    pub kind: String,
    /// Its name.
    pub name: String,
}

/// One declared type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Declaration {
    /// `struct`, `union`, `exception` or `enum`.
    pub keyword: String,
    /// Its name.
    pub name: String,
    /// Its fields, for the kinds that have numbered ones.
    pub fields: Vec<Field>,
}

/// One service method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Method {
    /// Its name.
    pub name: String,
    /// Its return type.
    pub returns: String,
    /// Whether it is `oneway`, and so has no reply at all.
    pub oneway: bool,
    /// The exceptions it declares it can throw.
    pub throws: Vec<String>,
}

/// One service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    /// Its name.
    pub name: String,
    /// The service it extends, when it extends one.
    pub extends: Option<String>,
    /// Its methods.
    pub methods: Vec<Method>,
}

/// View data produced by [`ThriftCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThriftView {
    /// The namespaces, as `language name`.
    pub namespaces: Vec<String>,
    /// The files it includes.
    pub includes: Vec<String>,
    /// The typedefs, as `from -> to`.
    pub typedefs: Vec<String>,
    /// The structs, unions, exceptions and enumerations.
    pub declarations: Vec<Declaration>,
    /// The services.
    pub services: Vec<Service>,
    /// Fields that state neither `required` nor `optional`, whose
    /// behaviour differs between language bindings.
    pub unspecified_requiredness: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// `line` with any trailing comment removed.
fn uncommented(line: &str) -> &str {
    for marker in ["//", "#"] {
        if let Some(at) = line.find(marker) {
            return &line[..at];
        }
    }
    line
}

/// The name a `keyword Name {` line declares.
fn declaration<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.trim().strip_prefix(keyword)?.strip_prefix(' ')?;
    let name = rest.split([' ', '{']).next()?.trim();
    (!name.is_empty()).then_some(name)
}

/// A `1: required string name` field, if `line` is one.
fn field(line: &str) -> Option<Field> {
    let trimmed = uncommented(line).trim().trim_end_matches([',', ';']).trim();
    let (number, rest) = trimmed.split_once(':')?;
    let number: i32 = number.trim().parse().ok()?;

    let mut words: Vec<&str> = rest.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    let requiredness = match words.first() {
        Some(&"required") => {
            words.remove(0);
            "required"
        }
        Some(&"optional") => {
            words.remove(0);
            "optional"
        }
        _ => "default",
    };
    // Anything after an `=` is a default value, not part of the name.
    if let Some(at) = words.iter().position(|word| *word == "=") {
        words.truncate(at);
    }
    let name = words.pop()?;
    let kind = words.join(" ");
    if kind.is_empty() {
        return None;
    }
    Some(Field {
        number,
        requiredness: requiredness.to_owned(),
        kind,
        name: name.to_owned(),
    })
}

/// Everything [`ThriftView`] holds, read from `text`.
fn parse(text: &str) -> ThriftView {
    let mut view = ThriftView {
        namespaces: Vec::new(),
        includes: Vec::new(),
        typedefs: Vec::new(),
        declarations: Vec::new(),
        services: Vec::new(),
        unspecified_requiredness: Vec::new(),
        content: String::new(),
        truncated: false,
    };
    let mut open: Option<String> = None;
    let mut service: Option<Service> = None;

    for raw in text.lines() {
        let line = uncommented(raw).trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with('}') {
            if let Some(done) = service.take() {
                view.services.push(done);
            }
            open = None;
            continue;
        }
        if let Some(rest) = line.strip_prefix("namespace ") {
            view.namespaces.push(rest.trim().to_owned());
            continue;
        }
        if line.starts_with("include ") {
            if let Some(file) = line.split('"').nth(1) {
                view.includes.push(file.to_owned());
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("typedef ") {
            let words: Vec<&str> = rest
                .trim_end_matches(&[',', ';'][..])
                .split_whitespace()
                .collect();
            if let Some((name, kind)) = words.split_last() {
                view.typedefs.push(format!("{} -> {name}", kind.join(" ")));
            }
            continue;
        }

        for keyword in ["struct", "union", "exception", "enum"] {
            if let Some(name) = declaration(line, keyword) {
                open = Some(name.to_owned());
                view.declarations.push(Declaration {
                    keyword: keyword.to_owned(),
                    name: name.to_owned(),
                    fields: Vec::new(),
                });
            }
        }
        if open.is_some() && line.starts_with(['s', 'u', 'e']) && declaration_opened(line) {
            continue;
        }
        if let Some(name) = declaration(line, "service") {
            let extends = line
                .split_once(" extends ")
                .and_then(|(_, rest)| rest.split(['{', ' ']).next())
                .map(|name| name.trim().to_owned())
                .filter(|name| !name.is_empty());
            service = Some(Service {
                name: name.to_owned(),
                extends,
                methods: Vec::new(),
            });
            continue;
        }

        if let Some(entry) = service.as_mut() {
            if let Some(method) = method(line) {
                entry.methods.push(method);
            }
            continue;
        }
        if let Some(name) = open.clone()
            && let Some(found) = field(line)
        {
            if found.requiredness == "default" {
                view.unspecified_requiredness
                    .push(format!("{name}.{}", found.name));
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

/// Whether `line` opened one of the declarations above, so its own text is
/// not also read as a field.
fn declaration_opened(line: &str) -> bool {
    ["struct", "union", "exception", "enum", "service"]
        .iter()
        .any(|keyword| declaration(line, keyword).is_some())
}

/// A `Type name(...) throws (...)` method, if `line` is one.
fn method(line: &str) -> Option<Method> {
    let trimmed = uncommented(line).trim().trim_end_matches([',', ';']).trim();
    let (before, rest) = trimmed.split_once('(')?;
    let mut words: Vec<&str> = before.split_whitespace().collect();
    let name = words.pop()?;
    let oneway = words.first() == Some(&"oneway");
    if oneway {
        words.remove(0);
    }
    let returns = words.join(" ");
    if returns.is_empty() {
        return None;
    }
    let throws = rest
        .split_once("throws")
        .map(|(_, after)| {
            after
                .trim()
                .trim_start_matches('(')
                .split(')')
                .next()
                .unwrap_or("")
                .split(',')
                .filter_map(|one| one.split_whitespace().nth(1).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    Some(Method {
        name: name.to_owned(),
        returns,
        oneway,
        throws,
    })
}

/// Whether `text` is a Thrift definition.
fn looks_like_it(text: &str) -> bool {
    let has = |keyword: &str| {
        text.lines()
            .any(|line| declaration(uncommented(line), keyword).is_some())
    };
    let namespaced = text
        .lines()
        .any(|line| line.trim().starts_with("namespace "));
    (namespaced && (has("struct") || has("service")))
        || (has("struct") && has("service") && text.contains("1:"))
}

/// The Apache Thrift IDL plugin's core half.
#[derive(Debug, Default)]
pub struct ThriftCore;

impl PluginCore for ThriftCore {
    fn name(&self) -> &'static str {
        "thrift"
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

/// The Apache Thrift IDL plugin's presentation half.
#[derive(Debug, Default)]
pub struct ThriftPresentation;

impl PluginPresentation for ThriftPresentation {
    fn name(&self) -> &'static str {
        "thrift"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "THRF",
            tint: 0x00d1_2127,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: ThriftView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.namespaces.is_empty() {
            lines.push(format!("Namespaces: {}", view.namespaces.join(", ")));
        }
        if !view.includes.is_empty() {
            lines.push(format!("Includes: {}", view.includes.join(", ")));
        }
        if !view.typedefs.is_empty() {
            lines.push(format!("Typedefs: {}", view.typedefs.join(", ")));
        }
        for entry in &view.declarations {
            lines.push(format!(
                "{} {} ({} field(s))",
                entry.keyword,
                entry.name,
                entry.fields.len()
            ));
            for field in &entry.fields {
                lines.push(format!(
                    "  {}: {} {} {}",
                    field.number, field.requiredness, field.kind, field.name
                ));
            }
        }
        for service in &view.services {
            let extends = service
                .extends
                .as_ref()
                .map_or_else(String::new, |base| format!(" extends {base}"));
            lines.push(format!(
                "service {}{extends} ({} method(s))",
                service.name,
                service.methods.len()
            ));
            for method in &service.methods {
                let oneway = if method.oneway { "oneway " } else { "" };
                let throws = if method.throws.is_empty() {
                    String::new()
                } else {
                    format!(" throws {}", method.throws.join(", "))
                };
                lines.push(format!(
                    "  {oneway}{} {}(){throws}",
                    method.returns, method.name
                ));
            }
        }
        if !view.unspecified_requiredness.is_empty() {
            lines.push(
                "Neither required nor optional, so the bindings disagree about them:".to_owned(),
            );
            for name in &view.unspecified_requiredness {
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
    use super::{ThriftCore, ThriftPresentation, ThriftView, field, method, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const IDL: &str = "namespace java com.example.orders\n\
        namespace py example.orders\n\
        include \"common.thrift\"\n\
        typedef i64 Timestamp\n\
        enum State {\n  DRAFT = 0,\n  PLACED = 1,\n}\n\
        struct Order {\n\
        \x20 1: required string id,\n\
        \x20 2: optional string note,\n\
        \x20 3: list<Line> lines,\n\
        }\n\
        struct Line {\n  1: required string sku,\n}\n\
        exception NotFound {\n  1: required string id,\n}\n\
        service Orders extends Base {\n\
        \x20 Order get(1: string id) throws (1: NotFound missing),\n\
        \x20 oneway void ping(),\n\
        }\n";

    #[test]
    fn sniffs_a_namespaced_definition() {
        assert!(ThriftCore.sniff(IDL.as_bytes()));
    }

    #[test]
    fn does_not_claim_other_curly_brace_languages() {
        assert!(!ThriftCore.sniff(b"class A { void go() {} }\n"));
        assert!(!ThriftCore.sniff(b""));
    }

    #[test]
    fn a_field_states_its_requiredness_or_defaults_to_neither() {
        assert_eq!(
            field("  1: required string id,").unwrap().requiredness,
            "required"
        );
        assert_eq!(
            field("  2: optional string note,").unwrap().requiredness,
            "optional"
        );
        assert_eq!(
            field("  3: list<Line> lines,").unwrap().requiredness,
            "default"
        );
        assert_eq!(field("  4: i32 count = 7,").unwrap().name, "count");
    }

    #[test]
    fn reads_a_method_with_its_throws_and_oneway() {
        let get = method("Order get(1: string id) throws (1: NotFound missing),").unwrap();
        assert_eq!(get.name, "get");
        assert_eq!(get.returns, "Order");
        assert_eq!(
            get.throws,
            vec!["NotFound".to_owned()],
            "the exception type is what a reader wants, not the parameter name"
        );

        let ping = method("oneway void ping(),").unwrap();
        assert!(ping.oneway);
    }

    #[test]
    fn reads_namespaces_includes_and_typedefs() {
        let view = parse(IDL);

        assert_eq!(view.namespaces.len(), 2);
        assert_eq!(view.includes, vec!["common.thrift".to_owned()]);
        assert_eq!(view.typedefs, vec!["i64 -> Timestamp".to_owned()]);
    }

    #[test]
    fn fields_land_on_the_declaration_that_holds_them() {
        let view = parse(IDL);

        let order = view
            .declarations
            .iter()
            .find(|d| d.name == "Order")
            .unwrap();
        let line = view.declarations.iter().find(|d| d.name == "Line").unwrap();

        assert_eq!(order.fields.len(), 3);
        assert_eq!(line.fields.len(), 1);
        assert!(view.declarations.iter().any(|d| d.keyword == "exception"));
    }

    #[test]
    fn names_the_fields_the_bindings_disagree_about() {
        let view = parse(IDL);

        assert_eq!(
            view.unspecified_requiredness,
            vec!["Order.lines".to_owned()]
        );
    }

    #[test]
    fn reads_a_service_and_what_it_extends() {
        let view = parse(IDL);

        assert_eq!(view.services.len(), 1);
        assert_eq!(view.services[0].extends.as_deref(), Some("Base"));
        assert_eq!(view.services[0].methods.len(), 2);
    }

    #[test]
    fn presents_the_disagreement_with_its_reason() {
        let data = serde_json::to_value(parse(IDL)).unwrap();

        let lines = ThriftPresentation.present(&data);

        assert!(lines.iter().any(|line| line.contains("bindings disagree")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/thrift/orders.thrift");

        let data = ThriftCore.view(&path).unwrap();
        let view: ThriftView = serde_json::from_value(data).unwrap();

        assert!(view.namespaces.len() >= 2);
        assert!(!view.includes.is_empty());
        assert!(!view.typedefs.is_empty());
        assert!(view.declarations.len() >= 4);
        assert!(view.declarations.iter().any(|d| d.keyword == "exception"));
        assert!(view.declarations.iter().any(|d| d.keyword == "enum"));
        assert!(view.services.len() >= 2);
        // Across the services, not just the first: `Base` comes first and
        // has neither a oneway method nor a throwing one.
        let methods: Vec<_> = view
            .services
            .iter()
            .flat_map(|service| service.methods.iter())
            .collect();
        assert!(methods.iter().any(|m| m.oneway));
        assert!(methods.iter().any(|m| !m.throws.is_empty()));
        assert!(view.services.iter().any(|s| s.extends.is_some()));
        assert!(!view.unspecified_requiredness.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::ThriftCore),
            plugin_api::PluginPresentation::extensions(&crate::ThriftPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
