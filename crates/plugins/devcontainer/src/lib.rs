//! Dev container configuration (`devcontainer.json`) file type plugin: core
//! and presentation halves.
//!
//! A dev container configuration answers the question a reader asks before
//! they have built anything: how does this project expect me to get a
//! working environment? It is a JavaScript Object Notation (JSON) object
//! that may hold `//` and `/* */` comments and a trailing comma before a
//! closing bracket - liberties this reads past before handing the rest to
//! `serde_json`, the same way the JSON5 plugin reads a JSON5 document, but
//! narrower: only what a real editor's JSON with Comments (JSONC) mode
//! accepts.

use plugin_api::{Icon, PluginCore, PluginPresentation, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;
use syntax::{Language, Quote};

/// The lowercase extensions this type claims, without their dot. `none;
/// recognised by file name` per the work order - there is no extension a
/// configuration carries, only the bare name `devcontainer.json`, and this
/// plugin's sniff reads the object shape a configuration has instead.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One dev container feature layered onto the base image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feature {
    /// Its identifier, such as `ghcr.io/devcontainers/features/node`.
    pub id: String,
    /// The version pinned, from a trailing `:version` on the identifier or
    /// a `version` field in its options, when either is given. Only the
    /// tag-shaped suffix is read, matching this project's precedent
    /// (`compose`'s short volume syntax) of handling the common form of a
    /// syntax rather than every shape a registry address can take.
    pub version: Option<String>,
}

/// View data produced by [`DevcontainerCore::view`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevcontainerView {
    /// Whether the file parsed as a JSON object at all. When `false` every
    /// other field is empty, and the presentation half says so instead of
    /// showing a configuration with nothing in it.
    pub valid: bool,
    /// The base image, when it is pulled rather than built.
    pub image: Option<String>,
    /// The Dockerfile it defers to, when it is built rather than pulled or
    /// composed.
    pub dockerfile: Option<String>,
    /// The Docker Compose file(s) it defers to.
    pub compose_files: Vec<String>,
    /// The Compose service it runs as, when `compose_files` is not empty.
    pub service: Option<String>,
    /// The dev container features layered on, ordered by identifier -
    /// `serde_json`'s object map is a `BTreeMap`, so this is the order a
    /// reader gets regardless of how the file writes them.
    pub features: Vec<Feature>,
    /// The ports forwarded from the container.
    pub forward_ports: Vec<String>,
    /// The command run once, after the container is created.
    pub post_create_command: Option<String>,
    /// The command run every time the container starts.
    pub post_start_command: Option<String>,
    /// The command run every time an editor attaches to the container.
    pub post_attach_command: Option<String>,
    /// The user commands run as, and the editor connects as.
    pub remote_user: Option<String>,
    /// The user the container itself runs as.
    pub container_user: Option<String>,
    /// The bind mounts and volumes it declares, as the file writes them.
    pub mounts: Vec<String>,
    /// The editor extensions it standardises on.
    pub extensions: Vec<String>,
    /// The editor settings it standardises on, by key.
    pub settings_keys: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Where the scan below is: in ordinary text, a string, or a comment.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Where {
    /// Outside any string or comment.
    Text,
    /// Inside a string opened with a double quote.
    InString,
    /// Inside a `//` comment.
    LineComment,
    /// Inside a `/* */` comment.
    BlockComment,
}

/// Strips `//` and `/* */` comments and a trailing comma before a closing
/// `}` or `]`, so `serde_json` can parse a JSON with Comments (JSONC)
/// document - the liberties a dev container configuration is written with
/// by hand, and strict JSON has nowhere to put.
///
/// A single pass, because whether a `/` starts a comment or a `,` is
/// trailing depends on whether it is inside a string, and that is only
/// knowable in order.
fn strip_jsonc(text: &str) -> String {
    let letters: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut state = Where::Text;
    let mut escaped = false;
    let mut at = 0usize;
    while at < letters.len() {
        let letter = letters[at];
        let next = letters.get(at + 1).copied().unwrap_or(' ');
        match state {
            Where::LineComment => {
                if letter == '\n' {
                    out.push(letter);
                    state = Where::Text;
                }
            }
            Where::BlockComment => {
                if letter == '*' && next == '/' {
                    state = Where::Text;
                    at += 1;
                }
            }
            Where::InString => {
                out.push(letter);
                if escaped {
                    escaped = false;
                } else if letter == '\\' {
                    escaped = true;
                } else if letter == '"' {
                    state = Where::Text;
                }
            }
            Where::Text => match letter {
                '"' => {
                    out.push(letter);
                    state = Where::InString;
                }
                '/' if next == '/' => {
                    state = Where::LineComment;
                    at += 1;
                }
                '/' if next == '*' => {
                    state = Where::BlockComment;
                    at += 1;
                }
                ',' if trailing_comma(&letters, at + 1) => {
                    // Dropped: a strict reader stops at the comma before a
                    // closing bracket.
                }
                _ => out.push(letter),
            },
        }
        at += 1;
    }
    out
}

/// Whether the next significant character after `from` - skipping
/// whitespace and comments - is a closing `}` or `]`, which makes the comma
/// before it a trailing one a strict reader has no use for.
fn trailing_comma(letters: &[char], mut from: usize) -> bool {
    loop {
        while from < letters.len() && letters[from].is_whitespace() {
            from += 1;
        }
        if from + 1 < letters.len() && letters[from] == '/' && letters[from + 1] == '/' {
            from += 2;
            while from < letters.len() && letters[from] != '\n' {
                from += 1;
            }
            continue;
        }
        if from + 1 < letters.len() && letters[from] == '/' && letters[from + 1] == '*' {
            from += 2;
            while from + 1 < letters.len() && !(letters[from] == '*' && letters[from + 1] == '/') {
                from += 1;
            }
            from += 2;
            continue;
        }
        break;
    }
    matches!(letters.get(from), Some('}' | ']'))
}

/// `value` as a string, when it is one.
fn string_of(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

/// The Dockerfile a `build` field names, whether written as a bare path or
/// an object with its own `dockerfile` key. The deprecated top-level
/// `dockerFile` is read the same way.
fn dockerfile_of(object: &serde_json::Map<String, Value>) -> Option<String> {
    match object.get("build") {
        Some(Value::Object(build)) => build.get("dockerfile").and_then(string_of),
        _ => object.get("dockerFile").and_then(string_of),
    }
}

/// The Docker Compose file(s) a `dockerComposeFile` field names, whether
/// written as a bare string or an array of them.
fn compose_files_of(object: &serde_json::Map<String, Value>) -> Vec<String> {
    match object.get("dockerComposeFile") {
        Some(Value::String(path)) => vec![path.clone()],
        Some(Value::Array(paths)) => paths.iter().filter_map(string_of).collect(),
        _ => Vec::new(),
    }
}

/// The features an object under `features` layers on, each with the
/// version its identifier or options name, if either does.
fn features_of(object: &serde_json::Map<String, Value>) -> Vec<Feature> {
    let Some(features) = object.get("features").and_then(Value::as_object) else {
        return Vec::new();
    };
    features
        .iter()
        .map(|(id, options)| {
            let version = options
                .as_object()
                .and_then(|options| options.get("version"))
                .and_then(string_of)
                .or_else(|| id.rsplit_once(':').map(|(_, version)| version.to_owned()));
            Feature {
                id: id
                    .rsplit_once(':')
                    .map_or(id.clone(), |(id, _)| id.to_owned()),
                version,
            }
        })
        .collect()
}

/// The ports a `forwardPorts` array names, each a number or a string.
fn forward_ports_of(object: &serde_json::Map<String, Value>) -> Vec<String> {
    object
        .get("forwardPorts")
        .and_then(Value::as_array)
        .map(|ports| {
            ports
                .iter()
                .map(|port| match port {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A lifecycle command field's value, as one string: a bare command, an
/// argument array joined with spaces, or an object of named commands
/// joined with `; `.
fn command_of(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let words: Vec<String> = items.iter().filter_map(string_of).collect();
            (!words.is_empty()).then(|| words.join(" "))
        }
        Value::Object(named) => {
            let parts: Vec<String> = named
                .iter()
                .filter_map(|(name, command)| {
                    command_of(command).map(|command| format!("{name}: {command}"))
                })
                .collect();
            (!parts.is_empty()).then(|| parts.join("; "))
        }
        _ => None,
    }
}

/// One entry of a `mounts` array, in its short `key=value,...` string form
/// or the equivalent object form.
fn mount_of(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Object(mount) => {
            let field = |key: &str| mount.get(key).and_then(string_of);
            let parts: Vec<String> = [
                ("type", field("type")),
                ("source", field("source")),
                ("target", field("target")),
            ]
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| format!("{key}={value}")))
            .collect();
            (!parts.is_empty()).then(|| parts.join(","))
        }
        _ => None,
    }
}

/// The editor extensions and settings under `customizations.vscode`, the
/// key every editor built on the specification reads.
fn vscode_of(object: &serde_json::Map<String, Value>) -> (Vec<String>, Vec<String>) {
    let Some(vscode) = object
        .get("customizations")
        .and_then(Value::as_object)
        .and_then(|customizations| customizations.get("vscode"))
        .and_then(Value::as_object)
    else {
        return (Vec::new(), Vec::new());
    };
    let extensions = vscode
        .get("extensions")
        .and_then(Value::as_array)
        .map(|extensions| extensions.iter().filter_map(string_of).collect())
        .unwrap_or_default();
    let settings_keys = vscode
        .get("settings")
        .and_then(Value::as_object)
        .map(|settings| settings.keys().cloned().collect())
        .unwrap_or_default();
    (extensions, settings_keys)
}

/// Everything [`DevcontainerView`] holds, read from `text`.
fn parse(text: &str) -> DevcontainerView {
    let Ok(root) = serde_json::from_str::<Value>(&strip_jsonc(text)) else {
        return DevcontainerView::default();
    };
    let Some(object) = root.as_object() else {
        return DevcontainerView::default();
    };

    let (extensions, settings_keys) = vscode_of(object);

    DevcontainerView {
        valid: true,
        image: object.get("image").and_then(string_of),
        dockerfile: dockerfile_of(object),
        compose_files: compose_files_of(object),
        service: object.get("service").and_then(string_of),
        features: features_of(object),
        forward_ports: forward_ports_of(object),
        post_create_command: object.get("postCreateCommand").and_then(command_of),
        post_start_command: object.get("postStartCommand").and_then(command_of),
        post_attach_command: object.get("postAttachCommand").and_then(command_of),
        remote_user: object.get("remoteUser").and_then(string_of),
        container_user: object.get("containerUser").and_then(string_of),
        mounts: object
            .get("mounts")
            .and_then(Value::as_array)
            .map(|mounts| mounts.iter().filter_map(mount_of).collect())
            .unwrap_or_default(),
        extensions,
        settings_keys,
        content: String::new(),
        truncated: false,
    }
}

/// Whether `text` is a dev container configuration: a JSON object - which
/// may hold comments - with at least one of `image`, `build`,
/// `dockerComposeFile` or a `features` *object*.
///
/// `features` alone, without checking its shape, would also claim a
/// `GeoJSON` `FeatureCollection` - `samples/geojson/london.geojson` has a
/// top-level `features` too, but as the array of geometries the format
/// requires, never the feature-id-to-options mapping a dev container
/// configuration writes.
fn looks_like_it(text: &str) -> bool {
    let Ok(root) = serde_json::from_str::<Value>(&strip_jsonc(text)) else {
        return false;
    };
    let Some(object) = root.as_object() else {
        return false;
    };
    ["image", "build", "dockerComposeFile"]
        .iter()
        .any(|key| object.contains_key(*key))
        || matches!(object.get("features"), Some(Value::Object(_)))
}

/// The dev container configuration plugin's core half.
#[derive(Debug, Default)]
pub struct DevcontainerCore;

/// How this language is coloured, for the shared tokeniser. GUIDANCE.md
/// §3.6: the plugin describes its own format, the pane paints what it is
/// told.
const DEVCONTAINER: Language = Language {
    line_comment: &["//"],
    block_comment: &[("/*", "*/")],
    quotes: &[Quote::simple('"')],
    keywords: &[
        "build",
        "containerUser",
        "customizations",
        "dockerComposeFile",
        "features",
        "forwardPorts",
        "image",
        "mounts",
        "postAttachCommand",
        "postCreateCommand",
        "postStartCommand",
        "remoteUser",
        "service",
    ],
    types: &[],
    calls: false,
    ignore_case: false,
};

impl PluginCore for DevcontainerCore {
    fn name(&self) -> &'static str {
        "devcontainer"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A configuration is JSON, and `json` owns the extension. Without
        // this the extension hint hands the file over regardless of order.
        &["json"]
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

/// The dev container configuration plugin's presentation half.
#[derive(Debug, Default)]
pub struct DevcontainerPresentation;

impl PluginPresentation for DevcontainerPresentation {
    fn classify(&self, text: &str) -> Vec<Span> {
        syntax::classify(text, &DEVCONTAINER)
    }

    fn name(&self) -> &'static str {
        "devcontainer"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "DEVC",
            tint: 0x0025_92c4,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: DevcontainerView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        if !view.valid {
            return vec![
                "not a valid dev container configuration: could not parse it as JSON".to_owned(),
            ];
        }

        let mut lines = Vec::new();
        let source = match (&view.dockerfile, &view.image, view.compose_files.is_empty()) {
            (Some(dockerfile), _, _) => format!("built from {dockerfile}"),
            (None, Some(image), _) => format!("image: {image}"),
            (None, None, false) => match &view.service {
                Some(service) => format!(
                    "compose: {} (service {service})",
                    view.compose_files.join(", ")
                ),
                None => format!("compose: {}", view.compose_files.join(", ")),
            },
            (None, None, true) => "no image, Dockerfile or Compose file given".to_owned(),
        };
        lines.push(source);

        if !view.features.is_empty() {
            lines.push("features:".to_owned());
            for feature in &view.features {
                match &feature.version {
                    Some(version) => lines.push(format!("  {} @ {version}", feature.id)),
                    None => lines.push(format!("  {}", feature.id)),
                }
            }
        }
        if !view.forward_ports.is_empty() {
            lines.push(format!(
                "forwarded ports: {}",
                view.forward_ports.join(", ")
            ));
        }
        if let Some(command) = &view.post_create_command {
            lines.push(format!("postCreateCommand: {command}"));
        }
        if let Some(command) = &view.post_start_command {
            lines.push(format!("postStartCommand: {command}"));
        }
        if let Some(command) = &view.post_attach_command {
            lines.push(format!("postAttachCommand: {command}"));
        }
        if let Some(user) = &view.remote_user {
            lines.push(format!("remote user: {user}"));
        }
        if let Some(user) = &view.container_user {
            lines.push(format!("container user: {user}"));
        }
        if !view.mounts.is_empty() {
            for mount in &view.mounts {
                lines.push(format!("mount: {mount}"));
            }
        }
        if !view.extensions.is_empty() {
            lines.push(format!("editor extensions: {}", view.extensions.join(", ")));
        }
        if !view.settings_keys.is_empty() {
            lines.push(format!(
                "editor settings: {}",
                view.settings_keys.join(", ")
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
    use super::{DevcontainerCore, DevcontainerPresentation, DevcontainerView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const DOCKERFILE_CONFIG: &str = r#"{
      // A container built from this project's own Dockerfile.
      "name": "Web API dev container",
      "build": {
        "dockerfile": "Dockerfile",
        "context": ".."
      },
      "features": {
        "ghcr.io/devcontainers/features/node:1": { "version": "20" },
        "ghcr.io/devcontainers/features/docker-in-docker:2": {}
      },
      "forwardPorts": [3000, 5432],
      "postCreateCommand": "npm install",
      "postStartCommand": "npm run db:migrate",
      "remoteUser": "node",
      "mounts": [
        "source=${localWorkspaceFolder}/.cache,target=/home/node/.cache,type=bind"
      ],
      "customizations": {
        "vscode": {
          "extensions": ["dbaeumer.vscode-eslint", "esbenp.prettier-vscode"],
          "settings": {
            "editor.formatOnSave": true,
          },
        }
      },
    }"#;

    const COMPOSE_CONFIG: &str = r#"{
      "name": "Full stack (Compose)",
      "dockerComposeFile": ["../docker-compose.yml", "docker-compose.override.yml"],
      "service": "app",
      "postAttachCommand": "echo attached",
      "containerUser": "vscode"
    }"#;

    const IMAGE_CONFIG: &str = r#"{
      "image": "mcr.microsoft.com/devcontainers/base:ubuntu-22.04"
    }"#;

    #[test]
    fn sniffs_a_dev_container_configuration() {
        assert!(DevcontainerCore.sniff(DOCKERFILE_CONFIG.as_bytes()));
        assert!(DevcontainerCore.sniff(COMPOSE_CONFIG.as_bytes()));
        assert!(DevcontainerCore.sniff(IMAGE_CONFIG.as_bytes()));
    }

    #[test]
    fn does_not_claim_json_with_none_of_the_marker_keys() {
        assert!(!DevcontainerCore.sniff(br#"{"name": "a container", "workspaceFolder": "/w"}"#));
        assert!(!DevcontainerCore.sniff(b""));
        assert!(!DevcontainerCore.sniff(b"not json at all"));
    }

    #[test]
    fn it_says_it_specialises_json() {
        assert_eq!(DevcontainerCore.specialises(), &["json"]);
    }

    #[test]
    fn strips_comments_and_trailing_commas_before_parsing() {
        let view = parse(DOCKERFILE_CONFIG);
        assert!(view.valid);
        assert_eq!(view.dockerfile.as_deref(), Some("Dockerfile"));
    }

    #[test]
    fn reads_a_dockerfile_based_configuration() {
        let view = parse(DOCKERFILE_CONFIG);

        assert_eq!(view.dockerfile.as_deref(), Some("Dockerfile"));
        assert!(view.image.is_none());
        assert!(view.compose_files.is_empty());

        assert_eq!(view.features.len(), 2);
        let node = view
            .features
            .iter()
            .find(|feature| feature.id == "ghcr.io/devcontainers/features/node")
            .expect("node feature");
        assert_eq!(node.version.as_deref(), Some("20"));
        let docker_in_docker = view
            .features
            .iter()
            .find(|feature| feature.id == "ghcr.io/devcontainers/features/docker-in-docker")
            .expect("docker-in-docker feature");
        assert_eq!(docker_in_docker.version.as_deref(), Some("2"));

        assert_eq!(
            view.forward_ports,
            vec!["3000".to_owned(), "5432".to_owned()]
        );
        assert_eq!(view.post_create_command.as_deref(), Some("npm install"));
        assert_eq!(
            view.post_start_command.as_deref(),
            Some("npm run db:migrate")
        );
        assert_eq!(view.remote_user.as_deref(), Some("node"));
        assert_eq!(view.mounts.len(), 1);
        assert!(view.mounts[0].contains("target=/home/node/.cache"));
        assert_eq!(
            view.extensions,
            vec![
                "dbaeumer.vscode-eslint".to_owned(),
                "esbenp.prettier-vscode".to_owned()
            ]
        );
        assert_eq!(view.settings_keys, vec!["editor.formatOnSave".to_owned()]);
    }

    #[test]
    fn reads_a_compose_based_configuration() {
        let view = parse(COMPOSE_CONFIG);

        assert!(view.valid);
        assert_eq!(
            view.compose_files,
            vec![
                "../docker-compose.yml".to_owned(),
                "docker-compose.override.yml".to_owned()
            ]
        );
        assert_eq!(view.service.as_deref(), Some("app"));
        assert_eq!(view.post_attach_command.as_deref(), Some("echo attached"));
        assert_eq!(view.container_user.as_deref(), Some("vscode"));
    }

    #[test]
    fn reads_an_image_based_configuration() {
        let view = parse(IMAGE_CONFIG);

        assert!(view.valid);
        assert_eq!(
            view.image.as_deref(),
            Some("mcr.microsoft.com/devcontainers/base:ubuntu-22.04")
        );
    }

    #[test]
    fn a_malformed_document_is_refused_rather_than_panicking() {
        let view = parse("{ this is not valid json");

        assert!(!view.valid);
        assert!(view.image.is_none());
        assert_eq!(
            DevcontainerPresentation.present(&serde_json::to_value(view).unwrap()),
            vec!["not a valid dev container configuration: could not parse it as JSON"]
        );
    }

    #[test]
    fn a_truncated_document_is_refused_rather_than_panicking() {
        let view = parse("{ \"image\": \"debian\"");

        assert!(!view.valid);
    }

    #[test]
    fn presents_the_dockerfile_configuration() {
        let data = serde_json::to_value(parse(DOCKERFILE_CONFIG)).unwrap();

        let lines = DevcontainerPresentation.present(&data);

        assert!(
            lines
                .iter()
                .any(|line| line.contains("built from Dockerfile"))
        );
        assert!(lines.iter().any(|line| line.contains("@ 20")));
    }

    #[test]
    fn presents_a_default_view() {
        assert_eq!(
            DevcontainerPresentation
                .present(&serde_json::to_value(DevcontainerView::default()).unwrap()),
            vec!["not a valid dev container configuration: could not parse it as JSON"]
        );
    }

    #[test]
    fn the_repository_fixtures_fill_every_field() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/devcontainer");

        let mut views = Vec::new();
        for entry in [
            "dockerfile-project/devcontainer.json",
            "compose-project/devcontainer.json",
            "image-project/devcontainer.json",
        ] {
            let data = DevcontainerCore.view(&root.join(entry)).unwrap();
            views.push(serde_json::from_value::<DevcontainerView>(data).unwrap());
        }

        assert!(views.iter().any(|view| view.image.is_some()));
        assert!(views.iter().any(|view| view.dockerfile.is_some()));
        assert!(views.iter().any(|view| !view.compose_files.is_empty()));
        assert!(views.iter().any(|view| view.service.is_some()));
        assert!(views.iter().any(|view| !view.features.is_empty()));
        assert!(views.iter().any(|view| !view.forward_ports.is_empty()));
        assert!(views.iter().any(|view| view.post_create_command.is_some()));
        assert!(views.iter().any(|view| view.post_start_command.is_some()));
        assert!(views.iter().any(|view| view.post_attach_command.is_some()));
        assert!(views.iter().any(|view| view.remote_user.is_some()));
        assert!(views.iter().any(|view| view.container_user.is_some()));
        assert!(views.iter().any(|view| !view.mounts.is_empty()));
        assert!(views.iter().any(|view| !view.extensions.is_empty()));
        assert!(views.iter().any(|view| !view.settings_keys.is_empty()));
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::DevcontainerCore),
            plugin_api::PluginPresentation::extensions(&crate::DevcontainerPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
