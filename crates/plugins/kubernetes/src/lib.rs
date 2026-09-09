//! Kubernetes manifest file type plugin: core and presentation halves.
//!
//! A specialisation of YAML: `apiVersion` and `kind` together at the top
//! level, with a `metadata:` mapping, is a manifest and nothing else.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;
/// How deep `line` is indented, in spaces.
fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The key of a `key:` or `key: value` line, at any depth.
fn key_of(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') || trimmed.starts_with('-') {
        return None;
    }
    let (key, rest) = trimmed.split_once(':')?;
    if !(rest.is_empty() || rest.starts_with(' ')) {
        return None;
    }
    let key = key.trim();
    if key.is_empty() || key.contains(' ') {
        return None;
    }
    Some(key)
}

/// The value of a `key: value` line, unquoted, or `None` when the line
/// only opens a block.
fn value_of(line: &str) -> Option<String> {
    let (_, rest) = line.trim_start().split_once(':')?;
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    Some(rest.trim_matches(['"', '\'']).to_owned())
}

/// The value of the first `key:` at indentation `depth` or deeper, within
/// the block starting at `from`.
fn nested_value(lines: &[&str], from: usize, wanted: &str) -> Option<String> {
    let base = indent(lines.get(from)?);
    for line in lines.iter().skip(from + 1) {
        if line.trim().is_empty() {
            continue;
        }
        if indent(line) <= base {
            break;
        }
        if key_of(line) == Some(wanted) {
            return value_of(line);
        }
    }
    None
}

/// One document in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    /// Its `kind`.
    pub kind: String,
    /// Its `metadata.name`.
    pub name: Option<String>,
    /// Its `metadata.namespace`, when it declares one.
    pub namespace: Option<String>,
    /// The API group and version it is written against.
    pub api_version: String,
    /// The container images it names.
    pub images: Vec<String>,
}

/// View data produced by [`KubernetesCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesView {
    /// Every document, in order.
    pub resources: Vec<Resource>,
    /// Every container image named anywhere, each once.
    pub images: Vec<String>,
    /// Images pinned to a moving tag rather than a digest, which is what
    /// makes a deployment reproduce differently tomorrow.
    pub unpinned_images: Vec<String>,
    /// Whether any workload declares resource limits. A pod without them
    /// can take a node down with it.
    pub declares_limits: bool,
    /// The namespaces the documents name, each once.
    pub namespaces: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Splits `text` into its YAML documents.
fn documents(text: &str) -> Vec<Vec<&str>> {
    let mut found = vec![Vec::new()];
    for line in text.lines() {
        if line.trim() == "---" {
            found.push(Vec::new());
        } else {
            found
                .last_mut()
                .expect("there is always a current document")
                .push(line);
        }
    }
    found.retain(|document| document.iter().any(|line| !line.trim().is_empty()));
    found
}

/// Everything [`KubernetesView`] holds, read from `text`.
fn parse(text: &str) -> KubernetesView {
    let mut view = KubernetesView {
        resources: Vec::new(),
        images: Vec::new(),
        unpinned_images: Vec::new(),
        declares_limits: false,
        namespaces: Vec::new(),
        content: String::new(),
        truncated: false,
    };

    for document in documents(text) {
        let top = |wanted: &str| {
            document
                .iter()
                .find(|line| indent(line) == 0 && key_of(line) == Some(wanted))
                .and_then(|line| value_of(line))
        };
        let Some(kind) = top("kind") else { continue };

        let metadata_at = document
            .iter()
            .position(|line| indent(line) == 0 && key_of(line) == Some("metadata"));
        let name = metadata_at.and_then(|at| nested_value(&document, at, "name"));
        let namespace = metadata_at.and_then(|at| nested_value(&document, at, "namespace"));

        let mut images = Vec::new();
        for line in &document {
            if key_of(line) == Some("image")
                && let Some(image) = value_of(line)
            {
                images.push(image);
            }
            if key_of(line) == Some("limits") {
                view.declares_limits = true;
            }
        }
        for image in &images {
            if !view.images.contains(image) {
                // A digest pins; a tag moves, and `:latest` moves fastest.
                if !image.contains("@sha256:") {
                    view.unpinned_images.push(image.clone());
                }
                view.images.push(image.clone());
            }
        }
        if let Some(namespace) = &namespace
            && !view.namespaces.contains(namespace)
        {
            view.namespaces.push(namespace.clone());
        }

        view.resources.push(Resource {
            kind,
            name,
            namespace,
            api_version: top("apiVersion").unwrap_or_else(|| "unstated".to_owned()),
            images,
        });
    }
    view
}

/// Whether `text` is a Kubernetes manifest.
fn looks_like_it(text: &str) -> bool {
    documents(text).iter().any(|document| {
        let top = |wanted: &str| {
            document
                .iter()
                .any(|line| indent(line) == 0 && key_of(line) == Some(wanted))
        };
        top("apiVersion") && top("kind") && top("metadata")
    })
}

/// The Kubernetes manifest plugin's core half.
#[derive(Debug, Default)]
pub struct KubernetesCore;

impl PluginCore for KubernetesCore {
    fn name(&self) -> &'static str {
        "kubernetes"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A specialisation of YAML, which owns the extension (D13).
        &["yaml"]
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

/// The Kubernetes manifest plugin's presentation half.
#[derive(Debug, Default)]
pub struct KubernetesPresentation;

impl PluginPresentation for KubernetesPresentation {
    fn name(&self) -> &'static str {
        "kubernetes"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "K8S",
            tint: 0x0032_6ce5,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: KubernetesView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("{} resource(s)", view.resources.len()));
        for resource in &view.resources {
            let name = resource.name.as_deref().unwrap_or("unnamed");
            let namespace = resource
                .namespace
                .as_ref()
                .map_or_else(String::new, |ns| format!(" in {ns}"));
            lines.push(format!(
                "  {} {name}{namespace}  ({})",
                resource.kind, resource.api_version
            ));
        }
        if !view.namespaces.is_empty() {
            lines.push(format!("Namespaces: {}", view.namespaces.join(", ")));
        }
        if !view.images.is_empty() {
            lines.push(format!("Images ({}):", view.images.len()));
            for image in &view.images {
                lines.push(format!("  {image}"));
            }
        }
        if !view.unpinned_images.is_empty() {
            lines.push("Pinned to a tag, not a digest, so tomorrow may differ:".to_owned());
            for image in &view.unpinned_images {
                lines.push(format!("  {image}"));
            }
        }
        if !view.declares_limits {
            lines.push(
                "No resource limits declared: a pod without them can take a node with it."
                    .to_owned(),
            );
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{KubernetesCore, KubernetesPresentation, KubernetesView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const MANIFEST: &str = "apiVersion: apps/v1\nkind: Deployment\n\
        metadata:\n  name: app\n  namespace: production\n\
        spec:\n  template:\n    spec:\n      containers:\n\
        \x20       - name: app\n          image: example/app:1.4.0\n\
        \x20         resources:\n            limits:\n              cpu: 500m\n\
        ---\napiVersion: v1\nkind: Service\nmetadata:\n  name: app\n  namespace: production\n";

    #[test]
    fn sniffs_a_manifest() {
        assert!(KubernetesCore.sniff(MANIFEST.as_bytes()));
    }

    #[test]
    fn does_not_claim_yaml_with_only_one_of_the_keys() {
        assert!(!KubernetesCore.sniff(b"kind: regards\nfrom: ada\n"));
        assert!(!KubernetesCore.sniff(b"apiVersion: v1\nother: thing\n"));
        assert!(!KubernetesCore.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_yaml() {
        assert_eq!(KubernetesCore.specialises(), &["yaml"]);
    }

    #[test]
    fn reads_every_document_as_its_own_resource() {
        let view = parse(MANIFEST);

        assert_eq!(view.resources.len(), 2);
        assert_eq!(view.resources[0].kind, "Deployment");
        assert_eq!(view.resources[0].api_version, "apps/v1");
        assert_eq!(view.resources[1].kind, "Service");
        assert_eq!(view.namespaces, vec!["production".to_owned()]);
    }

    #[test]
    fn tells_a_digest_from_a_moving_tag() {
        let view = parse(MANIFEST);

        assert_eq!(view.unpinned_images, vec!["example/app:1.4.0".to_owned()]);

        let pinned = parse(
            "apiVersion: v1\nkind: Pod\nmetadata:\n  name: a\nspec:\n  containers:\n\
             \x20   - image: example/app@sha256:abc123\n",
        );

        assert!(pinned.unpinned_images.is_empty());
    }

    #[test]
    fn notices_when_nothing_declares_limits() {
        assert!(parse(MANIFEST).declares_limits);

        let without = parse("apiVersion: v1\nkind: Pod\nmetadata:\n  name: a\n");

        assert!(!without.declares_limits);
    }

    #[test]
    fn presents_the_missing_limits_warning() {
        let data = serde_json::to_value(parse("apiVersion: v1\nkind: Pod\nmetadata:\n  name: a\n"))
            .unwrap();

        let lines = KubernetesPresentation.present(&data);

        assert!(
            lines
                .iter()
                .any(|line| line.contains("take a node with it"))
        );
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/kubernetes/deployment.yaml");

        let data = KubernetesCore.view(&path).unwrap();
        let view: KubernetesView = serde_json::from_value(data).unwrap();

        assert!(view.resources.len() >= 4);
        assert!(view.images.len() >= 2);
        assert!(!view.unpinned_images.is_empty());
        assert!(view.images.iter().any(|image| image.contains("@sha256:")));
        assert!(view.declares_limits);
        assert!(!view.namespaces.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::KubernetesCore),
            plugin_api::PluginPresentation::extensions(&crate::KubernetesPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
