//! `OpenAPI` description file type plugin: core and presentation halves.
//!
//! A specialisation of JSON: an `openapi` version with `info` and
//! `paths`, or the older `swagger` key. The YAML form of the same
//! document is left to a later work order; this reads the JSON one.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One operation on one path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Operation {
    /// The method, upper-cased.
    pub method: String,
    /// The path it lives at.
    pub path: String,
    /// Its `operationId`, when it has one. Client generators need it, and
    /// invent an unstable name when it is missing.
    pub id: Option<String>,
    /// Its summary.
    pub summary: Option<String>,
    /// Whether it declares a request body.
    pub request_body: bool,
    /// The response codes it documents.
    pub responses: Vec<String>,
    /// The security schemes it requires, if it overrides the default.
    pub security: Vec<String>,
}

/// View data produced by [`OpenapiCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenapiView {
    /// The specification version.
    pub version: String,
    /// The interface's title.
    pub title: Option<String>,
    /// The interface's own version, which is not the specification's.
    pub api_version: Option<String>,
    /// The servers it is published on.
    pub servers: Vec<String>,
    /// Every operation, in path then method order.
    pub operations: Vec<Operation>,
    /// The component schemas defined.
    pub schemas: Vec<String>,
    /// The security schemes defined.
    pub security_schemes: Vec<String>,
    /// Operations with no `operationId`, which every client generator
    /// then names for itself - differently, and unstably.
    pub missing_operation_ids: Vec<String>,
    /// Operations that document no failure at all.
    pub no_error_responses: Vec<String>,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The methods an operation may be.
const METHODS: &[&str] = &[
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Every operation under `paths`, added to `view`.
fn read_paths(root: &Value, view: &mut OpenapiView) {
    if let Some(paths) = root.get("paths").and_then(Value::as_object) {
        for (path, item) in paths {
            let Some(entries) = item.as_object() else {
                continue;
            };
            for (method, operation) in entries {
                if !METHODS.contains(&method.as_str()) {
                    continue;
                }
                let responses: Vec<String> = operation
                    .get("responses")
                    .and_then(Value::as_object)
                    .map(|codes| codes.keys().cloned().collect())
                    .unwrap_or_default();
                let id = operation
                    .get("operationId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let security = operation
                    .get("security")
                    .and_then(Value::as_array)
                    .map(|entries| {
                        entries
                            .iter()
                            .filter_map(Value::as_object)
                            .flat_map(|entry| entry.keys().cloned().collect::<Vec<_>>())
                            .collect()
                    })
                    .unwrap_or_default();

                let where_it_is = format!("{} {path}", method.to_uppercase());
                if id.is_none() {
                    view.missing_operation_ids.push(where_it_is.clone());
                }
                // A response code of 4xx or 5xx, or a `default`, is a
                // documented failure. Without one, a caller is told only
                // what success looks like.
                let documents_failure = responses.iter().any(|code| {
                    code == "default" || code.starts_with('4') || code.starts_with('5')
                });
                if !documents_failure {
                    view.no_error_responses.push(where_it_is.clone());
                }

                view.operations.push(Operation {
                    method: method.to_uppercase(),
                    path: path.clone(),
                    id,
                    summary: operation
                        .get("summary")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    request_body: operation.get("requestBody").is_some(),
                    responses,
                    security,
                });
            }
        }
    }
}

/// Everything [`OpenapiView`] holds, read from `text`.
fn parse(text: &str) -> OpenapiView {
    let mut view = OpenapiView {
        version: "unstated".to_owned(),
        title: None,
        api_version: None,
        servers: Vec::new(),
        operations: Vec::new(),
        schemas: Vec::new(),
        security_schemes: Vec::new(),
        missing_operation_ids: Vec::new(),
        no_error_responses: Vec::new(),
        truncated: false,
    };
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return view;
    };

    if let Some(version) = root.get("openapi").and_then(Value::as_str) {
        version.clone_into(&mut view.version);
    } else if let Some(version) = root.get("swagger").and_then(Value::as_str) {
        view.version = format!("swagger {version}");
    }
    if let Some(info) = root.get("info") {
        view.title = info.get("title").and_then(Value::as_str).map(str::to_owned);
        view.api_version = info
            .get("version")
            .and_then(Value::as_str)
            .map(str::to_owned);
    }
    if let Some(servers) = root.get("servers").and_then(Value::as_array) {
        view.servers = servers
            .iter()
            .filter_map(|server| server.get("url").and_then(Value::as_str))
            .map(str::to_owned)
            .collect();
    } else if let Some(host) = root.get("host").and_then(Value::as_str) {
        // Swagger 2 spelled it differently.
        view.servers.push(host.to_owned());
    }

    read_paths(&root, &mut view);

    if let Some(components) = root.get("components") {
        if let Some(schemas) = components.get("schemas").and_then(Value::as_object) {
            view.schemas = schemas.keys().cloned().collect();
        }
        if let Some(schemes) = components.get("securitySchemes").and_then(Value::as_object) {
            view.security_schemes = schemes.keys().cloned().collect();
        }
    }
    if let Some(definitions) = root.get("definitions").and_then(Value::as_object) {
        // Swagger 2 again.
        view.schemas
            .extend(definitions.keys().cloned().collect::<Vec<_>>());
    }
    view
}

/// Whether `text` is an `OpenAPI` description.
fn looks_like_it(text: &str) -> bool {
    // Cheap first: all three keys have to be in the prefix at all.
    let versioned = text.contains("\"openapi\"") || text.contains("\"swagger\"");
    if !versioned || !text.contains("\"info\"") || !text.contains("\"paths\"") {
        return false;
    }
    match serde_json::from_str::<Value>(text) {
        // A whole document has to say it honestly, at the top level: the
        // three keys nested inside something else are somebody else's file.
        Ok(root) => {
            let versioned = root.get("openapi").and_then(Value::as_str).is_some()
                || root.get("swagger").and_then(Value::as_str).is_some();
            versioned && root.get("info").is_some() && root.get("paths").is_some()
        }
        // A description longer than the sniffing prefix arrives cut off, so
        // it cannot parse. Three keys in something that opens as an object
        // is as honest an answer as the bytes allow.
        Err(_) => text.trim_start().starts_with('{'),
    }
}

/// The `OpenAPI` description plugin's core half.
#[derive(Debug, Default)]
pub struct OpenapiCore;

impl PluginCore for OpenapiCore {
    fn name(&self) -> &'static str {
        "openapi"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A specialisation of JSON, which owns the extension (D13).
        &["json"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        // A description of any size is mostly nesting the summary
        // flattens, and every field worth reading is named on the view.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The `OpenAPI` description plugin's presentation half.
#[derive(Debug, Default)]
pub struct OpenapiPresentation;

impl PluginPresentation for OpenapiPresentation {
    fn name(&self) -> &'static str {
        "openapi"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "API",
            tint: 0x0085_ea2d,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: OpenapiView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if view.version == "unstated" {
            lines.push("Not read: this did not parse as an OpenAPI description.".to_owned());
            if view.truncated {
                lines.push(format!(
                    "It is longer than {MAX_VIEW_BYTES} bytes, and only that much was read."
                ));
            }
            return lines;
        }
        if let Some(title) = &view.title {
            let api = view
                .api_version
                .as_ref()
                .map_or_else(String::new, |version| format!(" {version}"));
            lines.push(format!("{title}{api}"));
        }
        lines.push(format!("OpenAPI {}", view.version));
        if !view.servers.is_empty() {
            lines.push(format!("Servers: {}", view.servers.join(", ")));
        }
        lines.push(format!("{} operation(s):", view.operations.len()));
        for operation in &view.operations {
            let id = operation
                .id
                .as_ref()
                .map_or_else(|| "  (no operationId)".to_owned(), |id| format!("  {id}"));
            lines.push(format!("  {} {}{id}", operation.method, operation.path));
            if let Some(summary) = &operation.summary {
                lines.push(format!("      {summary}"));
            }
            lines.push(format!(
                "      responses: {}",
                if operation.responses.is_empty() {
                    "none documented".to_owned()
                } else {
                    operation.responses.join(", ")
                }
            ));
        }
        if !view.schemas.is_empty() {
            lines.push(format!(
                "Schemas ({}): {}",
                view.schemas.len(),
                view.schemas.join(", ")
            ));
        }
        if !view.security_schemes.is_empty() {
            lines.push(format!(
                "Security schemes: {}",
                view.security_schemes.join(", ")
            ));
        }
        if !view.missing_operation_ids.is_empty() {
            lines.push(
                "No operationId, so every client generator names these itself, and".to_owned(),
            );
            lines.push("differently each time the document changes:".to_owned());
            for where_it_is in &view.missing_operation_ids {
                lines.push(format!("  {where_it_is}"));
            }
        }
        if !view.no_error_responses.is_empty() {
            lines.push("No failure documented, so a caller is told only what success".to_owned());
            lines.push("looks like:".to_owned());
            for where_it_is in &view.no_error_responses {
                lines.push(format!("  {where_it_is}"));
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
    use super::{OpenapiCore, OpenapiPresentation, OpenapiView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const DESCRIPTION: &str = r#"{
      "openapi": "3.1.0",
      "info": { "title": "Orders", "version": "1.4.0" },
      "servers": [ { "url": "https://api.example.com/v1" } ],
      "paths": {
        "/orders": {
          "get": {
            "operationId": "listOrders",
            "summary": "Every order.",
            "responses": { "200": {}, "401": {} }
          },
          "post": {
            "requestBody": {},
            "responses": { "201": {} }
          }
        },
        "/orders/{id}": {
          "get": {
            "operationId": "getOrder",
            "responses": { "200": {}, "404": {} },
            "security": [ { "bearer": [] } ]
          }
        }
      },
      "components": {
        "schemas": { "Order": {}, "Line": {} },
        "securitySchemes": { "bearer": {} }
      }
    }"#;

    #[test]
    fn sniffs_a_description() {
        assert!(OpenapiCore.sniff(DESCRIPTION.as_bytes()));
        assert!(OpenapiCore.sniff(br#"{"swagger":"2.0","info":{},"paths":{}}"#));
    }

    #[test]
    fn does_not_claim_json_that_merely_has_paths() {
        assert!(!OpenapiCore.sniff(br#"{"paths": ["/a", "/b"], "info": "x"}"#));
        assert!(!OpenapiCore.sniff(br#"{"openapi": "3.1.0"}"#));
        assert!(!OpenapiCore.sniff(b""));
    }

    #[test]
    fn does_not_claim_a_document_that_only_embeds_one() {
        // Valid, parses, has all three words, but not at the top level.
        assert!(
            !OpenapiCore
                .sniff(br#"{"kind":"Config","spec":{"openapi":"3.1.0","info":{},"paths":{}}}"#)
        );
    }

    #[test]
    fn claims_a_description_longer_than_the_sniffing_prefix() {
        // The service hands `sniff` the first 32 kibibytes only, so a real
        // description arrives cut off and will not parse.
        let cut = &DESCRIPTION[..DESCRIPTION.len() / 2];

        assert!(serde_json::from_str::<serde_json::Value>(cut).is_err());
        assert!(OpenapiCore.sniff(cut.as_bytes()));
    }

    #[test]
    fn it_says_it_specialises_json() {
        assert_eq!(OpenapiCore.specialises(), &["json"]);
    }

    #[test]
    fn the_interface_version_is_not_the_specification_version() {
        let view = parse(DESCRIPTION);

        assert_eq!(view.version, "3.1.0");
        assert_eq!(view.api_version.as_deref(), Some("1.4.0"));
        assert_eq!(view.title.as_deref(), Some("Orders"));
    }

    #[test]
    fn reads_every_operation_and_ignores_what_is_not_one() {
        let view = parse(DESCRIPTION);

        assert_eq!(view.operations.len(), 3);
        assert!(
            view.operations
                .iter()
                .any(|op| op.method == "POST" && op.request_body)
        );
        assert!(view.operations.iter().any(|op| !op.security.is_empty()));
    }

    #[test]
    fn names_the_operations_with_no_identifier() {
        let view = parse(DESCRIPTION);

        assert_eq!(view.missing_operation_ids, vec!["POST /orders".to_owned()]);
    }

    #[test]
    fn names_the_operations_that_document_no_failure() {
        let view = parse(DESCRIPTION);

        assert_eq!(
            view.no_error_responses,
            vec!["POST /orders".to_owned()],
            "a 201 alone tells a caller nothing about what can go wrong"
        );
    }

    #[test]
    fn a_default_response_counts_as_documenting_failure() {
        let view = parse(
            r#"{"openapi":"3.1.0","info":{},"paths":{"/a":{"get":{
                "operationId":"a","responses":{"200":{},"default":{}}}}}}"#,
        );

        assert!(view.no_error_responses.is_empty());
    }

    #[test]
    fn presents_both_warnings_with_their_reasons() {
        let data = serde_json::to_value(parse(DESCRIPTION)).unwrap();

        let lines = OpenapiPresentation.present(&data);

        assert_eq!(lines[0], "Orders 1.4.0");
        assert!(lines.iter().any(|line| line.contains("names these itself")));
        assert!(lines.iter().any(|line| line.contains("only what success")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/openapi/orders.openapi.json");

        let data = OpenapiCore.view(&path).unwrap();
        let view: OpenapiView = serde_json::from_value(data).unwrap();

        assert_eq!(view.version, "3.1.0");
        assert!(view.title.is_some() && view.api_version.is_some());
        assert!(!view.servers.is_empty());
        assert!(view.operations.len() >= 5);
        assert!(view.operations.iter().any(|op| op.request_body));
        assert!(view.operations.iter().any(|op| op.summary.is_some()));
        assert!(view.operations.iter().any(|op| !op.security.is_empty()));
        assert!(view.schemas.len() >= 2);
        assert!(!view.security_schemes.is_empty());
        assert!(!view.missing_operation_ids.is_empty());
        assert!(!view.no_error_responses.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::OpenapiCore),
            plugin_api::PluginPresentation::extensions(&crate::OpenapiPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
