//! Comic book archive file type plugin: core and presentation halves.
//!
//! `.cbz`/`.cbr` are a ZIP or RAR archive of page images with no manifest of
//! their own, so this plugin's view is a page list (image entries only, in
//! name order) rather than the generic `archive` plugin's flat entry
//! listing - the shape a page-by-page comic reader needs, per the issue's
//! direction.

use plugin_api::{Graphic, Icon, PluginCore, PluginPresentation, thumbnail};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["cbz", "cbr"];

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
    /// The first page as a bounded PNG thumbnail, base64-encoded. `None`
    /// when it cannot be decoded - the page listing is still worth showing.
    #[serde(default)]
    pub cover: Option<String>,
}

/// The bytes of the entry named `name` in the ZIP archive at `path`.
fn read_zip_entry(path: &Path, name: &str) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut entry = archive.by_name(name).ok()?;
    let mut bytes = Vec::new();
    io::Read::read_to_end(&mut entry, &mut bytes).ok()?;
    Some(bytes)
}

/// The bytes of the member named `name` in the RAR archive at `path`.
fn read_rar_entry(path: &Path, name: &str) -> Option<Vec<u8>> {
    let archive = rars::ArchiveReader::read_path(path).ok()?;
    archive.read_member(name.as_bytes(), None).ok().flatten()
}

/// The first page, scaled for the wire. The cover is the page a reader sees
/// first, so it is taken from the sorted list rather than from whichever
/// entry the archive happens to store first.
fn cover_of(path: &Path, is_rar: bool, pages: &[ComicPage]) -> Option<String> {
    let first = pages.first()?;
    let bytes = if is_rar {
        read_rar_entry(path, &first.name)?
    } else {
        read_zip_entry(path, &first.name)?
    };
    thumbnail::encode_bytes(&bytes)
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
    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

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
        let is_rar = rars::detect_archive_family(&prefix[..read]).is_some();
        let mut pages = if is_rar {
            read_rar_pages(path)?
        } else {
            read_zip_pages(path)?
        };
        pages.sort_by(|a, b| a.name.cmp(&b.name));
        let page_count = pages.len();
        pages.truncate(MAX_PAGES);
        let cover = cover_of(path, is_rar, &pages);
        let view = ComicArchiveView {
            page_count,
            pages,
            cover,
        };
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

    fn icon(&self) -> Icon {
        Icon {
            label: "CBZ",
            tint: 0x00f9_7316,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
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

    fn graphic(&self, data: &serde_json::Value) -> Option<Graphic> {
        let view: ComicArchiveView = serde_json::from_value(data.clone()).ok()?;
        thumbnail::decode(&view.cover?)
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
            cover: None,
        })
        .unwrap();

        let lines = ComicArchivePresentation.present(&data);

        assert_eq!(lines, vec!["1 pages", "Page 1: page001.jpg (5 bytes)"]);
    }

    /// A solid PNG of the given size, for a page that is a real picture.
    fn page_png(width: u32, height: u32) -> Vec<u8> {
        let mut png = std::io::Cursor::new(Vec::new());
        let buffer = image::RgbaImage::from_pixel(width, height, image::Rgba([9, 9, 9, 255]));
        image::DynamicImage::ImageRgba8(buffer)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        png.into_inner()
    }

    /// A cbz whose pages are real images, stored out of order so the cover
    /// has to come from the sorted listing rather than from storage order.
    fn write_cbz_of_images(path: &std::path::Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("page002.png", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&page_png(8, 4)).unwrap();
        writer
            .start_file("page001.png", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&page_png(4, 2)).unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn the_cover_is_the_first_page_in_reading_order() {
        let path = unique_temp_file("cover.cbz");
        write_cbz_of_images(&path);

        let data = ComicArchiveCore.view(&path).unwrap();
        let view: ComicArchiveView = serde_json::from_value(data.clone()).unwrap();
        let graphic = ComicArchivePresentation
            .graphic(&data)
            .expect("a comic archive offers its first page");

        match graphic {
            plugin_api::Graphic::Rgba {
                width,
                height,
                pixels,
            } => {
                assert_eq!(pixels.len(), width as usize * height as usize * 4);
                assert_eq!(
                    (width, height),
                    (4, 2),
                    "page001, the first in reading order, not page002 which is stored first"
                );
            }
            plugin_api::Graphic::Svg(source) => panic!("expected pixels, got {source:.40}"),
        }
        assert_eq!(
            view.pages.first().map(|page| page.name.as_str()),
            Some("page001.png"),
            "and the listing agrees on which page is first"
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn an_archive_with_no_decodable_first_page_still_lists_its_pages() {
        let data = serde_json::json!({
            "page_count": 1,
            "pages": [{ "name": "page001.jpg", "size": 10 }],
            "cover": serde_json::Value::Null,
        });

        assert!(ComicArchivePresentation.graphic(&data).is_none());
        assert!(!ComicArchivePresentation.present(&data).is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::ComicArchiveCore),
            plugin_api::PluginPresentation::extensions(&crate::ComicArchivePresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
