//! SSH client configuration file type plugin: core and presentation halves.
//!
//! An SSH client configuration says which machine a short name really
//! means, as whom, with which key, and through what. This reads the host
//! blocks and what each sets, the identity files, the proxy jumps, the
//! port forwards, the included files - and the places where a check has
//! been turned off, with what each one stops catching.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One `Host` block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostBlock {
    /// The patterns the block matches.
    pub patterns: Vec<String>,
    /// The machine it actually connects to.
    pub hostname: Option<String>,
    /// The account it connects as.
    pub user: Option<String>,
    /// The port, when it is not the usual one.
    pub port: Option<String>,
    /// The keys it offers.
    pub identity_files: Vec<String>,
    /// The machine it goes through to get there.
    pub proxy_jump: Option<String>,
    /// The port forwards it sets up.
    pub forwards: Vec<String>,
}

/// View data produced by [`SshconfigCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshconfigView {
    /// Every host block, in the order they are written - which is the
    /// order they are applied in, first match winning per keyword.
    pub hosts: Vec<HostBlock>,
    /// The files pulled in with `Include`.
    pub includes: Vec<String>,
    /// Every key file the configuration names.
    pub identity_files: Vec<String>,
    /// Every machine used as a jump host.
    pub proxy_jumps: Vec<String>,
    /// Every port forward.
    pub forwards: Vec<String>,
    /// Settings that turn off a check, and what each one stops catching.
    pub checks_turned_off: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Keywords that switch a protection off, and what stops being caught.
const WEAKENING: &[(&str, &str, &str)] = &[
    (
        "stricthostkeychecking",
        "no",
        "a machine whose key has changed is accepted without asking",
    ),
    (
        "userknownhostsfile",
        "/dev/null",
        "no host key is ever remembered, so no change in one can be noticed",
    ),
    (
        "checkhostip",
        "no",
        "a key served from a different address is not questioned",
    ),
    (
        "forwardagent",
        "yes",
        "the far machine can use this agent's keys while the connection is open",
    ),
];

/// `line` with its comment stripped.
fn cleaned(line: &str) -> &str {
    let line = line.trim();
    if line.starts_with('#') { "" } else { line }
}

/// The keyword and argument of a configuration line.
///
/// The separator is whitespace, optionally with an `=`, and the keyword is
/// matched without regard to case - all three are how ssh reads it.
fn keyword_of(line: &str) -> Option<(String, String)> {
    let line = line.trim_start();
    let at = line.find([' ', '\t', '='])?;
    let keyword = line[..at].to_ascii_lowercase();
    let argument = line[at..].trim_start_matches(['=', ' ', '\t']).trim();
    if keyword.is_empty() || argument.is_empty() {
        return None;
    }
    Some((keyword, argument.to_owned()))
}

/// Applies one keyword inside a host block.
fn read_host(host: &mut HostBlock, keyword: &str, argument: &str) {
    match keyword {
        "hostname" => host.hostname = Some(argument.to_owned()),
        "user" => host.user = Some(argument.to_owned()),
        "port" => host.port = Some(argument.to_owned()),
        "identityfile" => host.identity_files.push(argument.to_owned()),
        "proxyjump" => host.proxy_jump = Some(argument.to_owned()),
        "localforward" => host.forwards.push(format!("local {argument}")),
        "remoteforward" => host.forwards.push(format!("remote {argument}")),
        "dynamicforward" => host.forwards.push(format!("dynamic {argument}")),
        _ => {}
    }
}

/// Everything [`SshconfigView`] holds, read from `text`.
fn parse(text: &str) -> SshconfigView {
    let mut view = SshconfigView {
        hosts: Vec::new(),
        includes: Vec::new(),
        identity_files: Vec::new(),
        proxy_jumps: Vec::new(),
        forwards: Vec::new(),
        checks_turned_off: Vec::new(),
        truncated: false,
    };
    for raw in text.lines() {
        let line = cleaned(raw);
        if line.is_empty() {
            continue;
        }
        let Some((keyword, argument)) = keyword_of(line) else {
            continue;
        };
        match keyword.as_str() {
            "host" => {
                view.hosts.push(HostBlock {
                    patterns: argument.split_whitespace().map(str::to_owned).collect(),
                    hostname: None,
                    user: None,
                    port: None,
                    identity_files: Vec::new(),
                    proxy_jump: None,
                    forwards: Vec::new(),
                });
                continue;
            }
            "include" => {
                view.includes.push(argument.clone());
                continue;
            }
            _ => {}
        }

        let where_it_is = view.hosts.last().map_or_else(
            || "before any Host".to_owned(),
            |host| host.patterns.join(" "),
        );
        if let Some((_, _, cost)) = WEAKENING
            .iter()
            .find(|(name, value, _)| *name == keyword && argument.eq_ignore_ascii_case(value))
        {
            view.checks_turned_off
                .push(format!("{where_it_is}: {cost}"));
        }

        if let Some(host) = view.hosts.last_mut() {
            read_host(host, &keyword, &argument);
        }
    }

    for host in &view.hosts {
        view.identity_files.extend(host.identity_files.clone());
        view.forwards.extend(host.forwards.clone());
        if let Some(jump) = &host.proxy_jump {
            view.proxy_jumps.push(jump.clone());
        }
    }
    view.identity_files.dedup();
    view
}

/// Whether `text` is an SSH client configuration.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    if view.hosts.is_empty() {
        return false;
    }
    // A `Host` line alone is not enough - plenty of formats have one. A
    // block has to set something ssh would recognise.
    view.hosts.iter().any(|host| {
        host.hostname.is_some()
            || host.user.is_some()
            || !host.identity_files.is_empty()
            || host.proxy_jump.is_some()
            || !host.forwards.is_empty()
    })
}

/// The SSH client configuration plugin's core half.
#[derive(Debug, Default)]
pub struct SshconfigCore;

impl PluginCore for SshconfigCore {
    fn name(&self) -> &'static str {
        "sshconfig"
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
        // The host blocks are the whole of the file, and every keyword
        // worth reading is already on the view.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The SSH client configuration plugin's presentation half.
#[derive(Debug, Default)]
pub struct SshconfigPresentation;

impl PluginPresentation for SshconfigPresentation {
    fn name(&self) -> &'static str {
        "sshconfig"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "SSH",
            tint: 0x0033_4b6e,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: SshconfigView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("{} host block(s):", view.hosts.len()));
        for host in &view.hosts {
            lines.push(format!("  {}", host.patterns.join(" ")));
            if let Some(hostname) = &host.hostname {
                let account = host
                    .user
                    .as_ref()
                    .map_or_else(String::new, |user| format!("{user}@"));
                let port = host
                    .port
                    .as_ref()
                    .map_or_else(String::new, |port| format!(" port {port}"));
                lines.push(format!("      connects to {account}{hostname}{port}"));
            }
            if let Some(jump) = &host.proxy_jump {
                lines.push(format!("      through {jump}"));
            }
            for key in &host.identity_files {
                lines.push(format!("      key {key}"));
            }
            for forward in &host.forwards {
                lines.push(format!("      forwards {forward}"));
            }
        }
        if !view.includes.is_empty() {
            lines.push(format!("Includes: {}", view.includes.join(", ")));
        }
        if !view.checks_turned_off.is_empty() {
            lines.push("Turned off here, so that while these hosts are used:".to_owned());
            for entry in &view.checks_turned_off {
                lines.push(format!("  {entry}"));
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
    use super::{SshconfigCore, SshconfigPresentation, SshconfigView, keyword_of, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const CONFIG: &str = concat!(
        "Include ~/.ssh/config.d/*.conf\n",
        "\n",
        "Host *\n",
        "    ServerAliveInterval 60\n",
        "    HashKnownHosts yes\n",
        "\n",
        "Host build\n",
        "    HostName build.example.com\n",
        "    User floor\n",
        "    Port 2222\n",
        "    IdentityFile ~/.ssh/id_ed25519_build\n",
        "    LocalForward 9090 127.0.0.1:9090\n",
        "\n",
        "Host db\n",
        "    HostName db.internal\n",
        "    ProxyJump build\n",
        "    IdentityFile ~/.ssh/id_ed25519_build\n",
        "\n",
        "Host scratch\n",
        "    HostName 10.0.0.9\n",
        "    StrictHostKeyChecking no\n",
        "    UserKnownHostsFile /dev/null\n",
    );

    #[test]
    fn sniffs_a_configuration() {
        assert!(SshconfigCore.sniff(CONFIG.as_bytes()));
    }

    #[test]
    fn does_not_claim_a_document_that_merely_has_a_host_line() {
        assert!(!SshconfigCore.sniff(b"Host: example.com\nAccept: */*\n"));
        assert!(!SshconfigCore.sniff(b""));
    }

    #[test]
    fn a_keyword_may_be_separated_by_an_equals_and_is_case_insensitive() {
        assert_eq!(
            keyword_of("HostName=build.example.com"),
            Some(("hostname".to_owned(), "build.example.com".to_owned()))
        );
        assert_eq!(
            keyword_of("   IDENTITYFILE   ~/.ssh/id"),
            Some(("identityfile".to_owned(), "~/.ssh/id".to_owned()))
        );
        assert_eq!(keyword_of("Host"), None);
    }

    #[test]
    fn reads_each_block_and_what_it_connects_to() {
        let view = parse(CONFIG);

        assert_eq!(view.hosts.len(), 4);
        assert_eq!(view.hosts[0].patterns, vec!["*".to_owned()]);
        assert_eq!(view.hosts[1].hostname.as_deref(), Some("build.example.com"));
        assert_eq!(view.hosts[1].user.as_deref(), Some("floor"));
        assert_eq!(view.hosts[1].port.as_deref(), Some("2222"));
        assert_eq!(view.includes, vec!["~/.ssh/config.d/*.conf".to_owned()]);
    }

    #[test]
    fn reads_the_jumps_the_keys_and_the_forwards() {
        let view = parse(CONFIG);

        assert_eq!(view.proxy_jumps, vec!["build".to_owned()]);
        assert_eq!(view.forwards, vec!["local 9090 127.0.0.1:9090".to_owned()]);
        assert_eq!(
            view.identity_files,
            vec!["~/.ssh/id_ed25519_build".to_owned()],
            "the same key named twice is one key"
        );
    }

    #[test]
    fn names_the_checks_turned_off_and_what_each_stops_catching() {
        let view = parse(CONFIG);

        assert_eq!(view.checks_turned_off.len(), 2);
        assert!(
            view.checks_turned_off
                .iter()
                .all(|entry| entry.starts_with("scratch:")),
            "a setting belongs to the block it is written in"
        );
        assert!(
            view.checks_turned_off
                .iter()
                .any(|entry| entry.contains("key has changed"))
        );
    }

    #[test]
    fn a_protection_left_on_is_not_reported_as_off() {
        let view = parse("Host a\n    HostName b\n    StrictHostKeyChecking yes\n");

        assert!(view.checks_turned_off.is_empty());
    }

    #[test]
    fn presents_the_weakened_checks_with_their_cost() {
        let data = serde_json::to_value(parse(CONFIG)).unwrap();

        let lines = SshconfigPresentation.present(&data);

        assert_eq!(lines[0], "4 host block(s):");
        assert!(lines.iter().any(|line| line.contains("without asking")));
        assert!(lines.iter().any(|line| line.contains("through build")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/sshconfig/config");

        let data = SshconfigCore.view(&path).unwrap();
        let view: SshconfigView = serde_json::from_value(data).unwrap();

        assert!(view.hosts.len() >= 4);
        assert!(!view.includes.is_empty());
        assert!(!view.identity_files.is_empty());
        assert!(!view.proxy_jumps.is_empty());
        assert!(view.forwards.len() >= 2);
        assert!(!view.checks_turned_off.is_empty());
        assert!(view.hosts.iter().any(|host| host.port.is_some()));
        assert!(view.hosts.iter().any(|host| host.user.is_some()));
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::SshconfigCore),
            plugin_api::PluginPresentation::extensions(&crate::SshconfigPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
