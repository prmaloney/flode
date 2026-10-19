//! The data a monitor node measures, and the modes it measures it in. The audio thread fills a
//! `MonitorData` for each monitor instance and the editor draws it. This module holds the types
//! both sides share. The measuring itself is in `Monitor`, below them.

use super::analyzer::{Analyzer, BINS, FLOOR_DB};
use super::frame::Frame;

/// How many points the scope and the vectorscope report.
pub const TRACE_POINTS: usize = 256;

/// What a monitor node shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MonitorMode {
    /// The waveform, held steady by a trigger on a rising zero crossing.
    #[default]
    Scope,
    /// The level of each band now, as a curve.
    Spectrum,
    /// The level of each band over time, as a scrolling picture.
    Spectrogram,
    /// Peak and RMS for each channel, with a history strip.
    Levels,
    /// The left channel against the right, with a correlation reading.
    Vectorscope,
}

impl MonitorMode {
    pub const ALL: [MonitorMode; 5] = [
        MonitorMode::Scope,
        MonitorMode::Spectrum,
        MonitorMode::Spectrogram,
        MonitorMode::Levels,
        MonitorMode::Vectorscope,
    ];

    /// The dropdown labels, in the order of `ALL`.
    pub const LABELS: [&'static str; 5] =
        ["Scope", "Spectrum", "Spectrogram", "Levels", "Vectorscope"];
}

/// One monitor instance's latest measurement. Each mode fills only the part it needs, so the rest
/// holds what it held before, or its starting value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorData {
    /// Scope: the left and right sample at each point, from -1.0 to 1.0, over a window of about
    /// two periods of the signal. Vectorscope: the left and right sample of each point, taken at
    /// even steps over the last few thousand samples.
    pub trace: [[f32; 2]; TRACE_POINTS],
    /// Spectrum and spectrogram: the level of each analyzer band in dB, at or above `FLOOR_DB`.
    pub spectrum: [f32; BINS],
    /// Levels: the peak left, peak right, RMS left, and RMS right, in dB at or above `FLOOR_DB`.
    /// The peak holds for a moment and falls, and the RMS is smoothed.
    pub levels: [f32; 4],
    /// Vectorscope: how alike the channels are, from -1.0 (opposite) through 0.0 (unrelated) to
    /// 1.0 (the same). Smoothed.
    pub correlation: f32,
}

impl Default for MonitorData {
    fn default() -> Self {
        Self {
            trace: [[0.0; 2]; TRACE_POINTS],
            spectrum: [FLOOR_DB; BINS],
            levels: [FLOOR_DB; 4],
            correlation: 0.0,
        }
    }
}

/// Samples the scope and the vectorscope keep. A power of two, so the ring wraps with a mask.
const RING: usize = 4096;
const MASK: usize = RING - 1;
/// Samples between scope and vectorscope updates.
const HOP: usize = 2048;
/// Samples between refreshes of the levels.
const LEVELS_HOP: usize = 256;
/// Samples between reads of the analyzer's bands. It makes a new spectrum every 1024.
const SPECTRUM_HOP: usize = 1024;
/// The scope window when the signal has no period to find, and its limits when it has.
const SCOPE_DEFAULT: usize = 1024;
const SCOPE_MIN: usize = 128;
const SCOPE_MAX: usize = 2048;
/// How long a peak holds, then how fast it falls.
const PEAK_HOLD_SECONDS: f32 = 0.5;
const PEAK_FALL_DB_PER_SECOND: f32 = 20.0;
/// The time constant of the RMS reading.
const RMS_SECONDS: f32 = 0.3;
/// The share of the gap the correlation closes on each update.
const CORRELATION_SMOOTH: f32 = 0.4;

/// One monitor instance. It measures the frames it is given, in the mode it is told, and keeps
/// the latest result in a `MonitorData`. It does only the work its current mode needs, and every
/// buffer is made in `new`, so `process` never allocates.
pub struct Monitor {
    mode: MonitorMode,
    data: MonitorData,
    /// The last `RING` frames. `write` is where the next one goes, which is also the oldest.
    ring: Vec<[f32; 2]>,
    write: usize,
    /// Samples since the last scope or vectorscope update, or since the last spectrum read.
    since: usize,
    analyzer: Analyzer,
    /// Levels state: held peak, samples left in each hold, and mean square, for each side.
    peak: [f32; 2],
    hold: [u32; 2],
    mean_square: [f32; 2],
    hold_samples: u32,
    fall: f32,
    rms_coef: f32,
}

impl Monitor {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        Self {
            mode: MonitorMode::default(),
            data: MonitorData::default(),
            ring: vec![[0.0; 2]; RING],
            write: 0,
            since: 0,
            analyzer: Analyzer::new(sample_rate),
            peak: [0.0; 2],
            hold: [0; 2],
            mean_square: [0.0; 2],
            hold_samples: (PEAK_HOLD_SECONDS * sample_rate) as u32,
            fall: (-std::f32::consts::LN_10 * PEAK_FALL_DB_PER_SECOND / 20.0 / sample_rate).exp(),
            rms_coef: 1.0 - (-1.0 / (RMS_SECONDS * sample_rate)).exp(),
        }
    }

    /// The latest measurement.
    pub fn data(&self) -> &MonitorData {
        &self.data
    }

    /// Forgets every sample and returns the data to its starting values.
    pub fn reset(&mut self) {
        self.data = MonitorData::default();
        self.clear_state();
    }

    fn clear_state(&mut self) {
        self.ring.iter_mut().for_each(|frame| *frame = [0.0; 2]);
        self.write = 0;
        self.since = 0;
        self.analyzer.clear();
        self.peak = [0.0; 2];
        self.hold = [0; 2];
        self.mean_square = [0.0; 2];
    }

    /// Switches mode. The new mode starts from nothing, and shows its starting values until its
    /// first update. The spectrum and the spectrogram share an analyzer, so moving between them
    /// keeps it.
    fn start_mode(&mut self, mode: MonitorMode) {
        use MonitorMode::{Spectrogram, Spectrum};
        let spectral = |m| matches!(m, Spectrum | Spectrogram);
        if !(spectral(mode) && spectral(self.mode)) {
            self.clear_state();
            match mode {
                MonitorMode::Scope => self.data.trace = [[0.0; 2]; TRACE_POINTS],
                MonitorMode::Spectrum | MonitorMode::Spectrogram => {
                    self.data.spectrum = [FLOOR_DB; BINS]
                }
                MonitorMode::Levels => self.data.levels = [FLOOR_DB; 4],
                MonitorMode::Vectorscope => {
                    self.data.trace = [[0.0; 2]; TRACE_POINTS];
                    self.data.correlation = 0.0;
                }
            }
        }
        self.mode = mode;
    }

    /// Takes in one frame and measures it in `mode`.
    pub fn process(&mut self, frame: Frame, mode: MonitorMode) {
        if mode != self.mode {
            self.start_mode(mode);
        }
        let l = if frame.l.is_finite() { frame.l } else { 0.0 };
        let r = if frame.r.is_finite() { frame.r } else { 0.0 };
        match mode {
            MonitorMode::Scope | MonitorMode::Vectorscope => {
                self.ring[self.write] = [l, r];
                self.write = (self.write + 1) & MASK;
                self.since += 1;
                if self.since >= HOP {
                    self.since = 0;
                    if mode == MonitorMode::Scope {
                        self.update_scope();
                    } else {
                        self.update_vectorscope();
                    }
                }
            }
            MonitorMode::Spectrum | MonitorMode::Spectrogram => {
                self.analyzer.push((l + r) * 0.5);
                self.since += 1;
                if self.since >= SPECTRUM_HOP {
                    self.since = 0;
                    self.data.spectrum = *self.analyzer.bins();
                }
            }
            MonitorMode::Levels => self.measure_levels([l, r]),
        }
    }

    /// The ring's frame `i` steps after the oldest.
    fn at(&self, i: usize) -> [f32; 2] {
        self.ring[(self.write + i) & MASK]
    }

    /// Finds a rising zero crossing of the summed channels in the first half of the ring, and the
    /// next one, and shows two periods from the first.
    fn update_scope(&mut self) {
        let rising = |this: &Self, i: usize| {
            let (a, b) = (this.at(i - 1), this.at(i));
            a[0] + a[1] < 0.0 && b[0] + b[1] >= 0.0
        };
        let first = (1..RING / 2).find(|&i| rising(self, i));
        let (start, span) = match first {
            Some(first) => {
                let second = (first + 1..RING).find(|&i| rising(self, i));
                let span = second.map_or(SCOPE_DEFAULT, |second| {
                    (2 * (second - first)).clamp(SCOPE_MIN, SCOPE_MAX)
                });
                (first, span)
            }
            None => (0, SCOPE_DEFAULT),
        };
        for point in 0..TRACE_POINTS {
            let frame = self.at(start + point * span / TRACE_POINTS);
            self.data.trace[point] = [frame[0].clamp(-1.0, 1.0), frame[1].clamp(-1.0, 1.0)];
        }
    }

    /// Takes points at even steps over the last `HOP` frames, and reads how alike the channels are
    /// over all of them.
    fn update_vectorscope(&mut self) {
        let oldest = RING - HOP;
        let step = HOP / TRACE_POINTS;
        for point in 0..TRACE_POINTS {
            let frame = self.at(oldest + point * step);
            self.data.trace[point] = [frame[0].clamp(-1.0, 1.0), frame[1].clamp(-1.0, 1.0)];
        }
        let (mut ll, mut rr, mut lr) = (0.0f32, 0.0f32, 0.0f32);
        for i in oldest..RING {
            let [l, r] = self.at(i);
            ll += l * l;
            rr += r * r;
            lr += l * r;
        }
        let power = ll * rr;
        let now = if power > 1e-12 {
            (lr / power.sqrt()).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        self.data.correlation += (now - self.data.correlation) * CORRELATION_SMOOTH;
    }

    fn measure_levels(&mut self, frame: [f32; 2]) {
        for side in 0..2 {
            let x = frame[side];
            let magnitude = x.abs();
            if magnitude >= self.peak[side] {
                self.peak[side] = magnitude;
                self.hold[side] = self.hold_samples;
            } else if self.hold[side] > 0 {
                self.hold[side] -= 1;
            } else {
                self.peak[side] *= self.fall;
            }
            self.mean_square[side] += (x * x - self.mean_square[side]) * self.rms_coef;
        }
        self.since += 1;
        if self.since >= LEVELS_HOP {
            self.since = 0;
            for side in 0..2 {
                // Flush values too small to read before they turn into denormals.
                if self.peak[side] < 1e-7 {
                    self.peak[side] = 0.0;
                }
                if self.mean_square[side] < 1e-14 {
                    self.mean_square[side] = 0.0;
                }
                self.data.levels[side] = (20.0 * self.peak[side].log10()).max(FLOOR_DB);
                self.data.levels[2 + side] = (10.0 * self.mean_square[side].log10()).max(FLOOR_DB);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    const RATE: f32 = 44100.0;

    /// Runs `seconds` of a sine through a monitor. `right` is the right channel's gain, so 1.0 is
    /// mono and -1.0 is opposite.
    fn run_sine(mode: MonitorMode, hz: f32, amp: f32, right: f32, seconds: f32) -> Monitor {
        let mut monitor = Monitor::new(RATE);
        for n in 0..(seconds * RATE) as usize {
            let x = amp * (TAU * hz * n as f32 / RATE).sin();
            monitor.process(Frame::new(x, x * right), mode);
        }
        monitor
    }

    #[test]
    fn the_scope_starts_at_a_rising_crossing_and_shows_two_periods() {
        let mut monitor = Monitor::new(RATE);
        for n in 0..20_000 {
            let x = 0.8 * (TAU * 440.0 * n as f32 / RATE).sin();
            monitor.process(Frame::mono(x), MonitorMode::Scope);
            // Check each update as it lands, to see the trace hold steady.
            if (n + 1) % HOP == 0 && n > 8000 {
                let trace = &monitor.data().trace;
                assert!(
                    trace[0][0] >= 0.0 && trace[0][0] < 0.1,
                    "start {}",
                    trace[0][0]
                );
                assert!(trace[4][0] > trace[0][0]);
                // Two periods of 440 Hz fill the window, so the middle is the next crossing.
                assert!(trace[128][0].abs() < 0.1, "middle {}", trace[128][0]);
                assert!(trace[32][0] > 0.75 && trace[96][0] < -0.75);
                assert_eq!(trace[5][0], trace[5][1]);
            }
        }
    }

    #[test]
    fn a_silent_scope_is_zeros() {
        let monitor = run_sine(MonitorMode::Scope, 440.0, 0.0, 1.0, 0.3);
        assert!(monitor.data().trace.iter().all(|p| *p == [0.0, 0.0]));
    }

    #[test]
    fn a_sine_peaks_in_the_right_band() {
        for mode in [MonitorMode::Spectrum, MonitorMode::Spectrogram] {
            let monitor = run_sine(mode, 1000.0, 1.0, 1.0, 0.5);
            let spectrum = &monitor.data().spectrum;
            let top = (0..BINS)
                .max_by(|&a, &b| spectrum[a].total_cmp(&spectrum[b]))
                .unwrap();
            // 1 kHz is band 18 on 32 bands from 20 Hz to 20 kHz.
            assert!((17..=19).contains(&top), "band {top}");
            assert!(spectrum[top] > -6.0, "level {}", spectrum[top]);
        }
    }

    #[test]
    fn a_silent_spectrum_is_the_floor() {
        let monitor = run_sine(MonitorMode::Spectrum, 1000.0, 0.0, 1.0, 0.2);
        assert!(monitor.data().spectrum.iter().all(|&db| db == FLOOR_DB));
    }

    #[test]
    fn a_half_scale_sine_reads_six_db_down() {
        let monitor = run_sine(MonitorMode::Levels, 440.0, 0.5, 1.0, 1.0);
        let levels = monitor.data().levels;
        assert!((levels[0] + 6.02).abs() < 0.5, "peak {}", levels[0]);
        assert!((levels[1] + 6.02).abs() < 0.5, "peak {}", levels[1]);
        // The RMS of a sine is 3 dB under its peak.
        assert!((levels[2] + 9.03).abs() < 0.8, "rms {}", levels[2]);
    }

    #[test]
    fn silent_levels_are_the_floor() {
        let monitor = run_sine(MonitorMode::Levels, 440.0, 0.0, 1.0, 0.2);
        assert_eq!(monitor.data().levels, [FLOOR_DB; 4]);
    }

    #[test]
    fn a_peak_holds_then_falls() {
        let mut monitor = Monitor::new(RATE);
        for _ in 0..4410 {
            monitor.process(Frame::mono(1.0), MonitorMode::Levels);
        }
        let loud = monitor.data().levels[0];
        let mut quiet = |seconds: f32| {
            for _ in 0..(seconds * RATE) as usize {
                monitor.process(Frame::SILENT, MonitorMode::Levels);
            }
            monitor.data().levels[0]
        };
        // Inside the hold it has not moved.
        assert!((quiet(0.3) - loud).abs() < 0.1);
        // Two seconds on it has fallen by well over 20 dB.
        assert!(quiet(1.7) < loud - 20.0);
    }

    #[test]
    fn the_vectorscope_reads_the_correlation() {
        let same = run_sine(MonitorMode::Vectorscope, 440.0, 0.7, 1.0, 0.5);
        assert!(
            same.data().correlation > 0.95,
            "{}",
            same.data().correlation
        );
        assert!(same.data().trace.iter().all(|p| p[0] == p[1]));
        let opposite = run_sine(MonitorMode::Vectorscope, 440.0, 0.7, -1.0, 0.5);
        assert!(opposite.data().correlation < -0.95);
        let silent = run_sine(MonitorMode::Vectorscope, 440.0, 0.0, 1.0, 0.5);
        assert_eq!(silent.data().correlation, 0.0);
        assert!(silent.data().trace.iter().all(|p| *p == [0.0, 0.0]));
    }

    #[test]
    fn a_mode_change_starts_clean_and_reset_clears_everything() {
        let mut monitor = run_sine(MonitorMode::Levels, 440.0, 0.5, 1.0, 0.5);
        assert!(monitor.data().levels[0] > FLOOR_DB);
        monitor.process(Frame::SILENT, MonitorMode::Scope);
        // Back in the levels mode nothing is held over from before.
        for _ in 0..LEVELS_HOP {
            monitor.process(Frame::SILENT, MonitorMode::Levels);
        }
        assert_eq!(monitor.data().levels, [FLOOR_DB; 4]);
        monitor.reset();
        assert_eq!(*monitor.data(), MonitorData::default());
    }

    #[test]
    fn a_bad_sample_does_not_poison_the_measurement() {
        let mut monitor = Monitor::new(RATE);
        for mode in MonitorMode::ALL {
            for _ in 0..5000 {
                monitor.process(Frame::new(f32::NAN, f32::INFINITY), mode);
            }
        }
        assert!(monitor.data().levels.iter().all(|v| v.is_finite()));
        assert!(monitor.data().correlation.is_finite());
    }
}
