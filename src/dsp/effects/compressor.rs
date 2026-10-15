//! Compressor: a stereo-linked feed-forward compressor, with a multiband mode that gives the
//! upward and downward squash of an OTT.
//!
//! Single band mode. The detector is a mean-square follower on the louder of the two sides, so the
//! stereo image does not shift. The level in dB goes through a static curve with a 6 dB soft knee.
//! The resulting gain is smoothed in the dB domain with separate attack and release time
//! constants. The effect has no lookahead and adds no latency, so the dry path needs no delay.
//!
//! Multiband mode. Two Linkwitz-Riley (24 dB per octave) crossovers at 120 Hz and 2.5 kHz split the
//! signal into low, mid, and high bands. The low band also runs through an all-pass at the upper
//! crossover so the three bands sum back with a flat magnitude. Each band has its own stereo-linked
//! detector and its own gain, built from a downward curve above a high threshold and an upward
//! curve below a low one. `depth` scales both curves in dB, so at 0.0 every band gain is unity.
//! The total band gain is limited to +30 dB.
//!
//! The two modes run side by side while `multiband` changes and crossfade over a few ms. The
//! unused path is reset once its weight reaches zero, so it never wakes up with stale state.

use crate::dsp::frame::Frame;

/// The compressor's controls, in plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompressorParams {
    /// The level above which the signal is turned down, in dB, from -60.0 to 0.0.
    pub threshold_db: f32,
    /// How much the signal above the threshold is turned down, from 1.0 (none) to 20.0.
    pub ratio: f32,
    /// How fast the gain falls when the signal rises past the threshold, in ms, from 0.1 to 100.0.
    pub attack_ms: f32,
    /// How fast the gain comes back, in ms, from 10.0 to 1000.0.
    pub release_ms: f32,
    /// Gain added after compression, in dB, from 0.0 to 24.0.
    pub makeup_db: f32,
    /// Dry/wet, from 0.0 (all dry) to 1.0 (all wet). A mix between them is parallel compression.
    pub mix: f32,
    /// True for the multiband mode.
    pub multiband: bool,
    /// Multiband only. How hard the upward and downward compression work, from 0.0 to 1.0.
    pub depth: f32,
}

impl Default for CompressorParams {
    fn default() -> Self {
        Self {
            threshold_db: -18.0,
            ratio: 4.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            makeup_db: 0.0,
            mix: 1.0,
            multiband: false,
            depth: 1.0,
        }
    }
}

/// The width of the soft knee in dB, centred on the threshold.
const KNEE_DB: f32 = 6.0;
/// The detector floor in dB. A silent input reads as this level.
const FLOOR_DB: f32 = -120.0;
/// The most gain the multiband mode may add in one band, in dB.
const MAX_BOOST_DB: f32 = 30.0;
/// The most gain the multiband mode may remove in one band, in dB.
const MAX_CUT_DB: f32 = -60.0;
/// The threshold knob value that leaves the multiband thresholds at their base values.
const NEUTRAL_THRESHOLD_DB: f32 = -18.0;
/// The upward compression slope, as the fraction of the distance under the threshold that is
/// recovered. 0.667 is a 3:1 upward ratio.
const UPWARD_SLOPE: f32 = 2.0 / 3.0;
/// The time constant of the single band detector in ms.
const SINGLE_DETECTOR_MS: f32 = 4.0;
/// The time constant for smoothing makeup, mix, and depth in ms.
const PARAM_SMOOTH_MS: f32 = 5.0;
/// The length of the crossfade between the two modes in ms.
const MODE_FADE_MS: f32 = 5.0;
/// The conversion from dB to a power of two, since 6.0206 dB is a factor of two in amplitude.
const DB_TO_LOG2: f32 = 1.0 / 6.020_6;
/// The conversion from a base-2 logarithm of a power to dB.
const LOG2_POWER_TO_DB: f32 = 3.010_3;
/// The two crossover frequencies in Hz.
const CROSSOVER_HZ: [f32; 2] = [120.0, 2500.0];

/// The fixed settings of one multiband band.
struct BandSpec {
    /// The downward threshold in dB at the neutral threshold knob.
    down_db: f32,
    /// The upward threshold in dB at the neutral threshold knob.
    up_db: f32,
    /// How the knob's attack and release are scaled for this band.
    time_scale: f32,
    /// The detector time constant in ms.
    detector_ms: f32,
    /// A static gain in dB at full depth. It adds weight to the lows and air to the highs.
    trim_db: f32,
}

/// The low, mid, and high bands. The highs have less energy, so their thresholds sit lower, and
/// they react faster. The lows react slower so the detector does not follow single cycles.
const BANDS: [BandSpec; 3] = [
    BandSpec {
        down_db: -20.0,
        up_db: -32.0,
        time_scale: 2.0,
        detector_ms: 20.0,
        trim_db: 1.0,
    },
    BandSpec {
        down_db: -24.0,
        up_db: -34.0,
        time_scale: 1.0,
        detector_ms: 6.0,
        trim_db: 0.0,
    },
    BandSpec {
        down_db: -28.0,
        up_db: -40.0,
        time_scale: 0.4,
        detector_ms: 2.0,
        trim_db: 2.0,
    },
];

/// A base-2 logarithm good to about 2e-5. The input must be a positive normal number.
fn fast_log2(x: f32) -> f32 {
    let bits = x.to_bits();
    let exponent = ((bits >> 23) & 0xff) as i32 - 127;
    let mantissa = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    let s = (mantissa - 1.0) / (mantissa + 1.0);
    let s2 = s * s;
    let series = 1.0 + s2 * (1.0 / 3.0 + s2 * (1.0 / 5.0 + s2 * (1.0 / 7.0 + s2 * (1.0 / 9.0))));
    exponent as f32 + 2.0 * std::f32::consts::LOG2_E * s * series
}

/// Two raised to `x`, with a relative error under 1e-5.
fn fast_exp2(x: f32) -> f32 {
    let x = x.clamp(-100.0, 100.0);
    let whole = x.floor();
    let f = x - whole;
    let poly = 1.0
        + f * (0.693_147_2
            + f * (0.240_226_5
                + f * (0.055_504_1 + f * (0.009_618_1 + f * (0.001_333_4 + f * 0.000_154_0)))));
    poly * f32::from_bits(((whole as i32 + 127) as u32) << 23)
}

/// Converts a gain in dB to a linear factor.
fn db_to_gain(db: f32) -> f32 {
    fast_exp2(db * DB_TO_LOG2)
}

/// The one-pole coefficient for a time constant in ms. The state moves `1 - coef` of the way to
/// its target each sample.
fn time_coef(ms: f32, sample_rate: f32) -> f32 {
    (-1.0 / (ms.max(0.001) * 0.001 * sample_rate)).exp()
}

/// The soft knee curve. `x` is the distance past the threshold in dB, and the result is `slope * x`
/// outside the knee, zero well before it, and a quadratic blend inside it.
fn knee_curve(x: f32, slope: f32) -> f32 {
    let half = KNEE_DB * 0.5;
    if x <= -half {
        0.0
    } else if x < half {
        let t = x + half;
        slope * t * t / (2.0 * KNEE_DB)
    } else {
        slope * x
    }
}

/// A mean-square level detector with a gain smoother behind it. One follower does the work for
/// one band, or for the whole signal in single band mode.
#[derive(Clone, Copy)]
struct Follower {
    /// The smoothed mean square of the louder side.
    mean_square: f32,
    /// The current gain in dB.
    gain_db: f32,
    /// The attack and release times the cached coefficients were made for.
    cached_attack_ms: f32,
    cached_release_ms: f32,
    attack: f32,
    release: f32,
}

impl Follower {
    const fn new() -> Self {
        Self {
            mean_square: 0.0,
            gain_db: 0.0,
            cached_attack_ms: -1.0,
            cached_release_ms: -1.0,
            attack: 0.0,
            release: 0.0,
        }
    }

    /// Feeds one stereo frame to the detector and returns its level in dB.
    fn level_db(&mut self, frame: Frame, detector_coef: f32) -> f32 {
        let power = (frame.l * frame.l).max(frame.r * frame.r);
        self.mean_square = power + detector_coef * (self.mean_square - power);
        if self.mean_square < 1e-20 {
            self.mean_square = 0.0;
        }
        if self.mean_square <= 1e-12 {
            FLOOR_DB
        } else {
            fast_log2(self.mean_square) * LOG2_POWER_TO_DB
        }
    }

    /// Moves the gain toward `target_db`, falling at the attack rate and rising at the release
    /// rate, and returns it.
    fn smooth(&mut self, target_db: f32, attack_ms: f32, release_ms: f32, sample_rate: f32) -> f32 {
        if attack_ms != self.cached_attack_ms || release_ms != self.cached_release_ms {
            self.cached_attack_ms = attack_ms;
            self.cached_release_ms = release_ms;
            self.attack = time_coef(attack_ms, sample_rate);
            self.release = time_coef(release_ms, sample_rate);
        }
        let coef = if target_db < self.gain_db {
            self.attack
        } else {
            self.release
        };
        self.gain_db = target_db + coef * (self.gain_db - target_db);
        if self.gain_db.abs() < 1e-20 {
            self.gain_db = 0.0;
        }
        self.gain_db
    }
}

/// The filter coefficients of one second-order section.
#[derive(Clone, Copy)]
struct Coefs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Coefs {
    /// A second-order Butterworth section (Q of 1/sqrt 2) of the given shape at `hz`.
    fn new(shape: Shape, hz: f32, sample_rate: f32) -> Self {
        let w0 = std::f64::consts::TAU * f64::from(hz) / f64::from(sample_rate);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let (b0, b1, b2) = match shape {
            Shape::LowPass => ((1.0 - cos) * 0.5, 1.0 - cos, (1.0 - cos) * 0.5),
            Shape::HighPass => ((1.0 + cos) * 0.5, -(1.0 + cos), (1.0 + cos) * 0.5),
            Shape::AllPass => (1.0 - alpha, -2.0 * cos, 1.0 + alpha),
        };
        let a0 = 1.0 + alpha;
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
        }
    }
}

/// The response shape of a second-order section.
#[derive(Clone, Copy)]
enum Shape {
    LowPass,
    HighPass,
    AllPass,
}

/// A second-order section in transposed direct form II. The state is f64 so a 120 Hz corner stays
/// accurate at 192 kHz.
#[derive(Clone, Copy)]
struct Biquad {
    c: Coefs,
    z1: f64,
    z2: f64,
}

impl Biquad {
    fn new(c: Coefs) -> Self {
        Self {
            c,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn run(&mut self, x: f64) -> f64 {
        let c = self.c;
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        if self.z1.abs() < 1e-30 {
            self.z1 = 0.0;
        }
        if self.z2.abs() < 1e-30 {
            self.z2 = 0.0;
        }
        y
    }
}

/// Runs a sample through two sections in series.
fn cascade(pair: &mut [Biquad; 2], x: f64) -> f64 {
    let y = pair[0].run(x);
    pair[1].run(y)
}

/// The coefficients for the whole crossover. They depend on the sample rate only.
#[derive(Clone, Copy)]
struct CrossoverCoefs {
    low_pass: [Coefs; 2],
    high_pass: [Coefs; 2],
    all_pass: Coefs,
}

impl CrossoverCoefs {
    fn new(sample_rate: f32) -> Self {
        // Keep both corners under Nyquist at the lowest sample rates.
        let nyquist_limit = sample_rate * 0.45;
        let low = CROSSOVER_HZ[0].min(nyquist_limit);
        let high = CROSSOVER_HZ[1].min(nyquist_limit);
        Self {
            low_pass: [
                Coefs::new(Shape::LowPass, low, sample_rate),
                Coefs::new(Shape::LowPass, high, sample_rate),
            ],
            high_pass: [
                Coefs::new(Shape::HighPass, low, sample_rate),
                Coefs::new(Shape::HighPass, high, sample_rate),
            ],
            all_pass: Coefs::new(Shape::AllPass, high, sample_rate),
        }
    }
}

/// A three-band Linkwitz-Riley splitter for one channel. Each Linkwitz-Riley filter is two
/// Butterworth sections in series. Low plus high of one crossover is a second-order all-pass, so
/// the low band takes the same all-pass at the upper corner and the three bands sum flat.
#[derive(Clone, Copy)]
struct Crossover {
    low_a: [Biquad; 2],
    low_b: [Biquad; 2],
    rest_a: [Biquad; 2],
    rest_b: [Biquad; 2],
    low_all_pass: Biquad,
}

impl Crossover {
    fn new(coefs: &CrossoverCoefs) -> Self {
        let pair = |c: Coefs| [Biquad::new(c), Biquad::new(c)];
        Self {
            low_a: pair(coefs.low_pass[0]),
            low_b: pair(coefs.high_pass[0]),
            rest_a: pair(coefs.low_pass[1]),
            rest_b: pair(coefs.high_pass[1]),
            low_all_pass: Biquad::new(coefs.all_pass),
        }
    }

    /// Splits one sample into the low, mid, and high bands.
    fn split(&mut self, x: f32) -> [f32; 3] {
        let x = f64::from(x);
        let low = cascade(&mut self.low_a, x);
        let rest = cascade(&mut self.low_b, x);
        let low = self.low_all_pass.run(low);
        let mid = cascade(&mut self.rest_a, rest);
        let high = cascade(&mut self.rest_b, rest);
        [low as f32, mid as f32, high as f32]
    }
}

/// The controls after clamping to their ranges.
struct Settings {
    threshold_db: f32,
    ratio: f32,
    attack_ms: f32,
    release_ms: f32,
    makeup_db: f32,
    mix: f32,
    multiband: bool,
    depth: f32,
}

impl Settings {
    fn new(p: &CompressorParams) -> Self {
        let clean = |x: f32, lo: f32, hi: f32| if x.is_finite() { x.clamp(lo, hi) } else { lo };
        Self {
            threshold_db: clean(p.threshold_db, -60.0, 0.0),
            ratio: clean(p.ratio, 1.0, 20.0),
            attack_ms: clean(p.attack_ms, 0.1, 100.0),
            release_ms: clean(p.release_ms, 10.0, 1000.0),
            makeup_db: clean(p.makeup_db, 0.0, 24.0),
            mix: clean(p.mix, 0.0, 1.0),
            multiband: p.multiband,
            depth: clean(p.depth, 0.0, 1.0),
        }
    }
}

/// The compressor. It holds the detector, crossover, and smoothing state for one stereo signal.
pub struct Compressor {
    sample_rate: f32,
    /// The smoothing coefficient for makeup, mix, and depth.
    param_coef: f32,
    /// How far the mode weight moves per sample.
    fade_step: f32,
    /// The single band detector coefficient.
    single_detector: f32,
    /// The detector coefficient of each multiband band.
    band_detectors: [f32; 3],
    crossover_coefs: CrossoverCoefs,
    crossovers: [Crossover; 2],
    single: Follower,
    bands: [Follower; 3],
    /// Smoothed copies of the controls that would click if they jumped.
    makeup_db: f32,
    mix: f32,
    depth: f32,
    /// The weight of the multiband path, from 0.0 (single band) to 1.0 (multiband).
    multiband_weight: f32,
    /// True once the first sample has set the smoothed values to the controls.
    primed: bool,
}

impl Compressor {
    /// Builds the compressor for a sample rate. It needs no memory beyond its own fields.
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.clamp(8000.0, 384_000.0);
        let crossover_coefs = CrossoverCoefs::new(sample_rate);
        Self {
            sample_rate,
            param_coef: time_coef(PARAM_SMOOTH_MS, sample_rate),
            fade_step: 1.0 / (MODE_FADE_MS * 0.001 * sample_rate),
            single_detector: time_coef(SINGLE_DETECTOR_MS, sample_rate),
            band_detectors: BANDS.map(|b| time_coef(b.detector_ms, sample_rate)),
            crossover_coefs,
            crossovers: [Crossover::new(&crossover_coefs); 2],
            single: Follower::new(),
            bands: [Follower::new(); 3],
            makeup_db: 0.0,
            mix: 1.0,
            depth: 1.0,
            multiband_weight: 0.0,
            primed: false,
        }
    }

    /// Clears the detectors, filters, and gains. The next sample sets the smoothed controls again.
    pub fn reset(&mut self) {
        *self = Self::new(self.sample_rate);
    }

    /// One stereo sample.
    pub fn process(&mut self, input: Frame, params: &CompressorParams) -> Frame {
        let p = Settings::new(params);
        let input = input.map(|x| if x.is_finite() { x } else { 0.0 });

        let target_weight = if p.multiband { 1.0 } else { 0.0 };
        if !self.primed {
            self.primed = true;
            self.makeup_db = p.makeup_db;
            self.mix = p.mix;
            self.depth = p.depth;
            self.multiband_weight = target_weight;
        } else {
            self.makeup_db = smooth_toward(self.makeup_db, p.makeup_db, self.param_coef);
            self.mix = smooth_toward(self.mix, p.mix, self.param_coef);
            self.depth = smooth_toward(self.depth, p.depth, self.param_coef);
            let w = self.multiband_weight;
            self.multiband_weight = if w < target_weight {
                (w + self.fade_step).min(target_weight)
            } else {
                (w - self.fade_step).max(target_weight)
            };
        }
        let weight = self.multiband_weight;

        let single = if weight < 1.0 {
            self.process_single(input, &p)
        } else {
            self.single = Follower::new();
            Frame::SILENT
        };
        let multi = if weight > 0.0 {
            self.process_multiband(input, &p)
        } else {
            self.bands = [Follower::new(); 3];
            self.crossovers = [Crossover::new(&self.crossover_coefs); 2];
            Frame::SILENT
        };
        let wet = if weight <= 0.0 {
            single
        } else if weight >= 1.0 {
            multi
        } else {
            single * (1.0 - weight) + multi * weight
        };

        let wet_gain = db_to_gain(self.makeup_db) * self.mix;
        let out = input * (1.0 - self.mix) + wet * wet_gain;
        out.map(|x| if x.is_finite() { x } else { 0.0 })
    }

    /// The single band path, before makeup and mix.
    fn process_single(&mut self, input: Frame, p: &Settings) -> Frame {
        let level = self.single.level_db(input, self.single_detector);
        let slope = 1.0 / p.ratio - 1.0;
        let target = knee_curve(level - p.threshold_db, slope);
        let gain_db = self
            .single
            .smooth(target, p.attack_ms, p.release_ms, self.sample_rate);
        input * db_to_gain(gain_db)
    }

    /// The multiband path, before makeup and mix.
    fn process_multiband(&mut self, input: Frame, p: &Settings) -> Frame {
        let left = self.crossovers[0].split(input.l);
        let right = self.crossovers[1].split(input.r);
        let shift = p.threshold_db - NEUTRAL_THRESHOLD_DB;
        let down_slope = 1.0 / p.ratio - 1.0;
        let mut out = Frame::SILENT;
        for (i, spec) in BANDS.iter().enumerate() {
            let band = Frame::new(left[i], right[i]);
            let follower = &mut self.bands[i];
            let level = follower.level_db(band, self.band_detectors[i]);
            let down = knee_curve(level - (spec.down_db + shift), down_slope);
            let up = knee_curve((spec.up_db + shift) - level, UPWARD_SLOPE);
            let target = (self.depth * (down + up + spec.trim_db)).clamp(MAX_CUT_DB, MAX_BOOST_DB);
            let gain_db = follower.smooth(
                target,
                p.attack_ms * spec.time_scale,
                p.release_ms * spec.time_scale,
                self.sample_rate,
            );
            out += band * db_to_gain(gain_db.min(MAX_BOOST_DB));
        }
        out
    }
}

/// Moves `value` toward `target` by one smoothing step, and lands on it once it is close.
fn smooth_toward(value: f32, target: f32, coef: f32) -> f32 {
    let next = target + coef * (value - target);
    if (next - target).abs() < 1e-6 {
        target
    } else {
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44100.0;

    fn sine(i: usize, hz: f32, amp: f32, sr: f32) -> f32 {
        amp * (std::f32::consts::TAU * hz * i as f32 / sr).sin()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn db(x: f32) -> f32 {
        20.0 * x.max(1e-12).log10()
    }

    /// A deterministic noise source in [-1, 1].
    struct Noise(u32);

    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (self.0 >> 8) as f32 / (1u32 << 23) as f32 - 1.0
        }
    }

    /// Runs a mono sine through the compressor and returns the output of the left side.
    fn run_sine(c: &mut Compressor, p: &CompressorParams, hz: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| c.process(Frame::mono(sine(i, hz, amp, c.sample_rate)), p).l)
            .collect()
    }

    /// The gain in dB of the last half second of a steady sine through a fresh compressor.
    fn steady_gain_db(p: &CompressorParams, hz: f32, amp: f32, sr: f32) -> f32 {
        let mut c = Compressor::new(sr);
        let n = (sr * 1.5) as usize;
        let out = run_sine(&mut c, p, hz, amp, n);
        let tail = (sr * 0.5) as usize;
        let input: Vec<f32> = (n - tail..n).map(|i| sine(i, hz, amp, sr)).collect();
        db(rms(&out[n - tail..])) - db(rms(&input))
    }

    fn single(threshold_db: f32, ratio: f32) -> CompressorParams {
        CompressorParams {
            threshold_db,
            ratio,
            attack_ms: 5.0,
            release_ms: 50.0,
            ..CompressorParams::default()
        }
    }

    fn multi() -> CompressorParams {
        CompressorParams {
            multiband: true,
            ..CompressorParams::default()
        }
    }

    #[test]
    fn fast_math_is_accurate() {
        for k in 0..2000 {
            let x = 1e-12 * 10f32.powf(k as f32 * 0.006);
            let err = (fast_log2(x) - x.log2()).abs();
            assert!(err < 5e-5, "log2({x}) off by {err}");
        }
        for k in -1000..1000 {
            let x = k as f32 * 0.05;
            let rel = (fast_exp2(x) / x.exp2() - 1.0).abs();
            assert!(rel < 2e-5, "exp2({x}) off by {rel}");
        }
        assert_eq!(fast_exp2(0.0), 1.0);
    }

    /// The RMS gain of the summed crossover bands for a sine, in dB.
    fn crossover_sum_gain_db(hz: f32, sr: f32) -> f32 {
        let coefs = CrossoverCoefs::new(sr);
        let mut x = Crossover::new(&coefs);
        let n = (sr * 0.6) as usize;
        let mut sum = Vec::with_capacity(n);
        let mut input = Vec::with_capacity(n);
        for i in 0..n {
            let s = sine(i, hz, 0.5, sr);
            let b = x.split(s);
            sum.push(b[0] + b[1] + b[2]);
            input.push(s);
        }
        let from = n / 2;
        db(rms(&sum[from..])) - db(rms(&input[from..]))
    }

    #[test]
    fn crossover_bands_sum_flat() {
        for sr in [22050.0, 44100.0, 48000.0, 96000.0, 192000.0] {
            for hz in [
                40.0, 80.0, 120.0, 200.0, 500.0, 1000.0, 2000.0, 2500.0, 3500.0, 6000.0, 9000.0,
            ] {
                if hz > sr * 0.4 {
                    continue;
                }
                let gain = crossover_sum_gain_db(hz, sr);
                assert!(gain.abs() < 0.1, "{hz} Hz at {sr}: {gain} dB");
            }
        }
    }

    #[test]
    fn crossover_bands_isolate_their_ranges() {
        let coefs = CrossoverCoefs::new(SR);
        let band_rms = |hz: f32| {
            let mut x = Crossover::new(&coefs);
            let mut bands = [Vec::new(), Vec::new(), Vec::new()];
            for i in 0..SR as usize {
                let b = x.split(sine(i, hz, 0.5, SR));
                for k in 0..3 {
                    bands[k].push(b[k]);
                }
            }
            [0, 1, 2].map(|k| db(rms(&bands[k][SR as usize / 2..])))
        };
        let low = band_rms(30.0);
        assert!(low[0] > -9.5 && low[1] < -45.0 && low[2] < -70.0, "{low:?}");
        let mid = band_rms(600.0);
        assert!(mid[1] > -9.5 && mid[0] < -30.0 && mid[2] < -30.0, "{mid:?}");
        let high = band_rms(12000.0);
        assert!(
            high[2] > -9.5 && high[1] < -45.0 && high[0] < -70.0,
            "{high:?}"
        );
        // At a crossover frequency the two neighbours are each 6 dB down.
        let at = band_rms(120.0);
        assert!((at[0] - at[1] - 0.0).abs() < 0.5, "{at:?}");
        assert!(
            (at[0] - (db(0.5 / 2f32.sqrt()) - 6.02)).abs() < 0.5,
            "{at:?}"
        );
    }

    #[test]
    fn ratio_one_is_transparent() {
        let p = CompressorParams {
            ratio: 1.0,
            threshold_db: -60.0,
            ..CompressorParams::default()
        };
        let mut c = Compressor::new(SR);
        for i in 0..20000 {
            let x = Frame::new(sine(i, 330.0, 0.9, SR), sine(i, 1234.0, 0.7, SR));
            let y = c.process(x, &p);
            assert!((y.l - x.l).abs() < 1e-6 && (y.r - x.r).abs() < 1e-6, "{i}");
        }
    }

    #[test]
    fn a_threshold_above_the_signal_is_transparent() {
        let p = single(0.0, 20.0);
        let g = steady_gain_db(&p, 1000.0, 0.1, SR);
        assert!(g.abs() < 0.01, "{g}");
    }

    #[test]
    fn makeup_adds_gain_after_compression() {
        for makeup in [3.0, 6.0, 12.0, 24.0] {
            let p = CompressorParams {
                ratio: 1.0,
                makeup_db: makeup,
                ..CompressorParams::default()
            };
            let g = steady_gain_db(&p, 700.0, 0.05, SR);
            assert!((g - makeup).abs() < 0.02, "{makeup}: {g}");
        }
        // Makeup stacks on top of the reduction.
        let base = steady_gain_db(&single(-20.0, 4.0), 1000.0, 0.5, SR);
        let boosted = steady_gain_db(
            &CompressorParams {
                makeup_db: 6.0,
                ..single(-20.0, 4.0)
            },
            1000.0,
            0.5,
            SR,
        );
        assert!((boosted - base - 6.0).abs() < 0.05, "{base} {boosted}");
    }

    /// The expected gain in dB for a sine of peak `amp`, ignoring the knee.
    fn expected_gain_db(amp: f32, threshold_db: f32, ratio: f32) -> f32 {
        let level = db(amp / 2f32.sqrt());
        (1.0 / ratio - 1.0) * (level - threshold_db)
    }

    #[test]
    fn gain_reduction_follows_the_ratio_and_threshold() {
        for (threshold, ratio, amp) in [
            (-20.0, 4.0, 0.5),
            (-30.0, 2.0, 0.5),
            (-12.0, 10.0, 0.9),
            (-40.0, 20.0, 0.25),
            (-24.0, 3.0, 0.3),
        ] {
            let want = expected_gain_db(amp, threshold, ratio);
            assert!(want < -2.0, "the case should sit well past the knee");
            let got = steady_gain_db(&single(threshold, ratio), 1000.0, amp, SR);
            assert!(
                (got - want).abs() < 0.25,
                "thr {threshold} ratio {ratio}: want {want}, got {got}"
            );
        }
    }

    #[test]
    fn the_knee_blends_in_smoothly() {
        // The static curve is continuous at both ends of the knee.
        let slope = 1.0 / 4.0 - 1.0;
        assert_eq!(knee_curve(-3.0, slope), 0.0);
        assert!((knee_curve(0.0, slope) - (-0.5625)).abs() < 1e-6);
        assert!((knee_curve(3.0, slope) - slope * 3.0).abs() < 1e-6);
        assert!((knee_curve(3.001, slope) - slope * 3.001).abs() < 1e-3);

        // Measured at the threshold, the reduction is the half-knee value.
        let amp = 0.5;
        let level = db(amp / 2f32.sqrt());
        let at = steady_gain_db(&single(level, 4.0), 1000.0, amp, SR);
        assert!((at - (-0.5625)).abs() < 0.12, "at threshold: {at}");
        // Four dB under the threshold the knee has not started.
        let under = steady_gain_db(&single(level + 4.0, 4.0), 1000.0, amp, SR);
        assert!(under.abs() < 0.03, "under: {under}");
        // Reduction grows monotonically through the knee.
        let mut last = -100.0;
        for offset in [-2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 5.0] {
            let g = steady_gain_db(&single(level + offset, 4.0), 1000.0, amp, SR);
            assert!(g >= last - 0.02, "offset {offset}: {g} after {last}");
            last = g;
        }
    }

    /// Feeds a step from a quiet sine to a loud one and back, and records the gain in dB.
    fn gain_trace(p: &CompressorParams, loud_samples: usize, quiet_samples: usize) -> Vec<f32> {
        let mut c = Compressor::new(SR);
        let mut trace = Vec::new();
        for i in 0..quiet_samples / 4 {
            c.process(Frame::mono(sine(i, 1000.0, 0.001, SR)), p);
        }
        for i in 0..loud_samples {
            c.process(Frame::mono(sine(i, 1000.0, 0.8, SR)), p);
            trace.push(c.single.gain_db);
        }
        for i in 0..quiet_samples {
            c.process(Frame::mono(sine(i, 1000.0, 0.001, SR)), p);
            trace.push(c.single.gain_db);
        }
        trace
    }

    /// The first index where the trace has covered 63.2 percent of the way from `from` to `to`.
    fn time_constant_ms(trace: &[f32], start: usize, from: f32, to: f32) -> f32 {
        let goal = from + (to - from) * 0.632;
        let idx = trace[start..]
            .iter()
            .position(|&g| if to < from { g <= goal } else { g >= goal })
            .expect("never reached the goal");
        idx as f32 / SR * 1000.0
    }

    #[test]
    fn attack_time_is_about_right() {
        for attack in [10.0, 30.0, 60.0] {
            let p = CompressorParams {
                attack_ms: attack,
                release_ms: 200.0,
                ..single(-30.0, 8.0)
            };
            let loud = (SR * 0.5) as usize;
            let trace = gain_trace(&p, loud, 10);
            let final_gain = trace[loud - 1];
            assert!(final_gain < -10.0, "{final_gain}");
            let t = time_constant_ms(&trace, 0, 0.0, final_gain);
            assert!(
                t > attack * 0.8 && t < attack * 1.35 + 3.0,
                "attack {attack} ms measured {t} ms"
            );
        }
    }

    #[test]
    fn release_time_is_about_right() {
        for release in [50.0, 150.0, 400.0] {
            let p = CompressorParams {
                attack_ms: 2.0,
                release_ms: release,
                ..single(-30.0, 8.0)
            };
            let loud = (SR * 0.5) as usize;
            let trace = gain_trace(&p, loud, (SR * 2.0) as usize);
            let bottom = trace[loud - 1];
            // The gain target is zero once the level is under the knee, which takes the detector
            // a few ms, so the measurement starts from the first sample of the quiet part.
            let t = time_constant_ms(&trace, loud, bottom, 0.0);
            assert!(
                t > release * 0.8 && t < release * 1.3 + 5.0,
                "release {release} ms measured {t} ms"
            );
        }
    }

    #[test]
    fn the_gain_moves_in_the_log_domain() {
        // A 20 dB step with an attack of 20 ms. In dB the first time constant covers 63 percent of
        // the step. A linear-domain smoother would pass through 1 - 0.632 of the amplitude, which
        // is a very different trace, so the dB value at one time constant pins the domain.
        let p = CompressorParams {
            attack_ms: 20.0,
            ..single(-40.0, 20.0)
        };
        let trace = gain_trace(&p, (SR * 0.5) as usize, 10);
        let end = trace[(SR * 0.5) as usize - 1];
        let at_tau = trace[(SR * 0.020) as usize + (SR * 0.004) as usize];
        let fraction = at_tau / end;
        assert!(fraction > 0.5 && fraction < 0.72, "{fraction}");
    }

    #[test]
    fn mix_zero_is_the_exact_dry_input() {
        for multiband in [false, true] {
            let p = CompressorParams {
                mix: 0.0,
                multiband,
                makeup_db: 12.0,
                ratio: 20.0,
                threshold_db: -50.0,
                ..CompressorParams::default()
            };
            let mut c = Compressor::new(SR);
            let mut noise = Noise(7);
            for i in 0..20000 {
                let x = Frame::new(noise.next() * 4.0 + sine(i, 90.0, 2.0, SR), noise.next());
                assert_eq!(c.process(x, &p), x, "multiband {multiband}, sample {i}");
            }
        }
    }

    #[test]
    fn mix_blends_dry_and_wet() {
        let run = |mix: f32| {
            let p = CompressorParams {
                mix,
                ..single(-30.0, 8.0)
            };
            let mut c = Compressor::new(SR);
            run_sine(&mut c, &p, 500.0, 0.6, 20000)
        };
        let wet = run(1.0);
        let half = run(0.5);
        for i in 0..wet.len() {
            let dry = sine(i, 500.0, 0.6, SR);
            assert!((half[i] - (0.5 * dry + 0.5 * wet[i])).abs() < 1e-5, "{i}");
        }
    }

    #[test]
    fn the_stereo_link_applies_one_gain_to_both_sides() {
        let p = single(-30.0, 8.0);
        let mut c = Compressor::new(SR);
        let (mut out_l, mut out_r, mut in_l, mut in_r) = (vec![], vec![], vec![], vec![]);
        for i in 0..44100 {
            let x = Frame::new(sine(i, 300.0, 0.8, SR), sine(i, 900.0, 0.02, SR));
            let y = c.process(x, &p);
            if i > 22050 {
                out_l.push(y.l);
                out_r.push(y.r);
                in_l.push(x.l);
                in_r.push(x.r);
            }
        }
        let gain_l = db(rms(&out_l)) - db(rms(&in_l));
        let gain_r = db(rms(&out_r)) - db(rms(&in_r));
        assert!(gain_l < -10.0, "{gain_l}");
        assert!((gain_l - gain_r).abs() < 0.3, "{gain_l} vs {gain_r}");
    }

    /// The RMS in dB of a steady sine through the multiband path, from a fresh compressor.
    fn multiband_gain_db(p: &CompressorParams, hz: f32, amp: f32, sr: f32) -> f32 {
        steady_gain_db(p, hz, amp, sr)
    }

    #[test]
    fn multiband_at_depth_zero_passes_the_signal() {
        let p = CompressorParams {
            depth: 0.0,
            ..multi()
        };
        for amp in [0.5, 0.01, 0.0001] {
            for hz in [60.0, 120.0, 400.0, 2500.0, 5000.0, 10000.0] {
                let g = multiband_gain_db(&p, hz, amp, SR);
                assert!(g.abs() < 0.1, "{hz} Hz amp {amp}: {g} dB");
            }
        }
    }

    /// A signal of alternating loud and quiet sine segments, returning the in and out RMS of the
    /// last full loud segment and the last full quiet segment, in dB.
    fn loud_quiet_levels(p: &CompressorParams, hz: f32) -> ([f32; 2], [f32; 2]) {
        let seg = (SR * 0.6) as usize;
        let tail = (SR * 0.15) as usize;
        let mut c = Compressor::new(SR);
        let mut result = ([0.0; 2], [0.0; 2]);
        for round in 0..4 {
            for (k, amp) in [0.5, 0.005].into_iter().enumerate() {
                let (mut i_buf, mut o_buf) = (vec![], vec![]);
                for i in 0..seg {
                    let x = sine(i, hz, amp, SR);
                    let y = c.process(Frame::mono(x), p).l;
                    if i >= seg - tail {
                        i_buf.push(x);
                        o_buf.push(y);
                    }
                }
                if round == 3 {
                    let pair = [db(rms(&i_buf)), db(rms(&o_buf))];
                    if k == 0 {
                        result.0 = pair;
                    } else {
                        result.1 = pair;
                    }
                }
            }
        }
        result
    }

    #[test]
    fn full_depth_shrinks_the_dynamic_range() {
        for hz in [80.0, 1000.0, 8000.0] {
            let (loud, quiet) = loud_quiet_levels(&multi(), hz);
            let in_range = loud[0] - quiet[0];
            let out_range = loud[1] - quiet[1];
            assert!(quiet[1] > quiet[0] + 3.0, "{hz} Hz quiet: {quiet:?}");
            assert!(loud[1] < loud[0] - 3.0, "{hz} Hz loud: {loud:?}");
            assert!(
                out_range < in_range * 0.6,
                "{hz} Hz range {in_range} dB became {out_range} dB"
            );
        }
    }

    #[test]
    fn depth_scales_the_effect() {
        let range_at = |depth: f32| {
            let (loud, quiet) = loud_quiet_levels(&CompressorParams { depth, ..multi() }, 1000.0);
            loud[1] - quiet[1]
        };
        let (r0, r5, r10) = (range_at(0.0), range_at(0.5), range_at(1.0));
        assert!(r0 > 39.0, "{r0}");
        assert!(r5 < r0 - 5.0 && r10 < r5 - 5.0, "{r0} {r5} {r10}");
    }

    #[test]
    fn multiband_bands_work_independently() {
        // A quiet 8 kHz tone gets the same lift with or without a loud 60 Hz bass under it.
        let level_at_8k = |bass: f32| {
            let mut c = Compressor::new(SR);
            let p = multi();
            let n = SR as usize * 2;
            let mut out = Vec::new();
            for i in 0..n {
                let x = sine(i, 8000.0, 0.002, SR) + sine(i, 60.0, bass, SR);
                let y = c.process(Frame::mono(x), &p).l;
                if i >= n - 8192 {
                    out.push(y);
                }
            }
            goertzel_db(&out, 8000.0)
        };
        let alone = level_at_8k(0.0);
        let with_bass = level_at_8k(0.7);
        assert!((alone - with_bass).abs() < 1.0, "{alone} vs {with_bass}");
        // And the lift is real.
        assert!(alone > db(0.002 / 2f32.sqrt()) + 5.0, "{alone}");
    }

    /// The level in dB of one frequency in a buffer, as the RMS of that component.
    fn goertzel_db(x: &[f32], hz: f32) -> f32 {
        let w = std::f32::consts::TAU * hz / SR;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, v) in x.iter().enumerate() {
            re += f64::from(*v) * f64::from(w * i as f32).cos();
            im += f64::from(*v) * f64::from(w * i as f32).sin();
        }
        let amp = 2.0 * (re * re + im * im).sqrt() / x.len() as f64;
        db((amp / 2f64.sqrt()) as f32)
    }

    #[test]
    fn the_boost_is_limited() {
        for hz in [100.0, 1000.0, 8000.0] {
            for amp in [1e-3, 1e-4, 1e-6] {
                let p = CompressorParams {
                    ratio: 20.0,
                    // A high threshold knob moves the upward thresholds up, so the boost has room
                    // to reach the limit.
                    threshold_db: 0.0,
                    ..multi()
                };
                let g = multiband_gain_db(&p, hz, amp, SR);
                assert!(g < 30.5, "{hz} Hz amp {amp}: {g} dB");
                assert!(g > 5.0, "{hz} Hz amp {amp}: {g} dB should lift");
            }
        }
        // The limit is on the gain, so a lower threshold knob cannot get past it either.
        let p = CompressorParams {
            threshold_db: 0.0,
            ..multi()
        };
        assert!(multiband_gain_db(&p, 1000.0, 1e-5, SR) < 30.5);
    }

    #[test]
    fn the_threshold_knob_shifts_the_multiband_thresholds() {
        let g_low = multiband_gain_db(
            &CompressorParams {
                threshold_db: -40.0,
                ..multi()
            },
            1000.0,
            0.2,
            SR,
        );
        let g_high = multiband_gain_db(
            &CompressorParams {
                threshold_db: -5.0,
                ..multi()
            },
            1000.0,
            0.2,
            SR,
        );
        assert!(g_low < g_high - 5.0, "{g_low} {g_high}");
    }

    /// The largest sample-to-sample step of a buffer.
    fn max_step(x: &[f32]) -> f32 {
        x.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    }

    #[test]
    fn toggling_multiband_does_not_click() {
        for (from, to) in [(false, true), (true, false)] {
            let mut p = CompressorParams {
                multiband: from,
                ..CompressorParams::default()
            };
            let mut c = Compressor::new(SR);
            let toggle_at = 30000;
            let n = 40000;
            let mut out = Vec::new();
            let mut input = Vec::new();
            for i in 0..n {
                if i == toggle_at {
                    p.multiband = to;
                }
                let x = sine(i, 220.0, 0.3, SR) + sine(i, 3000.0, 0.1, SR);
                input.push(x);
                out.push(c.process(Frame::mono(x), &p).l);
            }
            let around = &out[toggle_at - 50..toggle_at + 600];
            let allowed = max_step(&input) * 2.0;
            assert!(
                max_step(around) < allowed,
                "{from}->{to}: step {} over {allowed}",
                max_step(around)
            );
        }
    }

    #[test]
    fn a_param_jump_does_not_click_either() {
        let mut p = CompressorParams {
            makeup_db: 0.0,
            mix: 1.0,
            ratio: 1.0,
            ..CompressorParams::default()
        };
        let mut c = Compressor::new(SR);
        let mut out = Vec::new();
        for i in 0..20000 {
            if i == 10000 {
                p.makeup_db = 24.0;
                p.mix = 0.3;
            }
            out.push(c.process(Frame::mono(sine(i, 200.0, 0.02, SR)), &p).l);
        }
        // Without smoothing the jump from x1 to x4.7 on a 0.02 sine at 200 Hz is a step of about
        // 0.05. With 5 ms smoothing, no step beats the signal's own slope by much.
        assert!(max_step(&out[9990..10600]) < 0.02 * 5.0 * 0.0286 * 1.5);
    }

    #[test]
    fn output_is_finite_at_param_extremes() {
        let mut noise = Noise(99);
        let mut count = 0;
        for bits in 0..256u32 {
            let pick = |bit: u32, lo: f32, hi: f32| if bits >> bit & 1 == 0 { lo } else { hi };
            let p = CompressorParams {
                threshold_db: pick(0, -60.0, 0.0),
                ratio: pick(1, 1.0, 20.0),
                attack_ms: pick(2, 0.1, 100.0),
                release_ms: pick(3, 10.0, 1000.0),
                makeup_db: pick(4, 0.0, 24.0),
                mix: pick(5, 0.0, 1.0),
                multiband: bits >> 6 & 1 == 1,
                depth: pick(7, 0.0, 1.0),
            };
            for sr in [22050.0, 192000.0] {
                let mut c = Compressor::new(sr);
                for i in 0..3000 {
                    let x = match (i / 500) % 3 {
                        0 => Frame::new(noise.next() * 8.0, noise.next() * 8.0),
                        1 => Frame::new(8.0, -8.0),
                        _ => Frame::new(noise.next() * 1e-6, 0.0),
                    };
                    let y = c.process(x, &p);
                    assert!(y.l.is_finite() && y.r.is_finite(), "{p:?} at {i}");
                    assert!(y.l.abs() < 1e5 && y.r.abs() < 1e5, "{p:?} at {i}: {y:?}");
                    count += 1;
                }
            }
        }
        assert!(count > 0);
    }

    #[test]
    fn output_is_finite_when_params_jump_every_sample() {
        let mut c = Compressor::new(SR);
        let mut noise = Noise(5);
        for i in 0..20000 {
            let hi = i % 2 == 0;
            let p = CompressorParams {
                threshold_db: if hi { 0.0 } else { -60.0 },
                ratio: if hi { 20.0 } else { 1.0 },
                attack_ms: if hi { 0.1 } else { 100.0 },
                release_ms: if hi { 10.0 } else { 1000.0 },
                makeup_db: if hi { 24.0 } else { 0.0 },
                mix: if hi { 1.0 } else { 0.0 },
                multiband: i % 3 == 0,
                depth: if hi { 1.0 } else { 0.0 },
            };
            let y = c.process(Frame::new(noise.next() * 8.0, noise.next() * 8.0), &p);
            assert!(y.l.is_finite() && y.r.is_finite(), "{i}");
        }
    }

    #[test]
    fn non_finite_input_does_not_poison_the_state() {
        let mut c = Compressor::new(SR);
        let p = multi();
        c.process(Frame::new(f32::NAN, f32::INFINITY), &p);
        for i in 0..1000 {
            let y = c.process(Frame::mono(sine(i, 500.0, 0.3, SR)), &p);
            assert!(y.l.is_finite() && y.r.is_finite());
        }
    }

    #[test]
    fn reset_clears_all_state() {
        for multiband in [false, true] {
            let p = CompressorParams {
                multiband,
                ..single(-40.0, 10.0)
            };
            let mut used = Compressor::new(SR);
            run_sine(&mut used, &p, 100.0, 0.9, 20000);
            used.reset();
            let mut fresh = Compressor::new(SR);
            for i in 0..2000 {
                let x = Frame::mono(sine(i, 700.0, 0.2, SR));
                assert_eq!(used.process(x, &p), fresh.process(x, &p), "{i}");
            }
        }
    }

    #[test]
    fn silence_decays_without_denormals() {
        let p = multi();
        let mut c = Compressor::new(SR);
        run_sine(&mut c, &p, 100.0, 0.9, 20000);
        for _ in 0..(SR as usize * 4) {
            c.process(Frame::SILENT, &p);
        }
        let ok = |x: f32| x == 0.0 || x.is_normal();
        assert!(ok(c.bands[0].mean_square) && ok(c.bands[2].mean_square));
        assert!(ok(c.bands[1].gain_db));
        for x in &c.crossovers {
            for b in [&x.low_a[0], &x.rest_b[1], &x.low_all_pass] {
                assert!(b.z1 == 0.0 || b.z1.is_normal());
                assert!(b.z2 == 0.0 || b.z2.is_normal());
            }
        }
    }

    #[test]
    fn behaviour_holds_across_sample_rates() {
        for sr in [22050.0, 48000.0, 96000.0, 192000.0] {
            let want = expected_gain_db(0.5, -20.0, 4.0);
            let got = steady_gain_db(&single(-20.0, 4.0), 1000.0, 0.5, sr);
            assert!((got - want).abs() < 0.3, "{sr}: want {want}, got {got}");
            let g = steady_gain_db(
                &CompressorParams {
                    depth: 0.0,
                    ..multi()
                },
                1000.0,
                0.2,
                sr,
            );
            assert!(g.abs() < 0.1, "{sr}: {g}");
        }
    }
}
