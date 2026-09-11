//! Verilog file type plugin: core and presentation halves.
//!
//! A Verilog or `SystemVerilog` file is modules, their ports and what
//! happens on a clock. This reads the modules with their parameters and
//! typed ports, the procedural blocks with what each is sensitive to,
//! the instantiated submodules, the interfaces, the assertions - and the
//! clocked blocks written as a bare `always`, which leaves the tools to
//! infer what was meant.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["v", "sv", "svh", "vh"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One port on a module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Port {
    /// Its name.
    pub name: String,
    /// `input`, `output` or `inout`.
    pub direction: String,
    /// Its type and width, as written.
    pub kind: String,
}

/// One module the file declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    /// Its name.
    pub name: String,
    /// Its parameters, as written.
    pub parameters: Vec<String>,
    /// Its ports, in declaration order.
    pub ports: Vec<Port>,
    /// Its procedural blocks, each with what it is sensitive to.
    pub blocks: Vec<String>,
    /// The submodules it instantiates, as `type name`.
    pub instances: Vec<String>,
}

/// View data produced by [`VerilogCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerilogView {
    /// Every module in the file.
    pub modules: Vec<Module>,
    /// The interfaces declared.
    pub interfaces: Vec<String>,
    /// The assertions, which are the file saying what must never happen.
    pub assertions: Vec<String>,
    /// Clocked blocks written as `always @(posedge ...)` rather than
    /// `always_ff`, which lets the tools infer latches the author did not
    /// ask for.
    pub untyped_clocked_blocks: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The directions a port may be declared with.
const DIRECTIONS: &[&str] = &["input", "output", "inout"];

/// `text` with its comments removed.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let opener = ["/*", "//"]
            .iter()
            .filter_map(|mark| rest.find(mark).map(|at| (at, *mark)))
            .min_by_key(|(at, _)| *at);
        let Some((at, mark)) = opener else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..at]);
        let after = &rest[at + mark.len()..];
        let closer = if mark == "/*" { "*/" } else { "\n" };
        match after.find(closer) {
            Some(end) => {
                if closer == "\n" {
                    out.push('\n');
                }
                rest = &after[end + closer.len()..];
            }
            None => return out,
        }
    }
}

/// The port `line` declares, if it declares one.
fn port_of(line: &str) -> Option<Port> {
    let trimmed = line.trim().trim_end_matches(',').trim_end_matches(';');
    let direction = DIRECTIONS
        .iter()
        .find(|name| trimmed.starts_with(&format!("{name} ")))?;
    let rest = trimmed[direction.len()..].trim();
    let name = rest.split_whitespace().next_back()?.to_owned();
    let kind = rest[..rest.len() - name.len()].trim().to_owned();
    (!name.is_empty()).then_some(Port {
        name,
        direction: (*direction).to_owned(),
        kind: if kind.is_empty() {
            "wire".to_owned()
        } else {
            kind
        },
    })
}

/// What a procedural block is sensitive to, and how it was written.
fn block_of(line: &str) -> Option<String> {
    let trimmed = line.trim();
    for keyword in [
        "always_ff",
        "always_comb",
        "always_latch",
        "always",
        "initial",
        "final",
    ] {
        let Some(rest) = trimmed.strip_prefix(keyword) else {
            continue;
        };
        if !rest.is_empty() && !rest.starts_with([' ', '@', '(', '\t']) {
            // `always_ff` must not be found inside `always_ffx`.
            continue;
        }
        let sensitivity = rest
            .trim()
            .trim_start_matches('@')
            .trim()
            .trim_end_matches("begin")
            .trim();
        return Some(if sensitivity.is_empty() {
            keyword.to_owned()
        } else {
            format!("{keyword} {sensitivity}")
        });
    }
    None
}

/// Lines, with a parameter override that spans several joined onto one.
///
/// An instantiation is written `Type #(...) name (...)`, and the override
/// in the middle is routinely spread over a handful of lines. Read one
/// line at a time, the type is on the first, the name on the last, and
/// neither line is an instantiation by itself.
fn logical_lines(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut pending: Option<(String, usize)> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if let Some((buffer, depth)) = pending.as_mut() {
            buffer.push(' ');
            buffer.push_str(line);
            if closes(line, depth) {
                out.push(
                    pending
                        .take()
                        .map_or_else(String::new, |(buffer, _)| buffer),
                );
            }
            continue;
        }
        if let Some(at) = line.find("#(") {
            let mut depth = 0usize;
            if !closes(&line[at..], &mut depth) && depth > 0 {
                pending = Some((line.to_owned(), depth));
                continue;
            }
        }
        out.push(line.to_owned());
    }
    if let Some((buffer, _)) = pending {
        out.push(buffer);
    }
    out
}

/// Applies `text`'s brackets to `depth`, and says whether it reached zero.
fn closes(text: &str, depth: &mut usize) -> bool {
    let mut reached_zero = false;
    for letter in text.chars() {
        match letter {
            '(' => *depth += 1,
            ')' => {
                *depth = depth.saturating_sub(1);
                if *depth == 0 {
                    reached_zero = true;
                }
            }
            _ => {}
        }
    }
    reached_zero
}

/// `line` with a `#( ... )` parameter override removed.
fn without_parameters(line: &str) -> String {
    let Some(at) = line.find("#(") else {
        return line.to_owned();
    };
    let mut depth = 0usize;
    for (offset, letter) in line[at..].char_indices() {
        match letter {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return format!("{} {}", &line[..at], &line[at + offset + 1..]);
                }
            }
            _ => {}
        }
    }
    line[..at].to_owned()
}

/// The parameters inside a `#( ... )` list, each as written.
fn parameter_list(line: &str) -> Vec<String> {
    let Some(at) = line.find("#(") else {
        return Vec::new();
    };
    let mut depth = 0usize;
    let mut end = line.len();
    for (offset, letter) in line[at..].char_indices() {
        match letter {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = at + offset;
                    break;
                }
            }
            _ => {}
        }
    }
    line[at + 2..end]
        .split(',')
        .map(|part| {
            let part = part.trim();
            part.strip_prefix("parameter ")
                .unwrap_or(part)
                .trim()
                .to_owned()
        })
        .filter(|part| !part.is_empty())
        .collect()
}

/// The submodule `line` instantiates, if it instantiates one.
///
/// An instantiation is `Type #(...) name (...);` - two names and a
/// bracket - which is also the shape of a function call, so the keywords
/// that open a statement are excluded.
fn instance_of(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.starts_with('.') || trimmed.ends_with(';') {
        return None;
    }
    let stripped = without_parameters(trimmed);
    let open = stripped.find('(')?;
    let head = &stripped[..open];
    let words: Vec<&str> = head.split_whitespace().collect();
    let [kind, name] = words.as_slice() else {
        return None;
    };
    let is_name = |word: &str| {
        word.chars()
            .all(|letter| letter.is_alphanumeric() || letter == '_')
    };
    if !is_name(kind) || !is_name(name) || DIRECTIONS.contains(kind) {
        return None;
    }
    if ["module", "function", "task", "if", "for", "while", "case"].contains(kind) {
        return None;
    }
    Some(format!("{kind} {name}"))
}

/// Everything [`VerilogView`] holds, read from `text`.
fn parse(text: &str) -> VerilogView {
    let mut view = VerilogView {
        modules: Vec::new(),
        interfaces: Vec::new(),
        assertions: Vec::new(),
        untyped_clocked_blocks: Vec::new(),
        truncated: false,
    };
    for raw in logical_lines(&without_comments(text)) {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("module ") {
            let name = rest
                .split(['(', '#', ' ', ';'])
                .next()
                .unwrap_or(rest)
                .trim()
                .to_owned();
            view.modules.push(Module {
                name,
                // A module's parameters live in the `#( ... )` list of the
                // header, which `logical_lines` has joined onto this line.
                parameters: parameter_list(line),
                ports: Vec::new(),
                blocks: Vec::new(),
                instances: Vec::new(),
            });
            continue;
        }
        if let Some(rest) = line.strip_prefix("interface ") {
            view.interfaces.push(
                rest.split(['(', ';', ' '])
                    .next()
                    .unwrap_or(rest)
                    .trim()
                    .to_owned(),
            );
            continue;
        }
        if line.starts_with("assert ") || line.starts_with("assume ") || line.contains(" assert ") {
            view.assertions.push(line.trim_end_matches(';').to_owned());
            continue;
        }
        let Some(module) = view.modules.last_mut() else {
            continue;
        };
        if let Some(rest) = line.strip_prefix("parameter ") {
            module
                .parameters
                .push(rest.trim_end_matches([',', ';']).trim().to_owned());
            continue;
        }
        if let Some(port) = port_of(line) {
            module.ports.push(port);
            continue;
        }
        if let Some(block) = block_of(line) {
            // `always @(posedge ...)` leaves the tools to work out what
            // was meant; `always_ff` says it.
            if block.starts_with("always ") && block.contains("edge") {
                view.untyped_clocked_blocks
                    .push(format!("{} in {}", block, module.name));
            }
            module.blocks.push(block);
            continue;
        }
        if let Some(instance) = instance_of(line) {
            module.instances.push(instance);
        }
    }
    view
}

/// Whether `text` is Verilog or `SystemVerilog`.
fn looks_like_it(text: &str) -> bool {
    let stripped = without_comments(text);
    let view = parse(text);
    // `module` is a word half the world uses. `endmodule` is not.
    stripped.contains("endmodule") && (!view.modules.is_empty() || !view.interfaces.is_empty())
}

/// The Verilog plugin's core half.
#[derive(Debug, Default)]
pub struct VerilogCore;

impl PluginCore for VerilogCore {
    fn name(&self) -> &'static str {
        "verilog"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        // The modules and their ports are what a reader came for; the
        // bodies read better in the file itself.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Verilog plugin's presentation half.
#[derive(Debug, Default)]
pub struct VerilogPresentation;

impl PluginPresentation for VerilogPresentation {
    fn name(&self) -> &'static str {
        "verilog"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "VER",
            tint: 0x0000_6a8e,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: VerilogView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("{} module(s):", view.modules.len()));
        for module in &view.modules {
            lines.push(format!("  {}", module.name));
            if !module.parameters.is_empty() {
                lines.push(format!(
                    "      parameters: {}",
                    module.parameters.join(", ")
                ));
            }
            for port in &module.ports {
                lines.push(format!(
                    "      {} {} {}",
                    port.direction, port.kind, port.name
                ));
            }
            for block in &module.blocks {
                lines.push(format!("      {block}"));
            }
            for instance in &module.instances {
                lines.push(format!("      instantiates {instance}"));
            }
        }
        if !view.interfaces.is_empty() {
            lines.push(format!("Interfaces: {}", view.interfaces.join(", ")));
        }
        if !view.assertions.is_empty() {
            lines.push(format!("{} assertion(s):", view.assertions.len()));
            for assertion in &view.assertions {
                lines.push(format!("  {assertion}"));
            }
        }
        if !view.untyped_clocked_blocks.is_empty() {
            lines.push("Clocked with a bare `always`, which leaves the tools to".to_owned());
            lines.push("infer what was meant and can give you a latch you did".to_owned());
            lines.push("not ask for. `always_ff` says it instead:".to_owned());
            for block in &view.untyped_clocked_blocks {
                lines.push(format!("  {block}"));
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
    use super::{VerilogCore, VerilogPresentation, VerilogView, instance_of, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const SOURCE: &str = concat!(
        "// A counter with a synchronous reset.\n",
        "module counter #(\n",
        "    parameter int WIDTH = 8\n",
        ") (\n",
        "    input  logic             clk,\n",
        "    input  logic             rst_n,\n",
        "    input  logic             enable,\n",
        "    output logic [WIDTH-1:0] value\n",
        ");\n",
        "\n",
        "    always_ff @(posedge clk or negedge rst_n) begin\n",
        "        if (!rst_n) value <= '0;\n",
        "        else if (enable) value <= value + 1;\n",
        "    end\n",
        "\n",
        "    always @(posedge clk) begin\n",
        "        // written the old way, and never revisited\n",
        "    end\n",
        "\n",
        "    assert property (@(posedge clk) enable |-> ##1 value != $past(value));\n",
        "\n",
        "endmodule\n",
        "\n",
        "module top (\n",
        "    input logic clk,\n",
        "    input logic rst_n\n",
        ");\n",
        "    logic [7:0] count;\n",
        "    counter #(.WIDTH(8)) u_counter (\n",
        "        .clk(clk),\n",
        "        .rst_n(rst_n),\n",
        "        .enable(1'b1),\n",
        "        .value(count)\n",
        "    );\n",
        "endmodule\n",
    );

    #[test]
    fn sniffs_a_source_file() {
        assert!(VerilogCore.sniff(SOURCE.as_bytes()));
    }

    #[test]
    fn does_not_claim_anything_that_uses_the_word_module() {
        assert!(!VerilogCore.sniff(b"module.exports = { a: 1 };\n"));
        assert!(!VerilogCore.sniff(b""));
    }

    #[test]
    fn reads_the_ports_with_their_directions_and_widths() {
        let view = parse(SOURCE);

        assert_eq!(view.modules.len(), 2);
        let counter = &view.modules[0];
        assert_eq!(counter.name, "counter");
        assert_eq!(counter.ports.len(), 4);
        assert_eq!(counter.ports[0].direction, "input");
        assert_eq!(counter.ports[0].name, "clk");
        let value = counter.ports.iter().find(|p| p.name == "value").unwrap();
        assert_eq!(value.direction, "output");
        assert_eq!(value.kind, "logic [WIDTH-1:0]");
        assert_eq!(counter.parameters, vec!["int WIDTH = 8".to_owned()]);
    }

    #[test]
    fn reads_each_block_with_what_it_is_sensitive_to() {
        let view = parse(SOURCE);

        let counter = &view.modules[0];
        assert_eq!(counter.blocks.len(), 2);
        assert!(counter.blocks[0].starts_with("always_ff (posedge clk or negedge rst_n)"));
    }

    #[test]
    fn a_port_connection_is_not_an_instantiation() {
        assert_eq!(
            instance_of("counter #(.WIDTH(8)) u_counter ("),
            Some("counter u_counter".to_owned())
        );
        assert_eq!(
            instance_of(".clk(clk),"),
            None,
            "a dotted line is a connection"
        );
        assert_eq!(instance_of("value <= value + 1;"), None);
    }

    #[test]
    fn finds_the_instance_in_the_second_module() {
        let view = parse(SOURCE);

        assert_eq!(
            view.modules[1].instances,
            vec!["counter u_counter".to_owned()]
        );
    }

    #[test]
    fn names_the_block_clocked_the_old_way() {
        let view = parse(SOURCE);

        assert_eq!(view.untyped_clocked_blocks.len(), 1);
        assert!(view.untyped_clocked_blocks[0].contains("in counter"));
        assert!(
            !view.untyped_clocked_blocks[0].contains("always_ff"),
            "`always_ff` says what it means and is not reported"
        );
    }

    #[test]
    fn finds_the_assertion() {
        let view = parse(SOURCE);

        assert_eq!(view.assertions.len(), 1);
        assert!(view.assertions[0].contains("|->"));
    }

    #[test]
    fn presents_the_bare_always_with_its_reason() {
        let data = serde_json::to_value(parse(SOURCE)).unwrap();

        let lines = VerilogPresentation.present(&data);

        assert_eq!(lines[0], "2 module(s):");
        assert!(lines.iter().any(|line| line.contains("latch you did")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/verilog/rtl/counter.sv");

        let data = VerilogCore.view(&path).unwrap();
        let view: VerilogView = serde_json::from_value(data).unwrap();

        assert!(view.modules.len() >= 2);
        assert!(view.modules.iter().any(|m| !m.parameters.is_empty()));
        assert!(view.modules.iter().any(|m| m.ports.len() >= 4));
        assert!(view.modules.iter().any(|m| !m.instances.is_empty()));
        assert!(view.modules.iter().flat_map(|m| &m.blocks).count() >= 3);
        assert!(!view.interfaces.is_empty());
        assert!(!view.assertions.is_empty());
        assert!(!view.untyped_clocked_blocks.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::VerilogCore),
            plugin_api::PluginPresentation::extensions(&crate::VerilogPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
