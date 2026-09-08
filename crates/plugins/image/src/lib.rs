//! Image file type plugin: core and presentation halves.
//!
//! The view carries a bounded thumbnail alongside the metadata. This was
//! deferred while `plugin_api::PluginPresentation` could only return lines
//! of text; `Graphic` now carries decoded pixels, so an image previews as
//! the picture rather than as the words describing it.
//!
//! The core half does the scaling, via `plugin_api::thumbnail`, so what
//! crosses the wire is bounded rather than being whatever the file happens
//! to be: a photograph is a few hundred kilobytes of PNG here, not the tens
//! of megabytes its raw pixels would be.

use plugin_api::{Graphic, Icon, PluginCore, PluginPresentation, thumbnail};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// View data produced by [`ImageCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageView {
    /// The detected image format, e.g. `"Png"`.
    pub format: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Size of the file on disk, in bytes.
    pub file_size: u64,
    /// A PNG thumbnail, base64-encoded, or `None` if the image could not be
    /// decoded. Encoded rather than raw so the wire stays small.
    #[serde(default)]
    pub thumbnail: Option<String>,
}

/// The file's own picture, scaled for the wire. `None` when it cannot be
/// decoded - the metadata is still worth showing.
fn preview_of(path: &Path) -> Option<String> {
    thumbnail::encode(&image::ImageReader::open(path).ok()?.decode().ok()?)
}

/// The image plugin's core half.
#[derive(Debug, Default)]
pub struct ImageCore;

impl PluginCore for ImageCore {
    fn name(&self) -> &'static str {
        "image"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        image::guess_format(prefix).is_ok()
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let file_size = std::fs::metadata(path)?.len();
        let reader = image::ImageReader::open(path)?.with_guessed_format()?;
        let format = reader
            .format()
            .map_or_else(|| "unknown".to_owned(), |format| format!("{format:?}"));
        let (width, height) = reader
            .into_dimensions()
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let view = ImageView {
            format,
            width,
            height,
            file_size,
            thumbnail: preview_of(path),
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The image plugin's presentation half.
#[derive(Debug, Default)]
pub struct ImagePresentation;

impl PluginPresentation for ImagePresentation {
    fn name(&self) -> &'static str {
        "image"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "IMG",
            tint: 0x00db_2777,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["png", "jpg", "jpeg", "gif", "bmp", "webp", "ico", "tiff"]
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        match serde_json::from_value::<ImageView>(data.clone()) {
            Ok(view) => vec![
                format!("{} image", view.format),
                format!("{} x {} pixels", view.width, view.height),
                format!("{} bytes on disk", view.file_size),
            ],
            Err(err) => vec![format!("could not read view data: {err}")],
        }
    }

    fn graphic(&self, data: &serde_json::Value) -> Option<Graphic> {
        let view: ImageView = serde_json::from_value(data.clone()).ok()?;
        thumbnail::decode(&view.thumbnail?)
    }
}

#[cfg(test)]
mod tests {
    use super::{ImageCore, ImagePresentation, ImageView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-image-test-{}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn sniffs_a_real_png_header() {
        let png_magic = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert!(ImageCore.sniff(&png_magic));
        assert!(!ImageCore.sniff(b"not an image"));
    }

    #[test]
    fn views_a_real_png_file() {
        let path = unique_temp_file("test.png");
        let img = image::RgbImage::new(4, 3);
        img.save(&path).unwrap();

        let data = ImageCore.view(&path).unwrap();
        let view: ImageView = serde_json::from_value(data).unwrap();

        assert_eq!(view.width, 4);
        assert_eq!(view.height, 3);
        assert_eq!(view.format, "Png");
        assert!(view.file_size > 0);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_dimensions_and_format() {
        let data = serde_json::to_value(ImageView {
            format: "Png".to_owned(),
            width: 10,
            height: 20,
            file_size: 123,
            thumbnail: None,
        })
        .unwrap();

        let lines = ImagePresentation.present(&data);

        assert_eq!(lines[0], "Png image");
        assert_eq!(lines[1], "10 x 20 pixels");
        assert_eq!(lines[2], "123 bytes on disk");
    }
}
