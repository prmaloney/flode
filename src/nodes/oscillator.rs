//! The oscillator node. It makes the sound: a waveform with unison, detune, and phase randomness.

use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

pub struct Oscillator;

/// The waveform dropdown's rows. The LFO uses the same list.
pub const WAVEFORM_LABELS: [&str; 5] = ["Sine", "Saw", "Square", "Triangle", "Noise"];

/// The noise dropdown's rows. It shows only under the noise waveform.
pub const NOISE_LABELS: [&str; 4] = ["White", "Pink", "Brown", "Geiger"];

impl NodeBehavior for Oscillator {
    fn title(&self) -> &'static str {
        "Oscillator"
    }

    /// The rows in the order they show. The waveform and noise dropdowns sit together, and the warp
    /// dropdown sits directly above the warp amount.
    fn rows(&self, node: &FlodeNode) -> Vec<Row> {
        // The waveform leads, with the noise type under it. The warp dropdown sits right above the
        // warp amount.
        let mut rows = vec![Row::control(Control::Dropdown(0))];
        if waveform(node) == Waveform::Noise {
            rows.push(Row::control(Control::Dropdown(2)));
        }
        // The envelope input shapes this oscillator's amplitude, one copy per voice.
        rows.push(Row::input(Port::Shape));
        // The transpose knobs shift the oscillator's pitch. A control wire can drive each one.
        rows.push(Row::driven_knob(KnobId::Coarse));
        rows.push(Row::driven_knob(KnobId::Fine));
        rows.push(Row::driven_knob(KnobId::Octave));
        rows.push(Row::driven_knob(KnobId::Unison).with_output(Port::Audio));
        rows.push(Row::driven_knob(KnobId::Detune));
        rows.push(Row::driven_knob(KnobId::PhaseRandom));
        rows.push(Row::control(Control::Dropdown(1)));
        rows.push(Row::driven_knob(KnobId::WarpAmount));
        rows.push(Row::driven_knob(KnobId::OscGain));
        rows
    }

    fn audio_out(&self) -> bool {
        true
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Wave", "Warp", "Noise"]
    }

    /// The waveform, the warp mode, and under the noise waveform, the noise type.
    fn dropdowns(&self, node: &FlodeNode) -> Vec<&'static [&'static str]> {
        let mut dropdowns: Vec<&'static [&'static str]> = vec![&WAVEFORM_LABELS, &WARP_LABELS];
        if waveform(node) == Waveform::Noise {
            dropdowns.push(&NOISE_LABELS);
        }
        dropdowns
    }
}

/// The frequency ratio for the transpose knobs. Each octave is twelve semitones, each semitone is a
/// hundred cents, and a ratio of 2.0 is up an octave.
pub fn transpose_ratio(octave: f32, coarse: f32, fine: f32) -> f32 {
    let semitones = octave * 12.0 + coarse + fine / 100.0;
    (semitones / 12.0).exp2()
}

/// The waveform an oscillator node plays.
pub fn waveform(node: &FlodeNode) -> Waveform {
    Waveform::ALL[node.choice(0).min(Waveform::ALL.len() - 1)]
}

/// The noise type an oscillator node plays under the noise waveform.
pub fn noise_type(node: &FlodeNode) -> NoiseType {
    NoiseType::ALL[node.choice(2).min(NoiseType::ALL.len() - 1)]
}

/// The warp mode an oscillator node applies.
pub fn warp_mode(node: &FlodeNode) -> WarpMode {
    WarpMode::ALL[node.choice(1).min(WarpMode::ALL.len() - 1)]
}

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

    /// Stored as a u8 so the editor can publish the waveform to the audio thread atomically.
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// Unknown values fall back to sine.
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Waveform::Saw,
            2 => Waveform::Square,
            3 => Waveform::Triangle,
            4 => Waveform::Noise,
            _ => Waveform::Sine,
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

/// The kinds of noise. Each one is a different spectrum, from flat to dark, or a crackle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum NoiseType {
    /// Equal energy at every frequency.
    #[default]
    White = 0,
    /// Equal energy per octave, so it sounds even across the range.
    Pink = 1,
    /// Random walk, which sounds dark and rumbling.
    Brown = 2,
    /// Random clicks that decay quickly, like a counter.
    Geiger = 3,
}

impl NoiseType {
    pub const ALL: [NoiseType; 4] = [
        NoiseType::White,
        NoiseType::Pink,
        NoiseType::Brown,
        NoiseType::Geiger,
    ];

    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// Unknown values fall back to white.
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => NoiseType::Pink,
            2 => NoiseType::Brown,
            3 => NoiseType::Geiger,
            _ => NoiseType::White,
        }
    }
}

/// A random number from 0.0 up to 1.0, from a xorshift generator that the state advances.
pub fn random_unit(state: &mut u32) -> f32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x as f32 / 4_294_967_296.0
}

/// What one voice's noise remembers between samples. Each voice has its own, so voices don't
/// share a random walk or a click.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoiseState {
    /// The pink filter's stages.
    pink: [f32; 7],
    /// The brown walk's position.
    brown: f32,
    /// The current click's level, which decays.
    click: f32,
}

impl NoiseState {
    /// The next sample of a noise, from -1.0 to 1.0.
    pub fn sample(&mut self, kind: NoiseType, rng: &mut u32) -> f32 {
        let white = random_unit(rng) * 2.0 - 1.0;
        match kind {
            NoiseType::White => white,
            // Paul Kellet's refinement of pink noise: white through seven leaky stages.
            NoiseType::Pink => {
                let p = &mut self.pink;
                p[0] = 0.99886 * p[0] + white * 0.055_517_9;
                p[1] = 0.99332 * p[1] + white * 0.075_075_9;
                p[2] = 0.969_00 * p[2] + white * 0.153_852_0;
                p[3] = 0.866_50 * p[3] + white * 0.310_485_6;
                p[4] = 0.550_00 * p[4] + white * 0.532_952_2;
                p[5] = -0.761_6 * p[5] - white * 0.016_898_0;
                let sum: f32 = p.iter().sum::<f32>() + white * 0.5362;
                p[6] = white * 0.115_926;
                sum * 0.11
            }
            // A leaky random walk. It drifts slowly, so it stays between the ends.
            NoiseType::Brown => {
                self.brown = (self.brown + 0.02 * white) / 1.02;
                (self.brown * 3.5).clamp(-1.0, 1.0)
            }
            // About 60 clicks a second at 44.1 kHz. Each one jumps to a random level and decays.
            NoiseType::Geiger => {
                if random_unit(rng) < 0.0014 {
                    self.click = white;
                }
                self.click *= 0.996;
                self.click
            }
        }
    }
}

/// The warp modes. Each one bends the waveform's shape, and the warp amount sets how far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum WarpMode {
    #[default]
    None = 0,
    /// Crowds the cycle toward one end, so the waveform bends.
    Bend = 1,
    /// Restarts the cycle partway through, as a second oscillator in sync would.
    Sync = 2,
    /// Moves the point where the cycle switches halves, which widens or narrows a pulse.
    Pwm = 3,
    /// Stretches one half of the cycle and squeezes the other.
    Asym = 4,
    /// Plays the waveform backwards.
    Flip = 5,
    /// Folds the second half of the cycle back over the first.
    Mirror = 6,
    /// Rounds the amplitude to a few steps.
    Quantize = 7,
}

/// The warp dropdown's rows.
pub const WARP_LABELS: [&str; 8] = [
    "None", "Bend", "Sync", "PWM", "Asym", "Flip", "Mirror", "Quantize",
];

impl WarpMode {
    pub const ALL: [WarpMode; 8] = [
        WarpMode::None,
        WarpMode::Bend,
        WarpMode::Sync,
        WarpMode::Pwm,
        WarpMode::Asym,
        WarpMode::Flip,
        WarpMode::Mirror,
        WarpMode::Quantize,
    ];

    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// Unknown values fall back to no warp.
    pub fn from_u8(value: u8) -> Self {
        Self::ALL.get(value as usize).copied().unwrap_or_default()
    }
}

/// Moves a phase, from 0.0 to 1.0, through a warp. `amount` runs from 0.0, which leaves the phase
/// alone, to 1.0, the full warp. Quantize doesn't move the phase.
pub fn warp_phase(mode: WarpMode, phase: f32, amount: f32) -> f32 {
    let a = amount.clamp(0.0, 1.0);
    let warped = match mode {
        WarpMode::None | WarpMode::Quantize => return phase,
        // A power curve on the phase, which crowds it toward one end.
        WarpMode::Bend => phase.powf(2.0f32.powf(-(a * 2.0 - 1.0) * 2.0)),
        // The cycle restarts at a higher rate, so later cycles repeat sooner.
        WarpMode::Sync => (phase * (1.0 + a * 7.0)).fract(),
        // The split point moves from a tenth to nine tenths of the cycle, and each side is
        // stretched to fill half the output.
        WarpMode::Pwm | WarpMode::Asym => {
            let split = 0.5 + 0.4 * (2.0 * a - 1.0);
            if phase < split {
                phase * 0.5 / split
            } else {
                0.5 + (phase - split) * 0.5 / (1.0 - split)
            }
        }
        WarpMode::Flip => 1.0 - phase,
        WarpMode::Mirror => {
            if phase < 0.5 {
                phase * 2.0
            } else {
                2.0 - phase * 2.0
            }
        }
    };
    // Blend between the plain phase and the warped one, so the amount sets the strength.
    (phase + (warped - phase) * a).rem_euclid(1.0)
}

/// Rounds a value, from -1.0 to 1.0, to a few steps when the warp is quantize. The amount blends
/// the rounded value with the original, and at zero the value is untouched.
pub fn warp_value(mode: WarpMode, value: f32, amount: f32) -> f32 {
    if mode != WarpMode::Quantize {
        return value;
    }
    let a = amount.clamp(0.0, 1.0);
    // From 64 steps down to 2 as the amount rises.
    let steps = 64.0 - a * 62.0;
    let rounded = (value * steps * 0.5).round() / (steps * 0.5);
    value + (rounded - value) * a
}

#[cfg(test)]
mod warp_tests {
    use super::*;

    /// With no amount, every warp leaves the phase alone.
    #[test]
    fn zero_amount_leaves_the_phase_alone() {
        for mode in WarpMode::ALL {
            assert!((warp_phase(mode, 0.3, 0.0) - 0.3).abs() < 1e-6, "{mode:?}");
        }
    }

    /// Flip plays the cycle backwards at full amount, and mirror folds it.
    #[test]
    fn flip_and_mirror_reshape_the_cycle() {
        assert!((warp_phase(WarpMode::Flip, 0.25, 1.0) - 0.75).abs() < 1e-6);
        assert!((warp_phase(WarpMode::Mirror, 0.25, 1.0) - 0.5).abs() < 1e-6);
    }

    /// Quantize leaves a value alone at zero amount, and rounds it at full amount.
    #[test]
    fn quantize_rounds_only_when_it_is_on() {
        assert_eq!(warp_value(WarpMode::Quantize, 0.37, 0.0), 0.37);
        let rounded = warp_value(WarpMode::Quantize, 0.37, 1.0);
        assert!((rounded * 16.0 - (rounded * 16.0).round()).abs() < 1e-3);
    }
}

#[cfg(test)]
mod transpose_tests {
    use super::*;

    /// Zero on every knob is unity, each octave doubles, and twelve semitones make an octave.
    #[test]
    fn transpose_steps_map_to_ratios() {
        assert_eq!(transpose_ratio(0.0, 0.0, 0.0), 1.0);
        assert!((transpose_ratio(1.0, 0.0, 0.0) - 2.0).abs() < 1e-6);
        assert!((transpose_ratio(0.0, 12.0, 0.0) - 2.0).abs() < 1e-6);
        assert!((transpose_ratio(0.0, 0.0, -100.0) - transpose_ratio(0.0, -1.0, 0.0)).abs() < 1e-6);
    }
}
