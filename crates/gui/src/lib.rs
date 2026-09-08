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

use app::App;
use slint::{ModelRc, SharedString, VecModel};

/// Copies `app`'s current state into `ui`'s bound properties.
pub fn sync_ui(ui: &MainWindow, app: &App) {
    ui.set_folder_rows(string_model(app.folder_labels()));
    ui.set_folder_selected(row_index(app.folder_selected()));
    ui.set_content_rows(ModelRc::new(VecModel::from(
        app.content_rows()
            .into_iter()
            .map(|row| ContentRow {
                glyph: row.glyph.into(),
                name: row.name.into(),
                size: row.size.into(),
                kind: row.kind.into(),
                modified: row.modified.into(),
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_content_selected(row_index(app.content_selected()));
    ui.set_file_text(app.file_text().into());
    ui.set_status_text(app.status_text().into());
    ui.set_focus_pane(app.focus_index());
    ui.set_content_is_archive(app.selected_is_archive());
    ui.set_content_prompt_text(app.prompt_text().into());
    ui.set_content_prompt_row(app.prompt_row());
    ui.set_content_prompt_editable(app.prompt_is_editable());
    ui.set_content_sort_column(app.sort_column());
    ui.set_content_sort_ascending(app.sort_ascending());
    ui.set_breadcrumbs(string_model(app.breadcrumbs()));
    ui.set_can_go_back(app.can_go_back());
    ui.set_can_go_forward(app.can_go_forward());
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
