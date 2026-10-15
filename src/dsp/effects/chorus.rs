//! Chorus: several short modulated delays mixed with the dry signal, spread across the stereo
//! field.
//!
//! Four voices read each side's input from a delay line. Each voice has its own delay centre
//! between 12 and 22.5 ms and its own LFO phase, spaced a quarter cycle apart. `depth` swings
//! each delay by up to 2 ms either way, which is 4 ms peak to peak. The right side's LFOs run
//! ahead of the left side's by up to half a cycle, set by `spread`, so at full spread the two
//! sides see opposite modulation. `spread` also pans the voices across the field with a constant
//! power law, and at 0.0 every voice sits in the centre on both sides.
//!
//! The wet sum is scaled by one over the square root of the voice count, so noise keeps its level.
//! The dry/wet blend is equal power. At `mix` 0.0 the gains are exactly 1 and 0, and the effect
//! returns its input untouched, with no delay. There is no feedback.

use crate::dsp::delay::DelayLine;
use crate::dsp::frame::{pan_gains, Frame};
use std::f32::consts::PI;

/// How many delay voices run on each side.
const VOICES: usize = 4;

/// The delay each voice swings around, in ms. They differ a little so the voices do not comb at
/// the same frequencies.
const CENTRES_MS: [f32; VOICES] = [12.0, 15.5, 19.0, 22.5];

/// How far `depth` 1.0 swings each delay either way, in ms.
const SWING_MS: f32 = 2.0;

/// Where each voice sits in the field at `spread` 1.0, from -1.0 (left) to 1.0 (right).
const PANS: [f32; VOICES] = [-1.0, -1.0 / 3.0, 1.0 / 3.0, 1.0];

/// The longest delay any voice reaches, in ms, with a little headroom.
const MAX_DELAY_MS: f32 = 26.0;

/// The wet sum's scale. Voices are uncorrelated for most material, so their powers add.
const WET_SCALE: f32 = 0.5;

/// The time constant of the control smoothing, in seconds.
const SMOOTH_S: f32 = 0.03;

/// Below this size an input sample is flushed to zero, so a decaying tail never goes denormal.
const FLUSH: f32 = 1e-20;

/// The chorus's controls, in plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChorusParams {
    /// The speed of the modulation in Hz, from 0.05 to 200.0 (the knob is shared with the LFO).
    /// Values above about 10 Hz are an extreme effect, not a musical one, and must stay stable.
    pub rate_hz: f32,
    /// How far the delay swings, from 0.0 to 1.0.
    pub depth: f32,
    /// How far apart the left and right modulation sit, from 0.0 (identical) to 1.0 (opposite).
    pub spread: f32,
    /// Dry/wet, from 0.0 (all dry) to 1.0 (all wet).
    pub mix: f32,
}

impl Default for ChorusParams {
    fn default() -> Self {
        Self {
            rate_hz: 1.0,
            depth: 0.5,
            spread: 0.75,
            mix: 0.5,
        }
    }
}

/// A one-pole smoother that lands exactly on its target.
#[derive(Debug, Clone, Copy)]
struct Smoother {
    value: f32,
    coeff: f32,
}

impl Smoother {
    fn new(sample_rate: f32) -> Self {
        Self {
            value: 0.0,
            coeff: 1.0 - (-1.0 / (SMOOTH_S * sample_rate)).exp(),
        }
    }

    fn snap(&mut self, target: f32) {
        self.value = target;
    }

    /// Moves toward `target` and returns the new value.
    fn step(&mut self, target: f32) -> f32 {
        self.value += (target - self.value) * self.coeff;
        if (target - self.value).abs() < 1e-6 {
            self.value = target;
        }
        self.value
    }
}

/// A sine of `phase` cycles, from a parabola with one refinement step. Its error is about 0.1%,
/// which is inaudible in a modulator.
fn fast_sine(phase: f32) -> f32 {
    let x = PI * (1.0 - 2.0 * phase);
    let y = (4.0 / PI) * x - (4.0 / (PI * PI)) * x * x.abs();
    0.225 * (y * y.abs() - y) + y
}

/// What the last call to `process` did, kept for tests to read.
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
struct Probe {
    /// The delay of the left side's first voice, in samples.
    first_voice: f32,
    /// The shortest and longest delay any read asked for.
    lowest: f32,
    highest: f32,
}

pub struct Chorus {
    sample_rate: f32,
    left: DelayLine,
    right: DelayLine,
    /// The LFO phase of voice 0 on the left side, in cycles.
    phase: f32,
    rate: Smoother,
    depth: Smoother,
    spread: Smoother,
    mix: Smoother,
    /// Whether the smoothers have been set to the first params yet.
    primed: bool,
    /// The delay centres in samples, and the swing at depth 1.0.
    centres: [f32; VOICES],
    swing: f32,
    /// The gains the spread and mix currently give, recomputed only when those move.
    pan: [Frame; VOICES],
    pan_for: f32,
    dry_gain: f32,
    wet_gain: f32,
    mix_for: f32,
    #[cfg(test)]
    probe: Probe,
}

impl Chorus {
    pub fn new(sample_rate: f32) -> Self {
        let capacity = (MAX_DELAY_MS * 0.001 * sample_rate).ceil() as usize + 8;
        let mut chorus = Self {
            sample_rate,
            left: DelayLine::with_capacity(capacity),
            right: DelayLine::with_capacity(capacity),
            phase: 0.0,
            rate: Smoother::new(sample_rate),
            depth: Smoother::new(sample_rate),
            spread: Smoother::new(sample_rate),
            mix: Smoother::new(sample_rate),
            primed: false,
            centres: CENTRES_MS.map(|ms| ms * 0.001 * sample_rate),
            swing: SWING_MS * 0.001 * sample_rate,
            pan: [Frame::SILENT; VOICES],
            pan_for: f32::NAN,
            dry_gain: 1.0,
            wet_gain: 0.0,
            mix_for: f32::NAN,
            #[cfg(test)]
            probe: Probe {
                first_voice: 0.0,
                lowest: f32::MAX,
                highest: 0.0,
            },
        };
        chorus.reset();
        chorus
    }

    /// Clears the delay lines and restarts the LFOs. The controls jump to the next params.
    pub fn reset(&mut self) {
        self.left.clear();
        self.right.clear();
        self.phase = 0.0;
        self.primed = false;
    }

    pub fn process(&mut self, input: Frame, params: &ChorusParams) -> Frame {
        let rate_target = params.rate_hz.clamp(0.05, 200.0);
        let depth_target = params.depth.clamp(0.0, 1.0);
        let spread_target = params.spread.clamp(0.0, 1.0);
        let mix_target = params.mix.clamp(0.0, 1.0);
        if !self.primed {
            self.rate.snap(rate_target);
            self.depth.snap(depth_target);
            self.spread.snap(spread_target);
            self.mix.snap(mix_target);
            self.primed = true;
        }
        let rate = self.rate.step(rate_target);
        let depth = self.depth.step(depth_target);
        let spread = self.spread.step(spread_target);
        let mix = self.mix.step(mix_target);

        if mix != self.mix_for {
            self.mix_for = mix;
            let angle = mix * PI * 0.5;
            self.dry_gain = if mix >= 1.0 { 0.0 } else { angle.cos() };
            self.wet_gain = angle.sin();
        }
        if spread != self.pan_for {
            self.pan_for = spread;
            for (gains, pan) in self.pan.iter_mut().zip(PANS) {
                *gains = pan_gains(pan * spread) * WET_SCALE;
            }
        }

        self.left.push(flush(input.l));
        self.right.push(flush(input.r));
        self.phase += rate / self.sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }

        // A mix of exactly 0.0 is the dry input and nothing else.
        if self.wet_gain == 0.0 {
            return input;
        }

        let reach = depth * self.swing;
        let right_shift = 0.5 * spread;
        let mut wet = Frame::SILENT;
        for voice in 0..VOICES {
            let left_phase = self.phase + voice as f32 / VOICES as f32;
            let right_phase = left_phase + right_shift;
            let left_delay =
                self.centres[voice] + reach * fast_sine(left_phase - left_phase.floor());
            let right_delay =
                self.centres[voice] + reach * fast_sine(right_phase - right_phase.floor());
            #[cfg(test)]
            {
                if voice == 0 {
                    self.probe.first_voice = left_delay;
                }
                self.probe.lowest = self.probe.lowest.min(left_delay.min(right_delay));
                self.probe.highest = self.probe.highest.max(left_delay.max(right_delay));
            }
            let gains = self.pan[voice];
            wet.l += self.left.read(left_delay) * gains.l;
            wet.r += self.right.read(right_delay) * gains.r;
        }
        input * self.dry_gain + wet * self.wet_gain
    }
}

/// Zeroes a sample too small to matter, so a decaying tail never reaches the denormal range.
fn flush(x: f32) -> f32 {
    if x.abs() < FLUSH {
        0.0
    } else {
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    const RATES: [f32; 4] = [22050.0, 44100.0, 96000.0, 192000.0];

    /// A deterministic noise source in [-1, 1].
    struct Noise(u32);

    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
            (self.0 >> 8) as f32 / (1u32 << 23) as f32 - 1.0
        }
    }

    fn params(rate_hz: f32, depth: f32, spread: f32, mix: f32) -> ChorusParams {
        ChorusParams {
            rate_hz,
            depth,
            spread,
            mix,
        }
    }

    /// The size of `signal` at `freq`, by a single-bin DFT, scaled so a sine of amplitude 1 reads
    /// 1.
    fn goertzel(signal: &[f32], freq: f32, sample_rate: f32) -> f32 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (n, &x) in signal.iter().enumerate() {
            let angle = std::f64::consts::TAU * freq as f64 * n as f64 / sample_rate as f64;
            re += x as f64 * angle.cos();
            im -= x as f64 * angle.sin();
        }
        (2.0 * (re * re + im * im).sqrt() / signal.len() as f64) as f32
    }

    fn rms(signal: &[f32]) -> f32 {
        (signal.iter().map(|x| x * x).sum::<f32>() / signal.len() as f32).sqrt()
    }

    /// Runs `seconds` of a mono sine through a chorus and returns the left output.
    fn run_sine(
        sample_rate: f32,
        tone_hz: f32,
        p: &ChorusParams,
        seconds: f32,
        skip_seconds: f32,
    ) -> Vec<f32> {
        let mut chorus = Chorus::new(sample_rate);
        let total = (seconds * sample_rate) as usize;
        let skip = (skip_seconds * sample_rate) as usize;
        let mut out = Vec::new();
        for n in 0..total {
            let x = (TAU * tone_hz * n as f32 / sample_rate).sin();
            let y = chorus.process(Frame::mono(x), p);
            if n >= skip {
                out.push(y.l);
            }
        }
        out
    }

    /// The peak of each block of `block` samples.
    fn envelope(signal: &[f32], block: usize) -> Vec<f32> {
        signal
            .chunks_exact(block)
            .map(|c| c.iter().fold(0.0f32, |m, x| m.max(x.abs())))
            .collect()
    }

    fn remove_mean(signal: &[f32]) -> Vec<f32> {
        let mean = signal.iter().sum::<f32>() / signal.len() as f32;
        signal.iter().map(|x| x - mean).collect()
    }

    #[test]
    fn mix_zero_returns_the_dry_input_bit_for_bit() {
        let mut chorus = Chorus::new(44100.0);
        let mut noise = Noise(1);
        let p = params(3.0, 1.0, 1.0, 0.0);
        for _ in 0..20000 {
            let input = Frame::new(noise.next() * 8.0, noise.next() * 8.0);
            assert_eq!(chorus.process(input, &p), input);
        }
    }

    #[test]
    fn mix_returns_to_exact_dry_after_it_is_turned_down() {
        let mut chorus = Chorus::new(44100.0);
        let mut noise = Noise(2);
        let wet = params(1.0, 0.5, 0.5, 0.7);
        for _ in 0..5000 {
            chorus.process(Frame::mono(noise.next()), &wet);
        }
        let dry = params(1.0, 0.5, 0.5, 0.0);
        for _ in 0..44100 {
            chorus.process(Frame::mono(noise.next()), &dry);
        }
        for _ in 0..1000 {
            let input = Frame::new(noise.next(), noise.next());
            assert_eq!(chorus.process(input, &dry), input);
        }
    }

    #[test]
    fn full_mix_has_no_dry_signal_and_the_wet_starts_after_the_shortest_delay() {
        let sample_rate = 44100.0;
        let mut chorus = Chorus::new(sample_rate);
        let p = params(1.0, 1.0, 0.5, 1.0);
        let mut out = Vec::new();
        for n in 0..2000 {
            let x = if n == 0 { 1.0 } else { 0.0 };
            out.push(chorus.process(Frame::mono(x), &p).l);
        }
        // The shortest delay is 12 ms minus 2 ms of swing, about 441 samples. Allow the cubic's
        // reach of two samples.
        let first = out.iter().position(|x| x.abs() > 1e-6).unwrap();
        assert!(first > 435, "wet started at sample {first}");
    }

    #[test]
    fn a_steady_sine_is_modulated_at_the_rate() {
        let sample_rate = 48000.0;
        let tone = 3000.0;
        for rate in [2.0, 5.0] {
            let p = params(rate, 1.0, 0.0, 1.0);
            // Whole cycles of the modulation, after the smoothers settle.
            let seconds = 8.0 / rate;
            let out = run_sine(sample_rate, tone, &p, seconds + 0.5, 0.5);
            let env = remove_mean(&envelope(&out, 16));
            let env_rate = sample_rate / 16.0;
            let off_rate = goertzel(&env, rate * 1.37, env_rate);
            let mean_level = rms(&out);
            // The sum of voices leaves a comb that shifts as the delays move, so the level moves.
            // The motion has to have energy at the rate or at a multiple of it from the voice
            // phases, and it must be a real fraction of the level.
            let best = (1..=4)
                .map(|k| goertzel(&env, rate * k as f32, env_rate))
                .fold(0.0f32, f32::max);
            assert!(best > 0.05 * mean_level, "rate {rate}, best {best}");
            assert!(best > 4.0 * off_rate, "rate {rate}, {best} vs {off_rate}");
        }
    }

    /// With one voice's delay tracked directly, the swing is a sine at the rate.
    #[test]
    fn the_delay_swings_at_the_rate() {
        let sample_rate = 44100.0;
        for rate in [0.5, 4.0, 20.0] {
            let mut chorus = Chorus::new(sample_rate);
            let p = params(rate, 1.0, 0.0, 1.0);
            let mut track = Vec::new();
            let total = (sample_rate * 8.0 / rate) as usize;
            for _ in 0..total {
                chorus.process(Frame::mono(0.0), &p);
                track.push(chorus.probe.first_voice);
            }
            let track = remove_mean(&track);
            let at_rate = goertzel(&track, rate, sample_rate);
            let off_rate = goertzel(&track, rate * 1.5, sample_rate);
            let expected = SWING_MS * 0.001 * sample_rate;
            assert!(
                (at_rate - expected).abs() < 0.03 * expected,
                "{rate}: {at_rate}"
            );
            assert!(off_rate < 0.02 * expected, "{rate}: {off_rate}");
        }
    }

    /// At depth 0.0 the delays do not move, so a sine comes out as a steady sine of the same
    /// pitch with a constant level.
    #[test]
    fn depth_zero_is_a_static_comb() {
        let sample_rate = 44100.0;
        let tone = 1000.0;
        let p = params(7.0, 0.0, 0.8, 1.0);
        let out = run_sine(sample_rate, tone, &p, 1.0, 0.3);
        let env = envelope(&out, 441);
        let (lo, hi) = env
            .iter()
            .fold((f32::MAX, 0.0f32), |(l, h), &e| (l.min(e), h.max(e)));
        assert!(hi > 0.05, "silent comb");
        assert!((hi - lo) < 0.01 * hi, "level moved: {lo} to {hi}");
        // All the energy is at the input pitch.
        let main = goertzel(&out, tone, sample_rate);
        let power_ratio = main / (rms(&out) * std::f32::consts::SQRT_2);
        assert!(power_ratio > 0.99, "{power_ratio}");
    }

    #[test]
    fn depth_scales_the_delay_swing() {
        let sample_rate = 44100.0;
        let swing = |depth: f32| {
            let mut chorus = Chorus::new(sample_rate);
            let p = params(3.0, depth, 0.5, 1.0);
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for n in 0..(sample_rate as usize * 2) {
                chorus.process(Frame::mono(0.0), &p);
                if n > 1000 {
                    lo = lo.min(chorus.probe.first_voice);
                    hi = hi.max(chorus.probe.first_voice);
                }
            }
            (hi - lo) / sample_rate * 1000.0
        };
        assert!(swing(0.0) < 1e-3);
        let (quarter, half, full) = (swing(0.25), swing(0.5), swing(1.0));
        assert!((full - 4.0).abs() < 0.1, "full swing is {full} ms");
        assert!((half / full - 0.5).abs() < 0.02, "{half} of {full}");
        assert!((quarter / full - 0.25).abs() < 0.02, "{quarter} of {full}");
    }

    #[test]
    fn spread_zero_gives_identical_sides_for_a_centred_input() {
        for rate in RATES {
            let mut chorus = Chorus::new(rate);
            let mut noise = Noise(3);
            let p = params(2.0, 1.0, 0.0, 0.6);
            for _ in 0..10000 {
                let out = chorus.process(Frame::mono(noise.next()), &p);
                assert_eq!(out.l, out.r);
            }
        }
    }

    #[test]
    fn full_spread_decorrelates_the_sides() {
        let sample_rate = 44100.0;
        let correlation = |spread: f32| {
            let mut chorus = Chorus::new(sample_rate);
            let mut noise = Noise(4);
            let p = params(1.5, 1.0, spread, 1.0);
            let (mut ll, mut rr, mut lr) = (0.0f64, 0.0f64, 0.0f64);
            for n in 0..(sample_rate as usize * 2) {
                let out = chorus.process(Frame::mono(noise.next()), &p);
                if n > 4000 {
                    ll += (out.l * out.l) as f64;
                    rr += (out.r * out.r) as f64;
                    lr += (out.l * out.r) as f64;
                }
            }
            (lr / (ll * rr).sqrt()).abs()
        };
        assert!(correlation(0.0) > 0.999);
        let wide = correlation(1.0);
        assert!(wide < 0.2, "correlation at full spread is {wide}");
        assert!(correlation(0.5) > wide);
    }

    #[test]
    fn full_spread_runs_the_right_side_in_opposite_phase() {
        // With a mono input, the left voice 0 and the right voice 0 swing in opposite directions.
        // The right side delay is the centre minus the left's swing.
        let sample_rate = 44100.0;
        let mut chorus = Chorus::new(sample_rate);
        let p = params(1.0, 1.0, 1.0, 1.0);
        for _ in 0..2000 {
            chorus.process(Frame::mono(0.0), &p);
        }
        let centre = CENTRES_MS[0] * 0.001 * sample_rate;
        let left = chorus.probe.first_voice - centre;
        let phase = chorus.phase;
        let right = SWING_MS * 0.001 * sample_rate * fast_sine((phase + 0.5).fract());
        assert!((left + right).abs() < 0.05, "{left} and {right}");
    }

    #[test]
    fn extreme_rates_stay_stable_and_inside_the_buffer() {
        for sample_rate in RATES {
            for rate in [0.05, 1.0, 10.0, 60.0, 200.0] {
                for spread in [0.0, 1.0] {
                    let mut chorus = Chorus::new(sample_rate);
                    let mut noise = Noise(5);
                    let p = params(rate, 1.0, spread, 1.0);
                    let max_delay = chorus.left.max_delay();
                    for _ in 0..(sample_rate as usize / 2) {
                        let input = Frame::new(noise.next() * 8.0, noise.next() * 8.0);
                        let out = chorus.process(input, &p);
                        assert!(out.l.is_finite() && out.r.is_finite());
                        assert!(out.l.abs() < 40.0 && out.r.abs() < 40.0);
                    }
                    assert!(chorus.probe.lowest >= 1.0, "{sample_rate} {rate}");
                    assert!(
                        chorus.probe.highest <= max_delay,
                        "{sample_rate} {rate}: {} > {max_delay}",
                        chorus.probe.highest
                    );
                }
            }
        }
    }

    /// The delay in milliseconds does not depend on the sample rate, and a tone keeps its pitch.
    #[test]
    fn every_sample_rate_gives_the_same_delays_in_time() {
        for sample_rate in RATES {
            let mut chorus = Chorus::new(sample_rate);
            let p = params(2.0, 1.0, 0.5, 1.0);
            for _ in 0..(sample_rate as usize) {
                chorus.process(Frame::mono(0.0), &p);
            }
            let lowest_ms = chorus.probe.lowest / sample_rate * 1000.0;
            let highest_ms = chorus.probe.highest / sample_rate * 1000.0;
            assert!((lowest_ms - 10.0).abs() < 0.1, "{sample_rate}: {lowest_ms}");
            assert!(
                (highest_ms - 24.5).abs() < 0.1,
                "{sample_rate}: {highest_ms}"
            );

            let out = run_sine(sample_rate, 1000.0, &params(2.0, 0.0, 0.5, 1.0), 0.6, 0.3);
            assert!(goertzel(&out, 1000.0, sample_rate) > 0.05, "{sample_rate}");
        }
    }

    #[test]
    fn noise_keeps_its_level_at_half_mix() {
        for sample_rate in RATES {
            for spread in [0.0, 0.75, 1.0] {
                let mut chorus = Chorus::new(sample_rate);
                let mut noise = Noise(6);
                let p = params(1.0, 0.7, spread, 0.5);
                let (mut input_power, mut output_power) = (0.0f64, 0.0f64);
                let total = sample_rate as usize * 2;
                for n in 0..total {
                    let x = noise.next();
                    let out = chorus.process(Frame::mono(x), &p);
                    if n > total / 4 {
                        input_power += (x * x) as f64;
                        output_power += ((out.l * out.l + out.r * out.r) * 0.5) as f64;
                    }
                }
                let db = 10.0 * (output_power / input_power).log10();
                assert!(db.abs() < 2.5, "{sample_rate} spread {spread}: {db} dB");
            }
        }
    }

    #[test]
    fn output_is_finite_for_extreme_params_and_inputs() {
        let corners = [0.0, 1.0];
        let rates = [0.05, 200.0];
        for sample_rate in RATES {
            let mut chorus = Chorus::new(sample_rate);
            let mut noise = Noise(7);
            // The params jump between corners every 500 samples.
            for step in 0..64 {
                let p = params(
                    rates[step % 2],
                    corners[(step / 2) % 2],
                    corners[(step / 4) % 2],
                    corners[(step / 8) % 2],
                );
                for n in 0..500 {
                    let input = match n % 3 {
                        0 => Frame::new(8.0, -8.0),
                        1 => Frame::new(noise.next() * 8.0, noise.next() * 8.0),
                        _ => Frame::new(-8.0, 8.0),
                    };
                    let out = chorus.process(input, &p);
                    assert!(out.l.is_finite() && out.r.is_finite());
                    assert!(out.l.abs() < 40.0 && out.r.abs() < 40.0);
                }
            }
        }
    }

    #[test]
    fn a_changed_rate_does_not_click() {
        let sample_rate = 44100.0;
        let mut chorus = Chorus::new(sample_rate);
        let tone = 200.0;
        let mut last = 0.0f32;
        let mut biggest_step = 0.0f32;
        for n in 0..(sample_rate as usize * 2) {
            let rate = if n < sample_rate as usize { 0.5 } else { 8.0 };
            let x = (TAU * tone * n as f32 / sample_rate).sin();
            let out = chorus
                .process(Frame::mono(x), &params(rate, 0.5, 0.5, 0.5))
                .l;
            if n > 1000 {
                biggest_step = biggest_step.max((out - last).abs());
            }
            last = out;
        }
        // A 200 Hz sine of amplitude 1 moves at most 0.03 per sample.
        assert!(biggest_step < 0.2, "{biggest_step}");
    }

    #[test]
    fn reset_clears_the_tail() {
        let mut chorus = Chorus::new(44100.0);
        let p = params(1.0, 0.5, 0.5, 1.0);
        let mut noise = Noise(8);
        for _ in 0..5000 {
            chorus.process(Frame::mono(noise.next()), &p);
        }
        chorus.reset();
        for _ in 0..2000 {
            let out = chorus.process(Frame::SILENT, &p);
            assert_eq!(out, Frame::SILENT);
        }
    }
}
