//! Go checksum database file type plugin: core and presentation halves.
//!
//! A `go.sum` is not a lock file, though it is often called one.
//! `go.mod` decides the versions; this records what the contents of
//! those versions hashed to the first time anybody fetched them, so a
//! later fetch that differs is caught.
//!
//! Two lines per module: one for the module's own content, one for its
//! `go.mod` alone - Go needs the second to work out the version graph
//! without downloading everything. A module with only the second is one
//! whose version graph was read and whose code was never needed.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["sum"];

/// How much of a checksum file is read. One of these grows to megabytes.
const READ_CAP: usize = 4 * 1024 * 1024;

/// How many modules are listed before the rest are only counted.
const SHOWN: usize = 64;

/// One module at one version.
#[expect(
    clippy::struct_excessive_bools,
    reason = "four independent facts about one module: which of the two hashes               are present, and which of the two kinds of unusual version it               is. None implies another, and folding them into an enum would               say a module cannot be both a pseudo-version and read only for               its graph, which it can."
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The module path.
    pub module: String,
    /// The version.
    pub version: String,
    /// Whether the module's own content is hashed here.
    pub content_hashed: bool,
    /// Whether its `go.mod` alone is.
    pub go_mod_hashed: bool,
    /// Whether the version is a pseudo-version: built from a date and a
    /// commit, because the module has no tag to point at.
    pub pseudo_version: bool,
    /// Whether it carries `+incompatible`: a major version past one
    /// that never adopted modules.
    pub incompatible: bool,
}

/// View data produced by [`GosumCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GosumView {
    /// Every module at every version recorded.
    pub entries: Vec<Entry>,
    /// How many there are in all.
    pub entry_count: usize,
    /// How many distinct modules that is.
    pub module_count: usize,
    /// The modules recorded at more than one version, which is what a
    /// reader chasing a duplicate is looking for.
    pub at_several_versions: Vec<String>,
    /// The modules whose code was never needed: only their `go.mod` is
    /// hashed, because Go read the version graph and stopped there.
    pub go_mod_only: Vec<String>,
    /// Whether the file was longer than this reads.
    pub truncated: bool,
}

/// Whether `text` reads like a Go checksum file.
///
/// The `h1:` prefix is the hash algorithm's name and appears on every
/// line; nothing else writes it.
fn looks_like_it(text: &str) -> bool {
    let mut lines = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() == 3 && parts[1].starts_with('v') && parts[2].starts_with("h1:") {
            lines += 1;
        } else {
            return false;
        }
        if lines >= 2 {
            return true;
        }
    }
    false
}

/// Whether a version is a pseudo-version: `v0.0.0-20190905194746-02993c407bfb`.
fn is_pseudo(version: &str) -> bool {
    let Some((_, rest)) = version.split_once('-') else {
        return false;
    };
    // A date of fourteen digits, then a twelve-character commit prefix.
    let mut pieces = rest.split('-');
    let (Some(stamp), Some(commit)) = (pieces.next_back(), pieces.next_back()) else {
        return false;
    };
    stamp.len() == 12
        && stamp.chars().all(|one| one.is_ascii_hexdigit())
        && commit.len() == 14
        && commit.chars().all(|one| one.is_ascii_digit())
}

/// Everything [`GosumView`] holds, read from `source`.
fn parse(source: &str, truncated: bool) -> GosumView {
    let mut entries: Vec<Entry> = Vec::new();
    for line in source.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let [module, version, _hash] = parts.as_slice() else {
            continue;
        };
        // `module v1.2.3/go.mod h1:…` hashes the `go.mod` alone.
        let (version, go_mod) = match version.strip_suffix("/go.mod") {
            Some(version) => (version, true),
            None => (*version, false),
        };
        match entries
            .iter_mut()
            .find(|entry| entry.module == *module && entry.version == version)
        {
            Some(entry) => {
                entry.content_hashed |= !go_mod;
                entry.go_mod_hashed |= go_mod;
            }
            None => entries.push(Entry {
                module: (*module).to_owned(),
                version: version.to_owned(),
                content_hashed: !go_mod,
                go_mod_hashed: go_mod,
                pseudo_version: is_pseudo(version),
                incompatible: version.ends_with("+incompatible"),
            }),
        }
    }

    let mut modules: Vec<&str> = entries.iter().map(|entry| entry.module.as_str()).collect();
    modules.sort_unstable();
    modules.dedup();
    let module_count = modules.len();

    let mut at_several_versions: Vec<String> = Vec::new();
    for entry in &entries {
        let versions = entries
            .iter()
            .filter(|other| other.module == entry.module)
            .count();
        if versions > 1 && !at_several_versions.contains(&entry.module) {
            at_several_versions.push(entry.module.clone());
        }
    }
    let go_mod_only: Vec<String> = entries
        .iter()
        .filter(|entry| entry.go_mod_hashed && !entry.content_hashed)
        .map(|entry| format!("{} {}", entry.module, entry.version))
        .collect();

    let entry_count = entries.len();
    entries.truncate(SHOWN);
    GosumView {
        entries,
        entry_count,
        module_count,
        at_several_versions,
        go_mod_only,
        truncated,
    }
}

/// Everything [`GosumView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<GosumView> {
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
            "not a Go checksum file",
        ));
    }
    Ok(parse(source, truncated))
}

/// The Go checksum database plugin's core half.
#[derive(Debug, Default)]
pub struct GosumCore;

impl PluginCore for GosumCore {
    fn name(&self) -> &'static str {
        "gosum"
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

/// The Go checksum database plugin's presentation half.
#[derive(Debug, Default)]
pub struct GosumPresentation;

impl PluginPresentation for GosumPresentation {
    fn name(&self) -> &'static str {
        "gosum"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "SUM",
            tint: 0x0000_add8,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: GosumView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "Go checksums: {} module(s) at {} version(s)",
            view.module_count, view.entry_count
        )];
        lines.push("Not a lock file: go.mod decides the versions, and this".to_owned());
        lines.push("records what they hashed to the first time.".to_owned());
        if view.truncated {
            lines.push("Longer than this reads; what follows is the start.".to_owned());
        }
        if view.at_several_versions.is_empty() {
            lines.push("No module appears at more than one version.".to_owned());
        } else {
            lines.push("At more than one version:".to_owned());
            for module in &view.at_several_versions {
                lines.push(format!("  {module}"));
            }
        }
        if !view.go_mod_only.is_empty() {
            lines.push("Read for its version graph, never downloaded:".to_owned());
            for module in &view.go_mod_only {
                lines.push(format!("  {module}"));
            }
        }
        lines.push("Modules:".to_owned());
        for entry in &view.entries {
            let mut said = format!("  {} {}", entry.module, entry.version);
            if entry.pseudo_version {
                said.push_str("  (a pseudo-version: no tag to point at)");
            }
            if entry.incompatible {
                said.push_str("  (a major version that never adopted modules)");
            }
            lines.push(said);
        }
        if view.entry_count > view.entries.len() {
            lines.push(format!(
                "  ... and {} more",
                view.entry_count - view.entries.len()
            ));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{GosumCore, GosumPresentation, GosumView, is_pseudo, looks_like_it};
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/gosum/go.sum")
    }

    fn view_of() -> GosumView {
        serde_json::from_value(GosumCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&GosumCore),
            PluginPresentation::extensions(&GosumPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn every_line_has_to_be_one_of_these() {
        assert!(looks_like_it("a v1.0.0 h1:x=\na v1.0.0/go.mod h1:y=\n"));
        assert!(
            !looks_like_it("a v1.0.0 h1:x=\nsomething else entirely\n"),
            "one line that is not a checksum means it is not this file"
        );
        assert!(!looks_like_it("a v1.0.0 h1:x=\n"), "one line is not enough");
        assert!(!looks_like_it(""));
    }

    #[test]
    fn a_pseudo_version_is_told_from_a_tag() {
        assert!(is_pseudo("v0.0.0-20190905194746-02993c407bfb"));
        assert!(!is_pseudo("v1.6.0"));
        assert!(!is_pseudo("v12.0.0+incompatible"));
        assert!(!is_pseudo("v1.0.0-rc1"));
    }

    #[test]
    fn the_two_lines_of_a_module_are_one_entry() {
        let view = view_of();

        assert_eq!(view.entry_count, 15);
        assert_eq!(view.module_count, 14);
        let spew = view
            .entries
            .iter()
            .find(|one| one.module.ends_with("go-spew"))
            .expect("the first module");
        assert!(spew.content_hashed);
        assert!(spew.go_mod_hashed, "both lines fold into one entry");
    }

    #[test]
    fn a_module_at_two_versions_is_called_out() {
        let view = view_of();

        assert_eq!(view.at_several_versions, vec!["github.com/google/go-cmp"]);
        assert_eq!(
            view.entries
                .iter()
                .filter(|one| one.module == "github.com/google/go-cmp")
                .count(),
            2
        );
    }

    #[test]
    fn a_module_read_for_its_graph_alone_is_called_out() {
        let view = view_of();

        assert_eq!(
            view.go_mod_only,
            vec!["github.com/google/go-cmp v0.5.9"],
            "the older version's go.mod was read and its code never fetched"
        );
    }

    #[test]
    fn reads_the_pseudo_version_and_the_incompatible_one() {
        let view = view_of();

        assert!(
            view.entries
                .iter()
                .any(|one| one.module.contains("xeipuuv") && one.pseudo_version)
        );
        assert!(
            view.entries
                .iter()
                .any(|one| one.module.contains("client-go") && one.incompatible)
        );
        assert_eq!(
            view.entries.iter().filter(|one| one.pseudo_version).count(),
            1
        );
    }

    #[test]
    fn presents_what_it_is_and_what_it_is_not() {
        let data = GosumCore.view(&fixture()).unwrap();

        let lines = GosumPresentation.present(&data);

        assert!(lines[0].starts_with("Go checksums: 14 module(s) at 15 version(s)"));
        assert!(lines.iter().any(|line| line.contains("Not a lock file")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Read for its version graph, never downloaded"))
        );
        assert!(lines.iter().any(|line| line.contains("a pseudo-version")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("never adopted modules"))
        );
    }

    #[test]
    fn a_file_that_is_not_a_checksum_file_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really-go.sum");
        std::fs::write(&path, b"nothing of the sort at all").unwrap();

        assert!(GosumCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
