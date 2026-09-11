//! systemd unit file type plugin: core and presentation halves.
//!
//! A unit file says what systemd runs, when, and with how much of the
//! machine available to it. This reads the description, the units it is
//! ordered against, the service type, the commands, the restart policy,
//! the hardening it asks for - and the hardening it does not, with what
//! each of those would have shut off.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &[
    "service",
    "socket",
    "timer",
    "target",
    "mount",
    "automount",
    "swap",
    "path",
    "slice",
    "scope",
];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`SystemdunitCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemdunitView {
    /// What kind of unit this is, read from the section it carries:
    /// `service`, `socket`, `timer` and so on.
    pub kind: String,
    /// The one-line description.
    pub description: Option<String>,
    /// The addresses given as `Documentation`.
    pub documentation: Vec<String>,
    /// Units this one is ordered after.
    pub after: Vec<String>,
    /// Units it pulls in but can live without.
    pub wants: Vec<String>,
    /// Units it cannot start without.
    pub requires: Vec<String>,
    /// The `Type` of a service, which decides when systemd calls it up.
    pub service_type: Option<String>,
    /// Every `Exec*` line, with the key that introduced it.
    pub commands: Vec<String>,
    /// The restart policy, and what it means in practice.
    pub restart: Option<String>,
    /// The hardening directives the unit does set.
    pub hardening: Vec<String>,
    /// What pulls this unit in, from the `[Install]` section.
    pub install_targets: Vec<String>,
    /// The well-known hardening directives this unit does not set, each
    /// with what it would have shut off.
    pub hardening_not_set: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The section names that mark a file out as a systemd unit, lowercased.
const UNIT_SECTIONS: &[&str] = &[
    "unit",
    "install",
    "service",
    "socket",
    "timer",
    "mount",
    "automount",
    "swap",
    "path",
    "slice",
    "scope",
];

/// The sections that say what kind of unit this is - `[Unit]` and
/// `[Install]` appear in all of them and so say nothing.
const KIND_SECTIONS: &[&str] = &[
    "service",
    "socket",
    "timer",
    "mount",
    "automount",
    "swap",
    "path",
    "slice",
    "scope",
];

/// The hardening directives worth naming, and what each one shuts off.
const HARDENING: &[(&str, &str)] = &[
    (
        "NoNewPrivileges",
        "a child process can still gain privileges this one has not got",
    ),
    (
        "ProtectSystem",
        "the whole filesystem is writable, not just what the unit needs",
    ),
    (
        "ProtectHome",
        "the unit can read every user's home directory",
    ),
    (
        "PrivateTmp",
        "the unit shares /tmp with everything else on the machine",
    ),
    (
        "PrivateDevices",
        "the unit can reach the machine's raw devices",
    ),
    (
        "RestrictAddressFamilies",
        "the unit can open any kind of socket, raw ones included",
    ),
];

/// Keys whose value is a space-separated list of unit names.
fn unit_list(value: &str) -> Vec<String> {
    value.split_whitespace().map(str::to_owned).collect()
}

/// `text` with systemd's backslash continuations joined.
///
/// A long `ExecStart` is routinely written over several lines. Read line
/// by line, the tail of one becomes a directive of its own with no `=` in
/// it, and the command itself is reported truncated.
fn logical_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut pending = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(head) = line.strip_suffix('\\') {
            pending.push_str(head.trim_end());
            pending.push(' ');
            continue;
        }
        pending.push_str(line);
        if !pending.trim().is_empty() {
            lines.push(pending.trim().to_owned());
        }
        pending.clear();
    }
    if !pending.trim().is_empty() {
        lines.push(pending.trim().to_owned());
    }
    lines
}

/// What `Restart=` means, said plainly.
fn restart_meaning(value: &str) -> String {
    let plainly = match value {
        "no" => "so it never comes back on its own",
        "always" => "so systemd brings it back however it stopped",
        "on-failure" => "so systemd brings it back when it fails, but not when it exits cleanly",
        "on-success" => "so systemd brings it back only when it exits cleanly",
        "on-abnormal" => "so systemd brings it back on a signal or a timeout",
        "on-abort" => "so systemd brings it back only on an uncaught signal",
        "on-watchdog" => "so systemd brings it back only when the watchdog fires",
        _ => "",
    };
    if plainly.is_empty() {
        value.to_owned()
    } else {
        format!("{value}, {plainly}")
    }
}

/// Everything [`SystemdunitView`] holds, read from `text`.
fn parse(text: &str) -> SystemdunitView {
    let mut view = SystemdunitView {
        kind: "unit".to_owned(),
        description: None,
        documentation: Vec::new(),
        after: Vec::new(),
        wants: Vec::new(),
        requires: Vec::new(),
        service_type: None,
        commands: Vec::new(),
        restart: None,
        hardening: Vec::new(),
        install_targets: Vec::new(),
        hardening_not_set: Vec::new(),
        truncated: false,
    };
    let mut section = String::new();
    let mut set: Vec<String> = Vec::new();

    for line in logical_lines(text) {
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            section = name.to_ascii_lowercase();
            if KIND_SECTIONS.contains(&section.as_str()) {
                section.clone_into(&mut view.kind);
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        set.push(key.to_owned());
        match key {
            "Description" => view.description = Some(value.to_owned()),
            "Documentation" => view.documentation.extend(unit_list(value)),
            "After" => view.after.extend(unit_list(value)),
            "Wants" => view.wants.extend(unit_list(value)),
            "Requires" | "BindsTo" => view.requires.extend(unit_list(value)),
            "Type" => view.service_type = Some(value.to_owned()),
            "Restart" => view.restart = Some(restart_meaning(value)),
            "WantedBy" | "RequiredBy" => view.install_targets.extend(unit_list(value)),
            _ if key.starts_with("Exec") => view.commands.push(format!("{key}={value}")),
            _ if HARDENING.iter().any(|(name, _)| *name == key) => {
                view.hardening.push(format!("{key}={value}"));
            }
            _ => {}
        }
    }

    // Only a unit that runs a process can be hardened; a timer or a target
    // has nothing to shut off.
    if view.kind == "service" {
        view.hardening_not_set = HARDENING
            .iter()
            .filter(|(name, _)| !set.iter().any(|seen| seen == name))
            .map(|(name, cost)| format!("{name}: {cost}"))
            .collect();
    }
    let _ = section;
    view
}

/// Whether `text` is a systemd unit.
fn looks_like_it(text: &str) -> bool {
    let mut sections = 0;
    let mut directives = 0;
    for line in logical_lines(text) {
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            if !UNIT_SECTIONS.contains(&name.to_ascii_lowercase().as_str()) {
                // A section systemd does not have: this is some other
                // file in the same dialect.
                return false;
            }
            sections += 1;
        } else if line.contains('=') {
            directives += 1;
        } else if !line.is_empty() {
            return false;
        }
    }
    sections > 0 && directives > 0
}

/// The systemd unit plugin's core half.
#[derive(Debug, Default)]
pub struct SystemdunitCore;

impl PluginCore for SystemdunitCore {
    fn name(&self) -> &'static str {
        "systemdunit"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A unit is written in the INI dialect, and `ini` recognises it.
        // Saying so settles the file on the narrower reading whatever the
        // extension happens to be (D13).
        &["ini"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        // A unit file is short and every directive worth reading is
        // already on the view.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The systemd unit plugin's presentation half.
#[derive(Debug, Default)]
pub struct SystemdunitPresentation;

impl PluginPresentation for SystemdunitPresentation {
    fn name(&self) -> &'static str {
        "systemdunit"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "SYS",
            tint: 0x0030_a2e8,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: SystemdunitView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "systemd {} unit{}",
            view.kind,
            view.description
                .as_ref()
                .map_or_else(String::new, |said| format!(": {said}"))
        ));
        if let Some(kind) = &view.service_type {
            lines.push(format!("Type: {kind}"));
        }
        if let Some(restart) = &view.restart {
            lines.push(format!("Restart: {restart}"));
        }
        for relation in [
            ("Requires", &view.requires),
            ("Wants", &view.wants),
            ("After", &view.after),
        ] {
            if !relation.1.is_empty() {
                lines.push(format!("{}: {}", relation.0, relation.1.join(", ")));
            }
        }
        if !view.commands.is_empty() {
            lines.push(format!("{} command(s):", view.commands.len()));
            for command in &view.commands {
                lines.push(format!("  {command}"));
            }
        }
        if !view.install_targets.is_empty() {
            lines.push(format!("Pulled in by: {}", view.install_targets.join(", ")));
        }
        if !view.documentation.is_empty() {
            lines.push(format!("Documentation: {}", view.documentation.join(", ")));
        }
        if !view.hardening.is_empty() {
            lines.push(format!("Hardened with: {}", view.hardening.join(", ")));
        }
        if !view.hardening_not_set.is_empty() {
            lines.push("Not hardened, so while this service runs:".to_owned());
            for missing in &view.hardening_not_set {
                lines.push(format!("  {missing}"));
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
    use super::{SystemdunitCore, SystemdunitPresentation, SystemdunitView, logical_lines, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const UNIT: &str = concat!(
        "[Unit]\n",
        "Description=Repos Explorer service\n",
        "Documentation=https://example.com/docs man:explorer(8)\n",
        "After=network-online.target\n",
        "Wants=network-online.target\n",
        "Requires=explorer.socket\n",
        "\n",
        "[Service]\n",
        "Type=notify\n",
        "ExecStart=/usr/bin/explorer \\\n",
        "    --config /etc/explorer.toml \\\n",
        "    --verbose\n",
        "ExecReload=/bin/kill -HUP $MAINPID\n",
        "Restart=on-failure\n",
        "NoNewPrivileges=true\n",
        "PrivateTmp=true\n",
        "\n",
        "[Install]\n",
        "WantedBy=multi-user.target\n",
    );

    #[test]
    fn sniffs_a_unit() {
        assert!(SystemdunitCore.sniff(UNIT.as_bytes()));
    }

    #[test]
    fn does_not_claim_an_ini_file_with_sections_systemd_has_not_got() {
        assert!(!SystemdunitCore.sniff(b"[database]\nhost=localhost\nport=5432\n"));
        assert!(!SystemdunitCore.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_ini() {
        assert_eq!(SystemdunitCore.specialises(), &["ini"]);
    }

    #[test]
    fn a_continued_command_is_one_command() {
        let joined = logical_lines(UNIT);

        assert!(
            joined
                .iter()
                .any(|line| line.starts_with("ExecStart=") && line.contains("--verbose")),
            "the continuation lines belong to ExecStart, not to directives of their own"
        );
        assert!(
            !joined.iter().any(|line| line.starts_with("--config")),
            "a continuation is not a directive"
        );
    }

    #[test]
    fn reads_the_kind_from_the_section_not_the_name() {
        let view = parse(UNIT);

        assert_eq!(view.kind, "service");
        assert_eq!(
            parse("[Unit]\nDescription=x\n[Timer]\nOnCalendar=daily\n").kind,
            "timer",
            "`[Unit]` and `[Install]` appear in every unit and so say nothing"
        );
    }

    #[test]
    fn reads_the_relations_and_the_install_target() {
        let view = parse(UNIT);

        assert_eq!(view.description.as_deref(), Some("Repos Explorer service"));
        assert_eq!(view.documentation.len(), 2);
        assert_eq!(view.after, vec!["network-online.target".to_owned()]);
        assert_eq!(view.requires, vec!["explorer.socket".to_owned()]);
        assert_eq!(view.install_targets, vec!["multi-user.target".to_owned()]);
        assert_eq!(view.commands.len(), 2);
    }

    #[test]
    fn says_what_the_restart_policy_means() {
        assert!(
            parse(UNIT)
                .restart
                .is_some_and(|said| said.contains("not when it exits cleanly"))
        );
        assert!(
            parse("[Service]\nRestart=no\n")
                .restart
                .is_some_and(|said| said.contains("never comes back"))
        );
    }

    #[test]
    fn names_the_hardening_that_is_missing_and_what_it_costs() {
        let view = parse(UNIT);

        assert_eq!(view.hardening.len(), 2);
        assert!(
            view.hardening_not_set
                .iter()
                .any(|missing| missing.starts_with("ProtectSystem")),
            "the unit sets NoNewPrivileges and PrivateTmp, and not this one"
        );
        assert!(
            !view
                .hardening_not_set
                .iter()
                .any(|missing| missing.starts_with("PrivateTmp"))
        );
    }

    #[test]
    fn a_timer_is_not_asked_why_it_is_not_hardened() {
        let view = parse("[Unit]\nDescription=x\n[Timer]\nOnCalendar=daily\n");

        assert!(
            view.hardening_not_set.is_empty(),
            "a timer runs no process, so it has nothing to shut off"
        );
    }

    #[test]
    fn presents_the_missing_hardening_with_its_cost() {
        let data = serde_json::to_value(parse(UNIT)).unwrap();

        let lines = SystemdunitPresentation.present(&data);

        assert_eq!(lines[0], "systemd service unit: Repos Explorer service");
        assert!(
            lines
                .iter()
                .any(|line| line.contains("while this service runs"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("every user's home directory"))
        );
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/systemdunit/explorer.service");

        let data = SystemdunitCore.view(&path).unwrap();
        let view: SystemdunitView = serde_json::from_value(data).unwrap();

        assert_eq!(view.kind, "service");
        assert!(view.description.is_some());
        assert!(!view.documentation.is_empty());
        assert!(!view.after.is_empty());
        assert!(!view.wants.is_empty());
        assert!(!view.requires.is_empty());
        assert!(view.service_type.is_some());
        assert!(view.commands.len() >= 2);
        assert!(view.restart.is_some());
        assert!(view.hardening.len() >= 3);
        assert!(!view.install_targets.is_empty());
        assert!(!view.hardening_not_set.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::SystemdunitCore),
            plugin_api::PluginPresentation::extensions(&crate::SystemdunitPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
