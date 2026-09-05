//! Font file type plugin: core and presentation halves.
//!
//! One plugin covers the common font container formats (TrueType, OpenType,
//! WOFF, WOFF2), matching how the existing `image` plugin covers multiple
//! raster codecs with one crate pairing: `ttf-parser` reads the `sfnt` table
//! directory shared by TrueType and OpenType, while `wuff` decompresses a
//! WOFF or WOFF2 wrapper down to that same `sfnt` form first, since
//! `ttf-parser` itself only understands the uncompressed container. The view
//! is metadata only (family, style, glyph count, units per em), not a
//! rendered glyph preview: per `plugin-api`'s presentation half, a front end
//! gets lines of text, not toolkit-specific glyph outlines.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// A font's container format, identified by its leading magic bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Container {
    /// A bare TrueType outline font (`sfnt` version `0x00010000` or `true`).
    TrueType,
    /// A bare OpenType/CFF font (`sfnt` version `OTTO`).
    OpenType,
    /// A WOFF-wrapped `sfnt`, compressed with zlib.
    Woff,
    /// A WOFF2-wrapped `sfnt`, compressed with Brotli.
    Woff2,
}

impl Container {
    /// Detects the container format from a file's leading bytes.
    fn detect(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(&[0x00, 0x01, 0x00, 0x00]) || bytes.starts_with(b"true") {
            Some(Self::TrueType)
        } else if bytes.starts_with(b"OTTO") {
            Some(Self::OpenType)
        } else if bytes.starts_with(b"wOFF") {
            Some(Self::Woff)
        } else if bytes.starts_with(b"wOF2") {
            Some(Self::Woff2)
        } else {
            None
        }
    }

    /// The human-readable label for this container, used as [`FontView::format`].
    fn label(self) -> &'static str {
        match self {
            Self::TrueType => "TrueType",
            Self::OpenType => "OpenType",
            Self::Woff => "WOFF",
            Self::Woff2 => "WOFF2",
        }
    }
}

/// Returns the first decodable string for `name_id` in `names`, if any.
///
/// A font can repeat the same name ID across multiple platform/encoding
/// entries; the first one `ttf-parser` can decode to UTF-8 is good enough for
/// a metadata view.
fn find_name(names: ttf_parser::name::Names<'_>, name_id: u16) -> Option<String> {
    names
        .into_iter()
        .find(|name| name.name_id == name_id)
        .and_then(|name| name.to_string())
}

/// View data produced by [`FontCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontView {
    /// The detected container format, e.g. `"TrueType"`, `"WOFF2"`.
    pub format: String,
    /// The font family name, from the `name` table, if present.
    pub family: Option<String>,
    /// The font subfamily (style), e.g. `"Bold"`, `"Italic"`, if present.
    pub subfamily: Option<String>,
    /// Units per em, defining the glyph coordinate scale.
    pub units_per_em: u16,
    /// Number of glyphs in the font.
    pub number_of_glyphs: u16,
    /// Whether every glyph has the same advance width.
    pub is_monospaced: bool,
    /// Size of the file on disk, in bytes.
    pub file_size: u64,
}

/// The font plugin's core half.
#[derive(Debug, Default)]
pub struct FontCore;

impl PluginCore for FontCore {
    fn name(&self) -> &'static str {
        "font"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        Container::detect(prefix).is_some()
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let file_size = std::fs::metadata(path)?.len();
        let raw = std::fs::read(path)?;
        let container = Container::detect(&raw).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "not a recognised font container",
            )
        })?;
        let sfnt = match container {
            Container::Woff => wuff::decompress_woff1(&raw)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
            Container::Woff2 => wuff::decompress_woff2(&raw)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
            Container::TrueType | Container::OpenType => raw,
        };
        let face = ttf_parser::Face::parse(&sfnt, 0)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let names = face.names();
        let family = find_name(names, ttf_parser::name_id::TYPOGRAPHIC_FAMILY)
            .or_else(|| find_name(names, ttf_parser::name_id::FAMILY));
        let subfamily = find_name(names, ttf_parser::name_id::TYPOGRAPHIC_SUBFAMILY)
            .or_else(|| find_name(names, ttf_parser::name_id::SUBFAMILY));
        let view = FontView {
            format: container.label().to_owned(),
            family,
            subfamily,
            units_per_em: face.units_per_em(),
            number_of_glyphs: face.number_of_glyphs(),
            is_monospaced: face.is_monospaced(),
            file_size,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The font plugin's presentation half.
#[derive(Debug, Default)]
pub struct FontPresentation;

impl PluginPresentation for FontPresentation {
    fn name(&self) -> &'static str {
        "font"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: FontView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!("{} font", view.format)];
        match (&view.family, &view.subfamily) {
            (Some(family), Some(subfamily)) => lines.push(format!("{family} {subfamily}")),
            (Some(family), None) => lines.push(family.clone()),
            (None, _) => {}
        }
        lines.push(format!("{} units per em", view.units_per_em));
        lines.push(format!("{} glyphs", view.number_of_glyphs));
        if view.is_monospaced {
            lines.push("Monospaced".to_owned());
        }
        lines.push(format!("{} bytes on disk", view.file_size));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{FontCore, FontPresentation, FontView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-font-test-{}-{name}",
            std::process::id()
        ))
    }

    fn push_u16be(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn push_u32be(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_be_bytes());
    }

    /// A minimal, valid TrueType file: an `sfnt` header and table directory
    /// naming `head`, `hhea`, `maxp` (`ttf-parser`'s required tables) and a
    /// `name` table carrying a family name - enough for `ttf-parser` to parse
    /// a real `Face` and for this plugin to read back family/glyph metadata.
    fn write_test_ttf(path: &std::path::Path) {
        let family = "Test Font";
        let family_utf16be: Vec<u8> = family.encode_utf16().flat_map(u16::to_be_bytes).collect();

        let mut head = Vec::new();
        push_u16be(&mut head, 1); // majorVersion
        push_u16be(&mut head, 0); // minorVersion
        push_u32be(&mut head, 0x0001_0000); // fontRevision
        push_u32be(&mut head, 0); // checkSumAdjustment
        push_u32be(&mut head, 0x5F0F_3CF5); // magicNumber
        push_u16be(&mut head, 0); // flags
        push_u16be(&mut head, 1000); // unitsPerEm
        head.extend_from_slice(&[0; 8]); // created
        head.extend_from_slice(&[0; 8]); // modified
        push_u16be(&mut head, 0); // xMin/yMin/xMax/yMax
        push_u16be(&mut head, 0);
        push_u16be(&mut head, 0);
        push_u16be(&mut head, 0);
        push_u16be(&mut head, 0); // macStyle
        push_u16be(&mut head, 0); // lowestRecPPEM
        push_u16be(&mut head, 0); // fontDirectionHint
        push_u16be(&mut head, 0); // indexToLocFormat
        push_u16be(&mut head, 0); // glyphDataFormat
        assert_eq!(head.len(), 54);

        let mut hhea = Vec::new();
        push_u16be(&mut hhea, 1); // majorVersion
        push_u16be(&mut hhea, 0); // minorVersion
        hhea.extend_from_slice(&[0; 20]); // ascender .. caretOffset
        hhea.extend_from_slice(&[0; 8]); // reserved x4
        push_u16be(&mut hhea, 0); // metricDataFormat
        push_u16be(&mut hhea, 1); // numberOfHMetrics
        assert_eq!(hhea.len(), 36);

        let number_of_glyphs: u16 = 5;
        let mut maxp = Vec::new();
        push_u32be(&mut maxp, 0x0001_0000); // version 1.0
        push_u16be(&mut maxp, number_of_glyphs);
        maxp.extend_from_slice(&[0; 26]); // remaining v1.0 fields
        assert_eq!(maxp.len(), 32);

        let mut name = Vec::new();
        push_u16be(&mut name, 0); // format
        push_u16be(&mut name, 1); // count
        push_u16be(&mut name, 18); // stringOffset (6 header + 1 * 12 record bytes)
        push_u16be(&mut name, 3); // platformID: Windows
        push_u16be(&mut name, 1); // encodingID: Unicode BMP
        push_u16be(&mut name, 0x0409); // languageID: en-US
        push_u16be(&mut name, 1); // nameID: FAMILY
        push_u16be(&mut name, u16::try_from(family_utf16be.len()).unwrap());
        push_u16be(&mut name, 0); // offset within storage
        name.extend_from_slice(&family_utf16be);

        let tables: [(&[u8; 4], &[u8]); 4] = [
            (b"head", &head),
            (b"hhea", &hhea),
            (b"maxp", &maxp),
            (b"name", &name),
        ];

        let mut font = Vec::new();
        push_u32be(&mut font, 0x0001_0000); // sfnt version: TrueType
        push_u16be(&mut font, u16::try_from(tables.len()).unwrap());
        push_u16be(&mut font, 64); // searchRange
        push_u16be(&mut font, 2); // entrySelector
        push_u16be(&mut font, 0); // rangeShift

        let directory_end = 12 + tables.len() * 16;
        let mut offset = u32::try_from(directory_end).unwrap();
        for (tag, table) in &tables {
            font.extend_from_slice(*tag);
            push_u32be(&mut font, 0); // checksum, unchecked by ttf-parser
            push_u32be(&mut font, offset);
            push_u32be(&mut font, u32::try_from(table.len()).unwrap());
            offset += u32::try_from(table.len()).unwrap();
        }
        for (_, table) in &tables {
            font.extend_from_slice(table);
        }

        std::fs::write(path, font).unwrap();
    }

    #[test]
    fn sniffs_a_real_ttf_header() {
        assert!(FontCore.sniff(&[0x00, 0x01, 0x00, 0x00]));
        assert!(FontCore.sniff(b"true"));
        assert!(!FontCore.sniff(b"not a font"));
    }

    #[test]
    fn sniffs_an_otf_header() {
        assert!(FontCore.sniff(b"OTTO"));
    }

    #[test]
    fn sniffs_woff_and_woff2_headers() {
        assert!(FontCore.sniff(b"wOFF"));
        assert!(FontCore.sniff(b"wOF2"));
    }

    #[test]
    fn views_a_real_ttf_file() {
        let path = unique_temp_file("test.ttf");
        write_test_ttf(&path);

        let data = FontCore.view(&path).unwrap();
        let view: FontView = serde_json::from_value(data).unwrap();

        assert_eq!(view.format, "TrueType");
        assert_eq!(view.family.as_deref(), Some("Test Font"));
        assert_eq!(view.units_per_em, 1000);
        assert_eq!(view.number_of_glyphs, 5);
        assert!(view.file_size > 0);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_family_and_glyph_metadata() {
        let data = serde_json::to_value(FontView {
            format: "TrueType".to_owned(),
            family: Some("Test Font".to_owned()),
            subfamily: Some("Bold".to_owned()),
            units_per_em: 1000,
            number_of_glyphs: 5,
            is_monospaced: false,
            file_size: 123,
        })
        .unwrap();

        let lines = FontPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "TrueType font",
                "Test Font Bold",
                "1000 units per em",
                "5 glyphs",
                "123 bytes on disk",
            ]
        );
    }
}
