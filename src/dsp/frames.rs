//! Single-cycle frames for a custom wavetable. Each function here makes frames of exactly
//! `FRAME_SIZE` samples with values from -1.0 to 1.0, from a drawn curve, a harmonic recipe, a
//! sweep, or a slice of an audio file. Nothing in here knows about nodes or the editor.

// Nothing calls these yet. Remove this once the wavetable nodes use them.
#![allow(dead_code)]

use std::f64::consts::TAU;

use crate::dsp::curve::Curve;
use crate::dsp::sample::SampleData;

/// How many samples one frame holds.
pub const FRAME_SIZE: usize = 2048;

/// The most partials any recipe adds up.
const MAX_PARTIALS: usize = 64;

/// The most frames an import makes.
const MAX_IMPORT_FRAMES: usize = 256;

/// The shortest and longest slice, in samples, for `SliceMode::Fixed`.
const MIN_SLICE: usize = 64;
const MAX_SLICE: usize = 65536;

/// The slice length used when a file has no clear pitch.
const FALLBACK_SLICE: usize = 2048;

/// The lowest and highest pitch the detector looks for, in Hz.
const MIN_PITCH_HZ: f32 = 20.0;
const MAX_PITCH_HZ: f32 = 2000.0;

/// How well a file must repeat itself, from 0.0 to 1.0, to count as having a pitch.
const PITCH_CLARITY: f64 = 0.8;

/// Takes the average out of `frame` so the wave sits on zero.
fn remove_dc(frame: &mut [f32]) {
    if frame.is_empty() {
        return;
    }
    let mean = frame.iter().map(|&v| v as f64).sum::<f64>() / frame.len() as f64;
    for v in frame.iter_mut() {
        *v -= mean as f32;
    }
}

/// The biggest absolute value in `frame`.
fn peak(frame: &[f32]) -> f32 {
    frame.iter().fold(0.0_f32, |m, v| m.max(v.abs()))
}

/// Scales `frame` so its peak is 1.0. A silent frame stays silent.
fn peak_normalize(frame: &mut [f32]) {
    let p = peak(frame);
    if p > 1e-9 {
        for v in frame.iter_mut() {
            *v /= p;
        }
    }
}

/// `frame` with every value forced finite and inside -1.0 to 1.0.
fn sanitize(frame: &mut [f32]) {
    for v in frame.iter_mut() {
        *v = if v.is_finite() {
            v.clamp(-1.0, 1.0)
        } else {
            0.0
        };
    }
}

/// One cycle from a drawn curve, read the way the LFO's Custom wave reads it: x is the place in
/// the cycle from 0.0 to 1.0 and y from 0.0 to 1.0 becomes -1.0 to 1.0. No points, or points that
/// do not make a valid curve, give the default triangle. The average is removed, and the
/// amplitude is left as drawn.
pub fn curve_frame(points: &[[f32; 2]]) -> Vec<f32> {
    let curve = if points.is_empty() {
        Curve::default()
    } else {
        Curve::from_points(points).unwrap_or_default()
    };
    let mut frame: Vec<f32> = (0..FRAME_SIZE)
        .map(|n| curve.eval(n as f32 / FRAME_SIZE as f32) * 2.0 - 1.0)
        .collect();
    remove_dc(&mut frame);
    sanitize(&mut frame);
    frame
}

/// Adds up sine partials. `amp(k)` is the level of partial `k`, counting from 1.
fn additive(count: usize, amp: impl Fn(usize) -> f64) -> Vec<f32> {
    let mut frame = vec![0.0_f64; FRAME_SIZE];
    for k in 1..=count {
        let a = amp(k);
        if a == 0.0 {
            continue;
        }
        for (n, v) in frame.iter_mut().enumerate() {
            *v += a * (TAU * ((k * n) % FRAME_SIZE) as f64 / FRAME_SIZE as f64).sin();
        }
    }
    let mut out: Vec<f32> = frame.into_iter().map(|v| v as f32).collect();
    peak_normalize(&mut out);
    out
}

/// A frame built from sine partials. `count` partials are added, from 1 to 64. Partial `k` has
/// level 1/k^p, where `tilt` from 0.0 to 1.0 makes p run from 0 (every partial equal, buzzy)
/// through 1 (a saw) to 2. `odd_even` from -1.0 to 1.0 is 0 for no change, +1 for no even
/// partials (square-like), and -1 for no odd ones. The peak is 1.0.
pub fn harmonic_frame(count: usize, tilt: f32, odd_even: f32) -> Vec<f32> {
    let count = count.clamp(1, MAX_PARTIALS);
    let power = tilt.clamp(0.0, 1.0) as f64 * 2.0;
    let balance = odd_even.clamp(-1.0, 1.0) as f64;
    additive(count, |k| {
        let side = if k % 2 == 0 {
            1.0 - balance.max(0.0)
        } else {
            1.0 + balance.min(0.0)
        };
        side / (k as f64).powf(power)
    })
}

/// Which way a sweep changes the wave from the first frame to the last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum SweepKind {
    /// A saw that gains partials, from 1 up to 64.
    #[default]
    Harmonics = 0,
    /// A pulse whose width narrows from 50% to 5%.
    Pwm = 1,
    /// A sine hard synced to a faster partner, ratio 1 up to 8.
    Sync = 2,
    /// A sine through a wavefolder, gain 1 up to 8.
    Fold = 3,
}

impl SweepKind {
    /// Every kind, in menu order.
    pub const ALL: [SweepKind; 4] = [
        SweepKind::Harmonics,
        SweepKind::Pwm,
        SweepKind::Sync,
        SweepKind::Fold,
    ];

    /// The menu label of each kind, in the same order as `ALL`.
    pub const LABELS: [&'static str; 4] = ["Harmonics", "PWM", "Sync", "Fold"];

    /// The kind at menu position `i`, or the default when `i` is out of range.
    pub fn from_index(i: usize) -> SweepKind {
        Self::ALL.get(i).copied().unwrap_or_default()
    }

    /// This kind's menu position.
    pub fn index(self) -> usize {
        self as usize
    }
}

/// A saw with `partials` partials, where a fractional part fades the last one in.
fn growing_saw(partials: f64) -> Vec<f32> {
    let whole = partials.floor() as usize;
    let frac = partials - whole as f64;
    additive((whole + 1).min(MAX_PARTIALS), |k| {
        if k <= whole {
            1.0 / k as f64
        } else {
            frac / k as f64
        }
    })
}

/// A band-limited pulse with width `duty` from 0.0 to 1.0, built from up to 64 partials.
fn pulse(duty: f64) -> Vec<f32> {
    let mut out: Vec<f32> = (0..FRAME_SIZE)
        .map(|n| {
            let x = n as f64 / FRAME_SIZE as f64;
            let mut sum = 0.0;
            for k in 1..=MAX_PARTIALS {
                let kf = k as f64;
                sum += (std::f64::consts::PI * kf * duty).sin() / kf
                    * (TAU * kf * (x - duty * 0.5)).cos();
            }
            sum as f32
        })
        .collect();
    remove_dc(&mut out);
    peak_normalize(&mut out);
    out
}

/// A sine whose phase runs `ratio` times faster and restarts each cycle.
fn synced_sine(ratio: f64) -> Vec<f32> {
    let mut out: Vec<f32> = (0..FRAME_SIZE)
        .map(|n| {
            let phase = (n as f64 / FRAME_SIZE as f64 * ratio).fract();
            (TAU * phase).sin() as f32
        })
        .collect();
    remove_dc(&mut out);
    peak_normalize(&mut out);
    out
}

/// A sine through a wavefolder with `gain`.
fn folded_sine(gain: f64) -> Vec<f32> {
    let mut out: Vec<f32> = (0..FRAME_SIZE)
        .map(|n| {
            let x = TAU * n as f64 / FRAME_SIZE as f64;
            (gain * x.sin()).sin() as f32
        })
        .collect();
    remove_dc(&mut out);
    peak_normalize(&mut out);
    out
}

/// `frames` frames (from 2 to 64) that change step by step along a sweep of `kind`. `amount`
/// from 0.0 to 1.0 is how far the sweep goes: 0 makes every frame the first frame, 1 runs the
/// whole range. Each frame's peak is at most 1.0.
pub fn sweep_frames(kind: SweepKind, frames: usize, amount: f32) -> Vec<Vec<f32>> {
    let n = frames.clamp(2, 64);
    let amount = if amount.is_finite() {
        amount.clamp(0.0, 1.0) as f64
    } else {
        0.0
    };
    (0..n)
        .map(|i| {
            let s = amount * i as f64 / (n - 1) as f64;
            let mut frame = match kind {
                SweepKind::Harmonics => growing_saw(1.0 + (MAX_PARTIALS - 1) as f64 * s),
                SweepKind::Pwm => pulse(0.5 - 0.45 * s),
                SweepKind::Sync => synced_sine(1.0 + 7.0 * s),
                SweepKind::Fold => folded_sine(1.0 + 7.0 * s),
            };
            sanitize(&mut frame);
            frame
        })
        .collect()
}

/// How to cut an audio file into frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SliceMode {
    /// Consecutive slices of this many samples, from 64 to 65536.
    Fixed(usize),
    /// One slice per pitch period of the file. A file with no clear pitch uses `Fixed(2048)`.
    Cycles,
}

/// The file mixed down to one channel, with any bad value made silent.
fn mono_mix(data: &SampleData) -> Vec<f32> {
    mono_mix_head(data, data.len())
}

/// The first `count` samples of the file mixed down to one channel.
fn mono_mix_head(data: &SampleData, count: usize) -> Vec<f32> {
    (0..data.len().min(count))
        .map(|n| {
            let (l, r) = data.frame(n);
            let v = (l + r) * 0.5;
            if v.is_finite() {
                v
            } else {
                0.0
            }
        })
        .collect()
}

/// The pitch period of `mono`, in samples, or None if it has no clear pitch. It looks at the first
/// half second for the lag with the strongest self-match between 20 Hz and 2 kHz.
fn detect_period(mono: &[f32], sample_rate: f32) -> Option<f64> {
    if !(sample_rate.is_finite() && sample_rate > 0.0) {
        return None;
    }
    let n = mono.len().min((sample_rate * 0.5) as usize);
    let min_lag = ((sample_rate / MAX_PITCH_HZ).ceil() as usize).max(2);
    let max_lag = ((sample_rate / MIN_PITCH_HZ).floor() as usize).min(n / 2);
    if max_lag < min_lag + 3 {
        return None;
    }
    let mean = mono[..n].iter().map(|&v| v as f64).sum::<f64>() / n as f64;
    let x: Vec<f64> = mono[..n].iter().map(|&v| v as f64 - mean).collect();
    // Running sums of squares, so the energy of any stretch is two lookups.
    let mut energy = vec![0.0_f64; n + 1];
    for i in 0..n {
        energy[i + 1] = energy[i] + x[i] * x[i];
    }
    let mut score = vec![0.0_f64; max_lag + 2];
    for lag in min_lag.saturating_sub(1)..=max_lag + 1 {
        if lag >= n {
            break;
        }
        let len = n - lag;
        let dot: f64 = x[..len].iter().zip(&x[lag..]).map(|(a, b)| a * b).sum();
        let e1 = energy[len];
        let e2 = energy[n] - energy[lag];
        if e1 > 1e-12 && e2 > 1e-12 {
            score[lag] = dot / (e1 * e2).sqrt();
        }
    }
    let best = (min_lag..=max_lag).fold(0.0_f64, |m, lag| m.max(score[lag]));
    if best < PITCH_CLARITY {
        return None;
    }
    // The shortest lag that is a peak nearly as good as the best, so a pitch is not read an
    // octave low.
    let lag = (min_lag..=max_lag).find(|&l| {
        score[l] >= 0.9 * best && score[l] >= score[l - 1] && score[l] >= score[l + 1]
    })?;
    // Slide the estimate between samples with a parabola through the peak.
    let (a, b, c) = (score[lag - 1], score[lag], score[lag + 1]);
    let denom = a - 2.0 * b + c;
    let shift = if denom.abs() > 1e-12 {
        (0.5 * (a - c) / denom).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    Some(lag as f64 + shift)
}

/// The value of `mono` at the fractional position `pos`, by cubic interpolation. Past either end
/// is silence.
fn read_cubic(mono: &[f32], pos: f64) -> f32 {
    let i = pos.floor();
    let t = (pos - i) as f32;
    let at = |k: i64| -> f32 {
        if k < 0 || k as usize >= mono.len() {
            0.0
        } else {
            mono[k as usize]
        }
    };
    let i = i as i64;
    let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
    p1 + 0.5
        * t
        * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)))
}

/// Resamples the stretch of `mono` from `start` for `len` samples to one frame.
fn resample_cycle(mono: &[f32], start: f64, len: f64) -> Vec<f32> {
    (0..FRAME_SIZE)
        .map(|j| read_cubic(mono, start + len * j as f64 / FRAME_SIZE as f64))
        .collect()
}

/// Rotates `frame` so it starts at its first rising zero crossing, or at its lowest sample if it
/// never crosses upward.
fn align_phase(frame: &mut [f32]) {
    let n = frame.len();
    if n == 0 {
        return;
    }
    let crossing = (0..n).find(|&i| frame[i] <= 0.0 && frame[(i + 1) % n] > 0.0);
    let start = match crossing {
        Some(i) => {
            // Start on whichever side of the crossing is closer to zero.
            let next = (i + 1) % n;
            if frame[next].abs() < frame[i].abs() {
                next
            } else {
                i
            }
        }
        None => {
            let mut low = 0;
            for (i, &v) in frame.iter().enumerate() {
                if v < frame[low] {
                    low = i;
                }
            }
            low
        }
    };
    frame.rotate_left(start);
}

/// The pitch period to cut by, for `SliceMode::Cycles` on a file with a clear pitch. None for a
/// fixed mode, or for a file with no clear pitch.
fn cycle_period(mono: &[f32], sample_rate: f32, mode: SliceMode) -> Option<f64> {
    match mode {
        SliceMode::Cycles => detect_period(mono, sample_rate),
        SliceMode::Fixed(_) => None,
    }
}

/// Each slice of a file `len` samples long, as a start and a length in samples. `period` is the
/// detected pitch period, which only a `Cycles` mode uses: without one it falls back to fixed
/// slices of `FALLBACK_SLICE`. A fixed slice is made for the last stretch if it is at least half
/// full.
fn plan_slices(len: usize, period: Option<f64>, mode: SliceMode) -> Vec<(f64, f64)> {
    let fixed = |n: usize| {
        let n = n.clamp(MIN_SLICE, MAX_SLICE);
        let whole = len / n;
        let rest = len - whole * n;
        let count = whole + usize::from(rest * 2 >= n);
        (0..count)
            .map(|i| ((i * n) as f64, n as f64))
            .collect::<Vec<_>>()
    };
    match (mode, period) {
        (SliceMode::Fixed(n), _) => fixed(n),
        (SliceMode::Cycles, None) => fixed(FALLBACK_SLICE),
        (SliceMode::Cycles, Some(period)) => {
            let count = (len as f64 / period).floor() as usize;
            (0..count).map(|i| (i as f64 * period, period)).collect()
        }
    }
}

/// How much of the start of a file the pitch search and the first frame need: the half second the
/// search reads, or the longest slice, whichever is more.
fn head_len(sample_rate: f32) -> usize {
    let half_second = if sample_rate.is_finite() && sample_rate > 0.0 {
        (sample_rate * 0.5) as usize
    } else {
        0
    };
    half_second.max(MAX_SLICE) + 8
}

/// How many frames `slice_frames` makes for `data`, without cutting any of them. Only the start of
/// the file is looked at, for the pitch.
pub fn frame_count(data: &SampleData, mode: SliceMode) -> usize {
    if data.len() == 0 {
        return 0;
    }
    let mono = mono_mix_head(data, head_len(data.sample_rate));
    let period = cycle_period(&mono, data.sample_rate, mode);
    plan_slices(data.len(), period, mode)
        .len()
        .min(MAX_IMPORT_FRAMES)
}

/// A preview of the first frame `slice_frames` would make, or None if it makes none. It only reads
/// the start of the file, so it is cheap on a long one. It is scaled to its own loudest sample,
/// where `slice_frames` scales every frame by the loudest of them all.
pub fn first_frame(data: &SampleData, mode: SliceMode) -> Option<Vec<f32>> {
    if data.len() == 0 {
        return None;
    }
    let mono = mono_mix_head(data, head_len(data.sample_rate));
    let period = cycle_period(&mono, data.sample_rate, mode);
    let (start, len) = *plan_slices(data.len(), period, mode).first()?;
    let mut frame = resample_cycle(&mono, start, len);
    remove_dc(&mut frame);
    align_phase(&mut frame);
    let loudest = peak(&frame);
    if loudest > 1e-9 {
        for v in frame.iter_mut() {
            *v /= loudest;
        }
    } else {
        frame.fill(0.0);
    }
    sanitize(&mut frame);
    Some(frame)
}

/// Cuts an audio file into single-cycle frames. The channels are mixed to mono and sliced as
/// `mode` says. Each slice is stretched to `FRAME_SIZE` samples as one cycle, has its average
/// removed, and is rotated to start at a rising zero crossing. Then every frame is scaled by the
/// same gain so the loudest sample in the whole set is 1.0. A silent file gives silent frames,
/// an empty one gives no frames, and no more than 256 frames are made, spread evenly across the
/// file.
pub fn slice_frames(data: &SampleData, mode: SliceMode) -> Vec<Vec<f32>> {
    let mono = mono_mix(data);
    if mono.is_empty() {
        return Vec::new();
    }
    let period = cycle_period(&mono, data.sample_rate, mode);
    let slices = plan_slices(mono.len(), period, mode);
    let picked: Vec<(f64, f64)> = if slices.len() > MAX_IMPORT_FRAMES {
        (0..MAX_IMPORT_FRAMES)
            .map(|i| slices[i * slices.len() / MAX_IMPORT_FRAMES])
            .collect()
    } else {
        slices
    };
    let mut frames: Vec<Vec<f32>> = picked
        .iter()
        .map(|&(start, slice_len)| {
            let mut frame = resample_cycle(&mono, start, slice_len);
            remove_dc(&mut frame);
            align_phase(&mut frame);
            frame
        })
        .collect();
    let loudest = frames.iter().map(|f| peak(f)).fold(0.0_f32, f32::max);
    if loudest > 1e-9 {
        for frame in frames.iter_mut() {
            for v in frame.iter_mut() {
                *v /= loudest;
            }
        }
    } else {
        for frame in frames.iter_mut() {
            frame.fill(0.0);
        }
    }
    for frame in frames.iter_mut() {
        sanitize(frame);
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The size of partial `k` in `frame`, by a plain DFT.
    fn partial(frame: &[f32], k: usize) -> f32 {
        let (mut re, mut im) = (0.0_f64, 0.0_f64);
        for (n, &v) in frame.iter().enumerate() {
            let a = TAU * (k * n) as f64 / frame.len() as f64;
            re += v as f64 * a.cos();
            im += v as f64 * a.sin();
        }
        (2.0 * (re * re + im * im).sqrt() / frame.len() as f64) as f32
    }

    /// The summed size of partials `from` to `to`.
    fn band(frame: &[f32], from: usize, to: usize) -> f32 {
        (from..=to).map(|k| partial(frame, k)).sum()
    }

    /// Checks a frame is the right length, finite, in range, and centred on zero.
    fn assert_good(frame: &[f32]) {
        assert_eq!(frame.len(), FRAME_SIZE);
        assert!(frame.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
        let mean = frame.iter().sum::<f32>() / frame.len() as f32;
        assert!(mean.abs() < 1e-3, "mean {mean}");
    }

    /// The number of times `frame` crosses zero going up, around the loop.
    fn rising_crossings(frame: &[f32]) -> usize {
        let n = frame.len();
        (0..n)
            .filter(|&i| frame[i] < 0.0 && frame[(i + 1) % n] >= 0.0)
            .count()
    }

    /// A sine of `cycles` cycles per `period` samples, `len` samples long, at `rate`.
    fn sine_data(period: f64, len: usize, rate: f32, channels: u8) -> SampleData {
        let mut frames = Vec::new();
        for n in 0..len {
            let v = (TAU * n as f64 / period).sin() as f32 * 0.5;
            for _ in 0..channels {
                frames.push(v);
            }
        }
        SampleData {
            sample_rate: rate,
            channels,
            frames,
        }
    }

    #[test]
    fn the_default_curve_is_a_triangle() {
        let frame = curve_frame(&[]);
        assert_good(&frame);
        for (n, &v) in frame.iter().enumerate() {
            let x = n as f32 / FRAME_SIZE as f32;
            let want = if x < 0.5 {
                4.0 * x - 1.0
            } else {
                3.0 - 4.0 * x
            };
            assert!((v - want).abs() < 1e-4, "at {n}: {v} vs {want}");
        }
        // Points that are not a valid curve fall back to the same triangle.
        assert_eq!(curve_frame(&[[0.3, 0.3]]), frame);
    }

    #[test]
    fn a_curve_frame_reads_the_curve_like_the_lfo_custom_wave() {
        let points = [[0.0, 0.8], [0.25, 0.2], [0.5, 0.75], [1.0, 0.5]];
        let curve = Curve::from_points(&points).unwrap();
        let frame = curve_frame(&points);
        assert_eq!(frame.len(), FRAME_SIZE);
        // The LFO level is curve.eval(x), 0 to 1. The frame is that mapped to -1 to 1, less a
        // constant for the removed average.
        let offset = frame[0] - (curve.eval(0.0) * 2.0 - 1.0);
        for n in [0, 100, 512, 1000, 1500, 2047] {
            let x = n as f32 / FRAME_SIZE as f32;
            let want = curve.eval(x) * 2.0 - 1.0 + offset;
            assert!((frame[n] - want).abs() < 1e-4);
        }
        assert_good(&frame);
    }

    #[test]
    fn a_harmonic_frame_with_one_partial_is_a_sine() {
        let frame = harmonic_frame(1, 0.5, 0.0);
        assert_good(&frame);
        for (n, &v) in frame.iter().enumerate() {
            let want = (TAU * n as f64 / FRAME_SIZE as f64).sin() as f32;
            assert!((v - want).abs() < 1e-4);
        }
    }

    #[test]
    fn tilt_changes_the_high_partial_energy() {
        let buzzy = harmonic_frame(32, 0.0, 0.0);
        let saw = harmonic_frame(32, 0.5, 0.0);
        let soft = harmonic_frame(32, 1.0, 0.0);
        let high = |f: &[f32]| band(f, 10, 32) / partial(f, 1);
        assert!(high(&buzzy) > high(&saw));
        assert!(high(&saw) > high(&soft));
        for f in [&buzzy, &saw, &soft] {
            assert_good(f);
            assert!((peak(f) - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn odd_even_removes_one_family() {
        let square = harmonic_frame(16, 0.5, 1.0);
        let evens: f32 = (1..=8).map(|i| partial(&square, 2 * i)).sum();
        assert!(evens < 1e-3);
        assert!(partial(&square, 1) > 0.1 && partial(&square, 3) > 0.1);
        let evens_only = harmonic_frame(16, 0.5, -1.0);
        let odds: f32 = (0..8).map(|i| partial(&evens_only, 2 * i + 1)).sum();
        assert!(odds < 1e-3);
        assert!(partial(&evens_only, 2) > 0.1);
        assert_good(&square);
        assert_good(&evens_only);
    }

    #[test]
    fn a_silent_harmonic_frame_stays_silent() {
        let frame = harmonic_frame(1, 0.5, -1.0);
        assert!(frame.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn sweep_kinds_round_trip() {
        for (i, kind) in SweepKind::ALL.iter().enumerate() {
            assert_eq!(kind.index(), i);
            assert_eq!(SweepKind::from_index(i), *kind);
            assert_eq!(*kind as u8 as usize, i);
        }
        assert_eq!(SweepKind::from_index(99), SweepKind::Harmonics);
        assert_eq!(SweepKind::LABELS[1], "PWM");
    }

    #[test]
    fn sweeps_have_the_right_shape() {
        for kind in SweepKind::ALL {
            let frames = sweep_frames(kind, 8, 1.0);
            assert_eq!(frames.len(), 8);
            frames.iter().for_each(|f| assert_good(f));
            assert_eq!(sweep_frames(kind, 0, 1.0).len(), 2);
            assert_eq!(sweep_frames(kind, 500, 1.0).len(), 64);
            // At amount 0 every frame is the first frame, and that is the first frame of any
            // other amount.
            let flat = sweep_frames(kind, 4, 0.0);
            assert!(flat.iter().all(|f| *f == flat[0]));
            assert_eq!(flat[0], frames[0]);
        }
    }

    #[test]
    fn harmonics_sweep_gains_high_partials() {
        let frames = sweep_frames(SweepKind::Harmonics, 5, 1.0);
        let high: Vec<f32> = frames.iter().map(|f| band(f, 8, 64)).collect();
        assert!(high[0] < 1e-3);
        assert!(high.windows(2).all(|w| w[1] > w[0]));
        // The first frame is a sine.
        assert!(partial(&frames[0], 1) > 0.99);
    }

    #[test]
    fn pwm_sweep_narrows_the_pulse() {
        let frames = sweep_frames(SweepKind::Pwm, 4, 1.0);
        let positive = |f: &[f32]| f.iter().filter(|&&v| v > 0.0).count() as f32 / f.len() as f32;
        assert!((positive(&frames[0]) - 0.5).abs() < 0.02);
        assert!((positive(&frames[3]) - 0.05).abs() < 0.02);
    }

    #[test]
    fn sync_sweep_ends_on_eight_cycles() {
        let frames = sweep_frames(SweepKind::Sync, 4, 1.0);
        assert_eq!(rising_crossings(&frames[0]), 1);
        assert_eq!(rising_crossings(&frames[3]), 8);
    }

    #[test]
    fn fold_sweep_gains_harmonics() {
        let frames = sweep_frames(SweepKind::Fold, 4, 1.0);
        let high: Vec<f32> = frames.iter().map(|f| band(f, 5, 40)).collect();
        assert!(high[3] > high[1] && high[1] > high[0]);
    }

    #[test]
    fn fixed_slices_are_aligned_sines() {
        let data = sine_data(1000.0, 10_000, 44100.0, 1);
        let frames = slice_frames(&data, SliceMode::Fixed(1000));
        assert_eq!(frames.len(), 10);
        for f in &frames {
            assert_good(f);
            assert!(f[0].abs() < 0.05, "starts at {}", f[0]);
            assert!(f[1] > f[0]);
            assert!((f[FRAME_SIZE / 4] - 1.0).abs() < 0.02);
        }
    }

    #[test]
    fn a_short_last_slice_is_dropped_and_a_long_one_kept() {
        let mut data = sine_data(100.0, 1000, 44100.0, 1);
        data.frames.extend(vec![0.1; 400]);
        assert_eq!(slice_frames(&data, SliceMode::Fixed(1000)).len(), 1);
        data.frames.extend(vec![0.1; 200]);
        assert_eq!(slice_frames(&data, SliceMode::Fixed(1000)).len(), 2);
    }

    #[test]
    fn cycles_mode_cuts_one_period_per_slice() {
        let data = sine_data(44100.0 / 220.0, 44100, 44100.0, 1);
        let frames = slice_frames(&data, SliceMode::Cycles);
        let want = (44100.0_f64 / (44100.0 / 220.0)) as i64;
        assert!((frames.len() as i64 - want).abs() <= 1, "{}", frames.len());
        for f in &frames {
            assert_good(f);
            // One cycle per frame.
            assert_eq!(rising_crossings(f), 1);
            assert!(f[0].abs() < 0.05);
        }
    }

    #[test]
    fn noise_falls_back_to_fixed_slices() {
        let mut state = 12345_u32;
        let frames: Vec<f32> = (0..FALLBACK_SLICE * 10)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect();
        let data = SampleData {
            sample_rate: 44100.0,
            channels: 1,
            frames,
        };
        let cycles = slice_frames(&data, SliceMode::Cycles);
        assert_eq!(cycles.len(), 10);
        assert_eq!(
            cycles,
            slice_frames(&data, SliceMode::Fixed(FALLBACK_SLICE))
        );
    }

    #[test]
    fn stereo_mixes_to_mono() {
        let mono = sine_data(1000.0, 4000, 44100.0, 1);
        let stereo = sine_data(1000.0, 4000, 44100.0, 2);
        assert_eq!(
            slice_frames(&mono, SliceMode::Fixed(1000)),
            slice_frames(&stereo, SliceMode::Fixed(1000))
        );
        // Opposite channels cancel.
        let cancel = SampleData {
            sample_rate: 44100.0,
            channels: 2,
            frames: (0..8000)
                .map(|i| if i % 2 == 0 { 0.5 } else { -0.5 })
                .collect(),
        };
        let frames = slice_frames(&cancel, SliceMode::Fixed(1000));
        assert!(frames.iter().all(|f| f.iter().all(|&v| v == 0.0)));
    }

    #[test]
    fn one_gain_makes_the_loudest_sample_one() {
        let mut data = sine_data(1000.0, 2000, 44100.0, 1);
        for v in data.frames[1000..].iter_mut() {
            *v *= 0.25;
        }
        let frames = slice_frames(&data, SliceMode::Fixed(1000));
        let top = frames.iter().map(|f| peak(f)).fold(0.0_f32, f32::max);
        assert!((top - 1.0).abs() < 1e-5);
        // The quiet slice keeps its relative level.
        assert!((peak(&frames[1]) - 0.25).abs() < 0.02);
    }

    #[test]
    fn empty_and_silent_data() {
        let empty = SampleData {
            sample_rate: 44100.0,
            channels: 1,
            frames: Vec::new(),
        };
        assert!(slice_frames(&empty, SliceMode::Cycles).is_empty());
        assert!(slice_frames(&empty, SliceMode::Fixed(100)).is_empty());
        let silent = SampleData {
            sample_rate: 44100.0,
            channels: 1,
            frames: vec![0.0; 3000],
        };
        let frames = slice_frames(&silent, SliceMode::Cycles);
        assert!(!frames.is_empty());
        assert!(frames.iter().all(|f| f.iter().all(|&v| v == 0.0)));
    }

    #[test]
    fn the_frame_count_is_capped() {
        let data = sine_data(64.0, 64 * 300, 44100.0, 1);
        let frames = slice_frames(&data, SliceMode::Fixed(64));
        assert_eq!(frames.len(), 256);
    }

    /// The cheap count agrees with the real slicing in every mode.
    #[test]
    fn frame_count_matches_slice_frames() {
        let pitched = sine_data(100.0, 10_000, 44100.0, 1);
        let noise = SampleData {
            sample_rate: 44100.0,
            channels: 1,
            frames: (0..9000)
                .map(|n| ((n * 7919) % 1000) as f32 / 500.0 - 1.0)
                .collect(),
        };
        let many = sine_data(64.0, 64 * 300, 44100.0, 1);
        let empty = SampleData {
            sample_rate: 44100.0,
            channels: 1,
            frames: Vec::new(),
        };
        for data in [&pitched, &noise, &many, &empty] {
            for mode in [
                SliceMode::Cycles,
                SliceMode::Fixed(1024),
                SliceMode::Fixed(64),
            ] {
                assert_eq!(
                    frame_count(data, mode),
                    slice_frames(data, mode).len(),
                    "{mode:?}"
                );
            }
        }
    }

    /// The preview is a valid frame that has the shape of the real first frame.
    #[test]
    fn first_frame_previews_the_first_slice() {
        let data = sine_data(100.0, 10_000, 44100.0, 1);
        let preview = first_frame(&data, SliceMode::Cycles).expect("a frame");
        assert_good(&preview);
        let real = &slice_frames(&data, SliceMode::Cycles)[0];
        let worst = preview
            .iter()
            .zip(real)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(worst < 1e-3, "{worst}");
        let empty = SampleData {
            sample_rate: 44100.0,
            channels: 1,
            frames: Vec::new(),
        };
        assert!(first_frame(&empty, SliceMode::Cycles).is_none());
        let short = sine_data(100.0, 20, 44100.0, 1);
        assert!(first_frame(&short, SliceMode::Fixed(1024)).is_none());
    }
}
