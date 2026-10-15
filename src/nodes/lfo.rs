//! The LFO node. It is a low-frequency oscillator. Its shape goes out on the shape pin as a control
//! signal, and it drives any knob a control wire reaches. It makes no sound.
//!
//! With Sync set to a note division, the LFO follows the host's playhead instead of its rate.

use super::oscillator::WAVEFORM_LABELS;
use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};
use crate::dsp::wavetable::Waveform;

pub struct Lfo;

/// The note divisions Sync offers, with how many cycles of the LFO fit in one beat. Sync step `n`
/// is `DIVISIONS[n - 1]`. Step zero is free running.
pub const DIVISIONS: [(&str, f32); 8] = [
    ("1/1", 0.25),
    ("1/2", 0.5),
    ("1/4", 1.0),
    ("1/8", 2.0),
    ("1/16", 4.0),
    ("1/4 dotted", 2.0 / 3.0),
    ("1/8 dotted", 4.0 / 3.0),
    ("1/8 triplet", 3.0),
];
pub const DIVISION_COUNT: usize = DIVISIONS.len();

/// The cycles per beat for a Sync step past zero.
pub fn cycles_per_beat(step: usize) -> f32 {
    DIVISIONS[(step.max(1) - 1).min(DIVISION_COUNT - 1)].1
}

impl NodeBehavior for Lfo {
    fn title(&self) -> &'static str {
        "LFO"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::driven_knob(KnobId::LfoRate).with_output(Port::Shape),
            Row::plain_knob(KnobId::LfoSync),
            Row::control(Control::Dropdown(0)),
            Row::control(Control::Switch(0)),
        ]
    }

    fn switch_labels(&self) -> &'static [&'static str] {
        &["Retrigger"]
    }

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&WAVEFORM_LABELS]
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Wave"]
    }
}

/// The waveform an LFO node runs.
pub fn waveform(node: &FlodeNode) -> Waveform {
    Waveform::ALL[node.choice(0).min(Waveform::ALL.len() - 1)]
}
