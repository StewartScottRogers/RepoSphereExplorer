//! Bundler lock file file type plugin: core and presentation halves.
//!
//! A `Gemfile.lock` records what Bundler resolved, which is not what
//! the `Gemfile` asked for. Its sections are the point: `GEM` is what
//! came from a registry, `PATH` a gem vendored into the repository, and
//! `GIT` one pinned to a commit. A reader chasing "where did this
//! version come from" needs those told apart.
//!
//! `DEPENDENCIES` is the direct list. Everything resolved that is not
//! in it arrived transitively, and saying which is which is most of
//! what makes a lock file readable.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
///
/// Deliberately empty. The file is named `Gemfile.lock`, and `lock`
/// belongs to the Cargo lock plugin; the sections decide this one.
pub const EXTENSIONS: &[&str] = &[];

/// How much of a lock file is read.
const READ_CAP: usize = 2 * 1024 * 1024;

/// How many gems are listed before the rest are only counted.
const SHOWN: usize = 64;

/// One resolved gem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gem {
    /// Its name.
    pub name: String,
    /// The version resolved.
    pub version: String,
    /// Which section it came from, and from where.
    pub source: String,
    /// Whether the `Gemfile` named it, rather than something else
    /// pulling it in.
    pub direct: bool,
}

/// View data produced by [`GemfilelockCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GemfilelockView {
    /// Where the gems came from, one line each.
    pub sources: Vec<String>,
    /// The gems, in the order they are recorded.
    pub gems: Vec<Gem>,
    /// How many there are in all.
    pub gem_count: usize,
    /// How many the `Gemfile` named itself.
    pub direct_count: usize,
    /// The platforms the resolution covers.
    pub platforms: Vec<String>,
    /// The Ruby it records, when it records one.
    pub ruby_version: Option<String>,
    /// The Bundler that wrote it.
    pub bundled_with: Option<String>,
    /// Whether the file was longer than this reads.
    pub truncated: bool,
}

/// The section headings a lock file uses.
const SECTIONS: &[&str] = &[
    "GEM",
    "PATH",
    "GIT",
    "PLATFORMS",
    "DEPENDENCIES",
    "RUBY VERSION",
    "BUNDLED WITH",
    "CHECKSUMS",
];

/// Whether `text` reads like a Bundler lock file.
fn looks_like_it(text: &str) -> bool {
    let mut headings = 0usize;
    let mut specs = false;
    for line in text.lines() {
        if SECTIONS.contains(&line.trim_end()) {
            headings += 1;
        }
        if line.trim() == "specs:" {
            specs = true;
        }
        if headings >= 2 && specs {
            return true;
        }
    }
    false
}

/// A gem name and version from a `name (version)` line.
fn name_and_version(line: &str) -> Option<(String, String)> {
    let (name, rest) = line.trim().split_once(" (")?;
    let version = rest.strip_suffix(')')?;
    Some((name.to_owned(), version.to_owned()))
}

/// Everything [`GemfilelockView`] holds, read from `source`.
fn parse(source: &str, truncated: bool) -> GemfilelockView {
    let mut view = GemfilelockView {
        sources: Vec::new(),
        gems: Vec::new(),
        gem_count: 0,
        direct_count: 0,
        platforms: Vec::new(),
        ruby_version: None,
        bundled_with: None,
        truncated,
    };
    let mut section = String::new();
    let mut source_line = String::new();
    // A `GIT` section states its remote, then its revision, then its
    // branch, each on its own line. Keeping only the last leaves a
    // reader looking at `GIT branch: main`, which does not say where
    // the gem came from - so they are collected and composed.
    let mut remote = String::new();
    let mut revision = String::new();
    let mut in_specs = false;
    let mut direct: Vec<String> = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        if SECTIONS.contains(&trimmed) {
            trimmed.clone_into(&mut section);
            trimmed.clone_into(&mut source_line);
            remote.clear();
            revision.clear();
            in_specs = false;
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let body = line.trim();
        match section.as_str() {
            "GEM" | "PATH" | "GIT" => {
                if body == "specs:" {
                    in_specs = true;
                    source_line = match (remote.is_empty(), revision.is_empty()) {
                        (false, false) => format!("{section} {remote} at {revision}"),
                        (false, true) => format!("{section} {remote}"),
                        _ => section.clone(),
                    };
                    if !view.sources.contains(&source_line) {
                        view.sources.push(source_line.clone());
                    }
                    continue;
                }
                if !in_specs {
                    if let Some(said) = body.strip_prefix("remote:") {
                        said.trim().clone_into(&mut remote);
                    } else if let Some(said) = body.strip_prefix("revision:") {
                        said.trim().clone_into(&mut revision);
                    }
                    continue;
                }
                // A gem sits at four spaces; its own dependencies at six.
                if indent == 4
                    && let Some((name, version)) = name_and_version(body)
                {
                    view.gem_count += 1;
                    if view.gems.len() < SHOWN {
                        view.gems.push(Gem {
                            name,
                            version,
                            source: source_line.clone(),
                            direct: false,
                        });
                    }
                }
            }
            "PLATFORMS" => view.platforms.push(body.to_owned()),
            "DEPENDENCIES" => {
                // A `!` marks one that came from somewhere other than
                // the default source, and is not part of the name.
                let name = body
                    .split([' ', '('])
                    .next()
                    .unwrap_or(body)
                    .trim_end_matches('!')
                    .to_owned();
                direct.push(name);
            }
            "RUBY VERSION" => view.ruby_version = Some(body.to_owned()),
            "BUNDLED WITH" => view.bundled_with = Some(body.to_owned()),
            _ => {}
        }
    }
    for gem in &mut view.gems {
        gem.direct = direct.contains(&gem.name);
    }
    view.direct_count = direct.len();
    view
}

/// Everything [`GemfilelockView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<GemfilelockView> {
    let source = std::fs::read_to_string(path)?;
    let truncated = source.len() > READ_CAP;
    let source = if truncated {
        let mut end = READ_CAP;
        while end > 0 && !source.is_char_boundary(end) {
            end -= 1;
        }
        &source[..end]
    } else {
        source.as_str()
    };
    if !looks_like_it(source) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a Bundler lock file",
        ));
    }
    Ok(parse(source, truncated))
}

/// The Bundler lock file plugin's core half.
#[derive(Debug, Default)]
pub struct GemfilelockCore;

impl PluginCore for GemfilelockCore {
    fn name(&self) -> &'static str {
        "gemfilelock"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // It is text, and the text plugin recognises any of it. This is
        // the narrower reading of the same bytes (D13).
        &["text"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Bundler lock file plugin's presentation half.
#[derive(Debug, Default)]
pub struct GemfilelockPresentation;

impl PluginPresentation for GemfilelockPresentation {
    fn name(&self) -> &'static str {
        "gemfilelock"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "LOCK",
            tint: 0x00cc_342d,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: GemfilelockView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "Bundler lock: {} gem(s) resolved from {} the Gemfile named",
            view.gem_count, view.direct_count
        )];
        if view.truncated {
            lines.push("Longer than this reads; what follows is the start.".to_owned());
        }
        if let Some(bundler) = &view.bundled_with {
            lines.push(format!("Written by Bundler {bundler}"));
        }
        if let Some(ruby) = &view.ruby_version {
            lines.push(format!("Resolved for {ruby}"));
        }
        if !view.platforms.is_empty() {
            lines.push(format!("Platforms: {}", view.platforms.join(", ")));
        }
        if !view.sources.is_empty() {
            lines.push("Sources:".to_owned());
            for source in &view.sources {
                lines.push(format!("  {source}"));
            }
        }
        lines.push("Gems, with the ones the Gemfile named marked:".to_owned());
        for gem in &view.gems {
            let direct = if gem.direct { "*" } else { " " };
            lines.push(format!("  {direct} {} {}", gem.name, gem.version));
            if !gem.source.starts_with("GEM") {
                lines.push(format!("      from {}", gem.source));
            }
        }
        if view.gem_count > view.gems.len() {
            lines.push(format!(
                "  ... and {} more",
                view.gem_count - view.gems.len()
            ));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GemfilelockCore, GemfilelockPresentation, GemfilelockView, looks_like_it, name_and_version,
    };
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/gemfilelock/Gemfile.lock")
    }

    fn view_of() -> GemfilelockView {
        serde_json::from_value(GemfilelockCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&GemfilelockCore),
            PluginPresentation::extensions(&GemfilelockPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn it_claims_no_extension_because_the_cargo_lock_plugin_has_that_one() {
        assert!(PluginCore::extensions(&GemfilelockCore).is_empty());
        assert_eq!(GemfilelockCore.specialises(), &["text"]);
    }

    #[test]
    fn recognises_the_sections() {
        assert!(looks_like_it(
            "GEM\n  remote: https://rubygems.org/\n  specs:\n    a (1.0)\n\nDEPENDENCIES\n  a\n"
        ));
        assert!(
            !looks_like_it("GEM\n  something\n"),
            "one heading and no specs is not a lock file"
        );
        assert!(!looks_like_it(""));
    }

    #[test]
    fn a_name_and_version_come_off_one_line() {
        assert_eq!(
            name_and_version("    rubocop (1.65.1)"),
            Some(("rubocop".to_owned(), "1.65.1".to_owned()))
        );
        assert_eq!(
            name_and_version("      json (~> 2.3)"),
            Some(("json".to_owned(), "~> 2.3".to_owned()))
        );
        assert_eq!(name_and_version("PLATFORMS"), None);
    }

    #[test]
    fn reads_every_gem_once() {
        let view = view_of();

        assert_eq!(view.gem_count, 23, "a gem's own dependencies are not gems");
        assert!(view.gems.iter().any(|one| one.name == "rubocop"));
        assert_eq!(
            view.gems.iter().filter(|one| one.name == "ast").count(),
            1,
            "`ast` is listed once as a gem and again under `parser`"
        );
    }

    #[test]
    fn keeps_the_three_kinds_of_source_apart() {
        let view = view_of();

        let core = view
            .gems
            .iter()
            .find(|one| one.name == "readings-core")
            .expect("the vendored gem");
        assert!(core.source.starts_with("PATH"), "{}", core.source);
        assert!(core.source.contains("vendor/readings-core"));

        let fast = view
            .gems
            .iter()
            .find(|one| one.name == "csv-fast")
            .expect("the git gem");
        assert_eq!(
            fast.source,
            concat!(
                "GIT https://github.com/example/csv-fast.git",
                " at 9f3c1b2a4d5e6f708192a3b4c5d6e7f809a1b2c3"
            ),
            "a git section states its remote, then its revision, then its \
             branch, and keeping only the last says nothing about where the \
             gem came from"
        );

        let thor = view
            .gems
            .iter()
            .find(|one| one.name == "thor")
            .expect("a registry gem");
        assert!(thor.source.starts_with("GEM"), "{}", thor.source);
    }

    #[test]
    fn tells_a_direct_dependency_from_a_transitive_one() {
        let view = view_of();

        assert_eq!(view.direct_count, 5);
        let direct: Vec<&str> = view
            .gems
            .iter()
            .filter(|one| one.direct)
            .map(|one| one.name.as_str())
            .collect();
        assert_eq!(
            direct,
            vec!["readings-core", "csv-fast", "rspec", "rubocop", "thor"],
            "the `!` marking a non-default source is not part of the name"
        );
        assert!(
            view.gems
                .iter()
                .find(|one| one.name == "diff-lcs")
                .is_some_and(|one| !one.direct),
            "nothing named it; rspec pulled it in"
        );
    }

    #[test]
    fn reads_the_platforms_the_ruby_and_the_bundler() {
        let view = view_of();

        assert_eq!(
            view.platforms,
            vec!["arm64-darwin-23", "ruby", "x86_64-linux"]
        );
        assert_eq!(view.ruby_version.as_deref(), Some("ruby 3.3.4p94"));
        assert_eq!(view.bundled_with.as_deref(), Some("2.5.16"));
    }

    #[test]
    fn presents_which_gems_were_asked_for() {
        let data = GemfilelockCore.view(&fixture()).unwrap();

        let lines = GemfilelockPresentation.present(&data);

        assert!(lines[0].starts_with("Bundler lock: 23 gem(s) resolved from 5"));
        assert!(lines.iter().any(|line| line.contains("* rubocop 1.65.1")));
        assert!(lines.iter().any(|line| line.contains("  diff-lcs 1.5.1")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("from PATH vendor/readings-core"))
        );
    }

    #[test]
    fn a_file_that_is_not_a_lock_file_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really-Gemfile.lock");
        std::fs::write(&path, b"nothing of the sort at all").unwrap();

        assert!(GemfilelockCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
