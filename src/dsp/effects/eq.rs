//! A four band equalizer. Each band is one biquad. The two outer bands are shelves or cuts, and
//! the two inner bands are peaks or notches. The bands run in series on each side of the signal.
//!
//! The coefficients are the RBJ cookbook ones. A cut is a second order filter, so it falls at
//! 12 dB per octave, and its Q sets the resonance at the corner. The shelves use Q as the shelf
//! shape, where 0.71 is the flattest.

use crate::dsp::analyzer::{Analyzer, BINS as SPECTRUM_BINS};
use crate::dsp::frame::Frame;
use std::f64::consts::TAU;

/// How many bands the equalizer has.
pub const BANDS: usize = 4;

/// The lowest and highest band frequency in Hz. A frequency is also kept under 0.45 of the sample
/// rate, so a band stays inside the range a biquad handles well.
pub const MIN_FREQ_HZ: f32 = 20.0;
pub const MAX_FREQ_HZ: f32 = 20000.0;
/// The most a band boosts or cuts, in dB.
pub const MAX_GAIN_DB: f32 = 18.0;
/// The lowest and highest Q.
pub const MIN_Q: f32 = 0.1;
pub const MAX_Q: f32 = 10.0;

/// The highest band frequency as a fraction of the sample rate.
const MAX_FREQ_RATIO: f32 = 0.45;
/// How long a band takes to settle on a new frequency, gain, or Q, in seconds.
const SMOOTH_S: f32 = 0.01;
/// A smoothed value this close to its target lands on it, so a still band stops recomputing.
const SNAP: f32 = 1e-4;

/// What a band does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EqKind {
    /// A bump or dip at the frequency.
    #[default]
    Peak,
    /// A narrow dip that removes the frequency. Gain has no effect.
    Notch,
    /// Lifts or lowers everything under the frequency.
    LowShelf,
    /// Lifts or lowers everything over the frequency.
    HighShelf,
    /// Removes everything under the frequency. Gain has no effect.
    HighPass,
    /// Removes everything over the frequency. Gain has no effect.
    LowPass,
}

impl EqKind {
    /// True if the band's gain changes what it does. A cut or a notch ignores it.
    pub fn uses_gain(self) -> bool {
        matches!(self, EqKind::Peak | EqKind::LowShelf | EqKind::HighShelf)
    }
}

/// One band's settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqBand {
    pub kind: EqKind,
    /// The corner or center in Hz, from 20 to 20000.
    pub freq_hz: f32,
    /// The boost or cut in dB, from -18 to 18.
    pub gain_db: f32,
    /// The width of a peak or notch, the shelf shape, or the resonance of a cut, from 0.1 to 10.
    pub q: f32,
}

impl EqBand {
    /// The settings with every value inside its range.
    fn clamped(self, sample_rate: f32) -> Self {
        let top = (sample_rate * MAX_FREQ_RATIO).min(MAX_FREQ_HZ);
        Self {
            kind: self.kind,
            freq_hz: finite_or(self.freq_hz, 1000.0).clamp(MIN_FREQ_HZ, top),
            gain_db: finite_or(self.gain_db, 0.0).clamp(-MAX_GAIN_DB, MAX_GAIN_DB),
            q: finite_or(self.q, 0.71).clamp(MIN_Q, MAX_Q),
        }
    }
}

fn finite_or(x: f32, fallback: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        fallback
    }
}

/// The settings of all four bands, from lowest to highest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqParams {
    pub bands: [EqBand; BANDS],
    /// True to measure the output into `Eq::spectrum`. It costs an FFT every 1024 samples.
    pub analyzer: bool,
}

impl Default for EqParams {
    fn default() -> Self {
        let band = |kind, freq_hz, q| EqBand {
            kind,
            freq_hz,
            gain_db: 0.0,
            q,
        };
        Self {
            bands: [
                band(EqKind::LowShelf, 100.0, 0.71),
                band(EqKind::Peak, 500.0, 1.0),
                band(EqKind::Peak, 2500.0, 1.0),
                band(EqKind::HighShelf, 8000.0, 0.71),
            ],
            analyzer: false,
        }
    }
}

/// The coefficients of one biquad, divided through by a0.
#[derive(Debug, Clone, Copy)]
struct Coefficients {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Coefficients {
    const FLAT: Coefficients = Coefficients {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// The coefficients for a band that is already inside its ranges.
    fn new(band: &EqBand, sample_rate: f32) -> Self {
        let w0 = TAU * f64::from(band.freq_hz) / f64::from(sample_rate);
        let (sin, cos) = w0.sin_cos();
        let q = f64::from(band.q);
        let alpha = sin / (2.0 * q);
        let amp = 10f64.powf(f64::from(band.gain_db) / 40.0);
        let (b0, b1, b2, a0, a1, a2) = match band.kind {
            EqKind::Peak => (
                1.0 + alpha * amp,
                -2.0 * cos,
                1.0 - alpha * amp,
                1.0 + alpha / amp,
                -2.0 * cos,
                1.0 - alpha / amp,
            ),
            EqKind::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            EqKind::LowShelf => {
                let k = 2.0 * amp.sqrt() * alpha;
                (
                    amp * ((amp + 1.0) - (amp - 1.0) * cos + k),
                    2.0 * amp * ((amp - 1.0) - (amp + 1.0) * cos),
                    amp * ((amp + 1.0) - (amp - 1.0) * cos - k),
                    (amp + 1.0) + (amp - 1.0) * cos + k,
                    -2.0 * ((amp - 1.0) + (amp + 1.0) * cos),
                    (amp + 1.0) + (amp - 1.0) * cos - k,
                )
            }
            EqKind::HighShelf => {
                let k = 2.0 * amp.sqrt() * alpha;
                (
                    amp * ((amp + 1.0) + (amp - 1.0) * cos + k),
                    -2.0 * amp * ((amp - 1.0) + (amp + 1.0) * cos),
                    amp * ((amp + 1.0) + (amp - 1.0) * cos - k),
                    (amp + 1.0) - (amp - 1.0) * cos + k,
                    2.0 * ((amp - 1.0) - (amp + 1.0) * cos),
                    (amp + 1.0) - (amp - 1.0) * cos - k,
                )
            }
            EqKind::HighPass => (
                (1.0 + cos) / 2.0,
                -(1.0 + cos),
                (1.0 + cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            EqKind::LowPass => (
                (1.0 - cos) / 2.0,
                1.0 - cos,
                (1.0 - cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
        };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// The gain at `freq` Hz as a ratio, where 1.0 is unity.
    fn magnitude(&self, freq: f32, sample_rate: f32) -> f64 {
        let w = TAU * f64::from(freq) / f64::from(sample_rate);
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        let num_re = self.b0 + self.b1 * c1 + self.b2 * c2;
        let num_im = -(self.b1 * s1 + self.b2 * s2);
        let den_re = 1.0 + self.a1 * c1 + self.a2 * c2;
        let den_im = -(self.a1 * s1 + self.a2 * s2);
        ((num_re * num_re + num_im * num_im) / (den_re * den_re + den_im * den_im).max(1e-30))
            .sqrt()
    }
}

/// The combined gain of all four bands at `freq` Hz, in dB. It clamps each band the way the
/// effect does, so the number matches what a sine at that frequency comes out as. The editor
/// draws this.
pub fn response_db(params: &EqParams, freq: f32, sample_rate: f32) -> f32 {
    let freq = freq.clamp(1.0, sample_rate * 0.499);
    let gain: f64 = params
        .bands
        .iter()
        .map(|band| {
            Coefficients::new(&band.clamped(sample_rate), sample_rate).magnitude(freq, sample_rate)
        })
        .product();
    (20.0 * gain.max(1e-9).log10()) as f32
}

/// A biquad's memory, in transposed direct form II. That form stays well behaved when the
/// coefficients move.
#[derive(Debug, Clone, Copy, Default)]
struct Memory {
    z1: f64,
    z2: f64,
}

impl Memory {
    fn tick(&mut self, input: f64, c: &Coefficients) -> f64 {
        let out = c.b0 * input + self.z1;
        self.z1 = c.b1 * input - c.a1 * out + self.z2;
        self.z2 = c.b2 * input - c.a2 * out;
        // A tail decaying to zero must not reach denormals. Both states clear together, since
        // clearing one alone feeds the other back into it.
        if self.z1.abs() < 1e-20 && self.z2.abs() < 1e-20 {
            self.z1 = 0.0;
            self.z2 = 0.0;
        }
        out
    }
}

/// A band as it runs: the values it has eased to, and the biquad they make.
#[derive(Debug, Clone, Copy)]
struct Section {
    kind: EqKind,
    /// The frequency as its log2, so a sweep covers octaves at an even pace.
    log_freq: f32,
    gain_db: f32,
    q: f32,
    coefficients: Coefficients,
    left: Memory,
    right: Memory,
    /// The settings last asked for, so a band that is asked for the same again does no work.
    asked: Option<EqBand>,
    /// The clamped settings the band is easing toward, with the frequency as its log2.
    goal: EqBand,
    log_goal: f32,
    /// True once the band has reached its goal, so it stops easing and recomputing.
    settled: bool,
}

impl Section {
    fn new() -> Self {
        Self {
            kind: EqKind::Peak,
            log_freq: 10.0,
            gain_db: 0.0,
            q: 1.0,
            coefficients: Coefficients::FLAT,
            left: Memory::default(),
            right: Memory::default(),
            asked: None,
            goal: EqBand {
                kind: EqKind::Peak,
                freq_hz: 1000.0,
                gain_db: 0.0,
                q: 1.0,
            },
            log_goal: 10.0,
            settled: false,
        }
    }

    fn band(&self) -> EqBand {
        EqBand {
            kind: self.kind,
            freq_hz: self.log_freq.exp2(),
            gain_db: self.gain_db,
            q: self.q,
        }
    }
}

/// The equalizer. One instance serves one place in the signal chain.
pub struct Eq {
    sample_rate: f32,
    /// The one-pole coefficient that eases a band toward its target.
    smooth: f32,
    sections: [Section; BANDS],
    /// False until the first sample, when each band starts at its target with no ease.
    started: bool,
    analyzer: Analyzer,
    /// True while the analyzer is taking samples, so switching it off clears it once.
    analyzing: bool,
}

impl Eq {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        Self {
            sample_rate,
            smooth: 1.0 - (-1.0 / (SMOOTH_S * sample_rate)).exp(),
            sections: [Section::new(); BANDS],
            started: false,
            analyzer: Analyzer::new(sample_rate),
            analyzing: false,
        }
    }

    /// The level of each band of the output in dB, from 20 Hz to 20 kHz. It is at the floor while
    /// the analyzer is off.
    pub fn spectrum(&self) -> &[f32; SPECTRUM_BINS] {
        self.analyzer.bins()
    }

    /// Clears the filter memory. The bands start at their targets again.
    pub fn reset(&mut self) {
        self.sections = [Section::new(); BANDS];
        self.started = false;
        self.analyzer.clear();
    }

    /// One stereo sample.
    pub fn process(&mut self, input: Frame, params: &EqParams) -> Frame {
        let mut l = f64::from(input.l);
        let mut r = f64::from(input.r);
        for (section, target) in self.sections.iter_mut().zip(&params.bands) {
            // A band asked for what it was last asked costs two biquads and a compare. Only a
            // change to its settings makes it clamp, take a `log2`, and ease.
            if section.asked != Some(*target) {
                section.asked = Some(*target);
                section.goal = target.clamped(self.sample_rate);
                section.log_goal = section.goal.freq_hz.log2();
                section.settled = false;
                if !self.started {
                    // The first sample starts at the goal, with no ease.
                    section.log_freq = section.log_goal;
                    section.gain_db = section.goal.gain_db;
                    section.q = section.goal.q;
                }
            }
            if !section.settled {
                let mut moved = section.kind != section.goal.kind || !self.started;
                section.kind = section.goal.kind;
                moved |= ease(&mut section.log_freq, section.log_goal, self.smooth);
                moved |= ease(&mut section.gain_db, section.goal.gain_db, self.smooth);
                moved |= ease(&mut section.q, section.goal.q, self.smooth);
                if moved {
                    section.coefficients = Coefficients::new(
                        &section.band().clamped(self.sample_rate),
                        self.sample_rate,
                    );
                } else {
                    section.settled = true;
                }
            }
            l = section.left.tick(l, &section.coefficients);
            r = section.right.tick(r, &section.coefficients);
        }
        self.started = true;
        if !(l.is_finite() && r.is_finite()) {
            self.reset();
            return Frame::SILENT;
        }
        let out = Frame::new(l as f32, r as f32);
        if params.analyzer {
            self.analyzer.push(out.mid());
            self.analyzing = true;
        } else if self.analyzing {
            self.analyzer.clear();
            self.analyzing = false;
        }
        out
    }
}

/// Moves `value` a step toward `target`, landing on it once close. Returns true if it moved.
fn ease(value: &mut f32, target: f32, amount: f32) -> bool {
    let gap = target - *value;
    if gap == 0.0 {
        return false;
    }
    *value = if gap.abs() < SNAP {
        target
    } else {
        *value + gap * amount
    };
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 44100.0;

    fn params(f: impl Fn(&mut [EqBand; BANDS])) -> EqParams {
        let mut params = EqParams::default();
        f(&mut params.bands);
        params
    }

    /// The gain in dB of a sine at `freq` through the effect, measured after it settles.
    fn measured_db(params: &EqParams, freq: f32) -> f32 {
        let mut eq = Eq::new(RATE);
        let mut peak = 0.0f32;
        for n in 0..(RATE as usize) {
            let x = (std::f32::consts::TAU * freq * n as f32 / RATE).sin();
            let out = eq.process(Frame::mono(x), params);
            if n > RATE as usize / 2 {
                peak = peak.max(out.l.abs());
            }
        }
        20.0 * peak.max(1e-9).log10()
    }

    #[test]
    fn flat_bands_pass_the_input_unchanged() {
        let params = EqParams::default();
        let mut eq = Eq::new(RATE);
        for n in 0..2000 {
            let x = (n as f32 * 0.05).sin() * 0.5;
            let out = eq.process(Frame::new(x, -x), &params);
            assert!((out.l - x).abs() < 1e-5 && (out.r + x).abs() < 1e-5, "{n}");
        }
        assert_eq!(response_db(&params, 1000.0, RATE).abs() < 1e-4, true);
    }

    #[test]
    fn a_peak_boosts_at_its_frequency_and_not_far_from_it() {
        let params = params(|b| {
            b[1].gain_db = 12.0;
            b[1].freq_hz = 1000.0;
            b[1].q = 2.0;
        });
        assert!((measured_db(&params, 1000.0) - 12.0).abs() < 0.3);
        assert!(measured_db(&params, 50.0).abs() < 0.5);
        assert!(measured_db(&params, 15000.0).abs() < 0.5);
    }

    #[test]
    fn a_notch_removes_its_frequency() {
        let params = params(|b| {
            b[2].kind = EqKind::Notch;
            b[2].freq_hz = 2000.0;
            b[2].q = 4.0;
        });
        assert!(measured_db(&params, 2000.0) < -40.0);
        assert!(measured_db(&params, 200.0).abs() < 0.5);
        // A notch ignores its gain.
        let with_gain = self::params(|b| {
            b[2] = EqBand {
                gain_db: 15.0,
                ..params.bands[2]
            };
        });
        assert!(measured_db(&with_gain, 2000.0) < -40.0);
    }

    #[test]
    fn shelves_lift_one_end() {
        let low = params(|b| {
            b[0].gain_db = 9.0;
            b[0].freq_hz = 200.0;
        });
        assert!((measured_db(&low, 30.0) - 9.0).abs() < 0.5);
        assert!(measured_db(&low, 5000.0).abs() < 0.5);
        let high = params(|b| {
            b[3].gain_db = -9.0;
            b[3].freq_hz = 4000.0;
        });
        assert!((measured_db(&high, 18000.0) + 9.0).abs() < 0.8);
        assert!(measured_db(&high, 100.0).abs() < 0.5);
    }

    /// A cut falls 12 dB an octave past its corner and is 3 dB down at the corner at Q 0.71.
    #[test]
    fn cuts_fall_at_twelve_db_an_octave() {
        let high_pass = params(|b| {
            b[0].kind = EqKind::HighPass;
            b[0].freq_hz = 1000.0;
        });
        assert!((measured_db(&high_pass, 1000.0) + 3.0).abs() < 0.5);
        let one = measured_db(&high_pass, 250.0);
        let two = measured_db(&high_pass, 125.0);
        assert!((two - one + 12.0).abs() < 1.5, "{one} then {two}");
        assert!(measured_db(&high_pass, 8000.0).abs() < 0.5);

        let low_pass = params(|b| {
            b[3].kind = EqKind::LowPass;
            b[3].freq_hz = 2000.0;
        });
        assert!(measured_db(&low_pass, 100.0).abs() < 0.5);
        assert!(measured_db(&low_pass, 16000.0) < -30.0);
        // A cut ignores its gain.
        let with_gain = self::params(|b| {
            b[3] = EqBand {
                gain_db: 12.0,
                ..low_pass.bands[3]
            };
        });
        assert!((measured_db(&with_gain, 16000.0) - measured_db(&low_pass, 16000.0)).abs() < 0.1);
    }

    /// The response function agrees with sines run through the effect.
    #[test]
    fn the_response_matches_the_effect() {
        let params = params(|b| {
            b[0].gain_db = 6.0;
            b[1].gain_db = -8.0;
            b[1].freq_hz = 700.0;
            b[2].kind = EqKind::Notch;
            b[2].freq_hz = 3000.0;
            b[2].q = 1.5;
            b[3].kind = EqKind::LowPass;
            b[3].freq_hz = 9000.0;
        });
        for freq in [60.0, 300.0, 700.0, 2000.0, 4000.0, 8000.0] {
            let measured = measured_db(&params, freq);
            let predicted = response_db(&params, freq, RATE);
            assert!(
                (measured - predicted).abs() < 0.3,
                "{freq} Hz: measured {measured} dB, predicted {predicted} dB"
            );
        }
    }

    /// The rate changes the coefficients, so a band lands at the same Hz at any rate.
    #[test]
    fn a_band_keeps_its_frequency_at_any_sample_rate() {
        let params = params(|b| {
            b[1].gain_db = 12.0;
            b[1].freq_hz = 1000.0;
        });
        for rate in [22050.0, 48000.0, 96000.0, 192000.0] {
            let at = response_db(&params, 1000.0, rate);
            assert!((at - 12.0).abs() < 0.2, "{rate}: {at}");
        }
    }

    /// Output stays finite for loud input and every extreme of every range, including jumps.
    #[test]
    fn extremes_stay_finite() {
        let kinds = [
            EqKind::Peak,
            EqKind::Notch,
            EqKind::LowShelf,
            EqKind::HighShelf,
            EqKind::HighPass,
            EqKind::LowPass,
        ];
        for rate in [22050.0, 44100.0, 192000.0] {
            let mut eq = Eq::new(rate);
            let mut seed = 12345u32;
            for n in 0..40000 {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let pick = |shift: u32, n: usize| (seed >> shift) as usize % n;
                let band = |i: usize| EqBand {
                    kind: kinds[pick(3 + i as u32, kinds.len())],
                    freq_hz: [1.0, 20.0, 1000.0, 20000.0, 90000.0][pick(7 + i as u32, 5)],
                    gain_db: [-40.0, -18.0, 0.0, 18.0, 40.0][pick(11 + i as u32, 5)],
                    q: [0.0, 0.1, 0.71, 10.0, 50.0][pick(13 + i as u32, 5)],
                };
                // The bands jump every 500 samples, and hold between.
                let params = if n % 500 == 0 || n == 0 {
                    EqParams {
                        bands: std::array::from_fn(band),
                        analyzer: true,
                    }
                } else {
                    EqParams::default()
                };
                let x = ((n as f32) * 0.37).sin() * 8.0;
                let out = eq.process(Frame::new(x, -x), &params);
                assert!(out.l.is_finite() && out.r.is_finite(), "{rate} {n}");
                assert!(
                    out.l.abs() < 1e5 && out.r.abs() < 1e5,
                    "{rate} {n}: {out:?}"
                );
            }
        }
    }

    #[test]
    fn a_tail_decays_to_exact_silence() {
        let params = params(|b| {
            b[1].gain_db = 12.0;
            b[1].q = 8.0;
        });
        let mut eq = Eq::new(RATE);
        eq.process(Frame::mono(1.0), &params);
        let mut last = Frame::mono(1.0);
        for _ in 0..(RATE as usize * 20) {
            last = eq.process(Frame::SILENT, &params);
        }
        assert_eq!(last, Frame::SILENT);
    }

    /// A band that is dragged eases to its new place, with no jump in the output.
    #[test]
    fn moving_a_band_does_not_click() {
        let moving = |gain: f32| {
            params(|b| {
                b[1].gain_db = gain;
                b[1].freq_hz = 300.0;
            })
        };
        let mut eq = Eq::new(RATE);
        let mut previous = 0.0f32;
        let mut worst = 0.0f32;
        for n in 0..20000 {
            let x = (std::f32::consts::TAU * 300.0 * n as f32 / RATE).sin() * 0.5;
            let params = if n < 10000 { moving(0.0) } else { moving(18.0) };
            let out = eq.process(Frame::mono(x), &params).l;
            worst = worst.max((out - previous).abs());
            previous = out;
        }
        // A 300 Hz sine at 0.5 has a largest step of about 0.02 flat, and 0.16 at the 18 dB end.
        assert!(worst < 0.2, "{worst}");
    }

    /// With the analyzer on, the spectrum shows the output's tone. With it off, it is at the
    /// floor, and switching it off clears what was measured.
    #[test]
    fn the_analyzer_measures_only_while_it_is_on() {
        use crate::dsp::analyzer::FLOOR_DB;
        let mut params = EqParams::default();
        let mut eq = Eq::new(RATE);
        let play = |eq: &mut Eq, params: &EqParams| {
            for n in 0..8192 {
                let x = (std::f32::consts::TAU * 1000.0 * n as f32 / RATE).sin();
                eq.process(Frame::mono(x), params);
            }
        };
        play(&mut eq, &params);
        assert!(eq.spectrum().iter().all(|&db| db == FLOOR_DB));
        params.analyzer = true;
        play(&mut eq, &params);
        assert!(eq.spectrum().iter().any(|&db| db > -6.0));
        params.analyzer = false;
        eq.process(Frame::SILENT, &params);
        assert!(eq.spectrum().iter().all(|&db| db == FLOOR_DB));
    }

    /// The spectrum is of the output, so a boost shows as a taller peak.
    #[test]
    fn the_spectrum_is_post_eq() {
        let level = |gain: f32| {
            let mut params = params(|b| {
                b[1].freq_hz = 1000.0;
                b[1].gain_db = gain;
            });
            params.analyzer = true;
            let mut eq = Eq::new(RATE);
            for n in 0..8192 {
                let x = (std::f32::consts::TAU * 1000.0 * n as f32 / RATE).sin() * 0.25;
                eq.process(Frame::mono(x), &params);
            }
            eq.spectrum().iter().copied().fold(FLOOR_DB_TEST, f32::max)
        };
        const FLOOR_DB_TEST: f32 = -100.0;
        assert!((level(12.0) - level(0.0) - 12.0).abs() < 2.0);
    }
}
