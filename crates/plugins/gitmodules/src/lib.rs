//! Git submodule declaration (`.gitmodules`) file type plugin: core and
//! presentation halves.
//!
//! A specialisation of the git configuration syntax `gitconfig` already
//! reads and the general `ini` dialect beneath that: a `[submodule "name"]`
//! section is a shape neither sibling's own format defines, but their
//! looser sniffs both match it anyway - `gitconfig` treats a quoted
//! `submodule` subsection as one of its own markers, and `ini` accepts any
//! bracketed header. `specialises` settles it in this plugin's favour.
//!
//! The real file is always named `.gitmodules`, which would settle
//! recognition outright - but `PluginCore::sniff` only ever receives a
//! content prefix, never a path (`crates/plugin-api/src/lib.rs`), the same
//! conflict the Dockerfile (#39), Makefile (#38) and Docker Compose (#706)
//! plugins hit. Resolved the same way here: content alone decides.

use plugin_api::{Icon, PluginCore, PluginPresentation, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;
use syntax::{Language, Quote};

/// The lowercase extensions this type claims, without their dot. None: the
/// real file is recognised by name, which sniffing cannot see.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One submodule declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Submodule {
    /// The name in the section header.
    pub name: String,
    /// The path it is checked out at, inside the parent working copy.
    pub path: Option<String>,
    /// The address it is cloned from.
    pub url: Option<String>,
    /// The provider that address names, e.g. `github.com`.
    pub provider: Option<String>,
    /// The branch it tracks, when one is configured.
    pub branch: Option<String>,
    /// Its `update` mode - `checkout`, `rebase`, `merge`, `none`, or a
    /// custom `!command` - when one is configured.
    pub update: Option<String>,
    /// Whether it is cloned with a depth limit.
    pub shallow: bool,
    /// Its `ignore` setting - `all`, `dirty`, `untracked` or `none` - when
    /// one is configured.
    pub ignore: Option<String>,
}

/// View data produced by [`GitmodulesCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitmodulesView {
    /// The submodules declared, in file order.
    pub submodules: Vec<Submodule>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The section name and its quoted subsection, if `line` is a header.
fn header(line: &str) -> Option<(String, Option<String>)> {
    let inner = line.trim().strip_prefix('[')?.strip_suffix(']')?;
    let inner = inner.trim();
    if inner.is_empty() {
        return None;
    }
    match inner.split_once(char::is_whitespace) {
        Some((section, rest)) => Some((
            section.trim().to_lowercase(),
            Some(rest.trim().trim_matches('"').to_owned()),
        )),
        None => Some((inner.to_lowercase(), None)),
    }
}

/// The key and value of an assignment, if `line` is one.
fn assignment(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.starts_with('#') || trimmed.starts_with(';') {
        return None;
    }
    let (key, value) = trimmed.split_once('=')?;
    Some((key.trim().to_lowercase(), value.trim().to_owned()))
}

/// The host in a submodule's address: `github.com`, `gitlab.com`,
/// `bitbucket.org`, a self-hosted name, whatever it is.
///
/// Handles the two shapes an address is written in - a uniform resource
/// locator (`https://github.com/owner/name.git`) and the secure shell short
/// form (`git@github.com:owner/name.git`) - and gives up rather than
/// guessing on anything else, including a purely local path.
fn provider_of(address: &str) -> Option<String> {
    let address = address.trim();
    if address.is_empty() {
        return None;
    }

    if let Some((_scheme, rest)) = address.split_once("://") {
        let authority = rest.split(['/', '?', '#']).next()?;
        let host = authority.rsplit('@').next()?;
        let host = host.split(':').next()?;
        return (!host.is_empty()).then(|| host.to_ascii_lowercase());
    }

    if let Some((credentials, rest)) = address.split_once('@')
        && !credentials.contains('/')
    {
        let host = rest.split([':', '/']).next()?;
        return (!host.is_empty()).then(|| host.to_ascii_lowercase());
    }

    None
}

/// Everything [`GitmodulesView`] holds, read from `text`.
fn parse(text: &str) -> GitmodulesView {
    let mut view = GitmodulesView {
        submodules: Vec::new(),
        content: String::new(),
        truncated: false,
    };
    let mut section = String::new();

    for line in text.lines() {
        if let Some((name, sub)) = header(line) {
            section = match (name.as_str(), sub) {
                ("submodule", Some(sub_name)) => {
                    view.submodules.push(Submodule {
                        name: sub_name,
                        path: None,
                        url: None,
                        provider: None,
                        branch: None,
                        update: None,
                        shallow: false,
                        ignore: None,
                    });
                    name
                }
                _ => String::new(),
            };
            continue;
        }
        if section != "submodule" {
            continue;
        }
        let Some((key, value)) = assignment(line) else {
            continue;
        };
        let Some(submodule) = view.submodules.last_mut() else {
            continue;
        };
        match key.as_str() {
            "path" => submodule.path = Some(value),
            "url" => {
                submodule.provider = provider_of(&value);
                submodule.url = Some(value);
            }
            "branch" => submodule.branch = Some(value),
            "update" => submodule.update = Some(value),
            "shallow" => submodule.shallow = value.eq_ignore_ascii_case("true"),
            "ignore" => submodule.ignore = Some(value),
            _ => {}
        }
    }

    view
}

/// Whether `text` looks like a git submodule declaration: at least one
/// `[submodule "name"]` section.
fn looks_like_it(text: &str) -> bool {
    text.lines().any(|line| {
        matches!(header(line), Some((name, Some(sub))) if name == "submodule" && !sub.is_empty())
    })
}

/// The Git submodule declaration plugin's core half.
#[derive(Debug, Default)]
pub struct GitmodulesCore;

/// How this language is coloured, for the shared tokeniser. GUIDANCE.md
/// §3.6: the plugin describes its own format, the pane paints what it is
/// told.
const GITMODULES: Language = Language {
    line_comment: &["#", ";"],
    block_comment: &[],
    quotes: &[Quote::simple('"')],
    keywords: &[
        "submodule",
        "path",
        "url",
        "branch",
        "update",
        "shallow",
        "ignore",
    ],
    types: &[],
    calls: false,
    ignore_case: false,
};

impl PluginCore for GitmodulesCore {
    fn name(&self) -> &'static str {
        "gitmodules"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        &["gitconfig", "ini"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let mut view = parse(&content);
        view.content = content;
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Git submodule declaration plugin's presentation half.
#[derive(Debug, Default)]
pub struct GitmodulesPresentation;

impl PluginPresentation for GitmodulesPresentation {
    fn classify(&self, text: &str) -> Vec<Span> {
        syntax::classify(text, &GITMODULES)
    }

    fn name(&self) -> &'static str {
        "gitmodules"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "SUB",
            tint: 0x00f0_5033,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: GitmodulesView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if view.submodules.is_empty() {
            lines.push("No submodules declared".to_owned());
        } else {
            lines.push(format!("Submodules ({}):", view.submodules.len()));
            for submodule in &view.submodules {
                let path = submodule.path.as_deref().unwrap_or("no path");
                let url = submodule.url.as_deref().unwrap_or("no address");
                lines.push(format!("  {}  {path}", submodule.name));
                lines.push(format!("      {url}"));
                if let Some(provider) = &submodule.provider {
                    lines.push(format!("      via {provider}"));
                }
                if let Some(branch) = &submodule.branch {
                    lines.push(format!("      tracks {branch}"));
                }
                if let Some(update) = &submodule.update {
                    lines.push(format!("      update: {update}"));
                }
                if submodule.shallow {
                    lines.push("      shallow".to_owned());
                }
                if let Some(ignore) = &submodule.ignore {
                    lines.push(format!("      ignore: {ignore}"));
                }
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
    use super::{GitmodulesCore, GitmodulesPresentation, GitmodulesView, header, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_a_quoted_submodule_section() {
        assert!(GitmodulesCore.sniff(
            b"[submodule \"vendor/widgets\"]\n\tpath = vendor/widgets\n\turl = https://example.com/widgets.git\n"
        ));
    }

    #[test]
    fn does_not_claim_a_gitconfig_or_a_general_ini_file() {
        assert!(!GitmodulesCore.sniff(b"[remote \"origin\"]\n\turl = https://example.com/a\n"));
        assert!(!GitmodulesCore.sniff(b"[core]\n\tbare = false\n[user]\n\tname = Ada\n"));
        assert!(!GitmodulesCore.sniff(b"[server]\nport = 8080\n"));
        assert!(!GitmodulesCore.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_gitconfig_and_ini() {
        assert_eq!(GitmodulesCore.specialises(), &["gitconfig", "ini"]);
    }

    #[test]
    fn reads_a_quoted_subsection_name() {
        assert_eq!(
            header("[submodule \"vendor/widgets\"]"),
            Some(("submodule".to_owned(), Some("vendor/widgets".to_owned())))
        );
        assert_eq!(header("not a header"), None);
    }

    #[test]
    fn reads_name_path_url_and_provider() {
        let view = parse(
            "[submodule \"vendor/widgets\"]\n\tpath = vendor/widgets\n\
             \turl = https://github.com/example/widgets.git\n",
        );

        assert_eq!(view.submodules.len(), 1);
        let submodule = &view.submodules[0];
        assert_eq!(submodule.name, "vendor/widgets");
        assert_eq!(submodule.path.as_deref(), Some("vendor/widgets"));
        assert_eq!(
            submodule.url.as_deref(),
            Some("https://github.com/example/widgets.git")
        );
        assert_eq!(submodule.provider.as_deref(), Some("github.com"));
    }

    #[test]
    fn reads_a_short_form_address_provider() {
        let view =
            parse("[submodule \"libs/format\"]\n\turl = git@gitlab.com:example/format.git\n");

        assert_eq!(view.submodules[0].provider.as_deref(), Some("gitlab.com"));
    }

    #[test]
    fn reads_the_branch_it_tracks_and_shallow_flag() {
        let view = parse("[submodule \"libs/format\"]\n\tbranch = release\n\tshallow = true\n");

        assert_eq!(view.submodules[0].branch.as_deref(), Some("release"));
        assert!(view.submodules[0].shallow);
    }

    #[test]
    fn reads_an_update_mode_of_none_and_an_ignore_setting() {
        let view = parse("[submodule \"tools/legacy\"]\n\tupdate = none\n\tignore = dirty\n");

        assert_eq!(view.submodules[0].update.as_deref(), Some("none"));
        assert_eq!(view.submodules[0].ignore.as_deref(), Some("dirty"));
        assert!(!view.submodules[0].shallow);
    }

    #[test]
    fn reads_several_submodules_in_file_order() {
        let view = parse("[submodule \"a\"]\n\tpath = a\n[submodule \"b\"]\n\tpath = b\n");

        assert_eq!(view.submodules.len(), 2);
        assert_eq!(view.submodules[0].name, "a");
        assert_eq!(view.submodules[1].name, "b");
    }

    #[test]
    fn a_malformed_declaration_is_skipped_without_panicking() {
        // An unterminated section header names nothing to fill in, and is
        // simply not a header: the assignments that would have followed it
        // attach to whatever section came before, never to a submodule
        // that was never created.
        let view = parse("[submodule \"broken\n\tpath = never seen\n");

        assert!(view.submodules.is_empty());
    }

    #[test]
    fn a_nameless_submodule_section_creates_no_entry() {
        let view = parse("[submodule]\n\tpath = never seen\n");

        assert!(view.submodules.is_empty());
    }

    #[test]
    fn presents_a_count_and_falls_back_when_empty() {
        let empty = serde_json::to_value(parse("")).unwrap();
        assert_eq!(
            GitmodulesPresentation.present(&empty),
            vec!["No submodules declared".to_owned()]
        );

        let data = serde_json::to_value(parse(
            "[submodule \"a\"]\n\tpath = a\n\turl = https://github.com/example/a.git\n",
        ))
        .unwrap();
        let lines = GitmodulesPresentation.present(&data);
        assert_eq!(lines[0], "Submodules (1):");
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/gitmodules/repository.gitmodules");

        let data = GitmodulesCore.view(&path).unwrap();
        let view: GitmodulesView = serde_json::from_value(data).unwrap();

        assert!(view.submodules.len() >= 3);
        assert!(view.submodules.iter().all(|s| s.path.is_some()));
        assert!(view.submodules.iter().all(|s| s.url.is_some()));
        assert!(view.submodules.iter().all(|s| s.provider.is_some()));
        let providers: std::collections::BTreeSet<_> = view
            .submodules
            .iter()
            .filter_map(|s| s.provider.as_deref())
            .collect();
        assert!(providers.len() >= 2, "fixture should span two providers");
        assert!(view.submodules.iter().any(|s| s.branch.is_some()));
        assert!(view.submodules.iter().any(|s| s.shallow));
        assert!(
            view.submodules
                .iter()
                .any(|s| s.update.as_deref() == Some("none"))
        );
        assert!(view.submodules.iter().any(|s| s.ignore.is_some()));
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::GitmodulesCore),
            plugin_api::PluginPresentation::extensions(&crate::GitmodulesPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
