//! The window's text zoom (#586): the steps a reader can land on, and how
//! `App` moves between them.
//!
//! Kept apart from `settings.rs` (which only reads and writes `gui.json`)
//! and from `app.rs` (which holds the running window's state): this module
//! is the one place that knows what a "step" is, so the other two cannot
//! drift apart on what counts as a valid one.

/// The zoom levels a reader can land on, as percentages of the base size.
/// `100` draws exactly what the window drew before #586.
pub const STEPS: [u16; 8] = [80, 90, 100, 110, 125, 150, 175, 200];

/// The zoom a window opens at with no remembered or requested level.
pub const DEFAULT: u16 = 100;

/// `percent` if it is one of [`STEPS`], the same all-or-nothing shape
/// `settings::width_field` gives a corrupt pane width: a level between two
/// real steps would mean a reader could never step cleanly away from it.
#[must_use]
pub fn valid_step(percent: u16) -> Option<u16> {
    STEPS.contains(&percent).then_some(percent)
}

/// The step above `percent`, or `percent` unchanged if it is already the
/// last one - zooming in at the top of [`STEPS`] stays there rather than
/// wrapping around to the bottom.
#[must_use]
pub fn step_in(percent: u16) -> u16 {
    STEPS
        .iter()
        .find(|&&step| step > percent)
        .copied()
        .unwrap_or(percent)
}

/// The step below `percent`, the mirror of [`step_in`].
#[must_use]
pub fn step_out(percent: u16) -> u16 {
    STEPS
        .iter()
        .rev()
        .find(|&&step| step < percent)
        .copied()
        .unwrap_or(percent)
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT, STEPS, step_in, step_out, valid_step};

    #[test]
    fn stepping_in_walks_up_the_table() {
        assert_eq!(step_in(80), 90);
        assert_eq!(step_in(100), 110);
        assert_eq!(step_in(175), 200);
    }

    #[test]
    fn stepping_in_at_the_top_stays_there() {
        assert_eq!(step_in(200), 200);
    }

    #[test]
    fn stepping_out_walks_down_the_table() {
        assert_eq!(step_out(200), 175);
        assert_eq!(step_out(110), 100);
        assert_eq!(step_out(90), 80);
    }

    #[test]
    fn stepping_out_at_the_bottom_stays_there() {
        assert_eq!(step_out(80), 80);
    }

    #[test]
    fn every_step_is_valid() {
        for step in STEPS {
            assert_eq!(valid_step(step), Some(step));
        }
    }

    #[test]
    fn a_level_between_two_steps_is_invalid() {
        assert_eq!(valid_step(95), None);
        assert_eq!(valid_step(0), None);
        assert_eq!(valid_step(1000), None);
    }

    #[test]
    fn the_default_is_the_unzoomed_step() {
        assert_eq!(DEFAULT, 100);
        assert_eq!(valid_step(DEFAULT), Some(DEFAULT));
    }
}
