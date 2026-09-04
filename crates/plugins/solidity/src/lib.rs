//! Solidity file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`SolidityCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolidityView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `contract`/`interface`/`library` declarations
    /// found in the content.
    pub contracts: Vec<String>,
}

/// Extracts the identifier following `keyword` (`"contract"`,
/// `"interface"`, or `"library"`) at the start of `line`, if present.
fn top_level_name<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(keyword)?.strip_prefix(' ')?;
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level contract, interface, and library names out of
/// `content`, in source order.
fn parse_definitions(content: &str) -> Vec<String> {
    let mut contracts = Vec::new();
    for line in content.lines() {
        if let Some(name) = top_level_name(line, "contract")
            .or_else(|| top_level_name(line, "interface"))
            .or_else(|| top_level_name(line, "library"))
        {
            contracts.push(name.to_owned());
        }
    }
    contracts
}

/// Whether `text` looks like Solidity source: a `pragma solidity` version
/// directive, an SPDX license header (`SPDX-License-Identifier:`), a
/// top-level `contract` declaration, a `mapping(` type, an `emit ` event, a
/// `modifier ` declaration, or the `msg.sender`/`msg.value` built-ins —
/// markers not used by any sibling plugin. Deliberately does not sniff a
/// bare `interface `/`library ` declaration on its own, since a bare
/// `interface Name {` line is also valid TypeScript and GraphQL syntax;
/// those are still recognized once a stronger Solidity-only marker is also
/// present. Placed just ahead of `text` in `CORE_PLUGINS`, no ordering
/// constraint against a specific sibling since it has no overlapping
/// markers.
fn has_solidity_syntax(text: &str) -> bool {
    text.contains("pragma solidity")
        || text.contains("SPDX-License-Identifier:")
        || text.contains("mapping(")
        || text.contains("emit ")
        || text.contains("modifier ")
        || text.contains("msg.sender")
        || text.contains("msg.value")
        || text
            .lines()
            .any(|line| top_level_name(line, "contract").is_some())
}

/// The Solidity plugin's core half.
#[derive(Debug, Default)]
pub struct SolidityCore;

impl PluginCore for SolidityCore {
    fn name(&self) -> &'static str {
        "solidity"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_solidity_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let contracts = parse_definitions(&content);
        let view = SolidityView {
            content,
            truncated,
            contracts,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Solidity plugin's presentation half.
#[derive(Debug, Default)]
pub struct SolidityPresentation;

impl PluginPresentation for SolidityPresentation {
    fn name(&self) -> &'static str {
        "solidity"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: SolidityView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.contracts.is_empty() {
            lines.push(format!("contracts: {}", view.contracts.join(", ")));
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
    use super::{MAX_VIEW_BYTES, SolidityCore, SolidityPresentation, SolidityView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-solidity-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_solidity_markers_as_solidity() {
        assert!(SolidityCore.sniff(
            b"// SPDX-License-Identifier: MIT\npragma solidity ^0.8.0;\n\ncontract Token {}\n"
        ));
        assert!(SolidityCore.sniff(
            b"pragma solidity ^0.8.0;\n\ninterface IToken {\n    function totalSupply() external view returns (uint256);\n}\n"
        ));
        assert!(SolidityCore.sniff(b"pragma solidity ^0.8.0;\n\nlibrary SafeMath {\n    function add(uint256 a, uint256 b) internal pure returns (uint256) {}\n}\n"));
        assert!(
            SolidityCore
                .sniff(b"contract Token {\n    mapping(address => uint256) public balances;\n}\n")
        );
        assert!(SolidityCore.sniff(
            b"contract Token {\n    modifier onlyOwner() { require(msg.sender == owner); _; }\n}\n"
        ));
        assert!(SolidityCore.sniff(b"contract Token {\n    event Transfer(address indexed from, address indexed to);\n    function pay() public payable { emit Transfer(msg.sender, address(this)); }\n}\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_solidity() {
        assert!(!SolidityCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!SolidityCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!SolidityCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!SolidityCore.sniff(b"interface Greeter {\n  greet(): string;\n}\n"));
        assert!(!SolidityCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!SolidityCore.sniff(b"just a regular line of text\n"));
        assert!(!SolidityCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_solidity_file_and_extracts_definitions() {
        let path = unique_temp_file("token.sol");
        std::fs::write(
            &path,
            "// SPDX-License-Identifier: MIT\npragma solidity ^0.8.0;\n\ninterface IToken {\n    function totalSupply() external view returns (uint256);\n}\n\ncontract Token is IToken {\n    mapping(address => uint256) public balances;\n\n    function totalSupply() external view override returns (uint256) {\n        return balances[msg.sender];\n    }\n}\n",
        )
        .unwrap();

        let data = SolidityCore.view(&path).unwrap();
        let view: SolidityView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.contracts, vec!["IToken", "Token"]);
        assert!(view.content.contains("pragma solidity"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.sol");
        let mut content = "pragma solidity ^0.8.0;\n\ncontract Token {\n".to_owned();
        content.push_str(&"    // padding\n".repeat(MAX_VIEW_BYTES));
        content.push('}');
        std::fs::write(&path, content).unwrap();

        let data = SolidityCore.view(&path).unwrap();
        let view: SolidityView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_contracts_and_content() {
        let data = serde_json::to_value(SolidityView {
            content: "contract Token {\n}".to_owned(),
            truncated: false,
            contracts: vec!["Token".to_owned()],
        })
        .unwrap();

        let lines = SolidityPresentation.present(&data);

        assert_eq!(lines, vec!["contracts: Token", "contract Token {", "}"]);
    }
}
