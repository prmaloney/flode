//! Gain conversion.

use super::frame::Frame;
use nih_plug::util;

/// A level at or below this many dB is exactly silent.
pub const SILENT_DB: f32 = -30.0;

/// The linear gain for a level in dB. A level at the floor is exactly silent.
pub fn db_to_linear(db: f32) -> f32 {
    if db <= SILENT_DB {
        0.0
    } else {
        util::db_to_gain_fast(db)
    }
}

/// The Gain node's minimum, in dB. At this value the node is exactly silent. It sits lower than
/// `SILENT_DB` because the node also boosts, so its knob needs room to cut.
pub const UTILITY_MIN_DB: f32 = -60.0;

/// The linear gain for a Gain node's level in dB. A level at the minimum is exactly silent.
pub fn utility_db_to_linear(db: f32) -> f32 {
    if db <= UTILITY_MIN_DB {
        0.0
    } else {
        util::db_to_gain_fast(db)
    }
}

/// The two multipliers of a balance control, left then right. `pan` runs from -1.0 (left) to 1.0
/// (right). This is a linear balance law. At the center both sides pass at unity. Moving toward
/// one side ramps the other side down to silence at the end, and the near side stays at unity.
/// It leaves a stereo image where it was, unlike a panner, which would sum the sides.
pub fn balance(pan: f32) -> (f32, f32) {
    let pan = pan.clamp(-1.0, 1.0);
    ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0))
}

/// The Gain node's signal path: a level, a balance, and a polarity flip. Each of the three eases
/// toward its target one step per sample, so a move of a knob or a click on Invert never jumps.
///
/// The polarity is a sign that eases between -1.0 and 1.0. A flip then crosses zero over the same
/// few milliseconds as a knob move, and the signal fades through silence instead of snapping to
/// the opposite side.
#[derive(Clone, Copy)]
pub struct Utility {
    gain: f32,
    pan: f32,
    sign: f32,
}

impl Utility {
    /// A path already at its targets, so it does not ease in from silence.
    pub fn new(gain_db: f32, pan: f32, invert: bool) -> Self {
        Self {
            gain: utility_db_to_linear(gain_db),
            pan: pan.clamp(-1.0, 1.0),
            sign: Self::sign_of(invert),
        }
    }

    fn sign_of(invert: bool) -> f32 {
        if invert {
            -1.0
        } else {
            1.0
        }
    }

    /// One frame through the path. Each setting moves a fraction `smooth` of the way to its
    /// target first.
    pub fn tick(
        &mut self,
        input: Frame,
        gain_db: f32,
        pan: f32,
        invert: bool,
        smooth: f32,
    ) -> Frame {
        self.gain += (utility_db_to_linear(gain_db) - self.gain) * smooth;
        self.pan += (pan.clamp(-1.0, 1.0) - self.pan) * smooth;
        self.sign += (Self::sign_of(invert) - self.sign) * smooth;
        let (left, right) = balance(self.pan);
        let level = self.gain * self.sign;
        Frame::new(input.l * level * left, input.r * level * right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMOOTH: f32 = 0.01;

    /// Runs `ticks` frames of a steady input through the path and returns the last output.
    fn settle(path: &mut Utility, gain_db: f32, pan: f32, invert: bool, ticks: usize) -> Frame {
        let mut out = Frame::SILENT;
        for _ in 0..ticks {
            out = path.tick(Frame::mono(1.0), gain_db, pan, invert, SMOOTH);
        }
        out
    }

    #[test]
    fn the_utility_range_is_silent_at_the_minimum() {
        assert_eq!(utility_db_to_linear(UTILITY_MIN_DB), 0.0);
        assert_eq!(utility_db_to_linear(-90.0), 0.0);
        assert!(utility_db_to_linear(-59.0) > 0.0);
        assert!((utility_db_to_linear(0.0) - 1.0).abs() < 1e-6);
        assert!((utility_db_to_linear(6.0) - 1.995).abs() < 0.01);
        assert!((utility_db_to_linear(24.0) - 15.85).abs() < 0.1);
    }

    #[test]
    fn the_balance_is_unity_at_the_center() {
        assert_eq!(balance(0.0), (1.0, 1.0));
    }

    /// The far side goes to silence at the end of the travel, and the near side never moves.
    #[test]
    fn the_balance_silences_the_far_side_and_leaves_the_near_side() {
        assert_eq!(balance(1.0), (0.0, 1.0));
        assert_eq!(balance(-1.0), (1.0, 0.0));
        assert_eq!(balance(0.5), (0.5, 1.0));
        assert_eq!(balance(-0.25), (1.0, 0.75));
        assert_eq!(balance(3.0), (0.0, 1.0));
    }

    #[test]
    fn a_path_at_its_targets_applies_the_level() {
        let mut path = Utility::new(6.0, 0.0, false);
        let out = path.tick(Frame::mono(0.5), 6.0, 0.0, false, SMOOTH);
        assert!((out.l - 0.5 * 1.9953).abs() < 0.01);
        assert_eq!(out.l, out.r);
    }

    #[test]
    fn invert_flips_both_sides() {
        let mut path = Utility::new(0.0, 0.0, true);
        let out = path.tick(Frame::new(0.5, -0.25), 0.0, 0.0, true, SMOOTH);
        assert_eq!(out, Frame::new(-0.5, 0.25));
    }

    #[test]
    fn the_minimum_level_is_silent() {
        let mut path = Utility::new(UTILITY_MIN_DB, 0.0, false);
        assert_eq!(
            settle(&mut path, UTILITY_MIN_DB, 0.0, false, 100),
            Frame::SILENT
        );
    }

    #[test]
    fn a_path_pans_the_far_side_to_silence() {
        let mut path = Utility::new(0.0, 1.0, false);
        let out = settle(&mut path, 0.0, 1.0, false, 10);
        assert_eq!(out, Frame::new(0.0, 1.0));
    }

    /// A move of the level eases in. The first frame closes a small share of the gap.
    #[test]
    fn the_level_eases_without_a_jump() {
        let mut path = Utility::new(0.0, 0.0, false);
        let first = path.tick(Frame::mono(1.0), UTILITY_MIN_DB, 0.0, false, SMOOTH);
        assert!((first.l - 0.99).abs() < 1e-4, "{}", first.l);
        let last = settle(&mut path, UTILITY_MIN_DB, 0.0, false, 3000);
        assert!(last.l.abs() < 1e-6);
    }

    /// The pan eases too. One frame after a hard move the far side is still almost at unity.
    #[test]
    fn the_pan_eases_without_a_jump() {
        let mut path = Utility::new(0.0, 0.0, false);
        let first = path.tick(Frame::mono(1.0), 0.0, 1.0, false, SMOOTH);
        assert!((first.l - 0.99).abs() < 1e-4, "{}", first.l);
        assert_eq!(first.r, 1.0);
    }

    /// Flipping Invert fades through zero. No two neighboring samples differ by more than the
    /// easing step, and the output ends fully inverted.
    #[test]
    fn invert_crosses_zero_without_a_jump() {
        let mut path = Utility::new(0.0, 0.0, false);
        let mut last = 1.0;
        let mut crossed = false;
        for _ in 0..2000 {
            let out = path.tick(Frame::mono(1.0), 0.0, 0.0, true, SMOOTH).l;
            assert!((out - last).abs() <= 2.0 * SMOOTH + 1e-6, "{last} to {out}");
            crossed |= out.abs() < 0.05;
            last = out;
        }
        assert!(crossed);
        assert!((last + 1.0).abs() < 1e-3);
    }

    #[test]
    fn the_floor_is_silent_and_zero_db_is_unity() {
        assert_eq!(db_to_linear(SILENT_DB), 0.0);
        assert_eq!(db_to_linear(-60.0), 0.0);
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
        assert!((db_to_linear(-6.0) - 0.501).abs() < 0.01);
    }
}
