//! The MIDI input node. Its notes pin feeds the oscillators, and the node holds the voice
//! settings: mono and glide. It also sends MIDI as control signals: the note, its velocity, the
//! pitch bend, the mod wheel, and channel pressure. Each one is an output pin that can drive knobs
//! and maps. It makes no sound.

use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row, Section};
use serde::{Deserialize, Serialize};

pub struct Input;

/// The MIDI sources an input node sends. Each one is a control signal from 0.0 to 1.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
        "MIDI Input"
    }

    /// The notes pin is first, and an oscillator plays what it is wired to. The voice settings
    /// follow: glide, mono, and glide on every note. The five controllers fold away under their
    /// own header, since their pins are all most patches use.
    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        let mut rows = vec![
            Row::default().with_output(Port::Notes),
            Row::driven_knob(KnobId::Glide),
            Row::control(Control::Switch(0)),
            Row::control(Control::Switch(1)),
            Row::section(0),
        ];
        rows.extend(
            MidiSource::ALL
                .iter()
                .map(|&source| Row::default().with_output(Port::Midi(source))),
        );
        rows
    }

    fn sections(&self) -> &'static [Section] {
        &[Section {
            title: "Controllers",
            collapsed: true,
        }]
    }

    fn switch_labels(&self) -> &'static [&'static str] {
        &["Mono", "Always"]
    }
}

/// True if the node plays one voice at a time.
pub fn mono(node: &FlodeNode) -> bool {
    node.switch(0)
}

/// True if the node glides on every mono note, not only while another note is still held.
pub fn glide_always(node: &FlodeNode) -> bool {
    node.switch(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    #[test]
    fn the_notes_pin_is_the_first_output() {
        let node = FlodeNode::new(NodeKind::Input, Point::new(0.0, 0.0));
        assert_eq!(node.output_ports()[0], Port::Notes);
        assert_eq!(node.output_ports().len(), 1 + MIDI_SOURCE_COUNT);
    }

    #[test]
    fn the_voice_settings_live_here() {
        let mut node = FlodeNode::new(NodeKind::Input, Point::new(0.0, 0.0));
        assert!(!mono(&node) && !glide_always(&node));
        node.set_switch(0, true);
        assert!(mono(&node));
        assert!(node
            .rows()
            .iter()
            .any(|row| row.knob == Some(KnobId::Glide)));
    }
}
