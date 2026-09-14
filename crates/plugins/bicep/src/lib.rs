//! Bicep file type plugin: core and presentation halves.
//!
//! Bicep declares what should exist rather than what to do, so what a
//! reader wants from one is the shape of the result: what it takes in,
//! what it creates, and what it hands back.
//!
//! The target scope is read first because it decides where the whole
//! thing is deployed - a resource group, a subscription, a management
//! group or a tenant - and getting it wrong is the difference between
//! creating something and being told you cannot.

use plugin_api::{Icon, PluginCore, PluginPresentation, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;
use syntax::{Language, Quote};

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
///
/// `bicepparam` is the parameter file that goes with a template.
pub const EXTENSIONS: &[&str] = &["bicep", "bicepparam"];

/// How much of a template is read.
const READ_CAP: usize = 1024 * 1024;

/// How many of each kind are listed before the rest are only counted.
const SHOWN: usize = 64;

/// One parameter the template takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parameter {
    /// Its name.
    pub name: String,
    /// Its type.
    pub kind: String,
    /// Its default, when it has one. A parameter without one has to be
    /// supplied at deployment.
    pub default: Option<String>,
    /// Whether it is marked `@secure()`, so its value is never logged.
    pub secure: bool,
    /// What it is for, from an `@description`.
    pub description: Option<String>,
    /// The values `@allowed` restricts it to.
    pub allowed: Vec<String>,
}

/// One resource the template declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    /// The symbolic name the template refers to it by.
    pub symbol: String,
    /// Its Azure type.
    pub kind: String,
    /// The API version it is declared against.
    pub api_version: String,
    /// Whether it is declared as existing already rather than created.
    pub existing: bool,
}

/// One output the template hands back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    /// Its name.
    pub name: String,
    /// Its type.
    pub kind: String,
}

/// View data produced by [`BicepCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BicepView {
    /// Where the template is deployed. Absent means a resource group,
    /// which is the default and the one nobody writes down.
    pub target_scope: Option<String>,
    /// What it takes in.
    pub parameters: Vec<Parameter>,
    /// What it works out for itself.
    pub variables: Vec<String>,
    /// What it creates.
    pub resources: Vec<Resource>,
    /// The other templates it calls, as symbol and path.
    pub modules: Vec<String>,
    /// What it hands back.
    pub outputs: Vec<Output>,
    /// Whether the template was longer than this reads.
    pub truncated: bool,
}

/// Whether `text` reads like a Bicep template.
///
/// A resource declaration is the marker nothing else has: a name, a
/// quoted `type@version`, and an equals sign. `param` and `output`
/// lines on their own would match other languages.
fn looks_like_it(text: &str) -> bool {
    let mut markers = 0usize;
    for line in text.lines() {
        let line = line.trim_start();
        if line.starts_with("//") {
            continue;
        }
        if line.starts_with("targetScope ") || line.starts_with("targetScope=") {
            return true;
        }
        if line.starts_with("resource ")
            && line.contains('=')
            && quoted(line).is_some_and(|declared| declared.contains('@'))
        {
            // A quoted `type@version` after `resource` is Bicep's and
            // nothing else's.
            return true;
        }
        if line.starts_with("module ") && line.contains('\'') && line.contains('=') {
            markers += 2;
        }
        for opener in ["param ", "var ", "output "] {
            if line.starts_with(opener) && !line.contains(';') {
                markers += 1;
            }
        }
        if markers >= 3 {
            return true;
        }
    }
    false
}

/// The text inside the first pair of single quotes.
fn quoted(text: &str) -> Option<&str> {
    let start = text.find('\'')? + 1;
    let rest = &text[start..];
    Some(&rest[..rest.find('\'')?])
}

/// The word at the start of `text`, up to whitespace.
fn word(text: &str) -> &str {
    let text = text.trim_start();
    &text[..text.find(char::is_whitespace).unwrap_or(text.len())]
}

/// A decorator's name, which is followed by its bracket with no space
/// in between: `@description('...')` is the name `description`.
fn decorator_name(text: &str) -> &str {
    let text = text.trim_start();
    &text[..text
        .find(|character: char| character == '(' || character.is_whitespace())
        .unwrap_or(text.len())]
}

/// Everything [`BicepView`] holds, read from `source`.
///
/// Bicep's decorators sit on the lines *above* what they decorate, so
/// they are collected as the reader goes and attached to the next
/// declaration it meets.
fn parse(source: &str, truncated: bool) -> BicepView {
    let mut view = BicepView {
        target_scope: None,
        parameters: Vec::new(),
        variables: Vec::new(),
        resources: Vec::new(),
        modules: Vec::new(),
        outputs: Vec::new(),
        truncated,
    };
    let mut secure = false;
    let mut description: Option<String> = None;
    let mut allowed: Vec<String> = Vec::new();
    let mut in_allowed = false;

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if in_allowed {
            if trimmed.starts_with(']') {
                in_allowed = false;
            } else if let Some(value) = quoted(trimmed) {
                allowed.push(value.to_owned());
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('@') {
            match decorator_name(rest) {
                "secure" => secure = true,
                "description" | "sys.description" => {
                    description = quoted(rest).map(ToOwned::to_owned);
                }
                "allowed" => in_allowed = true,
                _ => {}
            }
            continue;
        }
        read_declaration(
            trimmed,
            &mut view,
            &mut secure,
            &mut description,
            &mut allowed,
        );
    }
    view
}

/// Reads one declaration, taking whatever decorators have piled up.
fn read_declaration(
    line: &str,
    view: &mut BicepView,
    secure: &mut bool,
    description: &mut Option<String>,
    allowed: &mut Vec<String>,
) {
    if let Some(rest) = line.strip_prefix("targetScope") {
        view.target_scope = quoted(rest).map(ToOwned::to_owned);
    } else if let Some(rest) = line.strip_prefix("param ") {
        let name = word(rest).to_owned();
        let after = rest[name.len()..].trim_start();
        let kind = word(after).to_owned();
        let default = after[kind.len()..]
            .trim_start()
            .strip_prefix('=')
            .map(|value| match value.trim() {
                // A default written across several lines leaves only its
                // opening bracket here, and a lone bracket says nothing.
                "{" => "(an object)".to_owned(),
                "[" => "(a list)".to_owned(),
                other => other.to_owned(),
            })
            .filter(|value| !value.is_empty());
        if view.parameters.len() < SHOWN {
            view.parameters.push(Parameter {
                name,
                kind,
                default,
                secure: *secure,
                description: description.clone(),
                allowed: std::mem::take(allowed),
            });
        }
    } else if let Some(rest) = line.strip_prefix("var ") {
        if view.variables.len() < SHOWN {
            view.variables.push(word(rest).to_owned());
        }
    } else if let Some(rest) = line.strip_prefix("resource ") {
        let symbol = word(rest).to_owned();
        let declared = quoted(rest).unwrap_or_default();
        let (kind, api_version) = declared.split_once('@').unwrap_or((declared, ""));
        if view.resources.len() < SHOWN {
            view.resources.push(Resource {
                symbol,
                kind: kind.to_owned(),
                api_version: api_version.to_owned(),
                existing: line.contains("existing"),
            });
        }
    } else if let Some(rest) = line.strip_prefix("module ") {
        let symbol = word(rest);
        let path = quoted(rest).unwrap_or_default();
        if view.modules.len() < SHOWN {
            view.modules.push(format!("{symbol} from {path}"));
        }
    } else if let Some(rest) = line.strip_prefix("output ") {
        let name = word(rest).to_owned();
        let kind = word(rest[name.len()..].trim_start()).to_owned();
        if view.outputs.len() < SHOWN {
            view.outputs.push(Output { name, kind });
        }
    } else {
        // Anything else ends the run of decorators without consuming
        // them, so they do not drift onto a later declaration.
        return;
    }
    *secure = false;
    *description = None;
}

/// Everything [`BicepView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<BicepView> {
    let source = std::fs::read_to_string(path)?;
    let truncated = source.len() > READ_CAP;
    let source = if truncated {
        let mut end = READ_CAP;
        while end > 0 && !source.is_char_boundary(end) {
            end -= 1;
        }
        &source[..end]
    } else {
        source.as_str()
    };
    if !looks_like_it(source) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a Bicep template",
        ));
    }
    Ok(parse(source, truncated))
}

/// The Bicep plugin's core half.
#[derive(Debug, Default)]
pub struct BicepCore;

/// How this language is coloured, for the shared tokeniser. GUIDANCE.md
/// §3.6: the plugin describes its own format, the pane paints what it is
/// told.
const BICEP: Language = Language {
    line_comment: &["//"],
    block_comment: &[("/*", "*/")],
    quotes: &[Quote::simple('"'), Quote::simple('\'')],
    keywords: &[
        "existing",
        "false",
        "for",
        "if",
        "import",
        "metadata",
        "module",
        "null",
        "output",
        "param",
        "resource",
        "targetScope",
        "true",
        "type",
        "var",
    ],
    types: &["array", "bool", "int", "object", "string"],
    calls: true,
    ignore_case: false,
};

impl PluginCore for BicepCore {
    fn name(&self) -> &'static str {
        "bicep"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Bicep plugin's presentation half.
#[derive(Debug, Default)]
pub struct BicepPresentation;

impl PluginPresentation for BicepPresentation {
    fn classify(&self, text: &str) -> Vec<Span> {
        syntax::classify(text, &BICEP)
    }

    fn name(&self) -> &'static str {
        "bicep"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "BCP",
            tint: 0x0000_5ba1,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: BicepView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "Bicep template: {} parameter(s), {} resource(s), {} output(s)",
            view.parameters.len(),
            view.resources.len(),
            view.outputs.len()
        )];
        lines.push(match &view.target_scope {
            Some(scope) => format!("Deployed to a {scope}"),
            None => "No targetScope, so it deploys to a resource group.".to_owned(),
        });
        if view.truncated {
            lines.push("Longer than this reads; what follows is the start.".to_owned());
        }
        if !view.parameters.is_empty() {
            lines.push("Takes:".to_owned());
            for parameter in &view.parameters {
                let mut said = format!("  {} {}", parameter.kind, parameter.name);
                match &parameter.default {
                    Some(default) => {
                        use std::fmt::Write as _;
                        let _ = write!(said, " = {default}");
                    }
                    None => said.push_str("  (must be supplied)"),
                }
                if parameter.secure {
                    said.push_str("  [secure]");
                }
                lines.push(said);
                if let Some(description) = &parameter.description {
                    lines.push(format!("      {description}"));
                }
                if !parameter.allowed.is_empty() {
                    lines.push(format!("      one of {}", parameter.allowed.join(", ")));
                }
            }
        }
        if !view.variables.is_empty() {
            lines.push(format!("Works out {}", view.variables.join(", ")));
        }
        if !view.resources.is_empty() {
            lines.push("Creates:".to_owned());
            for resource in &view.resources {
                let existing = if resource.existing {
                    "  (already exists; only referenced)"
                } else {
                    ""
                };
                lines.push(format!(
                    "  {} - {} @{}{existing}",
                    resource.symbol, resource.kind, resource.api_version
                ));
            }
        }
        if view.modules.is_empty() {
            lines.push("Calls no other template.".to_owned());
        } else {
            lines.push("Calls:".to_owned());
            for module in &view.modules {
                lines.push(format!("  {module}"));
            }
        }
        if !view.outputs.is_empty() {
            lines.push("Hands back:".to_owned());
            for output in &view.outputs {
                lines.push(format!("  {} {}", output.kind, output.name));
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{BicepCore, BicepPresentation, BicepView, looks_like_it, quoted};
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/bicep/main.bicep")
    }

    fn view_of() -> BicepView {
        serde_json::from_value(BicepCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&BicepCore),
            PluginPresentation::extensions(&BicepPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn recognises_a_resource_declaration() {
        assert!(looks_like_it("targetScope = 'subscription'\n"));
        assert!(looks_like_it(
            "resource a 'Microsoft.Storage/storageAccounts@2023-05-01' = {\n  name: 'x'\n}\n"
        ));
        assert!(
            !looks_like_it("param x int;\nvar y = 1;\noutput z int;\n"),
            "semicolons say this is another language borrowing the words"
        );
        assert!(!looks_like_it("resource: something\n"));
        assert!(!looks_like_it(""));
    }

    #[test]
    fn reads_the_target_scope() {
        let view = view_of();

        assert_eq!(view.target_scope.as_deref(), Some("resourceGroup"));
    }

    #[test]
    fn reads_the_parameters_with_their_types_and_defaults() {
        let view = view_of();

        assert_eq!(view.parameters.len(), 6);
        let location = &view.parameters[0];
        assert_eq!(location.name, "location");
        assert_eq!(location.kind, "string");
        assert_eq!(
            location.default.as_deref(),
            Some("resourceGroup().location")
        );
        assert_eq!(
            location.description.as_deref(),
            Some("Where everything is created.")
        );

        let name = &view.parameters[1];
        assert_eq!(name.name, "storageName");
        assert_eq!(name.default, None, "it has to be supplied");
    }

    #[test]
    fn a_decorator_attaches_to_the_declaration_below_it() {
        let view = view_of();

        let key = view
            .parameters
            .iter()
            .find(|one| one.name == "collectorKey")
            .expect("the secure parameter");
        assert!(key.secure, "`@secure()` sits on the line above it");
        assert!(
            !view
                .parameters
                .iter()
                .any(|one| one.name != "collectorKey" && one.secure),
            "and on nothing else"
        );

        let sku = view
            .parameters
            .iter()
            .find(|one| one.name == "storageSku")
            .expect("the restricted parameter");
        assert_eq!(sku.allowed, vec!["Standard_LRS", "Standard_GRS"]);
    }

    #[test]
    fn reads_the_resources_with_their_types_and_api_versions() {
        let view = view_of();

        assert_eq!(view.resources.len(), 4);
        let storage = &view.resources[0];
        assert_eq!(storage.symbol, "storage");
        assert_eq!(storage.kind, "Microsoft.Storage/storageAccounts");
        assert_eq!(storage.api_version, "2023-05-01");
        assert!(!storage.existing);
        assert!(
            view.resources
                .iter()
                .any(|one| one.kind.ends_with("queueServices/queues"))
        );
    }

    #[test]
    fn a_default_written_across_several_lines_is_said_in_words() {
        // Found in the running application, which showed a reader
        // `object tags = {`.
        let view = view_of();

        let tags = view
            .parameters
            .iter()
            .find(|one| one.name == "tags")
            .expect("the object parameter");
        assert_eq!(tags.default.as_deref(), Some("(an object)"));
    }

    #[test]
    fn reads_the_variables_the_modules_and_the_outputs() {
        let view = view_of();

        assert_eq!(
            view.variables,
            vec!["queueName", "containerName", "storageId"]
        );
        assert_eq!(view.modules.len(), 2);
        assert!(view.modules[0].contains("alerts from modules/alerts.bicep"));
        assert_eq!(view.outputs.len(), 4);
        assert_eq!(view.outputs[0].name, "storageAccountId");
        assert_eq!(view.outputs[0].kind, "string");
        assert!(view.outputs.iter().any(|one| one.kind == "int"));
    }

    #[test]
    fn a_quoted_value_stops_at_its_closing_quote() {
        assert_eq!(quoted("a 'b' c 'd'"), Some("b"));
        assert_eq!(quoted("no quotes"), None);
    }

    #[test]
    fn presents_the_scope_and_what_must_be_supplied() {
        let data = BicepCore.view(&fixture()).unwrap();

        let lines = BicepPresentation.present(&data);

        assert!(lines[0].starts_with("Bicep template: 6 parameter(s)"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Deployed to a resourceGroup"))
        );
        assert!(lines.iter().any(|line| line.contains("(must be supplied)")));
        assert!(lines.iter().any(|line| line.contains("[secure]")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("one of Standard_LRS, Standard_GRS"))
        );
    }

    #[test]
    fn a_file_that_is_not_a_template_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.bicep");
        std::fs::write(&path, b"nothing of the sort at all").unwrap();

        assert!(BicepCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
