//! Ways to combine two audio samples into one.

use serde::{Deserialize, Serialize};

/// How a mix combines its two signals.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum MixMode {
    #[default]
    Crossfade = 0,
    Am = 1,
    RingMod = 2,
    /// Signal B pushes the phase of signal A around, as FM does to a carrier. It needs a delay
    /// line, so the mix stage runs it, and `combine` only passes A through.
    Fm = 3,
}

/// How far a full-scale modulator moves the phase at the top of the mix amount, in cycles. One
/// cycle is a modulation index of about 6.3. Tuned by ear.
const FM_MAX_CYCLES: f32 = 2.0;

/// How far the modulator pushes the phase, in cycles, per unit of modulator signal. The curve is
/// squared so the low end of the knob has fine control, where the sound changes fastest.
pub fn fm_depth(amount: f32) -> f32 {
    let a = amount.clamp(0.0, 1.0);
    a * a * FM_MAX_CYCLES
}

impl MixMode {
    /// Every mode, in the order the dropdown lists them.
    pub const ALL: [MixMode; 4] = [
        MixMode::Crossfade,
        MixMode::Am,
        MixMode::RingMod,
        MixMode::Fm,
    ];

    /// Combines two samples, for the modes that need no memory. `mix` runs from 0.0, which is
    /// plain signal A, to 1.0, which is the full effect. For a crossfade that is all of B. FM
    /// passes A through, since the mix stage does it with a delay line.
    pub fn combine(self, a: f32, b: f32, mix: f32) -> f32 {
        match self {
            // An equal-power blend, so the perceived loudness stays level across the sweep.
            MixMode::Crossfade => {
                let angle = mix * std::f32::consts::FRAC_PI_2;
                a * angle.cos() + b * angle.sin()
            }
            // B modulates A's amplitude. At 1, B's swing from -1 to 1 takes A's amplitude from zero
            // to double.
            MixMode::Am => a * (1.0 + mix * b),
            // The product of the two signals, faded in as the mix rises.
            MixMode::RingMod => a * (1.0 - mix) + a * b * mix,
            MixMode::Fm => a,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossfade_at_the_ends_is_just_one_signal() {
        assert_eq!(MixMode::Crossfade.combine(1.0, 0.5, 0.0), 1.0);
        assert!((MixMode::Crossfade.combine(1.0, 0.5, 1.0) - 0.5).abs() < 1e-6);
    }

    /// An equal-power crossfade keeps the same power in the middle as at the ends.
    #[test]
    fn crossfade_keeps_equal_power() {
        let mid = MixMode::Crossfade.combine(1.0, 1.0, 0.5);
        assert!((mid * mid - 2.0).abs() < 1e-5);
    }

    #[test]
    fn am_at_zero_mix_passes_a_through() {
        assert_eq!(MixMode::Am.combine(0.3, 0.9, 0.0), 0.3);
    }

    /// FM depth grows with the amount, starts at nothing, and tops out at its maximum.
    #[test]
    fn fm_depth_rises_from_zero_to_its_maximum() {
        assert_eq!(fm_depth(0.0), 0.0);
        assert_eq!(fm_depth(1.0), FM_MAX_CYCLES);
        assert!(fm_depth(0.25) < fm_depth(0.5) && fm_depth(0.5) < fm_depth(1.0));
        // The knob clamps, so a driven amount can't push past the maximum.
        assert_eq!(fm_depth(3.0), FM_MAX_CYCLES);
    }

    #[test]
    fn ring_mod_at_full_mix_is_the_plain_product() {
        assert_eq!(MixMode::RingMod.combine(0.5, 0.4, 1.0), 0.2);
        assert_eq!(MixMode::RingMod.combine(0.5, 0.4, 0.0), 0.5);
    }
}
