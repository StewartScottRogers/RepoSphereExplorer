//! 3D model file type plugin: core and presentation halves.
//!
//! One plugin covers Wavefront OBJ, STL, and glTF, matching how the
//! existing `image` plugin covers multiple raster codecs with one crate.
//! The three formats have no shared magic bytes and no shared metadata
//! shape (OBJ and STL expose raw vertex/face counts, glTF is a JSON scene
//! graph with meshes and scenes instead), so [`Model3dView`] carries the
//! union of each format's own fields rather than forcing one geometry
//! model onto all three, the same choice `word-document` made for its own
//! unrelated container formats.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Upper bound on a binary STL's triangle count accepted during sniffing.
///
/// Binary STL has no true magic number: its 80-byte header is free-form
/// text, sometimes even starting with `solid` by convention despite that
/// keyword belonging to the ASCII variant. Bounding the triangle count read
/// from the header rejects most non-STL binary data while still admitting
/// any real-world mesh; an accepted content-sniffing limitation shared with
/// this project's other structurally-sniffed formats.
const MAX_PLAUSIBLE_STL_TRIANGLES: u32 = 5_000_000;

/// A 3D model's container format, identified by its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Container {
    /// Wavefront OBJ, a plain-text vertex/face format.
    Obj,
    /// STL in its plain-text variant (`solid` ... `endsolid`).
    StlAscii,
    /// STL in its binary variant (80-byte header, then a triangle count).
    StlBinary,
    /// glTF in its plain-text (JSON) variant.
    Gltf,
}

impl Container {
    /// Detects the container format from a file's bytes (a bounded prefix
    /// while sniffing, the whole file while viewing).
    fn detect(bytes: &[u8]) -> Option<Self> {
        if let Ok(text) = std::str::from_utf8(bytes) {
            if is_gltf(text) {
                return Some(Self::Gltf);
            }
            if is_stl_ascii(text) {
                return Some(Self::StlAscii);
            }
            if is_obj(text) {
                return Some(Self::Obj);
            }
        }
        if is_stl_binary(bytes) {
            return Some(Self::StlBinary);
        }
        None
    }

    /// The human-readable label for this container, used as [`Model3dView::format`].
    fn label(self) -> &'static str {
        match self {
            Self::Obj => "OBJ",
            Self::StlAscii => "STL (ASCII)",
            Self::StlBinary => "STL (binary)",
            Self::Gltf => "glTF",
        }
    }
}

/// Whether `text` opens with the `solid` keyword STL uses to start a
/// document, as a whole word (not merely a prefix like `solidify`).
fn starts_with_solid_keyword(text: &str) -> bool {
    let Some(rest) = text.trim_start().strip_prefix("solid") else {
        return false;
    };
    rest.chars().next().is_none_or(|ch| !ch.is_alphanumeric())
}

/// Detects a plain-text glTF document: JSON carrying the spec-mandated
/// `asset` object with its own required `version` field — a combination
/// not used by this project's other JSON-based formats.
fn is_gltf(text: &str) -> bool {
    text.trim_start().starts_with('{') && text.contains("\"asset\"") && text.contains("\"version\"")
}

/// Detects a plain-text STL document: the `solid` opener plus at least one
/// `facet normal` directive, so an unrelated file that merely starts with
/// the word `solid` isn't misidentified.
fn is_stl_ascii(text: &str) -> bool {
    starts_with_solid_keyword(text) && text.contains("facet normal")
}

/// Detects a binary STL document: 80-byte header, then a triangle count
/// that isn't implausibly large, with the `solid` keyword check skipped
/// (`is_stl_ascii` already claims those cases first in [`Container::detect`]).
fn is_stl_binary(bytes: &[u8]) -> bool {
    let Some(count_bytes) = bytes.get(80..84) else {
        return false;
    };
    let count = u32::from_le_bytes(count_bytes.try_into().unwrap_or_default());
    count > 0 && count <= MAX_PLAUSIBLE_STL_TRIANGLES
}

/// Detects a Wavefront OBJ document: any line opening with a directive
/// unique to OBJ (`vt`/`vn`/`vp`/`usemtl`/`mtllib`), or both a vertex (`v`)
/// and a face (`f`) line together, since either alone is too short a
/// marker to trust on its own.
fn is_obj(text: &str) -> bool {
    let mut has_strong_marker = false;
    let mut has_vertex = false;
    let mut has_face = false;
    for line in text.lines() {
        if line.starts_with("vt ")
            || line.starts_with("vn ")
            || line.starts_with("vp ")
            || line.starts_with("usemtl ")
            || line.starts_with("mtllib ")
        {
            has_strong_marker = true;
        }
        has_vertex |= line.starts_with("v ");
        has_face |= line.starts_with("f ");
    }
    has_strong_marker || (has_vertex && has_face)
}

/// Counts lines in `text` that start with `prefix`.
fn count_lines_starting_with(text: &str, prefix: &str) -> u64 {
    text.lines().filter(|line| line.starts_with(prefix)).count() as u64
}

/// View data produced by [`Model3dCore::view`].
///
/// Only the fields that apply to the detected [`Container::label`] are
/// populated: `vertex_count`/`face_count` for OBJ and STL, `mesh_count`/
/// `scene_count`/`generator` for glTF.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model3dView {
    /// The detected container format, e.g. `"OBJ"`, `"STL (binary)"`, `"glTF"`.
    pub format: String,
    /// Number of vertices (OBJ's own `v` lines, or 3 per STL facet).
    pub vertex_count: Option<u64>,
    /// Number of faces (OBJ's own `f` lines, or STL's triangle count).
    pub face_count: Option<u64>,
    /// Number of meshes declared in a glTF document.
    pub mesh_count: Option<u64>,
    /// Number of scenes declared in a glTF document.
    pub scene_count: Option<u64>,
    /// The tool that generated a glTF document, from its `asset.generator` field.
    pub generator: Option<String>,
    /// Size of the file on disk, in bytes.
    pub file_size: u64,
}

/// The 3D model plugin's core half.
#[derive(Debug, Default)]
pub struct Model3dCore;

impl PluginCore for Model3dCore {
    fn name(&self) -> &'static str {
        "model3d"
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
                "not a recognised 3D model format",
            )
        })?;
        let view = match container {
            Container::Obj => {
                let text = String::from_utf8_lossy(&raw);
                Model3dView {
                    format: container.label().to_owned(),
                    vertex_count: Some(count_lines_starting_with(&text, "v ")),
                    face_count: Some(count_lines_starting_with(&text, "f ")),
                    mesh_count: None,
                    scene_count: None,
                    generator: None,
                    file_size,
                }
            }
            Container::StlAscii => {
                let text = String::from_utf8_lossy(&raw);
                let face_count = text.matches("facet normal").count() as u64;
                Model3dView {
                    format: container.label().to_owned(),
                    vertex_count: Some(face_count * 3),
                    face_count: Some(face_count),
                    mesh_count: None,
                    scene_count: None,
                    generator: None,
                    file_size,
                }
            }
            Container::StlBinary => {
                let count_bytes = raw.get(80..84).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "binary STL header is truncated")
                })?;
                let face_count = u64::from(u32::from_le_bytes(count_bytes.try_into().unwrap()));
                Model3dView {
                    format: container.label().to_owned(),
                    vertex_count: Some(face_count * 3),
                    face_count: Some(face_count),
                    mesh_count: None,
                    scene_count: None,
                    generator: None,
                    file_size,
                }
            }
            Container::Gltf => {
                let json: serde_json::Value = serde_json::from_slice(&raw)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                let mesh_count = json
                    .get("meshes")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len() as u64);
                let scene_count = json
                    .get("scenes")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len() as u64);
                let generator = json
                    .get("asset")
                    .and_then(|asset| asset.get("generator"))
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                Model3dView {
                    format: container.label().to_owned(),
                    vertex_count: None,
                    face_count: None,
                    mesh_count,
                    scene_count,
                    generator,
                    file_size,
                }
            }
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The 3D model plugin's presentation half.
#[derive(Debug, Default)]
pub struct Model3dPresentation;

impl PluginPresentation for Model3dPresentation {
    fn name(&self) -> &'static str {
        "model3d"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: Model3dView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!("{} model", view.format)];
        if let Some(vertex_count) = view.vertex_count {
            lines.push(format!("{vertex_count} vertices"));
        }
        if let Some(face_count) = view.face_count {
            lines.push(format!("{face_count} faces"));
        }
        if let Some(mesh_count) = view.mesh_count {
            lines.push(format!("{mesh_count} meshes"));
        }
        if let Some(scene_count) = view.scene_count {
            lines.push(format!("{scene_count} scenes"));
        }
        if let Some(generator) = &view.generator {
            lines.push(format!("Generated by {generator}"));
        }
        lines.push(format!("{} bytes on disk", view.file_size));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{Model3dCore, Model3dPresentation, Model3dView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-model3d-test-{}-{name}",
            std::process::id()
        ))
    }

    const OBJ_TRIANGLE: &str = "\
# a single triangle
mtllib triangle.mtl
usemtl Default
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 0.0 1.0 0.0
vn 0.0 0.0 1.0
f 1 2 3
";

    const STL_ASCII_TRIANGLE: &str = "\
solid triangle
  facet normal 0.0 0.0 1.0
    outer loop
      vertex 0.0 0.0 0.0
      vertex 1.0 0.0 0.0
      vertex 0.0 1.0 0.0
    endloop
  endfacet
endsolid triangle
";

    const GLTF_MINIMAL: &str = "\
{
  \"asset\": { \"version\": \"2.0\", \"generator\": \"test-suite\" },
  \"scenes\": [ { \"nodes\": [0] } ],
  \"meshes\": [ { \"primitives\": [] } ]
}
";

    fn write_binary_stl_triangle(path: &std::path::Path) {
        let mut bytes = vec![0u8; 80];
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 50]);
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn sniffs_obj_content() {
        assert!(Model3dCore.sniff(OBJ_TRIANGLE.as_bytes()));
        assert!(!Model3dCore.sniff(b"just some plain text"));
    }

    #[test]
    fn sniffs_ascii_stl_content() {
        assert!(Model3dCore.sniff(STL_ASCII_TRIANGLE.as_bytes()));
        assert!(!Model3dCore.sniff(b"solidify this sentence, please"));
    }

    #[test]
    fn sniffs_binary_stl_content() {
        let path = unique_temp_file("sniff.stl");
        write_binary_stl_triangle(&path);
        let prefix = std::fs::read(&path).unwrap();
        assert!(Model3dCore.sniff(&prefix));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn sniffs_gltf_content() {
        assert!(Model3dCore.sniff(GLTF_MINIMAL.as_bytes()));
        assert!(!Model3dCore.sniff(b"{\"name\": \"not a model\"}"));
    }

    #[test]
    fn views_a_real_obj_file() {
        let path = unique_temp_file("triangle.obj");
        std::fs::write(&path, OBJ_TRIANGLE).unwrap();

        let data = Model3dCore.view(&path).unwrap();
        let view: Model3dView = serde_json::from_value(data).unwrap();

        assert_eq!(view.format, "OBJ");
        assert_eq!(view.vertex_count, Some(3));
        assert_eq!(view.face_count, Some(1));
        assert!(view.file_size > 0);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_real_ascii_stl_file() {
        let path = unique_temp_file("triangle_ascii.stl");
        std::fs::write(&path, STL_ASCII_TRIANGLE).unwrap();

        let data = Model3dCore.view(&path).unwrap();
        let view: Model3dView = serde_json::from_value(data).unwrap();

        assert_eq!(view.format, "STL (ASCII)");
        assert_eq!(view.vertex_count, Some(3));
        assert_eq!(view.face_count, Some(1));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_real_binary_stl_file() {
        let path = unique_temp_file("triangle_binary.stl");
        write_binary_stl_triangle(&path);

        let data = Model3dCore.view(&path).unwrap();
        let view: Model3dView = serde_json::from_value(data).unwrap();

        assert_eq!(view.format, "STL (binary)");
        assert_eq!(view.vertex_count, Some(3));
        assert_eq!(view.face_count, Some(1));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_real_gltf_file() {
        let path = unique_temp_file("scene.gltf");
        std::fs::write(&path, GLTF_MINIMAL).unwrap();

        let data = Model3dCore.view(&path).unwrap();
        let view: Model3dView = serde_json::from_value(data).unwrap();

        assert_eq!(view.format, "glTF");
        assert_eq!(view.mesh_count, Some(1));
        assert_eq!(view.scene_count, Some(1));
        assert_eq!(view.generator.as_deref(), Some("test-suite"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_obj_geometry_counts() {
        let data = serde_json::to_value(Model3dView {
            format: "OBJ".to_owned(),
            vertex_count: Some(3),
            face_count: Some(1),
            mesh_count: None,
            scene_count: None,
            generator: None,
            file_size: 123,
        })
        .unwrap();

        let lines = Model3dPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["OBJ model", "3 vertices", "1 faces", "123 bytes on disk"]
        );
    }

    #[test]
    fn presents_gltf_scene_metadata() {
        let data = serde_json::to_value(Model3dView {
            format: "glTF".to_owned(),
            vertex_count: None,
            face_count: None,
            mesh_count: Some(2),
            scene_count: Some(1),
            generator: Some("Blender".to_owned()),
            file_size: 456,
        })
        .unwrap();

        let lines = Model3dPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "glTF model",
                "2 meshes",
                "1 scenes",
                "Generated by Blender",
                "456 bytes on disk",
            ]
        );
    }
}
