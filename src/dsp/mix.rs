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
}

impl MixMode {
    /// Every mode, in the order the dropdown lists them.
    pub const ALL: [MixMode; 3] = [MixMode::Crossfade, MixMode::Am, MixMode::RingMod];

    /// Combines two samples. `mix` runs from 0.0, which is plain signal A, to 1.0, which is the
    /// full effect. For a crossfade that is all of B.
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

    #[test]
    fn ring_mod_at_full_mix_is_the_plain_product() {
        assert_eq!(MixMode::RingMod.combine(0.5, 0.4, 1.0), 0.2);
        assert_eq!(MixMode::RingMod.combine(0.5, 0.4, 0.0), 0.5);
    }
}
