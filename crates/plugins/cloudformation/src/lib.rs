//! `CloudFormation` template file type plugin: core and presentation halves.
//!
//! A `CloudFormation` template describes a stack, and may be written as
//! JavaScript Object Notation (JSON) or as YAML. This reads either: the
//! description, the parameters with their types and defaults, the
//! resources with their types, the outputs, the conditions and the
//! mappings, and names the parameters nothing refers to.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One declared parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parameter {
    /// Its name, which is what the rest of the template refers to it by.
    pub name: String,
    /// Its declared type.
    pub kind: String,
    /// Its default, when it has one. Without a default it must be given
    /// at every deployment.
    pub default: Option<String>,
}

/// One declared resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    /// Its logical identifier within the template.
    pub id: String,
    /// The service type it creates.
    pub kind: String,
}

/// View data produced by [`CloudformationCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudformationView {
    /// Whether the template was written as `json` or as `yaml`.
    pub written_as: String,
    /// The `AWSTemplateFormatVersion`, when it states one.
    pub format_version: Option<String>,
    /// The template's description.
    pub description: Option<String>,
    /// Every parameter, in declaration order.
    pub parameters: Vec<Parameter>,
    /// Every resource, in declaration order.
    pub resources: Vec<Resource>,
    /// The names of the outputs.
    pub outputs: Vec<String>,
    /// The names of the conditions.
    pub conditions: Vec<String>,
    /// The names of the mappings.
    pub mappings: Vec<String>,
    /// Parameters nothing in the template refers to, which are asked for
    /// at every deployment and then ignored.
    pub unused_parameters: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// How deep `line` is indented, in spaces.
fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The key of a `key:` or `key: value` line.
fn key_of(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
        return None;
    }
    let (key, _) = trimmed.split_once(':')?;
    Some(key.trim().trim_matches('"').trim_matches('\''))
}

/// The value of a `key: value` line, or `None` when it opens a block.
fn value_of(line: &str) -> Option<String> {
    let (_, value) = line.trim().split_once(':')?;
    let value = value.trim().trim_matches('"').trim_matches('\'');
    (!value.is_empty()).then(|| value.to_owned())
}

/// The lines under the top-level key `wanted`.
fn section<'a>(lines: &[&'a str], wanted: &str) -> Vec<&'a str> {
    let Some(at) = lines
        .iter()
        .position(|line| indent(line) == 0 && key_of(line) == Some(wanted))
    else {
        return Vec::new();
    };
    lines[at + 1..]
        .iter()
        .take_while(|line| line.trim().is_empty() || indent(line) > 0)
        .copied()
        .collect()
}

/// The names declared directly inside `block`, which is a mapping of them.
fn names(block: &[&str]) -> Vec<String> {
    let Some(depth) = block
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| indent(line))
        .min()
    else {
        return Vec::new();
    };
    block
        .iter()
        .filter(|line| indent(line) == depth)
        .filter_map(|line| key_of(line))
        .map(str::to_owned)
        .collect()
}

/// The value of `wanted` inside the entry named `name` of `block`.
fn member(block: &[&str], name: &str, wanted: &str) -> Option<String> {
    let at = block.iter().position(|line| key_of(line) == Some(name))?;
    let depth = indent(block[at]);
    block[at + 1..]
        .iter()
        .take_while(|line| line.trim().is_empty() || indent(line) > depth)
        .find(|line| key_of(line) == Some(wanted))
        .and_then(|line| value_of(line))
}

/// Reads a template written as YAML.
fn parse_yaml(text: &str) -> CloudformationView {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| line.trim() != "---" && line.trim() != "...")
        .collect();
    let parameters = section(&lines, "Parameters");
    let resources = section(&lines, "Resources");
    CloudformationView {
        written_as: "yaml".to_owned(),
        format_version: lines
            .iter()
            .find(|line| key_of(line) == Some("AWSTemplateFormatVersion"))
            .and_then(|line| value_of(line)),
        description: lines
            .iter()
            .find(|line| indent(line) == 0 && key_of(line) == Some("Description"))
            .and_then(|line| value_of(line)),
        parameters: names(&parameters)
            .into_iter()
            .map(|name| Parameter {
                kind: member(&parameters, &name, "Type").unwrap_or_else(|| "unstated".to_owned()),
                default: member(&parameters, &name, "Default"),
                name,
            })
            .collect(),
        resources: names(&resources)
            .into_iter()
            .map(|id| Resource {
                kind: member(&resources, &id, "Type").unwrap_or_else(|| "unstated".to_owned()),
                id,
            })
            .collect(),
        outputs: names(&section(&lines, "Outputs")),
        conditions: names(&section(&lines, "Conditions")),
        mappings: names(&section(&lines, "Mappings")),
        unused_parameters: Vec::new(),
        truncated: false,
    }
}

/// The keys of `root`'s member `wanted`, in the order it declares them.
fn json_names(root: &Value, wanted: &str) -> Vec<String> {
    root.get(wanted)
        .and_then(Value::as_object)
        .map(|entries| entries.keys().cloned().collect())
        .unwrap_or_default()
}

/// Reads a template written as JSON.
fn parse_json(root: &Value) -> CloudformationView {
    let string = |entry: &Value, key: &str| match entry.get(key) {
        Some(Value::String(said)) => Some(said.clone()),
        Some(other) if !other.is_null() => Some(other.to_string()),
        _ => None,
    };
    CloudformationView {
        written_as: "json".to_owned(),
        format_version: string(root, "AWSTemplateFormatVersion"),
        description: string(root, "Description"),
        parameters: json_names(root, "Parameters")
            .into_iter()
            .map(|name| {
                let entry = root.get("Parameters").and_then(|all| all.get(&name));
                Parameter {
                    kind: entry
                        .and_then(|entry| string(entry, "Type"))
                        .unwrap_or_else(|| "unstated".to_owned()),
                    default: entry.and_then(|entry| string(entry, "Default")),
                    name,
                }
            })
            .collect(),
        resources: json_names(root, "Resources")
            .into_iter()
            .map(|id| {
                let entry = root.get("Resources").and_then(|all| all.get(&id));
                Resource {
                    kind: entry
                        .and_then(|entry| string(entry, "Type"))
                        .unwrap_or_else(|| "unstated".to_owned()),
                    id,
                }
            })
            .collect(),
        outputs: json_names(root, "Outputs"),
        conditions: json_names(root, "Conditions"),
        mappings: json_names(root, "Mappings"),
        unused_parameters: Vec::new(),
        truncated: false,
    }
}

/// Whether anything in `text` refers to the parameter `name`.
///
/// Counting the bare word would call a parameter used because its name is
/// an English word that appears in a description. These are the six ways
/// a template actually refers to one.
fn referenced(text: &str, name: &str) -> bool {
    [
        format!("!Ref {name}"),
        format!("!Ref '{name}'"),
        format!("!Ref \"{name}\""),
        format!("Ref: {name}"),
        format!("\"Ref\": \"{name}\""),
        format!("${{{name}}}"),
    ]
    .iter()
    .any(|reference| text.contains(reference.as_str()))
}

/// Everything [`CloudformationView`] holds, read from `text`.
fn parse(text: &str) -> CloudformationView {
    let mut view = match serde_json::from_str::<Value>(text) {
        Ok(root) if root.is_object() => parse_json(&root),
        _ => parse_yaml(text),
    };
    view.unused_parameters = view
        .parameters
        .iter()
        .filter(|parameter| !referenced(text, &parameter.name))
        .map(|parameter| parameter.name.clone())
        .collect();
    view
}

/// Whether `text` is a `CloudFormation` template.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    // The format version alone is not enough, and neither is a `Resources`
    // mapping: it has to declare resources of Amazon's own types.
    view.format_version.is_some() && !view.resources.is_empty()
        || view
            .resources
            .iter()
            .any(|resource| resource.kind.starts_with("AWS::"))
}

/// The `CloudFormation` template plugin's core half.
#[derive(Debug, Default)]
pub struct CloudformationCore;

impl PluginCore for CloudformationCore {
    fn name(&self) -> &'static str {
        "cloudformation"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A template is JSON or YAML, and both own their extension. This
        // is the narrower reading of either (D13).
        &["json", "yaml"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        // The sections are the whole of what a template says, and each
        // one is on the view already.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The `CloudFormation` template plugin's presentation half.
#[derive(Debug, Default)]
pub struct CloudformationPresentation;

impl PluginPresentation for CloudformationPresentation {
    fn name(&self) -> &'static str {
        "cloudformation"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "CFN",
            tint: 0x00ff_9900,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: CloudformationView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "CloudFormation template, written as {}",
            view.written_as
        ));
        if let Some(description) = &view.description {
            lines.push(description.clone());
        }
        if let Some(version) = &view.format_version {
            lines.push(format!("Format version {version}"));
        }
        lines.push(format!("{} parameter(s):", view.parameters.len()));
        for parameter in &view.parameters {
            lines.push(format!(
                "  {} ({}){}",
                parameter.name,
                parameter.kind,
                parameter.default.as_ref().map_or_else(
                    || " - no default, so it must be given every time".to_owned(),
                    |default| format!(" = {default}")
                )
            ));
        }
        lines.push(format!("{} resource(s):", view.resources.len()));
        for resource in &view.resources {
            lines.push(format!("  {} - {}", resource.id, resource.kind));
        }
        if !view.conditions.is_empty() {
            lines.push(format!("Conditions: {}", view.conditions.join(", ")));
        }
        if !view.mappings.is_empty() {
            lines.push(format!("Mappings: {}", view.mappings.join(", ")));
        }
        if !view.outputs.is_empty() {
            lines.push(format!("Outputs: {}", view.outputs.join(", ")));
        }
        if !view.unused_parameters.is_empty() {
            lines.push("Nothing refers to these, so they are asked for at every".to_owned());
            lines.push("deployment and then ignored:".to_owned());
            for name in &view.unused_parameters {
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
    use super::{
        CloudformationCore, CloudformationPresentation, CloudformationView, parse, referenced,
    };
    use plugin_api::{PluginCore, PluginPresentation};

    const TEMPLATE: &str = concat!(
        "AWSTemplateFormatVersion: '2010-09-09'\n",
        "Description: A bucket and a queue.\n",
        "Parameters:\n",
        "  BucketName:\n",
        "    Type: String\n",
        "    Default: explorer-artifacts\n",
        "  RetentionDays:\n",
        "    Type: Number\n",
        "  UnusedKnob:\n",
        "    Type: String\n",
        "    Default: nothing\n",
        "Conditions:\n",
        "  IsProduction: !Equals [!Ref BucketName, 'explorer-production']\n",
        "Mappings:\n",
        "  RegionToImage:\n",
        "    eu-west-1:\n",
        "      Image: ami-1111\n",
        "Resources:\n",
        "  Bucket:\n",
        "    Type: AWS::S3::Bucket\n",
        "    Properties:\n",
        "      BucketName: !Ref BucketName\n",
        "  Queue:\n",
        "    Type: AWS::SQS::Queue\n",
        "    Properties:\n",
        "      MessageRetentionPeriod: !Ref RetentionDays\n",
        "Outputs:\n",
        "  BucketArn:\n",
        "    Value: !GetAtt Bucket.Arn\n",
    );

    const AS_JSON: &str = r#"{
      "AWSTemplateFormatVersion": "2010-09-09",
      "Description": "A bucket, written the other way.",
      "Parameters": {
        "BucketName": { "Type": "String", "Default": "explorer-artifacts" },
        "UnusedKnob": { "Type": "String" }
      },
      "Resources": {
        "Bucket": {
          "Type": "AWS::S3::Bucket",
          "Properties": { "BucketName": { "Ref": "BucketName" } }
        }
      },
      "Outputs": { "BucketArn": { "Value": "x" } }
    }"#;

    #[test]
    fn sniffs_a_template_written_either_way() {
        assert!(CloudformationCore.sniff(TEMPLATE.as_bytes()));
        assert!(CloudformationCore.sniff(AS_JSON.as_bytes()));
    }

    #[test]
    fn does_not_claim_any_document_with_resources() {
        assert!(!CloudformationCore.sniff(b"Resources:\n  budget:\n    Type: internal\n"));
        assert!(!CloudformationCore.sniff(br#"{"Resources":{"a":{"Type":"local"}}}"#));
        assert!(!CloudformationCore.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_both_carriers() {
        assert_eq!(CloudformationCore.specialises(), &["json", "yaml"]);
    }

    #[test]
    fn reads_the_same_template_the_same_way_in_either_form() {
        let as_yaml = parse(TEMPLATE);
        let as_json = parse(AS_JSON);

        assert_eq!(as_yaml.written_as, "yaml");
        assert_eq!(as_json.written_as, "json");
        assert_eq!(as_yaml.format_version, as_json.format_version);
        assert_eq!(as_yaml.resources[0].kind, as_json.resources[0].kind);
        assert_eq!(as_yaml.parameters[0].default, as_json.parameters[0].default);
        assert_eq!(as_yaml.outputs, as_json.outputs);
    }

    #[test]
    fn reads_every_section() {
        let view = parse(TEMPLATE);

        assert_eq!(view.description.as_deref(), Some("A bucket and a queue."));
        assert_eq!(view.parameters.len(), 3);
        assert_eq!(view.parameters[1].kind, "Number");
        assert_eq!(view.parameters[1].default, None);
        assert_eq!(view.resources.len(), 2);
        assert_eq!(view.conditions, vec!["IsProduction".to_owned()]);
        assert_eq!(view.mappings, vec!["RegionToImage".to_owned()]);
        assert_eq!(view.outputs, vec!["BucketArn".to_owned()]);
    }

    #[test]
    fn a_word_in_the_description_is_not_a_reference() {
        assert!(referenced("Value: !Ref Thing", "Thing"));
        assert!(referenced(r#"{"Ref": "Thing"}"#, "Thing"));
        assert!(referenced("Name: !Sub '${Thing}-suffix'", "Thing"));
        assert!(
            !referenced("Description: The Thing this stack builds.", "Thing"),
            "the bare word would call every parameter used"
        );
    }

    #[test]
    fn names_the_parameters_nothing_refers_to() {
        assert_eq!(
            parse(TEMPLATE).unused_parameters,
            vec!["UnusedKnob".to_owned()]
        );
        assert_eq!(
            parse(AS_JSON).unused_parameters,
            vec!["UnusedKnob".to_owned()]
        );
    }

    #[test]
    fn presents_the_unused_parameter_with_its_reason() {
        let data = serde_json::to_value(parse(TEMPLATE)).unwrap();

        let lines = CloudformationPresentation.present(&data);

        assert_eq!(lines[0], "CloudFormation template, written as yaml");
        assert!(lines.iter().any(|line| line.contains("then ignored")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("must be given every time"))
        );
    }

    #[test]
    fn the_repository_fixtures_fill_every_field() {
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for name in ["service.yaml", "bucket.template.json"] {
            let path = here.join("../../../samples/cloudformation").join(name);

            let data = CloudformationCore.view(&path).unwrap();
            let view: CloudformationView = serde_json::from_value(data).unwrap();

            assert!(view.format_version.is_some(), "{name}");
            assert!(view.description.is_some(), "{name}");
            assert!(view.parameters.len() >= 3, "{name}");
            assert!(
                view.parameters.iter().any(|p| p.default.is_some()),
                "{name}"
            );
            assert!(
                view.parameters.iter().any(|p| p.default.is_none()),
                "{name}"
            );
            assert!(view.resources.len() >= 6, "{name}");
            assert!(!view.outputs.is_empty(), "{name}");
            assert!(!view.conditions.is_empty(), "{name}");
            assert!(!view.mappings.is_empty(), "{name}");
            assert!(!view.unused_parameters.is_empty(), "{name}");
        }
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::CloudformationCore),
            plugin_api::PluginPresentation::extensions(&crate::CloudformationPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
