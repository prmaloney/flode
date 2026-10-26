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

/// How steep a low or high pass falls, in dB per octave. It is a tap along a four-stage ladder:
/// the first stage's output falls 6 dB per octave, the second 12, and so on. The resonance
/// feedback runs around all four stages, so every slope has it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Slope {
    Six,
    #[default]
    Twelve,
    Eighteen,
    TwentyFour,
}

impl Slope {
    /// The slope for a knob step, from 0 (6 dB) to 3 (24 dB).
    pub fn from_step(step: f32) -> Slope {
        match step.round() as i32 {
            ..=0 => Slope::Six,
            1 => Slope::Twelve,
            2 => Slope::Eighteen,
            _ => Slope::TwentyFour,
        }
    }

    /// How many stages of the ladder this slope takes its output from.
    fn stages(self) -> usize {
        match self {
            Slope::Six => 1,
            Slope::Twelve => 2,
            Slope::Eighteen => 3,
            Slope::TwentyFour => 4,
        }
    }
}

/// The feedback around the ladder at full resonance. Four is where it would sing on its own, so
/// the top stops just short and stays stable.
const LADDER_FEEDBACK_MAX: f32 = 3.9;

/// What the cutoff and resonance work out to for the ladder. Like `Coefficients`, it is made once
/// per sample and shared by both sides.
#[derive(Debug, Clone, Copy)]
pub struct LadderCoefficients {
    /// How much of the gap each stage closes: `g / (1 + g)`.
    a: f32,
    /// The feedback amount.
    k: f32,
    /// A Moog ladder loses level in its low end as the feedback rises. Half of that is won back.
    makeup: f32,
}

impl LadderCoefficients {
    /// `cutoff` is in Hz and `resonance` runs from 0.0 to 1.0.
    pub fn new(cutoff: f32, resonance: f32, sample_rate: f32) -> Self {
        let cutoff = cutoff.clamp(20.0, sample_rate * 0.49);
        let g = (PI * cutoff / sample_rate).tan();
        let k = LADDER_FEEDBACK_MAX * resonance.clamp(0.0, 1.0);
        Self {
            a: g / (1.0 + g),
            k,
            makeup: 1.0 + 0.5 * k,
        }
    }
}

/// A four-stage ladder of one-pole filters with feedback from the last stage to the input. It
/// gives the low pass at 6, 12, 18 and 24 dB per octave, and the matching high passes, from the
/// same four stages.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ladder {
    state: [f32; 4],
}

impl Ladder {
    pub const fn new() -> Self {
        Self { state: [0.0; 4] }
    }

    /// One sample through the ladder, as a low pass or, with `high`, a high pass.
    pub fn tick(&mut self, input: f32, c: &LadderCoefficients, slope: Slope, high: bool) -> f32 {
        let a = c.a;
        // Each stage's output is `a` times its input plus a part `rest` that its memory holds.
        // That lets the feedback loop be solved at once, without a one-sample delay.
        let rest = [
            (1.0 - a) * self.state[0],
            (1.0 - a) * self.state[1],
            (1.0 - a) * self.state[2],
            (1.0 - a) * self.state[3],
        ];
        let from_memory = a * a * a * rest[0] + a * a * rest[1] + a * rest[2] + rest[3];
        let drive = (input - c.k * from_memory) / (1.0 + c.k * a * a * a * a);

        let mut out = [drive; 5];
        for i in 0..4 {
            let v = a * (out[i] - self.state[i]);
            let y = v + self.state[i];
            self.state[i] = y + v;
            out[i + 1] = y;
        }
        // out[0] is the input to the first stage, out[n] the output of stage n.
        let n = slope.stages();
        let tap = if high {
            // The high pass is the input minus the low pass, taken n times over.
            const PASCAL: [[f32; 5]; 5] = [
                [0.0; 5],
                [1.0, -1.0, 0.0, 0.0, 0.0],
                [1.0, -2.0, 1.0, 0.0, 0.0],
                [1.0, -3.0, 3.0, -1.0, 0.0],
                [1.0, -4.0, 6.0, -4.0, 1.0],
            ];
            (0..=n).map(|j| PASCAL[n][j] * out[j]).sum::<f32>()
        } else {
            out[n]
        };
        tap * c.makeup
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

/// A complex number as (real, imaginary), for the ladder's response.
type Complex = (f32, f32);

fn mul(a: Complex, b: Complex) -> Complex {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}

/// The gain of the filter at `freq` Hz, as a ratio where 1.0 is unity. It uses the same cutoff
/// clamp and feedback as the coefficients, so the number matches what `Svf::tick` and
/// `Ladder::tick` do to a sine. The filters are bilinear transforms of the analog ones, so the
/// response is the analog one with the frequency warped by `tan`. `cutoff` is in Hz and
/// `resonance` runs from 0.0 to 1.0. The band pass is the state-variable filter and ignores the
/// slope. The low and high pass are the ladder.
pub fn magnitude(
    kind: FilterType,
    slope: Slope,
    freq: f32,
    cutoff: f32,
    resonance: f32,
    sample_rate: f32,
) -> f32 {
    let cutoff = cutoff.clamp(20.0, sample_rate * 0.49);
    let g = (PI * cutoff / sample_rate).tan();
    let freq = freq.clamp(1.0, sample_rate * 0.499);
    // The warped frequency over the warped cutoff.
    let x = (PI * freq / sample_rate).tan() / g;
    if kind == FilterType::BandPass {
        let k = 2.0 - 1.9 * resonance.clamp(0.0, 1.0);
        let re = 1.0 - x * x;
        let im = k * x;
        return im / (re * re + im * im).sqrt();
    }
    let k = LADDER_FEEDBACK_MAX * resonance.clamp(0.0, 1.0);
    // One stage of the ladder, 1 / (1 + jx), and the feedback loop around four of them.
    let scale = 1.0 / (1.0 + x * x);
    let stage: Complex = (scale, -x * scale);
    let mut four = (1.0, 0.0);
    for _ in 0..4 {
        four = mul(four, stage);
    }
    let loop_gain = (1.0 + k * four.0, k * four.1);
    let loop_size = (loop_gain.0 * loop_gain.0 + loop_gain.1 * loop_gain.1).sqrt();
    let n = slope.stages() as i32;
    let size = (scale.sqrt()).powi(n);
    let part = if kind == FilterType::HighPass {
        x.powi(n) * size
    } else {
        size
    };
    part / loop_size * (1.0 + 0.5 * k)
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

    /// The peak output of the real filter for a sine at `freq`: the state-variable filter for the
    /// band pass, and the ladder for the low and high pass.
    fn peak_with(kind: FilterType, slope: Slope, resonance: f32, freq: f32) -> f32 {
        let mut svf = Svf::new();
        let mut ladder = Ladder::new();
        let coefficients = LadderCoefficients::new(1000.0, resonance, 44100.0);
        let mut peak = 0.0f32;
        for n in 0..44100 {
            let input = (TAU * freq * n as f32 / 44100.0).sin();
            let out = if kind == FilterType::BandPass {
                svf.process(input, 1000.0, resonance, kind, 44100.0)
            } else {
                ladder.tick(input, &coefficients, slope, kind == FilterType::HighPass)
            };
            if n > 22050 {
                peak = peak.max(out.abs());
            }
        }
        peak
    }

    fn db(gain: f32) -> f32 {
        20.0 * gain.max(1e-9).log10()
    }

    const SLOPES: [Slope; 4] = [
        Slope::Six,
        Slope::Twelve,
        Slope::Eighteen,
        Slope::TwentyFour,
    ];

    /// The response function agrees with sines run through the real filter, below, at, and above
    /// the cutoff, for every type, slope and a few resonances.
    #[test]
    fn magnitude_matches_the_filter() {
        for kind in FilterType::ALL {
            for slope in SLOPES {
                for resonance in [0.0, 0.5, 0.9] {
                    for freq in [250.0, 1000.0, 4000.0] {
                        let measured = db(peak_with(kind, slope, resonance, freq));
                        let predicted =
                            db(magnitude(kind, slope, freq, 1000.0, resonance, 44100.0));
                        assert!(
                            (measured - predicted).abs() < 1.0,
                            "{kind:?} {slope:?} r={resonance} f={freq}: measured {measured} dB, predicted {predicted} dB"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn magnitude_has_each_types_shape() {
        let m = |kind, freq| magnitude(kind, Slope::Twelve, freq, 1000.0, 0.0, 44100.0);
        assert!(db(m(FilterType::LowPass, 50.0)).abs() < 0.5);
        assert!(m(FilterType::LowPass, 8000.0) < m(FilterType::LowPass, 1000.0));
        assert!(db(m(FilterType::HighPass, 15000.0)).abs() < 1.5);
        assert!(m(FilterType::HighPass, 100.0) < m(FilterType::HighPass, 1000.0));
        let at = m(FilterType::BandPass, 1000.0);
        assert!(at > m(FilterType::BandPass, 250.0));
        assert!(at > m(FilterType::BandPass, 4000.0));
        assert!((at - 1.0).abs() < 0.01);
    }

    /// Well past the cutoff, each step up in slope is quieter, in both the low and the high pass.
    #[test]
    fn each_slope_falls_further_than_the_last() {
        let far = |kind, freq, slope| magnitude(kind, slope, freq, 1000.0, 0.0, 44100.0);
        for pair in SLOPES.windows(2) {
            assert!(
                far(FilterType::LowPass, 8000.0, pair[0])
                    > far(FilterType::LowPass, 8000.0, pair[1])
            );
            assert!(
                far(FilterType::HighPass, 125.0, pair[0])
                    > far(FilterType::HighPass, 125.0, pair[1])
            );
        }
    }

    /// Every slope has the resonant peak, as on a ladder, where the feedback runs around all four
    /// stages. The peak at the cutoff rises with the resonance for each one.
    #[test]
    fn every_slope_resonates() {
        for kind in [FilterType::LowPass, FilterType::HighPass] {
            for slope in SLOPES {
                let at = |resonance| magnitude(kind, slope, 1000.0, 1000.0, resonance, 44100.0);
                assert!(at(0.9) > at(0.1) * 1.5, "{kind:?} {slope:?}");
            }
        }
    }

    /// Full resonance stays stable: a burst of noise dies away to silence.
    #[test]
    fn full_resonance_is_stable() {
        for slope in SLOPES {
            let mut ladder = Ladder::new();
            let c = LadderCoefficients::new(2000.0, 1.0, 44100.0);
            let mut x = 1u32;
            let mut last = 0.0f32;
            for n in 0..60000 {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                let input = if n < 100 {
                    (x >> 8) as f32 / 8388608.0 - 1.0
                } else {
                    0.0
                };
                last = ladder.tick(input, &c, slope, false);
                assert!(
                    last.is_finite() && last.abs() < 50.0,
                    "{slope:?} blew up at {n}"
                );
            }
            assert!(last.abs() < 1e-3, "{slope:?} still ringing: {last}");
        }
    }

    #[test]
    fn a_slope_means_nothing_to_a_band_pass() {
        let at = |slope| magnitude(FilterType::BandPass, slope, 2000.0, 1000.0, 0.5, 44100.0);
        assert_eq!(at(Slope::Six), at(Slope::TwentyFour));
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
