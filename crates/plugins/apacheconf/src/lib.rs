//! Apache configuration file type plugin: core and presentation halves.
//!
//! An Apache configuration is built from container directives. This
//! reads the virtual hosts with their names and what they serve, the
//! directory and location containers with their options, the modules
//! loaded, the rewrite rules, the certificate paths, and the containers
//! that serve a directory listing to anyone who asks.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
///
/// `.conf` is claimed by nothing and means nothing: nginx, systemd, and
/// a dozen other formats use it too. This is recognised by what is
/// inside it, the way the Caddyfile is.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One `<VirtualHost>` container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualHost {
    /// The address and port it was opened with.
    pub address: String,
    /// Its `ServerName`.
    pub server_name: Option<String>,
    /// Any `ServerAlias` names.
    pub aliases: Vec<String>,
    /// Its `DocumentRoot`.
    pub document_root: Option<String>,
    /// The certificate it presents, when it presents one.
    pub certificate: Option<String>,
    /// The log files it writes.
    pub logs: Vec<String>,
}

/// One `<Directory>` or `<Location>` container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Container {
    /// Whether it is a `Directory`, a `Location`, or a `Files`.
    pub kind: String,
    /// The path it governs.
    pub path: String,
    /// The words of its `Options` line.
    pub options: Vec<String>,
    /// Its `AllowOverride`, which says whether `.htaccess` is consulted.
    pub allow_override: Option<String>,
}

/// View data produced by [`ApacheconfCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApacheconfView {
    /// Every virtual host.
    pub virtual_hosts: Vec<VirtualHost>,
    /// Every directory, location or files container.
    pub containers: Vec<Container>,
    /// The modules the file loads, by name.
    pub modules: Vec<String>,
    /// The rewrite rules and conditions, in order.
    pub rewrites: Vec<String>,
    /// Every certificate or key path mentioned.
    pub certificates: Vec<String>,
    /// Containers that serve a directory listing when there is no index
    /// file, which shows a visitor everything in the directory.
    pub lists_directories: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The container tags worth recording on their own.
const CONTAINERS: &[&str] = &[
    "Directory",
    "Location",
    "Files",
    "DirectoryMatch",
    "LocationMatch",
];

/// `line` with its comment stripped.
fn cleaned(line: &str) -> &str {
    let line = line.trim();
    if line.starts_with('#') { "" } else { line }
}

/// The tag and arguments of a `<Tag args>` line, if it is one.
fn container_of(line: &str) -> Option<(String, String)> {
    let inner = line.strip_prefix('<')?.strip_suffix('>')?;
    if inner.starts_with('/') {
        return None;
    }
    let (tag, rest) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
    Some((tag.to_owned(), rest.trim().trim_matches('"').to_owned()))
}

/// The directive name and value of `line`, if it states one.
fn directive_of(line: &str) -> Option<(&str, &str)> {
    if line.starts_with('<') {
        return None;
    }
    let (name, value) = line.split_once(char::is_whitespace)?;
    Some((name, value.trim()))
}

/// Applies one directive inside a `<VirtualHost>`.
fn read_virtual_host(
    host: &mut VirtualHost,
    name: &str,
    value: &str,
    certificates: &mut Vec<String>,
) {
    match name {
        "ServerName" => host.server_name = Some(value.to_owned()),
        "ServerAlias" => host
            .aliases
            .extend(value.split_whitespace().map(str::to_owned)),
        "DocumentRoot" => host.document_root = Some(value.trim_matches('"').to_owned()),
        "SSLCertificateFile" => {
            let path = value.trim_matches('"').to_owned();
            host.certificate = Some(path.clone());
            certificates.push(path);
        }
        "SSLCertificateKeyFile" | "SSLCertificateChainFile" => {
            certificates.push(value.trim_matches('"').to_owned());
        }
        "ErrorLog" | "CustomLog" => {
            let path = value.split_whitespace().next().unwrap_or(value);
            host.logs.push(path.to_owned());
        }
        _ => {}
    }
}

/// Everything [`ApacheconfView`] holds, read from `text`.
fn parse(text: &str) -> ApacheconfView {
    let mut view = ApacheconfView {
        virtual_hosts: Vec::new(),
        containers: Vec::new(),
        modules: Vec::new(),
        rewrites: Vec::new(),
        certificates: Vec::new(),
        lists_directories: Vec::new(),
        truncated: false,
    };
    // The container tags currently open, outermost first.
    let mut stack: Vec<String> = Vec::new();

    for raw in text.lines() {
        let line = cleaned(raw);
        if line.is_empty() {
            continue;
        }
        if let Some(closed) = line
            .strip_prefix("</")
            .and_then(|rest| rest.strip_suffix('>'))
        {
            if stack.last().is_some_and(|open| open == closed.trim()) {
                stack.pop();
            }
            continue;
        }
        if let Some((tag, argument)) = container_of(line) {
            if tag == "VirtualHost" {
                view.virtual_hosts.push(VirtualHost {
                    address: argument,
                    server_name: None,
                    aliases: Vec::new(),
                    document_root: None,
                    certificate: None,
                    logs: Vec::new(),
                });
            } else if CONTAINERS.contains(&tag.as_str()) {
                view.containers.push(Container {
                    kind: tag.clone(),
                    path: argument,
                    options: Vec::new(),
                    allow_override: None,
                });
            }
            stack.push(tag);
            continue;
        }

        let Some((name, value)) = directive_of(line) else {
            continue;
        };
        match name {
            "LoadModule" => {
                if let Some(module) = value.split_whitespace().next() {
                    view.modules.push(module.to_owned());
                }
            }
            "RewriteRule" | "RewriteCond" => view.rewrites.push(format!("{name} {value}")),
            _ => {}
        }

        let innermost = stack.last().map_or("", String::as_str);
        if CONTAINERS.contains(&innermost)
            && let Some(container) = view.containers.last_mut()
        {
            match name {
                "Options" => container
                    .options
                    .extend(value.split_whitespace().map(str::to_owned)),
                "AllowOverride" => container.allow_override = Some(value.to_owned()),
                _ => {}
            }
            continue;
        }
        if innermost == "VirtualHost"
            && let Some(host) = view.virtual_hosts.last_mut()
        {
            read_virtual_host(host, name, value, &mut view.certificates);
        }
    }

    // `Options Indexes` - or `+Indexes` - serves the directory listing
    // when no index file is present.
    view.lists_directories = view
        .containers
        .iter()
        .filter(|container| {
            container
                .options
                .iter()
                .any(|option| option == "Indexes" || option == "+Indexes")
        })
        .map(|container| format!("{} {}", container.kind, container.path))
        .collect();
    view
}

/// Whether `text` is an Apache configuration.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    // Angle brackets alone are markup. It has to have opened one of
    // Apache's own containers, or loaded a module the way Apache does.
    !view.virtual_hosts.is_empty() || !view.containers.is_empty() || !view.modules.is_empty()
}

/// The Apache configuration plugin's core half.
#[derive(Debug, Default)]
pub struct ApacheconfCore;

impl PluginCore for ApacheconfCore {
    fn name(&self) -> &'static str {
        "apacheconf"
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
        // The containers are the whole of the structure, and every one
        // of them is on the view already.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Apache configuration plugin's presentation half.
#[derive(Debug, Default)]
pub struct ApacheconfPresentation;

impl PluginPresentation for ApacheconfPresentation {
    fn name(&self) -> &'static str {
        "apacheconf"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "APA",
            tint: 0x00d2_2128,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: ApacheconfView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("{} virtual host(s):", view.virtual_hosts.len()));
        for host in &view.virtual_hosts {
            lines.push(format!(
                "  {} - {}",
                host.address,
                host.server_name.as_deref().unwrap_or("(no ServerName)")
            ));
            if !host.aliases.is_empty() {
                lines.push(format!("      also {}", host.aliases.join(", ")));
            }
            if let Some(root) = &host.document_root {
                lines.push(format!("      serves {root}"));
            }
            if let Some(certificate) = &host.certificate {
                lines.push(format!("      certificate {certificate}"));
            }
            if !host.logs.is_empty() {
                lines.push(format!("      logs to {}", host.logs.join(", ")));
            }
        }
        if !view.containers.is_empty() {
            lines.push(format!("{} container(s):", view.containers.len()));
            for container in &view.containers {
                let options = if container.options.is_empty() {
                    "no Options".to_owned()
                } else {
                    container.options.join(" ")
                };
                let override_said = container
                    .allow_override
                    .as_ref()
                    .map_or_else(String::new, |said| format!(", AllowOverride {said}"));
                lines.push(format!(
                    "  {} {} - {options}{override_said}",
                    container.kind, container.path
                ));
            }
        }
        if !view.modules.is_empty() {
            lines.push(format!(
                "{} module(s): {}",
                view.modules.len(),
                view.modules.join(", ")
            ));
        }
        if !view.rewrites.is_empty() {
            lines.push(format!("{} rewrite line(s):", view.rewrites.len()));
            for rewrite in &view.rewrites {
                lines.push(format!("  {rewrite}"));
            }
        }
        if !view.certificates.is_empty() {
            lines.push(format!("Certificates: {}", view.certificates.join(", ")));
        }
        if !view.lists_directories.is_empty() {
            lines.push("Serves a directory listing when there is no index file,".to_owned());
            lines.push("so a visitor sees everything in the directory:".to_owned());
            for container in &view.lists_directories {
                lines.push(format!("  {container}"));
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
    use super::{ApacheconfCore, ApacheconfPresentation, ApacheconfView, container_of, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const CONFIG: &str = concat!(
        "LoadModule rewrite_module modules/mod_rewrite.so\n",
        "LoadModule ssl_module modules/mod_ssl.so\n",
        "\n",
        "<VirtualHost *:80>\n",
        "    ServerName explorer.example.com\n",
        "    ServerAlias www.explorer.example.com\n",
        "    RewriteEngine On\n",
        "    RewriteCond %{HTTPS} off\n",
        "    RewriteRule ^(.*)$ https://%{HTTP_HOST}$1 [R=301,L]\n",
        "</VirtualHost>\n",
        "\n",
        "<VirtualHost *:443>\n",
        "    ServerName explorer.example.com\n",
        "    DocumentRoot /srv/explorer\n",
        "    SSLCertificateFile /etc/ssl/certs/explorer.crt\n",
        "    SSLCertificateKeyFile /etc/ssl/private/explorer.key\n",
        "    ErrorLog /var/log/apache2/explorer.error.log\n",
        "    CustomLog /var/log/apache2/explorer.access.log combined\n",
        "\n",
        "    <Directory /srv/explorer>\n",
        "        Options FollowSymLinks\n",
        "        AllowOverride None\n",
        "    </Directory>\n",
        "\n",
        "    <Directory /srv/explorer/downloads>\n",
        "        Options +Indexes +FollowSymLinks\n",
        "        AllowOverride None\n",
        "    </Directory>\n",
        "</VirtualHost>\n",
    );

    #[test]
    fn sniffs_a_configuration() {
        assert!(ApacheconfCore.sniff(CONFIG.as_bytes()));
    }

    #[test]
    fn does_not_claim_markup_that_merely_has_tags() {
        assert!(!ApacheconfCore.sniff(b"<html>\n<body>Hello</body>\n</html>\n"));
        assert!(!ApacheconfCore.sniff(b""));
    }

    #[test]
    fn a_closing_tag_is_not_a_container() {
        assert_eq!(
            container_of("<Directory /srv>"),
            Some(("Directory".to_owned(), "/srv".to_owned()))
        );
        assert_eq!(container_of("</Directory>"), None);
        assert_eq!(
            container_of("<VirtualHost *:443>"),
            Some(("VirtualHost".to_owned(), "*:443".to_owned()))
        );
    }

    #[test]
    fn reads_each_virtual_host_and_what_it_serves() {
        let view = parse(CONFIG);

        assert_eq!(view.virtual_hosts.len(), 2);
        assert_eq!(view.virtual_hosts[0].address, "*:80");
        assert_eq!(view.virtual_hosts[0].aliases.len(), 1);
        assert_eq!(
            view.virtual_hosts[1].document_root.as_deref(),
            Some("/srv/explorer")
        );
        assert_eq!(view.virtual_hosts[1].logs.len(), 2);
    }

    #[test]
    fn a_directive_inside_a_directory_does_not_reach_the_virtual_host() {
        let view = parse(CONFIG);

        assert_eq!(view.containers.len(), 2);
        assert_eq!(
            view.containers[0].options,
            vec!["FollowSymLinks".to_owned()]
        );
        assert_eq!(view.containers[0].allow_override.as_deref(), Some("None"));
    }

    #[test]
    fn reads_the_modules_and_the_rewrite_lines() {
        let view = parse(CONFIG);

        assert_eq!(
            view.modules,
            vec!["rewrite_module".to_owned(), "ssl_module".to_owned()]
        );
        assert_eq!(
            view.rewrites.len(),
            2,
            "the condition and the rule, not the engine"
        );
    }

    #[test]
    fn names_the_container_that_lists_its_directory() {
        let view = parse(CONFIG);

        assert_eq!(
            view.lists_directories,
            vec!["Directory /srv/explorer/downloads".to_owned()],
            "`+Indexes` counts as much as a bare `Indexes`"
        );
    }

    #[test]
    fn presents_the_listing_warning_with_its_reason() {
        let data = serde_json::to_value(parse(CONFIG)).unwrap();

        let lines = ApacheconfPresentation.present(&data);

        assert_eq!(lines[0], "2 virtual host(s):");
        assert!(
            lines
                .iter()
                .any(|line| line.contains("everything in the directory"))
        );
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/apacheconf/explorer.conf");

        let data = ApacheconfCore.view(&path).unwrap();
        let view: ApacheconfView = serde_json::from_value(data).unwrap();

        assert!(view.virtual_hosts.len() >= 2);
        assert!(view.containers.len() >= 2);
        assert!(view.modules.len() >= 3);
        assert!(view.rewrites.len() >= 2);
        assert!(view.certificates.len() >= 2);
        assert!(!view.lists_directories.is_empty());
        assert!(view.virtual_hosts.iter().any(|h| !h.aliases.is_empty()));
        assert!(view.virtual_hosts.iter().any(|h| h.document_root.is_some()));
        assert!(view.virtual_hosts.iter().any(|h| h.certificate.is_some()));
        assert!(view.virtual_hosts.iter().any(|h| !h.logs.is_empty()));
        assert!(view.containers.iter().any(|c| c.allow_override.is_some()));
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::ApacheconfCore),
            plugin_api::PluginPresentation::extensions(&crate::ApacheconfPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
