//! Image file type plugin: core and presentation halves.
//!
//! The view is metadata only (format, dimensions, size on disk), not a
//! rendered thumbnail: `plugin-api`'s presentation half returns lines of
//! text, a front-end-agnostic shape chosen in step 3, and pixel rendering
//! doesn't fit it. A real thumbnail would need the presentation trait to
//! carry richer, toolkit-specific data - deferred until a plugin actually
//! needs it.

use plugin_api::{PluginCore, PluginPresentation};
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
        })
        .unwrap();

        let lines = ImagePresentation.present(&data);

        assert_eq!(lines[0], "Png image");
        assert_eq!(lines[1], "10 x 20 pixels");
        assert_eq!(lines[2], "123 bytes on disk");
    }
}
