//! Comic book archive file type plugin: core and presentation halves.
//!
//! `.cbz`/`.cbr` are a ZIP or RAR archive of page images with no manifest of
//! their own, so this plugin's view is a page list (image entries only, in
//! name order) rather than the generic `archive` plugin's flat entry
//! listing - the shape a page-by-page comic reader needs, per the issue's
//! direction.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of pages listed in the view; comics with more are
/// truncated, matching `archive`'s own entry limit.
const MAX_PAGES: usize = 200;

/// Recognised page image extensions, checked case-insensitively.
const IMAGE_EXTENSIONS: &[&str] = &[".jpg", ".jpeg", ".png", ".gif", ".bmp", ".webp"];

/// Whether `name` ends in one of [`IMAGE_EXTENSIONS`].
fn is_page_image(name: &str) -> bool {
    let lower = name.to_lowercase();
    IMAGE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

/// One page in a comic archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComicPage {
    /// The page image's path within the archive.
    pub name: String,
    /// Uncompressed size in bytes.
    pub size: u64,
}

/// View data produced by [`ComicArchiveCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComicArchiveView {
    /// Total number of page images in the archive.
    pub page_count: usize,
    /// The first [`MAX_PAGES`] pages, in name order.
    pub pages: Vec<ComicPage>,
}

/// Reads every image entry out of the ZIP archive at `path`, unsorted.
fn read_zip_pages(path: &Path) -> io::Result<Vec<ComicPage>> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let mut pages = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        if entry.is_dir() || !is_page_image(entry.name()) {
            continue;
        }
        pages.push(ComicPage {
            name: entry.name().to_owned(),
            size: entry.size(),
        });
    }
    Ok(pages)
}

/// Reads every image entry out of the RAR archive at `path`, unsorted.
fn read_rar_pages(path: &Path) -> io::Result<Vec<ComicPage>> {
    let archive = rars::ArchiveReader::read_path(path)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let mut pages = Vec::new();
    for member in archive.members() {
        if member.meta.is_directory {
            continue;
        }
        let name = String::from_utf8_lossy(&member.meta.name).into_owned();
        if !is_page_image(&name) {
            continue;
        }
        pages.push(ComicPage {
            name,
            size: member.meta.unpacked_size,
        });
    }
    Ok(pages)
}

/// The comic book archive plugin's core half. Recognises ZIP (`.cbz`) and
/// RAR (`.cbr`) archives of page images.
#[derive(Debug, Default)]
pub struct ComicArchiveCore;

impl PluginCore for ComicArchiveCore {
    fn name(&self) -> &'static str {
        "comic-archive"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        if rars::detect_archive_family(prefix).is_some() {
            return true;
        }
        if !prefix.starts_with(b"PK\x03\x04") {
            return false;
        }
        let lower = String::from_utf8_lossy(prefix).to_lowercase();
        IMAGE_EXTENSIONS.iter().any(|ext| lower.contains(ext))
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let mut prefix = [0u8; 8];
        let read = io::Read::read(&mut std::fs::File::open(path)?, &mut prefix)?;
        let mut pages = if rars::detect_archive_family(&prefix[..read]).is_some() {
            read_rar_pages(path)?
        } else {
            read_zip_pages(path)?
        };
        pages.sort_by(|a, b| a.name.cmp(&b.name));
        let page_count = pages.len();
        pages.truncate(MAX_PAGES);
        let view = ComicArchiveView { page_count, pages };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The comic book archive plugin's presentation half.
#[derive(Debug, Default)]
pub struct ComicArchivePresentation;

impl PluginPresentation for ComicArchivePresentation {
    fn name(&self) -> &'static str {
        "comic-archive"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: ComicArchiveView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!("{} pages", view.page_count)];
        lines.extend(view.pages.iter().enumerate().map(|(index, page)| {
            format!("Page {}: {} ({} bytes)", index + 1, page.name, page.size)
        }));
        if view.page_count > view.pages.len() {
            lines.push(format!(
                "... {} more pages not shown",
                view.page_count - view.pages.len()
            ));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{ComicArchiveCore, ComicArchivePresentation, ComicArchiveView, ComicPage};
    use plugin_api::{PluginCore, PluginPresentation};
    use std::io::Write;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-comic-archive-test-{}-{name}",
            std::process::id()
        ))
    }

    fn write_test_cbz(path: &std::path::Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("page002.jpg", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"second page bytes").unwrap();
        writer
            .start_file("page001.jpg", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"first page").unwrap();
        writer
            .start_file("ComicInfo.xml", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"<ComicInfo/>").unwrap();
        writer.finish().unwrap();
    }

    fn write_test_cbr(path: &std::path::Path) {
        let mut builder = rars::builder::Builder::new(rars::version::ArchiveVersion::Rar50);
        builder
            .add_bytes(
                b"page002.png".to_vec(),
                b"second page bytes".to_vec(),
                None,
                None,
            )
            .unwrap();
        builder
            .add_bytes(b"page001.png".to_vec(), b"first page".to_vec(), None, None)
            .unwrap();
        let bytes = builder.to_bytes().unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn sniffs_a_zip_comic_archive_by_an_image_entry_name() {
        let path = unique_temp_file("sniff.cbz");
        write_test_cbz(&path);
        let prefix = std::fs::read(&path).unwrap();

        assert!(ComicArchiveCore.sniff(&prefix));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn does_not_sniff_a_plain_zip_archive() {
        let path = unique_temp_file("plain.zip");
        let file = std::fs::File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("readme.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"not a comic").unwrap();
        writer.finish().unwrap();
        let prefix = std::fs::read(&path).unwrap();

        assert!(!ComicArchiveCore.sniff(&prefix));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn sniffs_a_rar_comic_archive_by_its_magic() {
        let path = unique_temp_file("sniff.cbr");
        write_test_cbr(&path);
        let prefix = std::fs::read(&path).unwrap();

        assert!(ComicArchiveCore.sniff(&prefix));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_real_cbz_in_page_order_excluding_non_images() {
        let path = unique_temp_file("test.cbz");
        write_test_cbz(&path);

        let data = ComicArchiveCore.view(&path).unwrap();
        let view: ComicArchiveView = serde_json::from_value(data).unwrap();

        assert_eq!(view.page_count, 2);
        assert_eq!(view.pages[0].name, "page001.jpg");
        assert_eq!(view.pages[0].size, 10);
        assert_eq!(view.pages[1].name, "page002.jpg");

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_real_cbr_in_page_order() {
        let path = unique_temp_file("test.cbr");
        write_test_cbr(&path);

        let data = ComicArchiveCore.view(&path).unwrap();
        let view: ComicArchiveView = serde_json::from_value(data).unwrap();

        assert_eq!(view.page_count, 2);
        assert_eq!(view.pages[0].name, "page001.png");
        assert_eq!(view.pages[1].name, "page002.png");

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_page_count_and_names() {
        let data = serde_json::to_value(ComicArchiveView {
            page_count: 1,
            pages: vec![ComicPage {
                name: "page001.jpg".to_owned(),
                size: 5,
            }],
        })
        .unwrap();

        let lines = ComicArchivePresentation.present(&data);

        assert_eq!(lines, vec!["1 pages", "Page 1: page001.jpg (5 bytes)"]);
    }
}
