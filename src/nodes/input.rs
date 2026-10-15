//! The input node. It sends MIDI as control signals: the note, its velocity, the pitch bend, the
//! mod wheel, and channel pressure. Each one is an output pin that can drive knobs and maps. It
//! makes no sound.

use super::{FlodeNode, NodeBehavior, Port, Row};
use serde::{Deserialize, Serialize};

pub struct Input;

/// The MIDI sources an input node sends. Each one is a control signal from 0.0 to 1.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MidiSource {
    /// The last note played, over the 0 to 127 MIDI range.
    Note,
    /// The velocity of the last note played.
    Velocity,
    /// The pitch wheel. 0.5 is centred.
    PitchBend,
    /// The mod wheel, CC 1.
    ModWheel,
    /// Channel pressure, from a keyboard's aftertouch.
    Aftertouch,
}

impl MidiSource {
    pub const ALL: [MidiSource; MIDI_SOURCE_COUNT] = [
        MidiSource::Note,
        MidiSource::Velocity,
        MidiSource::PitchBend,
        MidiSource::ModWheel,
        MidiSource::Aftertouch,
    ];

    /// The pin's name, shown beside it.
    pub fn label(self) -> &'static str {
        match self {
            MidiSource::Note => "Note",
            MidiSource::Velocity => "Velocity",
            MidiSource::PitchBend => "Pitch bend",
            MidiSource::ModWheel => "Mod wheel",
            MidiSource::Aftertouch => "Aftertouch",
        }
    }

    /// The slot this source takes in the audio thread's level table.
    pub fn index(self) -> usize {
        self as usize
    }

    /// The level this source starts at, before the host sends anything.
    pub fn default_level(self) -> f32 {
        match self {
            // Pitch bend rests in the middle of its range.
            MidiSource::PitchBend => 0.5,
            _ => 0.0,
        }
    }
}

/// How many MIDI sources there are.
pub const MIDI_SOURCE_COUNT: usize = 5;

impl NodeBehavior for Input {
    fn title(&self) -> &'static str {
        "Input"
    }

    /// One output pin for each source, top to bottom.
    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        MidiSource::ALL
            .iter()
            .map(|&source| Row::default().with_output(Port::Midi(source)))
            .collect()
    }
}
