//! A wire between two nodes.

use super::{FlodeNode, Port};
use serde::{Deserialize, Serialize};

/// A wire from an output port on one node to an input port on another.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub from: usize,
    pub out: Port,
    pub to: usize,
    pub input: Port,
}

impl Connection {
    /// True if the wire joins ports that exist and carry the same kind of signal. Audio goes to
    /// audio, and control goes to a knob.
    pub fn is_valid(&self, nodes: &[FlodeNode]) -> bool {
        let (Some(from), Some(to)) = (nodes.get(self.from), nodes.get(self.to)) else {
            return false;
        };
        self.from != self.to
            && (self.out == Port::Audio) == self.input.is_audio()
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
