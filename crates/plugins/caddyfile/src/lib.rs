//! Caddyfile file type plugin: core and presentation halves.
//!
//! A Caddyfile is a list of site addresses and what to do for each. This
//! reads the global options, every site with its directives, the reverse
//! proxy targets, the named matchers, the snippets - and the sites
//! written as http://, which is how a Caddyfile turns off the
//! certificate Caddy would otherwise obtain and renew by itself.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One site block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Site {
    /// The addresses the block is opened with.
    pub addresses: Vec<String>,
    /// The directives it uses, by name, in the order first seen.
    pub directives: Vec<String>,
    /// Where it sends requests on, when it proxies them.
    pub proxies_to: Vec<String>,
    /// Whether it serves files from disk.
    pub serves_files: bool,
}

/// View data produced by [`CaddyfileCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaddyfileView {
    /// The options in the leading global block.
    pub global_options: Vec<String>,
    /// Every site block.
    pub sites: Vec<Site>,
    /// The named matchers defined, with the site they belong to.
    pub matchers: Vec<String>,
    /// The snippets defined, which are `(name)` blocks.
    pub snippets: Vec<String>,
    /// Every reverse proxy target in the file.
    pub proxy_targets: Vec<String>,
    /// Sites written as `http://`, which turns off the certificate Caddy
    /// would otherwise get and renew by itself.
    pub plain_http: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Directives common enough that finding one settles what the file is.
const KNOWN_DIRECTIVES: &[&str] = &[
    "reverse_proxy",
    "file_server",
    "encode",
    "root",
    "handle",
    "handle_path",
    "respond",
    "redir",
    "rewrite",
    "route",
    "tls",
    "log",
    "header",
    "basicauth",
    "php_fastcgi",
    "templates",
    "try_files",
    "import",
    "bind",
    "uri",
];

/// `line` with its comment stripped.
fn cleaned(line: &str) -> &str {
    let line = line.trim();
    if line.starts_with('#') {
        return "";
    }
    match line.find(" #") {
        Some(at) => line[..at].trim_end(),
        None => line,
    }
}

/// The addresses a site block is opened with.
///
/// Caddy allows several, comma separated, and the brace closes the line.
fn addresses_of(line: &str) -> Vec<String> {
    line.trim_end_matches('{')
        .trim()
        .split(',')
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Everything [`CaddyfileView`] holds, read from `text`.
fn parse(text: &str) -> CaddyfileView {
    let mut view = CaddyfileView {
        global_options: Vec::new(),
        sites: Vec::new(),
        matchers: Vec::new(),
        snippets: Vec::new(),
        proxy_targets: Vec::new(),
        plain_http: Vec::new(),
        truncated: false,
    };
    let mut depth = 0usize;
    // What the outermost open block is: the global options, a snippet, or
    // a site. Only the first block in the file may be global, and only
    // when it is opened by a bare brace.
    let mut outer = String::new();
    let mut seen_a_block = false;

    for raw in text.lines() {
        let line = cleaned(raw);
        if line.is_empty() {
            continue;
        }
        let opens = line.matches('{').count();
        let closes = line.matches('}').count();

        if depth == 0 && opens > closes {
            let head = line.trim_end_matches('{').trim();
            if head.is_empty() && !seen_a_block {
                "global".clone_into(&mut outer);
            } else if let Some(name) = head.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
                "snippet".clone_into(&mut outer);
                view.snippets.push(name.to_owned());
            } else {
                "site".clone_into(&mut outer);
                view.sites.push(Site {
                    addresses: addresses_of(line),
                    directives: Vec::new(),
                    proxies_to: Vec::new(),
                    serves_files: false,
                });
            }
            seen_a_block = true;
            depth += opens - closes;
            continue;
        }

        if depth > 0 {
            read_member(&mut view, &outer, line);
        }
        depth = depth + opens - closes.min(depth + opens);
    }

    view.plain_http = view
        .sites
        .iter()
        .flat_map(|site| &site.addresses)
        .filter(|address| address.starts_with("http://"))
        .cloned()
        .collect();
    view
}

/// Applies one line inside whichever outermost block is open.
fn read_member(view: &mut CaddyfileView, outer: &str, line: &str) {
    let mut parts = line.split_whitespace();
    let Some(first) = parts.next() else {
        return;
    };
    if first == "}" {
        return;
    }
    if outer == "global" {
        view.global_options.push(first.to_owned());
        return;
    }
    // A named matcher is `@name ...`, and belongs to the site it is in.
    if let Some(name) = first.strip_prefix('@') {
        let site = view
            .sites
            .last()
            .and_then(|site| site.addresses.first().cloned())
            .unwrap_or_else(|| "a snippet".to_owned());
        view.matchers.push(format!("{name} in {site}"));
        return;
    }
    let rest: Vec<String> = parts.map(str::to_owned).collect();
    if first == "reverse_proxy" {
        let targets: Vec<String> = rest
            .iter()
            .filter(|word| !word.starts_with('@') && !word.starts_with('{'))
            .cloned()
            .collect();
        for target in &targets {
            if !view.proxy_targets.contains(target) {
                view.proxy_targets.push(target.clone());
            }
        }
        if let Some(site) = view.sites.last_mut() {
            site.proxies_to.extend(targets);
        }
    }
    if let Some(site) = view.sites.last_mut()
        && outer == "site"
    {
        if first == "file_server" {
            site.serves_files = true;
        }
        if !site.directives.contains(&first.to_owned()) {
            site.directives.push(first.to_owned());
        }
    }
}

/// Whether `text` is a Caddyfile.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    if view.sites.is_empty() {
        return false;
    }
    // Braces and bare words are most configuration languages. It has to
    // use directives Caddy has, and no site address may end in a
    // semicolon - that would be nginx.
    let uses_caddy = view
        .sites
        .iter()
        .flat_map(|site| &site.directives)
        .any(|directive| KNOWN_DIRECTIVES.contains(&directive.as_str()));
    let nginx_shaped = text.lines().any(|line| cleaned(line).ends_with(';'));
    uses_caddy && !nginx_shaped
}

/// The Caddyfile plugin's core half.
#[derive(Debug, Default)]
pub struct CaddyfileCore;

impl PluginCore for CaddyfileCore {
    fn name(&self) -> &'static str {
        "caddyfile"
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
        // The sites and their directives are the whole of the file, and
        // each of them is on the view already.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Caddyfile plugin's presentation half.
#[derive(Debug, Default)]
pub struct CaddyfilePresentation;

impl PluginPresentation for CaddyfilePresentation {
    fn name(&self) -> &'static str {
        "caddyfile"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "CAD",
            tint: 0x0022_b638,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: CaddyfileView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.global_options.is_empty() {
            lines.push(format!(
                "Global options: {}",
                view.global_options.join(", ")
            ));
        }
        if !view.snippets.is_empty() {
            lines.push(format!("Snippets: {}", view.snippets.join(", ")));
        }
        lines.push(format!("{} site(s):", view.sites.len()));
        for site in &view.sites {
            lines.push(format!("  {}", site.addresses.join(", ")));
            if !site.directives.is_empty() {
                lines.push(format!("      {}", site.directives.join(", ")));
            }
            if !site.proxies_to.is_empty() {
                lines.push(format!(
                    "      passes requests to {}",
                    site.proxies_to.join(", ")
                ));
            }
            if site.serves_files {
                lines.push("      serves files from disk".to_owned());
            }
        }
        if !view.matchers.is_empty() {
            lines.push(format!("Matchers: {}", view.matchers.join(", ")));
        }
        if !view.proxy_targets.is_empty() {
            lines.push(format!("Proxy targets: {}", view.proxy_targets.join(", ")));
        }
        if !view.plain_http.is_empty() {
            lines.push("Written as http://, which turns off the certificate Caddy".to_owned());
            lines.push("would otherwise obtain and renew on its own:".to_owned());
            for address in &view.plain_http {
                lines.push(format!("  {address}"));
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
    use super::{CaddyfileCore, CaddyfilePresentation, CaddyfileView, addresses_of, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const CADDYFILE: &str = concat!(
        "{\n",
        "    email admin@example.com\n",
        "    admin off\n",
        "}\n",
        "\n",
        "(logging) {\n",
        "    log {\n",
        "        output file /var/log/caddy/access.log\n",
        "    }\n",
        "}\n",
        "\n",
        "explorer.example.com, www.explorer.example.com {\n",
        "    import logging\n",
        "    encode gzip zstd\n",
        "    @api path /api/*\n",
        "    handle @api {\n",
        "        reverse_proxy 127.0.0.1:8080 127.0.0.1:8081\n",
        "    }\n",
        "    handle {\n",
        "        root * /srv/explorer\n",
        "        file_server\n",
        "    }\n",
        "}\n",
        "\n",
        "http://metrics.internal:8080 {\n",
        "    reverse_proxy 127.0.0.1:9090\n",
        "}\n",
    );

    #[test]
    fn sniffs_a_caddyfile() {
        assert!(CaddyfileCore.sniff(CADDYFILE.as_bytes()));
    }

    #[test]
    fn does_not_claim_an_nginx_configuration() {
        // Braces and bare words look alike; the semicolons do not.
        assert!(!CaddyfileCore.sniff(b"server {\n    listen 80;\n    root /srv;\n}\n"));
        assert!(!CaddyfileCore.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_nothing_and_claims_no_extension() {
        assert!(CaddyfileCore.extensions().is_empty());
    }

    #[test]
    fn a_site_may_be_opened_with_several_addresses() {
        assert_eq!(
            addresses_of("explorer.example.com, www.explorer.example.com {"),
            vec![
                "explorer.example.com".to_owned(),
                "www.explorer.example.com".to_owned()
            ]
        );
    }

    #[test]
    fn the_leading_bare_block_is_global_and_a_paren_block_is_a_snippet() {
        let view = parse(CADDYFILE);

        assert_eq!(
            view.global_options,
            vec!["email".to_owned(), "admin".to_owned()]
        );
        assert_eq!(view.snippets, vec!["logging".to_owned()]);
        assert_eq!(
            view.sites.len(),
            2,
            "neither the global block nor the snippet is a site"
        );
    }

    #[test]
    fn reads_the_directives_and_the_proxy_targets() {
        let view = parse(CADDYFILE);

        let first = &view.sites[0];
        assert!(first.directives.contains(&"encode".to_owned()));
        assert!(first.serves_files);
        assert_eq!(
            view.proxy_targets,
            vec![
                "127.0.0.1:8080".to_owned(),
                "127.0.0.1:8081".to_owned(),
                "127.0.0.1:9090".to_owned()
            ]
        );
    }

    #[test]
    fn a_matcher_is_recorded_against_the_site_it_is_in() {
        let view = parse(CADDYFILE);

        assert_eq!(
            view.matchers,
            vec!["api in explorer.example.com".to_owned()]
        );
    }

    #[test]
    fn names_the_site_that_gives_up_its_certificate() {
        let view = parse(CADDYFILE);

        assert_eq!(
            view.plain_http,
            vec!["http://metrics.internal:8080".to_owned()],
            "a bare address gets a certificate from Caddy; an http:// one does not"
        );
    }

    #[test]
    fn presents_the_plain_http_warning_with_its_reason() {
        let data = serde_json::to_value(parse(CADDYFILE)).unwrap();

        let lines = CaddyfilePresentation.present(&data);

        assert!(
            lines
                .iter()
                .any(|line| line.contains("obtain and renew on its own"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("serves files from disk"))
        );
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/caddyfile/Caddyfile");

        let data = CaddyfileCore.view(&path).unwrap();
        let view: CaddyfileView = serde_json::from_value(data).unwrap();

        assert!(view.global_options.len() >= 2);
        assert!(view.sites.len() >= 2);
        assert!(!view.snippets.is_empty());
        assert!(!view.matchers.is_empty());
        assert!(view.proxy_targets.len() >= 2);
        assert!(!view.plain_http.is_empty());
        assert!(view.sites.iter().any(|site| site.serves_files));
        assert!(view.sites.iter().any(|site| !site.proxies_to.is_empty()));
        assert!(view.sites.iter().any(|site| site.addresses.len() >= 2));
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::CaddyfileCore),
            plugin_api::PluginPresentation::extensions(&crate::CaddyfilePresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
