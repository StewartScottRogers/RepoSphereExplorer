//! Git configuration file type plugin: core and presentation halves.
//!
//! Registered before `ini`, whose dialect this is: `[remote "origin"]`
//! and `[branch "main"]` are subsection headers no general INI file
//! writes, and the keys inside them are git's own.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["gitconfig"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One remote and where it points.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remote {
    /// The name in the section header.
    pub name: String,
    /// Its fetch address.
    pub url: Option<String>,
    /// Its push address, when it differs.
    pub push_url: Option<String>,
}

/// One branch and what it follows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branch {
    /// The branch name.
    pub name: String,
    /// The remote it tracks.
    pub remote: Option<String>,
    /// The ref on that remote.
    pub merge: Option<String>,
}

/// View data produced by [`GitconfigCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitconfigView {
    /// The remotes, in file order.
    pub remotes: Vec<Remote>,
    /// The branches with tracking configured.
    pub branches: Vec<Branch>,
    /// The identity this configuration commits as, as `name <email>`.
    pub identity: Option<String>,
    /// The aliases defined, as `name = expansion`.
    pub aliases: Vec<String>,
    /// The files it includes, conditionally or otherwise.
    pub includes: Vec<String>,
    /// Every section it sets anything in, each once.
    pub sections: Vec<String>,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The section name and its subsection, if `line` is a header.
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
        // `[remote.origin]` is not git's spelling, but `[core]` is.
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

/// Everything [`GitconfigView`] holds, read from `text`.
fn parse(text: &str) -> GitconfigView {
    let mut view = GitconfigView {
        remotes: Vec::new(),
        branches: Vec::new(),
        identity: None,
        aliases: Vec::new(),
        includes: Vec::new(),
        sections: Vec::new(),
        content: String::new(),
        truncated: false,
    };
    let mut section = String::new();
    let mut subsection: Option<String> = None;
    let mut user_name: Option<String> = None;
    let mut user_email: Option<String> = None;

    for line in text.lines() {
        if let Some((name, sub)) = header(line) {
            if !view.sections.contains(&name) {
                view.sections.push(name.clone());
            }
            match (name.as_str(), sub.as_deref()) {
                ("remote", Some(remote)) => view.remotes.push(Remote {
                    name: remote.to_owned(),
                    url: None,
                    push_url: None,
                }),
                ("branch", Some(branch)) => view.branches.push(Branch {
                    name: branch.to_owned(),
                    remote: None,
                    merge: None,
                }),
                _ => {}
            }
            section = name;
            subsection = sub;
            continue;
        }
        let Some((key, value)) = assignment(line) else {
            continue;
        };

        match (section.as_str(), key.as_str()) {
            ("remote", "url") => {
                if let Some(remote) = view.remotes.last_mut() {
                    remote.url = Some(value);
                }
            }
            ("remote", "pushurl") => {
                if let Some(remote) = view.remotes.last_mut() {
                    remote.push_url = Some(value);
                }
            }
            ("branch", "remote") => {
                if let Some(branch) = view.branches.last_mut() {
                    branch.remote = Some(value);
                }
            }
            ("branch", "merge") => {
                if let Some(branch) = view.branches.last_mut() {
                    branch.merge = Some(value);
                }
            }
            ("user", "name") => user_name = Some(value),
            ("user", "email") => user_email = Some(value),
            ("alias", _) => view.aliases.push(format!("{key} = {value}")),
            ("include" | "includeif", "path") => {
                let condition = subsection
                    .as_ref()
                    .map_or_else(String::new, |when| format!(" (when {when})"));
                view.includes.push(format!("{value}{condition}"));
            }
            _ => {}
        }
    }

    view.identity = match (user_name, user_email) {
        (Some(name), Some(email)) => Some(format!("{name} <{email}>")),
        (Some(name), None) => Some(name),
        (None, Some(email)) => Some(email),
        (None, None) => None,
    };
    view
}

/// Whether `text` looks like a git configuration.
fn looks_like_it(text: &str) -> bool {
    let mut git_sections = 0usize;
    for line in text.lines() {
        let Some((name, sub)) = header(line) else {
            continue;
        };
        // A quoted subsection under one of git's own section names is a
        // shape a general INI file does not have.
        if sub.is_some()
            && matches!(
                name.as_str(),
                "remote" | "branch" | "submodule" | "includeif"
            )
        {
            return true;
        }
        if matches!(
            name.as_str(),
            "core" | "user" | "alias" | "push" | "pull" | "fetch" | "merge" | "diff" | "init"
        ) {
            git_sections += 1;
        }
    }
    git_sections >= 2
}

/// The Git configuration plugin's core half.
#[derive(Debug, Default)]
pub struct GitconfigCore;

impl PluginCore for GitconfigCore {
    fn name(&self) -> &'static str {
        "gitconfig"
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
        let content = String::from_utf8_lossy(slice).into_owned();
        let mut view = parse(&content);
        view.content = content;
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Git configuration plugin's presentation half.
#[derive(Debug, Default)]
pub struct GitconfigPresentation;

impl PluginPresentation for GitconfigPresentation {
    fn name(&self) -> &'static str {
        "gitconfig"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "GIT",
            tint: 0x00f0_5033,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: GitconfigView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(identity) = &view.identity {
            lines.push(format!("Commits as: {identity}"));
        }
        if !view.remotes.is_empty() {
            lines.push(format!("Remotes ({}):", view.remotes.len()));
            for remote in &view.remotes {
                let url = remote.url.as_deref().unwrap_or("no address");
                lines.push(format!("  {}  {url}", remote.name));
                if let Some(push) = &remote.push_url {
                    lines.push(format!("      pushes to {push}"));
                }
            }
        }
        if !view.branches.is_empty() {
            lines.push(format!(
                "Branches tracking something ({}):",
                view.branches.len()
            ));
            for branch in &view.branches {
                let remote = branch.remote.as_deref().unwrap_or("?");
                let merge = branch.merge.as_deref().unwrap_or("?");
                lines.push(format!("  {} -> {remote} {merge}", branch.name));
            }
        }
        if !view.aliases.is_empty() {
            lines.push(format!("Aliases ({}):", view.aliases.len()));
            for alias in &view.aliases {
                lines.push(format!("  {alias}"));
            }
        }
        if !view.includes.is_empty() {
            lines.push(format!("Includes: {}", view.includes.join(", ")));
        }
        lines.push(format!("Sections: {}", view.sections.join(", ")));

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{GitconfigCore, GitconfigPresentation, GitconfigView, header, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_a_quoted_subsection_or_several_git_sections() {
        assert!(GitconfigCore.sniff(b"[remote \"origin\"]\n\turl = https://example.com/a\n"));
        assert!(GitconfigCore.sniff(b"[core]\n\tbare = false\n[user]\n\tname = Ada\n"));
    }

    #[test]
    fn does_not_claim_a_general_ini_file() {
        assert!(!GitconfigCore.sniff(b"[server]\nport = 8080\n[logging]\nlevel = info\n"));
        assert!(!GitconfigCore.sniff(b""));
    }

    #[test]
    fn reads_a_quoted_subsection_name() {
        assert_eq!(
            header("[remote \"origin\"]"),
            Some(("remote".to_owned(), Some("origin".to_owned())))
        );
        assert_eq!(header("[core]"), Some(("core".to_owned(), None)));
        assert_eq!(header("not a header"), None);
    }

    #[test]
    fn reads_remotes_with_a_separate_push_address() {
        let view = parse(
            "[remote \"origin\"]\n\turl = https://example.com/a.git\n\
             \tpushurl = git@example.com:a.git\n[remote \"upstream\"]\n\
             \turl = https://example.com/b.git\n",
        );

        assert_eq!(view.remotes.len(), 2);
        assert_eq!(view.remotes[0].name, "origin");
        assert_eq!(
            view.remotes[0].push_url.as_deref(),
            Some("git@example.com:a.git")
        );
        assert!(view.remotes[1].push_url.is_none());
    }

    #[test]
    fn reads_what_each_branch_tracks() {
        let view = parse(
            "[branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n\
             [branch \"spike\"]\n\tremote = upstream\n\tmerge = refs/heads/spike\n",
        );

        assert_eq!(view.branches.len(), 2);
        assert_eq!(view.branches[1].remote.as_deref(), Some("upstream"));
        assert_eq!(view.branches[1].merge.as_deref(), Some("refs/heads/spike"));
    }

    #[test]
    fn joins_the_identity_and_reads_aliases_and_includes() {
        let view = parse(
            "[user]\n\tname = Ada Lovelace\n\temail = ada@example.com\n\
             [alias]\n\tlg = log --oneline --graph\n\
             [includeIf \"gitdir:~/work/\"]\n\tpath = ~/.gitconfig-work\n",
        );

        assert_eq!(
            view.identity.as_deref(),
            Some("Ada Lovelace <ada@example.com>")
        );
        assert_eq!(view.aliases, vec!["lg = log --oneline --graph".to_owned()]);
        assert!(view.includes[0].contains("gitdir:~/work/"));
    }

    #[test]
    fn a_comment_sets_nothing() {
        let view = parse("[user]\n# name = Nobody\n\tname = Ada\n");

        assert_eq!(view.identity.as_deref(), Some("Ada"));
    }

    #[test]
    fn presents_the_identity_first() {
        let data = serde_json::to_value(parse("[user]\n\tname = Ada\n")).unwrap();

        let lines = GitconfigPresentation.present(&data);

        assert_eq!(lines[0], "Commits as: Ada");
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/gitconfig/repository.gitconfig");

        let data = GitconfigCore.view(&path).unwrap();
        let view: GitconfigView = serde_json::from_value(data).unwrap();

        assert!(view.remotes.len() >= 2);
        assert!(view.remotes.iter().any(|remote| remote.push_url.is_some()));
        assert!(view.branches.len() >= 3);
        assert!(view.identity.is_some());
        assert!(!view.aliases.is_empty());
        assert!(!view.includes.is_empty());
        assert!(view.sections.len() >= 5);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::GitconfigCore),
            plugin_api::PluginPresentation::extensions(&crate::GitconfigPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
