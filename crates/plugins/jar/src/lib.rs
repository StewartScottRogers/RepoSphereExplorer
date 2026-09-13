//! Java archive file type plugin: core and presentation halves.
//!
//! A Java archive (JAR) is a zip with one entry that makes it more than a
//! zip: `META-INF/MANIFEST.MF`. The manifest is what tells a runtime
//! which class to start, which other archives to put on the class path,
//! and what version this is. Reading a Java archive means reading that
//! manifest, then counting what the rest of the zip holds.

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
pub const EXTENSIONS: &[&str] = &["jar", "war", "ear"];

/// The four bytes a zip's first local header opens with.
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// The entry that makes a zip a Java archive.
const MANIFEST_PATH: &str = "META-INF/MANIFEST.MF";

/// The entry that makes a zip an Android package instead. An Android
/// package carries a manifest of its own, so without this the two
/// formats would be indistinguishable by content alone.
const ANDROID_MARKER: &[u8] = b"AndroidManifest.xml";

/// How many packages are listed before the rest are only counted.
const SHOWN: usize = 32;

/// View data produced by [`JarCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JarView {
    /// The manifest format version, from `Manifest-Version`.
    pub manifest_version: Option<String>,
    /// What wrote the archive, from `Created-By`.
    pub created_by: Option<String>,
    /// The class a plain `java -jar` would start, from `Main-Class`. An
    /// archive without one is a library, not something to run.
    pub main_class: Option<String>,
    /// The other archives this one expects beside it, from `Class-Path`.
    pub class_path: Vec<String>,
    /// The interface version, from `Specification-Version`.
    pub specification_version: Option<String>,
    /// The build version, from `Implementation-Version`.
    pub implementation_version: Option<String>,
    /// The name the module system gives this archive on the module path,
    /// from `Automatic-Module-Name`.
    pub automatic_module_name: Option<String>,
    /// Every attribute of the manifest's main section, in file order.
    pub attributes: Vec<(String, String)>,
    /// How many entries the zip holds, directories included.
    pub entries: usize,
    /// How many of them are compiled classes.
    pub classes: usize,
    /// The packages those classes sit in, in order, up to [`SHOWN`].
    pub packages: Vec<String>,
    /// How many packages there are in total.
    pub package_count: usize,
    /// Whether the archive is signed: a `META-INF/*.SF` signature file
    /// with a matching key beside it.
    pub signed: bool,
    /// The signature files found, which name the signers.
    pub signature_files: Vec<String>,
    /// The interfaces declared under `META-INF/services/`, which is how
    /// an archive offers an implementation to something else's loader.
    pub services: Vec<String>,
}

/// Whether `haystack` holds `needle` anywhere as a contiguous byte run.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Whether `prefix` opens like a Java archive.
///
/// A Java archive is a zip, so the signature alone settles nothing; the
/// manifest entry is the marker. The tools that build one write
/// `META-INF/` first precisely so a reader streaming the archive meets
/// the manifest before the classes, which is why the name is in the
/// prefix at all. An archive that hides its manifest past the sniffed
/// prefix reads as a plain zip - the honest answer for bytes that have
/// not said otherwise yet.
fn looks_like_it(prefix: &[u8]) -> bool {
    prefix.starts_with(ZIP_MAGIC)
        && contains_bytes(prefix, MANIFEST_PATH.as_bytes())
        && !contains_bytes(prefix, ANDROID_MARKER)
}

/// The main section's attributes, in file order.
///
/// A manifest wraps any line longer than 72 bytes by breaking it and
/// opening the next line with a single space, so a continuation is
/// joined back on before the name is split off. The main section ends at
/// the first blank line; what follows names individual entries and is
/// not what this reads.
fn attributes_in(manifest: &str) -> Vec<(String, String)> {
    let mut joined: Vec<String> = Vec::new();
    for line in manifest.lines() {
        if line.is_empty() {
            break;
        }
        if let Some(rest) = line.strip_prefix(' ')
            && let Some(last) = joined.last_mut()
        {
            last.push_str(rest);
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

/// The value of the attribute named `wanted`, if the manifest has one.
fn attribute<'a>(attributes: &'a [(String, String)], wanted: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
        .map(|(_, value)| value.as_str())
}

/// The package a class entry sits in, dots for slashes, or `None` for a
/// class in the unnamed package at the archive's root.
fn package_of(path: &str) -> Option<String> {
    let (directory, _) = path.rsplit_once('/')?;
    Some(directory.replace('/', "."))
}

/// Everything [`JarView`] holds, read from the file at `path`.
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "these are zip entry names, not filesystem paths, and the Java               archive specification fixes their case: a compiled class is               `.class`, and a signature file is an uppercase `.SF` beside an               uppercase `.RSA` or `.DSA`. Matching case-insensitively would               claim entries the runtime itself would not."
)]
fn read(path: &Path) -> io::Result<JarView> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    let mut manifest = String::new();
    if let Ok(mut entry) = archive.by_name(MANIFEST_PATH) {
        entry.read_to_string(&mut manifest)?;
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no META-INF/MANIFEST.MF, so this is a zip and not a Java archive",
        ));
    }
    let attributes = attributes_in(&manifest);

    let names: Vec<String> = archive.file_names().map(ToOwned::to_owned).collect();
    let mut packages: Vec<String> = Vec::new();
    let mut classes = 0usize;
    let mut signature_files = Vec::new();
    let mut services = Vec::new();
    for name in &names {
        if name.ends_with(".class") {
            classes += 1;
            if let Some(package) = package_of(name)
                && !packages.contains(&package)
            {
                packages.push(package);
            }
        }
        if name.starts_with("META-INF/") && name.ends_with(".SF") {
            signature_files.push(name.clone());
        }
        if let Some(service) = name.strip_prefix("META-INF/services/")
            && !service.is_empty()
        {
            services.push(service.to_owned());
        }
    }
    let package_count = packages.len();
    packages.truncate(SHOWN);

    // A signature file on its own proves nothing: the key that signed it
    // travels beside it, under the same stem.
    let signed = signature_files.iter().any(|signature| {
        let stem = signature.trim_end_matches(".SF");
        names.iter().any(|name| {
            name.starts_with(stem) && (name.ends_with(".RSA") || name.ends_with(".DSA"))
        })
    });

    Ok(JarView {
        manifest_version: attribute(&attributes, "Manifest-Version").map(ToOwned::to_owned),
        created_by: attribute(&attributes, "Created-By").map(ToOwned::to_owned),
        main_class: attribute(&attributes, "Main-Class").map(ToOwned::to_owned),
        class_path: attribute(&attributes, "Class-Path")
            .map(|value| value.split_whitespace().map(ToOwned::to_owned).collect())
            .unwrap_or_default(),
        specification_version: attribute(&attributes, "Specification-Version")
            .map(ToOwned::to_owned),
        implementation_version: attribute(&attributes, "Implementation-Version")
            .map(ToOwned::to_owned),
        automatic_module_name: attribute(&attributes, "Automatic-Module-Name")
            .map(ToOwned::to_owned),
        attributes,
        entries: names.len(),
        classes,
        packages,
        package_count,
        signed,
        signature_files,
        services,
    })
}

/// The Java archive plugin's core half.
#[derive(Debug, Default)]
pub struct JarCore;

impl PluginCore for JarCore {
    fn name(&self) -> &'static str {
        "jar"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A Java archive is a zip, which `archive` recognises. This is
        // the narrower reading of the same bytes (D13).
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

/// The Java archive plugin's presentation half.
#[derive(Debug, Default)]
pub struct JarPresentation;

impl PluginPresentation for JarPresentation {
    fn name(&self) -> &'static str {
        "jar"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "JAR",
            tint: 0x00b0_7219,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: JarView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "Java archive: {} entry(ies), {} class(es) in {} package(s)",
            view.entries, view.classes, view.package_count
        )];
        lines.push(match view.main_class.as_deref() {
            Some(class) => format!("Starts at {class}"),
            None => "No Main-Class, so this is a library rather than something".to_owned(),
        });
        if view.main_class.is_none() {
            lines.push("`java -jar` can start.".to_owned());
        }
        if let Some(version) = &view.implementation_version {
            lines.push(format!("Implementation version {version}"));
        }
        if let Some(version) = &view.specification_version {
            lines.push(format!("Specification version {version}"));
        }
        if let Some(module) = &view.automatic_module_name {
            lines.push(format!("Module name {module}"));
        }
        if let Some(version) = &view.manifest_version {
            lines.push(format!("Manifest version {version}"));
        }
        if let Some(what) = &view.created_by {
            lines.push(format!("Created by {what}"));
        }
        if view.class_path.is_empty() {
            lines.push("Empty Class-Path: nothing else has to sit beside it.".to_owned());
        } else {
            lines.push("Class-Path, which must be found beside this archive:".to_owned());
            for entry in &view.class_path {
                lines.push(format!("  {entry}"));
            }
        }
        if !view.packages.is_empty() {
            lines.push("Packages:".to_owned());
            for package in &view.packages {
                lines.push(format!("  {package}"));
            }
            if view.package_count > view.packages.len() {
                lines.push(format!(
                    "  ... and {} more",
                    view.package_count - view.packages.len()
                ));
            }
        }
        for service in &view.services {
            lines.push(format!("Offers an implementation of {service}"));
        }
        if view.signed {
            lines.push(format!(
                "Signed by {}, so the classes cannot be altered without",
                view.signature_files.join(", ")
            ));
            lines.push("breaking the signature.".to_owned());
        } else {
            lines.push("Not signed.".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{JarCore, JarPresentation, JarView, attributes_in, looks_like_it};
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/jar/csvstats-1.0.3.jar")
    }

    fn view_of() -> JarView {
        serde_json::from_value(JarCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&JarCore),
            PluginPresentation::extensions(&JarPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn sniffs_a_zip_holding_a_manifest() {
        assert!(looks_like_it(b"PK\x03\x04.....META-INF/MANIFEST.MF...."));
        assert!(
            !looks_like_it(b"PK\x03\x04....hello.txt...."),
            "a zip without a manifest is a zip"
        );
        assert!(
            !looks_like_it(b"META-INF/MANIFEST.MF"),
            "the name on its own is not an archive"
        );
        assert!(
            !looks_like_it(b"PK\x03\x04..META-INF/MANIFEST.MF..AndroidManifest.xml.."),
            "an Android package carries a manifest too, and is not this"
        );
        assert!(!looks_like_it(b""));
    }

    #[test]
    fn it_says_it_specialises_the_archive_reading() {
        assert_eq!(JarCore.specialises(), &["archive"]);
    }

    #[test]
    fn a_folded_attribute_is_joined_back_together() {
        let manifest = "Main-Class: com.example.a\n Very.Long.Name\nX: 1\n\nName: other\n";

        let attributes = attributes_in(manifest);

        assert_eq!(
            attributes[0],
            (
                "Main-Class".to_owned(),
                "com.example.aVery.Long.Name".to_owned()
            ),
            "a continuation line opens with one space and joins with no gap"
        );
        assert_eq!(
            attributes.len(),
            2,
            "the main section ends at the blank line"
        );
    }

    #[test]
    fn reads_the_manifest_attributes() {
        let view = view_of();

        assert_eq!(
            view.main_class.as_deref(),
            Some("com.example.csvstats.Main")
        );
        assert_eq!(view.manifest_version.as_deref(), Some("1.0"));
        assert_eq!(view.implementation_version.as_deref(), Some("1.0.3"));
        assert_eq!(view.specification_version.as_deref(), Some("1.0"));
        assert_eq!(
            view.automatic_module_name.as_deref(),
            Some("com.example.csvstats")
        );
        assert!(view.created_by.is_some());
        assert!(view.attributes.len() >= 8);
    }

    #[test]
    fn reads_the_class_path_as_separate_entries() {
        let view = view_of();

        assert_eq!(
            view.class_path,
            vec!["lib/commons-csv-1.11.0.jar", "lib/slf4j-api-2.0.13.jar"],
            "Class-Path is one space-separated line, not one path"
        );
    }

    #[test]
    fn counts_the_entries_the_classes_and_their_packages() {
        let view = view_of();

        assert_eq!(view.classes, 4);
        assert!(view.entries > view.classes);
        assert_eq!(
            view.packages,
            vec!["com.example.csvstats", "com.example.csvstats.io"],
            "slashes in the path become dots in the package"
        );
        assert_eq!(view.package_count, 2);
    }

    #[test]
    fn says_the_archive_is_signed_and_by_what() {
        let view = view_of();

        assert!(view.signed);
        assert_eq!(view.signature_files, vec!["META-INF/FLOOR.SF"]);
    }

    #[test]
    fn reads_the_services_the_archive_offers() {
        let view = view_of();

        assert_eq!(view.services, vec!["com.example.csvstats.Reader"]);
    }

    #[test]
    fn presents_the_start_class_and_the_class_path() {
        let data = JarCore.view(&fixture()).unwrap();

        let lines = JarPresentation.present(&data);

        assert!(lines[0].starts_with("Java archive: "));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Starts at com.example.csvstats.Main"))
        );
        assert!(lines.iter().any(|line| line.contains("commons-csv")));
        assert!(lines.iter().any(|line| line.contains("Signed by")));
    }

    #[test]
    fn a_zip_without_a_manifest_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really-a.jar");
        std::fs::write(&path, b"PK\x03\x04 and then nothing of the sort").unwrap();

        assert!(JarCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
