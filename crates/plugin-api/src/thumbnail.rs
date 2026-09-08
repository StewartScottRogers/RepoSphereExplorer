//! Turning a picture into something a preview pane can draw.
//!
//! Five plugins need the same two steps - scale a decoded image down to
//! something a pane can show, and carry it over the wire - so they live here
//! rather than five times over. The scaling belongs in a plugin's *core*
//! half: a photograph is tens of megabytes of raw pixels and a few hundred
//! kilobytes of PNG, and only the core half is in a position to shrink it
//! before it crosses the wire.

use crate::Graphic;
use base64::Engine as _;

/// Longest edge of a thumbnail. Big enough to read at the size a preview
/// pane gives it, small enough that the wire does not carry a photograph.
pub const THUMBNAIL_EDGE: u32 = 512;

/// Scales `image` down to fit [`THUMBNAIL_EDGE`] and encodes it as PNG,
/// base64 for the wire. `None` if it cannot be encoded.
///
/// Down only. `DynamicImage::thumbnail` scales to the largest size that
/// fits the bounds, which *enlarges* anything smaller - a 16x16 icon came
/// back as a blurry 512x512 - and a preview should show a small picture at
/// its own size.
#[must_use]
pub fn encode(image: &image::DynamicImage) -> Option<String> {
    let oversized = image.width() > THUMBNAIL_EDGE || image.height() > THUMBNAIL_EDGE;
    let scaled = if oversized {
        image.thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE)
    } else {
        image.clone()
    };
    let mut png = std::io::Cursor::new(Vec::new());
    scaled.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(base64::engine::general_purpose::STANDARD.encode(png.into_inner()))
}

/// As [`encode`], from a picture still in its own file format - a page out
/// of a comic archive, a cover embedded in a music file. `None` if the bytes
/// are not a picture this build can decode.
#[must_use]
pub fn encode_bytes(bytes: &[u8]) -> Option<String> {
    encode(&image::load_from_memory(bytes).ok()?)
}

/// Decodes what [`encode`] produced back into pixels a front end can draw.
/// `None` if the value is not the PNG this module wrote.
#[must_use]
pub fn decode(encoded: &str) -> Option<Graphic> {
    let png = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let rgba = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    Some(Graphic::Rgba {
        width: rgba.width(),
        height: rgba.height(),
        pixels: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::{THUMBNAIL_EDGE, decode, encode, encode_bytes};
    use crate::Graphic;

    /// A solid image `width` by `height`.
    fn solid(width: u32, height: u32) -> image::DynamicImage {
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            width,
            height,
            image::Rgba([10, 20, 30, 255]),
        ))
    }

    #[test]
    fn a_picture_survives_the_round_trip_as_drawable_pixels() {
        let encoded = encode(&solid(8, 4)).expect("a small image encodes");

        match decode(&encoded).expect("and decodes again") {
            Graphic::Rgba {
                width,
                height,
                pixels,
            } => {
                assert_eq!((width, height), (8, 4));
                assert_eq!(pixels.len(), 8 * 4 * 4, "four bytes a pixel");
            }
            Graphic::Svg(source) => panic!("expected pixels, got svg {source:.40}"),
        }
    }

    #[test]
    fn a_picture_smaller_than_the_cap_keeps_its_own_size() {
        let encoded = encode(&solid(16, 16)).expect("encodes");

        match decode(&encoded).expect("decodes") {
            Graphic::Rgba { width, height, .. } => {
                assert_eq!(
                    (width, height),
                    (16, 16),
                    "a small picture is shown at its size, not enlarged"
                );
            }
            Graphic::Svg(source) => panic!("expected pixels, got svg {source:.40}"),
        }
    }

    #[test]
    fn a_picture_larger_than_the_cap_is_scaled_down_to_it() {
        let encoded = encode(&solid(THUMBNAIL_EDGE * 3, THUMBNAIL_EDGE)).expect("encodes");

        match decode(&encoded).expect("decodes") {
            Graphic::Rgba { width, height, .. } => {
                assert_eq!(width, THUMBNAIL_EDGE, "the long edge is capped");
                assert!(height <= THUMBNAIL_EDGE);
                assert!(height > 0, "and the aspect ratio is kept");
            }
            Graphic::Svg(source) => panic!("expected pixels, got svg {source:.40}"),
        }
    }

    #[test]
    fn bytes_that_are_not_a_picture_encode_to_nothing() {
        assert_eq!(encode_bytes(b"this is not an image"), None);
    }

    #[test]
    fn a_value_this_module_did_not_write_decodes_to_nothing() {
        assert_eq!(decode("not base64 at all !!"), None);
        assert_eq!(decode(""), None);
    }
}
