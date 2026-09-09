//! Protocol Buffers file type plugin: core and presentation halves.
//!
//! A `syntax = "proto3";` declaration settles it outright; otherwise
//! `message`, `service` and `rpc` together are a shape no sibling has.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["proto"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One field of a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// Its declared type.
    pub kind: String,
    /// Its name.
    pub name: String,
    /// Its wire number, which may never be reused once published.
    pub number: u32,
    /// Whether it is `repeated`.
    pub repeated: bool,
    /// Whether it is `optional`.
    pub optional: bool,
}

/// One message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Its name, qualified by the messages it is nested inside.
    pub name: String,
    /// Its fields.
    pub fields: Vec<Field>,
    /// How many `oneof` groups it declares.
    pub oneofs: usize,
}

/// One method of a service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Method {
    /// Its name.
    pub name: String,
    /// Its request type.
    pub request: String,
    /// Its response type.
    pub response: String,
    /// Whether the request streams.
    pub client_streaming: bool,
    /// Whether the response streams.
    pub server_streaming: bool,
}

/// One service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    /// Its name.
    pub name: String,
    /// Its methods.
    pub methods: Vec<Method>,
}

/// View data produced by [`ProtobufCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtobufView {
    /// `proto2` or `proto3`.
    pub syntax: String,
    /// The package the definitions live in.
    pub package: Option<String>,
    /// The files it imports.
    pub imports: Vec<String>,
    /// The messages, nested ones qualified by their parent.
    pub messages: Vec<Message>,
    /// The enumerations, by name.
    pub enums: Vec<String>,
    /// The services.
    pub services: Vec<Service>,
    /// The file-level options set, as `name = value`.
    pub options: Vec<String>,
    /// Field numbers marked reserved, which may never be used again.
    pub reserved: Vec<String>,
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
    let rest = line.trim().strip_prefix(keyword)?;
    let rest = rest.strip_prefix(' ')?;
    let name = rest.split(['{', ' ', '=']).next()?.trim();
    (!name.is_empty()).then_some(name)
}

/// A `type name = number;` field, if `line` is one.
fn field(line: &str) -> Option<Field> {
    let trimmed = uncommented(line).trim().trim_end_matches(';').trim();
    let (before, number) = trimmed.rsplit_once('=')?;
    let number: u32 = number.trim().split('[').next()?.trim().parse().ok()?;

    let mut words: Vec<&str> = before.split_whitespace().collect();
    if words.len() < 2 {
        return None;
    }
    let name = words.pop()?;
    let repeated = words.first() == Some(&"repeated");
    let optional = words.first() == Some(&"optional");
    if repeated || optional || words.first() == Some(&"required") {
        words.remove(0);
    }
    let kind = words.join(" ");
    if kind.is_empty() || name.is_empty() {
        return None;
    }
    Some(Field {
        kind,
        name: name.to_owned(),
        number,
        repeated,
        optional,
    })
}

/// An `rpc Name (Request) returns (Response);` line.
fn method(line: &str) -> Option<Method> {
    let rest = uncommented(line).trim().strip_prefix("rpc ")?;
    let (name, rest) = rest.split_once('(')?;
    let (request, rest) = rest.split_once(')')?;
    let rest = rest.trim().strip_prefix("returns")?.trim();
    let response = rest.strip_prefix('(')?.split(')').next()?;

    let strip = |text: &str| -> (String, bool) {
        let trimmed = text.trim();
        trimmed.strip_prefix("stream ").map_or_else(
            || (trimmed.to_owned(), false),
            |bare| (bare.trim().to_owned(), true),
        )
    };
    let (request, client_streaming) = strip(request);
    let (response, server_streaming) = strip(response);
    Some(Method {
        name: name.trim().to_owned(),
        request,
        response,
        client_streaming,
        server_streaming,
    })
}

/// What kind of block a `{` opened.
///
/// Every one of them is closed by a `}`, so they all have to be tracked.
/// Counting only messages meant a `oneof`'s closing brace popped the
/// message it sat inside, and the next nested message was reported as a
/// second top-level one.
enum Block {
    /// A message, which contributes its name to the qualified path.
    Message(String),
    /// A service, which collects methods.
    Service(Service),
    /// An enumeration, a `oneof`, or anything else with a body.
    Other,
}

/// The dotted path of the messages currently open.
fn path(stack: &[Block]) -> String {
    stack
        .iter()
        .filter_map(|block| match block {
            Block::Message(name) => Some(name.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Applies a file-level directive - `syntax`, `package`, `import`,
/// `option`, `reserved` - and says whether `line` was one.
///
/// Lifted out of [`parse`], which clippy counts at more lines than it
/// allows. The seam is real: this reads the declarations that stand alone,
/// and `parse` tracks the blocks.
fn directive(line: &str, stack: &[Block], view: &mut ProtobufView) -> bool {
    if let Some(rest) = line.strip_prefix("syntax") {
        if let Some(value) = rest.split('"').nth(1) {
            value.clone_into(&mut view.syntax);
        }
        return true;
    }
    if let Some(rest) = line.strip_prefix("package ") {
        view.package = Some(rest.trim_end_matches(';').trim().to_owned());
        return true;
    }
    if line.starts_with("import ") {
        if let Some(file) = line.split('"').nth(1) {
            view.imports.push(file.to_owned());
        }
        return true;
    }
    if let Some(rest) = line.strip_prefix("option ")
        && stack.is_empty()
    {
        view.options
            .push(rest.trim_end_matches(';').trim().to_owned());
        return true;
    }
    if let Some(rest) = line.strip_prefix("reserved ") {
        view.reserved
            .push(rest.trim_end_matches(';').trim().to_owned());
        return true;
    }
    false
}

/// Everything [`ProtobufView`] holds, read from `text`.
fn parse(text: &str) -> ProtobufView {
    let mut view = ProtobufView {
        syntax: "proto2".to_owned(),
        package: None,
        imports: Vec::new(),
        messages: Vec::new(),
        enums: Vec::new(),
        services: Vec::new(),
        options: Vec::new(),
        reserved: Vec::new(),
        content: String::new(),
        truncated: false,
    };
    let mut stack: Vec<Block> = Vec::new();

    for raw in text.lines() {
        let line = uncommented(raw).trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with('}') {
            if let Some(Block::Service(done)) = stack.pop() {
                view.services.push(done);
            }
            continue;
        }

        if directive(line, &stack, &mut view) {
            continue;
        }

        if let Some(name) = declaration(line, "message") {
            stack.push(Block::Message(name.to_owned()));
            view.messages.push(Message {
                name: path(&stack),
                fields: Vec::new(),
                oneofs: 0,
            });
            continue;
        }
        if let Some(name) = declaration(line, "enum") {
            let here = path(&stack);
            view.enums.push(if here.is_empty() {
                name.to_owned()
            } else {
                format!("{here}.{name}")
            });
            stack.push(Block::Other);
            continue;
        }
        if let Some(name) = declaration(line, "service") {
            stack.push(Block::Service(Service {
                name: name.to_owned(),
                methods: Vec::new(),
            }));
            continue;
        }
        if line.starts_with("oneof ") {
            let here = path(&stack);
            if let Some(message) = view.messages.iter_mut().rev().find(|m| m.name == here) {
                message.oneofs += 1;
            }
            stack.push(Block::Other);
            continue;
        }

        if let Some(Block::Service(entry)) = stack.last_mut() {
            if let Some(found) = method(line) {
                entry.methods.push(found);
            }
            continue;
        }

        // A field belongs to the innermost message, but only when that is
        // what is actually open: a `oneof`'s own fields belong to the
        // message around it, and an enumeration's values are not fields.
        let here = path(&stack);
        if !here.is_empty()
            && !matches!(stack.last(), Some(Block::Other))
            && let Some(found) = field(line)
            && let Some(message) = view.messages.iter_mut().rev().find(|m| m.name == here)
        {
            message.fields.push(found);
        }
    }
    view
}

/// Whether `text` is a Protocol Buffers definition.
fn looks_like_it(text: &str) -> bool {
    if text.contains("syntax = \"proto") || text.contains("syntax=\"proto") {
        return true;
    }
    let has = |keyword: &str| {
        text.lines()
            .any(|line| declaration(uncommented(line), keyword).is_some())
    };
    (has("message") && has("service")) || (has("message") && text.contains(" = 1;"))
}

/// The Protocol Buffers plugin's core half.
#[derive(Debug, Default)]
pub struct ProtobufCore;

impl PluginCore for ProtobufCore {
    fn name(&self) -> &'static str {
        "protobuf"
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

/// The Protocol Buffers plugin's presentation half.
#[derive(Debug, Default)]
pub struct ProtobufPresentation;

impl PluginPresentation for ProtobufPresentation {
    fn name(&self) -> &'static str {
        "protobuf"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "PROT",
            tint: 0x004a_86e8,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: ProtobufView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("Protocol Buffers, {}", view.syntax));
        if let Some(package) = &view.package {
            lines.push(format!("Package: {package}"));
        }
        if !view.imports.is_empty() {
            lines.push(format!("Imports: {}", view.imports.join(", ")));
        }
        if !view.messages.is_empty() {
            lines.push(format!("Messages ({}):", view.messages.len()));
            for message in &view.messages {
                let oneofs = if message.oneofs > 0 {
                    format!(", {} oneof", message.oneofs)
                } else {
                    String::new()
                };
                lines.push(format!(
                    "  {}  ({} field(s){oneofs})",
                    message.name,
                    message.fields.len()
                ));
                for field in &message.fields {
                    let mut label = field.kind.clone();
                    if field.repeated {
                        label = format!("repeated {label}");
                    }
                    if field.optional {
                        label = format!("optional {label}");
                    }
                    lines.push(format!("      {} {} = {}", label, field.name, field.number));
                }
            }
        }
        if !view.enums.is_empty() {
            lines.push(format!("Enumerations: {}", view.enums.join(", ")));
        }
        for service in &view.services {
            lines.push(format!(
                "Service {} ({} method(s)):",
                service.name,
                service.methods.len()
            ));
            for method in &service.methods {
                let request = if method.client_streaming {
                    format!("stream {}", method.request)
                } else {
                    method.request.clone()
                };
                let response = if method.server_streaming {
                    format!("stream {}", method.response)
                } else {
                    method.response.clone()
                };
                lines.push(format!("  {}({request}) -> {response}", method.name));
            }
        }
        if !view.options.is_empty() {
            lines.push(format!("Options: {}", view.options.join(", ")));
        }
        if !view.reserved.is_empty() {
            lines.push(format!(
                "Reserved, and never to be reused: {}",
                view.reserved.join("; ")
            ));
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{ProtobufCore, ProtobufPresentation, ProtobufView, field, method, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const PROTO: &str = "syntax = \"proto3\";\n\
        package example.orders.v1;\n\
        import \"google/protobuf/timestamp.proto\";\n\
        option go_package = \"example.com/orders\";\n\
        message Order {\n\
        \x20 reserved 4, 7 to 9;\n\
        \x20 string id = 1;\n\
        \x20 repeated Line lines = 2;\n\
        \x20 optional string note = 3;\n\
        \x20 oneof payment {\n    string card = 10;\n    string account = 11;\n  }\n\
        \x20 message Line {\n    string sku = 1;\n    int32 quantity = 2;\n  }\n\
        \x20 enum State {\n    DRAFT = 0;\n    PLACED = 1;\n  }\n\
        }\n\
        service Orders {\n\
        \x20 rpc Get (GetRequest) returns (Order);\n\
        \x20 rpc Watch (WatchRequest) returns (stream Order);\n\
        \x20 rpc Upload (stream Chunk) returns (UploadResult);\n\
        }\n";

    #[test]
    fn sniffs_a_syntax_declaration_or_a_message_and_service() {
        assert!(ProtobufCore.sniff(PROTO.as_bytes()));
        assert!(ProtobufCore.sniff(b"message A { string a = 1; }\nservice S { }\n"));
    }

    #[test]
    fn does_not_claim_other_curly_brace_languages() {
        assert!(!ProtobufCore.sniff(b"class A { void go() {} }\n"));
        assert!(!ProtobufCore.sniff(b"{\"a\": 1}\n"));
        assert!(!ProtobufCore.sniff(b""));
    }

    #[test]
    fn reads_a_field_with_its_number_and_label() {
        assert!(field("  repeated Line lines = 2;").unwrap().repeated);
        assert!(field("  optional string note = 3;").unwrap().optional);
        assert_eq!(field("  map<string, int32> counts = 5;").unwrap().number, 5);
        assert!(field("  // just a comment").is_none());
    }

    #[test]
    fn reads_streaming_on_either_side_of_a_method() {
        let watch = method("rpc Watch (WatchRequest) returns (stream Order);").unwrap();
        assert!(watch.server_streaming && !watch.client_streaming);

        let upload = method("rpc Upload (stream Chunk) returns (UploadResult);").unwrap();
        assert!(upload.client_streaming && !upload.server_streaming);
    }

    #[test]
    fn a_nested_message_is_qualified_by_the_one_it_sits_in() {
        let view = parse(PROTO);

        let names: Vec<&str> = view.messages.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"Order"));
        assert!(
            names.contains(&"Order.Line"),
            "a nested message is not a second top-level one: {names:?}"
        );
        assert!(view.enums.contains(&"Order.State".to_owned()));
    }

    #[test]
    fn fields_land_on_the_message_that_declares_them() {
        let view = parse(PROTO);

        let order = view.messages.iter().find(|m| m.name == "Order").unwrap();
        let line = view
            .messages
            .iter()
            .find(|m| m.name == "Order.Line")
            .unwrap();

        assert_eq!(line.fields.len(), 2);
        assert!(order.fields.iter().any(|f| f.name == "id"));
        assert!(
            !order.fields.iter().any(|f| f.name == "sku"),
            "the nested message keeps its own fields"
        );
        assert_eq!(order.oneofs, 1);
    }

    #[test]
    fn reads_the_syntax_package_imports_options_and_reservations() {
        let view = parse(PROTO);

        assert_eq!(view.syntax, "proto3");
        assert_eq!(view.package.as_deref(), Some("example.orders.v1"));
        assert_eq!(
            view.imports,
            vec!["google/protobuf/timestamp.proto".to_owned()]
        );
        assert_eq!(view.options.len(), 1);
        assert_eq!(view.reserved, vec!["4, 7 to 9".to_owned()]);
    }

    #[test]
    fn presents_the_syntax_first_and_the_reservations_with_their_reason() {
        let data = serde_json::to_value(parse(PROTO)).unwrap();

        let lines = ProtobufPresentation.present(&data);

        assert_eq!(lines[0], "Protocol Buffers, proto3");
        assert!(lines.iter().any(|line| line.contains("never to be reused")));
        assert!(lines.iter().any(|line| line.contains("stream Order")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/protobuf/orders.proto");

        let data = ProtobufCore.view(&path).unwrap();
        let view: ProtobufView = serde_json::from_value(data).unwrap();

        assert_eq!(view.syntax, "proto3");
        assert!(view.package.is_some());
        assert!(!view.imports.is_empty());
        assert!(view.messages.len() >= 4);
        assert!(view.messages.iter().any(|m| m.name.contains('.')));
        assert!(view.messages.iter().any(|m| m.oneofs > 0));
        assert!(!view.enums.is_empty());
        assert!(!view.services.is_empty());
        assert!(view.services[0].methods.iter().any(|m| m.server_streaming));
        assert!(view.services[0].methods.iter().any(|m| m.client_streaming));
        assert!(!view.options.is_empty());
        assert!(!view.reserved.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::ProtobufCore),
            plugin_api::PluginPresentation::extensions(&crate::ProtobufPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
