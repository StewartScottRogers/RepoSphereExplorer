//! Slint front end: renders state and sends intents to the service.

// Slint's generated component code (from build.rs, compiling ui/app.slint)
// carries no doc comments; scope the exception to this module rather than
// the whole crate.
#[allow(missing_docs)]
mod generated {
    slint::include_modules!();
}
pub use generated::{
    CodeEditorHarness, ColouredRun, ContentRow, FactRow, FolderRow, MainWindow, Theme,
};

pub mod app;
pub mod renderer;
pub mod settings;

pub mod document;
pub mod editor;

pub use app::PRESENTATION_PLUGINS;

use app::App;
use plugin_api::{Class, Graphic, Icon};
use slint::ComponentHandle as _;
use slint::{Image, ModelRc, SharedPixelBuffer, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

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

/// The rows of a pane on screen - every row any part of which shows -
/// given where it is scrolled, how tall it is and how many rows it holds.
///
/// Empty before the pane has been laid out, since nothing is on screen yet.
#[must_use]
pub fn visible_rows(
    scroll_y: f32,
    viewport_height: f32,
    row_count: usize,
) -> std::ops::Range<usize> {
    if viewport_height <= 0.0 {
        return 0..0;
    }
    let top = -scroll_y;
    let bottom = top + viewport_height;
    // `u16`, as in `scroll_offset_for`: it converts exactly, and nobody
    // scrolls a listing longer than that.
    let count = u16::try_from(row_count).unwrap_or(u16::MAX);
    let first = (0..count)
        .find(|&index| (f32::from(index) + 1.0) * ROW_HEIGHT > top)
        .unwrap_or(count);
    let end = (first..count)
        .find(|&index| f32::from(index) * ROW_HEIGHT >= bottom)
        .unwrap_or(count);
    usize::from(first)..usize::from(end)
}

/// Narrowest the File pane may be (#577). Below this, a working copy's
/// facts (#576) wrap across several lines instead of sitting on one.
pub const MIN_FILE_PANE_WIDTH: f32 = 280.0;
/// Narrowest the Folders pane may be, matching `app.slint`'s splitter clamp
/// and `settings::MIN_WIDTH`.
pub const MIN_FOLDERS_WIDTH: f32 = 120.0;
/// Narrowest the Contents pane may be, matching `app.slint`'s splitter
/// clamp.
pub const MIN_CONTENTS_WIDTH: f32 = 200.0;
/// The two splitters between the three panes, `app.slint`'s 5px each -
/// width no pane ever gets to claim.
const SPLITTERS_WIDTH: f32 = 10.0;

/// Shrinks `folders` and `contents` just enough that the File pane - what
/// `window_width` leaves once they and the splitters are taken out - keeps
/// its minimum. Contents gives way first, down to its own minimum, then
/// Folders; neither is ever left narrower than its floor, however small
/// `window_width` is, so the File pane simply gets whatever is left.
///
/// This lives here rather than in a `changed root.width` handler in
/// `app.slint`, for the reason `scroll_offset_for` does: such a handler
/// only runs inside an event loop, so nothing could test it.
#[must_use]
pub fn fit_pane_widths(window_width: f32, folders: f32, contents: f32) -> (f32, f32) {
    let folders = folders.max(MIN_FOLDERS_WIDTH);
    let contents = contents.max(MIN_CONTENTS_WIDTH);
    let available = window_width - SPLITTERS_WIDTH;
    let shortfall = folders + contents + MIN_FILE_PANE_WIDTH - available;
    if shortfall <= 0.0 {
        return (folders, contents);
    }
    let from_contents = shortfall.min(contents - MIN_CONTENTS_WIDTH).max(0.0);
    let contents = contents - from_contents;
    let remaining = shortfall - from_contents;
    let from_folders = remaining.min(folders - MIN_FOLDERS_WIDTH).max(0.0);
    let folders = folders - from_folders;
    (folders, contents)
}

/// Applies [`fit_pane_widths`] against `ui`'s actual current width, so
/// widths restored from `gui.json` or left over from a wider window are
/// corrected rather than trusted. `main` calls this on every tick - the
/// same timer that drives `sync_ui` - so a resize corrects them without
/// the reader doing anything.
pub fn fit_pane_widths_to_window(ui: &MainWindow) {
    let window = ui.window();
    let window_width = window.size().to_logical(window.scale_factor()).width;
    // Not yet laid out: nothing to correct against.
    if window_width <= 0.0 {
        return;
    }
    let (folders, contents) = fit_pane_widths(
        window_width,
        ui.get_folders_width(),
        ui.get_contents_width(),
    );
    ui.set_folders_width(folders);
    ui.set_contents_width(contents);
}

/// Asks for the working-tree status of the repository rows the Contents
/// pane has on screen. `main` calls this on every tick, after drawing, so
/// scrolling asks for the rows it brings into view.
pub fn ask_for_visible_statuses(ui: &MainWindow, app: &mut App) {
    app.ask_for_statuses(visible_rows(
        ui.get_content_scroll_y(),
        ui.get_content_viewport_height(),
        slint::Model::row_count(&ui.get_content_rows()),
    ));
}

/// A [`Class`] as the number `Theme.syntax-colour` maps to a brush.
///
/// A number rather than a colour because the palette lives in
/// `app.slint`: the front end knows what a keyword should look like in
/// each scheme, and Rust knows which runs are keywords. Neither has to
/// learn the other's half.
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

/// The editor's half of [`sync_ui`], which is most of what it does while
/// a file is open and none of what it does otherwise.
fn sync_editor(ui: &MainWindow, app: &App) {
    ui.set_editing_in_colour(app.editing_in_colour());
    ui.set_edit_modified(app.edit_modified());
    ui.set_edit_can_undo(app.edit_can_undo());
    ui.set_edit_can_redo(app.edit_can_redo());
    ui.set_edit_has_selection(app.edit_has_selection());
    let (line, column) = app.edit_position();
    ui.set_edit_line(row_index(line));
    ui.set_edit_column(row_index(column));
    if !app.editing_file() {
        return;
    }
    // Only while the editor is open: writing this back every sync would
    // fight the cursor as the user types.
    let text = app.edit_text();
    if ui.get_edit_text() != text.as_str() {
        ui.set_edit_text(text.into());
    }
    ui.set_edit_lines(ModelRc::new(VecModel::from(
        app.edit_lines()
            .into_iter()
            .map(|line| {
                ModelRc::new(VecModel::from(
                    line.into_iter()
                        .map(|run| ColouredRun {
                            text: run.text.into(),
                            class: class_number(run.class),
                        })
                        .collect::<Vec<_>>(),
                ))
            })
            .collect::<Vec<_>>(),
    )));
    let (line, column) = app.edit_caret();
    ui.set_edit_caret_line(row_index(line));
    ui.set_edit_caret_column(row_index(column));
    ui.set_edit_longest_line(row_index(app.edit_longest_line()));
    match app.edit_selection() {
        Some(((start_line, start_column), (end_line, end_column))) => {
            ui.set_edit_selection_start_line(row_index(start_line));
            ui.set_edit_selection_start_column(row_index(start_column));
            ui.set_edit_selection_end_line(row_index(end_line));
            ui.set_edit_selection_end_column(row_index(end_column));
        }
        None => ui.set_edit_selection_start_line(-1),
    }
}

/// The machine's clipboard, for [`wire_editor`].
///
/// Slint 1.17.1 keeps its own on the `Platform` trait where an
/// application cannot reach it, so this goes through `copypasta` -
/// which Slint's own windowing backend already depends on.
///
/// Every call can fail, and every failure is the same thing to a
/// reader: the clipboard did not work this time. A failed copy leaves
/// it as it was and a failed paste inserts nothing.
#[derive(Default)]
struct SystemClipboard {
    context: Option<copypasta::ClipboardContext>,
}

impl SystemClipboard {
    /// Opened on first use: a window that never edits anything should
    /// not hold a platform resource.
    fn context(&mut self) -> Option<&mut copypasta::ClipboardContext> {
        if self.context.is_none() {
            self.context = copypasta::ClipboardContext::new().ok();
        }
        self.context.as_mut()
    }
}

impl editor::Clipboard for SystemClipboard {
    fn read(&mut self) -> Option<String> {
        use copypasta::ClipboardProvider as _;
        self.context()?.get_contents().ok()
    }

    fn write(&mut self, text: &str) {
        use copypasta::ClipboardProvider as _;
        if let Some(context) = self.context() {
            let _ = context.set_contents(text.to_owned());
        }
    }
}

/// A clipboard for the editor's callbacks to hold.
fn clipboard() -> SystemClipboard {
    SystemClipboard::default()
}

/// The application id this process gives its window on a free desktop: the
/// `WM_CLASS` an X11 desktop environment reads off the window, and the
/// `app_id` a Wayland one does.
///
/// It is the name of the desktop entry the Linux install writes
/// (`reposphereexplorer.desktop`) and of the icon that entry names, because
/// that is how a desktop environment gets from an open window back to the
/// entry - and so to the icon it shows in the taskbar and in Alt+Tab, and to
/// one button for the application rather than one for every window.
/// `icon::THEMED_NAME` is the same string; `tests/desktop_entry.rs` holds
/// the three together.
pub const XDG_APP_ID: &str = "reposphereexplorer";

/// Gives the window [`XDG_APP_ID`] to carry.
///
/// In the library rather than in `main` for the reason [`wire_callbacks`]
/// is: a window that names itself and an entry that matches the name are
/// two halves, and only a test that runs this wiring can say they meet.
///
/// Slint keeps the id on the platform rather than on the window and reads
/// it when the window is realised, so this belongs after
/// `MainWindow::new`, which is what sets a platform up, and before the
/// window is shown. Away from X11 and Wayland it does nothing at all.
///
/// # Errors
///
/// When no windowing platform has been set up to carry it.
pub fn name_the_window() -> Result<(), slint::PlatformError> {
    slint::set_xdg_app_id(XDG_APP_ID)
}

/// Wires every callback the window has to the application behind it.
///
/// In the library rather than in `main` so that a test can wire a real
/// window to a real `App` exactly as the application does. Two suites
/// carried hand copies of these three before this moved, and a copy
/// proves nothing about what a reader gets: a callback hooked to the
/// wrong method, or not hooked at all, was invisible to every test in
/// the crate. See CLAUDE.md rule 14.
pub fn wire_callbacks(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    wire_rows(ui, app);
    wire_commands(ui, app);
    wire_content_operations(ui, app);
    wire_editor(ui, app);
}

/// Wires the callbacks that carry a row index or a signed delta.
fn wire_rows(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    let index = |i: i32| usize::try_from(i).unwrap_or(usize::MAX);

    macro_rules! on_row_event {
        ($setter:ident, $method:ident) => {{
            let app = app.clone();
            let ui_weak = ui.as_weak();
            ui.$setter(move |i| {
                let mut app = app.borrow_mut();
                app.$method(index(i));
                if let Some(ui) = ui_weak.upgrade() {
                    sync_ui(&ui, &app);
                }
            });
        }};
    }

    // Not `on_row_event!`: this one carries where in the row the click
    // landed, because the chevron opens the folder and the name selects it.
    {
        let app = app.clone();
        let ui_weak = ui.as_weak();
        ui.on_folder_row_clicked(move |i, x| {
            let mut app = app.borrow_mut();
            app.click_folder(index(i), x);
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
    on_row_event!(on_folder_row_double_clicked, toggle_folder);
    on_row_event!(on_content_row_clicked, select_content);
    on_row_event!(on_content_row_ctrl_clicked, toggle_content);
    on_row_event!(on_content_row_shift_clicked, extend_selection_to);

    {
        // A marquee reports the first and last row it covered.
        let app = app.clone();
        let ui_weak = ui.as_weak();
        ui.on_content_rows_marqueed(move |from, to| {
            let mut app = app.borrow_mut();
            app.select_range(index(from), index(to));
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
    on_row_event!(on_content_row_double_clicked, open_content);
    on_row_event!(on_file_view_selected, select_file_view);
    on_row_event!(on_file_tab_selected, select_file_tab);

    macro_rules! on_delta_event {
        ($setter:ident, $method:ident) => {{
            // `on_row_event!` converts its argument to a row index; these
            // carry a signed delta instead.
            let app = app.clone();
            let ui_weak = ui.as_weak();
            ui.$setter(move |delta| {
                let mut app = app.borrow_mut();
                app.$method(delta);
                if let Some(ui) = ui_weak.upgrade() {
                    sync_ui(&ui, &app);
                }
            });
        }};
    }

    on_delta_event!(on_selection_moved, move_selection);
    on_delta_event!(on_selection_extended, extend_selection_by);
    on_delta_event!(on_pane_cycled, cycle_focus);
    on_delta_event!(on_content_sort_requested, sort_by_column);
    on_delta_event!(on_breadcrumb_requested, navigate_to_breadcrumb);

    {
        // `edge-requested` carries 0 for Home and 1 for End.
        let app = app.clone();
        let ui_weak = ui.as_weak();
        ui.on_edge_requested(move |last| {
            let mut app = app.borrow_mut();
            app.select_edge(last != 0);
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
    {
        let app = Rc::clone(app);
        let ui_weak = ui.as_weak();
        ui.on_edge_extended(move |last| {
            let mut app = app.borrow_mut();
            app.extend_selection_to_edge(last != 0);
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
}

/// Wires the menu-bar, command-bar and keyboard commands that take no
/// argument.
fn wire_commands(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    macro_rules! on_event {
        ($setter:ident, $method:ident) => {{
            let app = app.clone();
            let ui_weak = ui.as_weak();
            ui.$setter(move || {
                let mut app = app.borrow_mut();
                app.$method();
                if let Some(ui) = ui_weak.upgrade() {
                    sync_ui(&ui, &app);
                }
            });
        }};
    }

    on_event!(on_cancel_requested, cancel_pending);
    on_event!(on_delete_requested, request_delete);
    on_event!(on_return_pressed, handle_return);
    on_event!(on_backspace_pressed, backspace);
    on_event!(on_parent_requested, navigate_to_parent);
    on_event!(on_back_requested, go_back);
    on_event!(on_forward_requested, go_forward);
    on_event!(on_find_requested, begin_find);
    on_event!(on_clipboard_copy_requested, copy_to_clipboard);
    on_event!(on_clipboard_cut_requested, cut_to_clipboard);
    {
        let app = app.clone();
        let ui_weak = ui.as_weak();
        ui.on_open_web_requested(move || {
            let mut app = app.borrow_mut();
            // Detached, so the browser outlives nothing it should not and a
            // slow start does not hold the window.
            app.open_on_the_web(|address| open::that_detached(address));
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
    {
        // Paste needs the system clipboard for the prompts, so it cannot be
        // one of the plain events above.
        let app = app.clone();
        let ui_weak = ui.as_weak();
        let mut clipboard = clipboard();
        ui.on_clipboard_paste_requested(move || {
            let mut app = app.borrow_mut();
            app.paste(&mut clipboard);
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
    on_event!(on_refresh_requested, refresh);
    on_event!(on_path_edit_requested, begin_path_edit);
    on_event!(on_edit_requested, begin_file_edit);

    {
        // Save takes the editor's current text from the UI first: the user
        // has been typing into it, not into `App`.
        let app = app.clone();
        let ui_weak = ui.as_weak();
        ui.on_save_requested(move || {
            let mut app = app.borrow_mut();
            if let Some(ui) = ui_weak.upgrade() {
                // Only the plain box holds text the application has not
                // seen; the coloured surface reports every keystroke as
                // it happens, and `set_edit_text` ignores it for that
                // reason. Asking anyway keeps this one call site simple.
                app.set_edit_text(&ui.get_edit_text());
            }
            app.save_file_edit();
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
    on_event!(on_select_all_requested, select_all);
    on_event!(on_undo_requested, undo);

    ui.on_quit_requested(|| {
        // The menu's File > Exit; the window's own close button goes through
        // Slint rather than here.
        let _ = slint::quit_event_loop();
    });

    {
        let app = app.clone();
        let ui_weak = ui.as_weak();
        ui.on_about_requested(move || {
            let mut app = app.borrow_mut();
            app.report(concat!(
                "Repos Explorer ",
                env!("CARGO_PKG_VERSION"),
                " - a front door to your development workspace"
            ));
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
    {
        // File > Repos Directory...: the same prompt a first run shows,
        // seeded with wherever the application is opening today.
        let app = app.clone();
        let ui_weak = ui.as_weak();
        ui.on_repos_root_requested(move || {
            let mut app = app.borrow_mut();
            let current = app::opening().root.to_string_lossy().into_owned();
            app.begin_repos_root_edit(&current);
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
    on_event!(on_new_folder_requested, request_new_folder);
    on_event!(on_new_file_requested, request_new_file);
}

/// Wires the operations that act on the selected contents row.
fn wire_content_operations(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    macro_rules! on_event {
        ($setter:ident, $method:ident) => {{
            let app = app.clone();
            let ui_weak = ui.as_weak();
            ui.$setter(move || {
                let mut app = app.borrow_mut();
                app.$method();
                if let Some(ui) = ui_weak.upgrade() {
                    sync_ui(&ui, &app);
                }
            });
        }};
    }

    on_event!(on_content_rename_requested, request_rename);
    on_event!(on_content_copy_requested, request_copy);
    on_event!(on_content_delete_requested, request_delete);
    on_event!(on_content_extract_requested, request_extract);

    let open_app = app.clone();
    let open_ui = ui.as_weak();
    ui.on_content_open_requested(move || {
        let mut app = open_app.borrow_mut();
        let selected = app.content_selected();
        app.open_content(selected);
        if let Some(ui) = open_ui.upgrade() {
            sync_ui(&ui, &app);
        }
    });

    let text_app = app.clone();
    let text_ui = ui.as_weak();
    ui.on_key_text(move |text| {
        let mut app = text_app.borrow_mut();
        app.handle_key_text(&text);
        if let Some(ui) = text_ui.upgrade() {
            sync_ui(&ui, &app);
        }
    });
}

/// The editing surface's own callbacks: a keystroke, a click, and the
/// commands in the pane's row.
///
/// In the library rather than in `main` so that a test can wire a real
/// window to a real `App` exactly as the application does. Every test
/// before this one held one half or the other, and the half nobody had
/// was the half that decides whether typing lands where the caret is.
///
/// Apart from `main` because it is the only part of the wiring that
/// holds something of its own - the clipboard - and because `main` was
/// already at the length the lints allow.
pub fn wire_editor(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    {
        let app = Rc::clone(app);
        let ui_weak = ui.as_weak();
        let mut clipboard = clipboard();
        ui.on_edit_key(move |text, shift, control| {
            let Some(ui) = ui_weak.upgrade() else {
                return false;
            };
            // A page is what the pane is showing, not a number chosen
            // here: at least one row, so a pane too short to show any
            // still moves.
            let rows = usize::try_from(ui.get_edit_visible_rows())
                .unwrap_or(20)
                .max(1);
            let mut app = app.borrow_mut();
            // The answer is what the markup uses to decide whether the
            // key stops here or carries on to the window behind.
            let used = app.edit_key(&mut clipboard, &text, shift, control, rows);
            sync_ui(&ui, &app);
            used
        });
    }
    macro_rules! on_edit_command {
        ($setter:ident, $command:ident) => {{
            let app = Rc::clone(app);
            let ui_weak = ui.as_weak();
            let mut clipboard = clipboard();
            ui.$setter(move || {
                let mut app = app.borrow_mut();
                app.edit_command(app::EditCommand::$command, &mut clipboard);
                if let Some(ui) = ui_weak.upgrade() {
                    sync_ui(&ui, &app);
                }
            });
        }};
    }
    on_edit_command!(on_edit_undo_requested, Undo);
    on_edit_command!(on_edit_redo_requested, Redo);
    on_edit_command!(on_edit_cut_requested, Cut);
    on_edit_command!(on_edit_copy_requested, Copy);
    on_edit_command!(on_edit_paste_requested, Paste);

    {
        let app = Rc::clone(app);
        let ui_weak = ui.as_weak();
        ui.on_edit_pressed(move |line, column| {
            let mut app = app.borrow_mut();
            app.edit_click(
                usize::try_from(line).unwrap_or(0),
                usize::try_from(column).unwrap_or(0),
                false,
            );
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
}

/// Copies `app`'s current state into `ui`'s bound properties.
pub fn sync_ui(ui: &MainWindow, app: &App) {
    ui.set_folder_rows(ModelRc::new(VecModel::from(
        app.folder_rows()
            .into_iter()
            .map(|row| FolderRow {
                icon: icon_image(row.icon, true),
                name: row.name.into(),
                depth: row_index(row.depth),
                expandable: row.expandable,
                expanded: row.expanded,
            })
            .collect::<Vec<_>>(),
    )));
    let folder_moved = ui.get_folder_selected() != row_index(app.folder_selected());
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
                branch: row.branch.into(),
                marker: row.marker.into(),
                marker_tooltip: row.marker_tooltip.into(),
                marker_warning: row.marker_warning,
                selected: app.is_selected(index),
            })
            .collect::<Vec<_>>(),
    )));
    // Whatever moved the selection - a click, type-ahead, an arrow key,
    // Home or End, or the reselect after an operation - it lands here, so
    // one adjustment per render covers every one of them.
    //
    // Only when it actually moved, though. This runs on a timer whether
    // anything happened or not, and an unconditional write meant a reader
    // who wheeled the listing past the selected row had it yanked back
    // within a tenth of a second: the pane could not be scrolled at all.
    // What the window is already showing is the record of what was last
    // drawn, so comparing against it needs no state of its own.
    let content_moved = ui.get_content_selected() != row_index(app.content_selected());
    ui.set_content_selected(row_index(app.content_selected()));
    if content_moved {
        ui.set_content_scroll_y(scroll_offset_for(
            app.content_selected(),
            ui.get_content_viewport_height(),
            ui.get_content_scroll_y(),
        ));
    }
    if folder_moved {
        ui.set_folders_scroll_y(scroll_offset_for(
            app.folder_selected(),
            ui.get_folders_viewport_height(),
            ui.get_folders_scroll_y(),
        ));
    }
    let graphic = app.file_graphic().as_ref().and_then(graphic_image);
    ui.set_file_has_graphic(graphic.is_some());
    ui.set_file_graphic(graphic.unwrap_or_default());
    ui.set_file_views(string_model(
        app.file_views().into_iter().map(str::to_owned).collect(),
    ));
    ui.set_file_view_index(row_index(app.file_view_index()));
    ui.set_file_tabs(string_model(app.file_tabs()));
    ui.set_file_tab_index(row_index(app.file_tab_index()));
    ui.set_file_lines(ModelRc::new(VecModel::from(
        app.file_lines()
            .into_iter()
            .map(|line| {
                ModelRc::new(VecModel::from(
                    line.into_iter()
                        .map(|run| ColouredRun {
                            text: run.text.into(),
                            class: class_number(run.class),
                        })
                        .collect::<Vec<_>>(),
                ))
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_file_text(app.file_text().into());
    ui.set_file_facts(ModelRc::new(VecModel::from(fact_rows(app))));
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
    ui.set_can_go_up(app.can_go_up());
    ui.set_address_path(app.address_path().into());
    ui.set_path_input(app.path_input().into());
    ui.set_editing_path(app.editing_path());
    ui.set_showing_found(app.showing_found());
    ui.set_content_size_column_visible(app.content_size_column_visible());
    ui.set_editing_file(app.editing_file());
    ui.set_can_edit(app.can_edit());
    ui.set_can_open(app.can_open());
    ui.set_web_provider(app.web_provider().unwrap_or_default().into());
    sync_editor(ui, app);
    ui.set_location_icon(icon_image(app::icon_for("", true), true));
}

/// The File pane's fact table rows, converted from `app`'s own
/// [`app::FactRow`] to the Slint-generated struct `sync_ui` binds.
fn fact_rows(app: &App) -> Vec<FactRow> {
    app.file_facts()
        .into_iter()
        .map(|fact| FactRow {
            label: fact.label.into(),
            display_value: fact.display_value.into(),
            full_value: fact.full_value.into(),
            dim: fact.dim,
        })
        .collect()
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
    use super::{
        MIN_CONTENTS_WIDTH, MIN_FOLDERS_WIDTH, ROW_HEIGHT, fit_pane_widths, scroll_offset_for,
        visible_rows,
    };

    #[test]
    fn the_rows_on_screen_are_the_ones_any_part_of_which_shows() {
        let viewport = 5.0 * ROW_HEIGHT;
        assert_eq!(visible_rows(0.0, viewport, 100), 0..5);
        assert_eq!(visible_rows(0.0, viewport, 3), 0..3, "a short listing");
        assert_eq!(
            visible_rows(-ROW_HEIGHT / 2.0, viewport, 100),
            0..6,
            "half a row scrolled off the top, half of another on at the bottom"
        );
        assert_eq!(visible_rows(-40.0 * ROW_HEIGHT, viewport, 100), 40..45);
        assert_eq!(visible_rows(0.0, 0.0, 100), 0..0, "not laid out yet");
    }

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

    #[test]
    fn widths_that_already_fit_are_left_unchanged() {
        // 240 + 470 + the File pane's 280px minimum, plus the two 5px
        // splitters, is exactly 1000: nothing has to give.
        assert_eq!(fit_pane_widths(1000.0, 240.0, 470.0), (240.0, 470.0));
    }

    #[test]
    fn a_smaller_window_shrinks_contents_before_folders() {
        // The window above, narrowed to 750px. Contents alone has enough
        // headroom above its 200px floor to absorb the shortfall, so
        // Folders is untouched.
        assert_eq!(fit_pane_widths(750.0, 240.0, 470.0), (240.0, 220.0));

        // Narrower still: Contents is already at its floor, so Folders
        // gives up the 120px Contents could not.
        assert_eq!(
            fit_pane_widths(600.0, 240.0, 470.0),
            (MIN_FOLDERS_WIDTH, MIN_CONTENTS_WIDTH)
        );
    }

    #[test]
    fn a_window_too_small_for_every_minimum_never_goes_negative() {
        let (folders, contents) = fit_pane_widths(50.0, 240.0, 470.0);

        assert_eq!(
            (folders, contents),
            (MIN_FOLDERS_WIDTH, MIN_CONTENTS_WIDTH),
            "both panes settle on their floor rather than going negative"
        );
    }
}
