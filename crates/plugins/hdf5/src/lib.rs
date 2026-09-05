//! HDF5 file type plugin: core and presentation halves.

use hdf5_metno::Group;
use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// HDF5's fixed 8-byte superblock signature, present at the very start of
/// every HDF5 file.
const HDF5_MAGIC: &[u8] = b"\x89HDF\r\n\x1a\n";

/// One dataset found while walking the file's group hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hdf5Dataset {
    /// The dataset's full path within the file, e.g. `/group/dataset`.
    pub path: String,
    /// The dataset's shape, one entry per dimension; empty for a scalar.
    pub shape: Vec<usize>,
    /// The dataset's element type, as HDF5 describes it.
    pub dtype: String,
}

/// View data produced by [`Hdf5Core::view`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Hdf5View {
    /// Every group's full path within the file, excluding the implicit
    /// root group, in traversal order.
    pub groups: Vec<String>,
    /// Every dataset in the file, in traversal order.
    pub datasets: Vec<Hdf5Dataset>,
}

/// Whether `prefix` opens with HDF5's fixed superblock signature.
fn looks_like_hdf5(prefix: &[u8]) -> bool {
    prefix.starts_with(HDF5_MAGIC)
}

/// Walks `group`'s subtree, appending every nested group's full path to
/// `view.groups` and every nested dataset to `view.datasets`.
fn walk(group: &Group, view: &mut Hdf5View) -> hdf5_metno::Result<()> {
    for child in group.groups()? {
        view.groups.push(child.name());
        walk(&child, view)?;
    }
    for dataset in group.datasets()? {
        let dtype = dataset
            .dtype()
            .and_then(|dtype| dtype.to_descriptor())
            .map_or_else(|err| format!("unknown ({err})"), |desc| format!("{desc:?}"));
        view.datasets.push(Hdf5Dataset {
            path: dataset.name(),
            shape: dataset.shape(),
            dtype,
        });
    }
    Ok(())
}

/// Opens `path` as an HDF5 file and walks its group hierarchy from the root.
fn read_hdf5(path: &Path) -> io::Result<Hdf5View> {
    let to_io_err = |err: hdf5_metno::Error| io::Error::new(io::ErrorKind::InvalidData, err);

    let file = hdf5_metno::File::open(path).map_err(to_io_err)?;
    let mut view = Hdf5View::default();
    walk(&file, &mut view).map_err(to_io_err)?;
    Ok(view)
}

/// Renders a dataset's shape as `a×b×c`, or `scalar` for a zero-dimensional
/// dataset.
fn render_shape(shape: &[usize]) -> String {
    if shape.is_empty() {
        return "scalar".to_owned();
    }
    shape
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("×")
}

/// The HDF5 plugin's core half.
#[derive(Debug, Default)]
pub struct Hdf5Core;

impl PluginCore for Hdf5Core {
    fn name(&self) -> &'static str {
        "hdf5"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_hdf5(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read_hdf5(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The HDF5 plugin's presentation half.
#[derive(Debug, Default)]
pub struct Hdf5Presentation;

impl PluginPresentation for Hdf5Presentation {
    fn name(&self) -> &'static str {
        "hdf5"
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: Hdf5View = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };

        if view.groups.is_empty() && view.datasets.is_empty() {
            return vec!["no groups or datasets".to_owned()];
        }

        let mut lines = Vec::new();
        lines.extend(view.groups.iter().map(|path| format!("{path}/")));
        lines.extend(view.datasets.iter().map(|dataset| {
            format!(
                "{} [{}] {}",
                dataset.path,
                render_shape(&dataset.shape),
                dataset.dtype
            )
        }));
        lines.sort();
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{Hdf5Core, Hdf5Dataset, Hdf5Presentation, Hdf5View};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-hdf5-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// Writes a real HDF5 file at `path` with a nested group hierarchy and
    /// one dataset each at the root and inside the nested group.
    fn write_test_file(path: &std::path::Path) {
        let file = hdf5_metno::File::create(path).unwrap();
        file.new_dataset::<i32>()
            .shape(3)
            .create("root_ints")
            .unwrap();

        let group = file.create_group("measurements").unwrap();
        group
            .new_dataset::<f64>()
            .shape((2, 2))
            .create("readings")
            .unwrap();
    }

    #[test]
    fn sniffs_the_hdf5_superblock_signature() {
        assert!(Hdf5Core.sniff(b"\x89HDF\r\n\x1a\nrest of header"));
        assert!(!Hdf5Core.sniff(b"not an hdf5 file"));
        assert!(!Hdf5Core.sniff(b""));
    }

    #[test]
    fn views_a_real_file_and_lists_its_groups_and_datasets() {
        let path = unique_temp_file("data.h5");
        write_test_file(&path);

        let data = Hdf5Core.view(&path).unwrap();
        let view: Hdf5View = serde_json::from_value(data).unwrap();

        assert_eq!(view.groups, vec!["/measurements".to_owned()]);
        assert_eq!(view.datasets.len(), 2);
        let root_dataset = view
            .datasets
            .iter()
            .find(|dataset| dataset.path == "/root_ints")
            .unwrap();
        assert_eq!(root_dataset.shape, vec![3]);
        let nested_dataset = view
            .datasets
            .iter()
            .find(|dataset| dataset.path == "/measurements/readings")
            .unwrap();
        assert_eq!(nested_dataset.shape, vec![2, 2]);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_groups_and_datasets_sorted_by_path() {
        let data = serde_json::to_value(Hdf5View {
            groups: vec!["/measurements".to_owned()],
            datasets: vec![
                Hdf5Dataset {
                    path: "/root_ints".to_owned(),
                    shape: vec![3],
                    dtype: "Integer(4)".to_owned(),
                },
                Hdf5Dataset {
                    path: "/measurements/readings".to_owned(),
                    shape: vec![2, 2],
                    dtype: "Float(8)".to_owned(),
                },
            ],
        })
        .unwrap();

        let lines = Hdf5Presentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "/measurements/",
                "/measurements/readings [2×2] Float(8)",
                "/root_ints [3] Integer(4)",
            ]
        );
    }

    #[test]
    fn presents_an_empty_file_with_a_placeholder_message() {
        let data = serde_json::to_value(Hdf5View::default()).unwrap();

        let lines = Hdf5Presentation.present(&data);

        assert_eq!(lines, vec!["no groups or datasets"]);
    }
}
