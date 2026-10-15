//! The oscillator node. It makes the sound: a waveform with unison, detune, and phase randomness.

use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};
use crate::dsp::noise::NoiseType;
use crate::dsp::warp::WarpMode;
use crate::dsp::wavetable::Waveform;

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

/// The warp dropdown's rows.
pub const WARP_LABELS: [&str; 8] = [
    "None", "Bend", "Sync", "PWM", "Asym", "Flip", "Mirror", "Quantize",
];

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
