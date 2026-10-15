//! The oscillator node. It makes the sound: a waveform with unison, detune, and phase randomness.

use super::{Bypass, Control, FlodeNode, KnobId, NodeBehavior, Port, Row, Section};
use crate::dsp::noise::NoiseType;
use crate::dsp::warp::WarpMode;
use crate::dsp::wavetable::Waveform;

pub struct Oscillator;

/// The numbers of the oscillator's sections, as `Row::section` and `sections` use them.
const SECTION_PITCH: usize = 0;
const SECTION_UNISON: usize = 1;
const SECTION_WARP: usize = 2;

/// The waveform dropdown's rows. The LFO uses the same list.
pub const WAVEFORM_LABELS: [&str; 5] = ["Sine", "Saw", "Square", "Triangle", "Noise"];

/// The noise dropdown's rows. It shows only under the noise waveform.
pub const NOISE_LABELS: [&str; 4] = ["White", "Pink", "Brown", "Geiger"];

impl NodeBehavior for Oscillator {
    fn title(&self) -> &'static str {
        "Oscillator"
    }

    /// The rows in the order they show. The waveform and noise dropdowns lead. The shape row has
    /// the envelope input and the audio output, so both pins stay in the same place whatever is
    /// folded. Level and pan follow, and the rest is in three sections that fold: pitch, unison,
    /// and warp. The warp dropdown sits directly above the warp amount.
    fn rows(&self, node: &FlodeNode) -> Vec<Row> {
        // The notes pin sits on the wave dropdown's row, at the very top. The dropdown starts
        // clear of the pin, so the row stays one dropdown tall. With no wire here the oscillator
        // is silent.
        let mut rows = vec![Row {
            input: Some(Port::Notes),
            ..Row::control(Control::Dropdown(0))
        }];
        if waveform(node) == Waveform::Noise {
            rows.push(Row::control(Control::Dropdown(2)));
        }
        // The envelope input shapes this oscillator's amplitude, one copy per voice.
        rows.push(Row::input(Port::Shape).with_output(Port::Audio));
        rows.push(Row::driven_knob(KnobId::OscGain));
        rows.push(Row::driven_knob(KnobId::Pan));

        // The transpose knobs shift the oscillator's pitch. A control wire can drive each one.
        rows.push(Row::section(SECTION_PITCH));
        rows.push(Row::driven_knob(KnobId::Coarse));
        rows.push(Row::driven_knob(KnobId::Fine));
        rows.push(Row::driven_knob(KnobId::Octave));

        rows.push(Row::section(SECTION_UNISON));
        rows.push(Row::driven_knob(KnobId::Unison));
        rows.push(Row::driven_knob(KnobId::Detune));
        rows.push(Row::driven_knob(KnobId::Spread));
        rows.push(Row::driven_knob(KnobId::PhaseRandom));

        rows.push(Row::section(SECTION_WARP));
        rows.push(Row::control(Control::Dropdown(1)));
        rows.push(Row::driven_knob(KnobId::WarpAmount));
        rows
    }

    /// Pitch and warp start folded, since they are touched less than the unison.
    fn sections(&self) -> &'static [Section] {
        &[
            Section {
                title: "Pitch",
                collapsed: true,
            },
            Section {
                title: "Unison",
                collapsed: false,
            },
            Section {
                title: "Warp",
                collapsed: true,
            },
        ]
    }

    fn audio_out(&self) -> bool {
        true
    }

    fn bypass(&self) -> Bypass {
        Bypass::Mute
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
    WarpMode::ALL
        .get(node.choice(1))
        .copied()
        .unwrap_or_default()
}

/// The warp dropdown's rows.
pub const WARP_LABELS: [&str; 8] = [
    "None", "Bend", "Sync", "PWM", "Asym", "Flip", "Mirror", "Quantize",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    fn oscillator() -> FlodeNode {
        FlodeNode::new(NodeKind::Oscillator, Point::new(0.0, 0.0))
    }

    /// The warp dropdown lists one row for every warp mode, in the same order.
    #[test]
    fn the_warp_labels_match_the_modes() {
        assert_eq!(WARP_LABELS.len(), WarpMode::ALL.len());
        assert!(!WARP_LABELS.contains(&"FM"));
    }

    /// An oscillator has no audio input under any warp. FM is a mix node mode.
    #[test]
    fn an_oscillator_has_no_audio_input() {
        let mut node = oscillator();
        for choice in 0..WarpMode::ALL.len() {
            node.choose(1, choice);
            assert!(!node.input_ports().iter().any(|port| port.is_audio()));
        }
    }

    /// A graph saved while the oscillator had an FM warp holds a choice past the last mode. It
    /// loads as no warp, not as whichever mode is last.
    #[test]
    fn a_saved_choice_past_the_last_warp_means_none() {
        let mut node = oscillator();
        node.choose(1, WarpMode::ALL.len());
        assert_eq!(warp_mode(&node), WarpMode::None);
    }
}
