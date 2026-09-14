//! Opens the editing surface on a file, so #479 can be driven before
//! #480 puts it in the File pane.
//!
//! ```text
//! cargo run -p gui --example code_editor -- samples/rust/src/main.rs
//! ```
//!
//! It goes when the File pane takes the surface over. Until then it is
//! the only way to press a key against it, and a widget nobody has typed
//! into is a widget nobody has tested.

use gui::document::Document;
use gui::{CodeEditorHarness, ColouredRun};
use plugin_api::{Class, PluginPresentation};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

/// `Class` as `Theme.syntax-colour` numbers it. The same table as
/// `sync_ui`'s, which is where it belongs once the pane owns this.
const fn class_number(class: Class) -> i32 {
    match class {
        Class::Plain => 0,
        Class::Keyword => 1,
        Class::Type => 2,
        Class::Function => 3,
        Class::Text => 4,
        Class::Number => 5,
        Class::Comment => 6,
        Class::Punctuation => 7,
    }
}

/// Which plugin opens `path`, asked the way the File pane asks it.
fn plugin_for(path: &str) -> Option<&'static dyn PluginPresentation> {
    let protocol::Response::FileView { plugin, .. } =
        service::view_file(std::path::Path::new(path)).ok()?
    else {
        return None;
    };
    gui::PRESENTATION_PLUGINS
        .iter()
        .copied()
        .find(|candidate| candidate.name() == plugin)
}

/// The document's lines, coloured by whichever plugin claims `path`.
fn lines(document: &Document, path: &str) -> Vec<ModelRc<ColouredRun>> {
    let text = document.text();
    let spans = plugin_for(path).map(|found| found.classify(text));
    let whole = [plugin_api::Span::new(0, text.len(), Class::Plain)];

    let mut rows: Vec<Vec<ColouredRun>> = vec![Vec::new()];
    for span in spans
        .as_deref()
        .filter(|spans| !spans.is_empty())
        .unwrap_or(&whole)
    {
        let Some(part) = text.get(span.start..span.start + span.len) else {
            continue;
        };
        let mut pieces = part.split('\n');
        if let Some(first) = pieces.next().filter(|piece| !piece.is_empty()) {
            rows.last_mut().unwrap().push(ColouredRun {
                text: SharedString::from(first),
                class: class_number(span.class),
            });
        }
        for piece in pieces {
            rows.push(Vec::new());
            if !piece.is_empty() {
                rows.last_mut().unwrap().push(ColouredRun {
                    text: SharedString::from(piece),
                    class: class_number(span.class),
                });
            }
        }
    }
    rows.into_iter()
        .map(|row| ModelRc::new(VecModel::from(row)))
        .collect()
}

/// The widest line, in columns, which is how far the surface can scroll.
fn longest(document: &Document) -> i32 {
    document
        .text()
        .lines()
        .map(|line| i32::try_from(line.chars().count()).unwrap_or(i32::MAX))
        .max()
        .unwrap_or(0)
}

fn main() -> Result<(), slint::PlatformError> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "samples/rust/src/main.rs".to_owned());
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        eprintln!("could not read {path}: {error}");
        std::process::exit(1);
    });

    let ui = CodeEditorHarness::new()?;
    let document = Rc::new(RefCell::new(Document::new(text)));

    let show: Rc<dyn Fn()> = Rc::new({
        let ui = ui.as_weak();
        let document = document.clone();
        let path = path.clone();
        move || {
            let Some(ui) = ui.upgrade() else {
                return;
            };
            let document = document.borrow();
            ui.set_lines(ModelRc::new(VecModel::from(lines(&document, &path))));
            ui.set_longest_line(longest(&document));
            let caret = document.caret();
            let line = document.line_of(caret);
            ui.set_caret_line(i32::try_from(line).unwrap_or(0));
            ui.set_caret_column(i32::try_from(document.column_of(caret)).unwrap_or(0));
            match document.selection() {
                Some(range) => {
                    let start = document.line_of(range.start);
                    let end = document.line_of(range.end);
                    ui.set_selection_start_line(i32::try_from(start).unwrap_or(0));
                    ui.set_selection_start_column(
                        i32::try_from(document.column_of(range.start)).unwrap_or(0),
                    );
                    ui.set_selection_end_line(i32::try_from(end).unwrap_or(0));
                    ui.set_selection_end_column(
                        i32::try_from(document.column_of(range.end)).unwrap_or(0),
                    );
                }
                None => ui.set_selection_start_line(-1),
            }
        }
    });
    show();

    {
        let document = document.clone();
        let ui_weak = ui.as_weak();
        let show = Rc::clone(&show);
        ui.on_key(move |text, shift, control| {
            let rows = ui_weak.upgrade().map_or(20, |ui| {
                usize::try_from(ui.get_visible_rows()).unwrap_or(20)
            });
            gui::editor::handle_key(&mut document.borrow_mut(), &text, shift, control, rows);
            show();
        });
    }
    {
        let document = document.clone();
        let show = Rc::clone(&show);
        ui.on_pressed(move |line, column| {
            gui::editor::handle_click(
                &mut document.borrow_mut(),
                usize::try_from(line).unwrap_or(0),
                usize::try_from(column).unwrap_or(0),
                false,
            );
            show();
        });
    }

    ui.run()
}
