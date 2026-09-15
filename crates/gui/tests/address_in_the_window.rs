//! The address bar and the navigation chrome, driven through a real window
//! standing on a real `App`.
//!
//! Every test here builds a `MainWindow`, builds an `App`, joins the two
//! with the same callbacks `main.rs` joins them with, and then acts on the
//! window only - a dispatched pointer or key event, never a call straight
//! into `App`. The crumb strip, the elision of a long path, the path field
//! and the Back/Forward/Up buttons each have a Slint half and a Rust half,
//! and each half already has tests of its own; what neither can see is the
//! join, which is what these measure.
//!
//! Where a test needs to know where something is on screen it asks the
//! drawn element, through `ElementHandle`, rather than working the position
//! out from a number copied from `app.slint`. A copied number that drifts
//! from the layout reports passes and failures that are about the copy.

use gui::app::App;
use gui::{MainWindow, sync_ui};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::path::PathBuf;
use std::rc::Rc;

/// The leading segments of a long path collapse behind this one.
const ELLIPSIS: &str = "\u{2026}";

/// `max-crumbs` in `app.slint`: how many segments of a long path the bar
/// keeps. Not a measurement - it is the design number the elision is
/// written around, and the thing these tests are checking the bar honours.
const MAX_CRUMBS: usize = 5;

/// The toolbar glyphs, as `app.slint` draws them.
const BACK: &str = "\u{2190}";
const FORWARD: &str = "\u{2192}";
const UP: &str = "\u{2191}";

/// A window, an `App`, and the address-bar callbacks joined the way
/// `main.rs` joins them.
///
/// The counters exist to tell two different failures apart: a button that
/// refuses a click because it is drawn greyed out never reaches its
/// callback, and a button that accepts one and then has nothing to do
/// reaches it and changes nothing.
struct Chrome {
    ui: MainWindow,
    app: Rc<RefCell<App>>,
    back: Rc<Cell<u32>>,
    forward: Rc<Cell<u32>>,
    parent: Rc<Cell<u32>>,
}

impl Chrome {
    /// Where the application actually is, as `App` reports it.
    fn current_path(&self) -> String {
        self.app.borrow().current_path()
    }

    /// The segments `App` says the bar should be showing.
    fn expected_crumbs(&self) -> Vec<String> {
        self.app.borrow().breadcrumbs()
    }
}

/// Builds a window over an `App` opened at `path` and wires the address bar
/// and navigation chrome exactly as `main.rs`'s `wire_callbacks` does.
fn chrome_at(path: &str) -> Chrome {
    i_slint_backend_testing::init_no_event_loop();
    let app = Rc::new(RefCell::new(App::new(PathBuf::from(path))));
    let ui = MainWindow::new().expect("the window should build");
    sync_ui(&ui, &app.borrow());

    let back = Rc::new(Cell::new(0));
    let forward = Rc::new(Cell::new(0));
    let parent = Rc::new(Cell::new(0));

    macro_rules! on_event {
        ($setter:ident, $method:ident) => {{
            let app = Rc::clone(&app);
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

    macro_rules! on_counted_event {
        ($setter:ident, $method:ident, $counter:expr) => {{
            let app = Rc::clone(&app);
            let ui_weak = ui.as_weak();
            let counter = Rc::clone(&$counter);
            ui.$setter(move || {
                counter.set(counter.get() + 1);
                let mut app = app.borrow_mut();
                app.$method();
                if let Some(ui) = ui_weak.upgrade() {
                    sync_ui(&ui, &app);
                }
            });
        }};
    }

    on_counted_event!(on_back_requested, go_back, back);
    on_counted_event!(on_forward_requested, go_forward, forward);
    on_counted_event!(on_parent_requested, navigate_to_parent, parent);
    on_event!(on_path_edit_requested, begin_path_edit);
    on_event!(on_return_pressed, handle_return);
    on_event!(on_backspace_pressed, backspace);
    on_event!(on_cancel_requested, cancel_pending);

    {
        let app = Rc::clone(&app);
        let ui_weak = ui.as_weak();
        ui.on_breadcrumb_requested(move |index| {
            let mut app = app.borrow_mut();
            app.navigate_to_breadcrumb(index);
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }
    {
        let app = Rc::clone(&app);
        let ui_weak = ui.as_weak();
        ui.on_key_text(move |text| {
            let mut app = app.borrow_mut();
            app.handle_key_text(&text);
            if let Some(ui) = ui_weak.upgrade() {
                sync_ui(&ui, &app);
            }
        });
    }

    ui.show().expect("the window should show");
    Chrome {
        ui,
        app,
        back,
        forward,
        parent,
    }
}

/// The label a drawn `Crumb` is carrying, taken from the `Text` inside it.
fn crumb_label(crumb: &ElementHandle) -> String {
    crumb
        .query_descendants()
        .match_predicate(|element| {
            element
                .accessible_label()
                .is_some_and(|label| !label.is_empty())
        })
        .find_first()
        .and_then(|element| element.accessible_label())
        .map_or_else(String::new, |label| label.to_string())
}

/// Every `Crumb` the window is actually drawing, left to right - the
/// ellipsis included, since it is a crumb too.
fn drawn_crumbs(ui: &MainWindow) -> Vec<String> {
    let mut found: Vec<(f32, String)> = ElementHandle::find_by_element_type_name(ui, "Crumb")
        .map(|crumb| (crumb.absolute_position().x, crumb_label(&crumb)))
        .collect();
    found.sort_by(|left, right| left.0.partial_cmp(&right.0).unwrap_or(Ordering::Equal));
    found.into_iter().map(|(_, label)| label).collect()
}

/// The drawn `Crumb` carrying `label`, for clicking.
fn crumb(ui: &MainWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_element_type_name(ui, "Crumb")
        .find(|crumb| crumb_label(crumb) == label)
        .unwrap_or_else(|| panic!("the bar should be drawing a crumb labelled {label:?}"))
}

/// Clicks the toolbar button carrying `glyph`.
fn click_nav(ui: &MainWindow, glyph: &str) {
    ElementHandle::find_by_accessible_label(ui, glyph)
        .next()
        .unwrap_or_else(|| panic!("the toolbar should carry a {glyph:?} button"))
        .mock_single_click(PointerEventButton::Left);
}

/// Presses `text` as a key, optionally with a modifier held. Slint tracks
/// modifier state from the modifier key's own press, so holding one means
/// pressing and releasing it around the key itself.
fn press(ui: &MainWindow, text: &str, modifier: Option<Key>) {
    let window = ui.window();
    if let Some(modifier) = modifier {
        window.dispatch_event(WindowEvent::KeyPressed {
            text: char::from(modifier).into(),
        });
    }
    window.dispatch_event(WindowEvent::KeyPressed { text: text.into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: text.into() });
    if let Some(modifier) = modifier {
        window.dispatch_event(WindowEvent::KeyReleased {
            text: char::from(modifier).into(),
        });
    }
}

/// Types `text` into the window one character at a time.
fn type_text(ui: &MainWindow, text: &str) {
    for character in text.chars() {
        press(ui, &character.to_string(), None);
    }
}

/// Empties the open path field the way a reader would, one Backspace per
/// character it is holding.
fn clear_path_field(ui: &MainWindow) {
    for _ in 0..ui.get_path_input().chars().count() {
        press(ui, &char::from(Key::Backspace).to_string(), None);
    }
}

/// Clicks inside the address bar to the right of its last crumb, which is
/// the empty run of bar that Explorer turns into a text field.
fn click_past_the_path(ui: &MainWindow) {
    let last = ElementHandle::find_by_element_type_name(ui, "Crumb")
        .max_by(|left, right| {
            left.absolute_position()
                .x
                .partial_cmp(&right.absolute_position().x)
                .unwrap_or(Ordering::Equal)
        })
        .expect("the bar should be drawing at least one crumb");
    let window = ui.window();
    let bar_right = window.size().to_logical(window.scale_factor()).width;
    let right_edge = last.absolute_position().x + last.size().width;
    assert!(
        right_edge < bar_right,
        "the path should leave some bar to click past"
    );
    let position = LogicalPosition::new(
        f32::midpoint(right_edge, bar_right),
        last.absolute_position().y + last.size().height / 2.0,
    );
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

#[test]
fn the_address_bar_draws_the_folder_the_application_is_in() {
    let chrome = chrome_at("/one/two/three");

    let drawn = drawn_crumbs(&chrome.ui);

    assert_eq!(
        drawn,
        chrome.expected_crumbs(),
        "the bar should be drawing the segments App says it is standing on"
    );
    assert!(
        drawn.len() <= MAX_CRUMBS,
        "a short path is not elided: {drawn:?}"
    );
}

#[test]
fn clicking_a_crumb_browses_to_that_folder_and_the_bar_follows() {
    let chrome = chrome_at("/one/two/three");
    let target = chrome.expected_crumbs()[1].clone();

    crumb(&chrome.ui, &target).mock_single_click(PointerEventButton::Left);

    assert!(
        chrome.current_path().ends_with(&target),
        "clicking {target:?} should browse there, not to {:?}",
        chrome.current_path()
    );
    assert_eq!(
        drawn_crumbs(&chrome.ui).last().cloned(),
        Some(target.clone()),
        "and the bar should end at the segment that was clicked"
    );
    assert_eq!(
        drawn_crumbs(&chrome.ui),
        chrome.expected_crumbs(),
        "the whole bar should say where the application now is"
    );
}

#[test]
fn a_path_longer_than_the_bar_keeps_only_its_last_segments() {
    let chrome = chrome_at("/a/bb/ccc/dddd/eeeee/ffffff/ggggggg");
    let full = chrome.expected_crumbs();
    assert!(
        full.len() > MAX_CRUMBS,
        "the fixture has to be longer than the bar keeps: {full:?}"
    );

    let drawn = drawn_crumbs(&chrome.ui);

    let segments: Vec<String> = drawn
        .iter()
        .filter(|label| *label != ELLIPSIS)
        .cloned()
        .collect();
    assert!(
        drawn.contains(&ELLIPSIS.to_owned()),
        "the collapsed head should be drawn as an ellipsis: {drawn:?}"
    );
    assert_eq!(
        segments,
        full[full.len() - MAX_CRUMBS..],
        "the bar keeps the tail, which is the part that says where you are"
    );
}

#[test]
fn clicking_the_ellipsis_puts_the_whole_path_back() {
    let chrome = chrome_at("/a/bb/ccc/dddd/eeeee/ffffff/ggggggg");
    let full = chrome.expected_crumbs();
    let before = chrome.current_path();

    crumb(&chrome.ui, ELLIPSIS).mock_single_click(PointerEventButton::Left);

    assert_eq!(
        drawn_crumbs(&chrome.ui),
        full,
        "the ellipsis expands the path in place rather than hiding it in a menu"
    );
    assert_eq!(
        chrome.current_path(),
        before,
        "and expanding is not a navigation"
    );
}

#[test]
fn a_path_elided_once_is_elided_again_after_navigating() {
    let chrome = chrome_at("/a/bb/ccc/dddd/eeeee/ffffff/ggggggg");
    let full = chrome.expected_crumbs();
    crumb(&chrome.ui, ELLIPSIS).mock_single_click(PointerEventButton::Left);
    assert_eq!(drawn_crumbs(&chrome.ui), full, "expanded to start with");

    // Somewhere else, still too long for the bar: the one before last.
    let target = full[full.len() - 2].clone();
    crumb(&chrome.ui, &target).mock_single_click(PointerEventButton::Left);

    let now = chrome.expected_crumbs();
    assert!(
        now.len() > MAX_CRUMBS,
        "the folder navigated to is still longer than the bar keeps: {now:?}"
    );
    let drawn = drawn_crumbs(&chrome.ui);
    assert!(
        drawn.contains(&ELLIPSIS.to_owned()),
        "a fresh path starts collapsed again, or the bar can never be put back: {drawn:?}"
    );
}

#[test]
fn clicking_a_crumb_of_an_elided_path_goes_to_that_crumbs_folder() {
    let chrome = chrome_at("/a/bb/ccc/dddd/eeeee/ffffff/ggggggg");
    let full = chrome.expected_crumbs();
    // The first segment still drawn once the head has collapsed: the one
    // whose index in the model is furthest from its position on screen.
    let target = full[full.len() - MAX_CRUMBS].clone();

    crumb(&chrome.ui, &target).mock_single_click(PointerEventButton::Left);

    assert!(
        chrome.current_path().ends_with(&target),
        "clicking {target:?} in an elided bar should browse there, not to {:?}",
        chrome.current_path()
    );
    assert_eq!(
        drawn_crumbs(&chrome.ui).last().cloned(),
        Some(target),
        "and the bar should end at the segment that was clicked"
    );
}

#[test]
fn clicking_the_last_crumb_opens_the_path_field() {
    let chrome = chrome_at("/one/two/three");
    let last = chrome.expected_crumbs().last().cloned().expect("a path");

    crumb(&chrome.ui, &last).mock_single_click(PointerEventButton::Left);

    assert!(
        chrome.ui.get_editing_path(),
        "the segment you are already on is not a place to go, so the click \
         falls through to the bar and starts typing"
    );
    assert_eq!(
        chrome.ui.get_path_input().to_string(),
        chrome.current_path(),
        "the field starts as the path you are on"
    );
}

#[test]
fn clicking_past_the_last_crumb_opens_the_path_field() {
    let chrome = chrome_at("/one/two/three");

    click_past_the_path(&chrome.ui);

    assert!(
        chrome.ui.get_editing_path(),
        "a click on the empty run of bar turns it into a text field"
    );
    assert_eq!(
        chrome.ui.get_path_input().to_string(),
        chrome.current_path(),
        "the field starts as the path you are on"
    );
}

#[test]
fn ctrl_l_opens_the_path_field_holding_where_you_are() {
    let chrome = chrome_at("/one/two/three");

    press(&chrome.ui, "l", Some(Key::Control));

    assert!(chrome.ui.get_editing_path(), "Ctrl+L opens the address bar");
    assert_eq!(
        chrome.ui.get_path_input().to_string(),
        chrome.current_path()
    );
}

#[test]
fn f4_opens_the_path_field_holding_where_you_are() {
    let chrome = chrome_at("/one/two/three");

    press(&chrome.ui, &char::from(Key::F4).to_string(), None);

    assert!(chrome.ui.get_editing_path(), "F4 opens the address bar");
    assert_eq!(
        chrome.ui.get_path_input().to_string(),
        chrome.current_path()
    );
}

#[test]
fn typing_a_path_and_pressing_enter_goes_there() {
    let chrome = chrome_at("/one/two/three");
    press(&chrome.ui, "l", Some(Key::Control));
    clear_path_field(&chrome.ui);
    assert_eq!(
        chrome.ui.get_path_input().to_string(),
        "",
        "the field should empty as it is backspaced"
    );

    type_text(&chrome.ui, "/four/five");
    assert_eq!(
        chrome.ui.get_path_input().to_string(),
        "/four/five",
        "what was typed should reach the field"
    );
    press(&chrome.ui, &char::from(Key::Return).to_string(), None);

    assert!(
        !chrome.ui.get_editing_path(),
        "Enter closes the field again"
    );
    assert_eq!(
        drawn_crumbs(&chrome.ui).last().cloned(),
        Some("five".to_owned()),
        "and the bar says where the typed path took us"
    );
    assert_eq!(drawn_crumbs(&chrome.ui), chrome.expected_crumbs());
}

#[test]
fn escape_leaves_the_typed_path_and_the_folder_alone() {
    let chrome = chrome_at("/one/two/three");
    let before = drawn_crumbs(&chrome.ui);
    press(&chrome.ui, "l", Some(Key::Control));
    type_text(&chrome.ui, "junk");

    press(&chrome.ui, &char::from(Key::Escape).to_string(), None);

    assert!(!chrome.ui.get_editing_path(), "Escape closes the field");
    assert_eq!(
        drawn_crumbs(&chrome.ui),
        before,
        "and leaves the bar where it was"
    );
}

#[test]
fn the_parent_button_and_then_back_return_the_bar_to_where_it_started() {
    let chrome = chrome_at("/one/two/three");
    let start = drawn_crumbs(&chrome.ui);

    click_nav(&chrome.ui, UP);
    let after_up = drawn_crumbs(&chrome.ui);
    assert_eq!(
        after_up.last().map(String::as_str),
        Some("two"),
        "Up moves the bar one segment off the end"
    );

    click_nav(&chrome.ui, BACK);
    assert_eq!(drawn_crumbs(&chrome.ui), start, "Back returns the bar");

    click_nav(&chrome.ui, FORWARD);
    assert_eq!(
        drawn_crumbs(&chrome.ui),
        after_up,
        "and Forward takes it out again"
    );
}

#[test]
fn alt_left_and_alt_right_walk_the_history_the_buttons_walk() {
    let chrome = chrome_at("/one/two/three");
    let start = drawn_crumbs(&chrome.ui);

    press(
        &chrome.ui,
        &char::from(Key::UpArrow).to_string(),
        Some(Key::Alt),
    );
    let after_up = drawn_crumbs(&chrome.ui);
    assert_eq!(
        after_up.last().map(String::as_str),
        Some("two"),
        "Alt+Up is the Up button"
    );

    press(
        &chrome.ui,
        &char::from(Key::LeftArrow).to_string(),
        Some(Key::Alt),
    );
    assert_eq!(drawn_crumbs(&chrome.ui), start, "Alt+Left is Back");

    press(
        &chrome.ui,
        &char::from(Key::RightArrow).to_string(),
        Some(Key::Alt),
    );
    assert_eq!(drawn_crumbs(&chrome.ui), after_up, "Alt+Right is Forward");
}

#[test]
fn back_and_forward_are_refused_until_there_is_history() {
    let chrome = chrome_at("/one/two/three");

    click_nav(&chrome.ui, BACK);
    click_nav(&chrome.ui, FORWARD);

    assert_eq!(
        (chrome.back.get(), chrome.forward.get()),
        (0, 0),
        "with nowhere to go the buttons are drawn greyed out and take no click"
    );

    click_nav(&chrome.ui, UP);
    click_nav(&chrome.ui, BACK);

    assert_eq!(chrome.back.get(), 1, "once there is history Back takes one");
    assert_eq!(
        drawn_crumbs(&chrome.ui).last().map(String::as_str),
        Some("three")
    );
}

#[test]
fn navigating_while_the_path_field_is_open_leaves_the_bar_saying_where_you_are() {
    let chrome = chrome_at("/one/two/three");
    press(&chrome.ui, "l", Some(Key::Control));
    assert!(
        chrome.ui.get_editing_path(),
        "the field is open to type into"
    );

    click_nav(&chrome.ui, UP);

    assert!(
        chrome.current_path().ends_with("two"),
        "Up moves the application even with the field open: {:?}",
        chrome.current_path()
    );
    let showing = chrome.ui.get_path_input();
    assert!(
        !chrome.ui.get_editing_path() || showing == chrome.current_path(),
        "the address bar still reads {showing:?} while the application is at {:?}",
        chrome.current_path()
    );
}

#[test]
fn the_up_button_is_refused_where_it_cannot_go() {
    // The top of the tree: there is no folder above this one.
    let chrome = chrome_at("/");
    let start = drawn_crumbs(&chrome.ui);

    click_nav(&chrome.ui, UP);

    assert_eq!(
        drawn_crumbs(&chrome.ui),
        start,
        "there is nowhere above the root, so nothing moves"
    );
    assert_eq!(
        chrome.parent.get(),
        0,
        "and a button that can do nothing should be drawn greyed out and \
         refuse the click, the way Back and Forward do"
    );
}

/// A modifier pressed on its own is not a character.
///
/// Slint reports Shift, Control and the other held keys as key presses
/// whose text is a control character, and anything the window's key
/// handler does not recognise falls through to the typed-text path. So
/// reaching for Ctrl+V to paste into the address bar typed a `\u{11}` in
/// front of the path, and Enter then went nowhere.
#[test]
fn a_modifier_pressed_on_its_own_types_nothing_into_the_path_field() {
    let chrome = chrome_at("/one/two/three");
    press(&chrome.ui, "l", Some(Key::Control));
    clear_path_field(&chrome.ui);
    type_text(&chrome.ui, "ab");

    for modifier in [Key::Shift, Key::Control, Key::Alt, Key::Meta] {
        press(&chrome.ui, &char::from(modifier).to_string(), None);
    }

    assert_eq!(
        chrome.ui.get_path_input().to_string(),
        "ab",
        "a held key is not text"
    );
}

/// Typing a capital holds Shift first, and only the letter belongs in the
/// field.
#[test]
fn a_capital_typed_into_the_path_field_is_only_the_letter() {
    let chrome = chrome_at("/one/two/three");
    press(&chrome.ui, "l", Some(Key::Control));
    clear_path_field(&chrome.ui);

    press(&chrome.ui, "Z", Some(Key::Shift));

    assert_eq!(chrome.ui.get_path_input().to_string(), "Z");
}
