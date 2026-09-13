//! Alpine Linux package file type plugin: core and presentation halves.
//!
//! An Alpine package shares its `.apk` extension with an Android one and
//! has nothing else in common with it: this is three gzip streams laid
//! end to end - the signature, the control entries, then the files -
//! while an Android package is a zip. The two are told apart by content,
//! which is the only honest way, and this plugin therefore claims no
//! extension at all: the bytes decide.
//!
//! Everything a reader wants is in `.PKGINFO`, a plain `key = value`
//! file in the control stream.

use flate2::read::MultiGzDecoder;
use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::io::Read as _;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
///
/// Deliberately empty. `apk` belongs to the Android package plugin,
/// which claims it because that is the format most people mean; two
/// plugins claiming one extension is a defect. Nothing is lost: the
/// extension hint only ever chooses between plugins that already
/// recognised the bytes (GUIDANCE.md section 3.3), and an Alpine package
/// is recognised outright by its control stream, which no other format
/// has.
pub const EXTENSIONS: &[&str] = &[];

/// The two bytes a gzip member opens with.
const GZIP_MAGIC: &[u8] = &[0x1f, 0x8b];

/// A tar header block, and the alignment every member starts on.
const BLOCK: usize = 512;

/// The control entry every package carries.
const PKGINFO: &str = ".PKGINFO";

/// How much of a package is decompressed to read the control stream. The
/// control stream is a few kilobytes; the files after it can be
/// hundreds of megabytes and none of them are read.
const CONTROL_LIMIT: u64 = 512 * 1024;

/// View data produced by [`ApkpkgCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApkpkgView {
    /// The package name.
    pub name: String,
    /// Its version, including the Alpine release suffix.
    pub version: String,
    /// The one-line description.
    pub description: Option<String>,
    /// Where the software lives.
    pub url: Option<String>,
    /// The architecture it was built for.
    pub architecture: Option<String>,
    /// How much it takes once installed, in bytes.
    pub installed_size: Option<u64>,
    /// The source package it was built from.
    pub origin: Option<String>,
    /// The licence.
    pub licence: Option<String>,
    /// Who looks after it.
    pub maintainer: Option<String>,
    /// What built it.
    pub packager: Option<String>,
    /// When, as seconds since the epoch.
    pub build_date: Option<i64>,
    /// The commit of the build recipe.
    pub commit: Option<String>,
    /// What has to be installed alongside it.
    pub depends: Vec<String>,
    /// What it satisfies for something else.
    pub provides: Vec<String>,
    /// The scripts that run around installing it. These are the reason
    /// installing a package is not just unpacking one.
    pub scripts: Vec<String>,
    /// Whether the package carries a signature.
    pub signed: bool,
    /// How many files it installs.
    pub file_count: usize,
    /// The first of those files.
    pub files: Vec<String>,
}

/// How many file paths are listed before the rest are only counted.
const SHOWN: usize = 32;

/// The name a tar header block gives, trimmed of its padding.
fn name_in(block: &[u8]) -> String {
    let end = block[..100]
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(100);
    String::from_utf8_lossy(&block[..end]).into_owned()
}

/// An octal header field's value, or `None` when it is not one.
fn octal(field: &[u8]) -> Option<u64> {
    let end = field
        .iter()
        .position(|byte| *byte == 0 || *byte == b' ')
        .unwrap_or(field.len());
    u64::from_str_radix(String::from_utf8_lossy(&field[..end]).trim(), 8).ok()
}

/// Every member of the tar stream in `bytes`, as a name and its content.
///
/// A package is several tars concatenated, so a run of zero blocks is
/// the end of one of them rather than the end of the stream: this skips
/// the run and carries on with the next.
fn members_of(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut members = Vec::new();
    let mut at = 0usize;
    while at + BLOCK <= bytes.len() {
        let block = &bytes[at..at + BLOCK];
        if block.iter().all(|byte| *byte == 0) {
            at += BLOCK;
            continue;
        }
        let name = name_in(block);
        let size = octal(&block[124..136])
            .and_then(|size| usize::try_from(size).ok())
            .unwrap_or(0);
        at += BLOCK;
        let end = (at + size).min(bytes.len());
        if !name.is_empty() {
            members.push((name, bytes[at..end].to_vec()));
        }
        at = end.div_ceil(BLOCK) * BLOCK;
    }
    members
}

/// Whether `prefix` opens like an Alpine package.
///
/// The gzip magic is not enough - a gzip is a gzip - so the first
/// member is decompressed and its first tar entry read. A package opens
/// with either its signature or, unsigned, its control entry, and no
/// other format puts a file of either name at offset zero.
fn looks_like_it(prefix: &[u8]) -> bool {
    if !prefix.starts_with(GZIP_MAGIC) {
        return false;
    }
    let mut head = vec![0u8; BLOCK];
    let mut decoder = MultiGzDecoder::new(prefix).take(BLOCK as u64);
    // A prefix cut short mid-member reads as much as it can, and one
    // block is all this needs.
    let mut filled = 0usize;
    while filled < BLOCK {
        match decoder.read(&mut head[filled..]) {
            Ok(0) | Err(_) => break,
            Ok(read) => filled += read,
        }
    }
    if filled < BLOCK {
        return false;
    }
    let name = name_in(&head);
    name == PKGINFO || name.starts_with(".SIGN.")
}

/// The `key = value` lines of a `.PKGINFO`, in file order.
fn pkginfo_in(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

/// The first value of `wanted`.
fn one<'a>(fields: &'a [(String, String)], wanted: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(key, _)| key == wanted)
        .map(|(_, value)| value.as_str())
}

/// Every value of `wanted`, which `.PKGINFO` repeats for a list.
fn every(fields: &[(String, String)], wanted: &str) -> Vec<String> {
    fields
        .iter()
        .filter(|(key, _)| key == wanted)
        .map(|(_, value)| value.clone())
        .collect()
}

/// Everything [`ApkpkgView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<ApkpkgView> {
    let bytes = std::fs::read(path)?;
    if !looks_like_it(&bytes) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not an Alpine package: no .PKGINFO or signature at the front",
        ));
    }
    let mut unpacked = Vec::new();
    MultiGzDecoder::new(bytes.as_slice())
        .take(CONTROL_LIMIT)
        .read_to_end(&mut unpacked)?;
    let members = members_of(&unpacked);

    let control = members
        .iter()
        .find(|(name, _)| name == PKGINFO)
        .map(|(_, body)| String::from_utf8_lossy(body).into_owned())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no .PKGINFO"))?;
    let fields = pkginfo_in(&control);

    // A control entry opens with a dot; anything else is a file the
    // package installs.
    let mut files: Vec<String> = members
        .iter()
        .map(|(name, _)| name.clone())
        .filter(|name| !name.starts_with('.') && !name.ends_with('/'))
        .collect();
    let file_count = files.len();
    files.truncate(SHOWN);

    Ok(ApkpkgView {
        name: one(&fields, "pkgname").unwrap_or_default().to_owned(),
        version: one(&fields, "pkgver").unwrap_or_default().to_owned(),
        description: one(&fields, "pkgdesc").map(ToOwned::to_owned),
        url: one(&fields, "url").map(ToOwned::to_owned),
        architecture: one(&fields, "arch").map(ToOwned::to_owned),
        installed_size: one(&fields, "size").and_then(|said| said.parse().ok()),
        origin: one(&fields, "origin").map(ToOwned::to_owned),
        licence: one(&fields, "license").map(ToOwned::to_owned),
        maintainer: one(&fields, "maintainer").map(ToOwned::to_owned),
        packager: one(&fields, "packager").map(ToOwned::to_owned),
        build_date: one(&fields, "builddate").and_then(|said| said.parse().ok()),
        commit: one(&fields, "commit").map(ToOwned::to_owned),
        depends: every(&fields, "depend"),
        provides: every(&fields, "provides"),
        scripts: members
            .iter()
            .map(|(name, _)| name.clone())
            .filter(|name| {
                name.starts_with(".pre-") || name.starts_with(".post-") || name == ".trigger"
            })
            .collect(),
        signed: members.iter().any(|(name, _)| name.starts_with(".SIGN.")),
        file_count,
        files,
    })
}

/// The Alpine package plugin's core half.
#[derive(Debug, Default)]
pub struct ApkpkgCore;

impl PluginCore for ApkpkgCore {
    fn name(&self) -> &'static str {
        "apkpkg"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A package is a gzip stream, which `gzip` recognises. This is
        // the narrower reading of the same bytes (D13), and it is what
        // wins the file without needing the extension - which the
        // Android package plugin has.
        &["gzip"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_it(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Alpine package plugin's presentation half.
#[derive(Debug, Default)]
pub struct ApkpkgPresentation;

impl PluginPresentation for ApkpkgPresentation {
    fn name(&self) -> &'static str {
        "apkpkg"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "APKG",
            tint: 0x000d_597f,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: ApkpkgView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!("Alpine package {} {}", view.name, view.version)];
        if let Some(description) = &view.description {
            lines.push(description.clone());
        }
        lines.push(match (&view.architecture, view.installed_size) {
            (Some(arch), Some(size)) => format!("For {arch}, {size} byte(s) installed"),
            (Some(arch), None) => format!("For {arch}"),
            (None, Some(size)) => format!("{size} byte(s) installed"),
            (None, None) => "The control entry states no architecture.".to_owned(),
        });
        if let Some(licence) = &view.licence {
            lines.push(format!("Licence {licence}"));
        }
        if let Some(maintainer) = &view.maintainer {
            lines.push(format!("Maintained by {maintainer}"));
        }
        if let Some(packager) = &view.packager {
            lines.push(format!("Built by {packager}"));
        }
        if let Some(url) = &view.url {
            lines.push(url.clone());
        }
        if let Some(origin) = &view.origin {
            lines.push(format!("Built from the {origin} recipe"));
        }
        if let Some(commit) = &view.commit {
            lines.push(format!("at commit {commit}"));
        }
        if view.depends.is_empty() {
            lines.push("Needs nothing else installed.".to_owned());
        } else {
            lines.push("Needs:".to_owned());
            for depend in &view.depends {
                lines.push(format!("  {depend}"));
            }
        }
        for provide in &view.provides {
            lines.push(format!("Provides {provide}"));
        }
        if view.scripts.is_empty() {
            lines.push("Installing it only unpacks files; nothing is run.".to_owned());
        } else {
            lines.push("Installing it runs:".to_owned());
            for script in &view.scripts {
                lines.push(format!("  {script}"));
            }
        }
        lines.push(if view.signed {
            "Signed, so the index can prove it came from its builder.".to_owned()
        } else {
            "Unsigned.".to_owned()
        });
        lines.push(format!("{} file(s):", view.file_count));
        for file in &view.files {
            lines.push(format!("  {file}"));
        }
        if view.file_count > view.files.len() {
            lines.push(format!(
                "  ... and {} more",
                view.file_count - view.files.len()
            ));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{ApkpkgCore, ApkpkgPresentation, ApkpkgView, looks_like_it, pkginfo_in};
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/apkpkg/csvstats-1.0.3-r0.apk")
    }

    fn view_of() -> ApkpkgView {
        serde_json::from_value(ApkpkgCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&ApkpkgCore),
            PluginPresentation::extensions(&ApkpkgPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn it_claims_no_extension_because_android_has_that_one() {
        assert!(
            PluginCore::extensions(&ApkpkgCore).is_empty(),
            "two plugins claiming `apk` is the defect this avoids; the bytes decide"
        );
    }

    #[test]
    fn sniffs_the_control_stream_rather_than_the_extension() {
        let bytes = std::fs::read(fixture()).unwrap();

        assert!(looks_like_it(&bytes));
        assert!(!looks_like_it(b"PK\x03\x04"), "an Android package is a zip");
        assert!(
            !looks_like_it(&bytes[..2]),
            "the magic on its own says only that it is a gzip"
        );
        assert!(!looks_like_it(b""));
    }

    #[test]
    fn a_plain_gzip_is_not_a_package() {
        // A gzip of something that is not a tar at all: the magic matches
        // and nothing else does.
        let path = std::env::temp_dir().join("plain.gz");
        let body: Vec<u8> = std::iter::repeat_n(b'x', 4096).collect();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &body).unwrap();
        std::fs::write(&path, encoder.finish().unwrap()).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert!(!looks_like_it(&bytes));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn it_says_it_specialises_the_gzip_reading() {
        assert_eq!(ApkpkgCore.specialises(), &["gzip"]);
    }

    #[test]
    fn a_comment_line_is_not_a_field() {
        let fields = pkginfo_in("# Generated by abuild\npkgname = a\npkgver = 1\n");

        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0], ("pkgname".to_owned(), "a".to_owned()));
    }

    #[test]
    fn reads_the_identity_out_of_the_control_entry() {
        let view = view_of();

        assert_eq!(view.name, "csvstats");
        assert_eq!(view.version, "1.0.3-r0");
        assert_eq!(
            view.description.as_deref(),
            Some("Summary statistics for a column of readings")
        );
        assert_eq!(view.architecture.as_deref(), Some("x86_64"));
        assert_eq!(view.installed_size, Some(24576));
        assert_eq!(view.origin.as_deref(), Some("csvstats"));
        assert_eq!(view.licence.as_deref(), Some("MIT"));
        assert_eq!(
            view.maintainer.as_deref(),
            Some("The floor <floor@example.com>")
        );
        assert!(view.packager.is_some());
        assert_eq!(view.build_date, Some(1_789_000_000));
        assert!(view.commit.is_some());
        assert_eq!(
            view.url.as_deref(),
            Some("https://example.com/floor/csvstats")
        );
    }

    #[test]
    fn a_repeated_key_becomes_a_list() {
        let view = view_of();

        assert_eq!(view.depends, vec!["musl", "so:libc.musl-x86_64.so.1"]);
        assert_eq!(
            view.provides,
            vec!["cmd:csvstats=1.0.3-r0", "cmd:csvstats-report=1.0.3-r0"]
        );
    }

    #[test]
    fn finds_the_scripts_the_signature_and_the_files() {
        let view = view_of();

        assert_eq!(view.scripts, vec![".post-install", ".pre-deinstall"]);
        assert!(view.signed);
        assert_eq!(view.file_count, 5);
        assert!(view.files.iter().any(|file| file == "usr/bin/csvstats"));
        assert!(
            !view.files.iter().any(|file| file.starts_with('.')),
            "a control entry is not a file the package installs"
        );
    }

    #[test]
    fn presents_that_installing_it_runs_something() {
        let data = ApkpkgCore.view(&fixture()).unwrap();

        let lines = ApkpkgPresentation.present(&data);

        assert!(lines[0].starts_with("Alpine package csvstats 1.0.3-r0"));
        assert!(lines.iter().any(|line| line.contains("For x86_64")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Installing it runs:"))
        );
        assert!(lines.iter().any(|line| line.contains(".post-install")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Provides cmd:csvstats"))
        );
    }

    #[test]
    fn a_file_that_is_not_a_package_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really-an-alpine.apk");
        std::fs::write(&path, b"\x1f\x8b and then nothing of the sort").unwrap();

        assert!(ApkpkgCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
