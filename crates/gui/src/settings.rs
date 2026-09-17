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

/// The window's own geometry, as it was left: the position and size it
/// would be restored to if un-maximised, and whether it was left maximised.
/// Logical pixels, the same units [`PaneWidths`] uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowGeometry {
    /// Left edge, in the desktop's coordinate space.
    pub x: f32,
    /// Top edge, in the desktop's coordinate space.
    pub y: f32,
    /// Width of the window.
    pub width: f32,
    /// Height of the window.
    pub height: f32,
    /// Whether the window was maximised when it closed.
    pub maximized: bool,
}

/// One connected display's usable area, in the same coordinate space and
/// units as [`WindowGeometry`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayBounds {
    /// Left edge, in the desktop's coordinate space.
    pub x: f32,
    /// Top edge, in the desktop's coordinate space.
    pub y: f32,
    /// Width of the display.
    pub width: f32,
    /// Height of the display.
    pub height: f32,
}

impl DisplayBounds {
    /// Whether `geometry` shares any area with this display - the window is
    /// at least partly reachable on it.
    fn overlaps(self, geometry: WindowGeometry) -> bool {
        self.x < geometry.x + geometry.width
            && self.x + self.width > geometry.x
            && self.y < geometry.y + geometry.height
            && self.y + self.height > geometry.y
    }
}

/// Narrowest or shortest a remembered window may be.
const MIN_WINDOW_DIMENSION: f32 = 200.0;

/// Widest or tallest a remembered window may be.
const MAX_WINDOW_DIMENSION: f32 = 20_000.0;

/// Furthest off the origin a remembered window edge may sit. Wide enough for
/// a many-monitor desktop stretching well past any single display, narrow
/// enough to catch a hand-edited or corrupted value.
const MAX_POSITION: f32 = 50_000.0;

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

/// Reads a window dimension out of `value` by key, rejecting anything
/// outside the usable range - the same shape of check [`width_field`] makes
/// for a pane.
fn dimension_field(value: &serde_json::Value, key: &str) -> Option<f32> {
    #[allow(clippy::cast_possible_truncation)]
    let dimension = value.get(key)?.as_f64()? as f32;
    (dimension.is_finite() && (MIN_WINDOW_DIMENSION..=MAX_WINDOW_DIMENSION).contains(&dimension))
        .then_some(dimension)
}

/// Reads a window edge position out of `value` by key. Unlike a dimension, a
/// position may be negative - a monitor to the left of or above the primary
/// one - so only its finiteness and an outer bound are checked.
fn position_field(value: &serde_json::Value, key: &str) -> Option<f32> {
    #[allow(clippy::cast_possible_truncation)]
    let position = value.get(key)?.as_f64()? as f32;
    (position.is_finite() && (-MAX_POSITION..=MAX_POSITION).contains(&position)).then_some(position)
}

/// The remembered window geometry, or `None` when there is no usable
/// settings file, or any one of its four numbers is missing, malformed or
/// out of range - the same all-or-nothing shape [`load_pane_widths`] has,
/// so a corrupt value falls back to today's default window instead of a
/// window built from a mix of remembered and default numbers.
///
/// `gui.json` written before this work order, or with these fields removed
/// by hand, has none of these keys: this simply returns `None`, the same as
/// a missing file.
#[must_use]
pub fn load_window_geometry() -> Option<WindowGeometry> {
    let text = std::fs::read_to_string(settings_path()?).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(WindowGeometry {
        x: position_field(&value, "window_x")?,
        y: position_field(&value, "window_y")?,
        width: dimension_field(&value, "window_width")?,
        height: dimension_field(&value, "window_height")?,
        maximized: value
            .get("window_maximized")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}

/// `existing` with its five `window_*` fields set from `geometry`, every
/// other field left as it was - the geometry counterpart to
/// [`merged_pane_widths`].
fn merged_window_geometry(
    existing: &serde_json::Value,
    geometry: WindowGeometry,
) -> serde_json::Value {
    let mut existing = existing.as_object().cloned().unwrap_or_default();
    existing.insert("window_x".to_owned(), serde_json::json!(geometry.x));
    existing.insert("window_y".to_owned(), serde_json::json!(geometry.y));
    existing.insert("window_width".to_owned(), serde_json::json!(geometry.width));
    existing.insert(
        "window_height".to_owned(),
        serde_json::json!(geometry.height),
    );
    existing.insert(
        "window_maximized".to_owned(),
        serde_json::json!(geometry.maximized),
    );
    serde_json::Value::Object(existing)
}

/// Writes `geometry` to the settings file, creating its directory if
/// needed. Best-effort, and reads the file first and changes only the five
/// geometry fields, for the same reasons [`save_pane_widths`] does.
pub fn save_window_geometry(geometry: WindowGeometry) {
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
    let value = merged_window_geometry(&existing, geometry);
    if let Ok(text) = serde_json::to_string_pretty(&value) {
        let _ = std::fs::write(&path, text);
    }
}

/// `geometry`, unchanged if it still shares area with at least one of
/// `displays` - it stayed on the same monitor, or the reader dragged it to
/// another one that is still connected - or centred on `primary` and
/// shrunk to fit it otherwise: the monitor it was left on has since been
/// unplugged.
///
/// `geometry.maximized` passes through unchanged either way: it says
/// nothing about where the *normal* bounds landed, only whether the window
/// should be maximised once they are applied.
#[must_use]
pub fn geometry_on_a_display(
    geometry: WindowGeometry,
    displays: &[DisplayBounds],
    primary: DisplayBounds,
) -> WindowGeometry {
    if displays.iter().any(|display| display.overlaps(geometry)) {
        return geometry;
    }
    let width = geometry.width.min(primary.width);
    let height = geometry.height.min(primary.height);
    WindowGeometry {
        x: primary.x + (primary.width - width) / 2.0,
        y: primary.y + (primary.height - height) / 2.0,
        width,
        height,
        maximized: geometry.maximized,
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
    use super::{
        DisplayBounds, MAX_WIDTH, MAX_WINDOW_DIMENSION, MIN_WIDTH, MIN_WINDOW_DIMENSION,
        PaneWidths, WindowGeometry, dimension_field, editor_field, geometry_on_a_display,
        merged_pane_widths, merged_window_geometry, position_field, width_field,
    };

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
    fn a_window_dimension_inside_the_usable_range_is_read_back() {
        let value = serde_json::json!({ "window_width": 1024.0 });
        assert_eq!(dimension_field(&value, "window_width"), Some(1024.0));
    }

    #[test]
    fn a_window_dimension_outside_the_usable_range_is_ignored() {
        for width in [
            MIN_WINDOW_DIMENSION - 1.0,
            MAX_WINDOW_DIMENSION + 1.0,
            0.0,
            -50.0,
        ] {
            let value = serde_json::json!({ "window_width": width });
            assert_eq!(
                dimension_field(&value, "window_width"),
                None,
                "{width} should not reopen the window at an unusable size"
            );
        }
    }

    #[test]
    fn a_window_position_may_be_negative() {
        let value = serde_json::json!({ "window_x": -1200.0 });
        assert_eq!(position_field(&value, "window_x"), Some(-1200.0));
    }

    #[test]
    fn an_out_of_range_or_malformed_window_position_is_ignored() {
        let value = serde_json::json!({ "window_x": 60_000.0 });
        assert_eq!(position_field(&value, "window_x"), None);
        let value = serde_json::json!({ "window_x": "left" });
        assert_eq!(position_field(&value, "window_x"), None);
        assert_eq!(position_field(&serde_json::json!({}), "window_x"), None);
    }

    #[test]
    fn merging_pane_widths_and_window_geometry_round_trips_the_full_layout() {
        let value = merged_pane_widths(
            &serde_json::Value::Null,
            PaneWidths {
                folders: 300.0,
                contents: 500.0,
            },
        );
        let value = merged_window_geometry(
            &value,
            WindowGeometry {
                x: -10.0,
                y: 20.0,
                width: 1024.0,
                height: 768.0,
                maximized: true,
            },
        );
        assert_eq!(width_field(&value, "folders_width"), Some(300.0));
        assert_eq!(width_field(&value, "contents_width"), Some(500.0));
        assert_eq!(position_field(&value, "window_x"), Some(-10.0));
        assert_eq!(position_field(&value, "window_y"), Some(20.0));
        assert_eq!(dimension_field(&value, "window_width"), Some(1024.0));
        assert_eq!(dimension_field(&value, "window_height"), Some(768.0));
        assert_eq!(
            value
                .get("window_maximized")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn a_file_with_widths_only_has_no_window_geometry() {
        let value = serde_json::json!({ "folders_width": 260.0, "contents_width": 400.0 });
        assert_eq!(dimension_field(&value, "window_width"), None);
        assert_eq!(position_field(&value, "window_x"), None);
    }

    #[test]
    fn saving_window_geometry_keeps_a_hand_edited_editor_key() {
        let existing = serde_json::json!({ "editor": "subl", "window_width": 900.0 });
        let merged = merged_window_geometry(
            &existing,
            WindowGeometry {
                x: 0.0,
                y: 0.0,
                width: 1200.0,
                height: 800.0,
                maximized: false,
            },
        );
        assert_eq!(
            merged,
            serde_json::json!({
                "editor": "subl",
                "window_x": 0.0,
                "window_y": 0.0,
                "window_width": 1200.0,
                "window_height": 800.0,
                "window_maximized": false,
            })
        );
    }

    /// D7: nothing this module writes may name a folder, file or path - a
    /// window's geometry is layout, not a location in the Repos Directory.
    #[test]
    fn window_geometry_fields_hold_no_path() {
        let merged = merged_window_geometry(
            &serde_json::Value::Null,
            WindowGeometry {
                x: 10.0,
                y: 20.0,
                width: 800.0,
                height: 600.0,
                maximized: true,
            },
        );
        for (key, value) in merged.as_object().expect("an object") {
            assert!(
                !value.is_string(),
                "{key} holds {value}, which could be a path; only pane widths, \
                 a window's geometry and the editor command belong here"
            );
        }
    }

    #[test]
    fn a_geometry_still_on_a_display_is_unchanged() {
        let geometry = WindowGeometry {
            x: 100.0,
            y: 100.0,
            width: 800.0,
            height: 600.0,
            maximized: false,
        };
        let displays = [DisplayBounds {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        }];
        let primary = displays[0];
        assert_eq!(
            geometry_on_a_display(geometry, &displays, primary),
            geometry
        );
    }

    #[test]
    fn a_geometry_off_every_display_is_centred_on_the_primary_and_shrunk_to_fit() {
        // The monitor this was last on, to the right of the primary one, is
        // gone: only the primary remains connected.
        let geometry = WindowGeometry {
            x: 2000.0,
            y: 100.0,
            width: 2200.0,
            height: 600.0,
            maximized: false,
        };
        let primary = DisplayBounds {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        let resolved = geometry_on_a_display(geometry, &[primary], primary);
        // Width shrinks to the primary display's; height was already
        // shorter, so it passes through; centring the shrunk width leaves
        // no horizontal margin, but the untouched height leaves one above
        // and below.
        assert_eq!(
            resolved,
            WindowGeometry {
                x: 0.0,
                y: 240.0,
                width: 1920.0,
                height: 600.0,
                maximized: false,
            }
        );
    }

    #[test]
    fn a_maximised_geometry_off_every_display_still_reports_maximised() {
        let geometry = WindowGeometry {
            x: 5000.0,
            y: 5000.0,
            width: 800.0,
            height: 600.0,
            maximized: true,
        };
        let primary = DisplayBounds {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        let resolved = geometry_on_a_display(geometry, &[primary], primary);
        assert!(resolved.maximized);
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
