//! Terraform (HCL) file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`TerraformCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerraformView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Top-level block headers found in source order, dotted from keyword
    /// down to label (e.g. `resource.aws_instance.example`, `variable.
    /// region`, `terraform`).
    pub blocks: Vec<String>,
}

/// The quoted string contents of `s`, in order, e.g. `"a" "b"` yields `["a",
/// "b"]`. Good enough for a sniff/extraction heuristic; does not handle
/// escaped quotes.
fn quoted_strings(s: &str) -> impl Iterator<Item = &str> {
    s.split('"').skip(1).step_by(2)
}

/// Whether `line`, trimmed, opens a top-level HCL block this project
/// recognises as Terraform's own vocabulary, and if so, a dotted descriptor
/// naming it. A block header ends its line with a bare `{` (HCL block
/// syntax), preceded by one of Terraform's reserved block keywords and,
/// for most of them, one or two quoted labels:
///
/// - `resource`/`data` take two labels (type and name), e.g. `resource
///   "aws_instance" "example" {`.
/// - `variable`/`output`/`module`/`provider` take one label, e.g.
///   `variable "region" {`.
/// - `terraform`/`locals` take none, e.g. `terraform {`.
///
/// These keyword-plus-quoted-label-plus-brace headers are markers not used
/// by any sibling plugin.
fn terraform_block(line: &str) -> Option<String> {
    let header = line.trim().strip_suffix('{')?.trim_end();
    let (keyword, rest) = match header.split_once(char::is_whitespace) {
        Some((keyword, rest)) => (keyword, rest.trim()),
        None => (header, ""),
    };
    match keyword {
        "resource" | "data" => {
            let mut labels = quoted_strings(rest);
            let kind = labels.next()?;
            let name = labels.next()?;
            Some(format!("{keyword}.{kind}.{name}"))
        }
        "variable" | "output" | "module" | "provider" => {
            let name = quoted_strings(rest).next()?;
            Some(format!("{keyword}.{name}"))
        }
        "terraform" | "locals" if rest.is_empty() => Some(keyword.to_owned()),
        _ => None,
    }
}

/// Parses top-level block descriptors out of `content`, in source order.
fn parse_blocks(content: &str) -> Vec<String> {
    content.lines().filter_map(terraform_block).collect()
}

/// Whether `text` looks like a Terraform (HCL) configuration: at least one
/// line opens a recognised top-level block (see [`terraform_block`]). A
/// `.tfvars` file consisting only of flat `key = "value"` assignments and
/// no block header carries none of these markers and is indistinguishable
/// by content alone from TOML's own key/value syntax; this is an accepted
/// content-sniffing limitation, matching this project's precedent for
/// other structurally-ambiguous formats.
fn has_terraform_syntax(text: &str) -> bool {
    text.lines().any(|line| terraform_block(line).is_some())
}

/// The Terraform plugin's core half.
#[derive(Debug, Default)]
pub struct TerraformCore;

impl PluginCore for TerraformCore {
    fn name(&self) -> &'static str {
        "terraform"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_terraform_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let blocks = parse_blocks(&content);
        let view = TerraformView {
            content,
            truncated,
            blocks,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Terraform plugin's presentation half.
#[derive(Debug, Default)]
pub struct TerraformPresentation;

impl PluginPresentation for TerraformPresentation {
    fn name(&self) -> &'static str {
        "terraform"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: TerraformView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.blocks.is_empty() {
            lines.push(format!("blocks: {}", view.blocks.join(", ")));
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
    use super::{MAX_VIEW_BYTES, TerraformCore, TerraformPresentation, TerraformView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-terraform-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_terraform_blocks_as_terraform() {
        assert!(TerraformCore.sniff(b"terraform {\n  required_version = \">= 1.0\"\n}\n"));
        assert!(TerraformCore.sniff(b"provider \"aws\" {\n  region = \"us-west-2\"\n}\n"));
        assert!(
            TerraformCore.sniff(b"resource \"aws_instance\" \"example\" {\n  ami = \"abc\"\n}\n")
        );
        assert!(TerraformCore.sniff(b"data \"aws_ami\" \"latest\" {\n  most_recent = true\n}\n"));
        assert!(TerraformCore.sniff(b"variable \"region\" {\n  type = string\n}\n"));
        assert!(TerraformCore.sniff(b"output \"instance_ip\" {\n  value = \"x\"\n}\n"));
        assert!(TerraformCore.sniff(b"module \"vpc\" {\n  source = \"./vpc\"\n}\n"));
        assert!(TerraformCore.sniff(b"locals {\n  name = \"widgets\"\n}\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_terraform() {
        assert!(!TerraformCore.sniff(b"name = \"widgets\"\nversion = \"0.1.0\"\n"));
        assert!(!TerraformCore.sniff(b"[package]\nname = \"widgets\"\n"));
        assert!(!TerraformCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!TerraformCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!TerraformCore.sniff(b"just a regular line of text\n"));
        assert!(!TerraformCore.sniff(b""));
        assert!(!TerraformCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_terraform_file_and_extracts_blocks() {
        let path = unique_temp_file("main.tf");
        std::fs::write(
            &path,
            "terraform {\n  required_version = \">= 1.0\"\n}\n\nprovider \"aws\" {\n  region = \"us-west-2\"\n}\n\nresource \"aws_instance\" \"example\" {\n  ami           = \"abc\"\n  instance_type = \"t3.micro\"\n}\n\nvariable \"region\" {\n  type    = string\n  default = \"us-west-2\"\n}\n\noutput \"instance_id\" {\n  value = aws_instance.example.id\n}\n",
        )
        .unwrap();

        let data = TerraformCore.view(&path).unwrap();
        let view: TerraformView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(
            view.blocks,
            vec![
                "terraform",
                "provider.aws",
                "resource.aws_instance.example",
                "variable.region",
                "output.instance_id",
            ]
        );
        assert!(view.content.contains("t3.micro"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.tf");
        let mut content = "terraform {\n".to_owned();
        content.push_str(&"  # padding\n".repeat(MAX_VIEW_BYTES));
        content.push_str("}\n");
        std::fs::write(&path, content).unwrap();

        let data = TerraformCore.view(&path).unwrap();
        let view: TerraformView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_blocks_and_content() {
        let data = serde_json::to_value(TerraformView {
            content: "resource \"aws_instance\" \"example\" {\n  ami = \"abc\"\n}".to_owned(),
            truncated: false,
            blocks: vec!["resource.aws_instance.example".to_owned()],
        })
        .unwrap();

        let lines = TerraformPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "blocks: resource.aws_instance.example",
                "resource \"aws_instance\" \"example\" {",
                "  ami = \"abc\"",
                "}",
            ]
        );
    }
}
