//! Dockerfile file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// Dockerfile instruction keywords that mark a line as a directive, checked
/// case-sensitively at the start of a trimmed line (Dockerfiles do not
/// carry a reliable extension, so this plugin cannot sniff by filename;
/// this project's plugin architecture sniffs content only, per
/// `plugin_api::PluginCore::sniff`). Matched case-sensitively, in the
/// uppercase style every Dockerfile linter and the Docker docs use, so a
/// plain-English sentence starting "From here, ..." does not false-positive
/// — none of these keywords is used by any sibling plugin.
const INSTRUCTION_MARKERS: &[&str] = &[
    "FROM ",
    "RUN ",
    "COPY ",
    "ADD ",
    "WORKDIR ",
    "EXPOSE ",
    "ENV ",
    "ARG ",
    "USER ",
    "LABEL ",
    "VOLUME ",
    "ENTRYPOINT ",
    "CMD ",
    "ONBUILD ",
    "HEALTHCHECK ",
    "STOPSIGNAL ",
    "SHELL ",
    "MAINTAINER ",
];

/// The `# syntax=` parser directive comment, conventionally a Dockerfile's
/// first line when present.
const SYNTAX_DIRECTIVE: &str = "# syntax=";

/// View data produced by [`DockerfileCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerfileView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Base images from `FROM` instructions, in source order.
    pub base_images: Vec<String>,
    /// Ports from `EXPOSE` instructions, in source order.
    pub exposed_ports: Vec<String>,
}

/// Extracts the image reference from a `FROM image[:tag] [AS name]` line, or
/// `None` if `trimmed` is not such an instruction.
fn parse_base_image(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix("FROM ")?.trim_start();
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let image = &rest[..end];
    (!image.is_empty()).then_some(image)
}

/// Extracts the port list from an `EXPOSE port [port ...]` line, or `None`
/// if `trimmed` is not such an instruction.
fn parse_exposed_ports(trimmed: &str) -> Option<impl Iterator<Item = &str>> {
    let rest = trimmed.strip_prefix("EXPOSE ")?.trim_start();
    (!rest.is_empty()).then(|| rest.split_whitespace())
}

/// Parses `FROM` and `EXPOSE` instructions out of `content`, in source
/// order.
fn parse_instructions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut base_images = Vec::new();
    let mut exposed_ports = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(image) = parse_base_image(trimmed) {
            base_images.push(image.to_owned());
        } else if let Some(ports) = parse_exposed_ports(trimmed) {
            exposed_ports.extend(ports.map(str::to_owned));
        }
    }
    (base_images, exposed_ports)
}

/// Whether `text` looks like a Dockerfile, per [`INSTRUCTION_MARKERS`] and
/// [`SYNTAX_DIRECTIVE`].
fn has_dockerfile_syntax(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with(SYNTAX_DIRECTIVE)
            || INSTRUCTION_MARKERS
                .iter()
                .any(|marker| trimmed.starts_with(marker))
    })
}

/// The Dockerfile plugin's core half.
#[derive(Debug, Default)]
pub struct DockerfileCore;

impl PluginCore for DockerfileCore {
    fn name(&self) -> &'static str {
        "dockerfile"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_dockerfile_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (base_images, exposed_ports) = parse_instructions(&content);
        let view = DockerfileView {
            content,
            truncated,
            base_images,
            exposed_ports,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Dockerfile plugin's presentation half.
#[derive(Debug, Default)]
pub struct DockerfilePresentation;

impl PluginPresentation for DockerfilePresentation {
    fn name(&self) -> &'static str {
        "dockerfile"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: DockerfileView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.base_images.is_empty() {
            lines.push(format!("base images: {}", view.base_images.join(", ")));
        }
        if !view.exposed_ports.is_empty() {
            lines.push(format!("exposed ports: {}", view.exposed_ports.join(", ")));
        }
        lines.extend(view.content.lines().map(str::to_owned));
        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{DockerfileCore, DockerfilePresentation, DockerfileView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-dockerfile-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_dockerfile_markers_as_dockerfile() {
        assert!(DockerfileCore.sniff(b"FROM ubuntu:22.04\n"));
        assert!(DockerfileCore.sniff(b"# syntax=docker/dockerfile:1\nFROM scratch\n"));
        assert!(DockerfileCore.sniff(b"RUN apt-get update && apt-get install -y curl\n"));
        assert!(DockerfileCore.sniff(b"COPY . /app\n"));
        assert!(DockerfileCore.sniff(b"WORKDIR /app\n"));
        assert!(DockerfileCore.sniff(b"EXPOSE 8080\n"));
        assert!(DockerfileCore.sniff(b"ENTRYPOINT [\"/app/run\"]\n"));
        assert!(DockerfileCore.sniff(b"HEALTHCHECK CMD curl -f http://localhost/ || exit 1\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_dockerfile() {
        assert!(!DockerfileCore.sniff(b"From here, we changed direction on the roadmap.\n"));
        assert!(!DockerfileCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!DockerfileCore.sniff(b"#!/bin/bash\nif [ -f foo ]; then\n  echo hi\nfi\n"));
        assert!(!DockerfileCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(
            !DockerfileCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n")
        );
        assert!(!DockerfileCore.sniff(b"just a regular line of text\n"));
        assert!(!DockerfileCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn does_not_misclaim_a_run_instruction_containing_shell_syntax() {
        assert!(DockerfileCore.sniff(
            b"FROM alpine\nRUN VERSION=$(cat VERSION) && \\\n    if [ -z \"$VERSION\" ]; then \\\n      exit 1; \\\n    fi\n"
        ));
    }

    #[test]
    fn views_a_real_dockerfile_and_extracts_instructions() {
        let path = unique_temp_file("Dockerfile");
        std::fs::write(
            &path,
            "FROM node:18-alpine AS builder\nWORKDIR /app\nCOPY . .\nRUN npm install\n\nFROM node:18-alpine\nEXPOSE 3000\nEXPOSE 3001 3002\nCMD [\"node\", \"server.js\"]\n",
        )
        .unwrap();

        let data = DockerfileCore.view(&path).unwrap();
        let view: DockerfileView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.base_images, vec!["node:18-alpine", "node:18-alpine"]);
        assert_eq!(view.exposed_ports, vec!["3000", "3001", "3002"]);
        assert!(view.content.contains("npm install"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("Dockerfile.large");
        let mut content = "FROM scratch\n".to_owned();
        content.push_str(&"# a comment line\n".repeat(MAX_VIEW_BYTES));
        std::fs::write(&path, content).unwrap();

        let data = DockerfileCore.view(&path).unwrap();
        let view: DockerfileView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_base_images_ports_and_content() {
        let data = serde_json::to_value(DockerfileView {
            content: "FROM alpine\nEXPOSE 8080".to_owned(),
            truncated: false,
            base_images: vec!["alpine".to_owned()],
            exposed_ports: vec!["8080".to_owned()],
        })
        .unwrap();

        let lines = DockerfilePresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "base images: alpine",
                "exposed ports: 8080",
                "FROM alpine",
                "EXPOSE 8080"
            ]
        );
    }
}
