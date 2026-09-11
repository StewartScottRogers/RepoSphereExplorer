//! nginx configuration file type plugin: core and presentation halves.
//!
//! An nginx configuration says what is served, from where, and to whom.
//! This reads the server blocks with their names and ports, what each
//! location does with a request, the upstreams and how many backends
//! each has, the log and certificate paths, and which servers answer
//! over plain HTTP rather than redirecting to the encrypted site.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
///
/// `.conf` is claimed by nothing and means nothing: Apache, systemd,
/// and a dozen other formats use it too. This is recognised by what is
/// inside it, the way the Caddyfile is.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One `location` block, and the one thing it mainly does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    /// The path it matches, modifier included.
    pub path: String,
    /// What it does with a request: proxies it, serves a directory,
    /// answers directly, or something this plugin does not name.
    pub does: String,
}

/// One `server` block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    /// The names it answers to.
    pub names: Vec<String>,
    /// The addresses and ports it listens on.
    pub listens: Vec<String>,
    /// Its locations, in the order they are written.
    pub locations: Vec<Location>,
    /// The certificate it presents, when it presents one.
    pub certificate: Option<String>,
    /// Where it writes its access log.
    pub access_log: Option<String>,
    /// Whether it does nothing but redirect.
    pub redirects: bool,
}

/// View data produced by [`NginxconfCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NginxconfView {
    /// Every server block.
    pub servers: Vec<Server>,
    /// Each upstream, with how many backends it names.
    pub upstreams: Vec<String>,
    /// Every log path the file mentions.
    pub logs: Vec<String>,
    /// Every certificate or key path the file mentions.
    pub certificates: Vec<String>,
    /// Servers listening on plain HTTP that serve rather than redirect.
    pub plain_http: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The directives that say what a location does, in the order they win.
const ACTIONS: &[&str] = &[
    "proxy_pass",
    "return",
    "alias",
    "root",
    "try_files",
    "fastcgi_pass",
];

/// `line` with its comment and trailing semicolon removed.
fn cleaned(line: &str) -> &str {
    let line = match line.find('#') {
        Some(at) => &line[..at],
        None => line,
    };
    line.trim().trim_end_matches(';').trim()
}

/// The words of a directive: its name, then its arguments.
fn words(line: &str) -> Vec<&str> {
    cleaned(line).split_whitespace().collect()
}

/// Whether `line` opens a block rather than stating a directive.
///
/// `server {` opens one; `server 127.0.0.1:8080;`, inside an upstream,
/// does not, and reading it as a block would lose every backend.
fn opens_block(line: &str) -> bool {
    cleaned(line).ends_with('{')
}

/// The name a block line opens, without its brace.
fn block_name(line: &str) -> String {
    cleaned(line).trim_end_matches('{').trim().to_owned()
}

/// Applies one line inside a `location` block.
fn read_location(location: &mut Location, line: &str) {
    if location.does != "not stated" {
        return;
    }
    let parts = words(line);
    let Some(name) = parts.first() else {
        return;
    };
    if !ACTIONS.contains(name) {
        return;
    }
    let rest = parts[1..].join(" ");
    location.does = match *name {
        "proxy_pass" | "fastcgi_pass" => format!("passes it to {rest}"),
        "return" => format!("answers {rest}"),
        "root" | "alias" => format!("serves files from {rest}"),
        _ => format!("tries {rest}"),
    };
}

/// Applies one line inside a `server` block.
///
/// The two collected lists are passed separately rather than the whole
/// view: the server being written to is borrowed out of that same view.
fn read_server(
    server: &mut Server,
    line: &str,
    certificates: &mut Vec<String>,
    logs: &mut Vec<String>,
) {
    let parts = words(line);
    let Some(name) = parts.first() else {
        return;
    };
    let rest: Vec<String> = parts[1..].iter().map(|word| (*word).to_owned()).collect();
    match *name {
        "listen" => server.listens.push(rest.join(" ")),
        "server_name" => server.names.extend(rest),
        "ssl_certificate" | "ssl_certificate_key" => {
            let path = rest.join(" ");
            if name == &"ssl_certificate" {
                server.certificate = Some(path.clone());
            }
            certificates.push(path);
        }
        "access_log" | "error_log" => {
            let path = rest.first().cloned().unwrap_or_default();
            if name == &"access_log" && server.access_log.is_none() {
                server.access_log = Some(path.clone());
            }
            if !path.is_empty() && !logs.contains(&path) {
                logs.push(path);
            }
        }
        "return" => server.redirects = true,
        _ => {}
    }
}

/// Whether `listen` is plain, unencrypted HTTP.
fn is_plain_http(listen: &str) -> bool {
    if listen.contains("ssl") || listen.contains("quic") {
        return false;
    }
    let port = listen
        .split_whitespace()
        .next()
        .and_then(|address| address.rsplit(':').next())
        .unwrap_or(listen);
    port == "80" || port == "8080" || listen.trim() == "80"
}

/// Everything [`NginxconfView`] holds, read from `text`.
fn parse(text: &str) -> NginxconfView {
    let mut view = NginxconfView {
        servers: Vec::new(),
        upstreams: Vec::new(),
        logs: Vec::new(),
        certificates: Vec::new(),
        plain_http: Vec::new(),
        truncated: false,
    };
    // The labels of the blocks currently open, outermost first.
    let mut stack: Vec<String> = Vec::new();
    let mut backends = 0usize;

    for raw in text.lines() {
        let line = cleaned(raw);
        if line.is_empty() {
            continue;
        }
        if line == "}" {
            if let Some(closed) = stack.pop()
                && closed.starts_with("upstream ")
            {
                let name = closed.trim_start_matches("upstream ").to_owned();
                view.upstreams
                    .push(format!("{name} ({backends} backend(s))"));
                backends = 0;
            }
            continue;
        }
        if opens_block(line) {
            let name = block_name(line);
            if name == "server" {
                view.servers.push(Server {
                    names: Vec::new(),
                    listens: Vec::new(),
                    locations: Vec::new(),
                    certificate: None,
                    access_log: None,
                    redirects: false,
                });
            } else if let Some(path) = name.strip_prefix("location ")
                && let Some(server) = view.servers.last_mut()
            {
                server.locations.push(Location {
                    path: path.trim().to_owned(),
                    does: "not stated".to_owned(),
                });
            }
            stack.push(name);
            continue;
        }
        let inside = stack.last().map_or("", String::as_str);
        if inside.starts_with("upstream ") {
            if words(line).first() == Some(&"server") {
                backends += 1;
            }
            continue;
        }
        if inside.starts_with("location ")
            && let Some(location) = view.servers.last_mut().and_then(|s| s.locations.last_mut())
        {
            read_location(location, line);
            continue;
        }
        let NginxconfView {
            servers,
            certificates,
            logs,
            ..
        } = &mut view;
        if let Some(server) = servers.last_mut()
            && inside == "server"
        {
            read_server(server, line, certificates, logs);
            continue;
        }
        // Outside any server: the top-level and `http` logs.
        let parts = words(line);
        if matches!(parts.first(), Some(&"access_log" | &"error_log"))
            && let Some(path) = parts.get(1)
            && !view.logs.contains(&(*path).to_owned())
        {
            view.logs.push((*path).to_owned());
        }
    }

    view.plain_http = view
        .servers
        .iter()
        .filter(|server| !server.redirects && server.listens.iter().any(|l| is_plain_http(l)))
        .map(|server| {
            let named = if server.names.is_empty() {
                "(no server_name)".to_owned()
            } else {
                server.names.join(", ")
            };
            format!("{named} on {}", server.listens.join(", "))
        })
        .collect();
    view
}

/// Whether `text` is an nginx configuration.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    // Braces and semicolons alone are most of C. It has to have declared
    // a server that listens, or an upstream, the way nginx spells them.
    !view.upstreams.is_empty()
        || view
            .servers
            .iter()
            .any(|server| !server.listens.is_empty() || !server.names.is_empty())
}

/// The nginx configuration plugin's core half.
#[derive(Debug, Default)]
pub struct NginxconfCore;

impl PluginCore for NginxconfCore {
    fn name(&self) -> &'static str {
        "nginxconf"
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
        // The servers and their locations are what a reader came for,
        // and they are on the view already.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The nginx configuration plugin's presentation half.
#[derive(Debug, Default)]
pub struct NginxconfPresentation;

impl PluginPresentation for NginxconfPresentation {
    fn name(&self) -> &'static str {
        "nginxconf"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "NGX",
            tint: 0x0000_9639,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: NginxconfView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("{} server block(s):", view.servers.len()));
        for server in &view.servers {
            let named = if server.names.is_empty() {
                "(no server_name)".to_owned()
            } else {
                server.names.join(", ")
            };
            lines.push(format!("  {named}"));
            if !server.listens.is_empty() {
                lines.push(format!("      listens on {}", server.listens.join("; ")));
            }
            if let Some(certificate) = &server.certificate {
                lines.push(format!("      certificate {certificate}"));
            }
            if let Some(log) = &server.access_log {
                lines.push(format!("      access log {log}"));
            }
            for location in &server.locations {
                lines.push(format!("      {} - {}", location.path, location.does));
            }
        }
        if !view.upstreams.is_empty() {
            lines.push(format!("Upstreams: {}", view.upstreams.join(", ")));
        }
        if !view.logs.is_empty() {
            lines.push(format!("Logs: {}", view.logs.join(", ")));
        }
        if !view.certificates.is_empty() {
            lines.push(format!("Certificates: {}", view.certificates.join(", ")));
        }
        if !view.plain_http.is_empty() {
            lines.push("Served over plain HTTP rather than redirected, so a".to_owned());
            lines.push("request and its reply cross the network unencrypted:".to_owned());
            for server in &view.plain_http {
                lines.push(format!("  {server}"));
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
    use super::{NginxconfCore, NginxconfPresentation, NginxconfView, is_plain_http, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const CONFIG: &str = concat!(
        "http {\n",
        "    access_log /var/log/nginx/access.log main;\n",
        "\n",
        "    upstream explorer_backend {\n",
        "        server 127.0.0.1:8080 weight=3;\n",
        "        server 127.0.0.1:8081 backup;\n",
        "    }\n",
        "\n",
        "    server {\n",
        "        listen 80;\n",
        "        server_name explorer.example.com;\n",
        "        return 301 https://$host$request_uri;\n",
        "    }\n",
        "\n",
        "    server {\n",
        "        listen 443 ssl http2;\n",
        "        server_name explorer.example.com;\n",
        "        ssl_certificate /etc/ssl/certs/explorer.crt;\n",
        "        ssl_certificate_key /etc/ssl/private/explorer.key;\n",
        "        access_log /var/log/nginx/explorer.access.log;\n",
        "\n",
        "        location / {\n",
        "            proxy_pass http://explorer_backend;\n",
        "        }\n",
        "        location /static/ {\n",
        "            root /srv/explorer;\n",
        "        }\n",
        "    }\n",
        "\n",
        "    server {\n",
        "        listen 80;\n",
        "        server_name metrics.example.com;\n",
        "        location / {\n",
        "            proxy_pass http://127.0.0.1:9090;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );

    #[test]
    fn sniffs_a_configuration() {
        assert!(NginxconfCore.sniff(CONFIG.as_bytes()));
    }

    #[test]
    fn does_not_claim_anything_with_braces_and_semicolons() {
        assert!(!NginxconfCore.sniff(b"struct Point { int x; int y; };\n"));
        assert!(!NginxconfCore.sniff(b""));
    }

    #[test]
    fn a_backend_line_is_not_a_server_block() {
        let view = parse(CONFIG);

        assert_eq!(
            view.servers.len(),
            3,
            "`server 127.0.0.1:8080;` inside an upstream is a backend, not a block"
        );
        assert_eq!(
            view.upstreams,
            vec!["explorer_backend (2 backend(s))".to_owned()]
        );
    }

    #[test]
    fn reads_what_each_location_does() {
        let view = parse(CONFIG);

        let secure = &view.servers[1];
        assert_eq!(secure.locations.len(), 2);
        assert_eq!(secure.locations[0].path, "/");
        assert!(secure.locations[0].does.contains("passes it to"));
        assert!(secure.locations[1].does.contains("serves files from"));
    }

    #[test]
    fn reads_the_certificates_and_the_logs() {
        let view = parse(CONFIG);

        assert_eq!(
            view.servers[1].certificate.as_deref(),
            Some("/etc/ssl/certs/explorer.crt")
        );
        assert_eq!(view.certificates.len(), 2);
        assert_eq!(view.logs.len(), 2);
    }

    #[test]
    fn a_redirect_is_not_a_plain_http_service() {
        let view = parse(CONFIG);

        assert_eq!(
            view.plain_http,
            vec!["metrics.example.com on 80".to_owned()],
            "the first :80 server redirects, so only the third serves in the clear"
        );
    }

    #[test]
    fn listening_with_ssl_is_not_plain_http() {
        assert!(is_plain_http("80"));
        assert!(is_plain_http("0.0.0.0:80"));
        assert!(!is_plain_http("443 ssl http2"));
        assert!(!is_plain_http("443"));
    }

    #[test]
    fn presents_the_plain_http_warning_with_its_reason() {
        let data = serde_json::to_value(parse(CONFIG)).unwrap();

        let lines = NginxconfPresentation.present(&data);

        assert_eq!(lines[0], "3 server block(s):");
        assert!(
            lines
                .iter()
                .any(|line| line.contains("cross the network unencrypted"))
        );
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/nginxconf/nginx.conf");

        let data = NginxconfCore.view(&path).unwrap();
        let view: NginxconfView = serde_json::from_value(data).unwrap();

        assert!(view.servers.len() >= 3);
        assert!(!view.upstreams.is_empty());
        assert!(view.logs.len() >= 2);
        assert!(view.certificates.len() >= 2);
        assert!(!view.plain_http.is_empty());
        assert!(view.servers.iter().any(|s| s.certificate.is_some()));
        assert!(view.servers.iter().any(|s| s.access_log.is_some()));
        assert!(view.servers.iter().any(|s| s.redirects));
        assert!(view.servers.iter().flat_map(|s| &s.locations).count() >= 4);
        assert!(
            view.servers
                .iter()
                .flat_map(|s| &s.locations)
                .any(|l| l.does.contains("answers"))
        );
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::NginxconfCore),
            plugin_api::PluginPresentation::extensions(&crate::NginxconfPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
