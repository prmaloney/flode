//! The warp modes: ways to bend a waveform's phase or amplitude.

use serde::{Deserialize, Serialize};

/// The warp modes. Each one bends the waveform's shape, and the warp amount sets how far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum WarpMode {
    #[default]
    None = 0,
    /// Crowds the cycle toward one end, so the waveform bends.
    Bend = 1,
    /// Restarts the cycle partway through, as a second oscillator in sync would.
    Sync = 2,
    /// Moves the point where the cycle switches halves, which widens or narrows a pulse.
    Pwm = 3,
    /// Stretches one half of the cycle and squeezes the other.
    Asym = 4,
    /// Plays the waveform backwards.
    Flip = 5,
    /// Folds the second half of the cycle back over the first.
    Mirror = 6,
    /// Rounds the amplitude to a few steps.
    Quantize = 7,
    /// Pushes the phase around with an audio signal from another oscillator, as phase modulation.
    /// The warp amount sets how far. The oscillator reads the signal from its FM input.
    Fm = 8,
}

impl WarpMode {
    pub const ALL: [WarpMode; 9] = [
        WarpMode::None,
        WarpMode::Bend,
        WarpMode::Sync,
        WarpMode::Pwm,
        WarpMode::Asym,
        WarpMode::Flip,
        WarpMode::Mirror,
        WarpMode::Quantize,
        WarpMode::Fm,
    ];
}

/// How far a full-scale modulator moves the phase at the top of the warp amount, in cycles. One
/// cycle is a modulation index of about 6.3. Tuned by ear.
const FM_MAX_CYCLES: f32 = 2.0;

/// How far the modulator pushes the phase, in cycles, per unit of modulator signal. The curve is
/// squared so the low end of the knob has fine control, where the sound changes fastest.
pub fn fm_depth(amount: f32) -> f32 {
    let a = amount.clamp(0.0, 1.0);
    a * a * FM_MAX_CYCLES
}

/// Moves a phase, from 0.0 to 1.0, through a warp. `amount` runs from 0.0, which leaves the phase
/// alone, to 1.0, the full warp. Quantize and FM don't move the phase here. FM needs a signal,
/// which the oscillator adds itself.
pub fn warp_phase(mode: WarpMode, phase: f32, amount: f32) -> f32 {
    let a = amount.clamp(0.0, 1.0);
    let warped = match mode {
        WarpMode::None | WarpMode::Quantize | WarpMode::Fm => return phase,
        // A power curve on the phase, which crowds it toward one end.
        WarpMode::Bend => phase.powf(2.0f32.powf(-(a * 2.0 - 1.0) * 2.0)),
        // The cycle restarts at a higher rate, so later cycles repeat sooner.
        WarpMode::Sync => (phase * (1.0 + a * 7.0)).fract(),
        // The split point moves from a tenth to nine tenths of the cycle, and each side is
        // stretched to fill half the output.
        WarpMode::Pwm | WarpMode::Asym => {
            let split = 0.5 + 0.4 * (2.0 * a - 1.0);
            if phase < split {
                phase * 0.5 / split
            } else {
                0.5 + (phase - split) * 0.5 / (1.0 - split)
            }
        }
        WarpMode::Flip => 1.0 - phase,
        WarpMode::Mirror => {
            if phase < 0.5 {
                phase * 2.0
            } else {
                2.0 - phase * 2.0
            }
        }
    };
    // Blend between the plain phase and the warped one, so the amount sets the strength.
    (phase + (warped - phase) * a).rem_euclid(1.0)
}

/// Rounds a value, from -1.0 to 1.0, to a few steps when the warp is quantize. The amount blends
/// the rounded value with the original, and at zero the value is untouched.
pub fn warp_value(mode: WarpMode, value: f32, amount: f32) -> f32 {
    if mode != WarpMode::Quantize {
        return value;
    }
    let a = amount.clamp(0.0, 1.0);
    // From 64 steps down to 2 as the amount rises.
    let steps = 64.0 - a * 62.0;
    let rounded = (value * steps * 0.5).round() / (steps * 0.5);
    value + (rounded - value) * a
}

#[cfg(test)]
mod warp_tests {
    use super::*;

    /// FM depth grows with the amount, starts at nothing, and tops out at its maximum.
    #[test]
    fn fm_depth_rises_from_zero_to_its_maximum() {
        assert_eq!(fm_depth(0.0), 0.0);
        assert_eq!(fm_depth(1.0), FM_MAX_CYCLES);
        assert!(fm_depth(0.25) < fm_depth(0.5) && fm_depth(0.5) < fm_depth(1.0));
        // The knob clamps, so a driven amount can't push past the maximum.
        assert_eq!(fm_depth(3.0), FM_MAX_CYCLES);
    }

    /// FM moves the phase through the oscillator's own signal, not through the warp.
    #[test]
    fn fm_leaves_the_phase_to_the_oscillator() {
        assert_eq!(warp_phase(WarpMode::Fm, 0.3, 1.0), 0.3);
    }

    /// With no amount, every warp leaves the phase alone.
    #[test]
    fn zero_amount_leaves_the_phase_alone() {
        for mode in WarpMode::ALL {
            assert!((warp_phase(mode, 0.3, 0.0) - 0.3).abs() < 1e-6, "{mode:?}");
        }
    }

    /// Flip plays the cycle backwards at full amount, and mirror folds it.
    #[test]
    fn flip_and_mirror_reshape_the_cycle() {
        assert!((warp_phase(WarpMode::Flip, 0.25, 1.0) - 0.75).abs() < 1e-6);
        assert!((warp_phase(WarpMode::Mirror, 0.25, 1.0) - 0.5).abs() < 1e-6);
    }

    /// Quantize leaves a value alone at zero amount, and rounds it at full amount.
    #[test]
    fn quantize_rounds_only_when_it_is_on() {
        assert_eq!(warp_value(WarpMode::Quantize, 0.37, 0.0), 0.37);
        let rounded = warp_value(WarpMode::Quantize, 0.37, 1.0);
        assert!((rounded * 16.0 - (rounded * 16.0).round()).abs() < 1e-3);
    }
}
