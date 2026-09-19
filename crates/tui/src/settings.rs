//! Where the terminal front end's own pane widths are remembered between
//! runs.
//!
//! D16 asks for this front end to remember which pane is wide "the way the
//! graphical front end remembers pane widths" - in terminal columns rather
//! than logical pixels, since that is the unit a reader resizes by (#650).
//! This is front-end state, not anything the service knows or cares about,
//! so it lives beside the graphical front end's own settings file, under the
//! same per-user data directory, rather than travelling through the service.

use std::path::PathBuf;

/// The Folders and Contents pane widths, in terminal columns, as the reader
/// last left them. The File pane always takes whatever is left, so only
/// these two are ever stored - the same shape the graphical front end's own
/// `PaneWidths` keeps, in pixels rather than columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneWidths {
    /// Width of the Folders tree, in columns.
    pub folders: u16,
    /// Width of the Contents pane, in columns.
    pub contents: u16,
}

/// Narrowest a pane may be, whether by resizing or by a stored width read
/// back from a settings file. A pane below this is not worth having a
/// border and a name on.
pub const MIN_WIDTH: u16 = 8;

/// Widest a remembered pane may be - wide enough for any realistic
/// terminal, narrow enough to catch a hand-edited or corrupted value.
const MAX_WIDTH: u16 = 2000;

/// `<data-local-dir>/RepoSphereExplorer/tui.json`, or `None` where the
/// platform reports no such directory.
fn settings_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("RepoSphereExplorer").join("tui.json"))
}

/// Reads a column count out of `value` by key, rejecting anything outside
/// the usable range.
fn width_field(value: &serde_json::Value, key: &str) -> Option<u16> {
    let width = value.get(key)?.as_u64()?;
    let width = u16::try_from(width).ok()?;
    (MIN_WIDTH..=MAX_WIDTH).contains(&width).then_some(width)
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
/// `widths`, every other field left as it was - a hand-edited `gui.json`
/// sharing this directory has none of these keys, and a value this front
/// end does not know about is not this front end's to drop.
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
/// Best-effort: a terminal that cannot save its layout should still exit
/// cleanly. Reads the file first and only changes the two width fields,
/// rather than overwriting it whole.
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

/// Reads the `editor` field out of `value`: the command a row is handed to
/// for "Open in editor" (#674), the same key and shape as the graphical
/// front end's own `gui.json`. `None` for a missing, non-string or blank
/// setting - the action falls back to Visual Studio Code, or reports it has
/// nothing to launch, rather than erroring.
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
        let value = serde_json::json!({ "folders_width": 24 });
        assert_eq!(width_field(&value, "folders_width"), Some(24));
    }

    #[test]
    fn a_width_outside_the_usable_range_is_ignored() {
        for width in [0_i64, i64::from(MIN_WIDTH) - 1, i64::from(MAX_WIDTH) + 1] {
            let value = serde_json::json!({ "folders_width": width });
            assert_eq!(
                width_field(&value, "folders_width"),
                None,
                "{width} should not reopen the terminal with an unusable pane"
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
    fn saving_pane_widths_keeps_a_hand_edited_unrelated_key() {
        let existing = serde_json::json!({ "editor": "subl", "folders_width": 20 });
        let merged = merged_pane_widths(
            &existing,
            PaneWidths {
                folders: 30,
                contents: 50,
            },
        );
        assert_eq!(
            merged,
            serde_json::json!({ "editor": "subl", "folders_width": 30, "contents_width": 50 })
        );
    }

    #[test]
    fn saving_pane_widths_over_a_missing_file_writes_only_the_widths() {
        let merged = merged_pane_widths(
            &serde_json::Value::Null,
            PaneWidths {
                folders: 30,
                contents: 50,
            },
        );
        assert_eq!(
            merged,
            serde_json::json!({ "folders_width": 30, "contents_width": 50 })
        );
    }

    #[test]
    fn a_configured_editor_is_read_back_trimmed() {
        let value = serde_json::json!({ "editor": "  subl  " });
        assert_eq!(editor_field(&value), Some("subl".to_owned()));
    }

    #[test]
    fn a_missing_or_blank_editor_is_treated_as_unset() {
        assert_eq!(editor_field(&serde_json::json!({})), None);
        assert_eq!(editor_field(&serde_json::json!({ "editor": "   " })), None);
        assert_eq!(editor_field(&serde_json::json!({ "editor": 5 })), None);
    }

    #[test]
    fn what_is_saved_is_what_load_would_read_back() {
        // The round trip [`load_pane_widths`] and [`save_pane_widths`]
        // themselves make, without touching the real settings file: a
        // width that survives `merged_pane_widths` still passes
        // `width_field`'s own range check on the way back in (#650).
        let merged = merged_pane_widths(
            &serde_json::Value::Null,
            PaneWidths {
                folders: 24,
                contents: 34,
            },
        );
        assert_eq!(width_field(&merged, "folders_width"), Some(24));
        assert_eq!(width_field(&merged, "contents_width"), Some(34));
    }
}
