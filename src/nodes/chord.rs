//! The chord node. It sits between a MIDI input and an oscillator, the same spot an arpeggiator
//! sits in: it reads the notes held at its Notes input and turns each one into a stack of notes on
//! its Notes output, the root plus the chosen chord's intervals. Unlike the arpeggiator it runs no
//! clock: it reacts to a note arriving or letting go, rather than stepping through a pattern. It
//! makes no sound by itself.

use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};

pub struct Chord;

/// A chord shape: semitone offsets from the root. The root itself is always offset 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordType {
    Major,
    Minor,
    Major7,
    Minor7,
    Dominant7,
    Sus2,
    Sus4,
    Power,
    Octave,
}

/// The most notes any one chord type lists, root included.
pub const MAX_CHORD_NOTES: usize = 4;

impl ChordType {
    pub const ALL: [ChordType; 9] = [
        ChordType::Major,
        ChordType::Minor,
        ChordType::Major7,
        ChordType::Minor7,
        ChordType::Dominant7,
        ChordType::Sus2,
        ChordType::Sus4,
        ChordType::Power,
        ChordType::Octave,
    ];

    pub const LABELS: [&'static str; 9] = [
        "Major",
        "Minor",
        "Major 7",
        "Minor 7",
        "Dominant 7",
        "Sus2",
        "Sus4",
        "Power",
        "Octave",
    ];

    /// The semitone offsets from the root this chord plays, root first at offset 0.
    pub fn intervals(self) -> &'static [u8] {
        match self {
            ChordType::Major => &[0, 4, 7],
            ChordType::Minor => &[0, 3, 7],
            ChordType::Major7 => &[0, 4, 7, 11],
            ChordType::Minor7 => &[0, 3, 7, 10],
            ChordType::Dominant7 => &[0, 4, 7, 10],
            ChordType::Sus2 => &[0, 2, 7],
            ChordType::Sus4 => &[0, 5, 7],
            ChordType::Power => &[0, 7],
            ChordType::Octave => &[0, 12],
        }
    }
}

impl NodeBehavior for Chord {
    fn title(&self) -> &'static str {
        "Chord"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::input(Port::Notes).with_output(Port::Notes),
            Row::plain_knob(KnobId::ChordSpread),
            Row::control(Control::Dropdown(0)),
        ]
    }

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&ChordType::LABELS]
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Chord"]
    }
}

/// The chord type the node's dropdown chooses.
pub fn chord_type(node: &FlodeNode) -> ChordType {
    ChordType::ALL[node.choice(0).min(ChordType::ALL.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    #[test]
    fn the_notes_pin_is_the_first_row_both_ways() {
        let node = FlodeNode::new(NodeKind::Chord, Point::new(0.0, 0.0));
        assert_eq!(node.input_ports()[0], Port::Notes);
        assert_eq!(node.output_ports()[0], Port::Notes);
    }

    #[test]
    fn the_chord_dropdown_starts_at_major() {
        let node = FlodeNode::new(NodeKind::Chord, Point::new(0.0, 0.0));
        assert_eq!(chord_type(&node), ChordType::Major);
    }

    /// Every chord type lists the root at offset 0, and its own notes besides.
    #[test]
    fn every_chord_type_includes_the_root() {
        for kind in ChordType::ALL {
            assert_eq!(kind.intervals()[0], 0, "{kind:?}");
        }
    }

    #[test]
    fn the_chord_intervals_are_correct() {
        assert_eq!(ChordType::Major.intervals(), &[0, 4, 7]);
        assert_eq!(ChordType::Minor.intervals(), &[0, 3, 7]);
        assert_eq!(ChordType::Major7.intervals(), &[0, 4, 7, 11]);
        assert_eq!(ChordType::Minor7.intervals(), &[0, 3, 7, 10]);
        assert_eq!(ChordType::Dominant7.intervals(), &[0, 4, 7, 10]);
        assert_eq!(ChordType::Sus2.intervals(), &[0, 2, 7]);
        assert_eq!(ChordType::Sus4.intervals(), &[0, 5, 7]);
        assert_eq!(ChordType::Power.intervals(), &[0, 7]);
        assert_eq!(ChordType::Octave.intervals(), &[0, 12]);
    }
}
