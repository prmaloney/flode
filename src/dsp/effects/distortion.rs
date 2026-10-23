//! Distortion: a waveshaper with a choice of curve, a drive, a tone control, and a dry/wet mix.
//!
//! The signal path per side is, in order:
//!
//! 1. The curve. Soft, Hard, Tube, and Fold run at 2x the sample rate, through a half-band FIR
//!    up and down, so the harmonics a curve makes above half the sample rate are filtered out
//!    instead of folding back. Crush runs at the plain rate and aliases on purpose.
//! 2. A DC blocker, for Tube only.
//! 3. The tone tilt, a one-pole split at 1 kHz with separate gains for the low and high parts.
//! 4. The level.
//!
//! Pre-gain law. Drive 0.0 to 1.0 maps to 0 dB to 40 dB, exponentially in dB, so the gain is
//! `10^(2 * drive)`.
//!
//! Loudness compensation. The curve output is divided by the curve's value at full-scale input
//! after the pre-gain, so a full-scale input peaks at about 1.0 at every drive. A full-scale sine
//! at mid drive comes out within a couple of dB of the dry one. Fold needs no division because
//! it never leaves [-1, 1].
//!
//! Latency. The two half-band filters delay the wet signal by `LATENCY` samples. The dry signal
//! is delayed by the same amount whenever the mix is above zero, so a partial mix does not make a
//! comb filter. At a mix of exactly 0.0 the output is the undelayed input, bit for bit. Crush
//! reads the delayed dry signal, so its wet path has the same delay.
//!
//! Smoothing. Pre-gain, level, mix, and tone slew with a 5 ms one-pole, so knob moves do not
//! zipper. The powers and exponentials that turn knobs into gains run when the params change,
//! never once per sample.

use crate::dsp::frame::Frame;

/// The curve a distortion applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DistortionKind {
    /// A smooth saturation, like a `tanh`. It rounds peaks and adds odd harmonics.
    #[default]
    Soft,
    /// A hard clip at full scale. It adds odd harmonics that fall off slowly.
    Hard,
    /// An asymmetric curve, like a tube stage. It adds even and odd harmonics.
    Tube,
    /// A wavefolder. Peaks past full scale fold back instead of clipping.
    Fold,
    /// Bit depth and sample rate reduction. Drive sets how coarse it gets.
    Crush,
    /// A diode-style soft clip. It has a gentler knee than `Soft`, so it starts to compress
    /// earlier and stays warmer at high drive. It adds odd harmonics.
    Overdrive,
}

impl DistortionKind {
    /// Every kind, in the order the dropdown lists them. New kinds go at the end, because patches
    /// store the dropdown row.
    pub const ALL: [DistortionKind; 6] = [
        DistortionKind::Soft,
        DistortionKind::Hard,
        DistortionKind::Tube,
        DistortionKind::Fold,
        DistortionKind::Crush,
        DistortionKind::Overdrive,
    ];
}

/// The distortion's controls, in plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistortionParams {
    pub kind: DistortionKind,
    /// How hard the input is pushed, from 0.0 to 1.0. At 0.0 the effect is nearly clean. At 1.0 it
    /// adds about 40 dB of gain before the curve. For `Crush` it sets the coarseness instead.
    pub drive: f32,
    /// A tilt on the output, from 0.0 (dark) to 1.0 (bright). 0.5 leaves the tone alone.
    pub tone: f32,
    /// The output level in dB, from -60.0 to 0.0, applied to the wet signal.
    pub level_db: f32,
    /// Dry/wet, from 0.0 (all dry) to 1.0 (all wet).
    pub mix: f32,
}

impl Default for DistortionParams {
    fn default() -> Self {
        Self {
            kind: DistortionKind::Soft,
            drive: 0.3,
            tone: 0.5,
            level_db: 0.0,
            mix: 1.0,
        }
    }
}

/// Half the number of taps in each polyphase branch of the half-band filter.
const HALF_BRANCH: usize = 8;
/// The taps in each polyphase branch. It is also the length of every history ring.
const BRANCH: usize = 2 * HALF_BRANCH;
/// The centre tap of the 31 tap half-band filter, in samples at the doubled rate.
const CENTRE: usize = 2 * HALF_BRANCH - 1;
/// The delay of the wet path, in samples at the plain rate: up and down each delay `CENTRE`
/// samples at the doubled rate.
const LATENCY: usize = 2 * HALF_BRANCH - 1;
/// Kaiser window shape for the half-band filter. About 70 dB of stopband.
const KAISER_BETA: f32 = 7.0;
/// The pre-gain at drive 1.0, in dB.
const MAX_PRE_GAIN_DB: f32 = 40.0;
/// The bias that makes the Tube curve asymmetric.
const TUBE_BIAS: f32 = 0.4;
/// Bit depth of Crush at drive 0.0 and 1.0.
const CRUSH_BITS_MAX: f32 = 16.0;
const CRUSH_BITS_MIN: f32 = 3.0;
/// How many samples Crush holds at drive 1.0.
const CRUSH_HOLD_MAX: f32 = 10.0;
/// The split frequency of the tone tilt, in Hz.
const TONE_SPLIT_HZ: f32 = 1000.0;
/// The corner of the Tube DC blocker, in Hz.
const DC_BLOCK_HZ: f32 = 20.0;
/// The time constant of the parameter smoothers, in seconds.
const SMOOTH_SECONDS: f32 = 0.005;
/// State magnitudes below this flush to zero so a decaying filter never goes denormal.
const DENORMAL_FLOOR: f32 = 1e-20;

/// The Tube curve. A biased `tanh` with the bias at zero input removed, so silence stays silent.
fn tube(u: f32) -> f32 {
    (u + TUBE_BIAS).tanh() - TUBE_BIAS.tanh()
}

/// The Overdrive curve. An exponential soft clip, odd-symmetric so it makes no DC.
fn overdrive(u: f32) -> f32 {
    (1.0 - (-u.abs()).exp()).copysign(u)
}

/// A triangle wavefolder. It is the identity inside [-1, 1] and reflects back at each edge.
fn fold(u: f32) -> f32 {
    ((u - 1.0).rem_euclid(4.0) - 2.0).abs() - 1.0
}

/// The curve before normalization.
fn shape(kind: DistortionKind, u: f32) -> f32 {
    match kind {
        DistortionKind::Soft => u.tanh(),
        DistortionKind::Hard => u.clamp(-1.0, 1.0),
        DistortionKind::Tube => tube(u),
        DistortionKind::Fold => fold(u),
        DistortionKind::Overdrive => overdrive(u),
        DistortionKind::Crush => u,
    }
}

/// The curve's peak output for a full-scale input at pre-gain `gain`.
fn peak(kind: DistortionKind, gain: f32) -> f32 {
    match kind {
        DistortionKind::Soft => gain.tanh(),
        DistortionKind::Tube => tube(gain).abs().max(tube(-gain).abs()),
        DistortionKind::Overdrive => overdrive(gain),
        DistortionKind::Hard | DistortionKind::Fold | DistortionKind::Crush => 1.0,
    }
}

/// The modified Bessel function I0, by its power series.
fn bessel_i0(x: f32) -> f32 {
    let quarter = x * x * 0.25;
    let mut term = 1.0;
    let mut sum = 1.0;
    for k in 1..40 {
        term *= quarter / (k * k) as f32;
        sum += term;
    }
    sum
}

/// The even-indexed taps of the half-band filter. Its odd-indexed taps are zero except the
/// centre, which is 0.5, so the even-indexed taps are all the filter needs.
fn half_band_taps() -> [f32; BRANCH] {
    let mut taps = [0.0; BRANCH];
    let centre = CENTRE as f32;
    let norm = bessel_i0(KAISER_BETA);
    for (i, tap) in taps.iter_mut().enumerate() {
        let offset = (2 * i) as f32 - centre;
        let x = std::f32::consts::FRAC_PI_2 * offset;
        let sinc = x.sin() / x;
        let ratio = offset / centre;
        let window = bessel_i0(KAISER_BETA * (1.0 - ratio * ratio).max(0.0).sqrt()) / norm;
        *tap = 0.5 * sinc * window;
    }
    taps
}

/// Values that depend on the sample rate only.
struct Coefficients {
    taps: [f32; BRANCH],
    smooth: f32,
    tone_split: f32,
    dc_pole: f32,
}

impl Coefficients {
    fn new(sample_rate: f32) -> Self {
        let rate = sample_rate.max(1.0);
        Self {
            taps: half_band_taps(),
            smooth: 1.0 - (-1.0 / (SMOOTH_SECONDS * rate)).exp(),
            tone_split: 1.0 - (-std::f32::consts::TAU * TONE_SPLIT_HZ / rate).exp(),
            dc_pole: 1.0 - std::f32::consts::TAU * DC_BLOCK_HZ / rate,
        }
    }
}

/// The values the knobs resolve to. They change when the params change, not per sample.
#[derive(Clone, Copy)]
struct Targets {
    kind: DistortionKind,
    gain: f32,
    level: f32,
    mix: f32,
    tone: f32,
    crush_step: f32,
    crush_hold: f32,
}

impl Targets {
    fn new(params: &DistortionParams) -> Self {
        let drive = params.drive.clamp(0.0, 1.0);
        let bits = CRUSH_BITS_MAX + (CRUSH_BITS_MIN - CRUSH_BITS_MAX) * drive;
        Self {
            kind: params.kind,
            gain: 10f32.powf(MAX_PRE_GAIN_DB / 20.0 * drive),
            level: 10f32.powf(params.level_db.clamp(-60.0, 0.0) / 20.0),
            mix: params.mix.clamp(0.0, 1.0),
            tone: params.tone.clamp(0.0, 1.0),
            crush_step: (1.0 - bits).exp2(),
            crush_hold: 1.0 + (CRUSH_HOLD_MAX - 1.0) * drive,
        }
    }
}

/// A ring of the last `BRANCH` samples.
#[derive(Clone, Copy)]
struct Ring {
    buf: [f32; BRANCH],
    newest: usize,
}

impl Ring {
    const fn new() -> Self {
        Self {
            buf: [0.0; BRANCH],
            newest: 0,
        }
    }

    fn push(&mut self, value: f32) {
        self.newest = (self.newest + 1) % BRANCH;
        self.buf[self.newest] = value;
    }

    /// The sample pushed `age` calls ago. Age 0 is the newest.
    fn get(&self, age: usize) -> f32 {
        self.buf[(self.newest + BRANCH - age) % BRANCH]
    }
}

/// A 2x oversampler. It upsamples with a half-band FIR, applies a function at the doubled rate,
/// and downsamples with the same filter.
#[derive(Clone, Copy)]
struct Oversampler {
    input: Ring,
    even: Ring,
    odd: Ring,
}

impl Oversampler {
    const fn new() -> Self {
        Self {
            input: Ring::new(),
            even: Ring::new(),
            odd: Ring::new(),
        }
    }

    /// Runs `curve` on the two doubled-rate samples that one input sample makes.
    fn process(&mut self, x: f32, taps: &[f32; BRANCH], curve: impl Fn(f32) -> f32) -> f32 {
        self.input.push(x);
        let mut up = 0.0;
        for (i, tap) in taps.iter().enumerate() {
            up += tap * self.input.get(i);
        }
        // The even phase is the filtered branch, with the gain of 2 that zero-stuffing costs. The
        // odd phase is the centre tap alone, which is a plain delay.
        self.even.push(curve(2.0 * up));
        self.odd.push(curve(self.input.get(HALF_BRANCH - 1)));
        let mut down = 0.5 * self.odd.get(HALF_BRANCH);
        for (i, tap) in taps.iter().enumerate() {
            down += tap * self.even.get(i);
        }
        down
    }
}

/// Moves `current` toward `target` by `coef`, and lands on `target` once it is close.
fn slew(current: f32, target: f32, coef: f32) -> f32 {
    let next = current + (target - current) * coef;
    if (target - next).abs() < 1e-6 * (1.0 + target.abs()) {
        target
    } else {
        next
    }
}

/// Zeroes a filter state that has decayed to nothing.
fn flush(value: f32) -> f32 {
    if value.abs() < DENORMAL_FLOOR {
        0.0
    } else {
        value
    }
}

/// All the state for one side.
#[derive(Clone, Copy)]
struct Channel {
    over: Oversampler,
    dry: Ring,
    gain: f32,
    level: f32,
    mix: f32,
    tone: f32,
    inv_peak: f32,
    peak_gain: f32,
    peak_kind: DistortionKind,
    crush_phase: f32,
    crush_held: f32,
    dc_in: f32,
    dc_out: f32,
    tone_low: f32,
    primed: bool,
}

impl Channel {
    const fn new() -> Self {
        Self {
            over: Oversampler::new(),
            dry: Ring::new(),
            gain: 1.0,
            level: 1.0,
            mix: 0.0,
            tone: 0.5,
            inv_peak: 1.0,
            peak_gain: 0.0,
            peak_kind: DistortionKind::Crush,
            crush_phase: 0.0,
            crush_held: 0.0,
            dc_in: 0.0,
            dc_out: 0.0,
            tone_low: 0.0,
            primed: false,
        }
    }

    fn process(&mut self, x: f32, t: &Targets, c: &Coefficients) -> f32 {
        if !self.primed {
            self.gain = t.gain;
            self.level = t.level;
            self.mix = t.mix;
            self.tone = t.tone;
            self.primed = true;
        }
        self.gain = slew(self.gain, t.gain, c.smooth);
        self.level = slew(self.level, t.level, c.smooth);
        self.mix = slew(self.mix, t.mix, c.smooth);
        self.tone = slew(self.tone, t.tone, c.smooth);

        self.dry.push(x);
        let dry = self.dry.get(LATENCY);

        let curved = match t.kind {
            DistortionKind::Crush => self.crush(dry, t),
            kind => self.curve(x, kind, &c.taps),
        };
        let blocked = if t.kind == DistortionKind::Tube {
            let out = curved - self.dc_in + c.dc_pole * self.dc_out;
            self.dc_in = curved;
            self.dc_out = flush(out);
            out
        } else {
            self.dc_in = curved;
            self.dc_out = 0.0;
            curved
        };

        // The tilt splits at the split frequency. Tone 0.5 has unity on both parts. Below that
        // the highs fall to 0.2. Above it the lows fall to 0.7 and the highs double.
        let bright = (2.0 * (self.tone - 0.5)).max(0.0);
        let dark = (2.0 * (0.5 - self.tone)).max(0.0);
        let low_gain = 1.0 - 0.3 * bright;
        let high_gain = 1.0 - 0.8 * dark + bright;
        self.tone_low = flush(self.tone_low + c.tone_split * (blocked - self.tone_low));
        let toned = low_gain * self.tone_low + high_gain * (blocked - self.tone_low);
        let wet = toned * self.level;

        // A mix of exactly zero is the undelayed input.
        if self.mix == 0.0 {
            x
        } else {
            dry * (1.0 - self.mix) + wet * self.mix
        }
    }

    /// One of the oversampled curves, with the pre-gain and the loudness division.
    fn curve(&mut self, x: f32, kind: DistortionKind, taps: &[f32; BRANCH]) -> f32 {
        if kind != self.peak_kind || (self.gain - self.peak_gain).abs() > 1e-4 * self.gain {
            self.peak_kind = kind;
            self.peak_gain = self.gain;
            self.inv_peak = 1.0 / peak(kind, self.gain).max(1e-6);
        }
        let gain = self.gain;
        let inv_peak = self.inv_peak;
        self.over
            .process(x, taps, |u| shape(kind, u * gain) * inv_peak)
    }

    /// Bit depth and sample rate reduction of the delayed dry sample.
    fn crush(&mut self, dry: f32, t: &Targets) -> f32 {
        self.crush_phase += 1.0;
        if self.crush_phase >= t.crush_hold {
            self.crush_phase = (self.crush_phase - t.crush_hold).clamp(0.0, t.crush_hold);
            self.crush_held = (dry / t.crush_step).round() * t.crush_step;
        }
        self.crush_held
    }
}

pub struct Distortion {
    coefficients: Coefficients,
    targets: Targets,
    last: Option<DistortionParams>,
    left: Channel,
    right: Channel,
}

impl Distortion {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            coefficients: Coefficients::new(sample_rate),
            targets: Targets::new(&DistortionParams::default()),
            last: None,
            left: Channel::new(),
            right: Channel::new(),
        }
    }

    pub fn reset(&mut self) {
        self.left = Channel::new();
        self.right = Channel::new();
    }

    pub fn process(&mut self, input: Frame, params: &DistortionParams) -> Frame {
        if self.last != Some(*params) {
            self.targets = Targets::new(params);
            self.last = Some(*params);
        }
        Frame::new(
            self.left
                .process(input.l, &self.targets, &self.coefficients),
            self.right
                .process(input.r, &self.targets, &self.coefficients),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    const SR: f32 = 48000.0;

    fn params(kind: DistortionKind, drive: f32) -> DistortionParams {
        DistortionParams {
            kind,
            drive,
            tone: 0.5,
            level_db: 0.0,
            mix: 1.0,
        }
    }

    /// A sine with `cycles` whole periods in every `period` samples.
    fn sine(amp: f32, cycles: usize, period: usize, len: usize) -> Vec<f32> {
        (0..len)
            .map(|n| amp * (TAU * (cycles * n) as f64 / period as f64).sin() as f32)
            .collect()
    }

    fn run(sample_rate: f32, p: &DistortionParams, input: &[f32]) -> Vec<f32> {
        let mut fx = Distortion::new(sample_rate);
        input
            .iter()
            .map(|&x| fx.process(Frame::mono(x), p).l)
            .collect()
    }

    /// The amplitude of DFT bin `k` of `signal`, scaled so a sine of amplitude `a` reads `a`.
    fn bin_amp(signal: &[f32], k: usize) -> f32 {
        let n = signal.len();
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, &s) in signal.iter().enumerate() {
            let phase = TAU * (k * i) as f64 / n as f64;
            re += s as f64 * phase.cos();
            im -= s as f64 * phase.sin();
        }
        (2.0 / n as f64 * (re * re + im * im).sqrt()) as f32
    }

    fn rms(signal: &[f32]) -> f32 {
        (signal.iter().map(|s| s * s).sum::<f32>() / signal.len() as f32).sqrt()
    }

    /// 480 Hz at 48 kHz, with whole periods in 4800 samples. Returns the last 4800 of the output.
    fn tone_run(p: &DistortionParams, amp: f32) -> Vec<f32> {
        let input = sine(amp, 48, 4800, 9600);
        run(SR, p, &input)[4800..].to_vec()
    }

    #[test]
    fn soft_makes_odd_harmonics_and_no_even_ones() {
        let out = tone_run(&params(DistortionKind::Soft, 0.5), 0.9);
        let fund = bin_amp(&out, 48);
        assert!(bin_amp(&out, 144) > 0.05 * fund, "third harmonic");
        assert!(bin_amp(&out, 96) < 1e-3 * fund, "second harmonic");
    }

    #[test]
    fn hard_makes_a_square_like_odd_spectrum() {
        let out = tone_run(&params(DistortionKind::Hard, 0.5), 0.9);
        let fund = bin_amp(&out, 48);
        let third = bin_amp(&out, 144);
        let fifth = bin_amp(&out, 240);
        assert!(third > 0.2 * fund && fifth > 0.1 * fund, "{third} {fifth}");
        assert!(third > fifth);
        assert!(bin_amp(&out, 96) < 1e-3 * fund, "second harmonic");
    }

    #[test]
    fn tube_makes_even_harmonics_and_leaves_no_dc() {
        let p = params(DistortionKind::Tube, 0.4);
        let input = sine(0.9, 48, 4800, 24000);
        let out = run(SR, &p, &input);
        let tail = &out[19200..];
        let fund = bin_amp(tail, 48);
        assert!(bin_amp(tail, 96) > 0.03 * fund, "second harmonic");
        let mean = tail.iter().sum::<f32>() / tail.len() as f32;
        assert!(mean.abs() < 0.003 * fund, "dc {mean}");
    }

    #[test]
    fn overdrive_makes_odd_harmonics_and_knees_softer_than_soft() {
        let out = tone_run(&params(DistortionKind::Overdrive, 0.5), 0.9);
        let fund = bin_amp(&out, 48);
        assert!(bin_amp(&out, 144) > 0.05 * fund, "third harmonic");
        assert!(bin_amp(&out, 96) < 1e-3 * fund, "second harmonic");
        // Below the knee it compresses earlier than tanh does.
        assert!(overdrive(0.5) < 0.5f32.tanh());
        assert_eq!(overdrive(-0.7), -overdrive(0.7));
        assert_eq!(overdrive(0.0), 0.0);
    }

    #[test]
    fn the_fold_curve_reflects_at_each_edge() {
        assert_eq!(fold(0.5), 0.5);
        assert_eq!(fold(-0.5), -0.5);
        assert!((fold(1.5) - 0.5).abs() < 1e-6);
        assert!(fold(2.0).abs() < 1e-6);
        assert!((fold(3.0) + 1.0).abs() < 1e-6);
        assert!((fold(5.0) - 1.0).abs() < 1e-6);
        for i in -2000..2000 {
            assert!(fold(i as f32 * 0.37).abs() <= 1.0 + 1e-6);
        }
    }

    #[test]
    fn fold_at_high_drive_stays_bounded_and_is_busy() {
        let out = tone_run(&params(DistortionKind::Fold, 0.8), 0.9);
        assert!(out.iter().all(|s| s.abs() <= 1.05));
        // A fundamental of 0.9 pushed 100 times past full scale folds many times per cycle, so
        // the output crosses zero far more often than the input does.
        let crossings = out.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
        assert!(crossings > 4 * 96, "{crossings}");
    }

    #[test]
    fn crush_quantizes_and_holds() {
        let p = params(DistortionKind::Crush, 1.0);
        let input = sine(0.9, 48, 4800, 4800);
        let out = run(SR, &p, &input);
        // Drive 1.0 is 3 bits, a step of 0.25, held for 10 samples.
        for s in &out {
            let steps = s / 0.25;
            assert!((steps - steps.round()).abs() < 1e-3, "{s}");
        }
        let mut levels: Vec<i32> = out.iter().map(|s| (s * 4.0).round() as i32).collect();
        levels.sort_unstable();
        levels.dedup();
        assert!(levels.len() <= 9, "{levels:?}");
        let changes = out
            .windows(2)
            .filter(|w| (w[0] - w[1]).abs() > 1e-3)
            .count();
        // One change per 10 samples at most.
        assert!(changes <= out.len() / 10, "{changes}");
    }

    #[test]
    fn crush_at_zero_drive_is_close_to_clean() {
        let p = params(DistortionKind::Crush, 0.0);
        let input = sine(0.5, 48, 4800, 4800);
        let out = run(SR, &p, &input);
        for n in LATENCY..out.len() {
            assert!((out[n] - input[n - LATENCY]).abs() < 1e-4);
        }
    }

    /// The gain of a clean path at `bin` of 4800, as a ratio of output to input amplitude.
    fn gain_at(tone: f32, bin: usize) -> f32 {
        let p = DistortionParams {
            kind: DistortionKind::Hard,
            drive: 0.0,
            tone,
            level_db: 0.0,
            mix: 1.0,
        };
        let input = sine(0.3, bin, 4800, 9600);
        let out = run(SR, &p, &input);
        bin_amp(&out[4800..], bin) / 0.3
    }

    #[test]
    fn tone_half_is_flat() {
        for bin in [10, 100, 800, 1600] {
            let g = gain_at(0.5, bin);
            assert!((g - 1.0).abs() < 0.04, "bin {bin} gain {g}");
        }
    }

    #[test]
    fn tone_zero_rolls_off_the_highs() {
        assert!(gain_at(0.0, 10) > 0.95);
        assert!(gain_at(0.0, 800) < 0.4);
    }

    #[test]
    fn tone_one_lifts_the_highs_and_trims_the_lows() {
        let low = gain_at(1.0, 10);
        assert!(low < 0.85 && low > 0.5, "low {low}");
        assert!(gain_at(1.0, 800) > 1.7);
    }

    #[test]
    fn a_full_scale_sine_at_mid_drive_keeps_its_level() {
        let input = sine(1.0, 48, 4800, 9600);
        let dry = rms(&input[4800..]);
        for kind in DistortionKind::ALL {
            let out = tone_run(&params(kind, 0.5), 1.0);
            let db = 20.0 * (rms(&out) / dry).log10();
            assert!(db.abs() < 4.0, "{kind:?} {db} dB");
        }
    }

    #[test]
    fn level_scales_the_wet_signal() {
        let mut p = params(DistortionKind::Hard, 0.0);
        let full = rms(&tone_run(&p, 0.3));
        p.level_db = -20.0;
        let quiet = rms(&tone_run(&p, 0.3));
        assert!((quiet / full - 0.1).abs() < 0.005);
    }

    /// A deterministic noise source in [-1, 1].
    fn noise(len: usize) -> Vec<f32> {
        let mut state = 12345u32;
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 8) as f32 / (1 << 23) as f32 - 1.0
            })
            .collect()
    }

    #[test]
    fn mix_zero_returns_the_dry_input_exactly() {
        let input = noise(5000);
        for kind in DistortionKind::ALL {
            let mut p = params(kind, 0.9);
            p.mix = 0.0;
            let out = run(SR, &p, &input);
            assert_eq!(out, input, "{kind:?}");
        }
    }

    #[test]
    fn mix_returns_to_exact_dry_after_it_was_wet() {
        let input = noise(30000);
        let mut fx = Distortion::new(SR);
        let mut p = params(DistortionKind::Fold, 0.7);
        for &x in &input[..5000] {
            fx.process(Frame::mono(x), &p);
        }
        p.mix = 0.0;
        for &x in &input[5000..] {
            fx.process(Frame::mono(x), &p);
        }
        for &x in &input[..200] {
            assert_eq!(fx.process(Frame::mono(x), &p).l, x);
        }
    }

    #[test]
    fn a_partial_mix_aligns_dry_with_wet() {
        // A clean path at mix 0.5 is dry and wet adding in phase, so the gain stays near 1.
        let p = DistortionParams {
            kind: DistortionKind::Hard,
            drive: 0.0,
            tone: 0.5,
            level_db: 0.0,
            mix: 0.5,
        };
        let input = sine(0.3, 800, 4800, 9600);
        let out = run(SR, &p, &input);
        let g = bin_amp(&out[4800..], 800) / 0.3;
        assert!((g - 1.0).abs() < 0.05, "{g}");
    }

    #[test]
    fn the_wet_path_is_delayed_by_the_reported_latency() {
        let p = params(DistortionKind::Hard, 0.0);
        let mut input = vec![0.0; 100];
        input[0] = 0.5;
        let out = run(SR, &p, &input);
        let mut at = 0;
        for (i, s) in out.iter().enumerate() {
            if s.abs() > out[at].abs() {
                at = i;
            }
        }
        assert_eq!(at, LATENCY);
        // The filters trim the impulse's highest frequencies, so the peak is a little under 0.5.
        assert!(out[at] > 0.43 && out[at] < 0.5, "{}", out[at]);
    }

    #[test]
    fn output_is_finite_for_extreme_inputs_and_params() {
        let loud: Vec<f32> = noise(4000).iter().map(|s| s * 8.0).collect();
        for kind in DistortionKind::ALL {
            for drive in [0.0, 1.0] {
                for tone in [0.0, 1.0] {
                    for level_db in [-60.0, 0.0] {
                        for mix in [0.0, 1.0] {
                            for sample_rate in [22050.0, 192000.0] {
                                let p = DistortionParams {
                                    kind,
                                    drive,
                                    tone,
                                    level_db,
                                    mix,
                                };
                                for s in run(sample_rate, &p, &loud) {
                                    assert!(s.is_finite() && s.abs() < 100.0, "{p:?} {s}");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn output_is_finite_when_params_jump_every_sample() {
        let input = noise(20000);
        let mut fx = Distortion::new(44100.0);
        for (n, &x) in input.iter().enumerate() {
            let p = DistortionParams {
                kind: DistortionKind::ALL[n % DistortionKind::ALL.len()],
                drive: (n % 2) as f32,
                tone: ((n / 3) % 2) as f32,
                level_db: -60.0 * ((n / 7) % 2) as f32,
                mix: ((n / 5) % 2) as f32,
            };
            let y = fx.process(Frame::new(8.0 * x, -8.0 * x), &p);
            assert!(y.l.is_finite() && y.r.is_finite() && y.peak() < 100.0);
        }
    }

    #[test]
    fn the_two_sides_do_not_share_state() {
        let mut fx = Distortion::new(SR);
        let p = params(DistortionKind::Tube, 0.8);
        for &x in &noise(2000) {
            let y = fx.process(Frame::new(x, 0.0), &p);
            assert!(y.r.abs() < 1e-6, "{}", y.r);
        }
    }

    #[test]
    fn a_decaying_tail_goes_to_exact_zero() {
        let mut fx = Distortion::new(SR);
        let p = params(DistortionKind::Tube, 0.5);
        for &x in &sine(0.9, 48, 4800, 4800) {
            fx.process(Frame::mono(x), &p);
        }
        let mut last = Frame::mono(1.0);
        for _ in 0..200_000 {
            last = fx.process(Frame::SILENT, &p);
        }
        assert_eq!(last, Frame::SILENT);
    }

    #[test]
    fn reset_clears_the_state() {
        let mut fx = Distortion::new(SR);
        let p = params(DistortionKind::Soft, 0.9);
        for &x in &noise(1000) {
            fx.process(Frame::mono(x), &p);
        }
        fx.reset();
        assert_eq!(fx.process(Frame::SILENT, &p), Frame::SILENT);
    }

    /// An in-place radix-2 FFT.
    fn fft(re: &mut [f64], im: &mut [f64]) {
        let n = re.len();
        let mut j = 0;
        for i in 1..n {
            let mut bit = n >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j ^= bit;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let angle = -TAU / len as f64;
            for start in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let (s, c) = (angle * k as f64).sin_cos();
                    let (a, b) = (start + k, start + k + len / 2);
                    let tr = re[b] * c - im[b] * s;
                    let ti = re[b] * s + im[b] * c;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len <<= 1;
        }
    }

    /// The power outside the harmonics of bin `cycles` that sit below Nyquist, as a ratio to the
    /// power in them. Everything else is aliasing, or a harmonic that folded back.
    fn alias_ratio(signal: &[f32], cycles: usize) -> f64 {
        let n = signal.len();
        let mut re: Vec<f64> = signal.iter().map(|&s| s as f64).collect();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im);
        let (mut wanted, mut unwanted) = (0.0, 0.0);
        for k in 1..=n / 2 {
            let power = re[k] * re[k] + im[k] * im[k];
            if k % cycles == 0 {
                wanted += power;
            } else {
                unwanted += power;
            }
        }
        unwanted / wanted
    }

    #[test]
    fn oversampling_lowers_aliasing_at_high_drive() {
        let sample_rate = 44100.0;
        let period = 4096;
        let cycles = 301;
        let input = sine(0.9, cycles, period, 2 * period);
        for kind in [
            DistortionKind::Soft,
            DistortionKind::Hard,
            DistortionKind::Tube,
            DistortionKind::Fold,
            DistortionKind::Overdrive,
        ] {
            let p = params(kind, 1.0);
            let oversampled = run(sample_rate, &p, &input)[period..].to_vec();
            let t = Targets::new(&p);
            let inv_peak = 1.0 / peak(kind, t.gain);
            let naive: Vec<f32> = input[period..]
                .iter()
                .map(|&x| shape(kind, x * t.gain) * inv_peak)
                .collect();
            let over = alias_ratio(&oversampled, cycles);
            let plain = alias_ratio(&naive, cycles);
            println!("{kind:?} oversampled {over:.5} naive {plain:.5}");
            assert!(over < 0.7 * plain, "{kind:?} {over} vs {plain}");
        }
    }
}
