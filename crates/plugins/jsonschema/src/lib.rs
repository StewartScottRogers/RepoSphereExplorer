//! JSON Schema file type plugin: core and presentation halves.
//!
//! A specialisation of JSON: a `$schema` naming a JSON Schema draft, or
//! `type` alongside `properties`, is a schema and not the data it
//! describes.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One declared property.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Property {
    /// Its name.
    pub name: String,
    /// Its declared type, or `any` when it names none.
    pub kind: String,
    /// Whether the schema lists it as required.
    pub required: bool,
    /// Its description, when it has one.
    pub description: Option<String>,
}

/// View data produced by [`JsonschemaCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonschemaView {
    /// The draft it declares.
    pub draft: Option<String>,
    /// Its `$id`.
    pub id: Option<String>,
    /// Its title.
    pub title: Option<String>,
    /// Its description.
    pub description: Option<String>,
    /// The top-level type.
    pub kind: String,
    /// The properties it declares.
    pub properties: Vec<Property>,
    /// The named definitions, whether under `$defs` or `definitions`.
    pub definitions: Vec<String>,
    /// The `$ref` targets used anywhere in the document.
    pub references: Vec<String>,
    /// The composition and conditional keywords it uses: `allOf`, `oneOf`,
    /// `anyOf`, `not`, `if`.
    pub composition: Vec<String>,
    /// Whether it forbids properties it did not declare. A schema that
    /// allows them validates a typo as valid data.
    pub additional_properties_allowed: bool,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The composition keywords, which are what make a schema more than a
/// list of fields.
const COMPOSITION: &[&str] = &["allOf", "oneOf", "anyOf", "not", "if", "then", "else"];

/// Every `$ref` value anywhere in `value`.
fn references_in(value: &Value, into: &mut Vec<String>) {
    match value {
        Value::Object(entries) => {
            for (key, child) in entries {
                if key == "$ref"
                    && let Some(target) = child.as_str()
                    && !into.contains(&target.to_owned())
                {
                    into.push(target.to_owned());
                }
                references_in(child, into);
            }
        }
        Value::Array(items) => {
            for child in items {
                references_in(child, into);
            }
        }
        _ => {}
    }
}

/// The type of a subschema, as a reader would say it.
fn type_of(value: &Value) -> String {
    match value.get("type") {
        Some(Value::String(name)) => name.clone(),
        Some(Value::Array(names)) => names
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" or "),
        _ => {
            if value.get("$ref").is_some() {
                "reference".to_owned()
            } else if value.get("enum").is_some() {
                "enumeration".to_owned()
            } else {
                "any".to_owned()
            }
        }
    }
}

/// Everything [`JsonschemaView`] holds, read from `text`.
fn parse(text: &str) -> JsonschemaView {
    let mut view = JsonschemaView {
        draft: None,
        id: None,
        title: None,
        description: None,
        kind: "any".to_owned(),
        properties: Vec::new(),
        definitions: Vec::new(),
        references: Vec::new(),
        composition: Vec::new(),
        additional_properties_allowed: true,
        truncated: false,
    };
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return view;
    };
    let text_at = |key: &str| root.get(key).and_then(Value::as_str).map(str::to_owned);

    view.draft = text_at("$schema");
    view.id = text_at("$id").or_else(|| text_at("id"));
    view.title = text_at("title");
    view.description = text_at("description");
    view.kind = type_of(&root);

    let required: Vec<&str> = root
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    if let Some(properties) = root.get("properties").and_then(Value::as_object) {
        for (name, schema) in properties {
            view.properties.push(Property {
                name: name.clone(),
                kind: type_of(schema),
                required: required.contains(&name.as_str()),
                description: schema
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }
    }

    for key in ["$defs", "definitions"] {
        if let Some(entries) = root.get(key).and_then(Value::as_object) {
            view.definitions
                .extend(entries.keys().cloned().collect::<Vec<_>>());
        }
    }
    for keyword in COMPOSITION {
        if root.get(*keyword).is_some() {
            view.composition.push((*keyword).to_owned());
        }
    }
    view.additional_properties_allowed =
        root.get("additionalProperties") != Some(&Value::Bool(false));

    references_in(&root, &mut view.references);
    view
}

/// Whether `text` is a JSON Schema.
fn looks_like_it(text: &str) -> bool {
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    if root
        .get("$schema")
        .and_then(Value::as_str)
        .is_some_and(|draft| draft.contains("json-schema.org"))
    {
        return true;
    }
    // A `type` with `properties` and either `required` or `$defs` is a
    // schema. `type` and `properties` alone appear in too much other JSON.
    root.get("type").is_some()
        && root.get("properties").is_some()
        && (root.get("required").is_some()
            || root.get("$defs").is_some()
            || root.get("definitions").is_some())
}

/// The JSON Schema plugin's core half.
#[derive(Debug, Default)]
pub struct JsonschemaCore;

impl PluginCore for JsonschemaCore {
    fn name(&self) -> &'static str {
        "jsonschema"
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
        // Every field worth reading is already named on the view, and a
        // schema of any size is mostly nesting the summary flattens.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The JSON Schema plugin's presentation half.
#[derive(Debug, Default)]
pub struct JsonschemaPresentation;

impl PluginPresentation for JsonschemaPresentation {
    fn name(&self) -> &'static str {
        "jsonschema"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "SCHM",
            tint: 0x0000_7fae,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: JsonschemaView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(title) = &view.title {
            lines.push(title.clone());
        }
        if let Some(description) = &view.description {
            lines.push(description.clone());
        }
        if let Some(draft) = &view.draft {
            lines.push(format!("Draft: {draft}"));
        }
        if let Some(id) = &view.id {
            lines.push(format!("Identifier: {id}"));
        }
        lines.push(format!("Top-level type: {}", view.kind));

        if !view.properties.is_empty() {
            let required = view.properties.iter().filter(|p| p.required).count();
            lines.push(format!(
                "Properties ({}, {required} required):",
                view.properties.len()
            ));
            for property in &view.properties {
                let mark = if property.required { "*" } else { " " };
                lines.push(format!("  {mark}{}: {}", property.name, property.kind));
                if let Some(description) = &property.description {
                    lines.push(format!("      {description}"));
                }
            }
        }
        if !view.definitions.is_empty() {
            lines.push(format!("Definitions: {}", view.definitions.join(", ")));
        }
        if !view.references.is_empty() {
            lines.push(format!("References: {}", view.references.join(", ")));
        }
        if !view.composition.is_empty() {
            lines.push(format!("Composition: {}", view.composition.join(", ")));
        }
        if view.additional_properties_allowed {
            lines.push(
                "Undeclared properties are allowed, so a misspelled key validates.".to_owned(),
            );
        } else {
            lines.push("Undeclared properties are refused.".to_owned());
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{JsonschemaCore, JsonschemaPresentation, JsonschemaView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const SCHEMA: &str = r##"{
      "$schema": "https://json-schema.org/draft/2020-12/schema",
      "$id": "https://example.com/order.schema.json",
      "title": "Order",
      "description": "One order and its lines.",
      "type": "object",
      "required": ["id", "lines"],
      "additionalProperties": false,
      "properties": {
        "id": { "type": "string", "description": "The order identifier." },
        "lines": { "type": "array", "items": { "$ref": "#/$defs/line" } },
        "note": { "type": ["string", "null"] },
        "state": { "enum": ["draft", "placed"] }
      },
      "$defs": {
        "line": {
          "type": "object",
          "properties": { "sku": { "type": "string" } }
        }
      },
      "allOf": [ { "$ref": "#/$defs/line" } ]
    }"##;

    #[test]
    fn sniffs_a_draft_declaration() {
        assert!(JsonschemaCore.sniff(SCHEMA.as_bytes()));
    }

    #[test]
    fn does_not_claim_the_data_a_schema_would_describe() {
        assert!(!JsonschemaCore.sniff(br#"{"type": "order", "properties": "many"}"#));
        assert!(!JsonschemaCore.sniff(br#"{"id": "a", "lines": []}"#));
        assert!(!JsonschemaCore.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_json() {
        assert_eq!(JsonschemaCore.specialises(), &["json"]);
    }

    #[test]
    fn reads_the_identity_and_the_top_level_type() {
        let view = parse(SCHEMA);

        assert_eq!(view.title.as_deref(), Some("Order"));
        assert!(view.draft.as_deref().is_some_and(|d| d.contains("2020-12")));
        assert_eq!(view.kind, "object");
    }

    #[test]
    fn a_union_type_reads_as_one_or_the_other() {
        let view = parse(SCHEMA);

        let note = view.properties.iter().find(|p| p.name == "note").unwrap();
        assert_eq!(note.kind, "string or null");
    }

    #[test]
    fn a_property_with_no_type_says_what_it_is_instead() {
        let view = parse(SCHEMA);

        let state = view.properties.iter().find(|p| p.name == "state").unwrap();
        assert_eq!(state.kind, "enumeration");
    }

    #[test]
    fn marks_the_required_properties_and_finds_nested_references() {
        let view = parse(SCHEMA);

        assert_eq!(view.properties.iter().filter(|p| p.required).count(), 2);
        assert_eq!(
            view.references,
            vec!["#/$defs/line".to_owned()],
            "a reference nested inside `items` still counts"
        );
        assert_eq!(view.definitions, vec!["line".to_owned()]);
        assert_eq!(view.composition, vec!["allOf".to_owned()]);
    }

    #[test]
    fn says_whether_a_misspelled_key_would_validate() {
        assert!(!parse(SCHEMA).additional_properties_allowed);

        let lax = parse(
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema",
                "type":"object","properties":{}}"#,
        );

        assert!(lax.additional_properties_allowed);

        let data = serde_json::to_value(lax).unwrap();
        let lines = JsonschemaPresentation.present(&data);

        assert!(
            lines
                .iter()
                .any(|line| line.contains("misspelled key validates"))
        );
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/jsonschema/order.schema.json");

        let data = JsonschemaCore.view(&path).unwrap();
        let view: JsonschemaView = serde_json::from_value(data).unwrap();

        assert!(view.draft.is_some() && view.id.is_some());
        assert!(view.title.is_some() && view.description.is_some());
        assert!(view.properties.len() >= 5);
        assert!(view.properties.iter().any(|p| p.required));
        assert!(view.properties.iter().any(|p| p.description.is_some()));
        assert!(view.definitions.len() >= 2);
        assert!(view.references.len() >= 2);
        assert!(!view.composition.is_empty());
        assert!(!view.additional_properties_allowed);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::JsonschemaCore),
            plugin_api::PluginPresentation::extensions(&crate::JsonschemaPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
