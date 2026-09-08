//! Photoshop document file type plugin: core and presentation halves.
//!
//! Presents a layer panel (the document's layer names, in stacking order),
//! distinct from the flat raster presentation of the existing image plugin,
//! and the composited document itself as a picture: a Photoshop file's whole
//! point is the image it flattens to.

use plugin_api::{Graphic, Icon, PluginCore, PluginPresentation, thumbnail};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["psd"];

/// The PSD/PSB signature, the first four bytes of every Photoshop document.
const PSD_SIGNATURE: &[u8] = b"8BPS";

/// View data produced by [`PsdCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PsdView {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Layer names, in the document's stacking order (bottom to top).
    pub layers: Vec<String>,
    /// The composited document as a bounded PNG thumbnail, base64-encoded.
    /// `None` when it cannot be flattened - the layer panel is still worth
    /// showing.
    #[serde(default)]
    pub composite: Option<String>,
}

/// The document flattened to a picture, scaled for the wire.
fn composite_of(document: &psd::Psd) -> Option<String> {
    let rgba = document.rgba();
    let buffer = image::RgbaImage::from_raw(document.width(), document.height(), rgba)?;
    thumbnail::encode(&image::DynamicImage::ImageRgba8(buffer))
}

/// The PSD plugin's core half.
#[derive(Debug, Default)]
pub struct PsdCore;

impl PluginCore for PsdCore {
    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn name(&self) -> &'static str {
        "psd"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        prefix.starts_with(PSD_SIGNATURE)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let document = psd::Psd::from_bytes(&bytes)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
        let layers = document
            .layers()
            .iter()
            .map(|layer| layer.name().to_owned())
            .collect();
        let view = PsdView {
            width: document.width(),
            height: document.height(),
            layers,
            composite: composite_of(&document),
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The PSD plugin's presentation half.
#[derive(Debug, Default)]
pub struct PsdPresentation;

impl PluginPresentation for PsdPresentation {
    fn name(&self) -> &'static str {
        "psd"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "PSD",
            tint: 0x0031_a8ff,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        match serde_json::from_value::<PsdView>(data.clone()) {
            Ok(view) => {
                let mut lines = vec![format!("{} x {} pixels", view.width, view.height)];
                lines.push(format!("{} layers", view.layers.len()));
                lines.extend(view.layers.iter().map(|name| format!("- {name}")));
                lines
            }
            Err(err) => vec![format!("could not read view data: {err}")],
        }
    }

    fn graphic(&self, data: &serde_json::Value) -> Option<Graphic> {
        let view: PsdView = serde_json::from_value(data.clone()).ok()?;
        thumbnail::decode(&view.composite?)
    }
}

#[cfg(test)]
mod tests {
    use super::{PSD_SIGNATURE, PsdCore, PsdPresentation, PsdView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rse-plugin-psd-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn sniffs_a_real_psd_signature() {
        assert!(PsdCore.sniff(PSD_SIGNATURE));
        assert!(!PsdCore.sniff(b"not a psd"));
    }

    #[test]
    fn views_a_real_psd_file() {
        let path = unique_temp_file("test.psd");
        std::fs::write(&path, minimal_psd_bytes()).unwrap();

        let data = PsdCore.view(&path).unwrap();
        let view: PsdView = serde_json::from_value(data).unwrap();

        assert_eq!(view.width, 2);
        assert_eq!(view.height, 1);
        assert_eq!(view.layers, vec!["Layer 1"]);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_dimensions_and_layer_panel() {
        let data = serde_json::to_value(PsdView {
            width: 10,
            height: 20,
            layers: vec!["Background".to_owned(), "Text".to_owned()],
            composite: None,
        })
        .unwrap();

        let lines = PsdPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["10 x 20 pixels", "2 layers", "- Background", "- Text"]
        );
    }

    /// Builds a minimal but spec-valid single-layer 2x1 RGB, 8-bit-depth PSD
    /// file: header, empty color mode data and image resources sections, one
    /// layer named "Layer 1" covering the whole canvas, and a raw composite
    /// image.
    fn minimal_psd_bytes() -> Vec<u8> {
        let mut file = Vec::new();

        // Header (26 bytes).
        file.extend_from_slice(b"8BPS");
        file.extend_from_slice(&1u16.to_be_bytes()); // version
        file.extend_from_slice(&[0u8; 6]); // reserved
        file.extend_from_slice(&3u16.to_be_bytes()); // channels
        file.extend_from_slice(&1u32.to_be_bytes()); // height
        file.extend_from_slice(&2u32.to_be_bytes()); // width
        file.extend_from_slice(&8u16.to_be_bytes()); // depth
        file.extend_from_slice(&3u16.to_be_bytes()); // color mode: RGB

        // Color mode data section: empty.
        file.extend_from_slice(&0u32.to_be_bytes());

        // Image resources section: empty.
        file.extend_from_slice(&0u32.to_be_bytes());

        // Layer and mask information section.
        let layer_name = b"Layer 1";
        let mut extra_data = Vec::new();
        extra_data.extend_from_slice(&0u32.to_be_bytes()); // layer mask data length
        extra_data.extend_from_slice(&0u32.to_be_bytes()); // layer blending ranges length
        extra_data.push(u8::try_from(layer_name.len()).unwrap()); // Pascal string length
        extra_data.extend_from_slice(layer_name); // already a multiple of 4 (1 + 7)

        let channel_data_len: u32 = 2 + 2; // 2-byte compression marker + 2 one-byte pixels
        let mut layer_record = Vec::new();
        layer_record.extend_from_slice(&0i32.to_be_bytes()); // top
        layer_record.extend_from_slice(&0i32.to_be_bytes()); // left
        layer_record.extend_from_slice(&1i32.to_be_bytes()); // bottom
        layer_record.extend_from_slice(&2i32.to_be_bytes()); // right
        layer_record.extend_from_slice(&3u16.to_be_bytes()); // number of channels
        for channel_id in [0i16, 1, 2] {
            layer_record.extend_from_slice(&channel_id.to_be_bytes());
            layer_record.extend_from_slice(&channel_data_len.to_be_bytes());
        }
        layer_record.extend_from_slice(b"8BIM"); // blend mode signature
        layer_record.extend_from_slice(b"norm"); // blend mode key
        layer_record.push(255); // opacity
        layer_record.push(0); // clipping
        layer_record.push(0); // flags
        layer_record.push(0); // filler
        layer_record.extend_from_slice(&u32::try_from(extra_data.len()).unwrap().to_be_bytes());
        layer_record.extend_from_slice(&extra_data);

        let mut channel_image_data = Vec::new();
        for plane in [[255u8, 255u8], [0u8, 0u8], [0u8, 0u8]] {
            channel_image_data.extend_from_slice(&0u16.to_be_bytes()); // raw compression
            channel_image_data.extend_from_slice(&plane);
        }

        let mut layer_info = Vec::new();
        layer_info.extend_from_slice(&1i16.to_be_bytes()); // layer count
        layer_info.extend_from_slice(&layer_record);
        layer_info.extend_from_slice(&channel_image_data);

        let mut layer_and_mask_info = Vec::new();
        layer_and_mask_info
            .extend_from_slice(&u32::try_from(layer_info.len()).unwrap().to_be_bytes());
        layer_and_mask_info.extend_from_slice(&layer_info);
        layer_and_mask_info.extend_from_slice(&0u32.to_be_bytes()); // global layer mask info: empty

        file.extend_from_slice(
            &u32::try_from(layer_and_mask_info.len())
                .unwrap()
                .to_be_bytes(),
        );
        file.extend_from_slice(&layer_and_mask_info);

        // Image data section: raw composite image, one plane per channel.
        file.extend_from_slice(&0u16.to_be_bytes()); // raw compression
        file.extend_from_slice(&[255, 255]); // red plane
        file.extend_from_slice(&[0, 0]); // green plane
        file.extend_from_slice(&[0, 0]); // blue plane

        file
    }

    #[test]
    fn a_document_offers_its_composite_as_drawable_pixels() {
        let bytes = include_bytes!("../../../../samples/psd/sample-artwork.psd");
        let path = unique_temp_file("composite.psd");
        std::fs::write(&path, bytes).unwrap();

        let data = PsdCore.view(&path).unwrap();
        let graphic = PsdPresentation
            .graphic(&data)
            .expect("a psd composites to a picture");

        match graphic {
            plugin_api::Graphic::Rgba {
                width,
                height,
                pixels,
            } => {
                assert!(width <= plugin_api::thumbnail::THUMBNAIL_EDGE);
                assert!(height <= plugin_api::thumbnail::THUMBNAIL_EDGE);
                assert_eq!(pixels.len(), width as usize * height as usize * 4);
            }
            plugin_api::Graphic::Svg(source) => panic!("expected pixels, got {source:.40}"),
        }

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_document_that_cannot_be_composited_still_presents_its_layers() {
        let data = serde_json::json!({
            "width": 4,
            "height": 4,
            "layers": ["Background"],
            "composite": serde_json::Value::Null,
        });

        assert!(PsdPresentation.graphic(&data).is_none());
        assert!(
            !PsdPresentation.present(&data).is_empty(),
            "the layer panel is still worth showing"
        );
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::PsdCore),
            plugin_api::PluginPresentation::extensions(&crate::PsdPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
