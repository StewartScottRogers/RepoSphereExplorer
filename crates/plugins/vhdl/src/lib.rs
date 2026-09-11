//! VHDL file type plugin: core and presentation halves.
//!
//! A VHDL design is an entity - what it looks like from outside - and an
//! architecture, which is how it works. This reads the libraries and
//! packages, the entities with their generics and ports, the
//! architectures and what each belongs to, the processes with their
//! sensitivity lists, the signals, the component instances - and the
//! processes that run once and then never again.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["vhd", "vhdl"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One port or generic on an entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Port {
    /// Its name.
    pub name: String,
    /// `in`, `out`, `inout` or `buffer`; a generic has no mode.
    pub mode: Option<String>,
    /// Its type, as written.
    pub kind: String,
}

/// One entity the file declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    /// Its name.
    pub name: String,
    /// Its generics.
    pub generics: Vec<Port>,
    /// Its ports.
    pub ports: Vec<Port>,
}

/// One process inside an architecture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Process {
    /// Its label, when it has one.
    pub label: Option<String>,
    /// The signals it is sensitive to.
    pub sensitivity: Vec<String>,
    /// The architecture it is in.
    pub architecture: String,
}

/// View data produced by [`VhdlCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VhdlView {
    /// The libraries the file opens with.
    pub libraries: Vec<String>,
    /// The packages brought in with `use`.
    pub uses: Vec<String>,
    /// Every entity.
    pub entities: Vec<Entity>,
    /// Each architecture, with the entity it belongs to.
    pub architectures: Vec<String>,
    /// Every process.
    pub processes: Vec<Process>,
    /// The signals declared.
    pub signals: Vec<String>,
    /// The components instantiated, as `label: component`.
    pub instances: Vec<String>,
    /// Processes with an empty sensitivity list that are not clocked by a
    /// `wait`, which run once and then never again.
    pub processes_that_run_once: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// `text` with its comments removed. VHDL has only the line comment.
fn without_comments(text: &str) -> String {
    text.lines()
        .map(|line| match line.find("--") {
            Some(at) => &line[..at],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The declaration `line` states, as `name : mode kind`.
fn port_of(line: &str) -> Option<Port> {
    let mut trimmed = line.trim().trim_end_matches(';').trim();
    // The port list's own closing bracket often shares a line with the
    // last port, and has to go. The type's brackets must not:
    // `unsigned(width - 1 downto 0)` keeps its own.
    while trimmed.ends_with(')') && trimmed.matches(')').count() > trimmed.matches('(').count() {
        trimmed = trimmed[..trimmed.len() - 1].trim_end();
    }
    let (names, rest) = trimmed.split_once(':')?;
    let name = names.split(',').next()?.trim().to_owned();
    if name.is_empty()
        || !name
            .chars()
            .all(|letter| letter.is_alphanumeric() || letter == '_')
    {
        return None;
    }
    let rest = rest.trim();
    let lower = rest.to_ascii_lowercase();
    let mode = ["inout", "buffer", "in", "out"]
        .iter()
        .find(|mode| lower.starts_with(&format!("{mode} ")))
        .map(|mode| (*mode).to_owned());
    let kind = match &mode {
        Some(mode) => rest[mode.len()..].trim(),
        None => rest,
    };
    let kind = kind.split(":=").next().unwrap_or(kind).trim().to_owned();
    (!kind.is_empty()).then_some(Port { name, mode, kind })
}

/// The signals a `process (a, b)` line is sensitive to.
fn sensitivity_of(line: &str) -> Vec<String> {
    let Some(open) = line.find('(') else {
        return Vec::new();
    };
    let Some(close) = line[open..].rfind(')').map(|at| at + open) else {
        return Vec::new();
    };
    line[open + 1..close]
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The label a line opens with, as in `label : process` or `u1 : counter`.
fn label_of(line: &str) -> Option<String> {
    let (label, _) = line.split_once(':')?;
    let label = label.trim();
    (!label.is_empty()
        && label
            .chars()
            .all(|letter| letter.is_alphanumeric() || letter == '_'))
    .then(|| label.to_owned())
}

/// Reads a library, a use, an entity or an architecture line.
///
/// Returns whether `line` was one of them, in which case whatever port
/// list was open has ended.
fn read_header(line: &str, lower: &str, view: &mut VhdlView, architecture: &mut String) -> bool {
    if let Some(rest) = lower.strip_prefix("library ") {
        view.libraries.extend(
            rest.trim_end_matches(';')
                .split(',')
                .map(|name| name.trim().to_owned()),
        );
        return true;
    }
    if let Some(rest) = lower.strip_prefix("use ") {
        view.uses.push(rest.trim_end_matches(';').trim().to_owned());
        return true;
    }
    if let Some(rest) = lower.strip_prefix("entity ")
        && lower.contains(" is")
    {
        let name = rest.split_whitespace().next().unwrap_or("");
        let at = line.len() - rest.len();
        view.entities.push(Entity {
            name: line[at..at + name.len()].to_owned(),
            generics: Vec::new(),
            ports: Vec::new(),
        });
        return true;
    }
    if let Some(rest) = lower.strip_prefix("architecture ") {
        let mut words = rest.split_whitespace();
        let name = words.next().unwrap_or("");
        let of = words.nth(1).unwrap_or("").trim_end_matches(" is");
        name.clone_into(architecture);
        view.architectures.push(format!("{name} of {of}"));
        return true;
    }
    false
}

/// Everything [`VhdlView`] holds, read from `text`.
fn parse(text: &str) -> VhdlView {
    let mut view = VhdlView {
        libraries: Vec::new(),
        uses: Vec::new(),
        entities: Vec::new(),
        architectures: Vec::new(),
        processes: Vec::new(),
        signals: Vec::new(),
        instances: Vec::new(),
        processes_that_run_once: Vec::new(),
        truncated: false,
    };
    // Which declaration the walk is inside, and which list a port line
    // belongs on.
    let mut architecture = String::new();
    let mut in_generics = false;
    let mut in_ports = false;
    // The process being read, and whether its body has a `wait`.
    let mut process: Option<(usize, bool)> = None;

    for raw in without_comments(text).lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();

        if read_header(line, &lower, &mut view, &mut architecture) {
            in_generics = false;
            in_ports = false;
            continue;
        }
        if lower.starts_with("generic (") || lower == "generic(" {
            in_generics = true;
            in_ports = false;
            continue;
        }
        if lower.starts_with("port (") || lower == "port(" {
            in_ports = true;
            in_generics = false;
            continue;
        }
        if lower.starts_with(");") {
            in_generics = false;
            in_ports = false;
            continue;
        }
        if let Some(rest) = lower.strip_prefix("signal ") {
            if let Some(name) = rest.split([':', ',']).next() {
                view.signals.push(name.trim().to_owned());
            }
            continue;
        }

        if let Some((at, has_wait)) = process.as_mut() {
            if lower.starts_with("wait ") || lower == "wait;" {
                *has_wait = true;
            }
            if lower.starts_with("end process") {
                let read = &view.processes[*at];
                if read.sensitivity.is_empty() && !*has_wait {
                    view.processes_that_run_once.push(format!(
                        "{} in {}",
                        read.label
                            .clone()
                            .unwrap_or_else(|| "(unlabelled)".to_owned()),
                        read.architecture
                    ));
                }
                process = None;
            }
            continue;
        }
        if lower.contains("process") && !lower.starts_with("end ") {
            view.processes.push(Process {
                label: label_of(line),
                sensitivity: sensitivity_of(line),
                architecture: architecture.clone(),
            });
            process = Some((view.processes.len() - 1, false));
            continue;
        }

        if in_generics || in_ports {
            if let Some(port) = port_of(line)
                && let Some(entity) = view.entities.last_mut()
            {
                if in_generics {
                    entity.generics.push(port);
                } else {
                    entity.ports.push(port);
                }
            }
            continue;
        }
        // `u1 : counter port map (...)` or `u1 : entity work.counter`.
        if (lower.contains("port map") || lower.contains(": entity "))
            && let Some(label) = label_of(line)
        {
            let component = line
                .split_once(':')
                .map(|(_, rest)| rest.trim())
                .and_then(|rest| rest.split_whitespace().next())
                .unwrap_or("")
                .to_owned();
            view.instances.push(format!("{label}: {component}"));
        }
    }
    view
}

/// Whether `text` is VHDL.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    // `entity` and `architecture` together are how VHDL opens, and no
    // other format in this registry writes both.
    (!view.entities.is_empty() && !view.architectures.is_empty())
        || (!view.entities.is_empty() && !view.libraries.is_empty())
}

/// The VHDL plugin's core half.
#[derive(Debug, Default)]
pub struct VhdlCore;

impl PluginCore for VhdlCore {
    fn name(&self) -> &'static str {
        "vhdl"
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
        // The entities, their ports and the processes are what a reader
        // came for, and each is on the view already.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The VHDL plugin's presentation half.
#[derive(Debug, Default)]
pub struct VhdlPresentation;

impl PluginPresentation for VhdlPresentation {
    fn name(&self) -> &'static str {
        "vhdl"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "VHD",
            tint: 0x0000_5c9e,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: VhdlView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.libraries.is_empty() {
            lines.push(format!("Libraries: {}", view.libraries.join(", ")));
        }
        if !view.uses.is_empty() {
            lines.push(format!("Uses: {}", view.uses.join(", ")));
        }
        lines.push(format!("{} entity(ies):", view.entities.len()));
        for entity in &view.entities {
            lines.push(format!("  {}", entity.name));
            for generic in &entity.generics {
                lines.push(format!("      generic {} : {}", generic.name, generic.kind));
            }
            for port in &entity.ports {
                lines.push(format!(
                    "      {} : {} {}",
                    port.name,
                    port.mode.as_deref().unwrap_or("(no mode)"),
                    port.kind
                ));
            }
        }
        if !view.architectures.is_empty() {
            lines.push(format!("Architectures: {}", view.architectures.join(", ")));
        }
        if !view.signals.is_empty() {
            lines.push(format!("Signals: {}", view.signals.join(", ")));
        }
        if !view.processes.is_empty() {
            lines.push(format!("{} process(es):", view.processes.len()));
            for process in &view.processes {
                let sensitivity = if process.sensitivity.is_empty() {
                    "no sensitivity list".to_owned()
                } else {
                    process.sensitivity.join(", ")
                };
                lines.push(format!(
                    "  {} in {} - {sensitivity}",
                    process.label.as_deref().unwrap_or("(unlabelled)"),
                    process.architecture
                ));
            }
        }
        if !view.instances.is_empty() {
            lines.push(format!("Instances: {}", view.instances.join(", ")));
        }
        if !view.processes_that_run_once.is_empty() {
            lines.push("No sensitivity list and no wait, so these run once when".to_owned());
            lines.push("simulation starts and then never again:".to_owned());
            for process in &view.processes_that_run_once {
                lines.push(format!("  {process}"));
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
    use super::{VhdlCore, VhdlPresentation, VhdlView, parse, port_of};
    use plugin_api::{PluginCore, PluginPresentation};

    const SOURCE: &str = concat!(
        "library ieee;\n",
        "use ieee.std_logic_1164.all;\n",
        "use ieee.numeric_std.all;\n",
        "\n",
        "entity counter is\n",
        "    generic (\n",
        "        width : positive := 8\n",
        "    );\n",
        "    port (\n",
        "        clk     : in  std_logic;\n",
        "        rst_n   : in  std_logic;\n",
        "        enable  : in  std_logic;\n",
        "        value   : out unsigned(width - 1 downto 0)\n",
        "    );\n",
        "end entity counter;\n",
        "\n",
        "architecture rtl of counter is\n",
        "    signal count : unsigned(width - 1 downto 0);\n",
        "begin\n",
        "\n",
        "    clocked : process (clk, rst_n)\n",
        "    begin\n",
        "        if rst_n = '0' then\n",
        "            count <= (others => '0');\n",
        "        elsif rising_edge(clk) then\n",
        "            if enable = '1' then\n",
        "                count <= count + 1;\n",
        "            end if;\n",
        "        end if;\n",
        "    end process clocked;\n",
        "\n",
        "    stuck : process\n",
        "    begin\n",
        "        count <= (others => '0');\n",
        "    end process stuck;\n",
        "\n",
        "    value <= count;\n",
        "\n",
        "end architecture rtl;\n",
    );

    #[test]
    fn sniffs_a_design() {
        assert!(VhdlCore.sniff(SOURCE.as_bytes()));
    }

    #[test]
    fn does_not_claim_prose_that_uses_the_word_entity() {
        assert!(!VhdlCore.sniff(b"The entity is a legal person under the act.\n"));
        assert!(!VhdlCore.sniff(b""));
    }

    #[test]
    fn a_generic_has_no_mode_and_a_port_does() {
        let generic = port_of("width : positive := 8").unwrap();
        assert_eq!(generic.mode, None);
        assert_eq!(generic.kind, "positive");

        let port = port_of("value   : out unsigned(width - 1 downto 0)").unwrap();
        assert_eq!(port.mode.as_deref(), Some("out"));
        assert_eq!(port.kind, "unsigned(width - 1 downto 0)");
    }

    #[test]
    fn reads_the_libraries_and_the_packages() {
        let view = parse(SOURCE);

        assert_eq!(view.libraries, vec!["ieee".to_owned()]);
        assert_eq!(view.uses.len(), 2);
    }

    #[test]
    fn separates_the_generics_from_the_ports() {
        let view = parse(SOURCE);

        assert_eq!(view.entities.len(), 1);
        let counter = &view.entities[0];
        assert_eq!(counter.name, "counter");
        assert_eq!(counter.generics.len(), 1);
        assert_eq!(counter.ports.len(), 4);
    }

    #[test]
    fn reads_the_architecture_and_its_signal() {
        let view = parse(SOURCE);

        assert_eq!(view.architectures, vec!["rtl of counter".to_owned()]);
        assert_eq!(view.signals, vec!["count".to_owned()]);
    }

    #[test]
    fn reads_each_process_with_its_sensitivity_list() {
        let view = parse(SOURCE);

        assert_eq!(view.processes.len(), 2);
        assert_eq!(view.processes[0].label.as_deref(), Some("clocked"));
        assert_eq!(
            view.processes[0].sensitivity,
            vec!["clk".to_owned(), "rst_n".to_owned()]
        );
        assert_eq!(view.processes[0].architecture, "rtl");
        assert!(view.processes[1].sensitivity.is_empty());
    }

    #[test]
    fn names_the_process_that_runs_once() {
        let view = parse(SOURCE);

        assert_eq!(
            view.processes_that_run_once,
            vec!["stuck in rtl".to_owned()],
            "`clocked` has a sensitivity list, so it is not reported"
        );
    }

    #[test]
    fn a_wait_is_a_sensitivity_list_by_another_name() {
        let view = parse(concat!(
            "entity e is\nend entity e;\n",
            "architecture a of e is\nbegin\n",
            "    ticking : process\n",
            "    begin\n",
            "        wait for 10 ns;\n",
            "    end process ticking;\n",
            "end architecture a;\n",
        ));

        assert!(
            view.processes_that_run_once.is_empty(),
            "a process with a wait suspends and resumes; it does not run once"
        );
    }

    #[test]
    fn presents_the_stuck_process_with_its_reason() {
        let data = serde_json::to_value(parse(SOURCE)).unwrap();

        let lines = VhdlPresentation.present(&data);

        assert!(lines.iter().any(|line| line.contains("then never again")));
        assert!(lines.iter().any(|line| line.contains("Libraries: ieee")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/vhdl/rtl/counter.vhd");

        let data = VhdlCore.view(&path).unwrap();
        let view: VhdlView = serde_json::from_value(data).unwrap();

        assert!(!view.libraries.is_empty());
        assert!(view.uses.len() >= 2);
        assert!(view.entities.len() >= 2);
        assert!(view.entities.iter().any(|e| !e.generics.is_empty()));
        assert!(view.entities.iter().any(|e| e.ports.len() >= 4));
        assert!(view.architectures.len() >= 2);
        assert!(view.processes.len() >= 3);
        assert!(!view.signals.is_empty());
        assert!(!view.instances.is_empty());
        assert!(!view.processes_that_run_once.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::VhdlCore),
            plugin_api::PluginPresentation::extensions(&crate::VhdlPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
