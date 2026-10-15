//! Phaser: a chain of all-pass filters whose frequency sweeps, summed with the dry signal so the
//! phase shift cuts notches.
//!
//! Each stage is a first-order all-pass, `H(z) = (a + z^-1) / (1 + a z^-1)`, in transposed form.
//! Its phase falls from 0 at DC to -180 degrees at Nyquist and passes -90 degrees at the stage
//! frequency. Where the whole chain reaches an odd multiple of -180 degrees the wet signal is
//! opposite to the dry one and the sum cancels, so a chain of N stages cuts N/2 notches.
//!
//! The sweep law, feedback placement, wet/dry law, and the way the stage count changes are
//! described on `Phaser`.

use crate::dsp::frame::Frame;

/// The phaser's controls, in plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhaserParams {
    /// The speed of the sweep in Hz, from 0.05 to 200.0 (the knob is shared with the LFO).
    pub rate_hz: f32,
    /// How far the sweep reaches around the centre, from 0.0 to 1.0.
    pub depth: f32,
    /// How much of the output goes back in, from 0.0 to 0.95.
    pub feedback: f32,
    /// The middle of the sweep in Hz, from 100.0 to 8000.0.
    pub center_hz: f32,
    /// The number of all-pass stages: 4, 6, 8, or 12. Each pair makes one notch.
    pub stages: usize,
    /// How far apart the left and right sweeps sit, from 0.0 (identical) to 1.0 (opposite).
    pub spread: f32,
    /// Dry/wet, from 0.0 (all dry) to 1.0 (all wet).
    pub mix: f32,
}

impl Default for PhaserParams {
    fn default() -> Self {
        Self {
            rate_hz: 1.0,
            depth: 0.5,
            feedback: 0.4,
            center_hz: 1000.0,
            stages: 4,
            spread: 0.75,
            mix: 0.5,
        }
    }
}

/// The most stages the chain can run.
const MAX_STAGES: usize = 12;
/// Samples between updates of the all-pass coefficient. The coefficient slews linearly between
/// updates, so the sweep stays smooth while the sine, `exp2`, and `tan` run once per block.
const BLOCK: usize = 8;
/// The sweep reaches this many octaves each way at full depth.
const SWEEP_OCTAVES: f32 = 3.0;
/// The all-pass frequency never goes above this fraction of the sample rate.
const MAX_FREQ_RATIO: f32 = 0.45;
/// The all-pass frequency never goes below this many Hz.
const MIN_FREQ_HZ: f32 = 10.0;
/// How long a stage takes to fade in or out when the stage count changes, in seconds.
const STAGE_FADE_S: f32 = 0.03;
/// The time constant of the parameter smoothers, in seconds.
const SMOOTH_S: f32 = 0.015;
/// The feedback signal is clamped to this magnitude, as a guard against runaway levels.
const FEEDBACK_LIMIT: f32 = 16.0;

/// Values under this magnitude are flushed to zero so tails never reach denormals.
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 {
        0.0
    } else {
        x
    }
}

/// A one-pole smoother that lands exactly on its target once it is close.
#[derive(Debug, Clone, Copy)]
struct Smoother {
    value: f32,
}

impl Smoother {
    fn step(&mut self, target: f32, coef: f32) -> f32 {
        let diff = target - self.value;
        if diff.abs() < 1e-6 {
            self.value = target;
        } else {
            self.value += diff * coef;
        }
        self.value
    }
}

/// One side's all-pass chain and its feedback memory.
#[derive(Debug, Clone, Copy)]
struct Chain {
    /// The transposed-form memory of each stage.
    state: [f32; MAX_STAGES],
    /// The previous chain output, which feeds back to the input.
    last: f32,
}

impl Chain {
    const fn new() -> Self {
        Self {
            state: [0.0; MAX_STAGES],
            last: 0.0,
        }
    }

    /// Runs `x` through the chain with coefficient `a` and returns the wet sum, half of the chain
    /// input plus half of the chain output. `weights` is how engaged each stage is, from 0.0
    /// (bypassed) to 1.0 (a full all-pass). Feedback `fb` mixes the previous wet sum into the
    /// chain input, so the dry half of the sum carries it too. That keeps the notches at full
    /// depth, because the dry and wet halves still cancel exactly where the chain is at 180
    /// degrees, while the loop raises the peaks between them.
    fn run(&mut self, x: f32, a: f32, fb: f32, weights: &[f32; MAX_STAGES]) -> f32 {
        // The input is scaled down a little as feedback rises, which holds back the level of the
        // resonant peaks without thinning the sound much.
        let input = (x + fb * self.last) * (1.0 - 0.3 * fb);
        let mut v = input;
        for (state, &w) in self.state.iter_mut().zip(weights) {
            if w <= 0.0 {
                *state = 0.0;
                continue;
            }
            let y = a * v + *state;
            *state = flush(v - a * y);
            v += w * (y - v);
        }
        let wet = 0.5 * (input + v);
        self.last = flush(wet.clamp(-FEEDBACK_LIMIT, FEEDBACK_LIMIT));
        wet
    }
}

/// A stereo phaser.
///
/// The sweep is exponential. The LFO is a sine, and the all-pass frequency is
/// `center_hz * 2^(3 * depth * lfo)`, so full depth reaches three octaves each way. The result is
/// clamped between 10 Hz and 0.45 of the sample rate. The right side's LFO phase leads the left
/// by half a cycle times `spread`.
///
/// All stages on one side share one frequency, so the notches sit where the chain's phase crosses
/// odd multiples of -180 degrees. Feedback takes the wet sum back to the chain input through one
/// sample of delay, scaled by `feedback` and by `1 - feedback / 2` to hold back the level.
///
/// The wet sum is `0.5 * (dry + chain)`, and the output is `dry * (1 - mix) + mix * wet`. At a
/// mix of 1.0 the output is the classic phaser sum, scaled by one half so it never exceeds the
/// dry level without feedback, and it still has the notches. At 0.0 the dry input comes back bit
/// for bit.
///
/// Stages above the requested count fade out, and new ones fade in, over 30 ms. A stage in the
/// middle of a fade blends its input with its all-pass output.
pub struct Phaser {
    sample_rate: f32,
    chains: [Chain; 2],
    /// How engaged each stage is, shared by both sides.
    weights: [f32; MAX_STAGES],
    /// The weight step per sample while a stage fades.
    weight_step: f32,
    /// The LFO phase of the left side, from 0.0 to 1.0.
    phase: f32,
    /// The current all-pass coefficient for each side.
    coef: [f32; 2],
    /// How much `coef` changes per sample on the way to the next block's value.
    coef_step: [f32; 2],
    /// Samples left in the current block.
    block_left: usize,
    /// The smoothed controls.
    depth: Smoother,
    spread: Smoother,
    feedback: Smoother,
    mix: Smoother,
    center_log2: Smoother,
    /// The centre that `center_log2` was last computed from, so the `log2` runs only on change.
    center_cache: (f32, f32),
    /// The one-pole coefficient for the smoothers.
    smooth_coef: f32,
    /// False until the first sample, when everything snaps to the first params.
    primed: bool,
}

impl Phaser {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        let mut phaser = Self {
            sample_rate,
            chains: [Chain::new(); 2],
            weights: [0.0; MAX_STAGES],
            weight_step: 1.0 / (STAGE_FADE_S * sample_rate),
            phase: 0.0,
            coef: [0.0; 2],
            coef_step: [0.0; 2],
            block_left: 0,
            depth: Smoother { value: 0.0 },
            spread: Smoother { value: 0.0 },
            feedback: Smoother { value: 0.0 },
            mix: Smoother { value: 0.0 },
            center_log2: Smoother { value: 0.0 },
            center_cache: (0.0, 0.0),
            smooth_coef: 1.0 - (-1.0 / (SMOOTH_S * sample_rate)).exp(),
            primed: false,
        };
        phaser.reset();
        phaser
    }

    /// Clears the filter memory and the sweep. The next sample snaps the smoothers to its params.
    pub fn reset(&mut self) {
        self.chains = [Chain::new(); 2];
        self.weights = [0.0; MAX_STAGES];
        self.phase = 0.0;
        self.block_left = 0;
        self.primed = false;
    }

    /// The all-pass coefficient for a sweep position, for the side whose LFO is at `phase`.
    fn coefficient(&self, center_log2: f32, depth: f32, phase: f32) -> f32 {
        let lfo = (std::f32::consts::TAU * phase).sin();
        let octaves = depth * SWEEP_OCTAVES * lfo;
        let max_hz = MAX_FREQ_RATIO * self.sample_rate;
        let hz = (center_log2 + octaves).exp2().clamp(MIN_FREQ_HZ, max_hz);
        let t = (std::f32::consts::PI * hz / self.sample_rate).tan();
        (t - 1.0) / (t + 1.0)
    }

    /// Sets the coefficient targets for the next block and the per-sample steps toward them.
    fn start_block(&mut self, center_log2: f32, depth: f32, spread: f32, phase_inc: f32) {
        let ahead = self.phase + BLOCK as f32 * phase_inc;
        for side in 0..2 {
            let offset = if side == 1 { 0.5 * spread } else { 0.0 };
            let target = self.coefficient(center_log2, depth, ahead + offset);
            self.coef_step[side] = (target - self.coef[side]) / BLOCK as f32;
        }
        self.block_left = BLOCK;
    }

    /// Moves every stage weight one step toward its target for `stages` stages.
    fn step_weights(&mut self, stages: usize) {
        for (k, w) in self.weights.iter_mut().enumerate() {
            let target = if k < stages { 1.0 } else { 0.0 };
            if *w < target {
                *w = (*w + self.weight_step).min(target);
            } else if *w > target {
                *w = (*w - self.weight_step).max(target);
            }
        }
    }

    /// Takes the smoothers and the sweep straight to the params, for the first sample.
    fn prime(&mut self, depth: f32, spread: f32, feedback: f32, mix: f32, center_log2: f32) {
        self.depth.value = depth;
        self.spread.value = spread;
        self.feedback.value = feedback;
        self.mix.value = mix;
        self.center_log2.value = center_log2;
        for side in 0..2 {
            let offset = if side == 1 { 0.5 * spread } else { 0.0 };
            self.coef[side] = self.coefficient(center_log2, depth, self.phase + offset);
            self.coef_step[side] = 0.0;
        }
        self.block_left = 0;
        self.primed = true;
    }

    pub fn process(&mut self, input: Frame, params: &PhaserParams) -> Frame {
        let max_hz = MAX_FREQ_RATIO * self.sample_rate;
        let rate = params.rate_hz.clamp(0.0, max_hz);
        let depth_t = params.depth.clamp(0.0, 1.0);
        let feedback_t = params.feedback.clamp(0.0, 0.95);
        let spread_t = params.spread.clamp(0.0, 1.0);
        let mix_t = params.mix.clamp(0.0, 1.0);
        let center = params.center_hz.clamp(MIN_FREQ_HZ, max_hz);
        if center != self.center_cache.0 {
            self.center_cache = (center, center.log2());
        }
        let center_t = self.center_cache.1;
        let stages = params.stages.min(MAX_STAGES);

        if !self.primed {
            self.prime(depth_t, spread_t, feedback_t, mix_t, center_t);
            for (k, w) in self.weights.iter_mut().enumerate() {
                *w = if k < stages { 1.0 } else { 0.0 };
            }
        }

        let c = self.smooth_coef;
        let depth = self.depth.step(depth_t, c);
        let spread = self.spread.step(spread_t, c);
        let feedback = self.feedback.step(feedback_t, c);
        let mix = self.mix.step(mix_t, c);
        let center_log2 = self.center_log2.step(center_t, c);

        let phase_inc = rate / self.sample_rate;
        if self.block_left == 0 {
            self.start_block(center_log2, depth, spread, phase_inc);
        }
        self.block_left -= 1;
        self.step_weights(stages);
        for side in 0..2 {
            self.coef[side] += self.coef_step[side];
        }
        self.phase += phase_inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }

        let wet_l = self.chains[0].run(input.l, self.coef[0], feedback, &self.weights);
        let wet_r = self.chains[1].run(input.r, self.coef[1], feedback, &self.weights);
        if mix == 0.0 {
            return input;
        }
        let blend = |dry: f32, wet: f32| dry * (1.0 - mix) + mix * wet;
        Frame::new(blend(input.l, wet_l), blend(input.r, wet_r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::rng::Rng;
    use std::f32::consts::PI;

    const FS: f32 = 44100.0;

    /// A static, fully wet phaser with no feedback, so the response is easy to predict.
    fn static_params(stages: usize) -> PhaserParams {
        PhaserParams {
            rate_hz: 1.0,
            depth: 0.0,
            feedback: 0.0,
            center_hz: 1000.0,
            stages,
            spread: 0.0,
            mix: 1.0,
        }
    }

    /// The frequency of an all-pass coefficient.
    fn freq_of(a: f32, fs: f32) -> f32 {
        let t = (1.0 + a) / (1.0 - a);
        t.atan() * fs / PI
    }

    /// The impulse response of the left side, which for fixed params is the exact response.
    fn impulse_response(fs: f32, params: &PhaserParams, len: usize) -> Vec<f32> {
        let mut fx = Phaser::new(fs);
        (0..len)
            .map(|n| {
                let x = if n == 0 { 1.0 } else { 0.0 };
                fx.process(Frame::mono(x), params).l
            })
            .collect()
    }

    /// The magnitude of an impulse response at `hz`.
    fn magnitude(h: &[f32], hz: f32, fs: f32) -> f32 {
        let w = 2.0 * std::f64::consts::PI * hz as f64 / fs as f64;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (n, &v) in h.iter().enumerate() {
            let angle = w * n as f64;
            re += v as f64 * angle.cos();
            im -= v as f64 * angle.sin();
        }
        (re * re + im * im).sqrt() as f32
    }

    /// The impulse response of the wet chain alone, recovered from the full-wet output
    /// `0.5 * (dry + chain)`.
    fn chain_response(fs: f32, params: &PhaserParams, len: usize) -> Vec<f32> {
        let mut h = impulse_response(fs, params, len);
        for v in h.iter_mut() {
            *v *= 2.0;
        }
        h[0] -= 1.0;
        h
    }

    /// Where the notches of `stages` all-pass stages sit for a stage frequency of `center`.
    fn notch_freqs(stages: usize, center: f32, fs: f32) -> Vec<f32> {
        let t = (PI * center / fs).tan();
        (0..stages / 2)
            .map(|k| {
                let u = ((2 * k + 1) as f32 * PI / (2 * stages) as f32).tan();
                (t * u).atan() * fs / PI
            })
            .collect()
    }

    /// The response at `points` frequencies spread evenly from 0 to Nyquist.
    fn gain_curve(h: &[f32], fs: f32, points: usize) -> Vec<f32> {
        (0..points)
            .map(|i| magnitude(h, 0.5 * fs * i as f32 / (points - 1) as f32, fs))
            .collect()
    }

    /// The frequency of the left all-pass each sample over `seconds`.
    fn sweep(fs: f32, params: &PhaserParams, seconds: f32) -> Vec<f32> {
        let mut fx = Phaser::new(fs);
        (0..(seconds * fs) as usize)
            .map(|_| {
                fx.process(Frame::SILENT, params);
                freq_of(fx.coef[0], fs)
            })
            .collect()
    }

    fn noise(rng: &mut Rng, amp: f32) -> Frame {
        Frame::new(rng.signed() * amp, rng.signed() * amp)
    }

    #[test]
    fn mix_zero_returns_the_dry_input_exactly() {
        let mut fx = Phaser::new(FS);
        let mut rng = Rng::new(7);
        let mut p = PhaserParams {
            mix: 0.0,
            feedback: 0.9,
            depth: 1.0,
            ..PhaserParams::default()
        };
        for n in 0..20000 {
            p.stages = [4, 6, 8, 12][(n / 3000) % 4];
            p.rate_hz = 0.05 + (n % 997) as f32 * 0.2;
            let x = noise(&mut rng, 8.0);
            assert_eq!(fx.process(x, &p), x, "sample {n}");
        }
    }

    #[test]
    fn mix_returns_to_exact_dry_after_it_is_turned_down() {
        let mut fx = Phaser::new(FS);
        let mut rng = Rng::new(3);
        let mut p = PhaserParams {
            mix: 1.0,
            ..PhaserParams::default()
        };
        for _ in 0..5000 {
            fx.process(noise(&mut rng, 1.0), &p);
        }
        p.mix = 0.0;
        // The mix smoother takes a fraction of a second to settle, then lands exactly on zero.
        for _ in 0..30000 {
            fx.process(noise(&mut rng, 1.0), &p);
        }
        for _ in 0..20000 {
            let x = noise(&mut rng, 1.0);
            assert_eq!(fx.process(x, &p), x);
        }
    }

    #[test]
    fn the_chain_has_unit_magnitude_at_every_frequency() {
        for stages in [4, 6, 8, 12] {
            let h = chain_response(FS, &static_params(stages), 32768);
            for i in 1..60 {
                let hz = 20.0 * (0.49 * FS / 20.0f32).powf(i as f32 / 59.0);
                let m = magnitude(&h, hz, FS);
                assert!((m - 1.0).abs() < 0.01, "{stages} stages at {hz} Hz: {m}");
            }
        }
    }

    #[test]
    fn the_chain_keeps_the_energy_of_noise() {
        for stages in [4, 8, 12] {
            let mut fx = Phaser::new(FS);
            let mut rng = Rng::new(11);
            let p = PhaserParams {
                stages,
                mix: 1.0,
                feedback: 0.0,
                depth: 0.6,
                rate_hz: 3.0,
                ..PhaserParams::default()
            };
            let (mut e_in, mut e_wet) = (0.0f64, 0.0f64);
            for n in 0..200000 {
                let x = noise(&mut rng, 1.0);
                let out = fx.process(x, &p);
                if n > 2000 {
                    let wet = 2.0 * out.l - x.l;
                    e_in += (x.l * x.l) as f64;
                    e_wet += (wet * wet) as f64;
                }
            }
            let ratio = e_wet / e_in;
            assert!((ratio - 1.0).abs() < 0.03, "{stages} stages: {ratio}");
        }
    }

    #[test]
    fn the_sum_has_one_notch_per_two_stages_at_the_expected_frequencies() {
        for stages in [4, 6, 8, 12] {
            let h = impulse_response(FS, &static_params(stages), 32768);
            let expected = notch_freqs(stages, 1000.0, FS);
            assert_eq!(expected.len(), stages / 2);
            for f in &expected {
                let g = magnitude(&h, *f, FS);
                assert!(g < 0.01, "{stages} stages, notch at {f} Hz has gain {g}");
            }
            // Counting dips on a grid finds exactly stages / 2 notches.
            let curve = gain_curve(&h, FS, 3000);
            let dips = (1..curve.len() - 1)
                .filter(|&i| curve[i] < 0.5 && curve[i] <= curve[i - 1] && curve[i] < curve[i + 1])
                .count();
            assert_eq!(dips, stages / 2, "{stages} stages");
            // DC passes at full level, and the peaks between notches reach it.
            assert!((curve[0] - 1.0).abs() < 0.01);
            assert!(curve.iter().cloned().fold(0.0, f32::max) > 0.98);
        }
    }

    #[test]
    fn the_notches_land_correctly_at_every_sample_rate() {
        for fs in [22050.0, 44100.0, 96000.0, 192000.0] {
            for center in [200.0, 1000.0, 5000.0] {
                let p = PhaserParams {
                    center_hz: center,
                    ..static_params(8)
                };
                let h = impulse_response(fs, &p, 65536);
                for f in notch_freqs(8, center, fs) {
                    let g = magnitude(&h, f, fs);
                    assert!(g < 0.02, "{fs} Hz rate, centre {center}, notch {f}: {g}");
                }
            }
        }
    }

    #[test]
    fn the_notches_move_with_the_lfo_at_the_rate() {
        for rate in [0.5f32, 3.0, 20.0, 200.0] {
            let seconds = (6.0 / rate).max(1.0);
            let p = PhaserParams {
                rate_hz: rate,
                depth: 0.5,
                ..static_params(4)
            };
            let freqs = sweep(FS, &p, seconds);
            let rising = freqs
                .windows(2)
                .filter(|w| w[0] < 1000.0 && w[1] >= 1000.0)
                .count() as f32;
            let expected = rate * seconds;
            assert!(
                (rising - expected).abs() <= 1.0 + 0.02 * expected,
                "rate {rate}: {rising} sweeps, expected {expected}"
            );
        }
    }

    #[test]
    fn the_centre_shifts_the_sweep() {
        for center in [100.0, 500.0, 2000.0, 8000.0] {
            let p = PhaserParams {
                rate_hz: 2.0,
                depth: 0.3,
                center_hz: center,
                ..static_params(4)
            };
            let freqs = sweep(FS, &p, 1.0);
            let lo = freqs.iter().cloned().fold(f32::MAX, f32::min);
            let hi = freqs.iter().cloned().fold(0.0, f32::max);
            let mid = (lo * hi).sqrt();
            assert!((mid / center - 1.0).abs() < 0.03, "{center}: {mid}");
        }
    }

    #[test]
    fn depth_scales_the_sweep_width_in_octaves() {
        for depth in [0.0, 0.2, 0.5, 1.0] {
            let p = PhaserParams {
                rate_hz: 2.0,
                depth,
                ..static_params(4)
            };
            let freqs = sweep(FS, &p, 1.0);
            let lo = freqs.iter().cloned().fold(f32::MAX, f32::min);
            let hi = freqs.iter().cloned().fold(0.0, f32::max);
            let octaves = (hi / lo).log2();
            assert!(
                (octaves - 6.0 * depth).abs() < 0.15,
                "depth {depth}: {octaves} octaves"
            );
        }
    }

    #[test]
    fn feedback_deepens_and_sharpens_the_notches() {
        for stages in [4, 8, 12] {
            let notch = notch_freqs(stages, 1000.0, FS)[0];
            // The peak level, and the peak over the level just beside the first notch.
            let mut peaks = Vec::new();
            let mut contrast = Vec::new();
            for feedback in [0.0, 0.5, 0.9] {
                let p = PhaserParams {
                    feedback,
                    ..static_params(stages)
                };
                let h = impulse_response(FS, &p, 65536);
                // The notch stays at its frequency and stays fully deep.
                assert!(magnitude(&h, notch, FS) < 0.01, "{stages} stages");
                let peak = gain_curve(&h, FS, 2000).into_iter().fold(0.0, f32::max);
                peaks.push(peak);
                contrast.push(peak / magnitude(&h, notch * 1.05, FS));
            }
            assert!(peaks[0] < peaks[1] && peaks[1] < peaks[2], "{peaks:?}");
            assert!(peaks[2] > 1.5 * peaks[0], "{stages} stages: {peaks:?}");
            assert!(
                contrast[0] < contrast[1] && contrast[1] < contrast[2],
                "{stages} stages: {contrast:?}"
            );
        }
    }

    #[test]
    fn feedback_at_the_limit_stays_bounded_for_every_stage_count() {
        for stages in [4, 6, 8, 12] {
            for rate in [0.05, 5.0, 200.0] {
                let mut fx = Phaser::new(FS);
                let mut rng = Rng::new(99);
                let p = PhaserParams {
                    rate_hz: rate,
                    depth: 1.0,
                    feedback: 0.95,
                    stages,
                    spread: 1.0,
                    mix: 1.0,
                    ..PhaserParams::default()
                };
                let mut peak = 0.0f32;
                for _ in 0..(FS as usize * 10) {
                    let out = fx.process(noise(&mut rng, 1.0), &p);
                    assert!(out.l.is_finite() && out.r.is_finite());
                    peak = peak.max(out.peak());
                }
                assert!(peak < 15.0, "{stages} stages, rate {rate}: peak {peak}");
            }
        }
    }

    #[test]
    fn spread_zero_gives_identical_sides_and_one_offsets_them() {
        let p = PhaserParams {
            rate_hz: 2.0,
            depth: 0.5,
            ..static_params(4)
        };
        let mut fx = Phaser::new(FS);
        let mut rng = Rng::new(5);
        for _ in 0..20000 {
            let out = fx.process(Frame::mono(rng.signed()), &p);
            assert_eq!(out.l, out.r);
            assert_eq!(fx.coef[0], fx.coef[1]);
        }

        // At full spread the sides sit half a cycle apart, so their sweeps mirror each other
        // in octaves around the centre.
        let p = PhaserParams { spread: 1.0, ..p };
        let mut fx = Phaser::new(FS);
        let mut apart = 0;
        for n in 0..44100 {
            fx.process(Frame::SILENT, &p);
            if n > 100 {
                let (l, r) = (freq_of(fx.coef[0], FS), freq_of(fx.coef[1], FS));
                assert!((l * r / 1.0e6 - 1.0).abs() < 0.05, "{l} {r}");
                if (l / r - 1.0).abs() > 0.1 {
                    apart += 1;
                }
            }
        }
        assert!(apart > 30000, "{apart}");

        // A half spread leaves the sides a quarter cycle apart, which is less than opposite.
        let p = PhaserParams { spread: 0.5, ..p };
        let mut fx = Phaser::new(FS);
        let mut max_gap = 0.0f32;
        for _ in 0..44100 {
            fx.process(Frame::SILENT, &p);
            max_gap = max_gap.max(
                (freq_of(fx.coef[0], FS) / freq_of(fx.coef[1], FS))
                    .log2()
                    .abs(),
            );
        }
        assert!(max_gap > 1.0 && max_gap < 4.5, "{max_gap}");
    }

    #[test]
    fn switching_stages_during_playback_does_not_click() {
        let p0 = PhaserParams {
            rate_hz: 0.5,
            depth: 0.4,
            feedback: 0.3,
            mix: 0.8,
            ..PhaserParams::default()
        };
        let sine = |n: usize| 0.5 * (2.0 * PI * 500.0 * n as f32 / FS).sin();
        let max_step = |sequence: &[usize]| {
            let mut fx = Phaser::new(FS);
            let mut last = 0.0f32;
            let mut worst = 0.0f32;
            for n in 0..(FS as usize) {
                let stages = sequence[(n / 4000).min(sequence.len() - 1)];
                let out = fx.process(Frame::mono(sine(n)), &PhaserParams { stages, ..p0 });
                if n > 2000 {
                    worst = worst.max((out.l - last).abs());
                }
                last = out.l;
            }
            worst
        };
        let steady = max_step(&[4]).max(max_step(&[12]));
        for sequence in [
            [4, 12, 4, 6, 8, 12, 4, 8, 12, 6, 4],
            [12, 4, 12, 4, 12, 4, 12, 4, 12, 4, 12],
        ] {
            let switched = max_step(&sequence);
            assert!(
                switched < 1.3 * steady,
                "switched {switched}, steady {steady}"
            );
        }
    }

    #[test]
    fn changing_rate_depth_and_centre_does_not_zipper() {
        let sine = |n: usize| 0.5 * (2.0 * PI * 300.0 * n as f32 / FS).sin();
        let run = |jump: bool| {
            let mut fx = Phaser::new(FS);
            let (mut last, mut worst) = (0.0f32, 0.0f32);
            for n in 0..FS as usize {
                let late = jump && n > 20000;
                let p = PhaserParams {
                    rate_hz: if late { 40.0 } else { 0.5 },
                    depth: if late { 1.0 } else { 0.2 },
                    center_hz: if late { 4000.0 } else { 300.0 },
                    feedback: 0.5,
                    mix: 1.0,
                    ..PhaserParams::default()
                };
                let out = fx.process(Frame::mono(sine(n)), &p).l;
                if n > 2000 {
                    worst = worst.max((out - last).abs());
                }
                last = out;
            }
            worst
        };
        let steady = run(false);
        let jumped = run(true);
        assert!(jumped < 2.5 * steady, "jumped {jumped}, steady {steady}");
    }

    #[test]
    fn extreme_rates_stay_stable() {
        for fs in [22050.0, 44100.0, 192000.0] {
            let mut fx = Phaser::new(fs);
            let mut rng = Rng::new(21);
            let p = PhaserParams {
                rate_hz: 200.0,
                depth: 1.0,
                feedback: 0.95,
                center_hz: 8000.0,
                stages: 12,
                spread: 1.0,
                mix: 1.0,
            };
            for _ in 0..(fs as usize * 3) {
                let out = fx.process(noise(&mut rng, 1.0), &p);
                assert!(out.l.is_finite() && out.r.is_finite());
                assert!(out.peak() < 15.0, "{fs}: {}", out.peak());
            }
        }
    }

    #[test]
    fn the_allpass_frequency_stays_under_the_limit_at_every_rate() {
        for fs in [22050.0, 44100.0, 96000.0, 192000.0] {
            let p = PhaserParams {
                rate_hz: 7.0,
                depth: 1.0,
                center_hz: 8000.0,
                ..static_params(12)
            };
            let freqs = sweep(fs, &p, 1.0);
            let hi = freqs.iter().cloned().fold(0.0, f32::max);
            assert!(hi <= 0.451 * fs, "{fs}: {hi}");
            assert!(hi > 0.1 * fs, "{fs}: {hi}");
            assert!(freqs.iter().all(|f| f.is_finite() && *f > 5.0));
        }
    }

    #[test]
    fn the_effect_works_at_every_sample_rate() {
        for fs in [22050.0, 44100.0, 96000.0, 192000.0] {
            let mut fx = Phaser::new(fs);
            let p = PhaserParams::default();
            let mut energy = 0.0f64;
            for n in 0..(fs as usize) {
                let x = 0.5 * (2.0 * PI * 440.0 * n as f32 / fs).sin();
                let out = fx.process(Frame::mono(x), &p);
                assert!(out.l.is_finite());
                energy += (out.l * out.l) as f64;
            }
            assert!(energy > 0.05 * fs as f64 * 0.125, "{fs}: {energy}");
        }
    }

    #[test]
    fn output_is_finite_for_loud_input_and_every_param_extreme() {
        let mut rng = Rng::new(1234);
        for rate in [0.05, 200.0] {
            for depth in [0.0, 1.0] {
                for feedback in [0.0, 0.95] {
                    for center in [100.0, 8000.0] {
                        for stages in [4, 6, 8, 12] {
                            for spread in [0.0, 1.0] {
                                for mix in [0.0, 0.5, 1.0] {
                                    let p = PhaserParams {
                                        rate_hz: rate,
                                        depth,
                                        feedback,
                                        center_hz: center,
                                        stages,
                                        spread,
                                        mix,
                                    };
                                    let mut fx = Phaser::new(22050.0);
                                    for n in 0..1500 {
                                        let x = if n % 50 < 25 { 8.0 } else { -8.0 };
                                        let x = Frame::new(x, rng.signed() * 8.0);
                                        let out = fx.process(x, &p);
                                        assert!(out.l.is_finite() && out.r.is_finite());
                                        assert!(out.peak() < 200.0, "{p:?}: {}", out.peak());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn params_that_jump_between_calls_stay_finite() {
        let mut fx = Phaser::new(FS);
        let mut rng = Rng::new(8);
        for n in 0..50000 {
            let p = PhaserParams {
                rate_hz: 0.05 + rng.unit() * 199.95,
                depth: rng.unit(),
                feedback: rng.unit() * 0.95,
                center_hz: 100.0 + rng.unit() * 7900.0,
                stages: [4, 6, 8, 12][(rng.unit() * 3.99) as usize],
                spread: rng.unit(),
                mix: rng.unit(),
            };
            let out = fx.process(noise(&mut rng, 8.0), &p);
            assert!(out.l.is_finite() && out.r.is_finite(), "sample {n}");
        }
    }

    #[test]
    fn reset_clears_the_tail() {
        let mut fx = Phaser::new(FS);
        let mut rng = Rng::new(2);
        let p = PhaserParams {
            feedback: 0.95,
            mix: 1.0,
            ..PhaserParams::default()
        };
        for _ in 0..5000 {
            fx.process(noise(&mut rng, 1.0), &p);
        }
        fx.reset();
        for _ in 0..1000 {
            assert_eq!(fx.process(Frame::SILENT, &p), Frame::SILENT);
        }
    }

    #[test]
    fn the_tail_decays_to_silence_without_denormals() {
        let mut fx = Phaser::new(FS);
        let mut rng = Rng::new(4);
        let p = PhaserParams {
            feedback: 0.95,
            mix: 1.0,
            ..PhaserParams::default()
        };
        for _ in 0..5000 {
            fx.process(noise(&mut rng, 1.0), &p);
        }
        let mut last = Frame::new(1.0, 1.0);
        for _ in 0..(FS as usize * 20) {
            last = fx.process(Frame::SILENT, &p);
            assert!(last.l == 0.0 || last.l.abs() >= f32::MIN_POSITIVE);
        }
        assert_eq!(last, Frame::SILENT);
    }
}
