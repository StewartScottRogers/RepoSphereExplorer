//! Ruby gem file type plugin: core and presentation halves.
//!
//! A gem is a tar holding three gzip members: `metadata.gz`, the gem
//! specification; `data.tar.gz`, the files that get installed; and
//! `checksums.yaml.gz`, which ties the two together. Everything a reader
//! wants is in the specification, which `RubyGems` writes as a YAML dump of
//! its own objects - `!ruby/object:Gem::Dependency` and the like - so it
//! is read here by shape rather than handed to a general YAML reader that
//! would have to be taught those tags.

use flate2::read::GzDecoder;
use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::io::Read as _;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["gem"];

/// A tar header block, and the alignment every member starts on.
const BLOCK: usize = 512;

/// The member holding the gem specification.
const METADATA: &str = "metadata.gz";

/// The member holding the files that get installed.
const DATA: &str = "data.tar.gz";

/// How many file paths are listed before the rest are only counted.
const SHOWN: usize = 32;

/// One gem the specification says this gem needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    /// The gem's name.
    pub name: String,
    /// What versions of it will do, as the specification words it.
    pub requirement: String,
    /// Whether it is needed to *run* the gem (`runtime`) or only to work
    /// on it (`development`). Only the runtime ones are installed
    /// alongside the gem, which is the whole reason to tell them apart.
    pub kind: String,
}

/// View data produced by [`RubygemCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RubygemView {
    /// The gem's name.
    pub name: String,
    /// Its version.
    pub version: String,
    /// The one-line summary.
    pub summary: Option<String>,
    /// Who wrote it.
    pub authors: Vec<String>,
    /// The licences it is offered under.
    pub licences: Vec<String>,
    /// Where it lives.
    pub homepage: Option<String>,
    /// The gems it needs, runtime and development alike.
    pub dependencies: Vec<Dependency>,
    /// The Ruby it asks for.
    pub required_ruby_version: Option<String>,
    /// The commands it installs onto the path.
    pub executables: Vec<String>,
    /// The files the specification lists, up to [`SHOWN`].
    pub files: Vec<String>,
    /// How many files the specification lists.
    pub file_count: usize,
    /// How many are actually packed in `data.tar.gz`. A gem whose
    /// specification and payload disagree installs less than it claims.
    pub packed_files: usize,
    /// The three members a gem is made of, in the order they are packed.
    pub members: Vec<String>,
}

/// The name a tar header block gives, trimmed of its padding.
fn name_in(block: &[u8]) -> String {
    let end = block[..100]
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(100);
    String::from_utf8_lossy(&block[..end]).into_owned()
}

/// The size a tar header block states, which it writes in octal.
fn size_in(block: &[u8]) -> usize {
    let field = &block[124..136];
    let end = field
        .iter()
        .position(|byte| *byte == 0 || *byte == b' ')
        .unwrap_or(field.len());
    let text = String::from_utf8_lossy(&field[..end]);
    usize::from_str_radix(text.trim(), 8).unwrap_or(0)
}

/// Whether a tar header block's checksum is the one it states.
///
/// This is the only thing that makes a tar a tar: the format has no
/// magic bytes at offset zero, only a file name. The checksum is
/// computed with its own field read as spaces.
fn checksum_matches(block: &[u8]) -> bool {
    let stated = size_of_octal(&block[148..156]);
    let sum: u32 = block
        .iter()
        .enumerate()
        .map(|(at, byte)| {
            if (148..156).contains(&at) {
                u32::from(b' ')
            } else {
                u32::from(*byte)
            }
        })
        .sum();
    stated.is_some_and(|stated| stated == sum)
}

/// An octal field's value, or `None` when it is not one.
fn size_of_octal(field: &[u8]) -> Option<u32> {
    let end = field
        .iter()
        .position(|byte| *byte == 0 || *byte == b' ')
        .unwrap_or(field.len());
    u32::from_str_radix(String::from_utf8_lossy(&field[..end]).trim(), 8).ok()
}

/// Every member of the tar in `bytes`, as a name and its content.
fn members_of(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut members = Vec::new();
    let mut at = 0usize;
    while at + BLOCK <= bytes.len() {
        let block = &bytes[at..at + BLOCK];
        if block.iter().all(|byte| *byte == 0) {
            break;
        }
        let name = name_in(block);
        let size = size_in(block);
        at += BLOCK;
        let end = (at + size).min(bytes.len());
        if !name.is_empty() {
            members.push((name, bytes[at..end].to_vec()));
        }
        // Every member is padded out to the next block boundary.
        at = end.div_ceil(BLOCK) * BLOCK;
    }
    members
}

/// Whether `prefix` opens like a gem.
///
/// A gem is a tar, and a tar is only recognisable by a header block whose
/// checksum is right; what makes this one a gem is that its first member
/// is the specification.
fn looks_like_it(prefix: &[u8]) -> bool {
    prefix.len() >= BLOCK && checksum_matches(&prefix[..BLOCK]) && name_in(prefix) == METADATA
}

/// The bytes a gzip member decompresses to.
fn ungzip(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    GzDecoder::new(bytes).read_to_end(&mut out)?;
    Ok(out)
}

/// The specification split into its top-level blocks.
///
/// A block opens on a line whose first character is a lowercase letter -
/// every gemspec key is one - and runs until the next such line. What
/// follows a key may be indented or may be a sequence at column zero, and
/// both belong to the key above them.
fn blocks_in(specification: &str) -> Vec<(String, Vec<String>)> {
    let mut blocks: Vec<(String, Vec<String>)> = Vec::new();
    for line in specification.lines() {
        let opens = line
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase())
            && line.contains(':');
        if opens {
            let (key, rest) = line.split_once(':').unwrap_or((line, ""));
            blocks.push((key.to_owned(), vec![rest.trim().to_owned()]));
        } else if let Some((_, lines)) = blocks.last_mut() {
            lines.push(line.to_owned());
        }
    }
    blocks
}

/// The block named `wanted`, if the specification has one.
fn block<'a>(blocks: &'a [(String, Vec<String>)], wanted: &str) -> Option<&'a [String]> {
    blocks
        .iter()
        .find(|(key, _)| key == wanted)
        .map(|(_, lines)| lines.as_slice())
}

/// A block's value when it is a plain scalar on the key's own line.
fn scalar(blocks: &[(String, Vec<String>)], wanted: &str) -> Option<String> {
    block(blocks, wanted)
        .and_then(<[String]>::first)
        .filter(|value| !value.is_empty())
        .map(|value| unquote(value).to_owned())
}

/// A block's items when it is a sequence at column zero.
fn sequence(blocks: &[(String, Vec<String>)], wanted: &str) -> Vec<String> {
    block(blocks, wanted)
        .unwrap_or_default()
        .iter()
        .filter_map(|line| line.strip_prefix("- "))
        .map(|item| unquote(item).to_owned())
        .collect()
}

/// A value without the quotes YAML puts round anything that would
/// otherwise read as a number.
fn unquote(value: &str) -> &str {
    let value = value.trim();
    value
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
        .or_else(|| {
            value
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
        })
        .unwrap_or(value)
}

/// The version a `Gem::Version` block states, which sits on a nested
/// `version:` line.
fn nested_version(lines: &[String]) -> Option<String> {
    lines
        .iter()
        .filter_map(|line| line.trim().strip_prefix("version:"))
        .map(|value| unquote(value).to_owned())
        .next()
}

/// A requirement written out, e.g. `>= 3.1.0` or `~> 1.3`.
///
/// A `Gem::Requirement` is a list of operator-and-version pairs, written
/// as a nested sequence: the operator on a `- - ` line, the version on
/// the `version:` line of the `Gem::Version` beneath it.
fn requirement_in(lines: &[String]) -> String {
    let mut operators = Vec::new();
    let mut versions = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if let Some(operator) = trimmed.strip_prefix("- - ") {
            operators.push(unquote(operator).to_owned());
        } else if let Some(version) = trimmed.strip_prefix("version:") {
            versions.push(unquote(version).to_owned());
        }
    }
    operators
        .iter()
        .zip(&versions)
        .map(|(operator, version)| format!("{operator} {version}"))
        .collect::<Vec<String>>()
        .join(", ")
}

/// Every dependency the specification declares.
fn dependencies_in(lines: &[String]) -> Vec<Dependency> {
    let mut dependencies = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let finish = |chunk: &[String], dependencies: &mut Vec<Dependency>| {
        if chunk.is_empty() {
            return;
        }
        let name = chunk
            .iter()
            .filter_map(|line| line.trim().strip_prefix("name:"))
            .map(|value| unquote(value).to_owned())
            .next()
            .unwrap_or_default();
        let kind = chunk
            .iter()
            .filter_map(|line| line.trim().strip_prefix("type: :"))
            .map(str::trim)
            .next()
            .unwrap_or("runtime")
            .to_owned();
        if !name.is_empty() {
            dependencies.push(Dependency {
                name,
                requirement: requirement_in(chunk),
                kind,
            });
        }
    };
    for line in lines {
        if line.starts_with("- !ruby/object:Gem::Dependency") {
            finish(&current, &mut dependencies);
            current = Vec::new();
            continue;
        }
        current.push(line.clone());
    }
    finish(&current, &mut dependencies);
    dependencies
}

/// Everything [`RubygemView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<RubygemView> {
    let bytes = std::fs::read(path)?;
    if !looks_like_it(&bytes) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a gem: the first tar member is not metadata.gz",
        ));
    }
    let members = members_of(&bytes);
    let names: Vec<String> = members.iter().map(|(name, _)| name.clone()).collect();

    let specification = members
        .iter()
        .find(|(name, _)| name == METADATA)
        .map(|(_, body)| ungzip(body))
        .transpose()?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no metadata.gz"))?;
    let specification = String::from_utf8_lossy(&specification).into_owned();
    let blocks = blocks_in(&specification);

    let packed_files = members
        .iter()
        .find(|(name, _)| name == DATA)
        .map(|(_, body)| ungzip(body))
        .transpose()?
        .map_or(0, |data| {
            members_of(&data)
                .iter()
                .filter(|(name, _)| !name.ends_with('/'))
                .count()
        });

    let mut files = sequence(&blocks, "files");
    let file_count = files.len();
    files.truncate(SHOWN);

    Ok(RubygemView {
        name: scalar(&blocks, "name").unwrap_or_default(),
        version: block(&blocks, "version")
            .and_then(nested_version)
            .unwrap_or_default(),
        summary: scalar(&blocks, "summary"),
        authors: sequence(&blocks, "authors"),
        licences: sequence(&blocks, "licenses"),
        homepage: scalar(&blocks, "homepage"),
        dependencies: block(&blocks, "dependencies")
            .map(dependencies_in)
            .unwrap_or_default(),
        required_ruby_version: block(&blocks, "required_ruby_version")
            .map(requirement_in)
            .filter(|said| !said.is_empty()),
        executables: sequence(&blocks, "executables"),
        files,
        file_count,
        packed_files,
        members: names,
    })
}

/// The Ruby gem plugin's core half.
#[derive(Debug, Default)]
pub struct RubygemCore;

impl PluginCore for RubygemCore {
    fn name(&self) -> &'static str {
        "rubygem"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A gem is a tar, which `tar` recognises. This is the narrower
        // reading of the same bytes (D13).
        &["tar"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_it(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Ruby gem plugin's presentation half.
#[derive(Debug, Default)]
pub struct RubygemPresentation;

impl PluginPresentation for RubygemPresentation {
    fn name(&self) -> &'static str {
        "rubygem"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "GEM",
            tint: 0x00cc_342d,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: RubygemView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!("Ruby gem {} {}", view.name, view.version)];
        if let Some(summary) = &view.summary {
            lines.push(summary.clone());
        }
        if !view.authors.is_empty() {
            lines.push(format!("By {}", view.authors.join(", ")));
        }
        if !view.licences.is_empty() {
            lines.push(format!("Licence {}", view.licences.join(", ")));
        }
        if let Some(homepage) = &view.homepage {
            lines.push(homepage.clone());
        }
        if let Some(ruby) = &view.required_ruby_version {
            lines.push(format!("Needs Ruby {ruby}"));
        }
        let runtime: Vec<&Dependency> = view
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind == "runtime")
            .collect();
        let development: Vec<&Dependency> = view
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind != "runtime")
            .collect();
        if runtime.is_empty() {
            lines.push("Installs on its own: no runtime dependencies.".to_owned());
        } else {
            lines.push("Installed alongside it:".to_owned());
            for dependency in runtime {
                lines.push(format!("  {} {}", dependency.name, dependency.requirement));
            }
        }
        if !development.is_empty() {
            lines.push("Only needed to work on the gem, not to use it:".to_owned());
            for dependency in development {
                lines.push(format!("  {} {}", dependency.name, dependency.requirement));
            }
        }
        if !view.executables.is_empty() {
            lines.push(format!("Puts on the path: {}", view.executables.join(", ")));
        }
        lines.push(if view.file_count == view.packed_files {
            format!("{} file(s), all of them packed", view.file_count)
        } else {
            format!(
                "{} file(s) listed but {} packed, so it installs less than it claims",
                view.file_count, view.packed_files
            )
        });
        for file in &view.files {
            lines.push(format!("  {file}"));
        }
        if view.file_count > view.files.len() {
            lines.push(format!(
                "  ... and {} more",
                view.file_count - view.files.len()
            ));
        }
        lines.push(format!("Packed as {}", view.members.join(", ")));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RubygemCore, RubygemPresentation, RubygemView, blocks_in, looks_like_it, requirement_in,
    };
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/rubygem/csvstats-1.0.3.gem")
    }

    fn view_of() -> RubygemView {
        serde_json::from_value(RubygemCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&RubygemCore),
            PluginPresentation::extensions(&RubygemPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn sniffs_a_tar_whose_first_member_is_the_specification() {
        let bytes = std::fs::read(fixture()).unwrap();

        assert!(looks_like_it(&bytes));
        let mut wrong = bytes.clone();
        wrong[..8].copy_from_slice(b"other.gz");
        assert!(
            !looks_like_it(&wrong),
            "a tar whose first member is something else is not a gem"
        );
        assert!(!looks_like_it(b"PK\x03\x04"), "that is a zip");
        assert!(!looks_like_it(b""));
    }

    #[test]
    fn it_says_it_specialises_the_tar_reading() {
        assert_eq!(RubygemCore.specialises(), &["tar"]);
    }

    #[test]
    fn a_sequence_at_column_zero_still_belongs_to_the_key_above_it() {
        let blocks = blocks_in("name: a\nauthors:\n- one\n- two\nsummary: b\n");

        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[1].0, "authors");
        assert_eq!(blocks[1].1, vec!["", "- one", "- two"]);
    }

    #[test]
    fn a_requirement_pairs_each_operator_with_its_version() {
        let lines: Vec<String> = [
            "  requirements:",
            "  - - \">=\"",
            "    - !ruby/object:Gem::Version",
            "      version: 1.0",
            "  - - \"<\"",
            "    - !ruby/object:Gem::Version",
            "      version: '2.0'",
        ]
        .iter()
        .map(|line| (*line).to_owned())
        .collect();

        assert_eq!(requirement_in(&lines), ">= 1.0, < 2.0");
    }

    #[test]
    fn reads_the_name_version_and_summary() {
        let view = view_of();

        assert_eq!(view.name, "csvstats");
        assert_eq!(view.version, "1.0.3");
        assert_eq!(
            view.summary.as_deref(),
            Some("Summary statistics for a column of readings.")
        );
        assert_eq!(view.authors, vec!["The floor"]);
        assert_eq!(view.licences, vec!["MIT"]);
        assert_eq!(
            view.homepage.as_deref(),
            Some("https://example.com/floor/csvstats")
        );
    }

    #[test]
    fn tells_runtime_dependencies_from_development_ones() {
        let view = view_of();

        let runtime: Vec<&str> = view
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind == "runtime")
            .map(|dependency| dependency.name.as_str())
            .collect();
        let development: Vec<&str> = view
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind == "development")
            .map(|dependency| dependency.name.as_str())
            .collect();

        assert_eq!(runtime, vec!["thor", "rainbow"]);
        assert_eq!(development, vec!["rspec", "rubocop"]);
        assert_eq!(view.dependencies[0].requirement, "~> 1.3");
        assert_eq!(view.dependencies[1].requirement, ">= 3.1.1");
    }

    #[test]
    fn reads_the_ruby_it_needs_and_the_commands_it_installs() {
        let view = view_of();

        assert_eq!(view.required_ruby_version.as_deref(), Some(">= 3.1.0"));
        assert_eq!(view.executables, vec!["csvstats", "csvstats-report"]);
    }

    #[test]
    fn counts_the_files_the_specification_lists_and_the_ones_packed() {
        let view = view_of();

        assert_eq!(view.file_count, 5);
        assert_eq!(view.packed_files, 5);
        assert!(view.files.iter().any(|file| file == "lib/csvstats.rb"));
        assert_eq!(
            view.members,
            vec!["metadata.gz", "data.tar.gz", "checksums.yaml.gz"]
        );
    }

    #[test]
    fn presents_the_two_kinds_of_dependency_apart() {
        let data = RubygemCore.view(&fixture()).unwrap();

        let lines = RubygemPresentation.present(&data);

        assert!(lines[0].starts_with("Ruby gem csvstats 1.0.3"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Installed alongside it"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Only needed to work on the gem"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Needs Ruby >= 3.1.0"))
        );
        assert!(lines.iter().any(|line| line.contains("all of them packed")));
    }

    #[test]
    fn a_file_that_is_not_a_gem_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really-a.gem");
        std::fs::write(&path, b"nothing of the sort").unwrap();

        assert!(RubygemCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
