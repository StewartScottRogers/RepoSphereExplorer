//! Text file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`TextCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The text plugin's core half.
#[derive(Debug, Default)]
pub struct TextCore;

impl PluginCore for TextCore {
    fn name(&self) -> &'static str {
        "text"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok()
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let view = TextView {
            content: String::from_utf8_lossy(slice).into_owned(),
            truncated,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The text plugin's presentation half.
#[derive(Debug, Default)]
pub struct TextPresentation;

impl PluginPresentation for TextPresentation {
    fn name(&self) -> &'static str {
        "text"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: TextView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines: Vec<String> = view.content.lines().map(str::to_owned).collect();
        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_VIEW_BYTES, TextCore, TextPresentation, TextView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-text-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_valid_utf8_as_text() {
        assert!(TextCore.sniff("hello, world".as_bytes()));
        assert!(!TextCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_small_file_without_truncation() {
        let path = unique_temp_file("small.txt");
        std::fs::write(&path, "line one\nline two\n").unwrap();

        let data = TextCore.view(&path).unwrap();
        let view: TextView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content, "line one\nline two\n");
        assert!(!view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.txt");
        std::fs::write(&path, "a".repeat(MAX_VIEW_BYTES + 10)).unwrap();

        let data = TextCore.view(&path).unwrap();
        let view: TextView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_lines_with_a_truncation_marker() {
        let data = serde_json::to_value(TextView {
            content: "a\nb".to_owned(),
            truncated: true,
        })
        .unwrap();

        let lines = TextPresentation.present(&data);

        assert_eq!(lines, vec!["a", "b", "… (truncated)"]);
    }
}
