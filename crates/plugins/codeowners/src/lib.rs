//! `CODEOWNERS` file type plugin: core and presentation halves.
//!
//! Recognised by name and by shape: lines of `pattern @owner`, where an
//! owner is a user, a team or an e-mail address. The last matching rule
//! wins, which is the opposite of most ignore files and the thing readers
//! most often get wrong.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["codeowners"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One ownership rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// The path pattern.
    pub pattern: String,
    /// The `@user` owners.
    pub users: Vec<String>,
    /// The `@org/team` owners.
    pub teams: Vec<String>,
    /// The e-mail owners.
    pub emails: Vec<String>,
}

/// View data produced by [`CodeownersCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeownersView {
    /// Every rule, in file order. The **last** match wins.
    pub rules: Vec<Rule>,
    /// Every owner named anywhere, each once.
    pub owners: Vec<String>,
    /// The patterns with no owner at all, which remove ownership rather
    /// than granting it - and which are almost always a mistake.
    pub unowned: Vec<String>,
    /// How many comment lines the file carries.
    pub comments: usize,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Everything [`CodeownersView`] holds, read from `text`.
fn parse(text: &str) -> CodeownersView {
    let mut view = CodeownersView {
        rules: Vec::new(),
        owners: Vec::new(),
        unowned: Vec::new(),
        comments: 0,
        content: String::new(),
        truncated: false,
    };

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            view.comments += 1;
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(pattern) = parts.next() else {
            continue;
        };
        let mut rule = Rule {
            pattern: pattern.to_owned(),
            users: Vec::new(),
            teams: Vec::new(),
            emails: Vec::new(),
        };
        for owner in parts {
            if !view.owners.contains(&owner.to_owned()) {
                view.owners.push(owner.to_owned());
            }
            if let Some(handle) = owner.strip_prefix('@') {
                if handle.contains('/') {
                    rule.teams.push(owner.to_owned());
                } else {
                    rule.users.push(owner.to_owned());
                }
            } else if owner.contains('@') {
                rule.emails.push(owner.to_owned());
            }
        }
        if rule.users.is_empty() && rule.teams.is_empty() && rule.emails.is_empty() {
            view.unowned.push(rule.pattern.clone());
        }
        view.rules.push(rule);
    }
    view
}

/// Whether `text` looks like a `CODEOWNERS` file.
fn looks_like_it(text: &str) -> bool {
    let mut owned = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(_pattern) = parts.next() else {
            continue;
        };
        let owners: Vec<&str> = parts.collect();
        if owners.is_empty() {
            continue;
        }
        if owners
            .iter()
            .all(|owner| owner.starts_with('@') || owner.contains('@'))
        {
            owned += 1;
        } else {
            return false;
        }
    }
    owned >= 2
}

/// The `CODEOWNERS` plugin's core half.
#[derive(Debug, Default)]
pub struct CodeownersCore;

impl PluginCore for CodeownersCore {
    fn name(&self) -> &'static str {
        "codeowners"
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

/// The `CODEOWNERS` plugin's presentation half.
#[derive(Debug, Default)]
pub struct CodeownersPresentation;

impl PluginPresentation for CodeownersPresentation {
    fn name(&self) -> &'static str {
        "codeowners"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "OWN",
            tint: 0x0024_292e,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: CodeownersView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "{} rule(s). The last match wins, not the first.",
            view.rules.len()
        ));
        for rule in &view.rules {
            let mut owners = Vec::new();
            owners.extend(rule.teams.iter().cloned());
            owners.extend(rule.users.iter().cloned());
            owners.extend(rule.emails.iter().cloned());
            if owners.is_empty() {
                lines.push(format!("  {}  (nobody)", rule.pattern));
            } else {
                lines.push(format!("  {}  {}", rule.pattern, owners.join(" ")));
            }
        }
        lines.push(format!("Owners: {}", view.owners.join(", ")));
        lines.push(format!("Comments: {}", view.comments));
        if !view.unowned.is_empty() {
            lines.push(format!(
                "Patterns that remove ownership rather than granting it: {}",
                view.unowned.join(", ")
            ));
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{CodeownersCore, CodeownersPresentation, CodeownersView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    #[test]
    fn sniffs_patterns_followed_by_owners() {
        assert!(CodeownersCore.sniff(b"* @octocat\ndocs/ @org/writers\n"));
        assert!(CodeownersCore.sniff(b"*.rs @a\n*.go ada@example.com\n"));
    }

    #[test]
    fn does_not_claim_an_ignore_file_or_prose() {
        assert!(!CodeownersCore.sniff(b"target/\nnode_modules/\n"));
        assert!(!CodeownersCore.sniff(b"a sentence about @somebody here\n"));
        assert!(!CodeownersCore.sniff(b""));
    }

    #[test]
    fn tells_a_team_from_a_user_from_an_address() {
        let view = parse("* @octocat @org/writers ada@example.com\n");

        assert_eq!(view.rules[0].users, vec!["@octocat".to_owned()]);
        assert_eq!(view.rules[0].teams, vec!["@org/writers".to_owned()]);
        assert_eq!(view.rules[0].emails, vec!["ada@example.com".to_owned()]);
    }

    #[test]
    fn a_pattern_with_no_owner_removes_ownership() {
        let view = parse("* @octocat\nvendor/\n");

        assert_eq!(view.unowned, vec!["vendor/".to_owned()]);
        assert_eq!(view.rules.len(), 2);
    }

    #[test]
    fn lists_every_owner_once() {
        let view = parse("* @a\ndocs/ @a @b\n");

        assert_eq!(view.owners, vec!["@a".to_owned(), "@b".to_owned()]);
    }

    #[test]
    fn presents_the_last_match_rule_where_a_reader_will_see_it() {
        let data = serde_json::to_value(parse("* @a\ndocs/ @b\n")).unwrap();

        let lines = CodeownersPresentation.present(&data);

        assert!(lines[0].contains("last match wins"));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/codeowners/CODEOWNERS");

        let data = CodeownersCore.view(&path).unwrap();
        let view: CodeownersView = serde_json::from_value(data).unwrap();

        assert!(view.rules.len() >= 5);
        assert!(view.rules.iter().any(|rule| !rule.teams.is_empty()));
        assert!(view.rules.iter().any(|rule| !rule.users.is_empty()));
        assert!(view.rules.iter().any(|rule| !rule.emails.is_empty()));
        assert!(!view.unowned.is_empty());
        assert!(view.comments >= 2);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::CodeownersCore),
            plugin_api::PluginPresentation::extensions(&crate::CodeownersPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
