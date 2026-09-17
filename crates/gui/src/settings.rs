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

/// `existing` with its `folders_width` and `contents_width` fields set from
/// `widths`, every other field - a hand-edited `editor` key (#581) among
/// them - left as it was. A non-object `existing` (a missing or malformed
/// file) is treated as empty rather than kept, since there is nothing in it
/// worth preserving.
fn merged_pane_widths(existing: &serde_json::Value, widths: PaneWidths) -> serde_json::Value {
    let mut existing = existing.as_object().cloned().unwrap_or_default();
    existing.insert(
        "folders_width".to_owned(),
        serde_json::json!(widths.folders),
    );
    existing.insert(
        "contents_width".to_owned(),
        serde_json::json!(widths.contents),
    );
    serde_json::Value::Object(existing)
}

/// Writes `widths` to the settings file, creating its directory if needed.
/// Best-effort: a window that cannot save its layout should still close.
///
/// Reads the file first and only changes the two width fields, rather than
/// overwriting it whole: a hand-edited `editor` key would otherwise be lost
/// the next time a splitter moved.
pub fn save_pane_widths(widths: PaneWidths) {
    let Some(path) = settings_path() else {
        return;
    };
    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return;
    }
    let existing = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(serde_json::Value::Null);
    let value = merged_pane_widths(&existing, widths);
    if let Ok(text) = serde_json::to_string_pretty(&value) {
        let _ = std::fs::write(&path, text);
    }
}

/// Reads the `editor` field out of `value`: the command a folder is handed
/// to for "Open in editor" (#581). `None` for a missing, non-string or
/// blank setting, the same as every other field this module reads - the
/// menu item this drives falls back to Visual Studio Code, or disables
/// itself, rather than erroring.
fn editor_field(value: &serde_json::Value) -> Option<String> {
    let editor = value.get("editor")?.as_str()?.trim();
    (!editor.is_empty()).then(|| editor.to_owned())
}

/// The `editor` setting from the settings file. `None` for a missing,
/// unreadable or malformed file, the same as [`load_pane_widths`].
#[must_use]
pub fn load_editor() -> Option<String> {
    let text = std::fs::read_to_string(settings_path()?).ok()?;
    editor_field(&serde_json::from_str(&text).ok()?)
}

#[cfg(test)]
mod tests {
    use super::{MAX_WIDTH, MIN_WIDTH, PaneWidths, editor_field, merged_pane_widths, width_field};

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

    #[test]
    fn saving_pane_widths_keeps_a_hand_edited_editor_key() {
        let existing = serde_json::json!({ "editor": "subl", "folders_width": 200.0 });
        let merged = merged_pane_widths(
            &existing,
            PaneWidths {
                folders: 300.0,
                contents: 500.0,
            },
        );
        assert_eq!(
            merged,
            serde_json::json!({ "editor": "subl", "folders_width": 300.0, "contents_width": 500.0 })
        );
    }

    #[test]
    fn saving_pane_widths_over_a_missing_file_writes_only_the_widths() {
        let merged = merged_pane_widths(
            &serde_json::Value::Null,
            PaneWidths {
                folders: 300.0,
                contents: 500.0,
            },
        );
        assert_eq!(
            merged,
            serde_json::json!({ "folders_width": 300.0, "contents_width": 500.0 })
        );
    }

    #[test]
    fn an_editor_setting_is_read_back() {
        let value = serde_json::json!({ "editor": "subl" });
        assert_eq!(editor_field(&value), Some("subl".to_owned()));
    }

    #[test]
    fn a_blank_or_missing_editor_setting_is_none() {
        assert_eq!(editor_field(&serde_json::json!({})), None);
        assert_eq!(editor_field(&serde_json::json!({ "editor": "   " })), None);
        assert_eq!(editor_field(&serde_json::json!({ "editor": 5 })), None);
    }
}
