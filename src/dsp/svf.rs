//! A state-variable filter. It computes the low, band, and high pass outputs from the same two
//! integrators, so changing the response only picks which output to return.

use serde::{Deserialize, Serialize};
use std::f32::consts::PI;

/// The filter responses.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum FilterType {
    #[default]
    LowPass = 0,
    HighPass = 1,
    BandPass = 2,
}

impl FilterType {
    /// Every type, in the order the dropdown lists them.
    pub const ALL: [FilterType; 3] = [
        FilterType::LowPass,
        FilterType::HighPass,
        FilterType::BandPass,
    ];

    /// The dropdown row for this type.
    pub fn index(self) -> usize {
        self as usize
    }
}

/// The filter's memory: the two integrators.
#[derive(Debug, Clone, Copy, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    pub const fn new() -> Self {
        Self { ic1: 0.0, ic2: 0.0 }
    }

    /// One sample through the filter. `cutoff` is in Hz and `resonance` runs from 0.0 to 1.0.
    pub fn process(
        &mut self,
        input: f32,
        cutoff: f32,
        resonance: f32,
        kind: FilterType,
        sample_rate: f32,
    ) -> f32 {
        let cutoff = cutoff.clamp(20.0, sample_rate * 0.49);
        let g = (PI * cutoff / sample_rate).tan();
        // Damping: 2.0 is no resonance, and the floor keeps the filter stable.
        let k = 2.0 - 1.9 * resonance.clamp(0.0, 1.0);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        let v3 = input - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;

        match kind {
            FilterType::LowPass => v2,
            // The high pass is what the bass and band pass leave out. It is zero at DC.
            FilterType::HighPass => input - k * v1 - v2,
            // Scaling by k keeps the peak near 1 at every resonance.
            FilterType::BandPass => k * v1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    /// The peak output for a sine at `freq`, after the filter has settled.
    fn peak(kind: FilterType, cutoff: f32, freq: f32) -> f32 {
        let mut svf = Svf::new();
        let mut peak = 0.0f32;
        for n in 0..44100 {
            let input = (TAU * freq * n as f32 / 44100.0).sin();
            let out = svf.process(input, cutoff, 0.0, kind, 44100.0);
            if n > 22050 {
                peak = peak.max(out.abs());
            }
        }
        peak
    }

    #[test]
    fn each_type_passes_its_own_band() {
        assert!(peak(FilterType::LowPass, 1000.0, 100.0) > 0.9);
        assert!(peak(FilterType::LowPass, 1000.0, 10000.0) < 0.2);

        assert!(peak(FilterType::HighPass, 1000.0, 10000.0) > 0.9);
        assert!(peak(FilterType::HighPass, 1000.0, 100.0) < 0.2);

        assert!(peak(FilterType::BandPass, 1000.0, 1000.0) > 0.9);
        assert!(peak(FilterType::BandPass, 1000.0, 100.0) < 0.2);
        assert!(peak(FilterType::BandPass, 1000.0, 10000.0) < 0.2);
    }

    /// Two filters keep separate memory, so one voice's signal never leaks into another's.
    #[test]
    fn filters_do_not_share_state() {
        let (mut a, mut b) = (Svf::new(), Svf::new());
        for _ in 0..100 {
            a.process(1.0, 1000.0, 0.0, FilterType::LowPass, 44100.0);
        }
        assert_eq!(
            b.process(0.0, 1000.0, 0.0, FilterType::LowPass, 44100.0),
            0.0
        );
    }
}
