//! Yarn lock file file type plugin: core and presentation halves.
//!
//! A Yarn lock file settles every requested range on one version. This
//! reads the generation that wrote it, each range against what it
//! resolved to, which packages are served through a local patch, and
//! which entries carry no integrity hash.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One resolved dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    /// The package's name, scope included.
    pub name: String,
    /// The range the dependant asked for.
    pub range: String,
    /// The single version the lock file settled on.
    pub version: String,
    /// Whether the entry carries an integrity hash.
    pub checksum: bool,
    /// Whether the entry is served through a local patch.
    pub patched: bool,
}

/// View data produced by [`YarnlockCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YarnlockView {
    /// Which generation of the format wrote this: `1` for the original,
    /// a metadata version for Berry.
    pub generation: String,
    /// Every requested range against the version it resolved to.
    pub entries: Vec<Resolution>,
    /// The packages served through a local patch, which is code that lives
    /// in this repository rather than in the registry.
    pub patched: Vec<String>,
    /// Entries with no integrity hash, so nothing checks what arrives.
    pub without_checksum: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// A block header, mid-read: the descriptors it names and what its body
/// has said so far.
#[derive(Default)]
struct Block {
    /// Every `name@range` the block heads, since one resolution can
    /// answer several requested ranges at once.
    descriptors: Vec<String>,
    /// The version its body resolved to.
    version: String,
    /// Whether its body carried an integrity hash.
    checksum: bool,
    /// Whether its body resolved through a patch.
    patched: bool,
}

/// `line` without the quotes a descriptor may be wrapped in.
fn unquoted(line: &str) -> &str {
    line.trim().trim_matches('"').trim_matches('\'')
}

/// The value on a `key value` or `key: value` body line.
fn value_of<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.trim().strip_prefix(key)?;
    let rest = rest.strip_prefix(':').unwrap_or(rest);
    if !rest.starts_with(' ') && !rest.is_empty() {
        // `versions:` is not `version`, and `checksumOf` is not `checksum`.
        return None;
    }
    Some(unquoted(rest))
}

/// Splits `descriptor` into its package name and its requested range.
///
/// The name may open with an `@` of its own - `@scope/package` - so the
/// separator is the *last* one, not the first.
fn name_and_range(descriptor: &str) -> (String, String) {
    match descriptor.rsplit_once('@') {
        Some((name, range)) if !name.is_empty() => (name.to_owned(), range.to_owned()),
        _ => (descriptor.to_owned(), String::new()),
    }
}

/// Adds `block` to `view`, once per descriptor it heads.
fn flush(block: &Block, view: &mut YarnlockView) {
    for descriptor in &block.descriptors {
        let (name, range) = name_and_range(descriptor);
        let label = format!("{name}@{range}");
        if block.patched && !view.patched.contains(&name) {
            view.patched.push(name.clone());
        }
        if !block.checksum {
            view.without_checksum.push(label);
        }
        view.entries.push(Resolution {
            name,
            range,
            version: block.version.clone(),
            checksum: block.checksum,
            patched: block.patched,
        });
    }
}

/// Everything [`YarnlockView`] holds, read from `text`.
fn parse(text: &str) -> YarnlockView {
    let mut view = YarnlockView {
        generation: "unstated".to_owned(),
        entries: Vec::new(),
        patched: Vec::new(),
        without_checksum: Vec::new(),
        truncated: false,
    };
    let mut block = Block::default();
    let mut in_metadata = false;

    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with('#') {
            if line.contains("yarn lockfile v") {
                "1".clone_into(&mut view.generation);
            }
            continue;
        }
        if !line.starts_with(' ') && !line.starts_with('\t') && line.trim_end().ends_with(':') {
            // A header closes whatever came before it.
            if !block.descriptors.is_empty() {
                flush(&block, &mut view);
            }
            block = Block::default();
            in_metadata = line.starts_with("__metadata");
            if in_metadata {
                continue;
            }
            let header = line.trim_end().trim_end_matches(':');
            block.descriptors = header
                .split(", ")
                .map(unquoted)
                .filter(|descriptor| !descriptor.is_empty())
                .map(str::to_owned)
                .collect();
            continue;
        }
        // A body line.
        if in_metadata {
            if let Some(version) = value_of(line, "version") {
                view.generation = format!("Berry, metadata version {version}");
            }
            continue;
        }
        if let Some(version) = value_of(line, "version") {
            version.clone_into(&mut block.version);
        } else if value_of(line, "integrity").is_some() || value_of(line, "checksum").is_some() {
            block.checksum = true;
        }
        if line.contains("patch:") {
            block.patched = true;
        }
    }
    if !block.descriptors.is_empty() {
        flush(&block, &mut view);
    }
    view
}

/// Whether `text` is a Yarn lock file.
fn looks_like_it(text: &str) -> bool {
    // Berry announces itself; the original writes a banner.
    let announced = text.contains("__metadata:") || text.contains("yarn lockfile v");
    if !announced {
        return false;
    }
    // And then it has to have resolved something, or it is a document
    // *about* a lock file rather than one.
    !parse(text).entries.is_empty()
}

/// The Yarn lock file plugin's core half.
#[derive(Debug, Default)]
pub struct YarnlockCore;

impl PluginCore for YarnlockCore {
    fn name(&self) -> &'static str {
        "yarnlock"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // The Berry form is YAML and `yaml` recognises it; this is the
        // narrower reading of the same bytes (D13).
        &["yaml"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        // A lock file is machine-written and long; what a reader wants
        // is the resolutions, not the ten thousand lines.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Yarn lock file plugin's presentation half.
#[derive(Debug, Default)]
pub struct YarnlockPresentation;

impl PluginPresentation for YarnlockPresentation {
    fn name(&self) -> &'static str {
        "yarnlock"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "YARN",
            tint: 0x002c_8ebb,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: YarnlockView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("Yarn lock file, generation {}", view.generation));
        lines.push(format!("{} resolution(s):", view.entries.len()));
        for entry in &view.entries {
            let version = if entry.version.is_empty() {
                "unresolved".to_owned()
            } else {
                entry.version.clone()
            };
            lines.push(format!(
                "  {} {} -> {version}{}",
                entry.name,
                if entry.range.is_empty() {
                    "(no range)"
                } else {
                    &entry.range
                },
                if entry.patched { " (patched)" } else { "" }
            ));
        }
        if !view.patched.is_empty() {
            lines.push("Served through a local patch, so the code that runs is not".to_owned());
            lines.push("what the registry publishes:".to_owned());
            for name in &view.patched {
                lines.push(format!("  {name}"));
            }
        }
        if !view.without_checksum.is_empty() {
            lines.push("No integrity hash, so nothing checks what arrives:".to_owned());
            for label in &view.without_checksum {
                lines.push(format!("  {label}"));
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
    use super::{YarnlockCore, YarnlockPresentation, YarnlockView, name_and_range, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const BERRY: &str = concat!(
        "__metadata:\n",
        "  version: 6\n",
        "  cacheKey: 8\n",
        "\n",
        "\"lodash@npm:^4.17.0, lodash@npm:^4.17.21\":\n",
        "  version: 4.17.21\n",
        "  resolution: \"lodash@npm:4.17.21\"\n",
        "  checksum: 10c0/aaaa\n",
        "  languageName: node\n",
        "  linkType: hard\n",
        "\n",
        "\"@scope/tool@npm:^2.0.0\":\n",
        "  version: 2.1.0\n",
        "  resolution: \"@scope/tool@npm:2.1.0\"\n",
        "  languageName: node\n",
    );

    const CLASSIC: &str = concat!(
        "# THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.\n",
        "# yarn lockfile v1\n",
        "\n",
        "lodash@^4.17.0:\n",
        "  version \"4.17.21\"\n",
        "  resolved \"https://registry.yarnpkg.com/lodash/-/lodash-4.17.21.tgz\"\n",
        "  integrity sha512-aaaa\n",
    );

    #[test]
    fn sniffs_both_generations() {
        assert!(YarnlockCore.sniff(BERRY.as_bytes()));
        assert!(YarnlockCore.sniff(CLASSIC.as_bytes()));
    }

    #[test]
    fn does_not_claim_a_document_that_only_mentions_one() {
        assert!(
            !YarnlockCore.sniff(b"# Notes\n\nWe deleted the yarn lockfile v1 banner by mistake.\n")
        );
        assert!(!YarnlockCore.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_yaml() {
        assert_eq!(YarnlockCore.specialises(), &["yaml"]);
    }

    #[test]
    fn reads_the_generation_from_the_metadata_block() {
        assert_eq!(parse(BERRY).generation, "Berry, metadata version 6");
        assert_eq!(parse(CLASSIC).generation, "1");
    }

    #[test]
    fn one_resolution_can_answer_several_requested_ranges() {
        let view = parse(BERRY);

        let lodash: Vec<_> = view
            .entries
            .iter()
            .filter(|entry| entry.name == "lodash")
            .collect();
        assert_eq!(lodash.len(), 2, "the header names two ranges, not one");
        assert!(lodash.iter().all(|entry| entry.version == "4.17.21"));
        assert_eq!(lodash[0].range, "npm:^4.17.0");
    }

    #[test]
    fn a_scoped_name_keeps_its_scope() {
        assert_eq!(
            name_and_range("@scope/tool@npm:^2.0.0"),
            ("@scope/tool".to_owned(), "npm:^2.0.0".to_owned()),
            "splitting on the first @ would leave the package nameless"
        );
    }

    #[test]
    fn names_the_entries_with_no_integrity_hash() {
        let view = parse(BERRY);

        assert_eq!(
            view.without_checksum,
            vec!["@scope/tool@npm:^2.0.0".to_owned()]
        );
    }

    #[test]
    fn a_body_key_is_not_a_prefix_of_another() {
        let view = parse(concat!(
            "__metadata:\n  version: 6\n\n",
            "\"a@npm:1\":\n  versionRange: nonsense\n  checksumOf: nonsense\n",
        ));

        assert_eq!(
            view.entries[0].version, "",
            "`versionRange` is not `version`"
        );
        assert!(!view.entries[0].checksum, "`checksumOf` is not `checksum`");
    }

    #[test]
    fn presents_the_patch_warning_with_its_reason() {
        let view = parse(concat!(
            "__metadata:\n  version: 6\n\n",
            "\"left-pad@patch:left-pad@npm%3A1.3.0#./.yarn/patches/left-pad.patch\":\n",
            "  version: 1.3.0\n",
            "  resolution: \"left-pad@patch:left-pad@npm%3A1.3.0#./.yarn/patches/x.patch\"\n",
            "  checksum: 10c0/bbbb\n",
        ));
        let data = serde_json::to_value(&view).unwrap();

        assert_eq!(view.patched.len(), 1);
        let lines = YarnlockPresentation.present(&data);
        assert!(lines.iter().any(|line| line.contains("not")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("what the registry publishes"))
        );
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/yarnlock/yarn.lock");

        let data = YarnlockCore.view(&path).unwrap();
        let view: YarnlockView = serde_json::from_value(data).unwrap();

        assert!(view.generation.starts_with("Berry"));
        assert!(view.entries.len() >= 5);
        assert!(view.entries.iter().any(|entry| entry.checksum));
        assert!(view.entries.iter().any(|entry| entry.patched));
        assert!(!view.patched.is_empty());
        assert!(!view.without_checksum.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::YarnlockCore),
            plugin_api::PluginPresentation::extensions(&crate::YarnlockPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
