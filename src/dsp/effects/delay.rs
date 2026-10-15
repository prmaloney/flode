//! Delay: a stereo delay with feedback, damping in the feedback path, and a ping pong mode that
//! bounces the repeats between the left and right.
//!
//! Each side has its own ring buffer, long enough for the 2 second maximum. The read point chases
//! the requested time through a one-pole slew, and a cubic Hermite read follows it, so turning the
//! time knob bends the pitch of the repeats like a tape delay and never jumps. The feedback path
//! has a one-pole low pass for damping. Feedback, mix, and the ping pong routing change through
//! short ramps, so no control click can reach the output.

use crate::dsp::frame::Frame;

/// The longest delay time the plugin asks for, in ms.
const MAX_TIME_MS: f32 = 2000.0;
/// The time constant of the delay time slew, in seconds. The glide is over in about five times it.
const TIME_GLIDE_S: f32 = 0.06;
/// The time constant of the damping coefficient slew, in seconds.
const DAMP_GLIDE_S: f32 = 0.003;
/// The length of the linear ramps on mix and feedback, in seconds.
const LEVEL_RAMP_S: f32 = 0.005;
/// The length of the linear ramp between normal and ping pong routing, in seconds.
const ROUTE_RAMP_S: f32 = 0.02;
/// The damping cutoff at `damp` 0.0 (the filter is bypassed there) and at 1.0, in Hz.
const DAMP_BRIGHT_HZ: f32 = 16000.0;
const DAMP_DARK_HZ: f32 = 500.0;
/// Extra samples in each line beyond the longest delay, for the interpolator's reach.
const MARGIN: usize = 8;
/// The shortest read distance in samples. The cubic read needs one newer neighbour.
const MIN_DELAY: f32 = 2.0;

/// The delay's controls, in plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DelayParams {
    /// The time between repeats in ms, from 1.0 to 2000.0. The caller has already turned a synced
    /// note division into milliseconds, so this is always the time to use.
    pub time_ms: f32,
    /// How much of the output goes back in, from 0.0 to 0.95.
    pub feedback: f32,
    /// How much high end each repeat loses, from 0.0 (bright) to 1.0 (dark).
    pub damp: f32,
    /// Dry/wet, from 0.0 (all dry) to 1.0 (all wet).
    pub mix: f32,
    /// True to send each repeat to the opposite side from the last one.
    pub ping_pong: bool,
}

impl Default for DelayParams {
    fn default() -> Self {
        Self {
            time_ms: 350.0,
            feedback: 0.4,
            damp: 0.5,
            mix: 0.3,
            ping_pong: false,
        }
    }
}

/// Sets values under about 1e-20 to zero, so a decaying tail never reaches denormals.
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 {
        0.0
    } else {
        x
    }
}

/// Moves `current` toward `target` by at most `step`, and lands on `target` exactly.
fn approach(current: f32, target: f32, step: f32) -> f32 {
    let gap = target - current;
    if gap.abs() <= step {
        target
    } else {
        current + step.copysign(gap)
    }
}

/// The one-pole coefficient for a cutoff in Hz. A `damp` of 0.0 returns 1.0, which passes the
/// signal untouched.
fn damp_coefficient(damp: f32, sample_rate: f32) -> f32 {
    if damp <= 0.0 {
        return 1.0;
    }
    let cutoff = DAMP_BRIGHT_HZ * (DAMP_DARK_HZ / DAMP_BRIGHT_HZ).powf(damp);
    let cutoff = cutoff.min(0.45 * sample_rate);
    1.0 - (-std::f32::consts::TAU * cutoff / sample_rate).exp()
}

/// The four-point cubic Hermite read. `t` runs from `x0` toward `x1`.
fn hermite(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * t + c2) * t + c1) * t + x0
}

/// One side's ring buffer.
struct Line {
    buf: Vec<f32>,
}

impl Line {
    /// Reads the sample written `back` calls ago (1 is the latest) at write position `w`.
    fn at(&self, w: usize, back: usize) -> f32 {
        let n = self.buf.len();
        let i = w + n - back;
        self.buf[if i >= n { i - n } else { i }]
    }

    /// Reads at a fractional distance `d` in samples, between 2 and the line length minus the
    /// margin.
    fn read(&self, w: usize, d: f32) -> f32 {
        let whole = d as usize;
        let frac = d - whole as f32;
        hermite(
            self.at(w, whole - 1),
            self.at(w, whole),
            self.at(w, whole + 1),
            self.at(w, whole + 2),
            frac,
        )
    }
}

/// The stereo delay.
pub struct Delay {
    sample_rate: f32,
    left: Line,
    right: Line,
    /// The next write position, shared by both lines.
    write: usize,
    /// The longest read distance in samples.
    max_delay: f32,
    /// The current read distance in samples, chasing the requested time.
    delay: f32,
    glide: f32,
    damp_glide: f32,
    level_step: f32,
    route_step: f32,
    /// The damp value the target coefficient was computed for.
    damp_key: f32,
    damp_target: f32,
    damp_coef: f32,
    lp_l: f32,
    lp_r: f32,
    feedback: f32,
    mix: f32,
    /// 0.0 for normal routing, 1.0 for ping pong.
    route: f32,
    /// False until the first sample, which sets the smoothed values straight to their targets.
    primed: bool,
}

impl Delay {
    /// Builds the delay with two lines of two seconds plus a small margin at `sample_rate`.
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(1.0);
        let max_delay = (MAX_TIME_MS * 0.001 * sr).ceil();
        let len = max_delay as usize + MARGIN;
        let one_pole = |seconds: f32| 1.0 - (-1.0 / (seconds * sr)).exp();
        Self {
            sample_rate: sr,
            left: Line {
                buf: vec![0.0; len],
            },
            right: Line {
                buf: vec![0.0; len],
            },
            write: 0,
            max_delay,
            delay: MIN_DELAY,
            glide: one_pole(TIME_GLIDE_S),
            damp_glide: one_pole(DAMP_GLIDE_S),
            level_step: 1.0 / (LEVEL_RAMP_S * sr),
            route_step: 1.0 / (ROUTE_RAMP_S * sr),
            damp_key: 0.0,
            damp_target: 1.0,
            damp_coef: 1.0,
            lp_l: 0.0,
            lp_r: 0.0,
            feedback: 0.0,
            mix: 0.0,
            route: 0.0,
            primed: false,
        }
    }

    /// Clears the tail and the filter memory. The next sample takes its parameters without a ramp.
    pub fn reset(&mut self) {
        self.left.buf.fill(0.0);
        self.right.buf.fill(0.0);
        self.write = 0;
        self.lp_l = 0.0;
        self.lp_r = 0.0;
        self.primed = false;
    }

    /// Runs one stereo sample.
    ///
    /// The output is `dry * (1 - mix) + wet * mix`. At mix 0.0 the dry sample passes through
    /// unchanged. In normal mode each side feeds back into itself. In ping pong mode the left line
    /// takes the mono sum of the input, the right line takes only the feedback of the left, and
    /// each line feeds the other, so repeats alternate left, right, left. Switching fades between
    /// the two routings over 20 ms.
    pub fn process(&mut self, input: Frame, params: &DelayParams) -> Frame {
        // `max` then `min` also turns a NaN parameter into the low end of its range.
        let time = params.time_ms.max(1.0).min(MAX_TIME_MS);
        let feedback = params.feedback.max(0.0).min(0.95);
        let damp = params.damp.max(0.0).min(1.0);
        let mix = params.mix.max(0.0).min(1.0);
        let route = if params.ping_pong { 1.0 } else { 0.0 };
        let dry = input.map(|x| if x.is_finite() { x } else { 0.0 });

        let target_delay = (time * 0.001 * self.sample_rate)
            .max(MIN_DELAY)
            .min(self.max_delay);
        if damp != self.damp_key || !self.primed {
            self.damp_key = damp;
            self.damp_target = damp_coefficient(damp, self.sample_rate);
        }
        if self.primed {
            self.delay += (target_delay - self.delay) * self.glide;
            self.damp_coef += (self.damp_target - self.damp_coef) * self.damp_glide;
            self.feedback = approach(self.feedback, feedback, self.level_step);
            self.mix = approach(self.mix, mix, self.level_step);
            self.route = approach(self.route, route, self.route_step);
        } else {
            self.delay = target_delay;
            self.damp_coef = self.damp_target;
            self.feedback = feedback;
            self.mix = mix;
            self.route = route;
            self.primed = true;
        }

        let wet_l = self.left.read(self.write, self.delay);
        let wet_r = self.right.read(self.write, self.delay);

        self.lp_l = flush(self.lp_l + self.damp_coef * (wet_l - self.lp_l));
        self.lp_r = flush(self.lp_r + self.damp_coef * (wet_r - self.lp_r));
        let fb_l = self.lp_l * self.feedback;
        let fb_r = self.lp_r * self.feedback;

        let straight = 1.0 - self.route;
        let mid = dry.mid();
        let in_l = dry.l * straight + mid * self.route;
        let in_r = dry.r * straight;
        let to_l = fb_l * straight + fb_r * self.route;
        let to_r = fb_r * straight + fb_l * self.route;
        self.left.buf[self.write] = flush(in_l + to_l);
        self.right.buf[self.write] = flush(in_r + to_r);
        self.write += 1;
        if self.write == self.left.buf.len() {
            self.write = 0;
        }

        let dry_gain = 1.0 - self.mix;
        Frame::new(
            dry.l * dry_gain + wet_l * self.mix,
            dry.r * dry_gain + wet_r * self.mix,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    fn params(time_ms: f32, feedback: f32, damp: f32, mix: f32, ping_pong: bool) -> DelayParams {
        DelayParams {
            time_ms,
            feedback,
            damp,
            mix,
            ping_pong,
        }
    }

    /// Runs `input` through a fresh delay and collects the output.
    fn run(sr: f32, p: &DelayParams, input: &[Frame]) -> Vec<Frame> {
        let mut d = Delay::new(sr);
        input.iter().map(|&f| d.process(f, p)).collect()
    }

    fn impulse(len: usize, frame: Frame) -> Vec<Frame> {
        let mut v = vec![Frame::SILENT; len];
        v[0] = frame;
        v
    }

    /// A deterministic noise source in [-1, 1].
    struct Noise(u32);
    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
            (self.0 >> 8) as f32 / (1u32 << 23) as f32 - 1.0
        }
    }

    fn sine(freq: f32, amp: f32, n: usize) -> f32 {
        amp * (std::f32::consts::TAU * freq * n as f32 / SR).sin()
    }

    /// The amplitude of one frequency in `x`, by a single-bin DFT.
    fn tone_amp(x: &[f32], freq: f32) -> f32 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (n, &s) in x.iter().enumerate() {
            let ph = std::f64::consts::TAU * freq as f64 * n as f64 / SR as f64;
            re += s as f64 * ph.cos();
            im += s as f64 * ph.sin();
        }
        (2.0 * (re * re + im * im).sqrt() / x.len() as f64) as f32
    }

    #[test]
    fn an_impulse_repeats_at_the_time_scaled_by_feedback() {
        let p = params(100.0, 0.5, 0.0, 1.0, false);
        let out = run(SR, &p, &impulse(4800 * 5, Frame::new(1.0, -0.5)));
        let step = 4800;
        assert!(out[..step]
            .iter()
            .all(|f| f.l.abs() < 1e-6 && f.r.abs() < 1e-6));
        for k in 1..5 {
            let gain = 0.5f32.powi(k as i32 - 1);
            let f = out[step * k];
            assert!((f.l - gain).abs() < 1e-5, "repeat {k} left {}", f.l);
            assert!((f.r + 0.5 * gain).abs() < 1e-5, "repeat {k} right {}", f.r);
            let between = &out[step * k + 1..step * (k + 1)];
            assert!(between.iter().all(|f| f.l.abs() < 1e-5), "repeat {k} gap");
        }
    }

    #[test]
    fn a_fractional_time_lands_between_samples() {
        // 100.5 ms at 48 kHz is 4824 samples.
        let p = params(100.5, 0.0, 0.0, 1.0, false);
        let out = run(SR, &p, &impulse(6000, Frame::mono(1.0)));
        let (peak, _) =
            out.iter().enumerate().fold(
                (0, 0.0f32),
                |a, (i, f)| if f.l > a.1 { (i, f.l) } else { a },
            );
        assert_eq!(peak, 4824);
        assert!((out[4824].l - 1.0).abs() < 1e-5);
    }

    #[test]
    fn mix_zero_returns_the_dry_input_exactly() {
        let mut n = Noise(7);
        let input: Vec<Frame> = (0..20000)
            .map(|_| Frame::new(n.next() * 8.0, n.next() * 8.0))
            .collect();
        let p = params(120.0, 0.9, 0.3, 0.0, true);
        let out = run(SR, &p, &input);
        assert!(out.iter().zip(&input).all(|(o, i)| o == i));

        // After the mix knob comes down from 0.5, the ramp lands on exact dry.
        let mut d = Delay::new(SR);
        let wet = params(120.0, 0.9, 0.3, 0.5, false);
        for f in &input[..5000] {
            d.process(*f, &wet);
        }
        let dry = params(120.0, 0.9, 0.3, 0.0, false);
        let tail: Vec<Frame> = input[5000..].iter().map(|f| d.process(*f, &dry)).collect();
        assert!(tail[500..].iter().zip(&input[5500..]).all(|(o, i)| o == i));
    }

    #[test]
    fn damping_darkens_each_repeat() {
        // A burst of a low and a high tone, then repeats at 300 ms with feedback 0.6.
        let (low, high) = (200.0, 6000.0);
        let burst = 4800;
        let step = 14400;
        let input: Vec<Frame> = (0..step * 4)
            .map(|n| {
                if n < burst {
                    Frame::mono(sine(low, 0.3, n) + sine(high, 0.3, n))
                } else {
                    Frame::SILENT
                }
            })
            .collect();
        let measure = |damp: f32| {
            let out = run(SR, &params(300.0, 0.6, damp, 1.0, false), &input);
            let band = |k: usize, f: f32| {
                let x: Vec<f32> = out[step * k + 1000..step * k + burst - 1000]
                    .iter()
                    .map(|f| f.l)
                    .collect();
                // The window starts mid-cycle, so compare amplitudes, which ignore phase.
                tone_amp(&x, f)
            };
            (band(3, high) / band(1, high), band(3, low) / band(1, low))
        };
        let (bright_hi, bright_lo) = measure(0.0);
        let (dark_hi, dark_lo) = measure(0.8);
        // Undamped, both bands fall by the same feedback squared, 0.36.
        assert!((bright_hi - 0.36).abs() < 0.03, "{bright_hi}");
        assert!((bright_lo - 0.36).abs() < 0.03, "{bright_lo}");
        // Damped, the low band keeps its level and the high band drops far below it.
        assert!(dark_lo > 0.3, "{dark_lo}");
        assert!(dark_hi < 0.1 * dark_lo, "hi {dark_hi} lo {dark_lo}");
    }

    #[test]
    fn full_feedback_without_damping_stays_bounded() {
        let mut d = Delay::new(SR);
        let p = params(10.0, 0.95, 0.0, 0.7, false);
        let pp = DelayParams {
            ping_pong: true,
            ..p
        };
        let mut n = Noise(99);
        let mut peak = 0.0f32;
        for i in 0..SR as usize * 20 {
            let x = if i < SR as usize * 5 {
                Frame::new(n.next() * 8.0, n.next() * 8.0)
            } else {
                Frame::SILENT
            };
            let o = d.process(x, if i % 100_000 < 50_000 { &p } else { &pp });
            assert!(o.l.is_finite() && o.r.is_finite());
            peak = peak.max(o.peak());
        }
        // The loop gain is 0.95, so a steady input of 8 settles near 160 at the very most.
        assert!(peak < 200.0, "{peak}");
    }

    #[test]
    fn the_tail_decays_to_silence() {
        let p = params(10.0, 0.95, 0.0, 1.0, false);
        let out = run(SR, &p, &impulse(SR as usize * 12, Frame::mono(1.0)));
        assert!(out[SR as usize * 11..]
            .iter()
            .all(|f| f.l == 0.0 && f.r == 0.0));
    }

    #[test]
    fn ping_pong_alternates_sides() {
        let p = params(100.0, 0.8, 0.0, 1.0, true);
        let out = run(SR, &p, &impulse(4800 * 5, Frame::mono(1.0)));
        let step = 4800;
        assert!(out[..step].iter().all(|f| f.l == 0.0 && f.r == 0.0));
        for k in 1..5 {
            let gain = 0.8f32.powi(k as i32 - 1);
            let (loud, quiet) = if k % 2 == 1 {
                (out[step * k].l, out[step * k].r)
            } else {
                (out[step * k].r, out[step * k].l)
            };
            assert!((loud - gain).abs() < 1e-5, "repeat {k}: {loud}");
            assert!(quiet.abs() < 1e-6, "repeat {k} other side: {quiet}");
        }
    }

    #[test]
    fn normal_mode_keeps_the_sides_independent() {
        let p = params(80.0, 0.7, 0.2, 1.0, false);
        let out = run(SR, &p, &impulse(SR as usize, Frame::new(1.0, 0.0)));
        assert!(out.iter().all(|f| f.r == 0.0));
        assert!(out.iter().skip(3840).any(|f| f.l.abs() > 0.1));

        let out = run(SR, &p, &impulse(SR as usize, Frame::new(0.0, 1.0)));
        assert!(out.iter().all(|f| f.l == 0.0));
        assert!(out.iter().skip(3840).any(|f| f.r.abs() > 0.1));
    }

    /// The count of rising zero crossings per second in `x`.
    fn crossing_freq(x: &[f32]) -> f32 {
        let n = x.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        n as f32 * SR / x.len() as f32
    }

    #[test]
    fn a_time_change_glides_the_pitch_without_clicks() {
        let freq = 220.0;
        for (from, to) in [(100.0, 80.0), (80.0, 100.0)] {
            let mut d = Delay::new(SR);
            let mut out = Vec::new();
            let change_at = SR as usize;
            for n in 0..change_at * 3 {
                let t = if n < change_at { from } else { to };
                let o = d.process(
                    Frame::mono(sine(freq, 0.5, n)),
                    &params(t, 0.0, 0.0, 1.0, false),
                );
                out.push(o.l);
            }
            // The sine's own slope is 0.5 * w. The read point moves at most 0.35 samples per
            // sample, so the output slope stays under 1.4 times that.
            let w = std::f32::consts::TAU * freq / SR;
            let bound = 0.5 * w * 1.4;
            let worst = out[change_at - 2000..]
                .windows(2)
                .map(|p| (p[1] - p[0]).abs())
                .fold(0.0f32, f32::max);
            assert!(worst < bound, "{from}->{to}: {worst} vs {bound}");

            // Shortening the delay reads faster and lifts the pitch. Lengthening drops it.
            let glide = crossing_freq(&out[change_at + 480..change_at + 4800]);
            if to < from {
                assert!(glide > freq * 1.08, "{glide}");
            } else {
                assert!(glide < freq * 0.92, "{glide}");
            }
            // The pitch settles back to the input's.
            let settled = crossing_freq(&out[change_at * 2..]);
            assert!((settled - freq).abs() < freq * 0.02, "{settled}");
        }
    }

    #[test]
    fn switching_ping_pong_does_not_click() {
        for (before, after) in [(false, true), (true, false)] {
            let mut d = Delay::new(SR);
            let freq = 220.0;
            let switch_at = SR as usize * 2;
            let mut out = Vec::new();
            for n in 0..switch_at * 2 {
                let on = if n < switch_at { before } else { after };
                let o = d.process(
                    Frame::new(sine(freq, 0.5, n), sine(freq * 1.5, 0.3, n)),
                    &params(50.0, 0.6, 0.2, 0.5, on),
                );
                out.push(o);
            }
            let region = &out[switch_at - 4000..];
            let peak = region.iter().map(|f| f.peak()).fold(0.0f32, f32::max);
            // A mix of 220 and 330 Hz tones moves no faster than the sum of their slopes.
            let w = std::f32::consts::TAU * 330.0 / SR;
            let bound = peak * w * 1.2 + 0.005;
            for side in [0, 1] {
                let worst = region
                    .windows(2)
                    .map(|p| {
                        if side == 0 {
                            p[1].l - p[0].l
                        } else {
                            p[1].r - p[0].r
                        }
                    })
                    .fold(0.0f32, |a, x| a.max(x.abs()));
                assert!(
                    worst < bound,
                    "{before}->{after} side {side}: {worst} vs {bound}"
                );
            }
        }
    }

    #[test]
    fn the_longest_time_works_at_the_extreme_sample_rates() {
        for sr in [22050.0f32, 48000.0, 192000.0] {
            let step = (2.0 * sr) as usize;
            let p = params(2000.0, 0.5, 0.0, 1.0, false);
            let out = run(sr, &p, &impulse(step * 2 + 100, Frame::mono(1.0)));
            assert!((out[step].l - 1.0).abs() < 1e-4, "{sr}: {}", out[step].l);
            assert!(
                (out[step * 2].l - 0.5).abs() < 1e-4,
                "{sr}: {}",
                out[step * 2].l
            );
            assert!(out[..step].iter().all(|f| f.l.abs() < 1e-6));
        }
    }

    #[test]
    fn a_line_holds_two_seconds_without_waste() {
        for sr in [22050.0f32, 44100.0, 192000.0] {
            let d = Delay::new(sr);
            let samples = (2.0 * sr) as usize;
            assert!(d.left.buf.len() >= samples + 4);
            assert!(d.left.buf.len() <= samples + 16);
        }
    }

    #[test]
    fn output_stays_finite_at_the_extremes() {
        let times = [1.0, 2000.0];
        let feedbacks = [0.0, 0.95];
        let damps = [0.0, 1.0];
        let mixes = [0.0, 1.0];
        for sr in [22050.0f32, 192000.0] {
            let mut d = Delay::new(sr);
            let mut n = Noise(3);
            let mut i = 0usize;
            for &t in &times {
                for &fb in &feedbacks {
                    for &dp in &damps {
                        for &m in &mixes {
                            for pp in [false, true] {
                                let p = params(t, fb, dp, m, pp);
                                for _ in 0..(sr as usize / 20) {
                                    let x = Frame::new(n.next() * 8.0, n.next() * 8.0);
                                    let o = d.process(x, &p);
                                    assert!(o.l.is_finite() && o.r.is_finite(), "{sr} {p:?}");
                                    assert!(o.peak() < 400.0, "{sr} {p:?} {}", o.peak());
                                    i += 1;
                                }
                            }
                        }
                    }
                }
            }
            assert!(i > 0);
        }
    }

    #[test]
    fn reset_clears_the_tail() {
        let p = params(100.0, 0.9, 0.2, 1.0, true);
        let mut d = Delay::new(SR);
        for n in 0..20000 {
            d.process(Frame::mono(sine(300.0, 1.0, n)), &p);
        }
        d.reset();
        for _ in 0..20000 {
            assert_eq!(d.process(Frame::SILENT, &p), Frame::SILENT);
        }
    }
}
