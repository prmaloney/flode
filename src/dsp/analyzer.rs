//! A spectrum analyzer. It keeps the last 1024 samples of a mono signal, and every 1024 samples it
//! takes a windowed FFT of them and folds the result into 32 bands spaced evenly on a log axis from
//! 20 Hz to 20 kHz. Each band holds a level in dB, where 0 dB is a full scale sine. A band rises
//! at once and falls a little each update, so a peak lingers long enough to read.
//!
//! All of its memory is made in `new`, so `push` never allocates.

use std::f32::consts::PI;

/// How many bands the analyzer reports.
pub const BINS: usize = 32;
/// The level of a band with nothing in it, in dB.
pub const FLOOR_DB: f32 = -100.0;
/// The band range, in Hz.
pub const LOW_HZ: f32 = 20.0;
pub const HIGH_HZ: f32 = 20000.0;

/// Samples in each FFT.
const SIZE: usize = 1024;
/// Samples between FFTs.
const HOP: usize = 1024;
/// How far a band falls on each update, in dB.
const FALL_DB: f32 = 2.5;

pub struct Analyzer {
    ring: Vec<f32>,
    write: usize,
    since_update: usize,
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    /// `cos` and `sin` of each twiddle angle, and the bit-reversed index of each sample.
    cos: Vec<f32>,
    sin: Vec<f32>,
    reversed: Vec<u16>,
    /// The FFT bin position of the middle of each band, as a fraction of a bin.
    centers: Vec<f32>,
    /// The first and last whole bin each band covers, or None when a band is narrower than a bin
    /// and reads between two.
    spans: Vec<Option<(usize, usize)>>,
    bins: [f32; BINS],
}

impl Analyzer {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        let bits = SIZE.trailing_zeros();
        let per_bin = sample_rate / SIZE as f32;
        let edge = |i: f32| LOW_HZ * (HIGH_HZ / LOW_HZ).powf(i / BINS as f32);
        let centers: Vec<f32> = (0..BINS).map(|i| edge(i as f32 + 0.5) / per_bin).collect();
        let spans = (0..BINS)
            .map(|i| {
                let lo = (edge(i as f32) / per_bin).ceil() as usize;
                let hi = (edge(i as f32 + 1.0) / per_bin).floor() as usize;
                (lo <= hi && lo >= 1).then_some((lo, hi))
            })
            .collect();
        Self {
            ring: vec![0.0; SIZE],
            write: 0,
            since_update: 0,
            window: (0..SIZE)
                .map(|n| 0.5 - 0.5 * (2.0 * PI * n as f32 / SIZE as f32).cos())
                .collect(),
            re: vec![0.0; SIZE],
            im: vec![0.0; SIZE],
            cos: (0..SIZE / 2)
                .map(|k| (2.0 * PI * k as f32 / SIZE as f32).cos())
                .collect(),
            sin: (0..SIZE / 2)
                .map(|k| -(2.0 * PI * k as f32 / SIZE as f32).sin())
                .collect(),
            reversed: (0..SIZE)
                .map(|n| (n.reverse_bits() >> (usize::BITS - bits)) as u16)
                .collect(),
            centers,
            spans,
            bins: [FLOOR_DB; BINS],
        }
    }

    /// The level of each band in dB, from the lowest.
    pub fn bins(&self) -> &[f32; BINS] {
        &self.bins
    }

    /// Forgets the samples and drops every band to the floor.
    pub fn clear(&mut self) {
        self.ring.iter_mut().for_each(|x| *x = 0.0);
        self.write = 0;
        self.since_update = 0;
        self.bins = [FLOOR_DB; BINS];
    }

    /// Takes in one sample.
    pub fn push(&mut self, sample: f32) {
        self.ring[self.write] = if sample.is_finite() { sample } else { 0.0 };
        self.write = (self.write + 1) % SIZE;
        self.since_update += 1;
        if self.since_update >= HOP {
            self.since_update = 0;
            self.update();
        }
    }

    fn update(&mut self) {
        // The oldest sample is at `write`.
        for n in 0..SIZE {
            let at = self.reversed[n] as usize;
            let source = (self.write + n) % SIZE;
            self.re[at] = self.ring[source] * self.window[n];
            self.im[at] = 0.0;
        }
        let mut half = 1;
        while half < SIZE {
            let step = SIZE / (half * 2);
            for start in (0..SIZE).step_by(half * 2) {
                for k in 0..half {
                    let (c, s) = (self.cos[k * step], self.sin[k * step]);
                    let (a, b) = (start + k, start + k + half);
                    let tr = self.re[b] * c - self.im[b] * s;
                    let ti = self.re[b] * s + self.im[b] * c;
                    self.re[b] = self.re[a] - tr;
                    self.im[b] = self.im[a] - ti;
                    self.re[a] += tr;
                    self.im[a] += ti;
                }
            }
            half *= 2;
        }

        // A full scale sine through a Hann window peaks at SIZE / 4.
        let scale = 4.0 / SIZE as f32;
        let nyquist = SIZE / 2;
        let magnitude =
            |k: usize| (self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt() * scale;
        for band in 0..BINS {
            let level = match self.spans[band] {
                Some((lo, hi)) if lo < nyquist => (lo..=hi.min(nyquist - 1))
                    .map(magnitude)
                    .fold(0.0, f32::max),
                Some(_) => 0.0,
                None => {
                    // Narrower than a bin: read between the two bins around the middle.
                    let at = self.centers[band];
                    let lo = at.floor() as usize;
                    if lo + 1 >= nyquist {
                        0.0
                    } else {
                        let t = at - lo as f32;
                        magnitude(lo) * (1.0 - t) + magnitude(lo + 1) * t
                    }
                }
            };
            let db = 20.0 * level.max(1e-9).log10();
            self.bins[band] = db.max(self.bins[band] - FALL_DB).max(FLOOR_DB);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 44100.0;

    fn run(freq: f32, amplitude: f32, samples: usize) -> Analyzer {
        let mut analyzer = Analyzer::new(RATE);
        for n in 0..samples {
            analyzer.push(amplitude * (2.0 * PI * freq * n as f32 / RATE).sin());
        }
        analyzer
    }

    /// The band holding `freq`.
    fn band_of(freq: f32) -> usize {
        ((freq / LOW_HZ).ln() / (HIGH_HZ / LOW_HZ).ln() * BINS as f32) as usize
    }

    fn loudest(analyzer: &Analyzer) -> usize {
        let bins = analyzer.bins();
        (0..BINS)
            .max_by(|&a, &b| bins[a].partial_cmp(&bins[b]).unwrap())
            .unwrap()
    }

    #[test]
    fn silence_stays_at_the_floor() {
        let analyzer = run(1000.0, 0.0, 8192);
        assert!(analyzer.bins().iter().all(|&db| db <= FLOOR_DB + 1.0));
        assert_eq!(Analyzer::new(RATE).bins(), &[FLOOR_DB; BINS]);
    }

    /// A sine peaks in its own band, near 0 dB for full scale, and the bands far away are lower.
    #[test]
    fn a_sine_peaks_in_its_band() {
        for freq in [300.0, 1000.0, 5000.0, 12000.0] {
            let analyzer = run(freq, 1.0, 8192);
            let at = loudest(&analyzer);
            assert!(
                (at as i32 - band_of(freq) as i32).abs() <= 2,
                "{freq} Hz: band {at}, wanted {}",
                band_of(freq)
            );
            let top = analyzer.bins()[at];
            assert!(top > -6.0 && top < 0.5, "{freq} Hz: {top} dB");
            let far = analyzer.bins()[(band_of(freq) + BINS / 2) % BINS];
            assert!(far < top - 40.0, "{freq} Hz: {far} dB against {top} dB");
        }
    }

    /// Half the amplitude reads 6 dB lower.
    #[test]
    fn the_level_is_in_dbfs() {
        let loud = run(1000.0, 1.0, 8192);
        let quiet = run(1000.0, 0.5, 8192);
        let at = band_of(1000.0);
        let difference = loud.bins()[at] - quiet.bins()[at];
        assert!((difference - 6.0).abs() < 1.0, "{difference}");
    }

    /// A band falls after the signal stops, and does not jump to the floor.
    #[test]
    fn a_peak_falls_slowly() {
        let mut analyzer = run(1000.0, 1.0, 8192);
        let at = band_of(1000.0);
        let before = analyzer.bins()[at];
        for _ in 0..HOP {
            analyzer.push(0.0);
        }
        let after = analyzer.bins()[at];
        assert!(after < before && after > before - 10.0, "{before} {after}");
        for _ in 0..(HOP * 60) {
            analyzer.push(0.0);
        }
        assert!(analyzer.bins()[at] <= FLOOR_DB + 1.0);
    }

    #[test]
    fn clear_drops_everything_to_the_floor() {
        let mut analyzer = run(1000.0, 1.0, 8192);
        analyzer.clear();
        assert_eq!(analyzer.bins(), &[FLOOR_DB; BINS]);
    }

    /// Bad input does not poison the bands.
    #[test]
    fn non_finite_input_is_ignored() {
        let mut analyzer = Analyzer::new(RATE);
        for n in 0..4096 {
            analyzer.push(if n % 7 == 0 { f32::NAN } else { f32::INFINITY });
        }
        assert!(analyzer.bins().iter().all(|db| db.is_finite()));
    }

    /// At a low sample rate the bands above the top of the spectrum read the floor.
    #[test]
    fn bands_above_nyquist_read_the_floor() {
        let mut analyzer = Analyzer::new(22050.0);
        for n in 0..8192 {
            analyzer.push((2.0 * PI * 1000.0 * n as f32 / 22050.0).sin());
        }
        assert!(analyzer.bins()[BINS - 1] <= FLOOR_DB + 1.0);
        assert!(analyzer.bins()[band_of(1000.0)] > -6.0);
    }
}
