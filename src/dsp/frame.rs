//! One stereo sample. Every audio stage reads and writes frames, so a signal keeps its left and
//! right sides all the way from the oscillators to the output.

use std::ops::{Add, AddAssign, Mul};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Frame {
    pub l: f32,
    pub r: f32,
}

impl Frame {
    pub const SILENT: Frame = Frame { l: 0.0, r: 0.0 };

    pub const fn new(l: f32, r: f32) -> Self {
        Self { l, r }
    }

    /// The same sample on both sides.
    #[cfg(test)]
    pub const fn mono(sample: f32) -> Self {
        Self::new(sample, sample)
    }

    /// The average of the two sides, for stages that read one signal.
    pub fn mid(self) -> f32 {
        (self.l + self.r) * 0.5
    }

    /// Applies `f` to each side.
    pub fn map(self, f: impl Fn(f32) -> f32) -> Self {
        Self::new(f(self.l), f(self.r))
    }

    /// Combines two frames side by side.
    pub fn zip(self, other: Frame, f: impl Fn(f32, f32) -> f32) -> Self {
        Self::new(f(self.l, other.l), f(self.r, other.r))
    }

    /// The larger of the two sides' magnitudes.
    #[cfg(test)]
    pub fn peak(self) -> f32 {
        self.l.abs().max(self.r.abs())
    }
}

impl Add for Frame {
    type Output = Frame;

    fn add(self, other: Frame) -> Frame {
        self.zip(other, |a, b| a + b)
    }
}

impl AddAssign for Frame {
    fn add_assign(&mut self, other: Frame) {
        *self = *self + other;
    }
}

impl Mul<f32> for Frame {
    type Output = Frame;

    fn mul(self, gain: f32) -> Frame {
        self.map(|x| x * gain)
    }
}

/// The left and right gains that place a mono signal at `position`, from -1.0 (left) to 1.0
/// (right). The pair keeps its power constant across the sweep. The centre is scaled to unity on
/// both sides, so a signal at the centre has the level it had before the engine was stereo.
pub fn pan_gains(position: f32) -> Frame {
    let angle = (position.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    Frame::new(angle.cos(), angle.sin()) * std::f32::consts::SQRT_2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_centre_has_unity_gain_on_both_sides() {
        let gains = pan_gains(0.0);
        assert!((gains.l - 1.0).abs() < 1e-6 && (gains.r - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_hard_pan_silences_the_other_side() {
        assert!(pan_gains(-1.0).r.abs() < 1e-6);
        assert!(pan_gains(1.0).l.abs() < 1e-6);
    }

    /// The power across the sweep stays level, and positions past the ends clamp.
    #[test]
    fn panning_keeps_the_power_constant() {
        for position in [-1.0, -0.5, 0.0, 0.3, 1.0, 5.0] {
            let g = pan_gains(position);
            assert!((g.l * g.l + g.r * g.r - 2.0).abs() < 1e-5, "{position}");
        }
        assert_eq!(pan_gains(5.0), pan_gains(1.0));
    }

    #[test]
    fn frames_add_and_scale_by_side() {
        let sum = Frame::new(1.0, 2.0) + Frame::new(0.5, -1.0);
        assert_eq!(sum, Frame::new(1.5, 1.0));
        assert_eq!(sum * 2.0, Frame::new(3.0, 2.0));
        assert_eq!(Frame::new(1.0, 3.0).mid(), 2.0);
    }
}
