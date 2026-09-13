//! Android package file type plugin: core and presentation halves.
//!
//! An Android package (APK) is a zip, but its `AndroidManifest.xml` is
//! not XML: the build tools compile it into Android's own binary XML
//! (AXML), a chunked format with a string pool at the front. Everything a
//! reader wants - the package name, the versions, the SDK range, the
//! permissions asked for - lives in that compiled manifest, so reading a
//! package means decoding it. There is no shortcut: the angle brackets
//! are gone.

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
///
/// `apk` is also Alpine Linux's package extension, and the two formats
/// are told apart by content rather than by name: an Android package is
/// a zip, an Alpine one a gzip stream.
pub const EXTENSIONS: &[&str] = &["apk"];

/// The four bytes a zip's first local header opens with.
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// The compiled manifest every package carries.
const MANIFEST_PATH: &str = "AndroidManifest.xml";

/// The Dalvik executable holding the compiled code.
const DEX_MARKER: &[u8] = b"classes.dex";

/// One activity, service or receiver the manifest declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Component {
    /// Which of them it is: `activity`, `service` or `receiver`.
    pub kind: String,
    /// Its class, as the manifest names it. A leading dot means the
    /// package name goes in front.
    pub name: String,
    /// Whether something outside the package may start it. An exported
    /// component is reachable by any other application on the device.
    pub exported: bool,
}

/// View data produced by [`ApkCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApkView {
    /// The application identifier, from the manifest's `package`.
    pub package: Option<String>,
    /// The number the store orders releases by, from `versionCode`.
    pub version_code: Option<i64>,
    /// The version a person reads, from `versionName`.
    pub version_name: Option<String>,
    /// The oldest Android this will install on, from `minSdkVersion`.
    pub min_sdk: Option<i64>,
    /// The Android it was built against, from `targetSdkVersion`.
    pub target_sdk: Option<i64>,
    /// Every permission asked for, in manifest order.
    pub permissions: Vec<String>,
    /// The activities, services and receivers declared.
    pub components: Vec<Component>,
    /// The architectures `lib/` carries code for.
    pub native_architectures: Vec<String>,
    /// How many `classes*.dex` files there are. More than one means the
    /// code did not fit in a single Dalvik executable.
    pub dex_files: usize,
    /// How many entries the zip holds.
    pub entries: usize,
    /// Which signing scheme the package uses, as far as the entries show.
    pub signing: String,
}

/// Whether `haystack` holds `needle` anywhere as a contiguous byte run.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Whether `prefix` opens like an Android package.
///
/// The build tools write the compiled manifest as the first entry, so
/// its name is in the prefix; the Dalvik executable is asked for as well
/// because a plain zip could hold a file of that name by coincidence and
/// a package could not exist without code.
fn looks_like_it(prefix: &[u8]) -> bool {
    prefix.starts_with(ZIP_MAGIC)
        && contains_bytes(prefix, MANIFEST_PATH.as_bytes())
        && contains_bytes(prefix, DEX_MARKER)
}

/// A cursor over a compiled manifest, reading little-endian.
struct Axml<'bytes> {
    /// The whole compiled manifest.
    bytes: &'bytes [u8],
    /// The strings the pool holds, in index order.
    pool: Vec<String>,
}

/// The chunk type of a resource string pool.
const CHUNK_STRING_POOL: u16 = 0x0001;
/// The chunk type opening an element.
const CHUNK_START_ELEMENT: u16 = 0x0102;
/// The string pool flag saying its strings are UTF-8 rather than UTF-16.
const POOL_UTF8: u32 = 0x0000_0100;
/// The typed-value type meaning the data is a string pool index.
const TYPE_STRING: u8 = 0x03;
/// The typed-value type meaning the data is a boolean.
const TYPE_BOOLEAN: u8 = 0x12;

/// Two bytes read little-endian at `at`, or `None` past the end.
fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]))
}

/// Four bytes read little-endian at `at`, or `None` past the end.
fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *bytes.get(at)?,
        *bytes.get(at + 1)?,
        *bytes.get(at + 2)?,
        *bytes.get(at + 3)?,
    ]))
}

/// Four bytes read little-endian at `at` as a signed index, where a
/// negative value means "no string".
fn i32_at(bytes: &[u8], at: usize) -> Option<i32> {
    u32_at(bytes, at).map(|value| i32::from_ne_bytes(value.to_ne_bytes()))
}

/// A string pool length, which is one or two units long: the high bit of
/// the first unit says a second follows. Returns the length and how many
/// bytes it took.
fn pool_length(bytes: &[u8], at: usize, utf8: bool) -> Option<(usize, usize)> {
    if utf8 {
        let first = usize::from(*bytes.get(at)?);
        if first & 0x80 == 0 {
            return Some((first, 1));
        }
        let second = usize::from(*bytes.get(at + 1)?);
        Some(((((first & 0x7f) << 8) | second), 2))
    } else {
        let first = usize::from(u16_at(bytes, at)?);
        if first & 0x8000 == 0 {
            return Some((first, 2));
        }
        let second = usize::from(u16_at(bytes, at + 2)?);
        Some(((((first & 0x7fff) << 16) | second), 4))
    }
}

/// Every string the pool at `at` holds, in index order.
fn strings_in(bytes: &[u8], at: usize) -> Option<Vec<String>> {
    let header_size = usize::from(u16_at(bytes, at + 2)?);
    let count = u32_at(bytes, at + 8)? as usize;
    let flags = u32_at(bytes, at + 16)?;
    let strings_start = at + u32_at(bytes, at + 20)? as usize;
    let utf8 = flags & POOL_UTF8 != 0;
    let mut strings = Vec::with_capacity(count);
    for index in 0..count {
        let offset = u32_at(bytes, at + header_size + index * 4)? as usize;
        let mut cursor = strings_start + offset;
        // UTF-8 entries state the length twice: in characters, then in
        // bytes. Only the second measures the run that follows.
        let (characters, taken) = pool_length(bytes, cursor, utf8)?;
        cursor += taken;
        if utf8 {
            let (length, taken) = pool_length(bytes, cursor, true)?;
            cursor += taken;
            let run = bytes.get(cursor..cursor + length)?;
            strings.push(String::from_utf8_lossy(run).into_owned());
        } else {
            let run = bytes.get(cursor..cursor + characters * 2)?;
            let (pairs, _) = run.as_chunks::<2>();
            let units: Vec<u16> = pairs.iter().copied().map(u16::from_le_bytes).collect();
            strings.push(String::from_utf16_lossy(&units));
        }
    }
    Some(strings)
}

/// One attribute of an element, with its value already resolved.
struct Attribute {
    /// Its name, as the string pool spells it.
    name: String,
    /// Its value as text, whatever the typed value's kind.
    value: String,
    /// Its value as a number, when the typed value is one.
    number: Option<i64>,
}

impl Axml<'_> {
    /// The string at `index`, or `None` for a negative or absent one.
    fn string(&self, index: i32) -> Option<&str> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.pool.get(index))
            .map(String::as_str)
    }

    /// The attributes of the element whose chunk starts at `at`.
    fn attributes(&self, at: usize) -> Vec<Attribute> {
        // A start-element chunk is a sixteen-byte node header, then the
        // element's own fields; the attributes sit at an offset counted
        // from the end of that header, not from the chunk.
        let extension = at + 16;
        let Some(start) = u16_at(self.bytes, extension + 8) else {
            return Vec::new();
        };
        let Some(size) = u16_at(self.bytes, extension + 10) else {
            return Vec::new();
        };
        let Some(count) = u16_at(self.bytes, extension + 12) else {
            return Vec::new();
        };
        let mut attributes = Vec::new();
        for index in 0..usize::from(count) {
            let entry = extension + usize::from(start) + index * usize::from(size);
            let (Some(name), Some(raw), Some(kind), Some(data)) = (
                i32_at(self.bytes, entry + 4),
                i32_at(self.bytes, entry + 8),
                self.bytes.get(entry + 15).copied(),
                i32_at(self.bytes, entry + 16),
            ) else {
                break;
            };
            let Some(name) = self.string(name) else {
                continue;
            };
            let value = match kind {
                TYPE_STRING => self
                    .string(raw)
                    .or_else(|| self.string(data))
                    .unwrap_or_default()
                    .to_owned(),
                TYPE_BOOLEAN => (data != 0).to_string(),
                _ => self
                    .string(raw)
                    .map_or_else(|| data.to_string(), ToOwned::to_owned),
            };
            attributes.push(Attribute {
                name: name.to_owned(),
                value,
                number: (kind != TYPE_STRING).then_some(i64::from(data)),
            });
        }
        attributes
    }
}

/// The value of `wanted`, if the element has an attribute by that name.
fn value_of<'a>(attributes: &'a [Attribute], wanted: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|attribute| attribute.name == wanted)
        .map(|attribute| attribute.value.as_str())
}

/// The number `wanted` holds, if the element has it and it is one.
fn number_of(attributes: &[Attribute], wanted: &str) -> Option<i64> {
    attributes
        .iter()
        .find(|attribute| attribute.name == wanted)
        .and_then(|attribute| attribute.number)
}

/// Reads the compiled manifest into the fields of `view`.
fn read_manifest(bytes: &[u8], view: &mut ApkView) -> io::Result<()> {
    let malformed = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "the compiled manifest is malformed",
        )
    };
    // The file is one outer chunk; everything else sits inside it, and
    // the string pool has to come first because every later chunk names
    // its strings by index into it.
    let mut at = 8usize;
    let mut pool = Vec::new();
    while let (Some(kind), Some(size)) = (u16_at(bytes, at), u32_at(bytes, at + 4)) {
        if size == 0 {
            break;
        }
        if kind == CHUNK_STRING_POOL {
            pool = strings_in(bytes, at).ok_or_else(malformed)?;
            at += size as usize;
            break;
        }
        at += size as usize;
    }
    if pool.is_empty() {
        return Err(malformed());
    }
    let axml = Axml { bytes, pool };

    while let (Some(kind), Some(size)) = (u16_at(bytes, at), u32_at(bytes, at + 4)) {
        if size == 0 {
            break;
        }
        if kind == CHUNK_START_ELEMENT {
            let name = i32_at(bytes, at + 20)
                .and_then(|index| axml.string(index))
                .unwrap_or_default()
                .to_owned();
            let attributes = axml.attributes(at);
            match name.as_str() {
                "manifest" => {
                    view.package = value_of(&attributes, "package").map(ToOwned::to_owned);
                    view.version_code = number_of(&attributes, "versionCode");
                    view.version_name = value_of(&attributes, "versionName").map(ToOwned::to_owned);
                }
                "uses-sdk" => {
                    view.min_sdk = number_of(&attributes, "minSdkVersion");
                    view.target_sdk = number_of(&attributes, "targetSdkVersion");
                }
                "uses-permission" => {
                    if let Some(asked) = value_of(&attributes, "name") {
                        view.permissions.push(asked.to_owned());
                    }
                }
                "activity" | "service" | "receiver" | "provider" => {
                    if let Some(class) = value_of(&attributes, "name") {
                        view.components.push(Component {
                            kind: name.clone(),
                            name: class.to_owned(),
                            exported: value_of(&attributes, "exported") == Some("true"),
                        });
                    }
                }
                _ => {}
            }
        }
        at += size as usize;
    }
    Ok(())
}

/// Everything [`ApkView`] holds, read from the file at `path`.
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "these are zip entry names, not filesystem paths, and the build               tools fix their case: the code is in `classes.dex`, never               `classes.DEX`."
)]
fn read(path: &Path) -> io::Result<ApkView> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let names: Vec<String> = archive.file_names().map(ToOwned::to_owned).collect();

    let mut view = ApkView {
        package: None,
        version_code: None,
        version_name: None,
        min_sdk: None,
        target_sdk: None,
        permissions: Vec::new(),
        components: Vec::new(),
        native_architectures: Vec::new(),
        dex_files: names
            .iter()
            .filter(|name| name.starts_with("classes") && name.ends_with(".dex"))
            .count(),
        entries: names.len(),
        signing: signing_scheme(&names),
    };
    for name in &names {
        if let Some(rest) = name.strip_prefix("lib/")
            && let Some((architecture, _)) = rest.split_once('/')
            && !view
                .native_architectures
                .iter()
                .any(|had| had == architecture)
        {
            view.native_architectures.push(architecture.to_owned());
        }
    }

    let mut manifest = Vec::new();
    match archive.by_name(MANIFEST_PATH) {
        Ok(mut entry) => entry.read_to_end(&mut manifest)?,
        Err(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "no AndroidManifest.xml, so this is a zip and not an Android package",
            ));
        }
    };
    read_manifest(&manifest, &mut view)?;
    Ok(view)
}

/// What the entries show about how the package is signed.
///
/// Only the first scheme leaves entries behind. Schemes two and later
/// sign the zip itself and live in a block between the entries and the
/// central directory, which the entry names cannot see - so this says
/// what it can see and no more.
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "a v1 signature file is an uppercase `.SF` under `META-INF/` by               specification, and matching any other case would claim an entry               no verifier would look at."
)]
fn signing_scheme(names: &[String]) -> String {
    let signature = names
        .iter()
        .any(|name| name.starts_with("META-INF/") && name.ends_with(".SF"));
    if signature {
        "v1 (JAR signing): META-INF holds a signature file".to_owned()
    } else {
        "no v1 signature; a v2 or later signature would sit outside the entries".to_owned()
    }
}

/// The Android package plugin's core half.
#[derive(Debug, Default)]
pub struct ApkCore;

impl PluginCore for ApkCore {
    fn name(&self) -> &'static str {
        "apk"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // An Android package is a zip, which `archive` recognises. This
        // is the narrower reading of the same bytes (D13).
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

/// The Android package plugin's presentation half.
#[derive(Debug, Default)]
pub struct ApkPresentation;

impl PluginPresentation for ApkPresentation {
    fn name(&self) -> &'static str {
        "apk"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "APK",
            tint: 0x003d_dc84,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: ApkView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "Android package {}, version {} ({})",
            view.package.as_deref().unwrap_or("(unnamed)"),
            view.version_name.as_deref().unwrap_or("?"),
            view.version_code
                .map_or_else(|| "?".to_owned(), |code| code.to_string())
        )];
        lines.push(match (view.min_sdk, view.target_sdk) {
            (Some(min), Some(target)) => {
                format!("Installs on API {min} and later; built against API {target}")
            }
            (Some(min), None) => format!("Installs on API {min} and later"),
            (None, Some(target)) => format!("Built against API {target}"),
            (None, None) => "The manifest states no SDK range.".to_owned(),
        });
        lines.push(format!(
            "{} entry(ies), {} Dalvik executable(s)",
            view.entries, view.dex_files
        ));
        if view.dex_files > 1 {
            lines.push("The code did not fit in one executable, so it is split.".to_owned());
        }
        if view.native_architectures.is_empty() {
            lines.push("No native libraries: this runs anywhere Android does.".to_owned());
        } else {
            lines.push(format!(
                "Native code for {}",
                view.native_architectures.join(", ")
            ));
        }
        if view.permissions.is_empty() {
            lines.push("Asks for no permissions.".to_owned());
        } else {
            lines.push("Asks for:".to_owned());
            for asked in &view.permissions {
                lines.push(format!("  {asked}"));
            }
        }
        if !view.components.is_empty() {
            lines.push("Declares:".to_owned());
            for component in &view.components {
                let reach = if component.exported {
                    "exported, so anything on the device can start it"
                } else {
                    "not exported"
                };
                lines.push(format!("  {} {} - {reach}", component.kind, component.name));
            }
        }
        lines.push(format!("Signing: {}", view.signing));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{ApkCore, ApkPresentation, ApkView, looks_like_it};
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/apk/csvstats-1.0.3.apk")
    }

    fn view_of() -> ApkView {
        serde_json::from_value(ApkCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&ApkCore),
            PluginPresentation::extensions(&ApkPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn sniffs_a_zip_holding_a_manifest_and_code() {
        assert!(looks_like_it(
            b"PK\x03\x04..AndroidManifest.xml..classes.dex.."
        ));
        assert!(
            !looks_like_it(b"PK\x03\x04..AndroidManifest.xml.."),
            "a manifest with no code is not a package"
        );
        assert!(
            !looks_like_it(b"PK\x03\x04..META-INF/MANIFEST.MF.."),
            "that is a Java archive"
        );
        assert!(!looks_like_it(b""));
    }

    #[test]
    fn it_says_it_specialises_the_archive_reading() {
        assert_eq!(ApkCore.specialises(), &["archive"]);
    }

    #[test]
    fn reads_the_package_and_its_versions_out_of_the_compiled_manifest() {
        let view = view_of();

        assert_eq!(view.package.as_deref(), Some("com.example.csvstats"));
        assert_eq!(view.version_code, Some(103));
        assert_eq!(view.version_name.as_deref(), Some("1.0.3"));
    }

    #[test]
    fn reads_the_sdk_range() {
        let view = view_of();

        assert_eq!(view.min_sdk, Some(26));
        assert_eq!(view.target_sdk, Some(34));
    }

    #[test]
    fn reads_every_permission_in_manifest_order() {
        let view = view_of();

        assert_eq!(
            view.permissions,
            vec![
                "android.permission.INTERNET",
                "android.permission.READ_EXTERNAL_STORAGE",
                "android.permission.ACCESS_FINE_LOCATION",
            ]
        );
    }

    #[test]
    fn reads_the_components_and_which_of_them_are_exported() {
        let view = view_of();

        let kinds: Vec<&str> = view
            .components
            .iter()
            .map(|component| component.kind.as_str())
            .collect();
        assert_eq!(kinds, vec!["activity", "activity", "service", "receiver"]);
        assert_eq!(view.components[0].name, ".MainActivity");
        assert!(view.components[0].exported, "the launcher activity is");
        assert!(!view.components[1].exported);
    }

    #[test]
    fn reads_the_native_architectures_and_counts_the_executables() {
        let view = view_of();

        assert_eq!(view.native_architectures, vec!["arm64-v8a", "armeabi-v7a"]);
        assert_eq!(view.dex_files, 2);
        assert!(view.entries >= 10);
    }

    #[test]
    fn says_which_signing_scheme_the_entries_show() {
        let view = view_of();

        assert!(view.signing.starts_with("v1 "), "{}", view.signing);
    }

    #[test]
    fn presents_the_package_the_permissions_and_the_split_code() {
        let data = ApkCore.view(&fixture()).unwrap();

        let lines = ApkPresentation.present(&data);

        assert!(lines[0].starts_with("Android package com.example.csvstats"));
        assert!(lines.iter().any(|line| line.contains("API 26 and later")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("android.permission.ACCESS_FINE_LOCATION"))
        );
        assert!(lines.iter().any(|line| line.contains("arm64-v8a")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("anything on the device can start it"))
        );
    }

    #[test]
    fn a_zip_without_a_manifest_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really-an.apk");
        std::fs::write(&path, b"PK\x03\x04 and then nothing of the sort").unwrap();

        assert!(ApkCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
