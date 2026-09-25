//! Slint front end: renders state and sends intents to the service.

// Slint's generated component code (from build.rs, compiling ui/app.slint)
// carries no doc comments; scope the exception to this module rather than
// the whole crate.
#[allow(missing_docs)]
mod generated {
    slint::include_modules!();
}
pub use generated::{
    CertificateRow, CodeEditorHarness, ColouredRun, ContentRow, FactRow, FolderRow, MainWindow,
    PaneMenuRow, ShortcutRow, SwitcherRow, Theme, Zoom,
};

pub mod app;
pub mod launch;
pub mod renderer;
pub mod settings;
pub mod shortcuts;
pub mod switcher;
pub mod zoom;

pub mod document;
pub mod editor;
pub mod tools;

pub use app::PRESENTATION_PLUGINS;

use app::{App, Pane, RepositoryMark};
use plugin_api::{Class, Graphic, Icon};
use slint::ComponentHandle as _;
use slint::{Image, ModelRc, SharedPixelBuffer, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// [`ICON_CACHE`]'s key: a plugin's label and tint, and the branch mark (if
/// any) drawn on top of them.
type IconCacheKey = (&'static str, u32, Option<RepositoryMark>);

thread_local! {
    /// Rendered icons, keyed by [`IconCacheKey`]. A folder of a thousand
    /// files holds a handful of distinct types, so this turns per-row
    /// rasterisation into per-type.
    static ICON_CACHE: RefCell<HashMap<IconCacheKey, Image>> =
        RefCell::new(HashMap::new());
}

/// The small provider badge drawn in a working copy folder icon's
/// bottom-right corner (#579): a filled circle in the provider's brand
/// colour, carrying a plain branch glyph rather than the provider's
/// trademarked logo. The glyph is the same shape for every provider, so
/// the badge reads as "working copy" by shape alone - the fact GUIDANCE.md
/// §2.4 asks for - even to a reader who cannot tell the fill colour from a
/// plain folder's tint; the colour then names *which* provider, for a
/// reader who can.
fn repository_mark_badge(mark: RepositoryMark) -> String {
    let tint = match mark {
        RepositoryMark::GitHub => "#24292f",
        RepositoryMark::GitLab => "#e24329",
        RepositoryMark::Bitbucket => "#0052cc",
        RepositoryMark::AzureDevOps => "#0078d4",
        RepositoryMark::Generic => "#57606a",
    };
    format!(
        "<circle cx='24' cy='23' r='7.5' fill='{tint}' stroke='#ffffff' stroke-width='1.2'/>         <line x1='21' y1='19' x2='21' y2='27' stroke='#ffffff' stroke-width='1.3' stroke-linecap='round'/>         <path d='M21 23c3 0 4.5-1.5 4.5-3' fill='none' stroke='#ffffff' stroke-width='1.3' stroke-linecap='round'/>         <circle cx='21' cy='19' r='1.3' fill='#ffffff'/>         <circle cx='21' cy='27' r='1.3' fill='#ffffff'/>         <circle cx='25.5' cy='19.4' r='1.3' fill='#ffffff'/>"
    )
}

/// Draws `icon` as a document sheet with a folded corner and a coloured
/// band carrying the type's label, or as a folder for the directory plugin.
/// The plugin owns the label and the colour (GUIDANCE.md §3); the shape is
/// shared, so a listing reads as one set rather than eighty-one drawings.
/// `mark`, only ever set on a folder, adds the working-copy badge (#579).
fn icon_svg(icon: Icon, folder: bool, mark: Option<RepositoryMark>) -> String {
    let (r, g, b) = (
        (icon.tint >> 16) & 0xff,
        (icon.tint >> 8) & 0xff,
        icon.tint & 0xff,
    );
    let tint = format!("#{r:02x}{g:02x}{b:02x}");
    let badge = mark.map(repository_mark_badge).unwrap_or_default();
    if folder {
        return format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32' viewBox='0 0 32 32'>             <path d='M2 7a2 2 0 0 1 2-2h8l3 3h11a2 2 0 0 1 2 2v15a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2z'              fill='{tint}'/>             <path d='M2 12h28v13a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2z' fill='{tint}'              fill-opacity='0.75'/>{badge}</svg>"
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
        "<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32' viewBox='0 0 32 32'>         <path d='M6 2h13l7 7v21a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1z'          fill='#ffffff' stroke='#9ca3af' stroke-width='1.2'/>         <path d='M19 2l7 7h-7z' fill='#d1d5db'/>{band}{badge}</svg>"
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
fn icon_image(icon: Icon, folder: bool, mark: Option<RepositoryMark>) -> Image {
    ICON_CACHE.with_borrow_mut(|cache| {
        cache
            .entry((icon.label, icon.tint, mark))
            .or_insert_with(|| {
                let svg = icon_svg(icon, folder, mark);
                Image::load_from_svg_data(svg.as_bytes()).unwrap_or_default()
            })
            .clone()
    })
}

/// Row height in `app.slint`'s panes. The scroll arithmetic below has to
/// agree with what is drawn, and a listing draws row `i` at `i * ROW_HEIGHT`.
const ROW_HEIGHT: f32 = 20.0;

/// Where a pane should be scrolled to so that row `selected` is fully
/// visible, given how much of the listing is on screen, where it is
/// scrolled now, and how tall a row is - `ROW_HEIGHT` scaled by the
/// window's zoom (#586), so this agrees with what is actually drawn at
/// every step rather than only at 100%.
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
pub fn scroll_offset_for(
    selected: usize,
    viewport_height: f32,
    current: f32,
    row_height: f32,
) -> f32 {
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
        return -(f32::from(u16::MAX) * row_height);
    };
    let top = f32::from(index) * row_height;
    let bottom = top + row_height;
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
/// given where it is scrolled, how tall it is, how many rows it holds, and
/// how tall a row is - the same zoom-scaled `row_height` [`scroll_offset_for`]
/// takes, and for the same reason: at zoom below 100% a row is shorter than
/// `ROW_HEIGHT`, so more of them fit the same viewport, and this has to ask
/// for that many or the last few would never be sent to the pane at all.
///
/// Empty before the pane has been laid out, since nothing is on screen yet.
#[must_use]
pub fn visible_rows(
    scroll_y: f32,
    viewport_height: f32,
    row_count: usize,
    row_height: f32,
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
        .find(|&index| (f32::from(index) + 1.0) * row_height > top)
        .unwrap_or(count);
    let end = (first..count)
        .find(|&index| f32::from(index) * row_height >= bottom)
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

/// `ui`'s current position and size, in logical pixels, as a
/// [`settings::WindowGeometry`] with `maximized: false` - or `None` while it
/// is maximised, since a maximised window's actual bounds are the display's
/// full work area, not the bounds a reader would want back on
/// un-maximising.
///
/// `main` calls this on every tick a window is not maximised, so the last
/// normal bounds are always in hand to save even if the window closes
/// maximised (#583's "un-maximising it returns to the last normal size and
/// position").
#[must_use]
pub fn normal_window_geometry(ui: &MainWindow) -> Option<settings::WindowGeometry> {
    let window = ui.window();
    if window.is_maximized() {
        return None;
    }
    let scale = window.scale_factor();
    let position = window.position().to_logical(scale);
    let size = window.size().to_logical(scale);
    Some(settings::WindowGeometry {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
        maximized: false,
    })
}

/// Applies `remembered` to `ui` and takes charge of its geometry from then
/// on: the correction against the displays actually connected, which needs
/// the event loop running, is scheduled for the moment it starts.
///
/// The wiring lives here rather than in `main` so a window test drives the
/// same code the application does (rule 14). `main` calls this before
/// showing the window, then [`observe_window_geometry`] on every tick and
/// [`geometry_to_save`] on the way out.
#[must_use]
pub fn wire_window_geometry(
    ui: &MainWindow,
    remembered: Option<settings::WindowGeometry>,
) -> Rc<RefCell<GeometryTracker>> {
    if let Some(geometry) = remembered {
        let window = ui.window();
        window.set_position(slint::LogicalPosition::new(geometry.x, geometry.y));
        window.set_size(slint::LogicalSize::new(geometry.width, geometry.height));
        if geometry.maximized {
            window.set_maximized(true);
        }
    }
    let tracker = Rc::new(RefCell::new(GeometryTracker::opening_at(remembered)));
    if let Some(geometry) = remembered {
        let settle_ui = ui.as_weak();
        let settle_tracker = tracker.clone();
        // Zero delay: as soon as the event loop is running, which is when
        // the connected displays can be asked for at all.
        slint::Timer::single_shot(std::time::Duration::ZERO, move || {
            if let Some(ui) = settle_ui.upgrade()
                && let Some(resolved) = settle_remembered_geometry(&ui, geometry)
            {
                settle_tracker.borrow_mut().corrected_to(resolved);
            }
        });
    }
    tracker
}

/// Keeps `tracker` up to date with the window, and puts the window back
/// onto a connected display the moment it stops being maximised - which is
/// when the platform has just restored bounds this application did not
/// choose. `main` calls this on every tick.
pub fn observe_window_geometry(ui: &MainWindow, tracker: &Rc<RefCell<GeometryTracker>>) {
    let just_restored = tracker
        .borrow_mut()
        .observed(normal_window_geometry(ui), ui.window().is_maximized());
    if just_restored && let Some(resolved) = settle_window_onto_a_display(ui) {
        tracker.borrow_mut().corrected_to(resolved);
    }
}

/// The geometry to write on the way out: the window's own bounds when it is
/// not maximised, otherwise the last bounds it had while it was not, with
/// `maximized` as the window is now.
#[must_use]
pub fn geometry_to_save(
    ui: &MainWindow,
    tracker: &Rc<RefCell<GeometryTracker>>,
) -> Option<settings::WindowGeometry> {
    let maximized = ui.window().is_maximized();
    normal_window_geometry(ui)
        .map(|geometry| settings::WindowGeometry {
            maximized,
            ..geometry
        })
        .or_else(|| tracker.borrow().closing_at(maximized))
}

/// What the window's geometry needs as the window is used: which bounds to
/// save, and when to put an off-display window back onto a display.
///
/// A window opened maximised carries its *last normal* bounds, which is
/// what un-maximising returns to - and those can name a display that has
/// since been unplugged. Correcting only the window that opens un-maximised
/// leaves the maximised one to un-maximise off-screen later, and then to
/// save those same bounds again, so it never heals (the review of #624).
///
/// Pure, so the rules can be tested without a desktop: `main` reads the
/// window and acts on what this returns.
#[derive(Debug, Default, Clone, Copy)]
pub struct GeometryTracker {
    last_normal: Option<settings::WindowGeometry>,
    was_maximized: bool,
}

impl GeometryTracker {
    /// A tracker for a window opening at `remembered`.
    #[must_use]
    pub fn opening_at(remembered: Option<settings::WindowGeometry>) -> Self {
        Self {
            last_normal: remembered.map(|geometry| settings::WindowGeometry {
                maximized: false,
                ..geometry
            }),
            was_maximized: remembered.is_some_and(|geometry| geometry.maximized),
        }
    }

    /// The remembered bounds, corrected against the displays that are
    /// actually connected. Held whether or not the window is maximised, so
    /// a window that opens maximised still un-maximises - and still saves -
    /// onto a display that exists.
    pub fn corrected_to(&mut self, resolved: settings::WindowGeometry) {
        self.last_normal = Some(settings::WindowGeometry {
            maximized: false,
            ..resolved
        });
    }

    /// Called every tick with the window's current normal bounds (`None`
    /// while it is maximised) and whether it is maximised now.
    ///
    /// Returns `true` the moment the window stops being maximised, which is
    /// when the platform has just restored bounds this application did not
    /// choose and which may be off every display.
    pub fn observed(&mut self, normal: Option<settings::WindowGeometry>, maximized: bool) -> bool {
        let just_restored = self.was_maximized && !maximized;
        self.was_maximized = maximized;
        if let Some(geometry) = normal {
            self.last_normal = Some(geometry);
        }
        just_restored
    }

    /// The bounds to save: the last ones the window had while not
    /// maximised, with `maximized` as it is now.
    #[must_use]
    pub fn closing_at(&self, maximized: bool) -> Option<settings::WindowGeometry> {
        self.last_normal.map(|geometry| settings::WindowGeometry {
            maximized,
            ..geometry
        })
    }
}

/// The connected displays' bounds, in logical pixels, and which of them is
/// the primary one - or `None` when they cannot be found, whether because
/// this window is not backed by winit (Slint's own UI-testing backend, used
/// throughout `tests/`, never is) or because the platform reports none.
///
/// Slint's own cross-platform `Window` has no notion of a display: only the
/// winit window underneath it does.
fn connected_displays(
    window: &slint::Window,
) -> Option<(Vec<settings::DisplayBounds>, settings::DisplayBounds)> {
    use slint::winit_030::{WinitWindowAccessor as _, winit};

    let to_bounds = |monitor: &winit::monitor::MonitorHandle| {
        #[allow(clippy::cast_possible_truncation)]
        let scale = monitor.scale_factor() as f32;
        let position = monitor.position();
        let size = monitor.size();
        settings::DisplayBounds {
            #[allow(clippy::cast_precision_loss)]
            x: position.x as f32 / scale,
            #[allow(clippy::cast_precision_loss)]
            y: position.y as f32 / scale,
            #[allow(clippy::cast_precision_loss)]
            width: size.width as f32 / scale,
            #[allow(clippy::cast_precision_loss)]
            height: size.height as f32 / scale,
        }
    };

    window.with_winit_window(|winit_window| {
        let monitors: Vec<_> = winit_window.available_monitors().collect();
        let primary = winit_window
            .primary_monitor()
            .or_else(|| monitors.first().cloned())?;
        Some((
            monitors.iter().map(to_bounds).collect(),
            to_bounds(&primary),
        ))
    })?
}

/// Corrects `remembered` against the displays actually connected right now,
/// and applies it to `ui` if that moved it - the window has already opened
/// at `remembered` by the time this runs (main applies it before showing
/// the window), so there is nothing to do when it is still on a display.
///
/// Finding the connected displays needs winit's event loop to be running,
/// which is only true once `ui.run()` has started - so `main` calls this
/// from a timer fired the moment the loop starts, rather than before
/// showing the window as the rest of the remembered geometry is applied.
#[must_use]
pub fn settle_remembered_geometry(
    ui: &MainWindow,
    remembered: settings::WindowGeometry,
) -> Option<settings::WindowGeometry> {
    let (displays, primary) = connected_displays(ui.window())?;
    let resolved = settings::geometry_on_a_display(remembered, &displays, primary);
    // A maximised window fills a display it is already on; moving it now
    // would un-maximise it. Its corrected bounds are still worth having,
    // for un-maximising and for saving, so they are returned either way.
    if resolved != remembered && !ui.window().is_maximized() {
        ui.window()
            .set_position(slint::LogicalPosition::new(resolved.x, resolved.y));
        ui.window()
            .set_size(slint::LogicalSize::new(resolved.width, resolved.height));
    }
    Some(resolved)
}

/// Puts the window back onto a connected display if the platform has just
/// left it off every one - what un-maximising does when the bounds it
/// restores name a display that has been unplugged since.
#[must_use]
pub fn settle_window_onto_a_display(ui: &MainWindow) -> Option<settings::WindowGeometry> {
    let current = normal_window_geometry(ui)?;
    settle_remembered_geometry(ui, current)
}

/// Asks for the working-tree status of the repository rows the Contents
/// pane has on screen. `main` calls this on every tick, after drawing, so
/// scrolling asks for the rows it brings into view.
pub fn ask_for_visible_statuses(ui: &MainWindow, app: &mut App) {
    let row_height = ROW_HEIGHT * app.zoom_factor();
    app.ask_for_statuses(visible_rows(
        ui.get_content_scroll_y(),
        ui.get_content_viewport_height(),
        slint::Model::row_count(&ui.get_content_rows()),
        row_height,
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
    wire_zoom(ui, app);
    wire_shortcuts_sheet(ui);
    wire_related_repository_link(ui, app);
    wire_switcher(ui, app);
    wire_all_repositories(ui, app);
    wire_certificates(ui, app);
    wire_tools(ui, app);
}

/// The application's open windows (#617): the main one, one more for every
/// pane currently popped out of it, and one more again for every tool
/// window ever pinned (#619) - which keeps its slot here for the rest of
/// its life, whether it is pinned right now or has gone back to following
/// the shared selection. All of them are wired against the same
/// `Rc<RefCell<App>>`, and driven by the same 100ms timer, so a change made
/// in any one of them reaches every other the moment it next ticks - see
/// `main`'s timer and `sync_ui`.
pub struct PaneWindows {
    main: MainWindow,
    /// Each popped-out pane's own window, alongside the tracker that keeps
    /// its geometry up to date for [`PaneWindows::pane_layout`] to save
    /// (#620) - the same `GeometryTracker` the main window carries in
    /// `main`'s own local, since a popped-out window needs the same "last
    /// normal bounds, in case the platform hands it back maximised or off a
    /// display that has since gone" bookkeeping.
    popped: HashMap<Pane, (MainWindow, Rc<RefCell<GeometryTracker>>)>,
    /// Windows a File pop-out has been pinned into (#619), by the id
    /// `App::pin_current`/`App::pin_extra_window` hands out - present here
    /// whether or not `App` still counts that id as pinned right now.
    pinned: HashMap<app::PinId, MainWindow>,
}

impl PaneWindows {
    /// A registry holding only `main`, with nothing popped out of it yet.
    #[must_use]
    pub fn new(main: MainWindow) -> Self {
        Self {
            main,
            popped: HashMap::new(),
            pinned: HashMap::new(),
        }
    }

    /// The main window.
    #[must_use]
    pub fn main(&self) -> &MainWindow {
        &self.main
    }

    /// Every open window: the main one, then each popped-out one, then
    /// each window a pin ever opened.
    pub fn windows(&self) -> impl Iterator<Item = &MainWindow> {
        std::iter::once(&self.main)
            .chain(self.popped.values().map(|(ui, _)| ui))
            .chain(self.pinned.values())
    }

    /// Every open window alongside the pin id its own slot is tracked
    /// under, if it has one - the main window and a live popped-out one
    /// carry `None`; a window a pin ever opened carries `Some`, whether or
    /// not `App` still counts it as pinned right now (`App::is_pinned`
    /// says which).
    pub fn windows_with_pin(&self) -> impl Iterator<Item = (&MainWindow, Option<app::PinId>)> {
        std::iter::once((&self.main, None))
            .chain(self.popped.values().map(|(ui, _)| (ui, None)))
            .chain(self.pinned.iter().map(|(&id, ui)| (ui, Some(id))))
    }

    /// The window holding `pane` right now: the one it popped out into, or
    /// the main window if it has not.
    #[must_use]
    pub fn window_for(&self, pane: Pane) -> &MainWindow {
        self.popped.get(&pane).map_or(&self.main, |(ui, _)| ui)
    }

    /// Whether `pane` is currently popped out of the main window.
    #[must_use]
    pub fn is_popped_out(&self, pane: Pane) -> bool {
        self.popped.contains_key(&pane)
    }

    /// The window `id` was ever pinned into, if it still has one open.
    #[must_use]
    pub fn pinned_window(&self, id: app::PinId) -> Option<&MainWindow> {
        self.pinned.get(&id)
    }

    /// The pane layout to remember at exit (#620): every pane currently
    /// popped out, at the last normal bounds its own tracker saw, with
    /// `maximized` as the window is right now. A pinned window is never in
    /// `popped` in the first place (pinning removes it - see
    /// `pin_the_popped_file_window`), so it is never part of this layout,
    /// matching the work order's "pins are not restored". `main` calls
    /// this on the way out, alongside saving the main window's own
    /// geometry (#583).
    #[must_use]
    pub fn pane_layout(&self) -> settings::PaneLayout {
        let geometry_for = |pane: Pane| {
            let (ui, tracker) = self.popped.get(&pane)?;
            let maximized = ui.window().is_maximized();
            normal_window_geometry(ui)
                .map(|geometry| settings::WindowGeometry {
                    maximized,
                    ..geometry
                })
                .or_else(|| tracker.borrow().closing_at(maximized))
        };
        settings::PaneLayout {
            folders: geometry_for(Pane::Folders),
            contents: geometry_for(Pane::Contents),
            file: geometry_for(Pane::File),
        }
    }

    /// Keeps every popped-out pane window's geometry tracker up to date,
    /// and puts one back onto a connected display the moment it stops
    /// being maximised - the popped-out-pane counterpart of
    /// [`observe_window_geometry`] for the main window (#583, extended by
    /// #620). `main` calls this on every tick.
    pub fn observe_pane_geometries(&self) {
        for (ui, tracker) in self.popped.values() {
            let just_restored = tracker
                .borrow_mut()
                .observed(normal_window_geometry(ui), ui.window().is_maximized());
            if just_restored && let Some(resolved) = settle_pane_onto_a_display_now(ui, &self.main)
            {
                tracker.borrow_mut().corrected_to(resolved);
            }
        }
    }
}

/// `pane`'s index among the three panes (0 Folders, 1 Contents, 2 File):
/// what `pop-out-requested`/`dock-requested` and a `PaneMenuRow` carry,
/// matching `App::focus_index`.
fn pane_index(pane: Pane) -> i32 {
    match pane {
        Pane::Folders => 0,
        Pane::Contents => 1,
        Pane::File => 2,
    }
}

/// The pane a `pop-out-requested`/`dock-requested` index names, or `None`
/// for a value that names none of the three - which the markup never
/// sends, but a stray one is ignored rather than mistaken for a pane.
fn pane_from_index(index: i32) -> Option<Pane> {
    match index {
        0 => Some(Pane::Folders),
        1 => Some(Pane::Contents),
        2 => Some(Pane::File),
        _ => None,
    }
}

/// What a popped-out window for `pane` is titled after: "Folders" and
/// "Contents" plainly, and the tool slot's own current title - which
/// changes with the selection - for the third.
fn pane_title(pane: Pane, app: &App) -> String {
    match pane {
        Pane::Folders => "Folders".to_string(),
        Pane::Contents => "Contents".to_string(),
        Pane::File => app.active_tool_title(),
    }
}

/// The View menu's Pop Out and Dock lists (#617): every pane not currently
/// popped out, and every one that is.
fn pane_menu_rows(windows: &PaneWindows, app: &App) -> (Vec<PaneMenuRow>, Vec<PaneMenuRow>) {
    let mut pop_out_rows = Vec::new();
    let mut dock_rows = Vec::new();
    for pane in [Pane::Folders, Pane::Contents, Pane::File] {
        let row = PaneMenuRow {
            label: pane_title(pane, app).into(),
            pane: pane_index(pane),
        };
        if windows.popped.contains_key(&pane) {
            dock_rows.push(row);
        } else {
            pop_out_rows.push(row);
        }
    }
    (pop_out_rows, dock_rows)
}

/// Pushes the View menu's Pop Out and Dock lists, freshly built from which
/// panes are popped out right now, onto every open window - so a pane
/// popped out or docked from any one of them is reflected in all.
fn refresh_pane_menus(windows: &Rc<RefCell<PaneWindows>>, app: &App) {
    let windows = windows.borrow();
    let (pop_out_rows, dock_rows) = pane_menu_rows(&windows, app);
    for ui in windows.windows() {
        ui.set_pop_out_rows(ModelRc::new(VecModel::from(pop_out_rows.clone())));
        ui.set_dock_rows(ModelRc::new(VecModel::from(dock_rows.clone())));
    }
}

/// Sizes `ui` sensibly and places it beside `main` (#617), rather than
/// leaving it wherever the platform's own default happens to put a new
/// window.
fn place_beside_main(main: &MainWindow, ui: &slint::Window) {
    const DEFAULT_WIDTH: f32 = 480.0;
    const DEFAULT_HEIGHT: f32 = 600.0;
    place_beside_main_sized(main, ui, DEFAULT_WIDTH, DEFAULT_HEIGHT);
}

/// [`place_beside_main`], at a size the caller chose: what a pane window
/// whose remembered position is on no connected display is moved to, since
/// #620 requirement 5 asks for it to keep its remembered size, shrunk to
/// fit, rather than be reset to the size a freshly popped-out pane gets.
fn place_beside_main_sized(main: &MainWindow, ui: &slint::Window, width: f32, height: f32) {
    ui.set_size(slint::LogicalSize::new(width, height));
    if let Some(geometry) = normal_window_geometry(main) {
        ui.set_position(slint::LogicalPosition::new(
            geometry.x + geometry.width,
            geometry.y,
        ));
    }
}

/// Pops `pane` out of the window that currently holds it into a new window
/// of its own (#617): titled for the pane, wired the same as any other
/// window - a no-op if it is already out. Placed and sized beside the main
/// window when `remembered` is `None`, a pane popped out during this run,
/// or restored to `remembered`'s own position, size and maximised state
/// when it is `Some`, a layout `restore_pane_layout` is replaying at
/// launch (#620).
fn pop_out(
    pane: Pane,
    remembered: Option<settings::WindowGeometry>,
    windows: &Rc<RefCell<PaneWindows>>,
    app: &Rc<RefCell<App>>,
) {
    if windows.borrow().popped.contains_key(&pane) {
        return;
    }
    let title = pane_title(pane, &app.borrow());
    let new_ui = MainWindow::new().expect("a popped-out window should build");
    new_ui.set_is_main_window(false);
    new_ui.set_show_folders_pane(pane == Pane::Folders);
    new_ui.set_show_contents_pane(pane == Pane::Contents);
    new_ui.set_show_file_pane(pane == Pane::File);
    new_ui.set_window_title(format!("Repos Explorer - {title}").into());
    let tracker = {
        let main_window = windows.borrow();
        wire_pane_geometry(&new_ui, &main_window.main, remembered)
    };
    wire_callbacks(&new_ui, app);
    wire_pop_out(&new_ui, Some(pane), windows, app);
    sync_ui(&new_ui, &app.borrow());
    new_ui.show().expect("a popped-out window should show");

    {
        let main_window = windows.borrow();
        match pane {
            Pane::Folders => main_window.main.set_show_folders_pane(false),
            Pane::Contents => main_window.main.set_show_contents_pane(false),
            Pane::File => main_window.main.set_show_file_pane(false),
        }
    }
    windows.borrow_mut().popped.insert(pane, (new_ui, tracker));
    refresh_pane_menus(windows, &app.borrow());
}

/// Applies `remembered` to a freshly created popped-out window, or places
/// it beside `main` when there is none - #617's original placement, still
/// used for a pane popped out live rather than restored from a remembered
/// layout - and returns the tracker that keeps its bounds up to date from
/// here on, the same bookkeeping [`GeometryTracker`] already does for the
/// main window (#583, extended to a popped-out pane's own window by #620).
fn wire_pane_geometry(
    new_ui: &MainWindow,
    main: &MainWindow,
    remembered: Option<settings::WindowGeometry>,
) -> Rc<RefCell<GeometryTracker>> {
    match remembered {
        Some(geometry) => {
            let window = new_ui.window();
            window.set_position(slint::LogicalPosition::new(geometry.x, geometry.y));
            window.set_size(slint::LogicalSize::new(geometry.width, geometry.height));
            if geometry.maximized {
                window.set_maximized(true);
            }
        }
        None => place_beside_main(main, new_ui.window()),
    }
    let tracker = Rc::new(RefCell::new(GeometryTracker::opening_at(remembered)));
    if let Some(geometry) = remembered {
        let ui_weak = new_ui.as_weak();
        let main_weak = main.as_weak();
        let settle_tracker = tracker.clone();
        // Zero delay, the same as `wire_window_geometry`'s own settle timer:
        // as soon as the event loop is running, which is when the
        // connected displays can be asked for at all.
        slint::Timer::single_shot(std::time::Duration::ZERO, move || {
            if let (Some(ui), Some(main)) = (ui_weak.upgrade(), main_weak.upgrade())
                && let Some(resolved) = settle_pane_onto_a_display(&ui, &main, geometry)
            {
                settle_tracker.borrow_mut().corrected_to(resolved);
            }
        });
    }
    tracker
}

/// The popped-out-pane counterpart of [`settle_remembered_geometry`]: the
/// same "correct once the event loop is running" shape, but a pane window
/// that has lost its display moves beside the main window (#620
/// requirement 5) rather than centring on the primary one, since a lone
/// pane window belongs next to the window it came from.
fn settle_pane_onto_a_display(
    ui: &MainWindow,
    main: &MainWindow,
    remembered: settings::WindowGeometry,
) -> Option<settings::WindowGeometry> {
    let (displays, primary) = connected_displays(ui.window())?;
    if !settings::on_any_display(remembered, &displays) && !ui.window().is_maximized() {
        // Its own remembered size, shrunk to whatever the primary display
        // can hold (#620 requirement 5) - the same shrink-to-fit the main
        // window's `geometry_on_a_display` does, rather than the size a
        // pane popped out live is given.
        let (width, height) = settings::shrunk_to_fit(remembered, primary);
        place_beside_main_sized(main, ui.window(), width, height);
    }
    normal_window_geometry(ui).or(Some(remembered))
}

/// [`settle_pane_onto_a_display`] against the window's own current bounds
/// rather than a specific remembered value - what
/// [`PaneWindows::observe_pane_geometries`] calls the moment a pane window
/// stops being maximised, the same way [`settle_window_onto_a_display`]
/// does for the main window.
fn settle_pane_onto_a_display_now(
    ui: &MainWindow,
    main: &MainWindow,
) -> Option<settings::WindowGeometry> {
    let current = normal_window_geometry(ui)?;
    settle_pane_onto_a_display(ui, main, current)
}

/// Restores a remembered pane layout (#620) at launch: pops each pane
/// `layout` names out, into its own window at the geometry it remembers. A
/// pinned window is never part of `layout` in the first place
/// ([`settings::PaneLayout`]'s own doc explains why), so there is nothing
/// here to reopen pinned - every restored window opens docked-turned-popped,
/// following the shared selection. `main` calls this once, with what
/// `settings::load_pane_layout` read, before the windows are shown.
pub fn restore_pane_layout(
    layout: settings::PaneLayout,
    windows: &Rc<RefCell<PaneWindows>>,
    app: &Rc<RefCell<App>>,
) {
    for (pane, geometry) in [
        (Pane::Folders, layout.folders),
        (Pane::Contents, layout.contents),
        (Pane::File, layout.file),
    ] {
        if let Some(geometry) = geometry {
            pop_out(pane, Some(geometry), windows, app);
        }
    }
}

/// Docks `pane` back into the main window, closing the window it had
/// popped out into (#617) - a no-op if it is not out.
fn dock(pane: Pane, windows: &Rc<RefCell<PaneWindows>>, app: &Rc<RefCell<App>>) {
    let Some((popped_ui, _tracker)) = windows.borrow_mut().popped.remove(&pane) else {
        return;
    };
    let _ = popped_ui.hide();
    {
        let main_window = windows.borrow();
        match pane {
            Pane::Folders => main_window.main.set_show_folders_pane(true),
            Pane::Contents => main_window.main.set_show_contents_pane(true),
            Pane::File => main_window.main.set_show_file_pane(true),
        }
    }
    refresh_pane_menus(windows, &app.borrow());
}

/// Docks every popped-out pane back at once (#618): the empty main
/// window's own "Dock All" button.
fn dock_all(windows: &Rc<RefCell<PaneWindows>>, app: &Rc<RefCell<App>>) {
    for pane in [Pane::Folders, Pane::Contents, Pane::File] {
        dock(pane, windows, app);
    }
}

/// Closes the main window on its own (#618), leaving any popped-out window
/// running: the application exits only once every window, this one
/// included, has closed - Slint's own default once none of them are
/// visible any more.
fn close_main_window(windows: &Rc<RefCell<PaneWindows>>) {
    let _ = windows.borrow().main.hide();
}

/// Brings the main window back (#618) after it was closed while panes were
/// still popped out - "View > Show Main Window" in any popped-out window.
/// It still holds whatever panes were docked when it closed, at the Repos
/// Directory's current selection: the timer syncs it on every tick whether
/// or not it is showing.
fn show_main_window(windows: &Rc<RefCell<PaneWindows>>) {
    let _ = windows.borrow().main.show();
}

/// Wires a window's pop-out/dock button and its View menu's Pop Out and
/// Dock lists (#617): `own_pane` is the pane this window is dedicated to
/// popped out into its own window, or `None` for the main window, which
/// holds all three until one of them pops out.
///
/// In the library rather than `main`, the same as [`wire_callbacks`], so a
/// window test drives the same code the application does (rule 14).
pub fn wire_pop_out(
    ui: &MainWindow,
    own_pane: Option<Pane>,
    windows: &Rc<RefCell<PaneWindows>>,
    app: &Rc<RefCell<App>>,
) {
    {
        let windows = windows.clone();
        let app = app.clone();
        ui.on_pop_out_requested(move |index| {
            if let Some(pane) = pane_from_index(index) {
                pop_out(pane, None, &windows, &app);
            }
        });
    }
    {
        let windows = windows.clone();
        let app = app.clone();
        ui.on_dock_requested(move |index| {
            if let Some(pane) = pane_from_index(index) {
                dock(pane, &windows, &app);
            }
        });
    }
    {
        let windows = windows.clone();
        let app = app.clone();
        ui.on_dock_all_requested(move || {
            dock_all(&windows, &app);
        });
    }
    {
        let windows = windows.clone();
        ui.on_show_main_window_requested(move || {
            show_main_window(&windows);
        });
    }
    if let Some(pane) = own_pane {
        // Closing a popped-out window from the platform's own decoration
        // docks it back, the same as its own dock button (GUIDANCE.md
        // §2.6) - unless it has been pinned (#619 requirement 7), which
        // `pane == Pane::File` below wires differently.
        {
            let windows = windows.clone();
            let app = app.clone();
            ui.window().on_close_requested(move || {
                dock(pane, &windows, &app);
                slint::CloseRequestResponse::KeepWindowShown
            });
        }
        // The pin button (#619) only ever appears on the File pane's own
        // popped-out window - see `PaneFrame`'s `pinnable`, set only where
        // `file-pane` is instantiated - so this is the only pane whose
        // pop-out window needs any of this wired at all.
        //
        // `pin_slot` remembers the id this window is pinned under, once it
        // ever has been - `None` until then. Every one of these four
        // callbacks is wired exactly once, right here, and reads or sets
        // `pin_slot` on each call rather than being replaced later:
        // replacing a callback from inside its own invocation (pinning,
        // from the very `pin-requested` handler a click just entered)
        // panics Slint's generated code ("Callback Handler set while
        // called").
        wire_pinning(ui, pane, windows, app);
    } else {
        // Closing the main window closes only the main window (#618): any
        // popped-out window keeps running, linked to every other, and the
        // application keeps running with it. The application exits once
        // every window - this one included - has closed, which is Slint's
        // own default once none of them are visible any more.
        let windows = windows.clone();
        ui.window().on_close_requested(move || {
            close_main_window(&windows);
            slint::CloseRequestResponse::KeepWindowShown
        });
    }
}

/// Wires the pin button and the two ways a File pop-out window can be
/// closed (#619). Its own function because the pinned case is not the
/// docking case: Dock unpins and keeps the window, and the platform's
/// close button refuses while an edit in it is unsaved.
fn wire_pinning(
    ui: &MainWindow,
    pane: Pane,
    windows: &Rc<RefCell<PaneWindows>>,
    app: &Rc<RefCell<App>>,
) {
    let pin_slot: Rc<std::cell::Cell<Option<app::PinId>>> = Rc::new(std::cell::Cell::new(None));
    {
        let windows = windows.clone();
        let app = app.clone();
        let pin_slot = pin_slot.clone();
        ui.window().on_close_requested(move || {
            match pin_slot.get() {
                // A pinned window holds the only copy of an edit in
                // progress: the shared selection has no room for it,
                // so closing would destroy it silently. Refuse, and
                // say why, the way `cancel_file_edit` says "edit
                // discarded" rather than losing one quietly.
                Some(id) if app.borrow().pinned_edit_modified(id) => {
                    refuse_to_lose_an_edit(&app);
                }
                Some(id) => close_extra(id, &windows, &app),
                None => dock(pane, &windows, &app),
            }
            slint::CloseRequestResponse::KeepWindowShown
        });
    }
    {
        let windows = windows.clone();
        let app = app.clone();
        let pin_slot = pin_slot.clone();
        ui.on_dock_requested(move |index| match pin_slot.get() {
            // #619 requirement 7, as written: docking a pinned
            // window unpins it. The window stays where it is and
            // follows the shared selection again, which is what the
            // Unpin button does - and nothing it was holding, an
            // edit in progress included, is lost.
            Some(id) => unpin_extra(id, &windows, &app),
            None => {
                if let Some(pane) = pane_from_index(index) {
                    dock(pane, &windows, &app);
                }
            }
        });
    }
    {
        let windows = windows.clone();
        let app = app.clone();
        let pin_slot = pin_slot.clone();
        ui.on_pin_requested(move || match pin_slot.get() {
            None => {
                if let Some(id) = pin_the_popped_file_window(&windows, &app) {
                    pin_slot.set(Some(id));
                }
            }
            Some(id) => pin_extra(id, &windows, &app),
        });
    }
    {
        let windows = windows.clone();
        let app = app.clone();
        ui.on_unpin_requested(move || {
            if let Some(id) = pin_slot.get() {
                unpin_extra(id, &windows, &app);
            }
        });
    }
}

/// Pins the tool slot's live File pop-out window to what it is showing
/// now (#619): a no-op if nothing is selected to pin. From here on this
/// window is its own thing - popping the File pane out again opens a
/// fresh window that follows the shared selection, rather than reclaiming
/// this one (requirement 4) - so its pin, dock and close all move to the
/// id-keyed handlers below, for the rest of its life.
///
/// Wires none of the window's own callbacks: `wire_pop_out` already wired
/// its pin/unpin/dock/close once, up front, to read the id this returns
/// back out of the `pin_slot` it is stored in - replacing them here, from
/// inside the very `pin-requested` handler a click just entered, is what
/// used to panic Slint's generated code ("Callback Handler set while
/// called").
fn pin_the_popped_file_window(
    windows: &Rc<RefCell<PaneWindows>>,
    app: &Rc<RefCell<App>>,
) -> Option<app::PinId> {
    let (ui, tracker) = windows.borrow_mut().popped.remove(&Pane::File)?;
    let Some(id) = app.borrow_mut().pin_current() else {
        windows
            .borrow_mut()
            .popped
            .insert(Pane::File, (ui, tracker));
        return None;
    };
    wire_file_pane_pinned(&ui, id, app);
    apply_pinned_state(&ui, &app.borrow(), id);
    windows.borrow_mut().pinned.insert(id, ui);
    refresh_pane_menus(windows, &app.borrow());
    Some(id)
}

/// Pins `id`'s window again, to whatever the shared selection is showing
/// now - the same window, already tracked under `id` from an earlier pin,
/// picking a fresh snapshot back up after having followed the shared
/// selection since it was last unpinned.
fn pin_extra(id: app::PinId, windows: &Rc<RefCell<PaneWindows>>, app: &Rc<RefCell<App>>) {
    if !app.borrow_mut().pin_extra_window(id) {
        return;
    }
    if let Some(ui) = windows.borrow().pinned.get(&id) {
        apply_pinned_state(ui, &app.borrow(), id);
    }
}

/// Unpins `id`'s window (#619 requirement 5): it keeps showing what it had
/// until the next tick, which - following the shared selection again from
/// this moment on - draws whatever that is right away.
fn unpin_extra(id: app::PinId, windows: &Rc<RefCell<PaneWindows>>, app: &Rc<RefCell<App>>) {
    app.borrow_mut().unpin(id);
    if let Some(ui) = windows.borrow().pinned.get(&id) {
        ui.set_pinned(false);
        ui.set_window_title(
            format!("Repos Explorer - {}", app.borrow().active_tool_title()).into(),
        );
        sync_ui(ui, &app.borrow());
    }
}

/// Keeps `id`'s window open because it holds unsaved changes, and says so
/// in its status line.
///
/// A pinned window's edit lives in its own `PinnedWindow` and nowhere else:
/// an ordinary popped-out File window docks its edit back into the shared
/// selection, and a pinned one has nothing to dock into. Closing it would
/// be the one silent loss of work in the application (the review of #659).
fn refuse_to_lose_an_edit(app: &Rc<RefCell<App>>) {
    // Reported through the application, not written onto the window: a
    // pinned window's status line is synced from the shared `App` on every
    // tick, so a line set on the window alone would be gone a moment later.
    app.borrow_mut()
        .report("unsaved changes: save them, or discard the edit, before closing this window");
}

/// Closes `id`'s window for good: the platform's close button, once
/// nothing would be lost by it. A pinned window has no pane slot of its
/// own in the main window to return into, so closing is closing - its Dock
/// button unpins instead.
fn close_extra(id: app::PinId, windows: &Rc<RefCell<PaneWindows>>, app: &Rc<RefCell<App>>) {
    if let Some(ui) = windows.borrow_mut().pinned.remove(&id) {
        let _ = ui.hide();
    }
    app.borrow_mut().unpin(id);
    refresh_pane_menus(windows, &app.borrow());
}

/// Pushes `id`'s freshly pinned state onto `ui`: the pinned flag and
/// tooltip its own button reads, the window's title, and its content.
fn apply_pinned_state(ui: &MainWindow, app: &App, id: app::PinId) {
    ui.set_pinned(true);
    if let Some(title) = app.pinned_title(id) {
        ui.set_window_title(format!("Repos Explorer - {title}").into());
    }
    sync_pinned_window(ui, app, id);
}

/// Wires a pinned window's File pane callbacks (#619) to act on its own
/// snapshot, under `id`, rather than the shared selection every other
/// window follows - the pinned counterpart of the editing and tab/view
/// choice callbacks `wire_editor`, `wire_rows` and `wire_commands` wire for
/// a live window. Called once, right after a window is pinned for the
/// first time; it stays bound for the rest of the window's life, pinned or
/// not, since an unpinned extra window still only ever shows its own File
/// pane.
fn wire_file_pane_pinned(ui: &MainWindow, id: app::PinId, app: &Rc<RefCell<App>>) {
    wire_editor_pinned(ui, id, app);
    {
        let app = Rc::clone(app);
        let ui_weak = ui.as_weak();
        ui.on_edit_requested(move || {
            let mut app = app.borrow_mut();
            app.pinned_begin_file_edit(id);
            if let Some(ui) = ui_weak.upgrade() {
                sync_pinned_window(&ui, &app, id);
            }
        });
    }
    {
        let app = Rc::clone(app);
        let ui_weak = ui.as_weak();
        ui.on_save_requested(move || {
            let mut app = app.borrow_mut();
            if let Some(ui) = ui_weak.upgrade() {
                app.pinned_set_edit_text(id, &ui.get_edit_text());
            }
            app.pinned_save_file_edit(id);
            if let Some(ui) = ui_weak.upgrade() {
                sync_pinned_window(&ui, &app, id);
            }
        });
    }
    {
        let app = Rc::clone(app);
        let ui_weak = ui.as_weak();
        ui.on_cancel_requested(move || {
            let mut app = app.borrow_mut();
            app.pinned_cancel_file_edit(id);
            if let Some(ui) = ui_weak.upgrade() {
                sync_pinned_window(&ui, &app, id);
            }
        });
    }
    {
        let app = Rc::clone(app);
        let ui_weak = ui.as_weak();
        ui.on_file_view_selected(move |i| {
            let mut app = app.borrow_mut();
            app.pinned_select_file_view(id, usize::try_from(i).unwrap_or(usize::MAX));
            if let Some(ui) = ui_weak.upgrade() {
                sync_pinned_window(&ui, &app, id);
            }
        });
    }
    {
        let app = Rc::clone(app);
        let ui_weak = ui.as_weak();
        ui.on_file_tab_selected(move |i| {
            let mut app = app.borrow_mut();
            app.pinned_select_file_tab(id, usize::try_from(i).unwrap_or(usize::MAX));
            if let Some(ui) = ui_weak.upgrade() {
                sync_pinned_window(&ui, &app, id);
            }
        });
    }
}

/// The keystroke and command half of [`wire_file_pane_pinned`], split out
/// only to keep that function under its line cap - the pinned counterpart
/// of [`wire_editor`].
fn wire_editor_pinned(ui: &MainWindow, id: app::PinId, app: &Rc<RefCell<App>>) {
    {
        let app = Rc::clone(app);
        let ui_weak = ui.as_weak();
        let mut clipboard = clipboard();
        ui.on_edit_key(move |text, shift, control| {
            let Some(ui) = ui_weak.upgrade() else {
                return false;
            };
            let rows = usize::try_from(ui.get_edit_visible_rows())
                .unwrap_or(20)
                .max(1);
            let mut app = app.borrow_mut();
            let used = app.pinned_edit_key(id, &mut clipboard, &text, shift, control, rows);
            sync_pinned_window(&ui, &app, id);
            used
        });
    }
    macro_rules! on_edit_command_pinned {
        ($setter:ident, $command:ident) => {{
            let app = Rc::clone(app);
            let ui_weak = ui.as_weak();
            let mut clipboard = clipboard();
            ui.$setter(move || {
                let mut app = app.borrow_mut();
                app.pinned_edit_command(id, app::EditCommand::$command, &mut clipboard);
                if let Some(ui) = ui_weak.upgrade() {
                    sync_pinned_window(&ui, &app, id);
                }
            });
        }};
    }
    on_edit_command_pinned!(on_edit_undo_requested, Undo);
    on_edit_command_pinned!(on_edit_redo_requested, Redo);
    on_edit_command_pinned!(on_edit_cut_requested, Cut);
    on_edit_command_pinned!(on_edit_copy_requested, Copy);
    on_edit_command_pinned!(on_edit_paste_requested, Paste);
    {
        let app = Rc::clone(app);
        let ui_weak = ui.as_weak();
        ui.on_edit_pressed(move |line, column| {
            let mut app = app.borrow_mut();
            app.pinned_edit_click(
                id,
                usize::try_from(line).unwrap_or(0),
                usize::try_from(column).unwrap_or(0),
                false,
            );
            if let Some(ui) = ui_weak.upgrade() {
                sync_pinned_window(&ui, &app, id);
            }
        });
    }
}

/// Wires the tool slot's picker (#616): choosing a tool from it.
fn wire_tools(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    let app = app.clone();
    let ui_weak = ui.as_weak();
    ui.on_tool_selected(move |index| {
        let mut app = app.borrow_mut();
        app.choose_tool(usize::try_from(index).unwrap_or(usize::MAX));
        if let Some(ui) = ui_weak.upgrade() {
            sync_ui(&ui, &app);
        }
    });
}

/// Wires View > All Repositories and its Folders tree entry (#591).
fn wire_all_repositories(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    let app = app.clone();
    let ui_weak = ui.as_weak();
    ui.on_all_repositories_requested(move || {
        let mut app = app.borrow_mut();
        app.open_all_repositories();
        if let Some(ui) = ui_weak.upgrade() {
            sync_ui(&ui, &app);
        }
    });
}

/// Wires View > Certificates (#622), and the tool's own table: sorting,
/// selecting a row, and expanding the private keys line.
fn wire_certificates(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    {
        let app = app.clone();
        let ui_weak = ui.as_weak();
        ui.on_certificates_requested(move || {
            let mut app = app.borrow_mut();
            app.open_certificates_tool();
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
    {
        let app = app.clone();
        let ui_weak = ui.as_weak();
        ui.on_certificates_sort_requested(move |column| {
            let mut app = app.borrow_mut();
            app.certificates_sort_by_column(column);
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
    {
        let app = app.clone();
        let ui_weak = ui.as_weak();
        ui.on_certificate_row_activated(move |index| {
            let mut app = app.borrow_mut();
            app.select_certificate_row(usize::try_from(index).unwrap_or(usize::MAX));
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
    {
        let app = app.clone();
        let ui_weak = ui.as_weak();
        ui.on_certificates_private_keys_toggled(move || {
            let mut app = app.borrow_mut();
            app.toggle_certificates_private_keys_shown();
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
}

/// Wires Ctrl+P / Cmd+P's Go to Repository switcher (#590): opening it, and
/// clicking one of its results.
fn wire_switcher(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    {
        let app = app.clone();
        let ui_weak = ui.as_weak();
        ui.on_switcher_open_requested(move || {
            let mut app = app.borrow_mut();
            app.begin_switcher();
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
    {
        let app = app.clone();
        let ui_weak = ui.as_weak();
        ui.on_switcher_row_clicked(move |index| {
            let mut app = app.borrow_mut();
            app.activate_switcher_result(usize::try_from(index).unwrap_or(usize::MAX));
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
}

/// Wires the File pane's "Worktree of"/"Submodule of" link (#587), split
/// out of [`wire_commands`] only to keep that function under its line cap.
fn wire_related_repository_link(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    let app = app.clone();
    let ui_weak = ui.as_weak();
    ui.on_file_related_repository_open_requested(move || {
        let mut app = app.borrow_mut();
        app.open_related_repository();
        if let Some(ui) = ui_weak.upgrade() {
            sync_ui(&ui, &app);
        }
    });
}

/// Fills in the Help > Keyboard shortcuts sheet's rows (#585). Set once,
/// rather than on every [`sync_ui`], because the table does not change
/// while the window runs; opening and closing the sheet is markup-only
/// state, in `shortcuts-open`.
fn wire_shortcuts_sheet(ui: &MainWindow) {
    let mac = matches!(launch::Platform::current(), launch::Platform::MacOs);
    ui.set_shortcut_rows(ModelRc::new(VecModel::from(shortcuts::rows(mac))));
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
    on_delta_event!(on_message_focus_requested, move_message_focus);

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
    on_event!(on_quick_look_close_requested, close_quick_look);
    on_event!(on_return_pressed, handle_return);
    on_event!(on_backspace_pressed, backspace);
    on_event!(on_parent_requested, navigate_to_parent);
    on_event!(on_back_requested, go_back);
    on_event!(on_forward_requested, go_forward);
    on_event!(on_find_requested, begin_find);
    wire_filter_actions(ui, app);
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
    wire_folder_actions(ui, app);
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
    on_event!(on_file_readme_open_requested, open_readme);
    on_event!(on_undo_requested, undo);
    on_event!(on_new_folder_requested, request_new_folder);
    on_event!(on_new_file_requested, request_new_file);
    wire_window_chrome(ui, app);
}

/// Wires File > Exit, Help > About and File > Repos Directory... - three
/// commands with no `App` state in common, split out of `wire_commands`
/// only because #721's quick-look wiring pushed it over
/// `clippy::too_many_lines`.
fn wire_window_chrome(ui: &MainWindow, app: &Rc<RefCell<App>>) {
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
}

/// Runs `command`, detached: the caller outlives nothing it should not, and
/// a slow-starting terminal or editor does not hold the window.
fn run_detached(command: &launch::Launch) -> std::io::Result<()> {
    // The full path when the `PATH` has it, so `code` finds `code.cmd` on
    // Windows; otherwise the name as given, for the operating system to
    // resolve or refuse.
    let program = launch::find_on_path(&command.program)
        .map_or_else(|| command.program.clone().into(), std::ffi::OsString::from);
    let mut process = std::process::Command::new(program);
    process.args(&command.args);
    if let Some(dir) = &command.current_dir {
        process.current_dir(dir);
    }
    process.spawn().map(|_| ())
}

/// Wires the Contents pane's filter field and the status bar's two links
/// (#582): Ctrl+F or a click on the field, the changed count, and "clear".
fn wire_filter_actions(ui: &MainWindow, app: &Rc<RefCell<App>>) {
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

    on_event!(on_filter_focus_requested, begin_filter);
    on_event!(on_status_changed_link_clicked, filter_to_changed);
    on_event!(on_status_clear_filter_clicked, clear_filters);
    on_event!(on_status_link_focus_requested, focus_status_link);
    on_event!(on_changed_filter_toggled, toggle_changed_filter);
}

/// Wires handing a selected folder to a program the user already has
/// (#581): the Contents pane's row menu and the File menu share one set of
/// callbacks, since both act on the Contents pane's selection the same way
/// Open, Rename and Delete already do; the Folders pane's row menu gets its
/// own, acting on whichever row was right-clicked there.
fn wire_folder_actions(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    use editor::Clipboard as _;

    macro_rules! on_launch {
        ($setter:ident, $method:ident) => {{
            let app = app.clone();
            let ui_weak = ui.as_weak();
            ui.$setter(move || {
                let mut app = app.borrow_mut();
                app.$method(run_detached);
                if let Some(ui) = ui_weak.upgrade() {
                    sync_ui(&ui, &app);
                }
            });
        }};
    }
    macro_rules! on_copy {
        ($setter:ident, $method:ident) => {{
            let app = app.clone();
            let ui_weak = ui.as_weak();
            let mut clipboard = clipboard();
            ui.$setter(move || {
                let mut app = app.borrow_mut();
                app.$method(|text| clipboard.write(text));
                if let Some(ui) = ui_weak.upgrade() {
                    sync_ui(&ui, &app);
                }
            });
        }};
    }

    on_launch!(on_open_terminal_requested, open_terminal_here);
    on_launch!(on_open_in_editor_requested, open_selected_in_editor);
    on_launch!(
        on_show_in_file_manager_requested,
        show_selected_in_file_manager
    );
    on_copy!(on_copy_path_requested, copy_selected_path);
    on_copy!(
        on_copy_remote_address_requested,
        copy_selected_remote_address
    );

    on_launch!(on_folder_open_terminal_requested, open_terminal_at_folder);
    on_launch!(on_folder_open_in_editor_requested, open_folder_in_editor);
    on_launch!(
        on_folder_show_in_file_manager_requested,
        show_folder_in_file_manager
    );
    on_copy!(on_folder_copy_path_requested, copy_folder_path);
    on_copy!(
        on_folder_copy_remote_address_requested,
        copy_folder_remote_address
    );
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

/// Wires Ctrl+Plus/Ctrl+=, Ctrl+Minus and Ctrl+0 (#586) - and the matching
/// View menu items, which fire the same three callbacks - to `App`'s zoom
/// steps.
fn wire_zoom(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    macro_rules! on_zoom_command {
        ($setter:ident, $method:ident) => {{
            let app = Rc::clone(app);
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
    on_zoom_command!(on_zoom_in_requested, zoom_in);
    on_zoom_command!(on_zoom_out_requested, zoom_out);
    on_zoom_command!(on_zoom_reset_requested, zoom_reset);
}

/// A tree row's application state, rendered into the Slint struct the pane
/// draws: the icon carries the branch mark (#579), and `is_repository`
/// tells the pane's name text to match it.
fn folder_row_view(row: app::FolderRow) -> FolderRow {
    FolderRow {
        icon: icon_image(row.icon, true, row.mark),
        name: row.name.into(),
        depth: row_index(row.depth),
        expandable: row.expandable,
        expanded: row.expanded,
        is_repository: row.mark.is_some(),
    }
}

/// Copies the Contents pane's filter field and the status bar's two links
/// (#582) into `ui`'s bound properties.
fn sync_filter(ui: &MainWindow, app: &App) {
    ui.set_filter_text(app.filter_text().into());
    ui.set_filter_focused(app.filter_focused());
    ui.set_status_changed_label(app.status_changed_label().into());
    ui.set_status_changed_accessible_label(app.status_changed_accessible_label().into());
    ui.set_status_show_clear_link(app.status_show_clear_link());
    ui.set_status_link_focused(app.status_link_focused());
}

/// Copies the Contents pane's centred message (#592) into `ui`'s bound
/// properties.
fn sync_contents_message(ui: &MainWindow, app: &App) {
    ui.set_message_title(app.contents_message_title().into());
    ui.set_message_detail(app.contents_message_detail().into());
    ui.set_message_show_retry(app.contents_message_show_retry());
    ui.set_message_show_choose(app.contents_message_show_choose());
    ui.set_message_show_clear_filter(app.contents_message_show_clear_filter());
    ui.set_message_focus_index(app.contents_message_focus());
}

/// Copies the Go to Repository switcher's state (#590) into `ui`'s bound
/// properties: whether it is open, its typed query, and its matches.
fn sync_switcher(ui: &MainWindow, app: &App) {
    ui.set_switcher_open(app.switcher_open());
    ui.set_switcher_query(app.switcher_query().into());
    ui.set_switcher_rows(ModelRc::new(VecModel::from(
        app.switcher_rows()
            .into_iter()
            .map(|row| SwitcherRow {
                name: row.name.into(),
                path: row.path.into(),
                branch: row.branch.into(),
                marker: row.marker.into(),
                marker_tooltip: row.marker_tooltip.into(),
                marker_warning: row.marker_warning,
                selected: row.selected,
            })
            .collect::<Vec<_>>(),
    )));
}

/// Copies the File pane's own preview of the selected row - its graphic,
/// views, coloured lines, plain text, fact table and README (#584) - into
/// `ui`'s bound properties.
/// Copies `id`'s own pinned state (#619) into `ui`'s bound properties - the
/// pinned counterpart of [`sync_ui`], since a pinned window only ever shows
/// the File pane and never the shared selection: no folders, contents,
/// filter, switcher or Repos Directory message to draw.
///
/// Public, like `sync_ui`, so `main`'s timer - which does not otherwise
/// need to know a pinned window from a live one - can sync every open
/// window the same way: ask [`PaneWindows::windows_with_pin`] and
/// [`app::App::is_pinned`] which function a given window needs.
pub fn sync_pinned_window(ui: &MainWindow, app: &App, id: app::PinId) {
    let graphic = app.pinned_file_graphic(id).as_ref().and_then(graphic_image);
    ui.set_file_has_graphic(graphic.is_some());
    ui.set_file_graphic(graphic.unwrap_or_default());
    ui.set_file_view_index(row_index(app.pinned_file_view_index(id)));
    ui.set_file_tabs(string_model(app.pinned_file_tabs(id)));
    ui.set_file_tab_index(row_index(app.pinned_file_tab_index(id)));
    ui.set_file_lines(ModelRc::new(VecModel::from(
        app.pinned_file_lines(id)
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
    ui.set_file_text(app.pinned_file_text(id).into());
    ui.set_file_facts(ModelRc::new(VecModel::from(
        app.pinned_file_facts(id)
            .into_iter()
            .map(|fact| FactRow {
                label: fact.label.into(),
                display_value: fact.display_value.into(),
                full_value: fact.full_value.into(),
                dim: fact.dim,
            })
            .collect::<Vec<_>>(),
    )));
    // A pinned window is always a file or folder's preview or edit, never
    // the working copy the shared selection happens to be browsing - so,
    // unlike the live File pane, it has no README section and no
    // "Worktree of"/"Submodule of" line to draw.
    ui.set_file_readme_name(SharedString::default());
    ui.set_file_readme_title(SharedString::default());
    ui.set_file_readme_excerpt(SharedString::default());
    ui.set_file_related_repository_label(SharedString::default());
    ui.set_file_related_repository_linked(false);
    ui.set_editing_file(app.pinned_editing_file(id));
    ui.set_can_edit(app.pinned_can_edit(id));
    ui.set_status_text(app.status_text().into());
    ui.set_focus_pane(2);
    if let Some(title) = app.pinned_title(id) {
        ui.set_active_tool_title(title.into());
    }
    ui.set_tool_showing_editor(true);
    ui.set_tool_titles(string_model(Vec::new()));
    ui.set_tool_index(0);
    sync_pinned_editor(ui, app, id);
}

/// The pinned counterpart of [`sync_editor`].
fn sync_pinned_editor(ui: &MainWindow, app: &App, id: app::PinId) {
    ui.set_editing_in_colour(app.pinned_editing_in_colour(id));
    ui.set_edit_modified(app.pinned_edit_modified(id));
    ui.set_edit_can_undo(app.pinned_edit_can_undo(id));
    ui.set_edit_can_redo(app.pinned_edit_can_redo(id));
    ui.set_edit_has_selection(app.pinned_edit_has_selection(id));
    let (line, column) = app.pinned_edit_position(id);
    ui.set_edit_line(row_index(line));
    ui.set_edit_column(row_index(column));
    if !app.pinned_editing_file(id) {
        return;
    }
    let text = app.pinned_edit_text(id);
    if ui.get_edit_text() != text.as_str() {
        ui.set_edit_text(text.into());
    }
    ui.set_edit_lines(ModelRc::new(VecModel::from(
        app.pinned_edit_lines(id)
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
    let (line, column) = app.pinned_edit_caret(id);
    ui.set_edit_caret_line(row_index(line));
    ui.set_edit_caret_column(row_index(column));
    ui.set_edit_longest_line(row_index(app.pinned_edit_longest_line(id)));
    match app.pinned_edit_selection(id) {
        Some(((start_line, start_column), (end_line, end_column))) => {
            ui.set_edit_selection_start_line(row_index(start_line));
            ui.set_edit_selection_start_column(row_index(start_column));
            ui.set_edit_selection_end_line(row_index(end_line));
            ui.set_edit_selection_end_column(row_index(end_column));
        }
        None => ui.set_edit_selection_start_line(-1),
    }
}

fn sync_file_pane(ui: &MainWindow, app: &App) {
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
    ui.set_file_readme_name(app.file_readme_name().into());
    ui.set_file_readme_title(app.file_readme_title().into());
    ui.set_file_readme_excerpt(app.file_readme_excerpt().into());
    ui.set_file_related_repository_label(app.file_related_repository_label().into());
    ui.set_file_related_repository_linked(app.file_related_repository_linked());
}

/// Copies `app`'s current state into `ui`'s bound properties.
pub fn sync_ui(ui: &MainWindow, app: &App) {
    ui.set_folder_rows(ModelRc::new(VecModel::from(
        app.folder_rows()
            .into_iter()
            .map(folder_row_view)
            .collect::<Vec<_>>(),
    )));
    let folder_moved = ui.get_folder_selected() != row_index(app.folder_selected());
    ui.set_folder_selected(row_index(app.folder_selected()));
    ui.set_content_rows(ModelRc::new(VecModel::from(
        app.content_rows()
            .into_iter()
            .enumerate()
            .map(|(index, row)| ContentRow {
                icon: icon_image(row.icon, row.is_dir, row.mark),
                is_repository: row.is_repository,
                name: row.name.into(),
                size: row.size.into(),
                kind: row.kind.into(),
                modified: row.modified.into(),
                branch: row.branch.into(),
                marker: row.marker.into(),
                marker_tooltip: row.marker_tooltip.into(),
                marker_warning: row.marker_warning,
                stale_marker: row.stale_marker.into(),
                stale_tooltip: row.stale_tooltip.into(),
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
    // The zoom-scaled row height (#586): scroll math has to agree with what
    // is actually drawn at the current zoom, not with the 100% row height.
    let row_height = ROW_HEIGHT * app.zoom_factor();
    if content_moved {
        ui.set_content_scroll_y(scroll_offset_for(
            app.content_selected(),
            ui.get_content_viewport_height(),
            ui.get_content_scroll_y(),
            row_height,
        ));
    }
    if folder_moved {
        ui.set_folders_scroll_y(scroll_offset_for(
            app.folder_selected(),
            ui.get_folders_viewport_height(),
            ui.get_folders_scroll_y(),
            row_height,
        ));
    }
    sync_file_pane(ui, app);
    ui.set_status_text(app.status_text().into());
    ui.global::<Zoom>()
        .set_percent(i32::from(app.zoom_percent()));
    sync_filter(ui, app);
    sync_contents_message(ui, app);
    sync_switcher(ui, app);
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
    ui.set_showing_all_repositories(app.showing_all_repositories());
    ui.set_all_repositories_icon(icon_image(app::icon_for("", true), true, None));
    ui.set_content_size_column_visible(app.content_size_column_visible());
    ui.set_content_holds_repository(app.content_holds_repository());
    ui.set_editing_file(app.editing_file());
    ui.set_can_edit(app.can_edit());
    ui.set_can_open(app.can_open());
    ui.set_web_provider(app.web_provider().unwrap_or_default().into());
    // Handing a selected folder to a program the user already has (#581).
    ui.set_editor_available(app.can_open_selected_in_editor());
    ui.set_content_remote_address_copyable(app.can_copy_selected_remote_address());
    ui.set_folder_editor_available(app.editor_available());
    ui.set_folder_remote_address_copyable(app.can_copy_folder_remote_address());
    ui.set_file_manager_label(app.file_manager_label().into());
    sync_editor(ui, app);
    ui.set_location_icon(icon_image(app::icon_for("", true), true, None));
    sync_tools(ui, app);
    sync_certificates(ui, app);
    // #721: GUIDANCE.md §2.3's macOS behaviour profile - Command in place
    // of Control for `key-scope`'s bindings, and the Trash-named delete
    // confirmation, both decided in Rust and read here as one flag.
    ui.set_mac_profile(app.mac_profile());
    ui.set_quick_look_open(app.quick_look_open());
    ui.set_quick_look_name(app.quick_look_name().into());
}

/// Copies the tool slot's picker state (#616) into `ui`'s bound properties:
/// the active tool's title, which its markup draws, and the titles the
/// picker offers - empty unless more than one tool applies, which is what
/// keeps it hidden while only the editor is registered.
fn sync_tools(ui: &MainWindow, app: &App) {
    ui.set_active_tool_title(app.active_tool_title().into());
    ui.set_tool_showing_editor(app.active_tool_is_editor());
    ui.set_tool_titles(string_model(app.tool_titles()));
    ui.set_tool_index(row_index(app.tool_index()));
}

/// Copies the Certificates tool's own state (#622) into `ui`'s bound
/// properties: whether it is the active tool, its table, the counts and
/// private keys line above it, and the current sort.
fn sync_certificates(ui: &MainWindow, app: &App) {
    ui.set_tool_showing_certificates(app.active_tool_is_certificates());
    ui.set_certificate_rows(ModelRc::new(VecModel::from(
        app.certificate_rows()
            .into_iter()
            .map(|row| CertificateRow {
                status: row.status.into(),
                status_warning: row.status_warning,
                subject: row.subject.into(),
                expires: row.expires.into(),
                repository: row.repository.into(),
                file: row.file.into(),
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_certificates_summary_line(app.certificates_summary_line().into());
    ui.set_certificates_private_key_line(app.certificates_private_key_line().into());
    ui.set_certificates_private_keys_shown(app.certificates_private_keys_shown());
    ui.set_certificates_private_key_files(string_model(app.certificates_private_key_files()));
    ui.set_certificates_loading(app.certificates_loading());
    ui.set_certificates_sort_column(app.certificates_sort_column());
    ui.set_certificates_sort_ascending(app.certificates_sort_ascending());
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
    use super::GeometryTracker;
    use crate::settings::WindowGeometry;

    const fn geometry(x: f32, maximized: bool) -> WindowGeometry {
        WindowGeometry {
            x,
            y: 10.0,
            width: 800.0,
            height: 600.0,
            maximized,
        }
    }

    #[test]
    fn a_window_that_opens_maximised_still_remembers_normal_bounds_to_return_to() {
        let tracker = GeometryTracker::opening_at(Some(geometry(100.0, true)));
        assert_eq!(tracker.closing_at(true), Some(geometry(100.0, true)));
        assert_eq!(tracker.closing_at(false), Some(geometry(100.0, false)));
    }

    #[test]
    fn correcting_replaces_bounds_that_named_a_display_that_is_gone() {
        // The review of #624: a window left maximised on a monitor since
        // unplugged never had its normal bounds corrected, so un-maximising
        // put it off-screen and saving put the same bounds back.
        let mut tracker = GeometryTracker::opening_at(Some(geometry(-4000.0, true)));
        tracker.corrected_to(geometry(60.0, true));
        assert_eq!(tracker.closing_at(true), Some(geometry(60.0, true)));
    }

    #[test]
    fn leaving_maximised_asks_once_for_the_window_to_be_settled() {
        let mut tracker = GeometryTracker::opening_at(Some(geometry(100.0, true)));

        assert!(
            !tracker.observed(None, true),
            "still maximised: nothing to settle"
        );
        assert!(
            tracker.observed(Some(geometry(-4000.0, false)), false),
            "just un-maximised: the platform chose these bounds, so check them"
        );
        assert!(
            !tracker.observed(Some(geometry(-4000.0, false)), false),
            "asked once, not on every tick afterwards"
        );
        assert!(
            !tracker.observed(None, true),
            "maximising again asks nothing"
        );
        assert!(tracker.observed(Some(geometry(50.0, false)), false));
    }

    #[test]
    fn a_window_that_never_maximises_is_never_asked_to_settle() {
        let mut tracker = GeometryTracker::opening_at(Some(geometry(100.0, false)));
        assert!(!tracker.observed(Some(geometry(100.0, false)), false));
        assert!(!tracker.observed(Some(geometry(120.0, false)), false));
        assert_eq!(tracker.closing_at(false), Some(geometry(120.0, false)));
    }

    #[test]
    fn with_nothing_remembered_there_is_nothing_to_save_until_the_window_reports_bounds() {
        let mut tracker = GeometryTracker::opening_at(None);
        assert_eq!(tracker.closing_at(false), None);
        tracker.observed(Some(geometry(30.0, false)), false);
        assert_eq!(tracker.closing_at(false), Some(geometry(30.0, false)));
    }

    use super::{
        MIN_CONTENTS_WIDTH, MIN_FOLDERS_WIDTH, ROW_HEIGHT, RepositoryMark, fit_pane_widths,
        icon_svg, scroll_offset_for, visible_rows,
    };
    use crate::app::icon_for;

    /// A plain folder's icon is unchanged by #579: nothing this project
    /// draws for the common case - a folder with no repository below it -
    /// should move.
    #[test]
    fn a_plain_folder_carries_no_badge() {
        let plain = icon_svg(icon_for("src", true), true, None);
        assert!(
            !plain.contains("circle"),
            "a plain folder should draw no badge: {plain}"
        );
    }

    /// Each known provider, one with no remote, and a plain folder all draw
    /// a distinct icon: the acceptance check for #579.
    #[test]
    fn a_working_copy_draws_a_badge_that_differs_by_provider() {
        let folder = icon_for("src", true);
        let plain = icon_svg(folder, true, None);
        let marks = [
            RepositoryMark::GitHub,
            RepositoryMark::GitLab,
            RepositoryMark::Bitbucket,
            RepositoryMark::AzureDevOps,
            RepositoryMark::Generic,
        ];
        let mut drawn: Vec<String> = marks
            .into_iter()
            .map(|mark| icon_svg(folder, true, Some(mark)))
            .collect();
        drawn.push(plain);
        for (index, icon) in drawn.iter().enumerate() {
            for (other_index, other) in drawn.iter().enumerate() {
                assert!(
                    index == other_index || icon != other,
                    "icon {index} and icon {other_index} should differ"
                );
            }
        }
    }

    #[test]
    fn the_rows_on_screen_are_the_ones_any_part_of_which_shows() {
        let viewport = 5.0 * ROW_HEIGHT;
        assert_eq!(visible_rows(0.0, viewport, 100, ROW_HEIGHT), 0..5);
        assert_eq!(
            visible_rows(0.0, viewport, 3, ROW_HEIGHT),
            0..3,
            "a short listing"
        );
        assert_eq!(
            visible_rows(-ROW_HEIGHT / 2.0, viewport, 100, ROW_HEIGHT),
            0..6,
            "half a row scrolled off the top, half of another on at the bottom"
        );
        assert_eq!(
            visible_rows(-40.0 * ROW_HEIGHT, viewport, 100, ROW_HEIGHT),
            40..45
        );
        assert_eq!(
            visible_rows(0.0, 0.0, 100, ROW_HEIGHT),
            0..0,
            "not laid out yet"
        );
    }

    /// #586: at zoom below 100% a row is shorter than `ROW_HEIGHT`, so more
    /// of them fit the same viewport - the same five rows' worth of pixels
    /// now hold eight rows at half height.
    #[test]
    fn zoomed_out_rows_are_shorter_so_more_of_them_are_visible() {
        let viewport = 5.0 * ROW_HEIGHT;
        let half_height = ROW_HEIGHT / 2.0;
        assert_eq!(visible_rows(0.0, viewport, 100, half_height), 0..10);
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
            assert!(same(
                scroll_offset_for(selected, VIEWPORT, 0.0, ROW_HEIGHT),
                0.0
            ));
        }
    }

    #[test]
    fn a_row_below_the_fold_comes_to_the_bottom_edge() {
        // Row 10 is one past the last visible row, so the pane moves by
        // exactly one row - not by a page.
        assert!(same(
            scroll_offset_for(10, VIEWPORT, 0.0, ROW_HEIGHT),
            -ROW_HEIGHT
        ));
        assert!(same(
            scroll_offset_for(11, VIEWPORT, 0.0, ROW_HEIGHT),
            -2.0 * ROW_HEIGHT
        ));
    }

    #[test]
    fn a_row_far_below_puts_that_row_last() {
        let offset = scroll_offset_for(199, VIEWPORT, 0.0, ROW_HEIGHT);

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

        let offset = scroll_offset_for(40, VIEWPORT, scrolled, ROW_HEIGHT);

        assert!(same(offset, -40.0 * ROW_HEIGHT), "the row sits at the top");
    }

    #[test]
    fn the_first_row_scrolls_the_listing_home() {
        assert!(same(
            scroll_offset_for(0, VIEWPORT, -100.0 * ROW_HEIGHT, ROW_HEIGHT),
            0.0
        ));
    }

    #[test]
    fn a_pane_that_has_not_been_laid_out_is_left_alone() {
        // Before the first layout the viewport has no height, and an
        // arithmetic answer from that would scroll the listing off screen.
        assert!(same(scroll_offset_for(50, 0.0, -20.0, ROW_HEIGHT), -20.0));
    }

    #[test]
    fn moving_one_row_at_a_time_scrolls_one_row_at_a_time() {
        // Walking down past the fold: each step moves the pane by exactly a
        // row, so the selected row stays at the bottom edge rather than the
        // view jumping ahead of the reader.
        let mut offset = 0.0;
        for selected in 0..30 {
            offset = scroll_offset_for(selected, VIEWPORT, offset, ROW_HEIGHT);
        }

        assert!(same(offset, -20.0 * ROW_HEIGHT));
    }

    /// #586: doubling the row height at 200% zoom halves how many rows fit
    /// the same viewport - a row 10 rows down the doubled listing is only 5
    /// rows' worth of pixels below the top.
    #[test]
    fn a_zoomed_in_row_below_the_fold_moves_by_its_doubled_height() {
        let doubled = 2.0 * ROW_HEIGHT;
        assert!(same(scroll_offset_for(5, VIEWPORT, 0.0, doubled), -doubled));
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
