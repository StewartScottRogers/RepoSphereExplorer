//! SVG file type plugin: core and presentation halves.
//!
//! Renders as a vector graphic, not the image plugin's raster metadata: the
//! view reports the declared `width`/`height` (falling back to `viewBox`'s
//! own width/height when explicit attributes are absent) rather than pixel
//! dimensions, and the presentation half labels it a vector image.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// SVG document markers, checked case-insensitively anywhere in the
/// content. `<svg`/`</svg>` boundaries and the SVG namespace URI are
/// specific enough not to overlap with any sibling plugin's markers, but a
/// bare `<svg` tag is also a generic XML/HTML document-structure tag, so
/// this plugin is placed just ahead of `html` and `xml` in `CORE_PLUGINS`
/// per those plugins' own notes, so a real SVG file is claimed before
/// either's own looser markers.
const SVG_MARKERS: &[&str] = &[
    "<svg>",
    "<svg ",
    "<svg\n",
    "<svg\r",
    "<svg\t",
    "<svg/",
    "</svg>",
    "xmlns=\"http://www.w3.org/2000/svg\"",
    "xmlns='http://www.w3.org/2000/svg'",
];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`SvgCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SvgView {
    /// Width in user units, from the `width` attribute or `viewBox` fallback.
    pub width: Option<f64>,
    /// Height in user units, from the `height` attribute or `viewBox` fallback.
    pub height: Option<f64>,
    /// The raw `viewBox` attribute value, if present.
    pub view_box: Option<String>,
    /// Size of the file on disk, in bytes.
    pub file_size: u64,
}

/// Extracts the first `<svg ...>` opening tag from `content`, or `None` if
/// absent.
fn find_svg_tag(content: &str) -> Option<&str> {
    let lower = content.to_ascii_lowercase();
    let start = lower.find("<svg")?;
    let end = content[start..].find('>')? + start + 1;
    Some(&content[start..end])
}

/// Extracts a `name="..."`/`name='...'` attribute value from `tag`, matched
/// case-insensitively by name.
fn parse_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut search_from = 0;
    loop {
        let found = lower[search_from..].find(name)?;
        let attr_start = search_from + found + name.len();
        match tag[attr_start..].chars().next() {
            Some('=') => {
                let value_start = attr_start + 1;
                let quote = tag[value_start..].chars().next()?;
                if quote == '"' || quote == '\'' {
                    let value_start = value_start + 1;
                    let end = tag[value_start..].find(quote)?;
                    return Some(&tag[value_start..value_start + end]);
                }
                search_from = value_start;
            }
            _ => search_from = attr_start,
        }
    }
}

/// Parses a CSS length such as `"120"` or `"120px"` into its numeric value,
/// ignoring any trailing unit suffix.
fn parse_length(value: &str) -> Option<f64> {
    let numeric: String = value
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    numeric.parse().ok()
}

/// Parses the third and fourth (width, height) numbers out of a `viewBox`
/// attribute value such as `"0 0 120 80"`.
fn parse_view_box_size(view_box: &str) -> Option<(f64, f64)> {
    let mut parts = view_box.split_whitespace();
    parts.next()?;
    parts.next()?;
    let width = parts.next()?.parse().ok()?;
    let height = parts.next()?.parse().ok()?;
    Some((width, height))
}

/// Whether `text` looks like an SVG document, per [`SVG_MARKERS`].
fn has_svg_syntax(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    SVG_MARKERS.iter().any(|marker| lower.contains(marker))
}

/// The SVG plugin's core half.
#[derive(Debug, Default)]
pub struct SvgCore;

impl PluginCore for SvgCore {
    fn name(&self) -> &'static str {
        "svg"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_svg_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let file_size = bytes.len() as u64;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();

        let tag = find_svg_tag(&content);
        let view_box = tag
            .and_then(|tag| parse_attr(tag, "viewbox"))
            .map(String::from);
        let explicit_width = tag
            .and_then(|tag| parse_attr(tag, "width"))
            .and_then(parse_length);
        let explicit_height = tag
            .and_then(|tag| parse_attr(tag, "height"))
            .and_then(parse_length);
        let view_box_size = view_box.as_deref().and_then(parse_view_box_size);

        let view = SvgView {
            width: explicit_width.or(view_box_size.map(|(width, _)| width)),
            height: explicit_height.or(view_box_size.map(|(_, height)| height)),
            view_box,
            file_size,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The SVG plugin's presentation half.
#[derive(Debug, Default)]
pub struct SvgPresentation;

impl PluginPresentation for SvgPresentation {
    fn name(&self) -> &'static str {
        "svg"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: SvgView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec!["SVG vector image".to_owned()];
        if let (Some(width), Some(height)) = (view.width, view.height) {
            lines.push(format!("{width} x {height} user units"));
        }
        if let Some(view_box) = &view.view_box {
            lines.push(format!("viewBox: {view_box}"));
        }
        lines.push(format!("{} bytes on disk", view.file_size));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_VIEW_BYTES, SvgCore, SvgPresentation, SvgView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-svg-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_svg_markers_as_svg() {
        assert!(
            SvgCore.sniff(
                b"<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>\n"
            )
        );
        assert!(SvgCore.sniff(b"<svg width=\"10\" height=\"10\"></svg>\n"));
        assert!(SvgCore.sniff(b"<SVG xmlns=\"http://www.w3.org/2000/svg\"></SVG>\n"));
        assert!(SvgCore.sniff(b"<svg/>\n"));
    }

    #[test]
    fn does_not_sniff_other_xml_or_html_as_svg() {
        assert!(!SvgCore.sniff(b"<?xml version=\"1.0\"?>\n<root></root>\n"));
        assert!(!SvgCore.sniff(b"<!DOCTYPE html>\n<html>\n</html>\n"));
        assert!(!SvgCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!SvgCore.sniff(b"just a regular line of text\n"));
        assert!(!SvgCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_svg_file_and_extracts_explicit_dimensions() {
        let path = unique_temp_file("icon.svg");
        std::fs::write(
            &path,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24px\" height=\"32\" viewBox=\"0 0 24 32\">\n<circle cx=\"12\" cy=\"16\" r=\"10\"/>\n</svg>\n",
        )
        .unwrap();

        let data = SvgCore.view(&path).unwrap();
        let view: SvgView = serde_json::from_value(data).unwrap();

        assert_eq!(view.width, Some(24.0));
        assert_eq!(view.height, Some(32.0));
        assert_eq!(view.view_box.as_deref(), Some("0 0 24 32"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn falls_back_to_view_box_size_when_width_and_height_are_absent() {
        let path = unique_temp_file("no-explicit-size.svg");
        std::fs::write(
            &path,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 120 80\">\n<rect width=\"120\" height=\"80\"/>\n</svg>\n",
        )
        .unwrap();

        let data = SvgCore.view(&path).unwrap();
        let view: SvgView = serde_json::from_value(data).unwrap();

        assert_eq!(view.width, Some(120.0));
        assert_eq!(view.height, Some(80.0));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit_but_still_reports_full_file_size() {
        let path = unique_temp_file("large.svg");
        let mut content =
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\">\n".to_owned();
        content.push_str(&"<!-- padding -->\n".repeat(MAX_VIEW_BYTES));
        content.push_str("</svg>\n");
        std::fs::write(&path, &content).unwrap();

        let data = SvgCore.view(&path).unwrap();
        let view: SvgView = serde_json::from_value(data).unwrap();

        assert_eq!(view.file_size, content.len() as u64);
        assert_eq!(view.width, Some(10.0));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_dimensions_view_box_and_file_size() {
        let data = serde_json::to_value(SvgView {
            width: Some(24.0),
            height: Some(32.0),
            view_box: Some("0 0 24 32".to_owned()),
            file_size: 512,
        })
        .unwrap();

        let lines = SvgPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "SVG vector image",
                "24 x 32 user units",
                "viewBox: 0 0 24 32",
                "512 bytes on disk",
            ]
        );
    }
}
