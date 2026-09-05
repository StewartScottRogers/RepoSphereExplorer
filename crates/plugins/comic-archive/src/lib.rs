//! Comic book archive file type plugin: core and presentation halves.
//!
//! CBZ and CBR are conventions, not specified formats: a CBZ is simply a ZIP
//! of image files and a CBR simply a RAR of image files, with nothing in
//! either container marking it as a comic (unlike, say, EPUB's mandated
//! mimetype entry). Sniffing therefore trusts RAR's magic outright — no
//! other registered plugin claims it — but for ZIP, which the generic
//! archive plugin also claims, this core additionally requires the first
//! entry's name to carry an image extension before it will claim the file.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::io::Read as _;
use std::path::Path;

/// RAR 1.5-4.x archive signature.
const RAR4_MAGIC: &[u8] = b"Rar!\x1a\x07\x00";
/// RAR 5.0+ archive signature.
const RAR5_MAGIC: &[u8] = b"Rar!\x1a\x07\x01\x00";

/// Extensions this plugin treats as comic pages.
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp"];

/// Whether `name`'s extension is one of [`IMAGE_EXTENSIONS`], case-insensitively.
fn has_image_extension(name: &str) -> bool {
    name.rsplit('.')
        .next()
        .is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Reads the first entry's filename directly out of a ZIP local file
/// header's fixed 30-byte layout, without pulling in the `zip` crate for
/// what `sniff` only has a byte prefix (not a seekable file) to work with.
fn first_zip_entry_name(prefix: &[u8]) -> Option<&str> {
    let name_len = usize::from(u16::from_le_bytes(prefix.get(26..28)?.try_into().ok()?));
    let name = prefix.get(30..30 + name_len)?;
    std::str::from_utf8(name).ok()
}

/// One page (image entry) in a comic archive, in reading order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComicPage {
    /// The page's 1-based position in reading order.
    pub number: usize,
    /// The page's entry name within the archive.
    pub name: String,
}

/// View data produced by [`ComicArchiveCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComicArchiveView {
    /// Total number of pages (image entries) in the archive.
    pub page_count: usize,
    /// Every page, in reading order (sorted by entry name).
    pub pages: Vec<ComicPage>,
}

/// Lists image entries in a ZIP-based (CBZ) comic archive.
fn read_cbz_pages(path: &Path) -> io::Result<Vec<String>> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let mut names = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        if entry.is_file() && has_image_extension(entry.name()) {
            names.push(entry.name().to_owned());
        }
    }
    Ok(names)
}

/// Lists image entries in a RAR-based (CBR) comic archive.
fn read_cbr_pages(path: &Path) -> io::Result<Vec<String>> {
    let open = unrar::Archive::new(path)
        .open_for_listing()
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let mut names = Vec::new();
    for header in open {
        let header = header.map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let name = header.filename.to_string_lossy().into_owned();
        if header.is_file() && has_image_extension(&name) {
            names.push(name);
        }
    }
    Ok(names)
}

/// Reads a comic archive's pages, dispatching to the ZIP or RAR reader by
/// the file's own magic bytes.
fn read_book(path: &Path) -> io::Result<ComicArchiveView> {
    let mut magic = [0u8; 8];
    let read = std::fs::File::open(path)?.read(&mut magic)?;
    let magic = &magic[..read];

    let mut names = if magic.starts_with(RAR4_MAGIC) || magic.starts_with(RAR5_MAGIC) {
        read_cbr_pages(path)?
    } else if magic.starts_with(b"PK\x03\x04") {
        read_cbz_pages(path)?
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a recognised comic book archive",
        ));
    };
    names.sort();

    let pages = names
        .into_iter()
        .enumerate()
        .map(|(index, name)| ComicPage {
            number: index + 1,
            name,
        })
        .collect::<Vec<_>>();
    Ok(ComicArchiveView {
        page_count: pages.len(),
        pages,
    })
}

/// The comic book archive plugin's core half. Recognises CBZ (ZIP) and CBR
/// (RAR) comic archives.
#[derive(Debug, Default)]
pub struct ComicArchiveCore;

impl PluginCore for ComicArchiveCore {
    fn name(&self) -> &'static str {
        "comic-archive"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        if prefix.starts_with(RAR4_MAGIC) || prefix.starts_with(RAR5_MAGIC) {
            return true;
        }
        prefix.starts_with(b"PK\x03\x04")
            && first_zip_entry_name(prefix).is_some_and(has_image_extension)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let view = read_book(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The comic book archive plugin's presentation half: a page-by-page
/// reader, distinct from the generic archive plugin's flat entry listing.
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

        if view.pages.is_empty() {
            return vec!["no pages".to_owned()];
        }

        let mut lines = vec![format!("{} pages", view.page_count)];
        lines.extend(
            view.pages
                .iter()
                .map(|page| format!("{}: {}", page.number, page.name)),
        );
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{ComicArchiveCore, ComicArchivePresentation, ComicArchiveView, ComicPage};
    use plugin_api::{PluginCore, PluginPresentation};
    use std::io::Write as _;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-comic-archive-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write_test_cbz(path: &std::path::Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();

        writer.start_file("page002.jpg", opts).unwrap();
        writer.write_all(b"second page bytes").unwrap();

        writer.start_file("page001.jpg", opts).unwrap();
        writer.write_all(b"first page bytes").unwrap();

        writer.start_file("ComicInfo.xml", opts).unwrap();
        writer.write_all(b"<ComicInfo/>").unwrap();

        writer.finish().unwrap();
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for &byte in data {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                let mask = 0u32.wrapping_sub(crc & 1);
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }

    /// Hand-assembles a minimal, valid RAR 4.x archive (signature,
    /// `MAIN_HEAD`, one `FILE_HEAD` per stored entry, `END_ARC_HEAD`), since
    /// the `unrar` crate only reads RAR archives and there is no RAR
    /// encoder crate available to build one instead.
    fn write_test_cbr(path: &std::path::Path, entries: &[(&str, &[u8])]) {
        let mut out = Vec::new();
        out.extend_from_slice(b"Rar!\x1a\x07\x00");

        let mut main_head = Vec::new();
        main_head.push(0x73u8);
        main_head.extend_from_slice(&0u16.to_le_bytes());
        main_head.extend_from_slice(&13u16.to_le_bytes());
        main_head.extend_from_slice(&0u16.to_le_bytes());
        main_head.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&((crc32(&main_head) & 0xFFFF) as u16).to_le_bytes());
        out.extend_from_slice(&main_head);

        for (name, data) in entries {
            let name_bytes = name.as_bytes();
            let mut body = Vec::new();
            body.push(0x74u8);
            body.extend_from_slice(&0x8000u16.to_le_bytes());
            let name_len = u16::try_from(name_bytes.len()).unwrap();
            let head_size = 7u16 + 4 + 4 + 1 + 4 + 4 + 1 + 1 + 2 + 4 + name_len;
            body.extend_from_slice(&head_size.to_le_bytes());
            let data_len = u32::try_from(data.len()).unwrap();
            body.extend_from_slice(&data_len.to_le_bytes());
            body.extend_from_slice(&data_len.to_le_bytes());
            body.push(0);
            body.extend_from_slice(&crc32(data).to_le_bytes());
            body.extend_from_slice(&0u32.to_le_bytes());
            body.push(20);
            body.push(0x30);
            body.extend_from_slice(&name_len.to_le_bytes());
            body.extend_from_slice(&0x20u32.to_le_bytes());
            body.extend_from_slice(name_bytes);

            out.extend_from_slice(&((crc32(&body) & 0xFFFF) as u16).to_le_bytes());
            out.extend_from_slice(&body);
            out.extend_from_slice(data);
        }

        let mut end_head = Vec::new();
        end_head.push(0x7Bu8);
        end_head.extend_from_slice(&0u16.to_le_bytes());
        end_head.extend_from_slice(&7u16.to_le_bytes());
        out.extend_from_slice(&((crc32(&end_head) & 0xFFFF) as u16).to_le_bytes());
        out.extend_from_slice(&end_head);

        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn sniffs_cbz_by_its_first_entrys_image_extension() {
        let path = unique_temp_file("sniff.cbz");
        write_test_cbz(&path);
        let prefix = std::fs::read(&path).unwrap();

        assert!(ComicArchiveCore.sniff(&prefix));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn does_not_sniff_a_plain_zip_archive_of_non_images() {
        let path = unique_temp_file("plain.zip");
        let file = std::fs::File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("readme.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"hello").unwrap();
        writer.finish().unwrap();
        let prefix = std::fs::read(&path).unwrap();

        assert!(!ComicArchiveCore.sniff(&prefix));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn sniffs_cbr_by_its_rar_magic() {
        assert!(ComicArchiveCore.sniff(b"Rar!\x1a\x07\x00rest"));
        assert!(ComicArchiveCore.sniff(b"Rar!\x1a\x07\x01\x00rest"));
        assert!(!ComicArchiveCore.sniff(b"not a comic archive"));
    }

    #[test]
    fn views_a_real_cbz_book_sorted_into_reading_order() {
        let path = unique_temp_file("test.cbz");
        write_test_cbz(&path);

        let data = ComicArchiveCore.view(&path).unwrap();
        let view: ComicArchiveView = serde_json::from_value(data).unwrap();

        assert_eq!(
            view,
            ComicArchiveView {
                page_count: 2,
                pages: vec![
                    ComicPage {
                        number: 1,
                        name: "page001.jpg".to_owned(),
                    },
                    ComicPage {
                        number: 2,
                        name: "page002.jpg".to_owned(),
                    },
                ],
            }
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_real_cbr_book_sorted_into_reading_order() {
        let path = unique_temp_file("test.cbr");
        write_test_cbr(
            &path,
            &[
                ("page002.png", b"second page"),
                ("page001.png", b"first page"),
            ],
        );

        let data = ComicArchiveCore.view(&path).unwrap();
        let view: ComicArchiveView = serde_json::from_value(data).unwrap();

        assert_eq!(
            view,
            ComicArchiveView {
                page_count: 2,
                pages: vec![
                    ComicPage {
                        number: 1,
                        name: "page001.png".to_owned(),
                    },
                    ComicPage {
                        number: 2,
                        name: "page002.png".to_owned(),
                    },
                ],
            }
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_page_count_and_reading_order() {
        let data = serde_json::to_value(ComicArchiveView {
            page_count: 2,
            pages: vec![
                ComicPage {
                    number: 1,
                    name: "page001.jpg".to_owned(),
                },
                ComicPage {
                    number: 2,
                    name: "page002.jpg".to_owned(),
                },
            ],
        })
        .unwrap();

        let lines = ComicArchivePresentation.present(&data);

        assert_eq!(lines, vec!["2 pages", "1: page001.jpg", "2: page002.jpg"]);
    }

    #[test]
    fn presents_no_pages_for_an_empty_archive() {
        let data = serde_json::to_value(ComicArchiveView {
            page_count: 0,
            pages: vec![],
        })
        .unwrap();

        let lines = ComicArchivePresentation.present(&data);

        assert_eq!(lines, vec!["no pages"]);
    }
}
