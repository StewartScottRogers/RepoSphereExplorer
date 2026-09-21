//! Docker Compose file type plugin: core and presentation halves.
//!
//! A specialisation of YAML: a top-level `services:` mapping whose entries
//! carry `image:` or `build:` is a Compose file and nothing else.

use plugin_api::{Icon, PluginCore, PluginPresentation, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;
use syntax::{Language, Quote};

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

/// The value of the first `key:` at any depth within the block starting at
/// `from`.
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

/// The index of the first `key:` at any depth within the block starting at
/// `from`, or `None`.
fn nested_line(lines: &[&str], from: usize, wanted: &str) -> Option<usize> {
    let base = indent(lines.get(from)?);
    for (index, line) in lines.iter().enumerate().skip(from + 1) {
        if line.trim().is_empty() {
            continue;
        }
        if indent(line) <= base {
            break;
        }
        if key_of(line) == Some(wanted) {
            return Some(index);
        }
    }
    None
}

/// The keys directly one level inside the block that starts at `from`.
fn children_of(lines: &[&str], from: usize) -> Vec<String> {
    let base = indent(lines[from]);
    let mut depth: Option<usize> = None;
    let mut found = Vec::new();
    for line in lines.iter().skip(from + 1) {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let here = indent(line);
        if here <= base {
            break;
        }
        let depth = *depth.get_or_insert(here);
        if here == depth
            && let Some(key) = key_of(line)
        {
            found.push(key.to_owned());
        }
    }
    found
}

/// The items of a `key:` list at `key_at`, whether written in flow style or
/// as a block.
fn list_under(lines: &[&str], key_at: usize) -> Vec<String> {
    if let Some(flow) = value_of(lines[key_at]) {
        return flow
            .trim_matches(['[', ']'])
            .split(',')
            .map(|one| one.trim().trim_matches(['"', '\'']).to_owned())
            .filter(|one| !one.is_empty())
            .collect();
    }
    let base = indent(lines[key_at]);
    lines
        .iter()
        .skip(key_at + 1)
        .take_while(|candidate| candidate.trim().is_empty() || indent(candidate) > base)
        .filter_map(|candidate| {
            candidate
                .trim_start()
                .strip_prefix("- ")
                .map(|one| one.trim().trim_matches(['"', '\'']).to_owned())
        })
        .collect()
}

/// The build context of the service at `service_at`, from either
/// `build: <path>` or a `build:` block carrying its own `context:`.
fn build_context_of(lines: &[&str], service_at: usize) -> Option<String> {
    let build_at = nested_line(lines, service_at, "build")?;
    if let Some(value) = value_of(lines[build_at]) {
        return Some(value);
    }
    nested_value(lines, build_at, "context")
}

/// The environment variable names a service declares, whether written as a
/// mapping (`KEY: value`) or a list (`- KEY=value` / `- KEY`). Never the
/// values.
fn environment_keys_of(lines: &[&str], env_at: usize) -> Vec<String> {
    let mapping = children_of(lines, env_at);
    if !mapping.is_empty() {
        return mapping;
    }
    list_under(lines, env_at)
        .into_iter()
        .map(|item| item.split('=').next().unwrap_or_default().to_owned())
        .collect()
}

/// The service names a `depends_on:` names, whether written as a mapping
/// (with conditions) or a plain list.
fn depends_on_of(lines: &[&str], at: usize) -> Vec<String> {
    let mapping = children_of(lines, at);
    if !mapping.is_empty() {
        return mapping;
    }
    list_under(lines, at)
}

/// One entry of a `volumes:` list, in its short `source:target[:mode]`
/// syntax. The long, multi-line mapping form is not parsed, matching this
/// project's precedent for other structurally-sniffed formats.
fn mount_of(entry: &str) -> Option<Mount> {
    let mut parts = entry.splitn(3, ':');
    let source = parts.next()?.to_owned();
    let target = parts.next()?.to_owned();
    if target.is_empty() {
        return None;
    }
    // A host path is what makes a mount a bind mount rather than a
    // reference to a named volume declared under the top-level `volumes:`.
    let bind = source.starts_with('.') || source.starts_with('/') || source.starts_with('~');
    Some(Mount {
        source,
        target,
        bind,
    })
}

/// One service's volume or bind mount.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mount {
    /// The host path or named volume being mounted.
    pub source: String,
    /// Where it lands inside the container.
    pub target: String,
    /// Whether `source` is a host path (a bind mount) rather than a named
    /// volume declared under the top-level `volumes:`.
    pub bind: bool,
}

/// One service in the stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    /// Its key in the `services:` mapping.
    pub name: String,
    /// The local build context, when it is built rather than pulled.
    pub build_context: Option<String>,
    /// The image it pulls, when it is not built locally.
    pub image: Option<String>,
    /// Whether `image` is pinned to a digest rather than a tag that can
    /// move. `None` when the service has no image (it is built locally).
    pub image_pinned: Option<bool>,
    /// The ports it publishes to the host.
    pub ports: Vec<String>,
    /// The volumes and bind mounts it declares.
    pub volumes: Vec<Mount>,
    /// The environment variable names it expects, never their values.
    pub environment_keys: Vec<String>,
    /// The services it waits for.
    pub depends_on: Vec<String>,
    /// The profiles that gate it. Empty means it always runs.
    pub profiles: Vec<String>,
}

/// View data produced by [`ComposeCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeView {
    /// Every service, in file order.
    pub services: Vec<Service>,
    /// The named volumes declared at the top level.
    pub volumes: Vec<String>,
    /// The named networks declared at the top level.
    pub networks: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Everything [`ComposeView`] holds, read from `text`.
fn parse(text: &str) -> ComposeView {
    let lines: Vec<&str> = text.lines().collect();
    let mut view = ComposeView {
        services: Vec::new(),
        volumes: Vec::new(),
        networks: Vec::new(),
        content: String::new(),
        truncated: false,
    };

    if let Some(at) = lines
        .iter()
        .position(|line| indent(line) == 0 && key_of(line) == Some("volumes"))
    {
        view.volumes = children_of(&lines, at);
    }
    if let Some(at) = lines
        .iter()
        .position(|line| indent(line) == 0 && key_of(line) == Some("networks"))
    {
        view.networks = children_of(&lines, at);
    }

    let Some(services_at) = lines
        .iter()
        .position(|line| indent(line) == 0 && key_of(line) == Some("services"))
    else {
        return view;
    };

    for name in children_of(&lines, services_at) {
        let Some(at) = lines
            .iter()
            .enumerate()
            .skip(services_at + 1)
            .find(|(_, line)| key_of(line) == Some(name.as_str()))
            .map(|(index, _)| index)
        else {
            continue;
        };

        let build_context = build_context_of(&lines, at);
        let image = nested_value(&lines, at, "image");
        let image_pinned = image
            .as_ref()
            .map(|reference| reference.contains("@sha256:"));

        let ports = nested_line(&lines, at, "ports")
            .map(|idx| list_under(&lines, idx))
            .unwrap_or_default();
        let volumes = nested_line(&lines, at, "volumes")
            .map(|idx| list_under(&lines, idx))
            .unwrap_or_default()
            .iter()
            .filter_map(|entry| mount_of(entry))
            .collect();
        let environment_keys = nested_line(&lines, at, "environment")
            .map(|idx| environment_keys_of(&lines, idx))
            .unwrap_or_default();
        let depends_on = nested_line(&lines, at, "depends_on")
            .map(|idx| depends_on_of(&lines, idx))
            .unwrap_or_default();
        let profiles = nested_line(&lines, at, "profiles")
            .map(|idx| list_under(&lines, idx))
            .unwrap_or_default();

        view.services.push(Service {
            name,
            build_context,
            image,
            image_pinned,
            ports,
            volumes,
            environment_keys,
            depends_on,
            profiles,
        });
    }

    view
}

/// Whether `text` is a Docker Compose file.
///
/// `PluginCore::sniff` only ever receives a content prefix, never a path
/// (`crates/plugin-api/src/lib.rs`), so the file-name recognition
/// (`docker-compose.yml`, `compose.yaml`, ...) the issue asked for cannot be
/// honoured - the same conflict the Dockerfile (#39) and Makefile (#38)
/// plugins hit, resolved there by substituting a content-based sniff. This
/// does the same: a top-level `services:` mapping whose block carries at
/// least one `image:` or `build:` key.
fn looks_like_it(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let Some(at) = lines
        .iter()
        .position(|line| indent(line) == 0 && key_of(line) == Some("services"))
    else {
        return false;
    };
    let base = indent(lines[at]);
    lines
        .iter()
        .skip(at + 1)
        .take_while(|line| line.trim().is_empty() || indent(line) > base)
        .any(|line| key_of(line) == Some("image") || key_of(line) == Some("build"))
}

/// The Docker Compose plugin's core half.
#[derive(Debug, Default)]
pub struct ComposeCore;

/// How this language is coloured, for the shared tokeniser. GUIDANCE.md
/// §3.6: the plugin describes its own format, the pane paints what it is
/// told.
const COMPOSE: Language = Language {
    line_comment: &["#"],
    block_comment: &[],
    quotes: &[Quote::simple('"')],
    keywords: &[
        "build",
        "depends_on",
        "environment",
        "image",
        "networks",
        "ports",
        "profiles",
        "services",
        "volumes",
    ],
    types: &[],
    calls: false,
    ignore_case: false,
};

impl PluginCore for ComposeCore {
    fn name(&self) -> &'static str {
        "compose"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A specialisation of YAML, which owns the extension. Without this
        // the extension hint hands the file over whatever the order (D13).
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

/// The Docker Compose plugin's presentation half.
#[derive(Debug, Default)]
pub struct ComposePresentation;

impl PluginPresentation for ComposePresentation {
    fn classify(&self, text: &str) -> Vec<Span> {
        syntax::classify(text, &COMPOSE)
    }

    fn name(&self) -> &'static str {
        "compose"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "CMPS",
            tint: 0x001b_76d2,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: ComposeView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("{} service(s)", view.services.len()));
        for service in &view.services {
            let source = match (&service.build_context, &service.image) {
                (Some(context), _) => format!("built from {context}"),
                (None, Some(image)) => {
                    if service.image_pinned == Some(true) {
                        format!("{image} (pinned)")
                    } else {
                        format!("{image} (floating)")
                    }
                }
                (None, None) => "no image or build context".to_owned(),
            };
            lines.push(format!("  {}  {source}", service.name));
            if !service.ports.is_empty() {
                lines.push(format!("    ports: {}", service.ports.join(", ")));
            }
            if !service.volumes.is_empty() {
                for mount in &service.volumes {
                    let kind = if mount.bind { "bind" } else { "volume" };
                    lines.push(format!("    {kind}: {} -> {}", mount.source, mount.target));
                }
            }
            if !service.environment_keys.is_empty() {
                lines.push(format!(
                    "    environment: {}",
                    service.environment_keys.join(", ")
                ));
            }
            if !service.depends_on.is_empty() {
                lines.push(format!("    depends on: {}", service.depends_on.join(", ")));
            }
            if !service.profiles.is_empty() {
                lines.push(format!("    profiles: {}", service.profiles.join(", ")));
            }
        }
        if !view.volumes.is_empty() {
            lines.push(format!("Named volumes: {}", view.volumes.join(", ")));
        }
        if !view.networks.is_empty() {
            lines.push(format!("Networks: {}", view.networks.join(", ")));
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{ComposeCore, ComposePresentation, ComposeView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const COMPOSE_FILE: &str = "\
services:
  app:
    build:
      context: ./app
    ports:
      - \"8080:80\"
    volumes:
      - ./app:/usr/src/app
    environment:
      DATABASE_URL: postgres://db
    depends_on:
      - db
  db:
    image: postgres:15.4
    volumes:
      - db-data:/var/lib/postgresql/data
    profiles:
      - backend
  cache:
    image: redis@sha256:abc123
    depends_on:
      - db
volumes:
  db-data:
networks:
  backend:
";

    #[test]
    fn sniffs_a_compose_file() {
        assert!(ComposeCore.sniff(COMPOSE_FILE.as_bytes()));
    }

    #[test]
    fn does_not_claim_yaml_that_merely_has_a_services_key() {
        assert!(!ComposeCore.sniff(b"services:\n  app:\n    ports:\n      - 80\n"));
        assert!(!ComposeCore.sniff(b"other: thing\n"));
        assert!(!ComposeCore.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_yaml() {
        assert_eq!(ComposeCore.specialises(), &["yaml"]);
    }

    #[test]
    fn reads_each_service_by_how_it_gets_its_image() {
        let view = parse(COMPOSE_FILE);

        assert_eq!(view.services.len(), 3);
        assert_eq!(view.services[0].name, "app");
        assert_eq!(view.services[0].build_context.as_deref(), Some("./app"));
        assert!(view.services[0].image.is_none());
        assert_eq!(view.services[1].image.as_deref(), Some("postgres:15.4"));
        assert_eq!(view.services[1].image_pinned, Some(false));
        assert_eq!(
            view.services[2].image.as_deref(),
            Some("redis@sha256:abc123")
        );
        assert_eq!(view.services[2].image_pinned, Some(true));
    }

    #[test]
    fn tells_a_bind_mount_from_a_named_volume() {
        let view = parse(COMPOSE_FILE);

        assert_eq!(view.services[0].volumes.len(), 1);
        assert!(view.services[0].volumes[0].bind);
        assert_eq!(view.services[0].volumes[0].source, "./app");

        assert_eq!(view.services[1].volumes.len(), 1);
        assert!(!view.services[1].volumes[0].bind);
        assert_eq!(view.services[1].volumes[0].source, "db-data");
    }

    #[test]
    fn reads_ports_environment_keys_and_dependencies() {
        let view = parse(COMPOSE_FILE);

        assert_eq!(view.services[0].ports, vec!["8080:80".to_owned()]);
        assert_eq!(
            view.services[0].environment_keys,
            vec!["DATABASE_URL".to_owned()]
        );
        assert_eq!(view.services[0].depends_on, vec!["db".to_owned()]);
    }

    #[test]
    fn reads_profiles_and_top_level_volumes_and_networks() {
        let view = parse(COMPOSE_FILE);

        assert_eq!(view.services[1].profiles, vec!["backend".to_owned()]);
        assert_eq!(view.volumes, vec!["db-data".to_owned()]);
        assert_eq!(view.networks, vec!["backend".to_owned()]);
    }

    #[test]
    fn presents_the_pinned_and_floating_images() {
        let data = serde_json::to_value(parse(COMPOSE_FILE)).unwrap();

        let lines = ComposePresentation.present(&data);

        assert!(lines.iter().any(|line| line.contains("pinned")));
        assert!(lines.iter().any(|line| line.contains("floating")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/compose/docker-compose.yml");

        let data = ComposeCore.view(&path).unwrap();
        let view: ComposeView = serde_json::from_value(data).unwrap();

        assert_eq!(view.services.len(), 3);
        assert!(
            view.services
                .iter()
                .any(|service| service.build_context.is_some())
        );
        assert!(
            view.services
                .iter()
                .any(|service| service.image_pinned == Some(true))
        );
        assert!(
            view.services
                .iter()
                .any(|service| service.image_pinned == Some(false))
        );
        assert!(
            view.services
                .iter()
                .any(|service| !service.ports.is_empty())
        );
        assert!(
            view.services
                .iter()
                .any(|service| service.volumes.iter().any(|mount| mount.bind))
        );
        assert!(
            view.services
                .iter()
                .any(|service| service.volumes.iter().any(|mount| !mount.bind))
        );
        assert!(
            view.services
                .iter()
                .any(|service| !service.environment_keys.is_empty())
        );
        assert!(
            view.services
                .iter()
                .any(|service| !service.depends_on.is_empty())
        );
        assert!(
            view.services
                .iter()
                .any(|service| !service.profiles.is_empty())
        );
        assert!(!view.volumes.is_empty());
        assert!(!view.networks.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::ComposeCore),
            plugin_api::PluginPresentation::extensions(&crate::ComposePresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
