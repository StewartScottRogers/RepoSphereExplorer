//! Slint front end: renders state and sends intents to the service.

// Slint's generated component code (from build.rs, compiling ui/app.slint)
// carries no doc comments; scope the exception to this module rather than
// the whole crate.
#[allow(missing_docs)]
mod generated {
    slint::include_modules!();
}
pub use generated::{ContentRow, MainWindow};

pub mod app;
pub mod settings;

use app::App;
use plugin_api::{Graphic, Icon};
use slint::{Image, ModelRc, SharedPixelBuffer, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// Rendered icons, keyed by the label and tint they were drawn from.
    /// A folder of a thousand files holds a handful of distinct types, so
    /// this turns per-row rasterisation into per-type.
    static ICON_CACHE: RefCell<HashMap<(&'static str, u32), Image>> =
        RefCell::new(HashMap::new());
}

/// Draws `icon` as a document sheet with a folded corner and a coloured
/// band carrying the type's label, or as a folder for the directory plugin.
/// The plugin owns the label and the colour (GUIDANCE.md §3); the shape is
/// shared, so a listing reads as one set rather than eighty-one drawings.
fn icon_svg(icon: Icon, folder: bool) -> String {
    let (r, g, b) = (
        (icon.tint >> 16) & 0xff,
        (icon.tint >> 8) & 0xff,
        icon.tint & 0xff,
    );
    let tint = format!("#{r:02x}{g:02x}{b:02x}");
    if folder {
        return format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32' viewBox='0 0 32 32'>             <path d='M2 7a2 2 0 0 1 2-2h8l3 3h11a2 2 0 0 1 2 2v15a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2z'              fill='{tint}'/>             <path d='M2 12h28v13a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2z' fill='{tint}'              fill-opacity='0.75'/></svg>"
        );
    }
    // A type with no label is the generic document: a plain sheet, no band.
    let band = if icon.label.is_empty() {
        String::new()
    } else {
        // The label has to fit the band, so it shrinks as it lengthens.
        let font = match icon.label.chars().count() {
            0..=2 => 11,
            3 => 9,
            _ => 7,
        };
        let label = icon
            .label
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        format!(
            "<rect x='5' y='17' width='21' height='10' rx='1.5' fill='{tint}'/>             <text x='15.5' y='24.4' font-family='Segoe UI, sans-serif' font-size='{font}'              font-weight='700' fill='#ffffff' text-anchor='middle'>{label}</text>"
        )
    };
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32' viewBox='0 0 32 32'>         <path d='M6 2h13l7 7v21a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1z'          fill='#ffffff' stroke='#9ca3af' stroke-width='1.2'/>         <path d='M19 2l7 7h-7z' fill='#d1d5db'/>{band}</svg>"
    )
}

/// Turns a plugin's [`Graphic`] into something Slint can draw. Decoded
/// pixels are handed over as they are; SVG source is rendered by Slint,
/// which already does that for the file-type icons.
fn graphic_image(graphic: &Graphic) -> Option<Image> {
    match graphic {
        Graphic::Rgba {
            width,
            height,
            pixels,
        } => {
            let expected = (*width as usize) * (*height as usize) * 4;
            // A plugin that miscounts its own pixels would otherwise panic
            // the front end inside Slint's buffer constructor.
            (pixels.len() == expected && expected > 0).then(|| {
                Image::from_rgba8(SharedPixelBuffer::clone_from_slice(pixels, *width, *height))
            })
        }
        Graphic::Svg(source) => Image::load_from_svg_data(source.as_bytes()).ok(),
    }
}

/// The rendered image for `icon`, drawing it the first time it is asked for.
fn icon_image(icon: Icon, folder: bool) -> Image {
    ICON_CACHE.with_borrow_mut(|cache| {
        cache
            .entry((icon.label, icon.tint))
            .or_insert_with(|| {
                let svg = icon_svg(icon, folder);
                Image::load_from_svg_data(svg.as_bytes()).unwrap_or_default()
            })
            .clone()
    })
}

/// Row height in `app.slint`'s panes. The scroll arithmetic below has to
/// agree with what is drawn, and a listing draws row `i` at `i * ROW_HEIGHT`.
const ROW_HEIGHT: f32 = 20.0;

/// Where a pane should be scrolled to so that row `selected` is fully
/// visible, given how much of the listing is on screen and where it is
/// scrolled now.
///
/// Offsets are what a `ScrollView` uses: zero at the top of the listing, and
/// negative as it scrolls down.
///
/// Moves by the smallest amount that does the job - to the top edge if the
/// row sits above the viewport, to the bottom edge if it sits below, and not
/// at all if it is already visible. A pane that jumped a full page whenever
/// the selection moved one row past the fold would be worse than one that
/// never scrolled: the reader would lose their place every time.
///
/// This lives in Rust rather than in a `changed selected` handler in
/// `app.slint` because such a handler is never dispatched without an event
/// loop, so nothing could test it - and the three defects this project has
/// already had in that file were all of that kind.
#[must_use]
pub fn scroll_offset_for(selected: usize, viewport_height: f32, current: f32) -> f32 {
    // A viewport that has not been laid out yet cannot be reasoned about,
    // and a listing shorter than its pane never scrolls.
    if viewport_height <= 0.0 {
        return current;
    }

    // A listing that reached the precision limit here would hold sixteen
    // million rows, and would have run out of memory long before it ran out
    // of mantissa. `u16` covers a listing anybody can scroll and converts
    // exactly, so the arithmetic stays honest without a cast that lies.
    let Ok(index) = u16::try_from(selected) else {
        // Past that, scroll to the end of what can be addressed and stop:
        // an answer that is off by a row is better than one that is off by
        // a listing.
        return -(f32::from(u16::MAX) * ROW_HEIGHT);
    };
    let top = f32::from(index) * ROW_HEIGHT;
    let bottom = top + ROW_HEIGHT;
    // `current` is zero or negative; the visible band is what it exposes.
    let visible_top = -current;
    let visible_bottom = visible_top + viewport_height;

    if top < visible_top {
        -top
    } else if bottom > visible_bottom {
        viewport_height - bottom
    } else {
        current
    }
}

/// Copies `app`'s current state into `ui`'s bound properties.
pub fn sync_ui(ui: &MainWindow, app: &App) {
    ui.set_folder_rows(string_model(app.folder_labels()));
    ui.set_folder_selected(row_index(app.folder_selected()));
    ui.set_content_rows(ModelRc::new(VecModel::from(
        app.content_rows()
            .into_iter()
            .enumerate()
            .map(|(index, row)| ContentRow {
                icon: icon_image(row.icon, row.is_dir),
                is_repository: row.is_repository,
                name: row.name.into(),
                size: row.size.into(),
                kind: row.kind.into(),
                modified: row.modified.into(),
                selected: app.is_selected(index),
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_content_selected(row_index(app.content_selected()));
    // Whatever moved the selection - a click, type-ahead, an arrow key,
    // Home or End, or the reselect after an operation - it lands here, so
    // one adjustment per render covers every one of them.
    ui.set_content_scroll_y(scroll_offset_for(
        app.content_selected(),
        ui.get_content_viewport_height(),
        ui.get_content_scroll_y(),
    ));
    ui.set_folders_scroll_y(scroll_offset_for(
        app.folder_selected(),
        ui.get_folders_viewport_height(),
        ui.get_folders_scroll_y(),
    ));
    let graphic = app.file_graphic().as_ref().and_then(graphic_image);
    ui.set_file_has_graphic(graphic.is_some());
    ui.set_file_graphic(graphic.unwrap_or_default());
    ui.set_file_views(string_model(
        app.file_views().into_iter().map(str::to_owned).collect(),
    ));
    ui.set_file_view_index(row_index(app.file_view_index()));
    ui.set_file_text(app.file_text().into());
    ui.set_status_text(app.status_text().into());
    ui.set_focus_pane(app.focus_index());
    ui.set_content_is_archive(app.selected_is_archive());
    ui.set_content_prompt_text(app.prompt_text().into());
    ui.set_content_prompt_row(app.prompt_row());
    ui.set_content_prompt_editable(app.prompt_is_editable());
    ui.set_content_sort_column(app.sort_column());
    ui.set_content_sort_ascending(app.sort_ascending());
    ui.set_has_selection(app.has_selection());
    ui.set_can_paste(app.can_paste());
    ui.set_breadcrumbs(string_model(app.breadcrumbs()));
    ui.set_can_go_back(app.can_go_back());
    ui.set_can_go_forward(app.can_go_forward());
    ui.set_path_input(app.path_input().into());
    ui.set_editing_path(app.editing_path());
    ui.set_editing_file(app.editing_file());
    ui.set_can_edit(app.can_edit());
    if app.editing_file() {
        // Only while the editor is open: writing this back every sync would
        // fight the cursor as the user types.
        let text = app.edit_text();
        if ui.get_edit_text() != text.as_str() {
            ui.set_edit_text(text.into());
        }
    }
    ui.set_location_icon(icon_image(app::icon_for("", true), true));
}

/// Converts a row index to the `i32` Slint properties expect, saturating
/// rather than panicking on the (unreachable in practice) overflow case.
fn row_index(index: usize) -> i32 {
    i32::try_from(index).unwrap_or(i32::MAX)
}

fn string_model(items: Vec<String>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        items
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{ROW_HEIGHT, scroll_offset_for};

    /// Offsets are whole multiples of a row height, so anything inside a
    /// pixel is the same answer. Stated once rather than comparing floats
    /// for exact equality all through the file.
    fn same(left: f32, right: f32) -> bool {
        (left - right).abs() < 1.0
    }

    /// A pane showing ten rows at a time.
    const VIEWPORT: f32 = 10.0 * ROW_HEIGHT;

    #[test]
    fn a_row_already_visible_does_not_move_the_pane() {
        // Rows 0 to 9 are on screen; selecting any of them changes nothing,
        // which is what stops the listing twitching as the reader arrows
        // down it.
        for selected in 0..10 {
            assert!(same(scroll_offset_for(selected, VIEWPORT, 0.0), 0.0));
        }
    }

    #[test]
    fn a_row_below_the_fold_comes_to_the_bottom_edge() {
        // Row 10 is one past the last visible row, so the pane moves by
        // exactly one row - not by a page.
        assert!(same(scroll_offset_for(10, VIEWPORT, 0.0), -ROW_HEIGHT));
        assert!(same(
            scroll_offset_for(11, VIEWPORT, 0.0),
            -2.0 * ROW_HEIGHT
        ));
    }

    #[test]
    fn a_row_far_below_puts_that_row_last() {
        let offset = scroll_offset_for(199, VIEWPORT, 0.0);

        // Row 199 occupies the band ending at the bottom edge.
        let visible_top = -offset;
        let visible_bottom = visible_top + VIEWPORT;
        let row_bottom = 200.0 * ROW_HEIGHT;
        assert!(same(visible_bottom, row_bottom));
    }

    #[test]
    fn a_row_above_the_fold_comes_to_the_top_edge() {
        // Scrolled down to row 100, then the selection jumps back to 40.
        let scrolled = -100.0 * ROW_HEIGHT;

        let offset = scroll_offset_for(40, VIEWPORT, scrolled);

        assert!(same(offset, -40.0 * ROW_HEIGHT), "the row sits at the top");
    }

    #[test]
    fn the_first_row_scrolls_the_listing_home() {
        assert!(same(
            scroll_offset_for(0, VIEWPORT, -100.0 * ROW_HEIGHT),
            0.0
        ));
    }

    #[test]
    fn a_pane_that_has_not_been_laid_out_is_left_alone() {
        // Before the first layout the viewport has no height, and an
        // arithmetic answer from that would scroll the listing off screen.
        assert!(same(scroll_offset_for(50, 0.0, -20.0), -20.0));
    }

    #[test]
    fn moving_one_row_at_a_time_scrolls_one_row_at_a_time() {
        // Walking down past the fold: each step moves the pane by exactly a
        // row, so the selected row stays at the bottom edge rather than the
        // view jumping ahead of the reader.
        let mut offset = 0.0;
        for selected in 0..30 {
            offset = scroll_offset_for(selected, VIEWPORT, offset);
        }

        assert!(same(offset, -20.0 * ROW_HEIGHT));
    }
}
