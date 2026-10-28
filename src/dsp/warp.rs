//! The warp modes: ways to bend a waveform's phase or amplitude.

use crate::dsp::mix::fm_depth;
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
    /// The signal on the oscillator's audio input pushes the phase around, as phase modulation
    /// does. The amount is the depth. With nothing wired in, the wave is plain.
    Pm = 8,
    /// The signal on the audio input moves the point where the cycle switches halves, as a
    /// Casio CZ's phase distortion does, so the wave bends with the signal. The amount is how far
    /// the signal can move it. With nothing wired in, the wave is plain.
    Pd = 9,
}

impl WarpMode {
    /// Every mode, in the order the dropdown lists them. The two that read the audio input are
    /// last, so a saved oscillator keeps the mode it was saved with.
    pub const ALL: [WarpMode; 10] = [
        WarpMode::None,
        WarpMode::Bend,
        WarpMode::Sync,
        WarpMode::Pwm,
        WarpMode::Asym,
        WarpMode::Flip,
        WarpMode::Mirror,
        WarpMode::Quantize,
        WarpMode::Pm,
        WarpMode::Pd,
    ];
}

/// `warp_phase` for a signal on the audio input, `modulator`, from -1.0 to 1.0. Only the two
/// modes that read the input look at it, and every other mode is just `warp_phase`.
pub fn warp_phase_with(mode: WarpMode, phase: f32, amount: f32, modulator: f32) -> f32 {
    match mode {
        // The same depth curve the mix node's FM has, squared so the low end of the knob has fine
        // control, and up to two cycles of push at the top.
        WarpMode::Pm => (phase + fm_depth(amount) * modulator).rem_euclid(1.0),
        WarpMode::Pd => {
            // The split point swings from 0.05 to 0.95 of the cycle as the signal does, and sits
            // at the middle, where the map is the identity, for a silent input.
            let swing = (amount.clamp(0.0, 1.0) * modulator).clamp(-1.0, 1.0);
            let split = 0.5 + 0.45 * swing;
            let warped = if phase < split {
                phase * 0.5 / split
            } else {
                0.5 + (phase - split) * 0.5 / (1.0 - split)
            };
            warped.rem_euclid(1.0)
        }
        _ => warp_phase(mode, phase, amount),
    }
}

/// Moves a phase, from 0.0 to 1.0, through a warp. `amount` runs from 0.0, which leaves the phase
/// alone, to 1.0, the full warp. Quantize doesn't move the phase here.
pub fn warp_phase(mode: WarpMode, phase: f32, amount: f32) -> f32 {
    let a = amount.clamp(0.0, 1.0);
    let warped = match mode {
        // The two that read the audio input are `warp_phase_with`'s, and are plain without it.
        WarpMode::None | WarpMode::Quantize | WarpMode::Pm | WarpMode::Pd => return phase,
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

    /// With no amount, every warp leaves the phase alone.
    #[test]
    fn zero_amount_leaves_the_phase_alone() {
        for mode in WarpMode::ALL {
            assert!((warp_phase(mode, 0.3, 0.0) - 0.3).abs() < 1e-6, "{mode:?}");
        }
    }

    /// A silent input leaves the phase alone under both modes that read it.
    #[test]
    fn a_silent_input_leaves_the_phase_alone() {
        for mode in [WarpMode::Pm, WarpMode::Pd] {
            for amount in [0.0, 0.5, 1.0] {
                for phase in [0.0, 0.3, 0.9] {
                    let out = warp_phase_with(mode, phase, amount, 0.0);
                    assert!((out - phase).abs() < 1e-6, "{mode:?} {amount} {phase}");
                }
            }
        }
    }

    /// Phase modulation moves the phase by the signal times the depth, and wraps.
    #[test]
    fn pm_pushes_the_phase_by_the_signal() {
        let depth = fm_depth(1.0);
        let out = warp_phase_with(WarpMode::Pm, 0.1, 1.0, 0.1);
        assert!((out - (0.1 + 0.1 * depth).rem_euclid(1.0)).abs() < 1e-6);
        let back = warp_phase_with(WarpMode::Pm, 0.1, 1.0, -0.1);
        assert!((0.0..1.0).contains(&back));
        // More depth, more push.
        let small = warp_phase_with(WarpMode::Pm, 0.1, 0.3, 0.1) - 0.1;
        let big = warp_phase_with(WarpMode::Pm, 0.1, 0.8, 0.1) - 0.1;
        assert!(big > small && small > 0.0);
    }

    /// Phase distortion bends the cycle toward one end or the other with the sign of the signal,
    /// and keeps the ends of the cycle where they are.
    #[test]
    fn pd_bends_with_the_signal() {
        let up = warp_phase_with(WarpMode::Pd, 0.25, 1.0, 1.0);
        let down = warp_phase_with(WarpMode::Pd, 0.25, 1.0, -1.0);
        assert!(up < 0.25 && down > 0.25, "{up} {down}");
        for m in [-1.0, -0.3, 0.7, 1.0] {
            assert!(warp_phase_with(WarpMode::Pd, 0.0, 1.0, m).abs() < 1e-6);
        }
        // Less amount, less bend.
        let gentle = warp_phase_with(WarpMode::Pd, 0.25, 0.2, 1.0);
        assert!(gentle > up && gentle < 0.25);
    }

    /// Only the two modes that read the input are asked about it.
    #[test]
    fn only_pm_and_pd_read_the_input() {
        for mode in WarpMode::ALL
            .into_iter()
            .filter(|m| !matches!(m, WarpMode::Pm | WarpMode::Pd))
        {
            assert_eq!(
                warp_phase_with(mode, 0.3, 0.7, 0.9),
                warp_phase(mode, 0.3, 0.7)
            );
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
