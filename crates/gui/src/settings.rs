//! Where the window's own layout is remembered between runs.
//!
//! This is front-end state, not anything the service knows or cares about:
//! how wide the panes are says nothing about the filesystem. It lives beside
//! the service's journal, under the same per-user data directory, so a user
//! looking for what this application keeps finds one place.

use std::path::PathBuf;

/// The pane widths, as the splitters last left them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneWidths {
    /// Width of the folders tree, in logical pixels.
    pub folders: f32,
    /// Width of the contents list, in logical pixels.
    pub contents: f32,
}

/// Narrowest a remembered pane may be. A stored width below this would
/// reopen the window with a pane too small to use, whether it got there by a
/// hand-edited file or by a display that has since changed size.
const MIN_WIDTH: f32 = 120.0;

/// Widest a remembered pane may be, for the same reason in the other
/// direction.
const MAX_WIDTH: f32 = 4000.0;

/// `<data-local-dir>/RepoSphereExplorer/gui.json`, or `None` where the
/// platform reports no such directory.
fn settings_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("RepoSphereExplorer").join("gui.json"))
}

/// Reads a number out of `value` by key, rejecting anything outside the
/// usable range.
fn width_field(value: &serde_json::Value, key: &str) -> Option<f32> {
    #[allow(clippy::cast_possible_truncation)]
    let width = value.get(key)?.as_f64()? as f32;
    (width.is_finite() && (MIN_WIDTH..=MAX_WIDTH).contains(&width)).then_some(width)
}

/// The remembered pane widths, or `None` when there is no usable settings
/// file. A missing, unreadable or malformed file is not an error worth
/// reporting: the window simply opens at its default layout.
#[must_use]
pub fn load_pane_widths() -> Option<PaneWidths> {
    let text = std::fs::read_to_string(settings_path()?).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(PaneWidths {
        folders: width_field(&value, "folders_width")?,
        contents: width_field(&value, "contents_width")?,
    })
}

/// Writes `widths` to the settings file, creating its directory if needed.
/// Best-effort: a window that cannot save its layout should still close.
pub fn save_pane_widths(widths: PaneWidths) {
    let Some(path) = settings_path() else {
        return;
    };
    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return;
    }
    let value = serde_json::json!({
        "folders_width": widths.folders,
        "contents_width": widths.contents,
    });
    if let Ok(text) = serde_json::to_string_pretty(&value) {
        let _ = std::fs::write(&path, text);
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_WIDTH, MIN_WIDTH, width_field};

    #[test]
    fn a_width_inside_the_usable_range_is_read_back() {
        let value = serde_json::json!({ "folders_width": 260.0 });
        assert_eq!(width_field(&value, "folders_width"), Some(260.0));
    }

    #[test]
    fn a_width_outside_the_usable_range_is_ignored() {
        for width in [MIN_WIDTH - 1.0, MAX_WIDTH + 1.0, 0.0, -50.0] {
            let value = serde_json::json!({ "folders_width": width });
            assert_eq!(
                width_field(&value, "folders_width"),
                None,
                "{width} should not reopen the window with an unusable pane"
            );
        }
    }

    #[test]
    fn a_missing_or_unreadable_field_is_ignored() {
        assert_eq!(width_field(&serde_json::json!({}), "folders_width"), None);
        let text = serde_json::json!({ "folders_width": "wide" });
        assert_eq!(width_field(&text, "folders_width"), None);
    }
}
