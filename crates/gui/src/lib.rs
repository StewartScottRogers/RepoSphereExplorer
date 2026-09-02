//! Slint front end: renders state and sends intents to the service.

// Slint's generated component code (from build.rs, compiling ui/app.slint)
// carries no doc comments; scope the exception to this module rather than
// the whole crate.
#[allow(missing_docs)]
mod generated {
    slint::include_modules!();
}
pub use generated::MainWindow;

pub mod app;

use app::App;
use slint::{ModelRc, SharedString, VecModel};

/// Copies `app`'s current state into `ui`'s bound properties.
pub fn sync_ui(ui: &MainWindow, app: &App) {
    ui.set_folder_rows(string_model(app.folder_labels()));
    ui.set_folder_selected(row_index(app.folder_selected()));
    ui.set_content_rows(string_model(app.content_labels()));
    ui.set_content_selected(row_index(app.content_selected()));
    ui.set_file_text(app.file_text().into());
    ui.set_status_text(app.status_text().into());
    ui.set_focus_pane(app.focus_index());
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
