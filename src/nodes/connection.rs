//! A wire between two nodes.

use super::{FlodeNode, Port};
use serde::{Deserialize, Serialize};

/// A wire from an output port on one node to an input port on another.
#[derive(Debug, Clone, Copy, PartialEq, Hash, Serialize, Deserialize)]
pub struct Connection {
    pub from: usize,
    pub out: Port,
    pub to: usize,
    pub input: Port,
}

impl Connection {
    /// True if the wire joins ports that exist and carry the same kind of signal. Audio goes to
    /// audio, control goes to a knob, and notes go to notes. The empty pin on a group's edge node
    /// takes no wire, since a drop on it makes a real pin first.
    pub fn is_valid(&self, nodes: &[FlodeNode]) -> bool {
        let (Some(from), Some(to)) = (nodes.get(self.from), nodes.get(self.to)) else {
            return false;
        };
        self.from != self.to
            && self.out != Port::NewPin
            && self.input != Port::NewPin
            && self.out.signal() == self.input.signal()
            && from.output_point(self.out).is_some()
            && to.input_point(self.input).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    fn node(kind: NodeKind) -> FlodeNode {
        FlodeNode::new(kind, Point::new(0.0, 0.0))
    }

    #[test]
    fn audio_joins_audio_and_a_node_cannot_wire_to_itself() {
        let nodes = [node(NodeKind::Oscillator), node(NodeKind::Filter)];
        let wire = Connection {
            from: 0,
            out: Port::Audio,
            to: 1,
            input: Port::Audio,
        };
        assert!(wire.is_valid(&nodes));
        assert!(!Connection { to: 0, ..wire }.is_valid(&nodes));
    }

    #[test]
    fn notes_join_only_notes() {
        let nodes = [
            node(NodeKind::Input),
            node(NodeKind::Oscillator),
            node(NodeKind::Filter),
        ];
        let notes = Connection {
            from: 0,
            out: Port::Notes,
            to: 1,
            input: Port::Notes,
        };
        assert!(notes.is_valid(&nodes));
        // Notes do not drive a knob or a shape pin, and audio does not feed a notes pin.
        let knob = Port::Knob(crate::nodes::KnobId::Cutoff);
        assert!(!Connection {
            to: 2,
            input: knob,
            ..notes
        }
        .is_valid(&nodes));
        assert!(!Connection {
            to: 1,
            input: Port::Shape,
            ..notes
        }
        .is_valid(&nodes));
        let audio = Connection {
            from: 1,
            out: Port::Audio,
            to: 1,
            input: Port::Notes,
        };
        assert!(!Connection { from: 2, ..audio }.is_valid(&nodes));
        // A controller pin does not feed a notes pin either.
        let controller = Connection {
            out: Port::Midi(crate::nodes::input::MidiSource::ModWheel),
            ..notes
        };
        assert!(!controller.is_valid(&nodes));
    }

    #[test]
    fn audio_cannot_drive_a_knob() {
        let nodes = [node(NodeKind::Oscillator), node(NodeKind::Filter)];
        let wire = Connection {
            from: 0,
            out: Port::Audio,
            to: 1,
            input: Port::Knob(crate::nodes::KnobId::Cutoff),
        };
        assert!(!wire.is_valid(&nodes));
    }
}

#[cfg(test)]
mod effect_rule_tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    fn node(kind: NodeKind) -> FlodeNode {
        FlodeNode::new(kind, Point::new(0.0, 0.0))
    }

    fn wire(nodes: &[FlodeNode], from: usize, to: usize, input: Port) -> bool {
        Connection {
            from,
            out: Port::Audio,
            to,
            input,
        }
        .is_valid(nodes)
    }

    /// An effect's output goes to anything that takes audio: another effect, a filter, a mix, or
    /// an output slot. A source takes no audio, so no wire runs into one.
    #[test]
    fn an_effect_feeds_anything_that_takes_audio() {
        let nodes = [
            node(NodeKind::Reverb),
            node(NodeKind::Delay),
            node(NodeKind::Filter),
            node(NodeKind::Mix),
            node(NodeKind::Output),
            node(NodeKind::Oscillator),
        ];
        assert!(wire(&nodes, 0, 1, Port::Audio));
        assert!(wire(&nodes, 0, 4, Port::Slot(0)));
        assert!(wire(&nodes, 0, 2, Port::Audio));
        assert!(wire(&nodes, 0, 3, Port::AudioIn(0)));
        assert!(wire(&nodes, 0, 3, Port::AudioIn(1)));
        assert!(wire(&nodes, 5, 0, Port::Audio));
        assert!(wire(&nodes, 2, 1, Port::Audio));
        assert!(!wire(&nodes, 0, 5, Port::Audio));
    }
}
