//! The chrome's own shape: that the command bar's groups are divided, and
//! that the divisions are visible rather than nominally present.
//!
//! The dividers existed before this and were a one-pixel rule at 60% height
//! in the border colour, on a panel of nearly the same colour, in a
//! nine-pixel slot. Eleven buttons read as eleven buttons. A test that only
//! counted them would have passed then too, so these check the geometry a
//! reader actually sees: a rule with width, with height, and with room
//! around it.

use gui::MainWindow;
use i_slint_backend_testing::ElementHandle;
use slint::ComponentHandle;

/// Divider width in `app.slint`, which is the room each group gets between
/// itself and the next.
const DIVIDER_SLOT: f32 = 15.0;

/// How many groups the command bar has: new, clipboard, file operations,
/// editing, history. One divider fewer than that, plus the one in the
/// address bar between the history buttons and up.
const COMMAND_BAR_DIVIDERS: usize = 4;
const ADDRESS_BAR_DIVIDERS: usize = 1;

/// Wide enough to span a bar rather than to be some incidental hairline:
/// every other rectangle this size in the tree sits inside a pane, which is
/// narrower than the window the bars run across.
const SPANS_A_BAR: f32 = 200.0;

fn shown_window() -> MainWindow {
    let ui = MainWindow::new().expect("the window should build");
    ui.show().expect("the window should show");
    ui
}

/// Every group divider in the window.
fn dividers(ui: &MainWindow) -> Vec<ElementHandle> {
    ElementHandle::find_by_element_id(ui, "CommandSeparator::rule").collect()
}

#[test]
fn the_command_bar_divides_its_groups() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();

    assert_eq!(
        dividers(&ui).len(),
        COMMAND_BAR_DIVIDERS + ADDRESS_BAR_DIVIDERS,
        "one divider between each pair of groups, and one in the address bar"
    );
}

#[test]
fn every_divider_is_visible_rather_than_nominal() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();

    for divider in dividers(&ui) {
        let size = divider.size();
        assert!(
            size.width > 0.0,
            "a divider with no width divides nothing: {size:?}"
        );
        assert!(
            size.height >= 10.0,
            "a divider has to span most of the bar to read as one: {size:?}"
        );
    }
}

#[test]
fn a_divider_has_room_around_it() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();

    // The rule is one pixel inside a slot several times wider, which is the
    // white space that makes a group read as a group rather than as a fence
    // between two buttons.
    for divider in dividers(&ui) {
        assert!(
            divider.size().width * 4.0 < DIVIDER_SLOT,
            "the rule should be a fraction of its slot, leaving room either side"
        );
    }
}

#[test]
fn the_bars_are_edged_off_from_what_is_below_them() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = shown_window();

    // The menu bar and the command bar each carry a hairline along their
    // bottom, so the chrome is a block and the panes start where it ends.
    // Both are one pixel tall and as wide as the window, which is what
    // distinguishes them from every other rectangle in the tree.
    let edges = ElementHandle::find_by_element_id(&ui, "ChromeEdge::rule")
        .filter(|edge| {
            let size = edge.size();
            (size.height - 1.0).abs() < f32::EPSILON && size.width > SPANS_A_BAR
        })
        .count();

    assert_eq!(
        edges, 2,
        "the menu bar and the command bar each end in a visible edge"
    );
}
