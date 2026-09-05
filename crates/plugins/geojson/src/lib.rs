//! `GeoJSON` file type plugin: core and presentation halves.
//!
//! Renders an ASCII map of the document's coordinates, distinct from the
//! generic `json` plugin's indented tree view despite being JSON under the
//! hood.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// UTF-8 byte order mark, stripped before sniffing.
const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// `GeoJSON`'s own `type` member values ([RFC 7946 §1.4]), checked as a
/// quoted value alongside a `"type"` key and one of
/// [`GEOJSON_COMPANION_KEYS`] — markers not used by any sibling plugin. A
/// genuine `GeoJSON` document is also valid JSON, so this plugin is placed
/// just ahead of `json` in `CORE_PLUGINS`, claiming the file with its own
/// stronger markers first.
///
/// [RFC 7946 §1.4]: https://www.rfc-editor.org/rfc/rfc7946#section-1.4
const GEOJSON_TYPES: &[&str] = &[
    "FeatureCollection",
    "GeometryCollection",
    "MultiLineString",
    "MultiPolygon",
    "LineString",
    "MultiPoint",
    "Polygon",
    "Feature",
    "Point",
];

/// Other RFC 7946 members that, alongside a `"type"` marker, confirm this is
/// `GeoJSON` rather than an unrelated JSON document that happens to carry a
/// `type` field with one of the same names.
const GEOJSON_COMPANION_KEYS: &[&str] = &["\"coordinates\"", "\"features\"", "\"geometries\""];

/// Maximum number of features and coordinate points rendered in the view.
const MAX_ENTRIES: usize = 200;

/// Width, in characters, of the rendered ASCII map (excluding its border).
const MAP_WIDTH: usize = 56;

/// Height, in characters, of the rendered ASCII map (excluding its border).
const MAP_HEIGHT: usize = 20;

/// A geometry's rendered summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeometrySummary {
    /// The geometry's own `type` member, e.g. `"Point"`, `"Polygon"`.
    pub geometry_type: String,
    /// Number of coordinate points the geometry carries.
    pub point_count: usize,
}

/// One feature's rendered summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureSummary {
    /// The feature's geometry, or `None` if it has none (or a null one).
    pub geometry: Option<GeometrySummary>,
    /// A `name`/`title`/`id` property value, if present, else `None`.
    pub label: Option<String>,
}

/// The document parsed as `GeoJSON`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeoJsonDocument {
    /// The top-level `type` member, e.g. `"FeatureCollection"`, `"Point"`.
    pub document_type: String,
    /// Total number of features (or the single implicit feature, for a bare
    /// geometry document).
    pub feature_count: usize,
    /// The first [`MAX_ENTRIES`] features.
    pub features: Vec<FeatureSummary>,
    /// The document's coordinate bounding box, as
    /// `(min_lon, min_lat, max_lon, max_lat)`, or `None` if it has no
    /// coordinates.
    pub bounding_box: Option<(f64, f64, f64, f64)>,
    /// The first [`MAX_ENTRIES`] coordinate points, used to render the map.
    pub points: Vec<(f64, f64)>,
}

/// View data produced by [`GeoJsonCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoJsonView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary), shown
    /// as a fallback when `parsed` is `None`.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// The file parsed as `GeoJSON`, or `None` if it is not valid `GeoJSON`.
    pub parsed: Option<GeoJsonDocument>,
}

/// Strips a leading UTF-8 BOM and ASCII whitespace from `prefix`.
fn trim_prefix(prefix: &[u8]) -> &[u8] {
    let without_bom = prefix.strip_prefix(UTF8_BOM).unwrap_or(prefix);
    let start = without_bom
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(without_bom.len());
    &without_bom[start..]
}

/// Whether `prefix` looks like `GeoJSON`, per [`GEOJSON_TYPES`] and
/// [`GEOJSON_COMPANION_KEYS`].
fn looks_like_geojson(prefix: &[u8]) -> bool {
    let trimmed = trim_prefix(prefix);
    if !trimmed.starts_with(b"{") {
        return false;
    }
    let text = String::from_utf8_lossy(trimmed);
    text.contains("\"type\"")
        && GEOJSON_TYPES
            .iter()
            .any(|geojson_type| text.contains(&format!("\"{geojson_type}\"")))
        && GEOJSON_COMPANION_KEYS.iter().any(|key| text.contains(key))
}

/// Recursively collects coordinate `(lon, lat)` pairs out of a `coordinates`
/// member's value: a bare pair is an array whose first element is a number,
/// anything else is an array of nested coordinate arrays.
fn collect_coordinates(value: &Value, points: &mut Vec<(f64, f64)>) {
    let Some(items) = value.as_array() else {
        return;
    };
    if items.first().is_some_and(Value::is_number) {
        if let (Some(lon), Some(lat)) = (
            items.first().and_then(Value::as_f64),
            items.get(1).and_then(Value::as_f64),
        ) {
            points.push((lon, lat));
        }
        return;
    }
    for item in items {
        collect_coordinates(item, points);
    }
}

/// Reads a geometry object's summary, appending its coordinate points to
/// `points`. Recurses into a `GeometryCollection`'s own `geometries`.
fn geometry_summary(geometry: &Value, points: &mut Vec<(f64, f64)>) -> Option<GeometrySummary> {
    let geometry_type = geometry.get("type")?.as_str()?.to_owned();
    if geometry_type == "GeometryCollection" {
        let before = points.len();
        for child in geometry
            .get("geometries")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            geometry_summary(child, points);
        }
        return Some(GeometrySummary {
            geometry_type,
            point_count: points.len() - before,
        });
    }
    let coordinates = geometry.get("coordinates")?;
    let before = points.len();
    collect_coordinates(coordinates, points);
    Some(GeometrySummary {
        geometry_type,
        point_count: points.len() - before,
    })
}

/// Reads a `name`/`title`/`id` property value, converted to display text.
fn label_from_properties(properties: Option<&Value>) -> Option<String> {
    let properties = properties?.as_object()?;
    for key in ["name", "title", "id"] {
        match properties.get(key) {
            Some(Value::String(text)) => return Some(text.clone()),
            Some(Value::Number(number)) => return Some(number.to_string()),
            _ => {}
        }
    }
    None
}

/// Reads a `Feature` object's summary.
fn feature_summary(feature: &Value, points: &mut Vec<(f64, f64)>) -> FeatureSummary {
    let geometry = feature
        .get("geometry")
        .filter(|geometry| !geometry.is_null())
        .and_then(|geometry| geometry_summary(geometry, points));
    let label = label_from_properties(feature.get("properties"));
    FeatureSummary { geometry, label }
}

/// The bounding box `(min_lon, min_lat, max_lon, max_lat)` of `points`, or
/// `None` if it is empty.
fn bounding_box(points: &[(f64, f64)]) -> Option<(f64, f64, f64, f64)> {
    let mut bounds: Option<(f64, f64, f64, f64)> = None;
    for &(lon, lat) in points {
        bounds = Some(match bounds {
            None => (lon, lat, lon, lat),
            Some((min_lon, min_lat, max_lon, max_lat)) => (
                min_lon.min(lon),
                min_lat.min(lat),
                max_lon.max(lon),
                max_lat.max(lat),
            ),
        });
    }
    bounds
}

/// Parses `value` as a `GeoJSON` document, or `None` if its top-level
/// `type` is missing or not a `GeoJSON` object type.
fn parse_geojson(value: &Value) -> Option<GeoJsonDocument> {
    let document_type = value.get("type")?.as_str()?.to_owned();
    let mut points = Vec::new();
    let features: Vec<FeatureSummary> = match document_type.as_str() {
        "FeatureCollection" => value
            .get("features")?
            .as_array()?
            .iter()
            .map(|feature| feature_summary(feature, &mut points))
            .collect(),
        "Feature" => vec![feature_summary(value, &mut points)],
        "Point" | "MultiPoint" | "LineString" | "MultiLineString" | "Polygon" | "MultiPolygon"
        | "GeometryCollection" => vec![FeatureSummary {
            geometry: geometry_summary(value, &mut points),
            label: None,
        }],
        _ => return None,
    };
    let bounding_box = bounding_box(&points);
    points.truncate(MAX_ENTRIES);
    let feature_count = features.len();
    let mut features = features;
    features.truncate(MAX_ENTRIES);
    Some(GeoJsonDocument {
        document_type,
        feature_count,
        features,
        bounding_box,
        points,
    })
}

/// Scales `value` from `[min, max]` onto a grid axis `[0, size - 1]`,
/// rounding to the nearest cell. `size` is always one of this plugin's own
/// small, fixed grid dimensions ([`MAP_WIDTH`]/[`MAP_HEIGHT`]), so the
/// `usize`-`f64` round trip neither loses precision nor overflows; `value`
/// is one of the same points `min`/`max` were computed from, so the
/// division result is always within `[0, 1]` and the cast is never
/// negative.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]
fn scale_to_grid(value: f64, min: f64, max: f64, size: usize) -> usize {
    let span = max - min;
    if span > 0.0 {
        (((value - min) / span) * (size - 1) as f64).round() as usize
    } else {
        (size - 1) / 2
    }
}

/// Renders `points` as an ASCII map bounded by `bounding_box`, one line per
/// map row plus a bordering top/bottom rule.
fn render_map(points: &[(f64, f64)], bounding_box: (f64, f64, f64, f64)) -> Vec<String> {
    let (min_lon, min_lat, max_lon, max_lat) = bounding_box;
    let mut grid = vec![vec![b'.'; MAP_WIDTH]; MAP_HEIGHT];
    for &(lon, lat) in points {
        let column = scale_to_grid(lon, min_lon, max_lon, MAP_WIDTH);
        // Rows run top to bottom, but latitude increases north (up), so the
        // highest latitude maps to row 0.
        let row = scale_to_grid(max_lat - lat, 0.0, max_lat - min_lat, MAP_HEIGHT);
        grid[row.min(MAP_HEIGHT - 1)][column.min(MAP_WIDTH - 1)] = b'*';
    }
    let rule = format!("+{}+", "-".repeat(MAP_WIDTH));
    let mut lines = vec![rule.clone()];
    for row in grid {
        lines.push(format!("|{}|", String::from_utf8(row).unwrap()));
    }
    lines.push(rule);
    lines
}

/// The `GeoJSON` plugin's core half.
#[derive(Debug, Default)]
pub struct GeoJsonCore;

impl PluginCore for GeoJsonCore {
    fn name(&self) -> &'static str {
        "geojson"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_geojson(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let parsed = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|value| parse_geojson(&value));
        let view = GeoJsonView {
            content,
            truncated,
            parsed,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The `GeoJSON` plugin's presentation half.
#[derive(Debug, Default)]
pub struct GeoJsonPresentation;

impl PluginPresentation for GeoJsonPresentation {
    fn name(&self) -> &'static str {
        "geojson"
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: GeoJsonView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let Some(document) = view.parsed else {
            let mut lines = vec!["could not parse as GeoJSON; showing raw content".to_owned()];
            lines.extend(view.content.lines().map(str::to_owned));
            if view.truncated {
                lines.push("… (truncated)".to_owned());
            }
            return lines;
        };
        let mut lines = vec![format!(
            "GeoJSON: {} ({} feature{})",
            document.document_type,
            document.feature_count,
            if document.feature_count == 1 { "" } else { "s" }
        )];
        if let Some(bounding_box) = document.bounding_box {
            let (min_lon, min_lat, max_lon, max_lat) = bounding_box;
            lines.push(format!(
                "Bounding box: ({min_lon:.4}, {min_lat:.4}) to ({max_lon:.4}, {max_lat:.4})"
            ));
            lines.extend(render_map(&document.points, bounding_box));
        } else {
            lines.push("No coordinates found".to_owned());
        }
        for (index, feature) in document.features.iter().enumerate() {
            let geometry = match &feature.geometry {
                Some(geometry) => format!(
                    "{} ({} point(s))",
                    geometry.geometry_type, geometry.point_count
                ),
                None => "no geometry".to_owned(),
            };
            match &feature.label {
                Some(label) => lines.push(format!("  [{index}] {label}: {geometry}")),
                None => lines.push(format!("  [{index}] {geometry}")),
            }
        }
        if document.feature_count > document.features.len() {
            lines.push(format!(
                "  ... {} more features not shown",
                document.feature_count - document.features.len()
            ));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{GeoJsonCore, GeoJsonDocument, GeoJsonPresentation, GeoJsonView};
    use plugin_api::{PluginCore, PluginPresentation};
    use serde_json::json;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-geojson-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_geojson_documents() {
        assert!(GeoJsonCore.sniff(
            br#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":{"type":"Point","coordinates":[1,2]},"properties":{}}]}"#
        ));
        assert!(GeoJsonCore.sniff(br#"{"type": "Point", "coordinates": [1.0, 2.0]}"#));
    }

    #[test]
    fn does_not_sniff_other_json_or_text_as_geojson() {
        assert!(!GeoJsonCore.sniff(br#"{"type": "success", "value": 1}"#));
        assert!(!GeoJsonCore.sniff(b"{\"a\": 1}"));
        assert!(!GeoJsonCore.sniff(b"just a regular line of text\n"));
        assert!(!GeoJsonCore.sniff(b""));
        assert!(!GeoJsonCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_geojson_feature_collection() {
        let path = unique_temp_file("cities.geojson");
        std::fs::write(
            &path,
            json!({
                "type": "FeatureCollection",
                "features": [
                    {
                        "type": "Feature",
                        "properties": {"name": "Origin"},
                        "geometry": {"type": "Point", "coordinates": [0.0, 0.0]}
                    },
                    {
                        "type": "Feature",
                        "properties": {"name": "Northeast"},
                        "geometry": {"type": "Point", "coordinates": [10.0, 10.0]}
                    }
                ]
            })
            .to_string(),
        )
        .unwrap();

        let data = GeoJsonCore.view(&path).unwrap();
        let view: GeoJsonView = serde_json::from_value(data).unwrap();
        let document = view.parsed.expect("valid GeoJSON should parse");

        assert_eq!(document.document_type, "FeatureCollection");
        assert_eq!(document.feature_count, 2);
        assert_eq!(document.bounding_box, Some((0.0, 0.0, 10.0, 10.0)));
        assert_eq!(document.features[0].label.as_deref(), Some("Origin"));
        assert_eq!(
            document.features[0].geometry,
            Some(super::GeometrySummary {
                geometry_type: "Point".to_owned(),
                point_count: 1,
            })
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_bare_polygon_geometry() {
        let path = unique_temp_file("plot.geojson");
        std::fs::write(
            &path,
            json!({
                "type": "Polygon",
                "coordinates": [[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 0.0]]]
            })
            .to_string(),
        )
        .unwrap();

        let data = GeoJsonCore.view(&path).unwrap();
        let view: GeoJsonView = serde_json::from_value(data).unwrap();
        let document = view.parsed.expect("valid GeoJSON should parse");

        assert_eq!(document.document_type, "Polygon");
        assert_eq!(document.bounding_box, Some((0.0, 0.0, 4.0, 4.0)));
        assert_eq!(
            document.features[0].geometry.as_ref().unwrap().point_count,
            4
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_an_invalid_geojson_file_with_no_parsed_value() {
        let path = unique_temp_file("invalid.geojson");
        std::fs::write(&path, "{ not json").unwrap();

        let data = GeoJsonCore.view(&path).unwrap();
        let view: GeoJsonView = serde_json::from_value(data).unwrap();

        assert!(view.parsed.is_none());

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_a_header_bounding_box_map_and_feature_list() {
        let document = GeoJsonDocument {
            document_type: "FeatureCollection".to_owned(),
            feature_count: 1,
            features: vec![super::FeatureSummary {
                geometry: Some(super::GeometrySummary {
                    geometry_type: "Point".to_owned(),
                    point_count: 1,
                }),
                label: Some("Origin".to_owned()),
            }],
            bounding_box: Some((0.0, 0.0, 1.0, 1.0)),
            points: vec![(0.0, 0.0)],
        };
        let data = serde_json::to_value(GeoJsonView {
            content: String::new(),
            truncated: false,
            parsed: Some(document),
        })
        .unwrap();

        let lines = GeoJsonPresentation.present(&data);

        assert_eq!(lines[0], "GeoJSON: FeatureCollection (1 feature)");
        assert_eq!(
            lines[1],
            "Bounding box: (0.0000, 0.0000) to (1.0000, 1.0000)"
        );
        assert!(lines.iter().any(|line| line.contains('*')));
        assert!(
            lines
                .iter()
                .any(|line| line == "  [0] Origin: Point (1 point(s))")
        );
    }

    #[test]
    fn presents_raw_content_when_not_parseable() {
        let data = serde_json::to_value(GeoJsonView {
            content: "{ not json".to_owned(),
            truncated: true,
            parsed: None,
        })
        .unwrap();

        let lines = GeoJsonPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "could not parse as GeoJSON; showing raw content",
                "{ not json",
                "… (truncated)",
            ]
        );
    }
}
