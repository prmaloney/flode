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

/// What the cutoff and resonance work out to. Making them takes a `tan`, so they are made once
/// per sample and handed to each filter that needs them.
#[derive(Debug, Clone, Copy)]
pub struct Coefficients {
    /// The damping. 2.0 is no resonance, and the floor keeps the filter stable.
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
}

impl Coefficients {
    /// `cutoff` is in Hz and `resonance` runs from 0.0 to 1.0.
    pub fn new(cutoff: f32, resonance: f32, sample_rate: f32) -> Self {
        let cutoff = cutoff.clamp(20.0, sample_rate * 0.49);
        let g = (PI * cutoff / sample_rate).tan();
        let k = 2.0 - 1.9 * resonance.clamp(0.0, 1.0);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        Self { k, a1, a2, a3 }
    }
}

/// The gain of the filter at `freq` Hz, as a ratio where 1.0 is unity. It uses the same cutoff
/// clamp and damping as `Coefficients::new`, so the number matches what `Svf::tick` does to a sine.
/// The filter is a bilinear transform of the analog one, so the response is the analog one with
/// the frequency warped by `tan`. `cutoff` is in Hz and `resonance` runs from 0.0 to 1.0.
pub fn magnitude(
    kind: FilterType,
    freq: f32,
    cutoff: f32,
    resonance: f32,
    sample_rate: f32,
) -> f32 {
    let cutoff = cutoff.clamp(20.0, sample_rate * 0.49);
    let k = 2.0 - 1.9 * resonance.clamp(0.0, 1.0);
    let g = (PI * cutoff / sample_rate).tan();
    let freq = freq.clamp(1.0, sample_rate * 0.499);
    // The warped frequency over the warped cutoff.
    let x = (PI * freq / sample_rate).tan() / g;
    let re = 1.0 - x * x;
    let im = k * x;
    let denominator = (re * re + im * im).sqrt();
    match kind {
        FilterType::LowPass => 1.0 / denominator,
        FilterType::HighPass => x * x / denominator,
        FilterType::BandPass => im / denominator,
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
    #[cfg(test)]
    pub fn process(
        &mut self,
        input: f32,
        cutoff: f32,
        resonance: f32,
        kind: FilterType,
        sample_rate: f32,
    ) -> f32 {
        self.tick(
            input,
            &Coefficients::new(cutoff, resonance, sample_rate),
            kind,
        )
    }

    /// One sample through the filter with coefficients made ahead of time. Two filters that run
    /// the same response, such as the two sides of a stereo signal, can share them.
    pub fn tick(&mut self, input: f32, c: &Coefficients, kind: FilterType) -> f32 {
        let Coefficients { k, a1, a2, a3 } = *c;
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

    fn peak_with(kind: FilterType, cutoff: f32, resonance: f32, freq: f32) -> f32 {
        let mut svf = Svf::new();
        let mut peak = 0.0f32;
        for n in 0..44100 {
            let input = (TAU * freq * n as f32 / 44100.0).sin();
            let out = svf.process(input, cutoff, resonance, kind, 44100.0);
            if n > 22050 {
                peak = peak.max(out.abs());
            }
        }
        peak
    }

    fn db(gain: f32) -> f32 {
        20.0 * gain.max(1e-9).log10()
    }

    /// The response function agrees with sines run through the real filter, below, at, and above
    /// the cutoff, for every type and a few resonances.
    #[test]
    fn magnitude_matches_the_filter() {
        for kind in FilterType::ALL {
            for resonance in [0.0, 0.5, 0.9] {
                for freq in [250.0, 1000.0, 4000.0] {
                    let measured = db(peak_with(kind, 1000.0, resonance, freq));
                    let predicted = db(magnitude(kind, freq, 1000.0, resonance, 44100.0));
                    assert!(
                        (measured - predicted).abs() < 1.0,
                        "{kind:?} r={resonance} f={freq}: measured {measured} dB, predicted {predicted} dB"
                    );
                }
            }
        }
    }

    #[test]
    fn magnitude_has_each_types_shape() {
        let m = |kind, freq| magnitude(kind, freq, 1000.0, 0.0, 44100.0);
        assert!(db(m(FilterType::LowPass, 50.0)).abs() < 0.5);
        assert!(m(FilterType::LowPass, 8000.0) < m(FilterType::LowPass, 1000.0));
        assert!(db(m(FilterType::HighPass, 15000.0)).abs() < 1.5);
        assert!(m(FilterType::HighPass, 100.0) < m(FilterType::HighPass, 1000.0));
        let at = m(FilterType::BandPass, 1000.0);
        assert!(at > m(FilterType::BandPass, 250.0));
        assert!(at > m(FilterType::BandPass, 4000.0));
        assert!((at - 1.0).abs() < 0.01);
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
