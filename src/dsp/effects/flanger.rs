//! Flanger: a very short modulated delay with feedback, mixed with the dry signal so the comb
//! filter sweeps.
//!
//! Each side has its own delay line, read with 4-point cubic Lagrange interpolation. The line is
//! built here rather than taken from `dsp::delay`, because a flanger at a high sample rate needs
//! delays under one sample's worth of that line's minimum, and because the feedback loop needs
//! its read to come before its write.
//!
//! Signal path per side, with `x` the input and `d` the modulated delay:
//!
//! ```text
//! y = line.read(d)
//! lp = one_pole_low_pass(y)
//! line.write(x + feedback * soft_limit(lp))
//! out = x * (1 - mix) + y * mix
//! ```
//!
//! The loop delay is exactly `d` plus the lag of the one-pole, so the comb notches sit at odd
//! multiples of `1 / (2 * d)` and the feedback peaks at multiples of `1 / d`.
//!
//! Wet/dry law: a linear crossfade. At a mix of 0.5 the dry and delayed signals have equal gain,
//! so the notches go to zero and the peaks have unity gain with no feedback. A mix of 0.0 returns
//! the input unchanged, and a mix of 1.0 returns only the delayed signal, which has no comb
//! without feedback. Feedback raises the level of the resonant peaks by up to `1 / (1 - fb)`.
//!
//! Sweep law: the delay is `delay_ms * (1 + 0.9 * depth * lfo)` with a sine `lfo` in [-1, 1]. The
//! result is clamped to at least 0.05 ms and at least two samples (the cubic read needs one
//! sample of look-behind on each side), and to the end of the buffer.

use crate::dsp::frame::Frame;

/// The flanger's controls, in plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlangerParams {
    /// The speed of the sweep in Hz, from 0.05 to 200.0 (the knob is shared with the LFO).
    pub rate_hz: f32,
    /// How far the delay swings around its base, from 0.0 to 1.0.
    pub depth: f32,
    /// How much of the output goes back in, from 0.0 to 0.95.
    pub feedback: f32,
    /// The middle of the delay in ms, from 0.1 to 10.0.
    pub delay_ms: f32,
    /// How far apart the left and right sweeps sit, from 0.0 (identical) to 1.0 (opposite).
    pub spread: f32,
    /// Dry/wet, from 0.0 (all dry) to 1.0 (all wet).
    pub mix: f32,
}

impl Default for FlangerParams {
    fn default() -> Self {
        Self {
            rate_hz: 1.0,
            depth: 0.5,
            feedback: 0.4,
            delay_ms: 2.0,
            spread: 0.75,
            mix: 0.5,
        }
    }
}

/// The fraction of the centre delay that the sweep covers at full depth.
const SWING: f32 = 0.9;
/// The shortest delay in ms.
const MIN_DELAY_MS: f32 = 0.05;
/// The shortest delay in samples, so the cubic read never needs the sample not yet written.
const MIN_DELAY_SAMPLES: f32 = 2.0;
/// The longest centre delay in ms, which sizes the buffer.
const MAX_CENTRE_MS: f32 = 10.0;
/// The corner of the one-pole low pass in the feedback loop.
const LOOP_CUTOFF_HZ: f32 = 6000.0;
/// The time constant of the delay time smoother, in seconds. It is slow enough that a knob move
/// glides like a tape machine and never jumps.
const DELAY_SLEW_S: f32 = 0.015;
/// The time constant of the depth, feedback, spread, and mix smoothers, in seconds.
const PARAM_SLEW_S: f32 = 0.01;
/// Values under this size in the loop are set to zero, so a decaying tail never goes denormal.
const FLUSH: f32 = 1e-20;
/// A smoothed value this close to its target snaps onto it, so a settled mix of 0.0 is exact.
const SNAP: f32 = 1e-5;
/// Level at which the feedback path starts to compress.
const LIMIT_KNEE: f32 = 4.0;

/// A circular buffer of one side's samples.
struct Line {
    buf: Box<[f32]>,
    mask: usize,
    write: usize,
}

impl Line {
    fn new(min_len: usize) -> Self {
        let len = min_len.next_power_of_two();
        Self {
            buf: vec![0.0; len].into_boxed_slice(),
            mask: len - 1,
            write: 0,
        }
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.write = 0;
    }

    /// The sample written `k` calls ago, where 1 is the newest.
    fn at(&self, k: usize) -> f32 {
        self.buf[self.write.wrapping_sub(k) & self.mask]
    }

    /// Reads `delay` samples back with cubic Lagrange interpolation. `delay` must be at least 2
    /// and at most the buffer length minus 3.
    fn read(&self, delay: f32) -> f32 {
        let whole = delay as usize;
        let t = delay - whole as f32;
        let a = self.at(whole - 1);
        let b = self.at(whole);
        let c = self.at(whole + 1);
        let d = self.at(whole + 2);
        let ca = -t * (t - 1.0) * (t - 2.0) * (1.0 / 6.0);
        let cb = (t + 1.0) * (t - 1.0) * (t - 2.0) * 0.5;
        let cc = -(t + 1.0) * t * (t - 2.0) * 0.5;
        let cd = (t + 1.0) * t * (t - 1.0) * (1.0 / 6.0);
        a * ca + b * cb + c * cc + d * cd
    }

    fn push(&mut self, x: f32) {
        self.buf[self.write] = x;
        self.write = (self.write + 1) & self.mask;
    }
}

/// One side of the flanger: its line and the state of its loop filter.
struct Side {
    line: Line,
    low_pass: f32,
}

impl Side {
    fn new(min_len: usize) -> Self {
        Self {
            line: Line::new(min_len),
            low_pass: 0.0,
        }
    }

    fn reset(&mut self) {
        self.line.clear();
        self.low_pass = 0.0;
    }

    /// One sample. Returns the delayed signal and feeds the loop.
    fn tick(&mut self, x: f32, delay: f32, feedback: f32, lp_coef: f32) -> f32 {
        let y = self.line.read(delay);
        self.low_pass = flush(self.low_pass + lp_coef * (y - self.low_pass));
        self.line
            .push(flush(x + feedback * soft_limit(self.low_pass)));
        y
    }
}

/// Zeroes a value too small to matter.
fn flush(x: f32) -> f32 {
    if x.abs() < FLUSH {
        0.0
    } else {
        x
    }
}

/// Passes values up to `LIMIT_KNEE` unchanged and bends larger ones toward twice that, so the
/// loop cannot run away even at full feedback and full-scale resonant input.
fn soft_limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= LIMIT_KNEE {
        x
    } else {
        let bent = LIMIT_KNEE + LIMIT_KNEE * (a - LIMIT_KNEE) / a;
        bent.copysign(x)
    }
}

/// Clamps `x` into the range, and maps NaN to the low end.
fn sane(x: f32, lo: f32, hi: f32) -> f32 {
    if x.is_nan() {
        lo
    } else {
        x.clamp(lo, hi)
    }
}

/// Moves `value` toward `target` by `coef`, and snaps when it is close.
fn slew(value: &mut f32, target: f32, coef: f32) {
    *value += (target - *value) * coef;
    if (target - *value).abs() < SNAP {
        *value = target;
    }
}

pub struct Flanger {
    sample_rate: f32,
    left: Side,
    right: Side,
    /// The LFO phase of the left side, in cycles.
    phase: f32,
    /// Smoothed copies of the controls.
    delay_ms: f32,
    depth: f32,
    feedback: f32,
    spread: f32,
    mix: f32,
    /// False until the first call, which sets the smoothers straight to their targets.
    primed: bool,
    lp_coef: f32,
    delay_coef: f32,
    param_coef: f32,
    min_delay: f32,
    max_delay: f32,
    /// The shortest and longest delay read, in samples, since the last reset.
    #[cfg(test)]
    seen: (f32, f32),
}

impl Flanger {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = if sample_rate.is_finite() {
            sample_rate.max(1000.0)
        } else {
            44100.0
        };
        let longest = MAX_CENTRE_MS * (1.0 + SWING) * 0.001 * sample_rate;
        let len = longest.ceil() as usize + 8;
        let side = Side::new(len);
        let max_delay = (side.line.buf.len() - 4) as f32;
        let one_pole = |seconds: f32| 1.0 - (-1.0 / (seconds * sample_rate)).exp();
        let cutoff = LOOP_CUTOFF_HZ.min(sample_rate * 0.4);
        let lp_coef = 1.0 - (-std::f32::consts::TAU * cutoff / sample_rate).exp();
        Self {
            sample_rate,
            left: side,
            right: Side::new(len),
            phase: 0.0,
            delay_ms: 0.0,
            depth: 0.0,
            feedback: 0.0,
            spread: 0.0,
            mix: 0.0,
            primed: false,
            lp_coef,
            delay_coef: one_pole(DELAY_SLEW_S),
            param_coef: one_pole(PARAM_SLEW_S),
            min_delay: (MIN_DELAY_MS * 0.001 * sample_rate).max(MIN_DELAY_SAMPLES),
            max_delay,
            #[cfg(test)]
            seen: (f32::MAX, 0.0),
        }
    }

    /// Clears the line, the loop filter, and the LFO. The next call takes its params as they are
    /// instead of gliding to them.
    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
        self.phase = 0.0;
        self.primed = false;
        #[cfg(test)]
        {
            self.seen = (f32::MAX, 0.0);
        }
    }

    /// The delay in samples for an LFO value in [-1, 1], from the smoothed controls.
    fn delay_samples(&self, lfo: f32) -> f32 {
        let ms = self.delay_ms * (1.0 + SWING * self.depth * lfo);
        (ms * 0.001 * self.sample_rate).clamp(self.min_delay, self.max_delay)
    }

    pub fn process(&mut self, input: Frame, params: &FlangerParams) -> Frame {
        let rate = sane(params.rate_hz, 0.05, 200.0);
        let depth = sane(params.depth, 0.0, 1.0);
        let feedback = sane(params.feedback, 0.0, 0.95);
        let delay_ms = sane(params.delay_ms, 0.1, MAX_CENTRE_MS);
        let spread = sane(params.spread, 0.0, 1.0);
        let mix = sane(params.mix, 0.0, 1.0);

        if self.primed {
            slew(&mut self.delay_ms, delay_ms, self.delay_coef);
            slew(&mut self.depth, depth, self.param_coef);
            slew(&mut self.feedback, feedback, self.param_coef);
            slew(&mut self.spread, spread, self.param_coef);
            slew(&mut self.mix, mix, self.param_coef);
        } else {
            self.delay_ms = delay_ms;
            self.depth = depth;
            self.feedback = feedback;
            self.spread = spread;
            self.mix = mix;
            self.primed = true;
        }

        self.phase += rate / self.sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        let mut right_phase = self.phase + self.spread * 0.5;
        if right_phase >= 1.0 {
            right_phase -= 1.0;
        }
        let lfo_l = (std::f32::consts::TAU * self.phase).sin();
        let lfo_r = (std::f32::consts::TAU * right_phase).sin();
        let delay_l = self.delay_samples(lfo_l);
        let delay_r = self.delay_samples(lfo_r);
        #[cfg(test)]
        {
            self.seen.0 = self.seen.0.min(delay_l).min(delay_r);
            self.seen.1 = self.seen.1.max(delay_l).max(delay_r);
        }

        let finite = |x: f32| if x.is_finite() { x } else { 0.0 };
        let x = input.map(finite);
        let wet_l = self.left.tick(x.l, delay_l, self.feedback, self.lp_coef);
        let wet_r = self.right.tick(x.r, delay_r, self.feedback, self.lp_coef);
        let dry_gain = 1.0 - self.mix;
        Frame::new(
            x.l * dry_gain + wet_l * self.mix,
            x.r * dry_gain + wet_r * self.mix,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATES: [f32; 4] = [22050.0, 44100.0, 96000.0, 192000.0];

    fn noise(state: &mut u32) -> f32 {
        *state ^= *state << 13;
        *state ^= *state >> 17;
        *state ^= *state << 5;
        (*state as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// A static comb: no sweep, no feedback, equal dry and wet.
    fn comb(delay_ms: f32) -> FlangerParams {
        FlangerParams {
            rate_hz: 1.0,
            depth: 0.0,
            feedback: 0.0,
            delay_ms,
            spread: 0.0,
            mix: 0.5,
        }
    }

    /// The amplitude of a sine at `freq` after it passes through the flanger, measured once the
    /// loop has settled.
    fn tone_gain(sample_rate: f32, freq: f32, params: &FlangerParams) -> f32 {
        let mut fx = Flanger::new(sample_rate);
        let warm = 8000;
        let n = 8192;
        let (mut c, mut s) = (0.0f64, 0.0f64);
        for i in 0..warm + n {
            let ph = std::f64::consts::TAU * freq as f64 * i as f64 / sample_rate as f64;
            let out = fx.process(Frame::mono(ph.sin() as f32), params).l as f64;
            if i >= warm {
                c += out * ph.cos();
                s += out * ph.sin();
            }
        }
        (2.0 / n as f64 * (c * c + s * s).sqrt()) as f32
    }

    /// The frequency in `lo..hi` where the comb is deepest, scanned in 1 Hz steps.
    fn deepest_notch(sample_rate: f32, delay_ms: f32, lo: u32, hi: u32) -> f32 {
        let params = comb(delay_ms);
        (lo..=hi)
            .map(|f| (f, tone_gain(sample_rate, f as f32, &params)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap()
            .0 as f32
    }

    #[test]
    fn mix_zero_returns_the_dry_input_exactly() {
        let mut fx = Flanger::new(44100.0);
        let params = FlangerParams {
            rate_hz: 3.0,
            depth: 1.0,
            feedback: 0.9,
            delay_ms: 3.0,
            spread: 1.0,
            mix: 0.0,
        };
        let mut state = 7;
        for _ in 0..20000 {
            let input = Frame::new(noise(&mut state) * 8.0, noise(&mut state) * 8.0);
            assert_eq!(fx.process(input, &params), input);
        }
    }

    #[test]
    fn mix_settles_to_exact_dry_after_a_change() {
        let mut fx = Flanger::new(44100.0);
        let mut params = FlangerParams {
            mix: 0.5,
            feedback: 0.8,
            ..FlangerParams::default()
        };
        let mut state = 11;
        for _ in 0..5000 {
            fx.process(Frame::mono(noise(&mut state)), &params);
        }
        params.mix = 0.0;
        for _ in 0..44100 {
            fx.process(Frame::mono(noise(&mut state)), &params);
        }
        for _ in 0..1000 {
            let input = Frame::mono(noise(&mut state));
            assert_eq!(fx.process(input, &params), input);
        }
    }

    #[test]
    fn the_dry_path_adds_no_latency() {
        for mix in [0.0, 0.5] {
            let mut fx = Flanger::new(44100.0);
            let params = FlangerParams { mix, ..comb(1.0) };
            let first = fx.process(Frame::mono(1.0), &params);
            assert_eq!(first.l, 1.0 - mix, "mix {mix}");
            let second = fx.process(Frame::SILENT, &params);
            assert_eq!(second.l, 0.0);
        }
    }

    #[test]
    fn comb_notches_sit_at_odd_multiples_of_one_over_twice_the_delay() {
        // 2 ms at 48 kHz is an integer 96 samples, so the first notch is at 250 Hz.
        let params = comb(2.0);
        for (freq, expect_notch) in [(250.0, true), (750.0, true), (500.0, false)] {
            let gain = tone_gain(48000.0, freq, &params);
            if expect_notch {
                assert!(gain < 0.02, "{freq} Hz gain {gain}");
            } else {
                assert!((gain - 1.0).abs() < 0.02, "{freq} Hz gain {gain}");
            }
        }
        // A fractional delay still puts the notch where 1 / (2 d) says.
        for delay_ms in [1.37, 0.6, 3.3] {
            let predicted = 1000.0 / (2.0 * delay_ms);
            let found = deepest_notch(
                48000.0,
                delay_ms,
                predicted as u32 - 30,
                predicted as u32 + 30,
            );
            assert!(
                (found - predicted).abs() <= 1.5,
                "delay {delay_ms} ms: notch {found} Hz, predicted {predicted} Hz"
            );
        }
    }

    #[test]
    fn notches_land_correctly_at_every_sample_rate() {
        for sr in RATES {
            let delay_ms = 0.8;
            let predicted = 1000.0 / (2.0 * delay_ms);
            let found = deepest_notch(sr, delay_ms, 550, 700);
            assert!(
                (found - predicted).abs() <= 1.5,
                "{sr} Hz: notch {found}, predicted {predicted}"
            );
            let gain = tone_gain(sr, predicted, &comb(delay_ms));
            assert!(gain < 0.05, "{sr} Hz: notch gain {gain}");
        }
    }

    #[test]
    fn the_notches_sweep_at_the_lfo_rate() {
        // A tone sits on the centre delay's first notch. The sweep crosses the centre twice per
        // LFO cycle, and each crossing drags the notch through the tone.
        let sr = 48000.0;
        let rate = 2.0;
        let params = FlangerParams {
            rate_hz: rate,
            depth: 0.5,
            feedback: 0.0,
            delay_ms: 3.0,
            spread: 0.0,
            mix: 0.5,
        };
        let freq = 1000.0 / (2.0 * 3.0);
        let window = (sr / freq * 1.0) as usize;
        let seconds = 2.0;
        let mut fx = Flanger::new(sr);
        let mut dips = 0;
        let mut in_dip = false;
        let mut energy = 0.0f64;
        for i in 0..(sr * seconds) as usize {
            let t = i as f64 / sr as f64;
            let x = (std::f64::consts::TAU * freq as f64 * t).sin() as f32;
            let y = fx.process(Frame::mono(x), &params).l as f64;
            energy += y * y;
            if (i + 1) % window == 0 {
                let amp = (2.0 * energy / window as f64).sqrt();
                energy = 0.0;
                if !in_dip && amp < 0.3 {
                    in_dip = true;
                    dips += 1;
                } else if in_dip && amp > 0.6 {
                    in_dip = false;
                }
            }
        }
        let expected = (2.0 * rate * seconds) as i32;
        assert!(
            (dips - expected).abs() <= 1,
            "{dips} dips, expected about {expected}"
        );
    }

    /// The shortest and longest delay in ms after a few LFO cycles.
    fn sweep_extent(sr: f32, params: &FlangerParams) -> (f32, f32) {
        let mut fx = Flanger::new(sr);
        let n = (sr * 2.0 / params.rate_hz.min(2.0)) as usize;
        for _ in 0..n {
            fx.process(Frame::SILENT, params);
        }
        (fx.seen.0 / sr * 1000.0, fx.seen.1 / sr * 1000.0)
    }

    #[test]
    fn depth_scales_the_sweep_and_delay_sets_its_centre() {
        let base = FlangerParams {
            rate_hz: 2.0,
            delay_ms: 4.0,
            ..comb(4.0)
        };
        let mut previous_width = -1.0;
        for depth in [0.0, 0.25, 0.5, 1.0] {
            let (lo, hi) = sweep_extent(48000.0, &FlangerParams { depth, ..base });
            let centre = (lo + hi) * 0.5;
            let half = (hi - lo) * 0.5;
            assert!(
                (centre - 4.0).abs() < 0.01,
                "depth {depth}: centre {centre}"
            );
            assert!(
                (half - 4.0 * SWING * depth).abs() < 0.02,
                "depth {depth}: half width {half}"
            );
            assert!(half > previous_width);
            previous_width = half;
        }
        for delay_ms in [0.5, 2.0, 9.0] {
            let params = FlangerParams {
                delay_ms,
                depth: 0.5,
                ..base
            };
            let (lo, hi) = sweep_extent(48000.0, &params);
            let centre = (lo + hi) * 0.5;
            assert!(
                (centre - delay_ms).abs() < 0.01 * delay_ms.max(1.0),
                "delay {delay_ms}: centre {centre}"
            );
        }
    }

    #[test]
    fn the_delay_stays_in_range_at_extreme_rates_and_sample_rates() {
        for sr in RATES {
            for rate_hz in [0.05, 20.0, 200.0] {
                for delay_ms in [0.1, 10.0] {
                    let params = FlangerParams {
                        rate_hz,
                        depth: 1.0,
                        feedback: 0.95,
                        delay_ms,
                        spread: 1.0,
                        mix: 0.5,
                    };
                    let mut fx = Flanger::new(sr);
                    let mut state = 3;
                    for _ in 0..(sr as usize / 2) {
                        let out = fx.process(Frame::mono(noise(&mut state) * 8.0), &params);
                        assert!(out.l.is_finite() && out.r.is_finite());
                        assert!(out.peak() < 30.0, "{sr} Hz, {rate_hz} Hz: {}", out.peak());
                    }
                    let min = (0.05e-3 * sr).max(2.0);
                    assert!(fx.seen.0 >= min - 1e-3, "{sr}: low {} < {min}", fx.seen.0);
                    assert!(fx.seen.1 <= fx.max_delay);
                    assert!(fx.max_delay < fx.left.line.buf.len() as f32 - 3.0);
                }
            }
        }
    }

    #[test]
    fn feedback_deepens_the_resonance() {
        // A tone on the first feedback peak, 1 / d, which is 500 Hz for 2 ms.
        let none = tone_gain(48000.0, 500.0, &comb(2.0));
        let some = tone_gain(
            48000.0,
            500.0,
            &FlangerParams {
                feedback: 0.5,
                ..comb(2.0)
            },
        );
        let lots = tone_gain(
            48000.0,
            500.0,
            &FlangerParams {
                feedback: 0.75,
                ..comb(2.0)
            },
        );
        assert!((none - 1.0).abs() < 0.02);
        assert!(some > 1.4 && some < lots, "{some} {lots}");
        assert!(lots > 1.9, "{lots}");
    }

    #[test]
    fn feedback_of_0_95_stays_bounded_over_a_long_noise_run() {
        for sr in [44100.0, 96000.0] {
            let params = FlangerParams {
                rate_hz: 0.3,
                depth: 0.7,
                feedback: 0.95,
                delay_ms: 1.5,
                spread: 0.5,
                mix: 0.5,
            };
            let mut fx = Flanger::new(sr);
            let mut state = 99;
            let mut peak = 0.0f32;
            for _ in 0..(sr as usize * 10) {
                let input = Frame::new(noise(&mut state) * 8.0, noise(&mut state) * 8.0);
                let out = fx.process(input, &params);
                assert!(out.l.is_finite() && out.r.is_finite());
                peak = peak.max(out.peak());
            }
            assert!(peak > 1.0, "the effect should be audible, peak {peak}");
            assert!(peak < 20.0, "peak {peak}");
        }
    }

    #[test]
    fn a_resonant_tone_at_full_scale_is_held_by_the_soft_limit() {
        let params = FlangerParams {
            feedback: 0.95,
            ..comb(2.0)
        };
        let mut fx = Flanger::new(48000.0);
        let mut peak = 0.0f32;
        for i in 0..48000 {
            let x = 8.0 * (std::f32::consts::TAU * 500.0 * i as f32 / 48000.0).sin();
            peak = peak.max(fx.process(Frame::mono(x), &params).l.abs());
        }
        assert!(peak < 20.0, "peak {peak}");
    }

    #[test]
    fn the_loop_roll_off_darkens_each_repeat() {
        // 5 ms at 48 kHz is 240 samples. With a static delay each repeat is one more pass
        // through the loop filter, so the second echo has less high end than the first.
        let sr = 48000.0;
        let params = FlangerParams {
            feedback: 0.9,
            mix: 1.0,
            ..comb(5.0)
        };
        let mut fx = Flanger::new(sr);
        let out: Vec<f32> = (0..2000)
            .map(|i| {
                let x = if i == 0 { 1.0 } else { 0.0 };
                fx.process(Frame::mono(x), &params).l
            })
            .collect();
        // Share of the energy that is in the sample-to-sample difference, a measure of treble.
        let brightness = |centre: usize| {
            let seg = &out[centre - 4..centre + 100];
            let diff: f32 = seg.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
            let total: f32 = seg.iter().map(|v| v * v).sum();
            diff / total
        };
        let first = brightness(240);
        let second = brightness(480);
        let third = brightness(720);
        assert!(first > 1.5, "the first echo is the raw impulse: {first}");
        assert!(second < first * 0.7, "{first} {second}");
        assert!(third < second, "{second} {third}");
    }

    #[test]
    fn spread_zero_gives_identical_sides_and_one_offsets_them() {
        let base = FlangerParams {
            rate_hz: 1.5,
            depth: 0.8,
            feedback: 0.6,
            delay_ms: 3.0,
            spread: 0.0,
            mix: 0.5,
        };
        let mut together = Flanger::new(44100.0);
        let mut apart = Flanger::new(44100.0);
        let wide = FlangerParams {
            spread: 1.0,
            ..base
        };
        let mut state = 5;
        let mut worst = 0.0f32;
        for _ in 0..44100 {
            let x = Frame::mono(noise(&mut state));
            let a = together.process(x, &base);
            assert_eq!(a.l, a.r);
            let b = apart.process(x, &wide);
            worst = worst.max((b.l - b.r).abs());
        }
        assert!(worst > 0.3, "sides differ by at most {worst}");
    }

    #[test]
    fn spread_one_sweeps_the_sides_in_opposite_directions() {
        let params = FlangerParams {
            rate_hz: 2.0,
            depth: 1.0,
            spread: 1.0,
            ..comb(4.0)
        };
        let mut fx = Flanger::new(48000.0);
        for _ in 0..2000 {
            fx.process(Frame::SILENT, &params);
            // 4 ms at 48 kHz.
            let centre = 192.0;
            let l = fx.delay_samples((std::f32::consts::TAU * fx.phase).sin());
            let mut p = fx.phase + 0.5;
            if p >= 1.0 {
                p -= 1.0;
            }
            let r = fx.delay_samples((std::f32::consts::TAU * p).sin());
            assert!(((l - centre) + (r - centre)).abs() < 0.05);
        }
    }

    #[test]
    fn output_stays_finite_for_extreme_params_and_inputs() {
        let rates = [0.05, 200.0];
        let depths = [0.0, 1.0];
        let feedbacks = [0.0, 0.95];
        let delays = [0.1, 10.0];
        let spreads = [0.0, 1.0];
        let mixes = [0.0, 1.0];
        let mut corners = Vec::new();
        for &rate_hz in &rates {
            for &depth in &depths {
                for &feedback in &feedbacks {
                    for &delay_ms in &delays {
                        for &spread in &spreads {
                            for &mix in &mixes {
                                corners.push(FlangerParams {
                                    rate_hz,
                                    depth,
                                    feedback,
                                    delay_ms,
                                    spread,
                                    mix,
                                });
                            }
                        }
                    }
                }
            }
        }
        for sr in [22050.0, 192000.0] {
            // Each corner on its own, then a run that jumps between corners every sample.
            for (k, params) in corners.iter().enumerate() {
                let mut fx = Flanger::new(sr);
                let mut state = 17 + k as u32;
                for i in 0..3000 {
                    let x = if i % 64 < 32 { 8.0 } else { -8.0 };
                    let input = Frame::new(x, noise(&mut state) * 8.0);
                    let out = fx.process(input, params);
                    assert!(out.l.is_finite() && out.r.is_finite());
                    assert!(out.peak() < 30.0, "corner {k}: {}", out.peak());
                }
            }
            let mut fx = Flanger::new(sr);
            let mut state = 1;
            for i in 0..20000usize {
                let params = &corners[(i * 7 + i / 3) % corners.len()];
                let input = Frame::new(noise(&mut state) * 8.0, noise(&mut state) * 8.0);
                let out = fx.process(input, params);
                assert!(out.l.is_finite() && out.r.is_finite());
                assert!(out.peak() < 30.0, "{}", out.peak());
            }
        }
    }

    #[test]
    fn bad_inputs_and_params_do_not_poison_the_state() {
        let mut fx = Flanger::new(44100.0);
        let bad = FlangerParams {
            rate_hz: f32::NAN,
            depth: f32::INFINITY,
            feedback: f32::NAN,
            delay_ms: f32::NAN,
            spread: f32::NAN,
            mix: f32::NAN,
        };
        let out = fx.process(Frame::new(f32::NAN, f32::INFINITY), &bad);
        assert!(out.l.is_finite() && out.r.is_finite());
        for _ in 0..1000 {
            let out = fx.process(Frame::mono(0.5), &FlangerParams::default());
            assert!(out.l.is_finite() && out.r.is_finite());
        }
    }

    #[test]
    fn a_delay_knob_move_glides_instead_of_jumping() {
        let sr = 48000.0;
        let mut fx = Flanger::new(sr);
        let mut params = FlangerParams {
            depth: 0.0,
            feedback: 0.0,
            delay_ms: 2.0,
            mix: 1.0,
            ..FlangerParams::default()
        };
        let freq = 440.0;
        let mut prev = 0.0f32;
        let mut worst_step = 0.0f32;
        for i in 0..48000 {
            if i == 20000 {
                params.delay_ms = 9.0;
            }
            let x = (std::f32::consts::TAU * freq * i as f32 / sr).sin();
            let out = fx.process(Frame::mono(x), &params).l;
            if i > 1000 {
                worst_step = worst_step.max((out - prev).abs());
            }
            prev = out;
        }
        // A 440 Hz sine moves by at most 2 pi f / sr = 0.058 per sample. A glide that bends the
        // pitch by a few percent stays near that, while a click would be far larger.
        assert!(worst_step < 0.1, "largest step {worst_step}");
    }

    #[test]
    fn reset_clears_the_tail() {
        let mut fx = Flanger::new(44100.0);
        let params = FlangerParams {
            feedback: 0.95,
            ..FlangerParams::default()
        };
        let mut state = 2;
        for _ in 0..5000 {
            fx.process(Frame::mono(noise(&mut state)), &params);
        }
        fx.reset();
        for _ in 0..5000 {
            assert_eq!(fx.process(Frame::SILENT, &params), Frame::SILENT);
        }
    }

    #[test]
    fn a_decaying_tail_reaches_true_silence() {
        let mut fx = Flanger::new(44100.0);
        let params = FlangerParams {
            feedback: 0.9,
            mix: 1.0,
            ..FlangerParams::default()
        };
        fx.process(Frame::mono(1.0), &params);
        let mut last = Frame::mono(1.0);
        for _ in 0..(44100 * 20) {
            last = fx.process(Frame::SILENT, &params);
        }
        assert_eq!(last, Frame::SILENT);
    }
}
