//! Custom wavetables: band-limited frames grouped into regions, and the ramp that maps a position
//! from 0 to 1 onto the frames that play.

// Nothing outside this file uses these yet. Remove this once the table node is wired in.
#![allow(dead_code)]

use super::wavetable::{level_for, max_partials, TABLE_LEVELS, TABLE_SIZE};
use std::collections::hash_map::DefaultHasher;
use std::f64::consts::TAU;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

/// Samples in one frame. It matches the built-in band-limited tables.
pub const FRAME_SIZE: usize = TABLE_SIZE;
/// The most regions a table can hold.
pub const MAX_REGIONS: usize = 8;
/// The smallest share a region may have, so no region vanishes from the ramp.
const MIN_SHARE: f32 = 0.02;
/// The most tables the build cache keeps.
const CACHE_LIMIT: usize = 16;

/// A complex FFT in place, for a power-of-two length. The forward transform is unscaled. The
/// inverse divides by the length, so a forward and an inverse transform give back the input.
pub fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
    if n < 2 {
        return;
    }
    // Put the samples in bit-reversed order.
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
    // One table of twiddle factors serves every stage.
    let sign = if inverse { 1.0 } else { -1.0 };
    let cos_t: Vec<f64> = (0..n / 2)
        .map(|k| (TAU * k as f64 / n as f64).cos())
        .collect();
    let sin_t: Vec<f64> = (0..n / 2)
        .map(|k| sign * (TAU * k as f64 / n as f64).sin())
        .collect();
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        let stride = n / len;
        for start in (0..n).step_by(len) {
            for k in 0..half {
                let (wr, wi) = (cos_t[k * stride], sin_t[k * stride]);
                let (a, b) = (start + k, start + k + half);
                let tr = re[b] * wr - im[b] * wi;
                let ti = re[b] * wi + im[b] * wr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len *= 2;
    }
    if inverse {
        let scale = 1.0 / n as f64;
        for (r, i) in re.iter_mut().zip(im.iter_mut()) {
            *r *= scale;
            *i *= scale;
        }
    }
}

/// One cycle stored as band-limited copies, one per pitch range, like `WaveTables`. Level 0 keeps
/// the most partials. The DC offset is removed from every level.
pub struct MippedFrame {
    /// `TABLE_LEVELS` cycles of `FRAME_SIZE` samples, level 0 first.
    levels: Vec<f32>,
}

impl std::fmt::Debug for MippedFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MippedFrame")
            .field("levels", &TABLE_LEVELS)
            .finish()
    }
}

impl MippedFrame {
    /// Build the copies from one cycle in the time domain. A cycle of the wrong length is
    /// truncated or padded with silence, and trips a debug assertion.
    pub fn new(frame: &[f32]) -> Self {
        debug_assert_eq!(frame.len(), FRAME_SIZE);
        let n = FRAME_SIZE;
        let mut spec_re = vec![0.0f64; n];
        let mut spec_im = vec![0.0f64; n];
        for (dst, &src) in spec_re.iter_mut().zip(frame.iter()) {
            *dst = if src.is_finite() { src as f64 } else { 0.0 };
        }
        fft(&mut spec_re, &mut spec_im, false);

        let mut levels = Vec::with_capacity(TABLE_LEVELS * n);
        let mut re = vec![0.0f64; n];
        let mut im = vec![0.0f64; n];
        for level in 0..TABLE_LEVELS {
            let partials = max_partials(level);
            for k in 0..n {
                // Bin k and bin n - k are the same partial, mirrored.
                let partial = k.min(n - k);
                if partial == 0 || partial > partials {
                    re[k] = 0.0;
                    im[k] = 0.0;
                } else {
                    re[k] = spec_re[k];
                    im[k] = spec_im[k];
                }
            }
            fft(&mut re, &mut im, true);
            levels.extend(re.iter().map(|&x| x as f32));
        }
        Self { levels }
    }

    /// The value at `phase`, from 0.0 to 1.0, for a phase advance of `step` per sample.
    #[inline]
    pub fn sample(&self, phase: f32, step: f32) -> f32 {
        let level = level_for(step.abs());
        let table = &self.levels[level * FRAME_SIZE..(level + 1) * FRAME_SIZE];
        let phase = phase - phase.floor();
        let position = phase * FRAME_SIZE as f32;
        let i = (position as usize) % FRAME_SIZE;
        let frac = position - position.floor();
        let next = (i + 1) % FRAME_SIZE;
        table[i] + (table[next] - table[i]) * frac
    }

    /// The cycle with the most bandwidth, for drawing.
    pub fn full(&self) -> &[f32] {
        &self.levels[..FRAME_SIZE]
    }
}

/// A run of frames the position sweeps through.
#[derive(Debug)]
pub struct Region {
    /// At least one frame.
    pub frames: Vec<MippedFrame>,
}

/// The built frames of a custom wavetable, grouped into regions.
#[derive(Debug)]
pub struct TableData {
    pub regions: Vec<Region>,
}

impl TableData {
    /// Build a table from one list of frames per region. A region with no frames gets one silent
    /// frame, and regions past `MAX_REGIONS` are dropped.
    pub fn new(regions: Vec<Vec<Vec<f32>>>) -> Self {
        let mut out: Vec<Region> = regions
            .into_iter()
            .take(MAX_REGIONS)
            .map(|frames| {
                let frames = if frames.is_empty() {
                    vec![MippedFrame::new(&vec![0.0; FRAME_SIZE])]
                } else {
                    frames.iter().map(|f| MippedFrame::new(f)).collect()
                };
                Region { frames }
            })
            .collect();
        if out.is_empty() {
            out.push(Region {
                frames: vec![MippedFrame::new(&vec![0.0; FRAME_SIZE])],
            });
        }
        Self { regions: out }
    }

    /// How many regions the table has.
    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    /// How many frames there are in all regions.
    pub fn frame_count(&self) -> usize {
        self.regions.iter().map(|r| r.frames.len()).sum()
    }

    /// The same as `new`, but reuses a recent build of identical input, so recompiling the patch
    /// on every knob drag doesn't rebuild an unchanged table. Call it from the editor thread only.
    pub fn build_cached(regions: Vec<Vec<Vec<f32>>>) -> Arc<TableData> {
        let key = hash_regions(&regions);
        if let Ok(cache) = cache().lock() {
            if let Some((_, hit)) = cache.iter().find(|(k, _)| *k == key) {
                return hit.clone();
            }
        }
        // Build outside the lock so a slow build doesn't hold it.
        let built = Arc::new(TableData::new(regions));
        if let Ok(mut cache) = cache().lock() {
            if !cache.iter().any(|(k, _)| *k == key) {
                if cache.len() >= CACHE_LIMIT {
                    cache.remove(0);
                }
                cache.push((key, built.clone()));
            }
        }
        built
    }

    /// The value at a location, for a note at `phase` (0.0 to 1.0) advancing `step` per sample.
    #[inline]
    pub fn sample(&self, loc: &Locate, phase: f32, step: f32) -> f32 {
        let a = self.read_tap(&loc.a, phase, step);
        if loc.t == 0.0 {
            return a;
        }
        let b = self.read_tap(&loc.b, phase, step);
        a + (b - a) * loc.t
    }

    /// One tap, interpolating between the two frames either side of its fractional index.
    #[inline]
    fn read_tap(&self, tap: &Tap, phase: f32, step: f32) -> f32 {
        let region = &self.regions[tap.region.min(self.regions.len() - 1)];
        let last = region.frames.len() - 1;
        let f = tap.frame.max(0.0);
        let i = (f as usize).min(last);
        let frac = f - i as f32;
        let first = region.frames[i].sample(phase, step);
        if frac == 0.0 || i >= last {
            return first;
        }
        let second = region.frames[i + 1].sample(phase, step);
        first + (second - first) * frac
    }

    /// The full-bandwidth cycle at a location, for the editor's wave display.
    pub fn frame_at(&self, loc: &Locate) -> Vec<f32> {
        let a = self.tap_frame(&loc.a);
        if loc.t == 0.0 {
            return a;
        }
        let b = self.tap_frame(&loc.b);
        a.iter().zip(&b).map(|(x, y)| x + (y - x) * loc.t).collect()
    }

    /// The full-bandwidth cycle at one tap.
    fn tap_frame(&self, tap: &Tap) -> Vec<f32> {
        let region = &self.regions[tap.region.min(self.regions.len() - 1)];
        let last = region.frames.len() - 1;
        let f = tap.frame.max(0.0);
        let i = (f as usize).min(last);
        let frac = f - i as f32;
        let first = region.frames[i].full();
        if frac == 0.0 || i >= last {
            return first.to_vec();
        }
        let second = region.frames[i + 1].full();
        first
            .iter()
            .zip(second)
            .map(|(x, y)| x + (y - x) * frac)
            .collect()
    }
}

/// The same as `TableData::build_cached`, for callers that want a plain function.
pub fn build_cached(regions: Vec<Vec<Vec<f32>>>) -> Arc<TableData> {
    TableData::build_cached(regions)
}

/// The process-wide build cache, oldest entry first.
fn cache() -> &'static Mutex<Vec<(u64, Arc<TableData>)>> {
    static CACHE: Mutex<Vec<(u64, Arc<TableData>)>> = Mutex::new(Vec::new());
    &CACHE
}

/// A hash of every sample bit and the region and frame boundaries.
fn hash_regions(regions: &[Vec<Vec<f32>>]) -> u64 {
    let mut h = DefaultHasher::new();
    regions.len().hash(&mut h);
    for region in regions {
        region.len().hash(&mut h);
        for frame in region {
            frame.len().hash(&mut h);
            for s in frame {
                s.to_bits().hash(&mut h);
            }
        }
    }
    h.finish()
}

/// How the position moves from one region to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum BlendMode {
    /// A straight crossfade.
    #[default]
    Linear = 0,
    /// A crossfade that eases in and out.
    Smooth = 1,
    /// A hard switch at the boundary.
    Step = 2,
}

impl BlendMode {
    /// Every mode, in the order the dropdown lists them.
    pub const ALL: [BlendMode; 3] = [BlendMode::Linear, BlendMode::Smooth, BlendMode::Step];

    /// The mode for a dropdown row. A row out of range gives the default.
    pub fn from_index(i: usize) -> BlendMode {
        Self::ALL.get(i).copied().unwrap_or_default()
    }

    /// The dropdown row for this mode.
    pub fn index(self) -> usize {
        self as usize
    }
}

/// One read of a region, at a fractional frame index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tap {
    pub region: usize,
    /// The frame index inside the region. The fraction mixes the next frame in.
    pub frame: f32,
}

/// Where a position lands. The output is the mix of `a` and `b` by `t`. With no blend, `t` is
/// 0.0 and `b` equals `a`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Locate {
    pub a: Tap,
    pub b: Tap,
    pub t: f32,
}

/// The layout of regions along the position, from 0 to 1, and how they blend.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ramp {
    count: usize,
    /// Where each region starts, then 1.0 as the end of the last. Only `count + 1` are used.
    edges: [f32; MAX_REGIONS + 1],
    blend: BlendMode,
    width: f32,
}

impl Ramp {
    /// Lay out regions by their relative `shares`, one per region. A share is at least 0.02, extra
    /// shares past `MAX_REGIONS` are ignored, and no shares means one region. `width` is the
    /// blend zone size from 0.0 to 1.0, as a fraction of the shorter neighbouring region.
    pub fn new(shares: &[f32], blend: BlendMode, width: f32) -> Ramp {
        let count = shares.len().clamp(1, MAX_REGIONS);
        let mut clamped = [1.0f32; MAX_REGIONS];
        for (dst, &s) in clamped.iter_mut().zip(shares.iter().take(count)) {
            *dst = if s.is_finite() {
                s.max(MIN_SHARE)
            } else {
                MIN_SHARE
            };
        }
        let total: f32 = clamped[..count].iter().sum();
        let mut edges = [1.0f32; MAX_REGIONS + 1];
        let mut run = 0.0f32;
        for i in 0..count {
            edges[i] = run / total;
            run += clamped[i];
        }
        edges[count] = 1.0;
        Ramp {
            count,
            edges,
            blend,
            width: if width.is_finite() {
                width.clamp(0.0, 1.0)
            } else {
                0.0
            },
        }
    }

    /// How many regions the ramp has.
    pub fn region_count(&self) -> usize {
        self.count
    }

    /// The start and end of a region along the position, from 0 to 1.
    pub fn span(&self, region: usize) -> (f32, f32) {
        let r = region.min(self.count - 1);
        (self.edges[r], self.edges[r + 1])
    }

    /// The boundaries between regions. The first `region_count() - 1` entries are real, and the
    /// rest are 1.0.
    pub fn boundaries(&self) -> [f32; MAX_REGIONS] {
        let mut out = [1.0f32; MAX_REGIONS];
        for (i, slot) in out.iter_mut().enumerate().take(self.count - 1) {
            *slot = self.edges[i + 1];
        }
        out
    }

    /// The half width of the blend zone on the boundary after `region`. Step has none.
    fn half_zone(&self, region: usize) -> f32 {
        if self.blend == BlendMode::Step {
            return 0.0;
        }
        let (s0, e0) = self.span(region);
        let (s1, e1) = self.span(region + 1);
        self.width * (e0 - s0).min(e1 - s1) * 0.5
    }

    /// Where `position`, from 0 to 1, lands. `table_frame_counts` is the number of frames in each
    /// region.
    pub fn locate(&self, table_frame_counts: &[usize], position: f32) -> Locate {
        let p = if position.is_nan() {
            0.0
        } else {
            position.clamp(0.0, 1.0)
        };
        let frames_in = |r: usize| table_frame_counts.get(r).copied().unwrap_or(1).max(1);
        // Inside a blend zone, the neighbouring regions mix.
        for j in 0..self.count - 1 {
            let edge = self.edges[j + 1];
            let half = self.half_zone(j);
            if half > 0.0 && p >= edge - half && p <= edge + half {
                let x = ((p - (edge - half)) / (2.0 * half)).clamp(0.0, 1.0);
                let t = match self.blend {
                    BlendMode::Smooth => x * x * (3.0 - 2.0 * x),
                    _ => x,
                };
                return Locate {
                    a: Tap {
                        region: j,
                        frame: (frames_in(j) - 1) as f32,
                    },
                    b: Tap {
                        region: j + 1,
                        frame: 0.0,
                    },
                    t,
                };
            }
        }
        // Otherwise it sits plainly inside one region.
        let mut region = 0;
        while region + 1 < self.count && p >= self.edges[region + 1] {
            region += 1;
        }
        let (start, end) = self.span(region);
        let along = ((p - start) / (end - start)).clamp(0.0, 1.0);
        let tap = Tap {
            region,
            frame: along * (frames_in(region) - 1) as f32,
        };
        Locate {
            a: tap,
            b: tap,
            t: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU as TAU32;

    /// A tiny deterministic generator, so tests need no rand crate.
    struct Lcg(u32);

    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
            (self.0 >> 8) as f32 / (1u32 << 24) as f32
        }
    }

    fn sine(harmonic: usize) -> Vec<f32> {
        (0..FRAME_SIZE)
            .map(|i| (TAU32 * (harmonic * i) as f32 / FRAME_SIZE as f32).sin())
            .collect()
    }

    fn saw() -> Vec<f32> {
        (0..FRAME_SIZE)
            .map(|i| 2.0 * i as f32 / FRAME_SIZE as f32 - 1.0)
            .collect()
    }

    /// The magnitude of each bin of a cycle.
    fn magnitudes(frame: &[f32]) -> Vec<f64> {
        let mut re: Vec<f64> = frame.iter().map(|&x| x as f64).collect();
        let mut im = vec![0.0; frame.len()];
        fft(&mut re, &mut im, false);
        re.iter().zip(&im).map(|(r, i)| r.hypot(*i)).collect()
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn fft_round_trip() {
        let mut lcg = Lcg(7);
        let orig: Vec<f64> = (0..256).map(|_| lcg.next() as f64 - 0.5).collect();
        let mut re = orig.clone();
        let mut im = vec![0.0; 256];
        fft(&mut re, &mut im, false);
        fft(&mut re, &mut im, true);
        for (a, b) in re.iter().zip(&orig) {
            assert!((a - b).abs() < 1e-9);
        }
        assert!(im.iter().all(|x| x.abs() < 1e-9));
    }

    #[test]
    fn fft_finds_a_sine_bin() {
        let mags = magnitudes(&sine(5));
        assert!((mags[5] - FRAME_SIZE as f64 / 2.0).abs() < 1e-3);
        assert!(mags[6] < 1e-3);
    }

    #[test]
    fn level_zero_of_a_sine_is_a_sine() {
        let frame = MippedFrame::new(&sine(1));
        let want = sine(1);
        for (a, b) in frame.full().iter().zip(&want) {
            assert!((a - b).abs() < 1e-4);
        }
        assert!((frame.sample(0.25, 0.0001) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn high_levels_drop_the_partials_above_the_limit() {
        let frame = MippedFrame::new(&saw());
        for level in [3, 5, 7] {
            let step = (LOWEST_STEP_FOR_TEST * (1 << level) as f32) * 0.99;
            assert_eq!(level_for(step), level);
            let table = &frame.levels[level * FRAME_SIZE..(level + 1) * FRAME_SIZE];
            let mags = magnitudes(table);
            let limit = max_partials(level);
            for k in (limit + 1)..FRAME_SIZE / 2 {
                assert!(mags[k] < 1e-2, "level {level} bin {k}: {}", mags[k]);
            }
            assert!(mags[1] > 100.0);
        }
    }

    /// The top step of level 0, repeated here so the test reads on its own.
    const LOWEST_STEP_FOR_TEST: f32 = 1.0 / 2048.0;

    #[test]
    fn dc_is_removed_in_every_level() {
        let offset: Vec<f32> = sine(2).iter().map(|x| x + 0.5).collect();
        let frame = MippedFrame::new(&offset);
        for level in 0..TABLE_LEVELS {
            let table = &frame.levels[level * FRAME_SIZE..(level + 1) * FRAME_SIZE];
            let mean: f32 = table.iter().sum::<f32>() / FRAME_SIZE as f32;
            assert!(mean.abs() < 1e-4, "level {level} mean {mean}");
        }
    }

    #[test]
    fn high_pitch_saw_stays_in_range_and_centred() {
        let frame = MippedFrame::new(&saw());
        let step = 2000.0 / 44100.0;
        let n = (1.0 / step) as usize;
        let mut sum = 0.0;
        let mut peak = 0.0f32;
        for i in 0..n {
            let y = frame.sample((i as f32 * step).fract(), step);
            sum += y;
            peak = peak.max(y.abs());
        }
        assert!((sum / n as f32).abs() < 0.02);
        assert!(peak < 1.15, "peak {peak}");
    }

    #[test]
    fn sample_wraps_phase_and_negative_step() {
        let frame = MippedFrame::new(&sine(1));
        assert!(close(frame.sample(1.25, 0.001), frame.sample(0.25, 0.001)));
        assert!(close(frame.sample(-0.75, 0.001), frame.sample(0.25, 0.001)));
        assert!(close(frame.sample(0.3, -0.001), frame.sample(0.3, 0.001)));
    }

    #[test]
    fn shares_become_spans() {
        let r = Ramp::new(&[1.0, 1.0, 1.0, 1.0], BlendMode::Linear, 0.0);
        assert_eq!(r.region_count(), 4);
        assert!(close(r.span(1).0, 0.25) && close(r.span(1).1, 0.5));
        assert_eq!(r.span(3).1, 1.0);

        let r = Ramp::new(&[1.0, 3.0], BlendMode::Linear, 0.0);
        assert!(close(r.span(0).1, 0.25) && close(r.span(1).0, 0.25));

        // A tiny share is raised to the minimum, so the region keeps some room.
        let r = Ramp::new(&[0.0, 1.0], BlendMode::Linear, 0.0);
        let want = 0.02 / 1.02;
        assert!(close(r.span(0).1, want));

        let r = Ramp::new(&[5.0], BlendMode::Linear, 0.5);
        assert_eq!((r.region_count(), r.span(0)), (1, (0.0, 1.0)));

        let r = Ramp::new(&[], BlendMode::Linear, 0.5);
        assert_eq!((r.region_count(), r.span(0)), (1, (0.0, 1.0)));

        let r = Ramp::new(&[1.0; 12], BlendMode::Linear, 0.0);
        assert_eq!(r.region_count(), MAX_REGIONS);
    }

    #[test]
    fn boundaries_list_the_cumulative_edges() {
        let r = Ramp::new(&[1.0, 1.0, 2.0], BlendMode::Linear, 0.0);
        let b = r.boundaries();
        assert!(close(b[0], 0.25) && close(b[1], 0.5));
        assert!(b[2..].iter().all(|&x| x == 1.0));
        let single = Ramp::new(&[1.0], BlendMode::Linear, 0.0);
        assert!(single.boundaries().iter().all(|&x| x == 1.0));
    }

    #[test]
    fn blend_mode_indices() {
        for (i, m) in BlendMode::ALL.iter().enumerate() {
            assert_eq!(m.index(), i);
            assert_eq!(BlendMode::from_index(i), *m);
        }
        assert_eq!(BlendMode::from_index(99), BlendMode::Linear);
    }

    #[test]
    fn a_one_frame_region_is_constant() {
        let r = Ramp::new(&[1.0], BlendMode::Linear, 0.5);
        for p in [0.0, 0.3, 1.0] {
            let loc = r.locate(&[1], p);
            assert_eq!(loc.a.frame, 0.0);
            assert_eq!(loc.t, 0.0);
            assert_eq!(loc.a, loc.b);
        }
    }

    #[test]
    fn a_multi_frame_region_runs_its_index_linearly() {
        let r = Ramp::new(&[1.0], BlendMode::Linear, 0.0);
        let loc = r.locate(&[5], 0.5);
        assert!(close(loc.a.frame, 2.0));
        assert!(close(r.locate(&[5], 1.0).a.frame, 4.0));
        assert!(close(r.locate(&[5], 0.0).a.frame, 0.0));
        // Two regions: each runs from its own start to its own end.
        let r = Ramp::new(&[1.0, 1.0], BlendMode::Linear, 0.0);
        let loc = r.locate(&[3, 5], 0.25);
        assert_eq!(loc.a.region, 0);
        assert!(close(loc.a.frame, 1.0));
        let loc = r.locate(&[3, 5], 0.75);
        assert_eq!(loc.a.region, 1);
        assert!(close(loc.a.frame, 2.0));
    }

    #[test]
    fn blend_zones_follow_the_mode() {
        let counts = [3, 5];
        // Zone is 0.25 wide, from 0.375 to 0.625.
        let linear = Ramp::new(&[1.0, 1.0], BlendMode::Linear, 0.5);
        let loc = linear.locate(&counts, 0.5);
        assert!(close(loc.t, 0.5));
        assert_eq!((loc.a.region, loc.a.frame), (0, 2.0));
        assert_eq!((loc.b.region, loc.b.frame), (1, 0.0));
        assert!(close(linear.locate(&counts, 0.4375).t, 0.25));
        assert!(close(linear.locate(&counts, 0.5625).t, 0.75));
        assert_eq!(linear.locate(&counts, 0.3).t, 0.0);
        assert_eq!(linear.locate(&counts, 0.7).t, 0.0);

        let smooth = Ramp::new(&[1.0, 1.0], BlendMode::Smooth, 0.5);
        assert!(close(smooth.locate(&counts, 0.5).t, 0.5));
        assert!(close(smooth.locate(&counts, 0.4375).t, 0.15625));
        assert!(close(smooth.locate(&counts, 0.5625).t, 0.84375));

        let step = Ramp::new(&[1.0, 1.0], BlendMode::Step, 0.5);
        let before = step.locate(&counts, 0.49);
        assert_eq!((before.t, before.a.region), (0.0, 0));
        let after = step.locate(&counts, 0.51);
        assert_eq!((after.t, after.a.region), (0.0, 1));
        assert_eq!(after.a, after.b);
    }

    #[test]
    fn zero_width_is_a_hard_switch() {
        let r = Ramp::new(&[1.0, 1.0], BlendMode::Linear, 0.0);
        let before = r.locate(&[2, 2], 0.499);
        let after = r.locate(&[2, 2], 0.501);
        assert_eq!((before.t, after.t), (0.0, 0.0));
        assert_eq!((before.a.region, after.a.region), (0, 1));
    }

    #[test]
    fn zones_never_overlap() {
        let mut lcg = Lcg(42);
        for _ in 0..200 {
            let n = 2 + (lcg.next() * 7.0) as usize;
            let shares: Vec<f32> = (0..n).map(|_| lcg.next() * 5.0).collect();
            let width = lcg.next();
            let r = Ramp::new(&shares, BlendMode::Linear, width);
            let mut prev_end = 0.0f32;
            for j in 0..r.region_count() - 1 {
                let edge = r.span(j).1;
                let half = r.half_zone(j);
                assert!(edge - half >= prev_end - 1e-6, "zone {j} overlaps");
                assert!(edge - half >= r.span(j).0 - 1e-6);
                assert!(edge + half <= r.span(j + 1).1 + 1e-6);
                prev_end = edge + half;
            }
        }
    }

    #[test]
    fn position_is_clamped() {
        let r = Ramp::new(&[1.0, 1.0], BlendMode::Linear, 0.0);
        assert_eq!(r.locate(&[3, 3], -2.0), r.locate(&[3, 3], 0.0));
        assert_eq!(r.locate(&[3, 3], 7.0), r.locate(&[3, 3], 1.0));
        assert!(close(r.locate(&[3, 3], 7.0).a.frame, 2.0));
        assert_eq!(r.locate(&[3, 3], f32::NAN), r.locate(&[3, 3], 0.0));
    }

    fn two_region_table() -> TableData {
        TableData::new(vec![vec![sine(1), saw()], vec![sine(3)]])
    }

    #[test]
    fn table_counts_and_defaults() {
        let t = two_region_table();
        assert_eq!((t.region_count(), t.frame_count()), (2, 3));
        let t = TableData::new(vec![vec![], vec![sine(1)]]);
        assert_eq!((t.region_count(), t.frame_count()), (2, 2));
        assert!(t.regions[0].frames[0].full().iter().all(|&x| x == 0.0));
        let t = TableData::new(vec![vec![sine(1)]; 12]);
        assert_eq!(t.region_count(), MAX_REGIONS);
        assert_eq!(TableData::new(vec![]).region_count(), 1);
    }

    #[test]
    fn sample_matches_a_direct_read_without_a_blend() {
        let t = two_region_table();
        let tap = Tap {
            region: 0,
            frame: 0.0,
        };
        let loc = Locate {
            a: tap,
            b: tap,
            t: 0.0,
        };
        let (phase, step) = (0.37, 0.003);
        assert_eq!(
            t.sample(&loc, phase, step),
            t.regions[0].frames[0].sample(phase, step)
        );
        // A fractional index mixes two frames of the region.
        let tap = Tap {
            region: 0,
            frame: 0.25,
        };
        let loc = Locate {
            a: tap,
            b: tap,
            t: 0.0,
        };
        let f = &t.regions[0].frames;
        let x = f[0].sample(phase, step);
        let y = f[1].sample(phase, step);
        assert!(close(t.sample(&loc, phase, step), x + (y - x) * 0.25));
    }

    #[test]
    fn sample_blends_two_taps() {
        let t = two_region_table();
        let loc = Locate {
            a: Tap {
                region: 0,
                frame: 0.0,
            },
            b: Tap {
                region: 1,
                frame: 0.0,
            },
            t: 0.25,
        };
        let (phase, step) = (0.61, 0.002);
        let a = t.regions[0].frames[0].sample(phase, step);
        let b = t.regions[1].frames[0].sample(phase, step);
        assert!(close(t.sample(&loc, phase, step), a + (b - a) * 0.25));
    }

    #[test]
    fn frame_at_halfway_is_the_average() {
        let t = two_region_table();
        let loc = Locate {
            a: Tap {
                region: 0,
                frame: 0.0,
            },
            b: Tap {
                region: 1,
                frame: 0.0,
            },
            t: 0.5,
        };
        let out = t.frame_at(&loc);
        let a = t.regions[0].frames[0].full();
        let b = t.regions[1].frames[0].full();
        assert_eq!(out.len(), FRAME_SIZE);
        for i in 0..FRAME_SIZE {
            assert!(close(out[i], (a[i] + b[i]) * 0.5));
        }
    }

    /// One test holds every cache check, because the cache is shared by the whole process.
    #[test]
    fn build_cached_reuses_identical_input() {
        let make = |amp: f32| vec![vec![sine(1).iter().map(|x| x * amp).collect::<Vec<f32>>()]];
        let a = TableData::build_cached(make(0.31));
        let b = TableData::build_cached(make(0.31));
        assert!(Arc::ptr_eq(&a, &b));
        let c = TableData::build_cached(make(0.32));
        assert!(!Arc::ptr_eq(&a, &c));
        // Region boundaries are part of the key.
        let one = vec![vec![sine(1), sine(1)]];
        let two = vec![vec![sine(1)], vec![sine(1)]];
        assert!(!Arc::ptr_eq(
            &TableData::build_cached(one),
            &TableData::build_cached(two)
        ));
        for i in 0..(CACHE_LIMIT + 4) {
            TableData::build_cached(make(0.5 + i as f32 * 0.01));
        }
        let len = cache().lock().map(|c| c.len()).unwrap_or(0);
        assert!(len <= CACHE_LIMIT);
        // The oldest build has been evicted, so it is rebuilt as a new table.
        assert!(!Arc::ptr_eq(&a, &TableData::build_cached(make(0.31))));
    }
}
