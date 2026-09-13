//! Python wheel file type plugin: core and presentation halves.
//!
//! A wheel is a zip that installs by being unpacked - no build step, no
//! code run on the way in. What it can be unpacked *into* is decided by
//! the compatibility tags, and those are in the file name rather than
//! inside the archive: `csvstats-1.0.3-py3-none-any.whl` says any Python
//! 3, no application binary interface (ABI), any platform. A wheel that
//! will not install is usually one whose tags do not match, so this
//! reads the name as carefully as it reads the metadata.

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
pub const EXTENSIONS: &[&str] = &["whl"];

/// The four bytes a zip's first local header opens with.
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// The entry that makes a zip a wheel, under a `*.dist-info/` directory.
const WHEEL_MARKER: &[u8] = b".dist-info/WHEEL";

/// How many requirements are listed before the rest are only counted.
const SHOWN: usize = 32;

/// One entry point: a name the installer wires up to something in the
/// distribution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryPoint {
    /// The group it belongs to. `console_scripts` becomes a command on
    /// the path; anything else is a plugin hook something looks up.
    pub group: String,
    /// The name it is registered under.
    pub name: String,
    /// What it points at, as `module:object`.
    pub target: String,
}

/// View data produced by [`WheelCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WheelView {
    /// The distribution's name, from the `.dist-info` directory.
    pub distribution: String,
    /// Its version, likewise.
    pub version: String,
    /// The wheel format version, from `WHEEL`'s `Wheel-Version`.
    pub wheel_version: Option<String>,
    /// What built it, from `WHEEL`'s `Generator`.
    pub generator: Option<String>,
    /// Whether it is pure Python, from `Root-Is-Purelib`. A pure wheel
    /// installs the same way everywhere; an impure one carries compiled
    /// code and only fits the platform it was built for.
    pub pure: Option<bool>,
    /// The compatibility tags from the file name: Python, application
    /// binary interface, platform.
    pub tags: Vec<String>,
    /// The Python the metadata asks for, from `Requires-Python`.
    pub requires_python: Option<String>,
    /// The distributions it needs, in metadata order, up to [`SHOWN`].
    pub requirements: Vec<String>,
    /// How many requirements there are in total.
    pub requirement_count: usize,
    /// The optional feature sets, from `Provides-Extra`. A requirement
    /// marked with one is only installed when the extra is asked for.
    pub extras: Vec<String>,
    /// The entry points the installer wires up.
    pub entry_points: Vec<EntryPoint>,
    /// The packages the wheel unpacks at the top level.
    pub top_level: Vec<String>,
    /// How many entries the zip holds.
    pub entries: usize,
}

/// Whether `haystack` holds `needle` anywhere as a contiguous byte run.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Whether `prefix` opens like a wheel.
///
/// A wheel is a zip, so the signature settles nothing; the `WHEEL` entry
/// under a `.dist-info` directory is the marker. A wheel large enough to
/// push its metadata past the sniffed prefix reads as a plain zip, which
/// is the honest answer for bytes that have not said otherwise yet.
fn looks_like_it(prefix: &[u8]) -> bool {
    prefix.starts_with(ZIP_MAGIC) && contains_bytes(prefix, WHEEL_MARKER)
}

/// The attributes of a metadata file, in file order, unfolding any
/// continuation line - a line opening with whitespace continues the one
/// before it.
fn fields_in(text: &str) -> Vec<(String, String)> {
    let mut joined: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            // The body follows the blank line, and it is prose.
            break;
        }
        if line.starts_with([' ', '\t'])
            && let Some(last) = joined.last_mut()
        {
            last.push('\n');
            last.push_str(line.trim());
            continue;
        }
        joined.push(line.to_owned());
    }
    joined
        .iter()
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

/// The first value of the field named `wanted`.
fn field<'a>(fields: &'a [(String, String)], wanted: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
        .map(|(_, value)| value.as_str())
}

/// Every value of the field named `wanted`, which metadata may repeat.
fn every_field(fields: &[(String, String)], wanted: &str) -> Vec<String> {
    fields
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(wanted))
        .map(|(_, value)| value.clone())
        .collect()
}

/// The entry points an `entry_points.txt` declares. It is an INI file:
/// `[group]` headings, then `name = target` under each.
fn entry_points_in(text: &str) -> Vec<EntryPoint> {
    let mut group = String::new();
    let mut points = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(heading) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            heading.clone_into(&mut group);
            continue;
        }
        if let Some((name, target)) = line.split_once('=') {
            points.push(EntryPoint {
                group: group.clone(),
                name: name.trim().to_owned(),
                target: target.trim().to_owned(),
            });
        }
    }
    points
}

/// The compatibility tags a wheel's file name carries.
///
/// The name is `distribution-version(-build)?-python-abi-platform.whl`,
/// and each of the last three may be several tags joined by dots: one
/// wheel can be built for more than one Python at once.
fn tags_in(file_name: &str) -> Vec<String> {
    let stem = file_name.trim_end_matches(".whl");
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() < 5 {
        return Vec::new();
    }
    parts[parts.len() - 3..]
        .iter()
        .map(|part| (*part).to_owned())
        .collect()
}

/// The contents of `name` in `archive`, as text.
fn text_of<R>(archive: &mut zip::ZipArchive<R>, name: &str) -> Option<String>
where
    R: io::Read + io::Seek,
{
    let mut entry = archive.by_name(name).ok()?;
    let mut text = String::new();
    entry.read_to_string(&mut text).ok()?;
    Some(text)
}

/// Everything [`WheelView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<WheelView> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let names: Vec<String> = archive.file_names().map(ToOwned::to_owned).collect();

    let Some(dist_info) = names
        .iter()
        .filter_map(|name| name.split_once(".dist-info/"))
        .map(|(before, _)| before.to_owned())
        .next()
    else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no .dist-info directory, so this is a zip and not a wheel",
        ));
    };
    let (distribution, version) = dist_info
        .rsplit_once('-')
        .map_or((dist_info.clone(), String::new()), |(name, version)| {
            (name.to_owned(), version.to_owned())
        });

    let wheel =
        text_of(&mut archive, &format!("{dist_info}.dist-info/WHEEL")).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "no WHEEL file, so this is a zip and not a wheel",
            )
        })?;
    let wheel = fields_in(&wheel);
    let metadata = text_of(&mut archive, &format!("{dist_info}.dist-info/METADATA"))
        .map(|text| fields_in(&text))
        .unwrap_or_default();
    let entry_points = text_of(
        &mut archive,
        &format!("{dist_info}.dist-info/entry_points.txt"),
    )
    .as_deref()
    .map(entry_points_in)
    .unwrap_or_default();

    let mut requirements = every_field(&metadata, "Requires-Dist");
    let requirement_count = requirements.len();
    requirements.truncate(SHOWN);

    // What the wheel unpacks at the top level: the first path segment of
    // every entry outside the metadata directory.
    let mut top_level: Vec<String> = Vec::new();
    for name in &names {
        if name.starts_with(&format!("{dist_info}.dist-info/")) || name.contains(".data/") {
            continue;
        }
        let head = name.split_once('/').map_or(name.as_str(), |(head, _)| head);
        if !head.is_empty() && !top_level.iter().any(|had| had == head) {
            top_level.push(head.to_owned());
        }
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    Ok(WheelView {
        distribution,
        version,
        wheel_version: field(&wheel, "Wheel-Version").map(ToOwned::to_owned),
        generator: field(&wheel, "Generator").map(ToOwned::to_owned),
        pure: field(&wheel, "Root-Is-Purelib").map(|said| said.eq_ignore_ascii_case("true")),
        // The tags inside `WHEEL` and the tags in the name say the same
        // thing, and the name is what an installer actually reads.
        tags: if file_name.is_empty() {
            every_field(&wheel, "Tag")
        } else {
            tags_in(file_name)
        },
        requires_python: field(&metadata, "Requires-Python").map(ToOwned::to_owned),
        requirements,
        requirement_count,
        extras: every_field(&metadata, "Provides-Extra"),
        entry_points,
        top_level,
        entries: names.len(),
    })
}

/// The Python wheel plugin's core half.
#[derive(Debug, Default)]
pub struct WheelCore;

impl PluginCore for WheelCore {
    fn name(&self) -> &'static str {
        "wheel"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A wheel is a zip, which `archive` recognises. This is the
        // narrower reading of the same bytes (D13).
        &["archive"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_it(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Python wheel plugin's presentation half.
#[derive(Debug, Default)]
pub struct WheelPresentation;

impl PluginPresentation for WheelPresentation {
    fn name(&self) -> &'static str {
        "wheel"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "WHL",
            tint: 0x0030_6998,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: WheelView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "Python wheel {} {}, {} entry(ies)",
            view.distribution, view.version, view.entries
        )];
        if view.tags.len() == 3 {
            lines.push(format!(
                "Installs into Python {}, ABI {}, platform {}",
                view.tags[0], view.tags[1], view.tags[2]
            ));
        } else if !view.tags.is_empty() {
            lines.push(format!("Tags: {}", view.tags.join(", ")));
        }
        lines.push(match view.pure {
            Some(true) => "Pure Python: it installs the same way everywhere.".to_owned(),
            Some(false) => "Not pure Python: it carries compiled code.".to_owned(),
            None => "The WHEEL file does not say whether it is pure Python.".to_owned(),
        });
        if let Some(python) = &view.requires_python {
            lines.push(format!("Needs Python {python}"));
        }
        if let Some(version) = &view.wheel_version {
            lines.push(format!("Wheel format {version}"));
        }
        if let Some(generator) = &view.generator {
            lines.push(format!("Built by {generator}"));
        }
        if view.requirements.is_empty() {
            lines.push("Needs nothing else installed.".to_owned());
        } else {
            lines.push("Requires:".to_owned());
            for requirement in &view.requirements {
                lines.push(format!("  {requirement}"));
            }
            if view.requirement_count > view.requirements.len() {
                lines.push(format!(
                    "  ... and {} more",
                    view.requirement_count - view.requirements.len()
                ));
            }
        }
        if !view.extras.is_empty() {
            lines.push(format!(
                "Optional extras, installed only when asked for: {}",
                view.extras.join(", ")
            ));
        }
        for point in &view.entry_points {
            let what = if point.group == "console_scripts" {
                "command"
            } else {
                "plugin"
            };
            lines.push(format!(
                "  {what} {} -> {} ({})",
                point.name, point.target, point.group
            ));
        }
        if !view.top_level.is_empty() {
            lines.push(format!("Unpacks {}", view.top_level.join(", ")));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{WheelCore, WheelPresentation, WheelView, entry_points_in, looks_like_it, tags_in};
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/wheel/csvstats-1.0.3-py3-none-any.whl")
    }

    fn view_of() -> WheelView {
        serde_json::from_value(WheelCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&WheelCore),
            PluginPresentation::extensions(&WheelPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn sniffs_a_zip_holding_a_dist_info_wheel_file() {
        assert!(looks_like_it(
            b"PK\x03\x04..csvstats-1.0.3.dist-info/WHEEL.."
        ));
        assert!(
            !looks_like_it(b"PK\x03\x04..csvstats/__init__.py.."),
            "a zip of Python files is not a wheel"
        );
        assert!(!looks_like_it(b".dist-info/WHEEL"), "not a zip at all");
        assert!(!looks_like_it(b""));
    }

    #[test]
    fn it_says_it_specialises_the_archive_reading() {
        assert_eq!(WheelCore.specialises(), &["archive"]);
    }

    #[test]
    fn the_tags_come_from_the_file_name() {
        assert_eq!(
            tags_in("csvstats-1.0.3-py3-none-any.whl"),
            vec!["py3", "none", "any"]
        );
        assert_eq!(
            tags_in("numpy-2.1.0-cp312-cp312-win_amd64.whl"),
            vec!["cp312", "cp312", "win_amd64"],
            "a compiled wheel names the Python, the ABI and the platform"
        );
        assert_eq!(
            tags_in("thing-1.0-1-py3-none-any.whl"),
            vec!["py3", "none", "any"],
            "an optional build number sits between the version and the tags"
        );
        assert!(tags_in("not-a-wheel.whl").is_empty());
    }

    #[test]
    fn entry_points_keep_the_group_they_were_declared_under() {
        let points = entry_points_in("[console_scripts]\na = m:f\n\n[other.group]\nb = m:g\n");

        assert_eq!(points[0].group, "console_scripts");
        assert_eq!(points[0].name, "a");
        assert_eq!(points[0].target, "m:f");
        assert_eq!(points[1].group, "other.group");
    }

    #[test]
    fn reads_the_distribution_and_its_version() {
        let view = view_of();

        assert_eq!(view.distribution, "csvstats");
        assert_eq!(view.version, "1.0.3");
        assert!(view.entries >= 7);
    }

    #[test]
    fn reads_the_wheel_file() {
        let view = view_of();

        assert_eq!(view.wheel_version.as_deref(), Some("1.0"));
        assert_eq!(view.generator.as_deref(), Some("hatchling 1.25.0"));
        assert_eq!(view.pure, Some(true));
        assert_eq!(view.tags, vec!["py3", "none", "any"]);
    }

    #[test]
    fn reads_the_requirements_the_python_and_the_extras() {
        let view = view_of();

        assert_eq!(view.requires_python.as_deref(), Some(">=3.9"));
        assert_eq!(view.requirement_count, 4);
        assert!(
            view.requirements
                .iter()
                .any(|line| line.starts_with("click"))
        );
        assert!(
            view.requirements
                .iter()
                .any(|line| line.contains("extra == \"test\"")),
            "a requirement carries the marker that gates it"
        );
        assert_eq!(view.extras, vec!["test", "docs"]);
    }

    #[test]
    fn reads_the_entry_points_and_the_top_level_packages() {
        let view = view_of();

        assert_eq!(view.entry_points.len(), 4);
        assert!(
            view.entry_points
                .iter()
                .any(|point| point.group == "console_scripts" && point.name == "csvstats")
        );
        assert!(
            view.entry_points
                .iter()
                .any(|point| point.group == "csvstats.readers")
        );
        assert_eq!(view.top_level, vec!["csvstats"]);
    }

    #[test]
    fn presents_the_tags_and_what_it_needs() {
        let data = WheelCore.view(&fixture()).unwrap();

        let lines = WheelPresentation.present(&data);

        assert!(lines[0].starts_with("Python wheel csvstats 1.0.3"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Python py3, ABI none, platform any"))
        );
        assert!(lines.iter().any(|line| line.contains("Pure Python")));
        assert!(lines.iter().any(|line| line.contains("Needs Python >=3.9")));
        assert!(lines.iter().any(|line| line.contains("command csvstats")));
    }

    #[test]
    fn a_zip_that_is_not_a_wheel_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really-a.whl");
        std::fs::write(&path, b"PK\x03\x04 and then nothing of the sort").unwrap();

        assert!(WheelCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
