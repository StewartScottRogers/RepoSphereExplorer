//! `NumPy` array file type plugin: core and presentation halves.
//!
//! A `.npy` file is a short header saying the shape, the data type and
//! the layout, then the numbers. A `.npz` is a zip archive of those.
//! This reads the format version, the shape, the type, the element
//! count, whether the array is laid out column by column - and, for an
//! archive, the arrays it holds.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Read, Seek};
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["npy", "npz"];

/// The magic every `.npy` file opens with.
const MAGIC: &[u8] = b"\x93NUMPY";

/// One array, whether alone in a file or inside an archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Array {
    /// Its name. Empty for an array that is the whole file.
    pub name: String,
    /// Its dimensions.
    pub shape: Vec<usize>,
    /// Its data type, as `NumPy` spells it.
    pub kind: String,
    /// How many elements it holds.
    pub elements: usize,
    /// Whether it is laid out column by column rather than row by row.
    pub column_major: bool,
}

/// View data produced by [`NumpyCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumpyView {
    /// `array` for a `.npy`, `archive` for a `.npz`.
    pub kind: String,
    /// The format version the file was written to.
    pub format_version: String,
    /// Every array the file holds.
    pub arrays: Vec<Array>,
    /// Arrays laid out column by column. A reader that ignores the flag
    /// gets a transposed array and is never told.
    pub column_major: Vec<String>,
    /// Data types that carry Python objects rather than numbers, which
    /// `NumPy` loads by unpickling - and unpickling runs whatever the
    /// stream says to.
    pub object_arrays: Vec<String>,
}

/// The value of `key` in a `NumPy` header dictionary.
///
/// The header is a Python literal, not JSON: single quotes, `True` and
/// `False`, and a trailing comma in every tuple of one.
fn field<'a>(header: &'a str, key: &str) -> Option<&'a str> {
    let at = header.find(&format!("'{key}':"))?;
    let rest = header[at + key.len() + 3..].trim_start();
    let end = match rest.chars().next()? {
        '(' => rest.find(')')? + 1,
        '\'' => rest[1..].find('\'')? + 2,
        _ => rest.find([',', '}']).unwrap_or(rest.len()),
    };
    Some(rest[..end].trim())
}

/// The dimensions in a shape tuple like `(24, 6)`.
fn shape_of(said: &str) -> Vec<usize> {
    said.trim_matches(['(', ')'])
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

/// The array a `.npy` header describes.
fn array_of(name: &str, header: &str) -> Option<Array> {
    let shape = shape_of(field(header, "shape")?);
    let kind = field(header, "descr")?.trim_matches('\'').to_owned();
    Some(Array {
        name: name.to_owned(),
        elements: shape
            .iter()
            .product::<usize>()
            .max(usize::from(shape.is_empty())),
        shape,
        kind,
        column_major: field(header, "fortran_order") == Some("True"),
    })
}

/// The header of a `.npy` stream, and the version it was written to.
fn header_of(bytes: &[u8]) -> Option<(String, String)> {
    if !bytes.starts_with(MAGIC) {
        return None;
    }
    let major = *bytes.get(6)?;
    let minor = *bytes.get(7)?;
    // Version 1 writes a two-byte header length; 2 and 3 write four.
    let (length, from): (usize, usize) = if major >= 2 {
        let raw = u32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?);
        (usize::try_from(raw).ok()?, 12)
    } else {
        let raw = u16::from_le_bytes(bytes.get(8..10)?.try_into().ok()?);
        (usize::from(raw), 10)
    };
    let header = bytes.get(from..from.checked_add(length)?)?;
    Some((
        String::from_utf8_lossy(header).into_owned(),
        format!("{major}.{minor}"),
    ))
}

/// Everything [`NumpyView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<NumpyView> {
    let mut handle = std::fs::File::open(path)?;
    let mut opening = [0u8; 4];
    let read = handle.read(&mut opening)?;
    handle.rewind()?;

    let mut view = NumpyView {
        kind: "array".to_owned(),
        format_version: "unstated".to_owned(),
        arrays: Vec::new(),
        column_major: Vec::new(),
        object_arrays: Vec::new(),
    };

    if opening.get(..2) == Some(b"PK") && read >= 2 {
        // A `.npz` is a zip archive whose members are each a `.npy`.
        "archive".clone_into(&mut view.kind);
        let mut archive = zip::ZipArchive::new(handle)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        for index in 0..archive.len() {
            let mut member = archive
                .by_index(index)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            let name = member.name().trim_end_matches(".npy").to_owned();
            // Only the head is read: the header is a few hundred bytes
            // and the numbers behind it may be gigabytes.
            let mut head = vec![0u8; 4096];
            let got = member.read(&mut head)?;
            head.truncate(got);
            if let Some((header, version)) = header_of(&head)
                && let Some(array) = array_of(&name, &header)
            {
                view.format_version = version;
                view.arrays.push(array);
            }
        }
    } else {
        let mut head = vec![0u8; 4096];
        let got = handle.read(&mut head)?;
        head.truncate(got);
        let (header, version) = header_of(&head)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not a NumPy array file"))?;
        view.format_version = version;
        view.arrays.push(array_of("", &header).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "the header names no shape")
        })?);
    }

    for array in &view.arrays {
        let named = if array.name.is_empty() {
            "the array".to_owned()
        } else {
            array.name.clone()
        };
        if array.column_major {
            view.column_major.push(named.clone());
        }
        // `|O` is the object data type: `NumPy` stores those by pickling
        // them, and loading one unpickles.
        if array.kind.contains('O') {
            view.object_arrays.push(named);
        }
    }
    Ok(view)
}

/// Whether `prefix` opens like a `NumPy` array or an archive of them.
fn looks_like_it(prefix: &[u8]) -> bool {
    // A `.npz` is a zip archive, and `archive` already claims those; the
    // extension is what settles it, so only the plain form is sniffed.
    prefix.starts_with(MAGIC)
}

/// The `NumPy` array plugin's core half.
#[derive(Debug, Default)]
pub struct NumpyCore;

impl PluginCore for NumpyCore {
    fn name(&self) -> &'static str {
        "numpy"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A `.npz` is a zip archive, which `archive` recognises. This is
        // the narrower reading of the same bytes (D13).
        &["archive"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_it(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The `NumPy` array plugin's presentation half.
#[derive(Debug, Default)]
pub struct NumpyPresentation;

impl PluginPresentation for NumpyPresentation {
    fn name(&self) -> &'static str {
        "numpy"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "NPY",
            tint: 0x004d_77cf,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: NumpyView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "NumPy {}, format version {}",
            view.kind, view.format_version
        )];
        for array in &view.arrays {
            let shape = array
                .shape
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(" x ");
            let shape = if shape.is_empty() {
                "a single value".to_owned()
            } else {
                shape
            };
            let named = if array.name.is_empty() {
                String::new()
            } else {
                format!("{}: ", array.name)
            };
            lines.push(format!(
                "  {named}{shape} of {} ({} element(s))",
                array.kind, array.elements
            ));
        }
        if !view.column_major.is_empty() {
            lines.push("Laid out column by column, so anything reading it row by".to_owned());
            lines.push("row without checking gets a transposed array:".to_owned());
            for name in &view.column_major {
                lines.push(format!("  {name}"));
            }
        }
        if !view.object_arrays.is_empty() {
            lines.push("Holds Python objects rather than numbers, which NumPy".to_owned());
            lines.push("loads by unpickling - and unpickling runs whatever the".to_owned());
            lines.push("stream says to:".to_owned());
            for name in &view.object_arrays {
                lines.push(format!("  {name}"));
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{NumpyCore, NumpyPresentation, NumpyView, field, looks_like_it, shape_of};
    use plugin_api::{PluginCore, PluginPresentation};

    fn sample(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/numpy")
            .join(name)
    }

    fn view_of(name: &str) -> NumpyView {
        serde_json::from_value(NumpyCore.view(&sample(name)).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&NumpyCore),
            PluginPresentation::extensions(&NumpyPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn sniffs_the_magic() {
        assert!(looks_like_it(b"\x93NUMPY\x01\x00"));
        assert!(
            !looks_like_it(b"PK\x03\x04"),
            "a zip is settled by its extension"
        );
        assert!(!looks_like_it(b""));
    }

    #[test]
    fn it_says_it_specialises_the_archive_reading() {
        assert_eq!(NumpyCore.specialises(), &["archive"]);
    }

    #[test]
    fn a_header_is_a_python_literal_not_json() {
        let header = "{'descr': '<f8', 'fortran_order': False, 'shape': (24, 6), }";

        assert_eq!(field(header, "descr"), Some("'<f8'"));
        assert_eq!(field(header, "fortran_order"), Some("False"));
        assert_eq!(field(header, "shape"), Some("(24, 6)"));
        assert_eq!(shape_of("(24, 6)"), vec![24, 6]);
        assert_eq!(shape_of("(7,)"), vec![7], "a tuple of one keeps its comma");
    }

    #[test]
    fn reads_a_plain_array() {
        let view = view_of("readings.npy");

        assert_eq!(view.kind, "array");
        assert_eq!(view.arrays.len(), 1);
        assert_eq!(view.arrays[0].shape, vec![24, 6]);
        assert_eq!(view.arrays[0].elements, 144);
        assert!(view.arrays[0].kind.contains("f8"));
        assert!(!view.arrays[0].column_major);
        assert!(view.column_major.is_empty());
    }

    #[test]
    fn names_an_array_laid_out_the_other_way_round() {
        let view = view_of("column-major.npy");

        assert!(view.arrays[0].column_major);
        assert_eq!(view.column_major.len(), 1);
    }

    #[test]
    fn reads_every_array_in_an_archive() {
        let view = view_of("station.npz");

        assert_eq!(view.kind, "archive");
        assert_eq!(view.arrays.len(), 3);
        let named: Vec<&str> = view.arrays.iter().map(|a| a.name.as_str()).collect();
        assert!(named.contains(&"identifiers"));
        assert!(named.contains(&"readings"));
        assert!(named.contains(&"calibrated"));
        assert!(
            view.arrays.iter().any(|a| a.kind.contains("i4")),
            "the identifiers are 32-bit integers"
        );
        assert!(view.arrays.iter().any(|a| a.kind.contains("b1")));
    }

    #[test]
    fn presents_the_transposition_warning_with_its_reason() {
        let data = NumpyCore.view(&sample("column-major.npy")).unwrap();

        let lines = NumpyPresentation.present(&data);

        assert!(lines[0].starts_with("NumPy array, format version"));
        assert!(lines.iter().any(|line| line.contains("transposed array")));
    }

    #[test]
    fn a_file_that_is_not_numpy_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.npy");
        std::fs::write(&path, b"\x93NUMPY and then nothing of the sort").unwrap();

        assert!(NumpyCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
