//! OS package file type plugin: core and presentation halves.
//!
//! Covers `.deb` (Debian binary package) and `.rpm` (RPM package) as one
//! plugin, matching the issue's direction that this be one "OS package"
//! view rather than a plugin per package format.
//!
//! `.deb` is a plain Unix `ar` archive (sniffed by the format's fixed
//! `!<arch>\n` global header plus a `debian-binary` first member name, a
//! marker not used by any sibling plugin) carrying a `control.tar.*` member
//! (package metadata: name, version, architecture, dependencies) and a
//! `data.tar.*` member (the installed file list) — hand-rolled with the
//! `ar` and `tar` crates plus `flate2`/`lzma-rs` for the gzip/xz member
//! compression real `.deb` files use; a zstd-compressed member is not
//! decoded, an accepted limitation matching this project's other
//! structurally-sniffed formats. `.rpm` is read with the `rpm` crate, whose
//! header tags carry the same metadata, dependencies, and file list
//! without needing to touch the compressed payload at all.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io::{self, Read as _};
use std::path::Path;

/// Maximum number of file entries listed in the view; packages with more
/// are truncated, matching the `archive` plugin's own `MAX_ENTRIES` limit.
const MAX_ENTRIES: usize = 200;

/// The `ar` format's fixed global header, opening every `.deb` file.
const DEB_AR_MAGIC: &[u8] = b"!<arch>\n";

/// The identifier of a `.deb` archive's first member, right after the
/// global header, distinguishing it from an arbitrary `ar` archive (e.g. a
/// static library) that no sibling plugin sniffs today.
const DEB_FIRST_MEMBER_NAME: &[u8] = b"debian-binary";

/// The RPM lead's fixed 4-byte magic number.
const RPM_MAGIC: [u8; 4] = [0xed, 0xab, 0xee, 0xdb];

/// One file entry inside a package's file list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageFileEntry {
    /// The file's installation path.
    pub path: String,
    /// The file's size in bytes.
    pub size: u64,
}

/// View data produced by [`PackageArchiveCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageArchiveView {
    /// The detected package format: `"deb"` or `"rpm"`.
    pub format: String,
    /// The package name.
    pub name: String,
    /// The package version (for RPM, `version-release`).
    pub version: String,
    /// The package's target architecture.
    pub architecture: String,
    /// A one-line summary of the package, if present.
    pub summary: Option<String>,
    /// The packages or capabilities this package depends on.
    pub dependencies: Vec<String>,
    /// Total number of files the package installs.
    pub file_count: usize,
    /// The first [`MAX_ENTRIES`] installed files.
    pub files: Vec<PackageFileEntry>,
}

/// Whether `prefix` opens with the `ar` global header followed by a
/// `debian-binary` first member, the shape every `.deb` file has.
fn looks_like_deb(prefix: &[u8]) -> bool {
    prefix.starts_with(DEB_AR_MAGIC)
        && prefix.get(DEB_AR_MAGIC.len()..DEB_AR_MAGIC.len() + DEB_FIRST_MEMBER_NAME.len())
            == Some(DEB_FIRST_MEMBER_NAME)
}

/// Whether `prefix` opens with the RPM lead's fixed magic number.
fn looks_like_rpm(prefix: &[u8]) -> bool {
    prefix.starts_with(&RPM_MAGIC)
}

/// Decompresses an `ar`/`tar` member's raw bytes based on the compression
/// suffix its name carries (`.gz`, `.xz`, or a bare `.tar`).
fn decompress_member(name: &str, raw: &[u8]) -> io::Result<Vec<u8>> {
    match Path::new(name).extension().and_then(|ext| ext.to_str()) {
        Some("gz") => {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(raw).read_to_end(&mut out)?;
            Ok(out)
        }
        Some("xz") => {
            let mut out = Vec::new();
            lzma_rs::xz_decompress(&mut io::BufReader::new(raw), &mut out)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            Ok(out)
        }
        Some("tar") => Ok(raw.to_vec()),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported compression for deb member {name}"),
        )),
    }
}

/// Reads the text of the tar member whose file name is `target_name`.
fn read_tar_member(tar_bytes: &[u8], target_name: &str) -> io::Result<String> {
    let mut archive = tar::Archive::new(io::Cursor::new(tar_bytes));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let is_match =
            entry.path()?.file_name().and_then(|name| name.to_str()) == Some(target_name);
        if is_match {
            let mut contents = String::new();
            entry.read_to_string(&mut contents)?;
            return Ok(contents);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{target_name} not found in control.tar"),
    ))
}

/// Lists every entry in a tar archive's bytes, up to [`MAX_ENTRIES`], along
/// with the true total count.
fn list_tar_entries(tar_bytes: &[u8]) -> io::Result<(usize, Vec<PackageFileEntry>)> {
    let mut archive = tar::Archive::new(io::Cursor::new(tar_bytes));
    let mut file_count = 0;
    let mut files = Vec::new();
    for entry in archive.entries()? {
        let entry = entry?;
        file_count += 1;
        if files.len() < MAX_ENTRIES {
            let path = entry.path()?.to_string_lossy().into_owned();
            let size = entry.header().size()?;
            files.push(PackageFileEntry { path, size });
        }
    }
    Ok((file_count, files))
}

/// Finds the first line of `control` starting with `field:` and returns the
/// rest of that line, trimmed. Per Debian control file convention, this is
/// enough for `Package`/`Version`/`Architecture`/`Depends`, and, for
/// `Description`, its required one-line synopsis.
fn control_field(control: &str, field: &str) -> Option<String> {
    let prefix = format!("{field}:");
    control.lines().find_map(|line| {
        line.strip_prefix(&prefix)
            .map(|rest| rest.trim().to_owned())
    })
}

/// Reads a `.deb` file's metadata, dependencies, and file list.
fn view_deb(path: &Path) -> io::Result<PackageArchiveView> {
    let file = std::fs::File::open(path)?;
    let mut archive = ar::Archive::new(file);
    let mut control_text = None;
    let mut file_count = 0;
    let mut files = Vec::new();
    while let Some(entry) = archive.next_entry() {
        let mut entry = entry?;
        let name = String::from_utf8_lossy(entry.header().identifier()).into_owned();
        let mut raw = Vec::new();
        if name.starts_with("control.tar") {
            entry.read_to_end(&mut raw)?;
            let decompressed = decompress_member(&name, &raw)?;
            control_text = Some(read_tar_member(&decompressed, "control")?);
        } else if name.starts_with("data.tar") {
            entry.read_to_end(&mut raw)?;
            let decompressed = decompress_member(&name, &raw)?;
            (file_count, files) = list_tar_entries(&decompressed)?;
        }
    }
    let control_text = control_text.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "deb package has no control member",
        )
    })?;
    let dependencies = control_field(&control_text, "Depends")
        .map(|depends| {
            depends
                .split(',')
                .map(|dep| dep.trim().to_owned())
                .collect()
        })
        .unwrap_or_default();
    Ok(PackageArchiveView {
        format: "deb".to_owned(),
        name: control_field(&control_text, "Package").unwrap_or_default(),
        version: control_field(&control_text, "Version").unwrap_or_default(),
        architecture: control_field(&control_text, "Architecture").unwrap_or_default(),
        summary: control_field(&control_text, "Description"),
        dependencies,
        file_count,
        files,
    })
}

/// Maps an [`rpm::Error`] onto an [`io::Error`], matching this project's
/// pattern for a third-party parsing crate's own error type.
fn rpm_error(err: rpm::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

/// Reads an `.rpm` file's metadata, dependencies, and file list, entirely
/// from its header tags, without decompressing the cpio payload.
fn view_rpm(path: &Path) -> io::Result<PackageArchiveView> {
    let package = rpm::Package::open(path).map_err(rpm_error)?;
    let metadata = &package.metadata;
    let version = metadata.get_version().map_err(rpm_error)?;
    let release = metadata.get_release().map_err(rpm_error)?;
    let dependencies = metadata
        .get_requires()
        .map_err(rpm_error)?
        .into_iter()
        .map(|dependency| dependency.to_string())
        .collect();
    let file_entries = metadata.get_file_entries().map_err(rpm_error)?;
    let file_count = file_entries.len();
    let files = file_entries
        .into_iter()
        .take(MAX_ENTRIES)
        .map(|entry| PackageFileEntry {
            path: entry.path().to_string_lossy().into_owned(),
            size: u64::try_from(entry.size()).unwrap_or(u64::MAX),
        })
        .collect();
    Ok(PackageArchiveView {
        format: "rpm".to_owned(),
        name: metadata.get_name().map_err(rpm_error)?.to_owned(),
        version: format!("{version}-{release}"),
        architecture: metadata.get_arch().map_err(rpm_error)?.to_owned(),
        summary: metadata.get_summary().ok().map(str::to_owned),
        dependencies,
        file_count,
        files,
    })
}

/// The OS package plugin's core half. Recognises `.deb` and `.rpm` files.
#[derive(Debug, Default)]
pub struct PackageArchiveCore;

impl PluginCore for PackageArchiveCore {
    fn name(&self) -> &'static str {
        "package-archive"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_deb(prefix) || looks_like_rpm(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let prefix = std::fs::read(path)?;
        let view = if looks_like_deb(&prefix) {
            view_deb(path)?
        } else {
            view_rpm(path)?
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The OS package plugin's presentation half.
#[derive(Debug, Default)]
pub struct PackageArchivePresentation;

impl PluginPresentation for PackageArchivePresentation {
    fn name(&self) -> &'static str {
        "package-archive"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: PackageArchiveView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "{} package: {} {} ({})",
            view.format, view.name, view.version, view.architecture
        )];
        if let Some(summary) = &view.summary {
            lines.push(summary.clone());
        }
        if view.dependencies.is_empty() {
            lines.push("no dependencies".to_owned());
        } else {
            lines.push(format!("depends: {}", view.dependencies.join(", ")));
        }
        lines.push(format!("{} files", view.file_count));
        lines.extend(
            view.files
                .iter()
                .map(|entry| format!("{} ({} bytes)", entry.path, entry.size)),
        );
        if view.file_count > view.files.len() {
            lines.push(format!(
                "... {} more files not shown",
                view.file_count - view.files.len()
            ));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{PackageArchiveCore, PackageArchivePresentation, PackageArchiveView};
    use plugin_api::{PluginCore, PluginPresentation};
    use std::io::Write as _;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-package-archive-test-{}-{name}",
            std::process::id()
        ))
    }

    /// Builds a real, minimal `.deb` file: an `ar` archive carrying
    /// `debian-binary`, a gzip-compressed `control.tar.gz` (with a
    /// `control` file), and a gzip-compressed `data.tar.gz` (with one
    /// installed file), matching the layout `dpkg-deb` itself produces.
    fn write_test_deb(path: &std::path::Path) {
        let mut control_tar = tar::Builder::new(Vec::new());
        let control_contents =
            b"Package: hello\nVersion: 1.0\nArchitecture: amd64\nDepends: libc6 (>= 2.34)\nDescription: a friendly greeting program\n";
        let mut header = tar::Header::new_gnu();
        header.set_path("./control").unwrap();
        header.set_size(control_contents.len() as u64);
        header.set_cksum();
        control_tar.append(&header, &control_contents[..]).unwrap();
        let control_tar = control_tar.into_inner().unwrap();
        let mut control_tar_gz = Vec::new();
        flate2::write::GzEncoder::new(&mut control_tar_gz, flate2::Compression::default())
            .write_all(&control_tar)
            .unwrap();

        let mut data_tar = tar::Builder::new(Vec::new());
        let data_contents = b"#!/bin/sh\necho hello\n";
        let mut header = tar::Header::new_gnu();
        header.set_path("./usr/bin/hello").unwrap();
        header.set_size(data_contents.len() as u64);
        header.set_cksum();
        data_tar.append(&header, &data_contents[..]).unwrap();
        let data_tar = data_tar.into_inner().unwrap();
        let mut data_tar_gz = Vec::new();
        flate2::write::GzEncoder::new(&mut data_tar_gz, flate2::Compression::default())
            .write_all(&data_tar)
            .unwrap();

        let file = std::fs::File::create(path).unwrap();
        let mut builder = ar::Builder::new(file);
        builder
            .append(
                &ar::Header::new(b"debian-binary".to_vec(), 4),
                &b"2.0\n"[..],
            )
            .unwrap();
        builder
            .append(
                &ar::Header::new(b"control.tar.gz".to_vec(), control_tar_gz.len() as u64),
                &control_tar_gz[..],
            )
            .unwrap();
        builder
            .append(
                &ar::Header::new(b"data.tar.gz".to_vec(), data_tar_gz.len() as u64),
                &data_tar_gz[..],
            )
            .unwrap();
    }

    /// Builds a real, minimal `.rpm` file with the `rpm` crate's own
    /// builder: one dependency and one installed file.
    fn write_test_rpm(path: &std::path::Path) {
        let mut builder = rpm::PackageBuilder::new(
            "hello",
            "1.0.0",
            "MIT",
            "x86_64",
            "a friendly greeting program",
        );
        builder.release("1");
        builder.requires(rpm::Dependency::any("glibc"));
        builder
            .with_file_contents(
                b"#!/bin/sh\necho hello\n".to_vec(),
                rpm::FileOptions::new("/usr/bin/hello"),
            )
            .unwrap();
        let package = builder.build().unwrap();
        package.write_file(path).unwrap();
    }

    #[test]
    fn views_a_real_rpm_and_reads_its_metadata_and_files() {
        let path = unique_temp_file("test.rpm");
        write_test_rpm(&path);

        let data = PackageArchiveCore.view(&path).unwrap();
        let view: PackageArchiveView = serde_json::from_value(data).unwrap();

        assert_eq!(view.format, "rpm");
        assert_eq!(view.name, "hello");
        assert_eq!(view.version, "1.0.0-1");
        assert_eq!(view.architecture, "x86_64");
        assert_eq!(view.summary.as_deref(), Some("a friendly greeting program"));
        assert!(view.dependencies.iter().any(|dep| dep.contains("glibc")));
        assert_eq!(view.file_count, view.files.len());
        let hello = view
            .files
            .iter()
            .find(|entry| entry.path == "/usr/bin/hello")
            .expect("the installed file should be listed");
        assert_eq!(hello.size, 21);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn sniffs_a_real_deb_and_ignores_unrelated_content() {
        let path = unique_temp_file("sniff.deb");
        write_test_deb(&path);

        assert!(PackageArchiveCore.sniff(&std::fs::read(&path).unwrap()));
        assert!(!PackageArchiveCore.sniff(b"!<arch>\nnot-debian-binary"));
        assert!(!PackageArchiveCore.sniff(b"not a package"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn sniffs_the_rpm_lead_magic() {
        assert!(PackageArchiveCore.sniff(&[0xed, 0xab, 0xee, 0xdb, 0, 0]));
        assert!(!PackageArchiveCore.sniff(b"not an rpm"));
    }

    #[test]
    fn views_a_real_deb_and_reads_its_metadata_and_files() {
        let path = unique_temp_file("test.deb");
        write_test_deb(&path);

        let data = PackageArchiveCore.view(&path).unwrap();
        let view: PackageArchiveView = serde_json::from_value(data).unwrap();

        assert_eq!(view.format, "deb");
        assert_eq!(view.name, "hello");
        assert_eq!(view.version, "1.0");
        assert_eq!(view.architecture, "amd64");
        assert_eq!(view.summary.as_deref(), Some("a friendly greeting program"));
        assert_eq!(view.dependencies, vec!["libc6 (>= 2.34)"]);
        assert_eq!(view.file_count, 1);
        assert_eq!(view.files[0].path, "usr/bin/hello");
        assert_eq!(view.files[0].size, 21);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_metadata_dependencies_and_files() {
        let data = serde_json::to_value(PackageArchiveView {
            format: "deb".to_owned(),
            name: "hello".to_owned(),
            version: "1.0".to_owned(),
            architecture: "amd64".to_owned(),
            summary: Some("a friendly greeting program".to_owned()),
            dependencies: vec!["libc6 (>= 2.34)".to_owned()],
            file_count: 1,
            files: vec![super::PackageFileEntry {
                path: "usr/bin/hello".to_owned(),
                size: 22,
            }],
        })
        .unwrap();

        let lines = PackageArchivePresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "deb package: hello 1.0 (amd64)",
                "a friendly greeting program",
                "depends: libc6 (>= 2.34)",
                "1 files",
                "usr/bin/hello (22 bytes)",
            ]
        );
    }
}
