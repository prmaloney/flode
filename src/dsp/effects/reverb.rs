//! Reverb: a stereo algorithmic reverb with a decay time, a room size, damping, and a dry/wet
//! mix.
//!
//! The design is a feedback delay network of eight lines. Each side of the input passes through
//! four all-pass filters to spread single hits, then enters the lines through orthogonal sign
//! patterns. The lines feed back through a Householder matrix, which keeps the loop lossless
//! before the gains are applied. Every line has its own gain, set so all lines lose 60 dB in the
//! same time, which is the `decay_s` knob. A one-pole low pass in each line makes the high end
//! die faster than the low end. The read point of each line drifts slowly, which breaks up the
//! fixed resonances that make a small FDN ring. The two outputs tap the lines with different
//! sign patterns, so a mono input gives a decorrelated stereo tail.

use crate::dsp::frame::Frame;

/// The reverb's controls, in plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReverbParams {
    /// How long the tail takes to fall by 60 dB, in seconds, from 0.1 to 20.0.
    pub decay_s: f32,
    /// The size of the room, from 0.0 (small) to 1.0 (large). It scales the delay lengths.
    pub size: f32,
    /// How much high end the tail loses on each pass, from 0.0 (bright) to 1.0 (dark).
    pub damp: f32,
    /// Dry/wet, from 0.0 (all dry) to 1.0 (all wet).
    pub mix: f32,
}

impl Default for ReverbParams {
    fn default() -> Self {
        Self {
            decay_s: 2.0,
            size: 0.5,
            damp: 0.5,
            mix: 0.3,
        }
    }
}

/// Lines in the feedback network.
const LINES: usize = 8;
/// Delay of each line in milliseconds at a size scale of 1.0. The values are spread over a
/// roughly geometric range and share no simple ratios, so the echoes do not line up.
const LINE_MS: [f32; LINES] = [21.3, 26.9, 32.3, 38.1, 43.7, 49.9, 56.3, 62.9];
/// The size scale at `size` 0.0 and the factor it grows by across the knob. Size 0.5 gives 1.0.
const MIN_SCALE: f32 = 0.5;
const SCALE_RANGE: f32 = 4.0;
/// The largest scale, at `size` 1.0.
const MAX_SCALE: f32 = MIN_SCALE * SCALE_RANGE;
/// Peak swing of the read-point drift, in milliseconds.
const MOD_MS: f32 = 0.3;
/// Rates of the drift in hertz, one per line.
const MOD_HZ: [f32; LINES] = [0.11, 0.17, 0.23, 0.29, 0.13, 0.19, 0.31, 0.37];
/// Input all-pass delays in milliseconds for the left and right sides.
const DIFFUSER_MS: [[f32; 4]; 2] = [[5.1, 3.7, 2.6, 1.9], [5.7, 4.1, 2.9, 2.1]];
/// Feedback coefficient of the input all-passes.
const DIFFUSER_GAIN: f32 = 0.6;
/// Samples between recalculations of the loop gains and the damping coefficient.
const CONTROL_PERIOD: u32 = 16;
/// Values below this magnitude are flushed to zero so a tail cannot turn into denormals.
const FLUSH: f32 = 1e-20;
/// Ratio between the wet output level and the loop's own gain, tuned so noise in gives noise of
/// about the same level out.
const WET_TRIM: f32 = 4.0;

/// Smoothing times in seconds.
const SIZE_SMOOTH_S: f32 = 0.2;
const DECAY_SMOOTH_S: f32 = 0.03;
const DAMP_SMOOTH_S: f32 = 0.03;
const MIX_SMOOTH_S: f32 = 0.01;

fn flush(x: f32) -> f32 {
    if x.abs() < FLUSH {
        0.0
    } else {
        x
    }
}

/// The one-pole smoothing coefficient for a time constant at a sample rate.
fn smooth_coef(seconds: f32, sample_rate: f32) -> f32 {
    1.0 - (-1.0 / (seconds * sample_rate)).exp()
}

/// A value if it is a finite number, else a fallback.
fn finite_or(x: f32, fallback: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        fallback
    }
}

/// The sign of row `row` of a Hadamard matrix at column `i`.
fn hadamard(row: usize, i: usize) -> f32 {
    if (row & i).count_ones() % 2 == 0 {
        1.0
    } else {
        -1.0
    }
}

/// A circular buffer with a cubic read at a fractional delay.
struct Line {
    buf: Vec<f32>,
    write: usize,
}

impl Line {
    fn new(capacity: usize) -> Self {
        Self {
            buf: vec![0.0; capacity],
            write: 0,
        }
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.write = 0;
    }

    /// The sample written `k` calls to `push` ago, where 1 is the latest.
    fn tap(&self, k: usize) -> f32 {
        let len = self.buf.len();
        let mut i = self.write + len - k;
        if i >= len {
            i -= len;
        }
        self.buf[i]
    }

    /// Reads at `delay` samples, which must be at least 2 and below the capacity minus 3.
    fn read(&self, delay: f32) -> f32 {
        let whole = delay as usize;
        let frac = delay - whole as f32;
        let t = 1.0 - frac;
        let pm1 = self.tap(whole + 2);
        let p0 = self.tap(whole + 1);
        let p1 = self.tap(whole);
        let p2 = self.tap(whole - 1);
        let c1 = 0.5 * (p1 - pm1);
        let c2 = pm1 - 2.5 * p0 + 2.0 * p1 - 0.5 * p2;
        let c3 = 0.5 * (p2 - pm1) + 1.5 * (p0 - p1);
        ((c3 * t + c2) * t + c1) * t + p0
    }

    fn push(&mut self, x: f32) {
        self.buf[self.write] = x;
        self.write += 1;
        if self.write == self.buf.len() {
            self.write = 0;
        }
    }
}

/// A Schroeder all-pass filter with a fixed whole-sample delay.
struct AllPass {
    buf: Vec<f32>,
    pos: usize,
}

impl AllPass {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            pos: 0,
        }
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.pos = 0;
    }

    fn process(&mut self, x: f32) -> f32 {
        let d = self.buf[self.pos];
        let v = flush(x - DIFFUSER_GAIN * d);
        self.buf[self.pos] = v;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
        d + DIFFUSER_GAIN * v
    }
}

/// A stereo feedback delay network reverb.
pub struct Reverb {
    sample_rate: f32,
    lines: [Line; LINES],
    diffusers: [[AllPass; 4]; 2],
    /// Low-pass state of each line.
    damp_state: [f32; LINES],
    /// Phase of each line's drift, from 0 to 1.
    mod_phase: [f32; LINES],
    /// Per-sample phase step of each drift.
    mod_step: [f32; LINES],
    /// Loop gain of each line, refreshed every `CONTROL_PERIOD` samples.
    gains: [f32; LINES],
    damp_coef: f32,
    wet_gain: f32,
    /// Smoothed controls.
    size_scale: f32,
    inv_decay: f32,
    damp: f32,
    mix: f32,
    size_k: f32,
    decay_k: f32,
    damp_k: f32,
    mix_k: f32,
    /// Peak drift in samples.
    mod_depth: f32,
    /// Samples until the next control update.
    countdown: u32,
    /// False until the first call, which sets the smoothed controls to the first params.
    primed: bool,
}

impl Reverb {
    /// Builds a reverb for `sample_rate`. It allocates every buffer here and nowhere else.
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = if sample_rate.is_finite() {
            sample_rate.clamp(8000.0, 400_000.0)
        } else {
            44_100.0
        };
        let to_samples = |ms: f32| ms * 0.001 * sample_rate;
        let mod_depth = to_samples(MOD_MS);
        let longest = LINE_MS.iter().cloned().fold(0.0, f32::max);
        let capacity = (to_samples(longest * MAX_SCALE) + mod_depth).ceil() as usize + 8;
        let diffuser = |side: usize| {
            std::array::from_fn(|i| AllPass::new(to_samples(DIFFUSER_MS[side][i]) as usize))
        };
        Self {
            sample_rate,
            lines: std::array::from_fn(|_| Line::new(capacity)),
            diffusers: [diffuser(0), diffuser(1)],
            damp_state: [0.0; LINES],
            mod_phase: std::array::from_fn(|i| i as f32 / LINES as f32),
            mod_step: std::array::from_fn(|i| MOD_HZ[i] / sample_rate),
            gains: [0.0; LINES],
            damp_coef: 1.0,
            wet_gain: 0.0,
            size_scale: 1.0,
            inv_decay: 0.5,
            damp: 0.0,
            mix: 0.0,
            size_k: smooth_coef(SIZE_SMOOTH_S, sample_rate),
            decay_k: smooth_coef(DECAY_SMOOTH_S, sample_rate),
            damp_k: smooth_coef(DAMP_SMOOTH_S, sample_rate),
            mix_k: smooth_coef(MIX_SMOOTH_S, sample_rate),
            mod_depth,
            countdown: 0,
            primed: false,
        }
    }

    /// Clears the tail and all filter memory. The next call takes its controls from its params
    /// without gliding.
    pub fn reset(&mut self) {
        for line in &mut self.lines {
            line.clear();
        }
        for side in &mut self.diffusers {
            for ap in side {
                ap.clear();
            }
        }
        self.damp_state = [0.0; LINES];
        self.primed = false;
    }

    /// Recomputes the loop gains, damping coefficient, and wet level from the smoothed controls.
    fn update_control(&mut self) {
        let sr = self.sample_rate;
        // Gain per line so that a pass of `len` samples loses its share of 60 dB.
        let per_sample = -std::f32::consts::LN_10 * 3.0 * self.inv_decay / sr;
        let mut mean_gain = 0.0;
        for (g, ms) in self.gains.iter_mut().zip(LINE_MS) {
            let len = ms * 0.001 * sr * self.size_scale;
            *g = (per_sample * len).exp();
            mean_gain += *g / LINES as f32;
        }
        // A louder loop sums more energy, so the output level follows the loop's own loss.
        self.wet_gain = WET_TRIM * (1.0 - mean_gain * mean_gain).max(0.0).sqrt();
        self.damp_coef = if self.damp < 1e-3 {
            1.0
        } else {
            let hz = 18_000.0 * (1000.0f32 / 18_000.0).powf(self.damp);
            (1.0 - (-std::f32::consts::TAU * hz / sr).exp()).min(1.0)
        };
    }

    /// Runs one stereo sample.
    pub fn process(&mut self, input: Frame, params: &ReverbParams) -> Frame {
        let defaults = ReverbParams::default();
        let decay = finite_or(params.decay_s, defaults.decay_s).clamp(0.05, 60.0);
        let size = finite_or(params.size, defaults.size).clamp(0.0, 1.0);
        let damp = finite_or(params.damp, defaults.damp).clamp(0.0, 1.0);
        let mix = finite_or(params.mix, defaults.mix).clamp(0.0, 1.0);
        let scale_target = MIN_SCALE * SCALE_RANGE.powf(size);

        if !self.primed {
            self.primed = true;
            self.size_scale = scale_target;
            self.inv_decay = 1.0 / decay;
            self.damp = damp;
            self.mix = mix;
            self.countdown = 0;
        } else {
            self.size_scale += (scale_target - self.size_scale) * self.size_k;
            self.inv_decay += (1.0 / decay - self.inv_decay) * self.decay_k;
            self.damp += (damp - self.damp) * self.damp_k;
            self.mix += (mix - self.mix) * self.mix_k;
            if (self.mix - mix).abs() < 1e-5 {
                self.mix = mix;
            }
        }
        if self.countdown == 0 {
            self.update_control();
            self.countdown = CONTROL_PERIOD;
        }
        self.countdown -= 1;

        let dry = input;
        let in_l = finite_or(input.l, 0.0).clamp(-64.0, 64.0);
        let in_r = finite_or(input.r, 0.0).clamp(-64.0, 64.0);

        // Spread the input before it enters the loop.
        let mut diff_l = in_l;
        for ap in &mut self.diffusers[0] {
            diff_l = ap.process(diff_l);
        }
        let mut diff_r = in_r;
        for ap in &mut self.diffusers[1] {
            diff_r = ap.process(diff_r);
        }

        // Read each line, drift its read point, damp it, and apply its loop gain.
        let base = self.sample_rate * 0.001 * self.size_scale;
        let mut taps = [0.0f32; LINES];
        for i in 0..LINES {
            let phase = self.mod_phase[i];
            let x = phase * 2.0 - 1.0;
            let wobble = 4.0 * x * (1.0 - x.abs());
            self.mod_phase[i] = if phase + self.mod_step[i] >= 1.0 {
                phase + self.mod_step[i] - 1.0
            } else {
                phase + self.mod_step[i]
            };
            let delay = base * LINE_MS[i] + wobble * self.mod_depth;
            let raw = self.lines[i].read(delay);
            let state = &mut self.damp_state[i];
            *state = flush(*state + self.damp_coef * (raw - *state));
            taps[i] = *state * self.gains[i];
        }

        // Mix the lines with a Householder reflection and add the input.
        let sum: f32 = taps.iter().sum();
        let reflect = sum * (2.0 / LINES as f32);
        let inject = 0.35;
        let mut wet_l = 0.0;
        let mut wet_r = 0.0;
        for i in 0..LINES {
            let feed = diff_l * hadamard(1, i) + diff_r * hadamard(2, i);
            self.lines[i].push(flush(taps[i] - reflect + feed * inject));
            wet_l += taps[i] * hadamard(3, i);
            wet_r += taps[i] * hadamard(5, i);
        }
        let out_scale = self.wet_gain * (1.0 / (LINES as f32).sqrt());
        let wet = Frame::new(wet_l * out_scale, wet_r * out_scale);

        if self.mix == 0.0 {
            return dry;
        }
        dry * (1.0 - self.mix) + wet * self.mix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44_100.0;

    fn params(decay_s: f32, size: f32, damp: f32, mix: f32) -> ReverbParams {
        ReverbParams {
            decay_s,
            size,
            damp,
            mix,
        }
    }

    /// A small deterministic noise source in [-1, 1].
    struct Noise(u32);

    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (self.0 >> 8) as f32 / (1u32 << 23) as f32 - 1.0
        }
    }

    /// The response to a mono unit impulse, as left and right buffers.
    fn impulse_response(sr: f32, p: &ReverbParams, seconds: f32) -> (Vec<f32>, Vec<f32>) {
        let mut reverb = Reverb::new(sr);
        let n = (seconds * sr) as usize;
        let mut l = Vec::with_capacity(n);
        let mut r = Vec::with_capacity(n);
        for i in 0..n {
            let x = if i == 0 { 1.0 } else { 0.0 };
            let out = reverb.process(Frame::mono(x), p);
            l.push(out.l);
            r.push(out.r);
        }
        (l, r)
    }

    /// Seconds for the backward-integrated energy to fall from -5 dB to `to_db`, scaled to 60 dB.
    fn rt60_of(energy: &[f32], sr: f32, to_db: f32) -> f32 {
        let mut curve = vec![0.0f64; energy.len()];
        let mut acc = 0.0f64;
        for (c, e) in curve.iter_mut().zip(energy).rev() {
            acc += *e as f64;
            *c = acc;
        }
        let total = curve[0];
        let find = |db: f64| {
            let target = total * 10f64.powf(db / 10.0);
            curve
                .iter()
                .position(|&c| c < target)
                .unwrap_or(curve.len()) as f64
        };
        let t1 = find(-5.0);
        let t2 = find(to_db as f64);
        (((t2 - t1) / sr as f64) * (60.0 / (to_db as f64 + 5.0).abs())) as f32
    }

    fn stereo_energy(l: &[f32], r: &[f32]) -> Vec<f32> {
        l.iter().zip(r).map(|(a, b)| a * a + b * b).collect()
    }

    fn measure_rt60(sr: f32, decay: f32) -> f32 {
        let (l, r) = impulse_response(sr, &params(decay, 0.5, 0.0, 1.0), decay * 1.6);
        rt60_of(&stereo_energy(&l, &r), sr, -35.0)
    }

    #[test]
    fn rt60_matches_decay_at_three_times() {
        for decay in [0.5, 2.0, 8.0] {
            let rt = measure_rt60(SR, decay);
            println!("decay {decay}: measured RT60 {rt}");
            assert!(
                (rt - decay).abs() <= decay * 0.25,
                "decay {decay} measured {rt}"
            );
        }
    }

    #[test]
    fn rt60_holds_across_size() {
        for size in [0.0, 1.0] {
            let (l, r) = impulse_response(SR, &params(2.0, size, 0.0, 1.0), 3.2);
            let rt = rt60_of(&stereo_energy(&l, &r), SR, -35.0);
            println!("size {size}: measured RT60 {rt}");
            assert!((rt - 2.0).abs() <= 0.5, "size {size} measured {rt}");
        }
    }

    /// A first-order low pass, for splitting bands in the tests.
    fn low_pass(x: &[f32], hz: f32, sr: f32) -> Vec<f32> {
        let a = 1.0 - (-std::f32::consts::TAU * hz / sr).exp();
        let mut y = 0.0;
        x.iter()
            .map(|&v| {
                y += a * (v - y);
                y
            })
            .collect()
    }

    fn high_band(x: &[f32], sr: f32) -> Vec<f32> {
        let once: Vec<f32> = x
            .iter()
            .zip(low_pass(x, 3000.0, sr))
            .map(|(a, b)| a - b)
            .collect();
        once.iter()
            .zip(low_pass(&once, 3000.0, sr))
            .map(|(a, b)| a - b)
            .collect()
    }

    fn low_band(x: &[f32], sr: f32) -> Vec<f32> {
        low_pass(&low_pass(x, 300.0, sr), 300.0, sr)
    }

    fn band_rt60(p: &ReverbParams) -> (f32, f32) {
        let (l, _) = impulse_response(SR, p, 8.0);
        let sq = |x: Vec<f32>| x.iter().map(|v| v * v).collect::<Vec<f32>>();
        let high = rt60_of(&sq(high_band(&l, SR)), SR, -25.0);
        let low = rt60_of(&sq(low_band(&l, SR)), SR, -25.0);
        (high, low)
    }

    #[test]
    fn damping_makes_the_high_band_decay_faster() {
        let (high, low) = band_rt60(&params(4.0, 0.5, 0.8, 1.0));
        println!("damp 0.8: high {high} s, low {low} s");
        assert!(high < low * 0.6, "high {high} low {low}");
        let (high0, low0) = band_rt60(&params(4.0, 0.5, 0.0, 1.0));
        println!("damp 0.0: high {high0} s, low {low0} s");
        assert!(high0 > low0 * 0.7, "high {high0} low {low0}");
    }

    /// Samples whose magnitude passes a threshold in the first 80 ms after the input diffusion
    /// has finished. A larger room has fewer echoes there.
    fn early_echoes(size: f32) -> usize {
        let (l, r) = impulse_response(SR, &params(3.0, size, 0.0, 1.0), 0.1);
        let start = (0.020 * SR) as usize;
        let end = (0.100 * SR) as usize;
        (start..end)
            .filter(|&i| l[i].abs().max(r[i].abs()) > 1e-3)
            .count()
    }

    #[test]
    fn size_changes_the_echo_spacing() {
        let small = early_echoes(0.0);
        let large = early_echoes(1.0);
        println!("early echoes: small {small}, large {large}");
        assert!(
            small as f32 > large as f32 * 1.5,
            "small {small} large {large}"
        );
    }

    fn correlation(a: &[f32], b: &[f32]) -> f32 {
        let ab: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let aa: f32 = a.iter().map(|x| x * x).sum();
        let bb: f32 = b.iter().map(|x| x * x).sum();
        ab / (aa * bb).sqrt()
    }

    #[test]
    fn a_mono_impulse_gives_a_decorrelated_tail() {
        let (l, r) = impulse_response(SR, &params(2.0, 0.5, 0.3, 1.0), 2.0);
        let start = (0.05 * SR) as usize;
        let c = correlation(&l[start..], &r[start..]);
        println!("tail correlation {c}");
        assert!(c.abs() < 0.2, "correlation {c}");
        let energy: f32 = l.iter().map(|x| x * x).sum();
        assert!(energy > 0.0);
    }

    #[test]
    fn mix_zero_returns_the_dry_input_exactly() {
        let mut reverb = Reverb::new(SR);
        let p = params(5.0, 0.7, 0.2, 0.0);
        let mut noise = Noise(7);
        for _ in 0..20_000 {
            let x = Frame::new(noise.next() * 4.0, noise.next() * 4.0);
            assert_eq!(reverb.process(x, &p), x);
        }
        // After a loud wet section, a mix back at zero settles to exact dry.
        let wet = params(5.0, 0.7, 0.2, 1.0);
        for _ in 0..5000 {
            reverb.process(Frame::new(noise.next(), noise.next()), &wet);
        }
        for _ in 0..20_000 {
            reverb.process(Frame::new(0.3, -0.3), &p);
        }
        let x = Frame::new(0.25, -0.5);
        assert_eq!(reverb.process(x, &p), x);
    }

    #[test]
    fn the_tail_decays_to_true_silence() {
        let mut reverb = Reverb::new(SR);
        let p = params(0.5, 0.5, 0.5, 1.0);
        let mut noise = Noise(3);
        for _ in 0..10_000 {
            reverb.process(Frame::new(noise.next(), noise.next()), &p);
        }
        let mut last = Frame::new(1.0, 1.0);
        for _ in 0..(12.0 * SR) as usize {
            last = reverb.process(Frame::SILENT, &p);
        }
        assert_eq!(last, Frame::SILENT);
    }

    #[test]
    fn it_stays_stable_at_the_longest_decay() {
        let mut reverb = Reverb::new(SR);
        let p = params(20.0, 1.0, 0.0, 1.0);
        let mut noise = Noise(11);
        let mut peak = 0.0f32;
        for _ in 0..(2.0 * SR) as usize {
            let out = reverb.process(Frame::new(noise.next() * 8.0, noise.next() * 8.0), &p);
            assert!(out.l.is_finite() && out.r.is_finite());
            peak = peak.max(out.peak());
        }
        for _ in 0..(40.0 * SR) as usize {
            let out = reverb.process(Frame::SILENT, &p);
            assert!(out.l.is_finite() && out.r.is_finite());
            peak = peak.max(out.peak());
        }
        println!("longest decay peak {peak}");
        assert!(peak < 50.0, "peak {peak}");
    }

    #[test]
    fn changing_size_and_decay_while_playing_does_not_click() {
        let sine = |n: usize| 0.5 * (std::f32::consts::TAU * 220.0 * n as f32 / SR).sin();
        let mut reverb = Reverb::new(SR);
        let mut n = 0usize;
        let mut prev = 0.0f32;
        let mut run = |reverb: &mut Reverb, p: &ReverbParams, secs: f32| {
            let mut worst = 0.0f32;
            let mut peak = 0.0f32;
            for _ in 0..(secs * SR) as usize {
                let out = reverb.process(Frame::mono(sine(n)), p);
                assert!(out.l.is_finite() && out.r.is_finite());
                worst = worst.max((out.l - prev).abs());
                peak = peak.max(out.l.abs());
                prev = out.l;
                n += 1;
            }
            (worst, peak)
        };
        let a = params(1.0, 0.2, 0.3, 1.0);
        let b = params(10.0, 0.9, 0.3, 1.0);
        let (base_step, base_peak) = run(&mut reverb, &a, 1.5);
        let (step, peak) = run(&mut reverb, &b, 1.5);
        let (back_step, back_peak) = run(&mut reverb, &a, 1.5);
        println!("steps {base_step} {step} {back_step}, peaks {base_peak} {peak} {back_peak}");
        // A 220 Hz sine steps by at most 3 percent of its peak per sample. A click would be a
        // step many times that, so allow under three times the sine's own rate.
        assert!(base_step < 0.1 * base_peak, "base step {base_step}");
        assert!(step < 0.1 * peak, "step {step}");
        assert!(back_step < 0.1 * back_peak, "back step {back_step}");
        assert!(peak < 8.0 && back_peak < 8.0);
    }

    #[test]
    fn it_works_at_every_sample_rate() {
        for sr in [22_050.0, 44_100.0, 96_000.0, 192_000.0] {
            let rt = measure_rt60(sr, 2.0);
            println!("{sr} Hz: RT60 {rt}");
            assert!((rt - 2.0).abs() <= 0.5, "{sr} Hz measured {rt}");
        }
    }

    #[test]
    fn memory_stays_small_at_the_highest_rate() {
        let reverb = Reverb::new(192_000.0);
        let bytes: usize = reverb.lines.iter().map(|l| l.buf.len() * 4).sum::<usize>()
            + reverb
                .diffusers
                .iter()
                .flatten()
                .map(|a| a.buf.len() * 4)
                .sum::<usize>();
        println!("{bytes} bytes at 192 kHz");
        assert!(bytes < 1_200_000);
    }

    #[test]
    fn output_stays_finite_for_loud_input_and_extreme_params() {
        let extremes = [0.0f32, 1.0];
        let decays = [0.1f32, 20.0];
        let mut noise = Noise(99);
        for &decay in &decays {
            for &size in &extremes {
                for &damp in &extremes {
                    for &mix in &extremes {
                        let mut reverb = Reverb::new(SR);
                        let p = params(decay, size, damp, mix);
                        for i in 0..20_000 {
                            let x = Frame::new(noise.next() * 8.0, noise.next() * 8.0);
                            let out = reverb.process(x, &p);
                            assert!(out.l.is_finite() && out.r.is_finite(), "{p:?} at {i}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn params_that_jump_every_sample_stay_finite() {
        let mut reverb = Reverb::new(SR);
        let mut noise = Noise(5);
        for i in 0..100_000 {
            let flip = i % 2 == 0;
            let p = params(
                if flip { 0.1 } else { 20.0 },
                if flip { 0.0 } else { 1.0 },
                if flip { 1.0 } else { 0.0 },
                if i % 3 == 0 { 0.0 } else { 1.0 },
            );
            let out = reverb.process(Frame::new(noise.next() * 8.0, noise.next() * 8.0), &p);
            assert!(out.l.is_finite() && out.r.is_finite());
        }
    }

    #[test]
    fn reset_clears_the_tail() {
        let mut reverb = Reverb::new(SR);
        let p = params(5.0, 0.5, 0.2, 1.0);
        reverb.process(Frame::mono(1.0), &p);
        for _ in 0..5000 {
            reverb.process(Frame::SILENT, &p);
        }
        reverb.reset();
        for _ in 0..5000 {
            assert_eq!(reverb.process(Frame::SILENT, &p), Frame::SILENT);
        }
    }

    #[test]
    fn noise_in_gives_a_comparable_level_out() {
        let mut levels = Vec::new();
        for decay in [0.3, 2.0, 10.0] {
            let mut reverb = Reverb::new(SR);
            let p = params(decay, 0.5, 0.3, 1.0);
            let mut noise = Noise(21);
            let (mut sin, mut sout) = (0.0f64, 0.0f64);
            for i in 0..(decay.max(1.0) * 4.0 * SR) as usize {
                let x = noise.next();
                let out = reverb.process(Frame::mono(x), &p);
                if i > (decay.max(1.0) * 2.0 * SR) as usize {
                    sin += (x * x) as f64;
                    sout += ((out.l * out.l + out.r * out.r) * 0.5) as f64;
                }
            }
            let db = 10.0 * (sout / sin).log10();
            println!("decay {decay}: wet level {db} dB");
            levels.push(db);
        }
        println!("levels {levels:?}");
        assert!(levels.iter().all(|db| db.abs() < 6.0), "levels {levels:?}");
    }
}
