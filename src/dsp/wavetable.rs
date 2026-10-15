//! Waveform shapes and the band-limited tables the oscillators read.

use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum Waveform {
    #[default]
    Sine = 0,
    Saw = 1,
    Square = 2,
    Triangle = 3,
    /// Noise. It is not pitched, and its type comes from the noise dropdown.
    Noise = 4,
}

impl Waveform {
    /// Every waveform, in the order the dropdown lists them.
    pub const ALL: [Waveform; 5] = [
        Waveform::Sine,
        Waveform::Saw,
        Waveform::Square,
        Waveform::Triangle,
        Waveform::Noise,
    ];

    /// The dropdown row for this waveform.
    pub fn index(self) -> usize {
        self as usize
    }

    /// Stored as a u8 so the editor can publish the waveform to the audio thread atomically.
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// Unknown values fall back to sine.
    pub fn from_u8(value: u8) -> Self {
        Self::ALL.get(value as usize).copied().unwrap_or_default()
    }

    /// The shape of one cycle at `phase`, from 0.0 to 1.0, in the range -1.0 to 1.0. It has sharp
    /// edges for saw and square. Played directly, those alias at high pitches. Audio uses
    /// `WaveTables`, which drops the partials above Nyquist.
    pub fn naive(self, phase: f32) -> f32 {
        match self {
            Waveform::Sine => (phase * TAU).sin(),
            // Noise has no cycle. The LFO holds random values instead, so this is never read.
            Waveform::Noise => 0.0,
            Waveform::Saw => 2.0 * phase - 1.0,
            Waveform::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Waveform::Triangle => {
                if phase < 0.5 {
                    4.0 * phase - 1.0
                } else {
                    3.0 - 4.0 * phase
                }
            }
        }
    }
}

/// Samples in one cycle of a band-limited table.
const TABLE_SIZE: usize = 2048;
/// Tables for pitch ranges that double in step. Each one holds only the partials that stay below
/// Nyquist for the highest pitch in its range.
const TABLE_LEVELS: usize = 11;
/// The phase step per sample at the top of the lowest range. Its partial count is the most any
/// table needs, half the sample rate over this step.
const LOWEST_TOP_STEP: f32 = 1.0 / 2048.0;

/// Band-limited waveform tables. A table is one cycle built from the partials below Nyquist. The
/// audio thread picks the table for the note's pitch and reads it with linear interpolation.
/// Building the tables allocates, so the plugin builds them on the main thread.
pub struct WaveTables {
    /// Indexed by `(waveform index * TABLE_LEVELS) + level`. Sine has no table.
    tables: Vec<Vec<f32>>,
}

impl WaveTables {
    pub fn new() -> Self {
        let sin: Vec<f32> = (0..TABLE_SIZE)
            .map(|m| (TAU * m as f32 / TABLE_SIZE as f32).sin())
            .collect();
        let cos: Vec<f32> = (0..TABLE_SIZE)
            .map(|m| (TAU * m as f32 / TABLE_SIZE as f32).cos())
            .collect();
        let mut tables = Vec::new();
        for waveform in [Waveform::Saw, Waveform::Square, Waveform::Triangle] {
            let (a, b) = fourier(waveform, &sin, &cos);
            for level in 0..TABLE_LEVELS {
                let partials = max_partials(level);
                tables.push(build_table(&a, &b, partials, &sin, &cos));
            }
        }
        Self { tables }
    }

    /// The value of a waveform at `phase`, from 0.0 to 1.0. `step` is the phase advance per sample.
    #[inline]
    pub fn sample(&self, waveform: Waveform, phase: f32, step: f32) -> f32 {
        let index = match waveform {
            Waveform::Sine => return (phase * TAU).sin(),
            // Noise voices don't read a table. Their samples come from `NoiseState`.
            Waveform::Noise => return 0.0,
            Waveform::Saw => 0,
            Waveform::Square => 1,
            Waveform::Triangle => 2,
        };
        let table = &self.tables[index * TABLE_LEVELS + level_for(step.abs())];
        let position = phase * TABLE_SIZE as f32;
        let i = (position as usize) % TABLE_SIZE;
        let frac = position - position.floor();
        let next = (i + 1) % TABLE_SIZE;
        table[i] + (table[next] - table[i]) * frac
    }
}

impl Default for WaveTables {
    fn default() -> Self {
        Self::new()
    }
}

/// The table for a phase step. Each level covers twice the steps of the one below it.
fn level_for(step: f32) -> usize {
    let mut level = 0;
    let mut top = LOWEST_TOP_STEP;
    while step > top && level < TABLE_LEVELS - 1 {
        level += 1;
        top *= 2.0;
    }
    level
}

/// The most partials a table at this level keeps. Any partial past this one would pass Nyquist
/// at the level's top step.
fn max_partials(level: usize) -> usize {
    (0.5 / (LOWEST_TOP_STEP * (1 << level) as f32)) as usize
}

/// The sine and cosine coefficients of a waveform's partials, from the sampled shape. Each
/// partial is found by correlating one cycle with a sine and a cosine.
fn fourier(waveform: Waveform, sin: &[f32], cos: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let n = TABLE_SIZE;
    let shape: Vec<f32> = (0..n)
        .map(|i| waveform.naive(i as f32 / n as f32))
        .collect();
    let top = max_partials(0);
    let mut a = vec![0.0; top + 1];
    let mut b = vec![0.0; top + 1];
    for k in 1..=top {
        let (mut sum_cos, mut sum_sin) = (0.0f32, 0.0f32);
        for (i, &x) in shape.iter().enumerate() {
            let m = (k * i) % n;
            sum_cos += x * cos[m];
            sum_sin += x * sin[m];
        }
        a[k] = 2.0 * sum_cos / n as f32;
        b[k] = 2.0 * sum_sin / n as f32;
    }
    (a, b)
}

/// One cycle that keeps the first `partials` harmonics.
fn build_table(a: &[f32], b: &[f32], partials: usize, sin: &[f32], cos: &[f32]) -> Vec<f32> {
    let n = TABLE_SIZE;
    (0..n)
        .map(|i| {
            (1..=partials)
                .map(|k| {
                    let m = (k * i) % n;
                    a[k] * cos[m] + b[k] * sin[m]
                })
                .sum()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// At a high pitch the tables keep only the partials below Nyquist, so the peak stays near
    /// the naive shape and the average is zero.
    #[test]
    fn high_pitch_tables_stay_in_range_and_centred() {
        let tables = WaveTables::new();
        for waveform in [Waveform::Saw, Waveform::Square, Waveform::Triangle] {
            let step = 2000.0 / 44100.0;
            let n = (1.0 / step) as usize;
            let mut sum = 0.0;
            let mut peak = 0.0f32;
            for i in 0..n {
                let phase = (i as f32 * step).fract();
                let y = tables.sample(waveform, phase, step);
                sum += y;
                peak = peak.max(y.abs());
            }
            assert!(
                (sum / n as f32).abs() < 0.02,
                "{waveform:?} mean {}",
                sum / n as f32
            );
            assert!(peak < 1.15, "{waveform:?} peak {peak}");
        }
    }

    /// A high note uses fewer partials than a low note, so its table is smoother.
    #[test]
    fn higher_notes_use_higher_levels() {
        assert!(level_for(0.001) < level_for(0.01));
        assert!(level_for(0.01) < level_for(0.2));
        assert_eq!(level_for(0.9), TABLE_LEVELS - 1);
    }

    /// Below the lowest level's top, a table keeps every partial up to the Nyquist limit.
    #[test]
    fn the_lowest_level_keeps_the_most_partials() {
        assert_eq!(max_partials(0), 1024);
        assert_eq!(max_partials(TABLE_LEVELS - 1), 1);
    }
}
