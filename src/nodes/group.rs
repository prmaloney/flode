//! Node groups. A group is a node that holds a graph of its own. Inside that graph one
//! `GroupInput` node is where the group's input pins enter, and one `GroupOutput` node is where
//! its output pins leave. A wire crosses the boundary through a numbered pin, so pin `n` on the
//! group node is pin `n` on the matching edge node.
//!
//! The group node keeps a copy of the pin lists on its face, so it can draw and wire them without
//! opening the graph inside. The editor keeps the copy in step with the edge nodes.

use super::connection::Connection;
use super::{Bypass, FlodeNode, KnobId, NodeBehavior, NodeKind, Port, Row, Signal};
use nih_plug_iced::Point;
use serde::{Deserialize, Serialize};

/// The most knobs a group node shows on its face.
pub const GROUP_KNOB_COUNT: usize = 8;

/// The knob ids a group node's face knobs use, in order. A group node has no knobs of its own, so
/// these ids only name its drag state and its rows. Every read of one goes through the node, which
/// swaps in the id of the knob the pin feeds, so the range, name, and snapping are that knob's.
pub const GROUP_KNOBS: [KnobId; GROUP_KNOB_COUNT] = [
    KnobId::Gain,
    KnobId::Attack,
    KnobId::Decay,
    KnobId::Sustain,
    KnobId::Release,
    KnobId::Cutoff,
    KnobId::Resonance,
    KnobId::Unison,
];

fn middle() -> f32 {
    0.5
}

/// One pin on the boundary of a group: its name and the kind of signal it carries.
///
/// A control pin that feeds a knob inside the group also has a value, which the group node's face
/// shows as a knob while nothing is wired into the pin. The value is normalized, and it follows
/// the range of the first knob the pin feeds, which is `knob`. The face copy of the pin is the
/// one that counts, and the editor keeps `knob` current.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupPin {
    pub name: String,
    pub signal: Signal,
    #[serde(default = "middle")]
    pub value: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knob: Option<KnobId>,
}

impl GroupPin {
    pub fn new(name: impl Into<String>, signal: Signal) -> Self {
        Self {
            name: name.into(),
            signal,
            value: middle(),
            knob: None,
        }
    }
}

/// The graph a group node holds. Wire indices name nodes in `nodes`.
pub struct GroupData {
    pub name: String,
    pub nodes: Vec<FlodeNode>,
    pub links: Vec<Connection>,
}

impl GroupData {
    /// An empty group graph: just its two edge nodes, with no pins and no wires.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            nodes: vec![
                FlodeNode::new(NodeKind::GroupInput, Point::new(0.0, 0.0)),
                FlodeNode::new(NodeKind::GroupOutput, Point::new(400.0, 0.0)),
            ],
            links: Vec::new(),
        }
    }

    /// A deep copy: every node, with its own knob values and any group inside it, and every wire.
    pub fn duplicate(&self) -> GroupData {
        GroupData {
            name: self.name.clone(),
            nodes: self
                .nodes
                .iter()
                .map(|node| node.duplicate(node.position))
                .collect(),
            links: self.links.clone(),
        }
    }

    /// The index of the node of `kind` in this graph, if there is one.
    pub fn find(&self, kind: NodeKind) -> Option<usize> {
        self.nodes.iter().position(|node| node.kind == kind)
    }

    /// Feeds what the graph compiles from into `state`. See `FlodeNode::hash_compile_state`.
    pub fn hash_compile_state<H: std::hash::Hasher>(&self, state: &mut H) {
        use std::hash::Hash;
        state.write_usize(self.nodes.len());
        for node in &self.nodes {
            node.hash_compile_state(state);
        }
        self.links.hash(state);
    }
}

/// The group node, as the outer graph shows it: a pin on the left edge for each input and one on
/// the right edge for each output, sharing rows.
pub struct Group;

/// The edge node inside a group where its input pins enter. Its pins are outputs, since they send
/// what came in to the nodes inside.
pub struct GroupInput;

/// The edge node inside a group where its output pins leave. Its pins are inputs, since they take
/// what the nodes inside made.
pub struct GroupOutput;

fn pin(pins: &[GroupPin], index: usize) -> Option<Port> {
    pins.get(index).map(|pin| Port::Pin(pin.signal, index))
}

impl NodeBehavior for Group {
    fn title(&self) -> &'static str {
        "Group"
    }

    /// A control pin that feeds a knob inside has a knob on its row.
    fn rows(&self, node: &FlodeNode) -> Vec<Row> {
        let knobs = node.face_knobs();
        (0..node.inputs.len().max(node.outputs.len()))
            .map(|index| Row {
                input: pin(&node.inputs, index),
                knob: knobs
                    .iter()
                    .position(|&(pin, _)| pin == index)
                    .map(|slot| GROUP_KNOBS[slot]),
                output: pin(&node.outputs, index),
                ..Row::default()
            })
            .collect()
    }

    /// A switched-off group hands each input to the output of the same signal kind in the same
    /// order, and an output with no such input goes silent. See `FlodeNode::pass_through_input`.
    fn bypass(&self) -> Bypass {
        Bypass::PassThrough
    }
}

impl NodeBehavior for GroupInput {
    fn title(&self) -> &'static str {
        "Group Input"
    }

    /// A row for each pin, then one with the empty pin that makes a new one. A pin with a knob on
    /// the group's face has the same knob here.
    fn rows(&self, node: &FlodeNode) -> Vec<Row> {
        let knobs = node.face_knobs();
        (0..node.outputs.len())
            .map(|index| Row {
                knob: knobs
                    .iter()
                    .position(|&(pin, _)| pin == index)
                    .map(|slot| GROUP_KNOBS[slot]),
                output: pin(&node.outputs, index),
                ..Row::default()
            })
            .chain([Row::default().with_output(Port::NewPin)])
            .collect()
    }
}

impl NodeBehavior for GroupOutput {
    fn title(&self) -> &'static str {
        "Group Output"
    }

    /// A row for each pin, then one with the empty pin that makes a new one.
    fn rows(&self, node: &FlodeNode) -> Vec<Row> {
        (0..node.inputs.len())
            .map(|index| Row {
                input: pin(&node.inputs, index),
                ..Row::default()
            })
            .chain([Row::input(Port::NewPin)])
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(kind: NodeKind) -> FlodeNode {
        FlodeNode::new(kind, Point::new(0.0, 0.0))
    }

    #[test]
    fn a_group_node_has_a_row_for_the_longer_pin_list() {
        let mut group = node(NodeKind::Group);
        assert!(group.rows().is_empty());
        group.inputs = vec![GroupPin::new("A", Signal::Audio)];
        group.outputs = vec![
            GroupPin::new("X", Signal::Control),
            GroupPin::new("Y", Signal::Notes),
        ];
        let rows = group.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].input, Some(Port::Pin(Signal::Audio, 0)));
        assert_eq!(rows[0].output, Some(Port::Pin(Signal::Control, 0)));
        assert_eq!(rows[1].input, None);
        assert_eq!(rows[1].output, Some(Port::Pin(Signal::Notes, 1)));
        assert_eq!(group.pin_name(Port::Pin(Signal::Notes, 1), true), Some("Y"));
        assert_eq!(group.pin_name(Port::Pin(Signal::Notes, 1), false), None);
        assert_eq!(group.pin_name(Port::Audio, false), None);
    }

    #[test]
    fn the_edge_nodes_show_one_pin_each_on_their_inner_side() {
        let mut input = node(NodeKind::GroupInput);
        input.outputs = vec![GroupPin::new("In", Signal::Audio)];
        assert_eq!(
            input.output_ports(),
            vec![Port::Pin(Signal::Audio, 0), Port::NewPin]
        );
        assert!(input.input_ports().is_empty());
        let mut output = node(NodeKind::GroupOutput);
        output.inputs = vec![GroupPin::new("Out", Signal::Audio)];
        assert_eq!(
            output.input_ports(),
            vec![Port::Pin(Signal::Audio, 0), Port::NewPin]
        );
        assert!(output.output_ports().is_empty());
    }

    #[test]
    fn the_empty_pin_never_ends_a_wire() {
        let mut edge = node(NodeKind::GroupInput);
        edge.outputs = vec![GroupPin::new("In", Signal::Audio)];
        let nodes = [edge, node(NodeKind::GroupOutput), node(NodeKind::Filter)];
        let from_empty = Connection {
            from: 0,
            out: Port::NewPin,
            to: 2,
            input: Port::Audio,
        };
        assert!(!from_empty.is_valid(&nodes));
        let into_empty = Connection {
            from: 2,
            out: Port::Audio,
            to: 1,
            input: Port::NewPin,
        };
        assert!(!into_empty.is_valid(&nodes));
    }

    #[test]
    fn a_pin_that_does_not_exist_makes_a_wire_invalid() {
        let mut group = node(NodeKind::Group);
        group.inputs = vec![GroupPin::new("In", Signal::Audio)];
        let nodes = [node(NodeKind::Oscillator), group];
        let wire = Connection {
            from: 0,
            out: Port::Audio,
            to: 1,
            input: Port::Pin(Signal::Audio, 0),
        };
        assert!(wire.is_valid(&nodes));
        let missing = Connection {
            input: Port::Pin(Signal::Audio, 1),
            ..wire
        };
        assert!(!missing.is_valid(&nodes));
        let wrong_kind = Connection {
            input: Port::Pin(Signal::Control, 0),
            ..wire
        };
        assert!(!wrong_kind.is_valid(&nodes));
    }

    #[test]
    fn a_copy_of_a_group_has_its_own_graph() {
        let mut group = node(NodeKind::Group);
        group.inputs = vec![GroupPin::new("In", Signal::Audio)];
        group
            .group
            .as_mut()
            .unwrap()
            .nodes
            .push(node(NodeKind::Filter));
        let mut copy = group.duplicate(Point::new(5.0, 5.0));
        assert_eq!(copy.inputs, group.inputs);
        assert_eq!(copy.group.as_ref().unwrap().nodes.len(), 3);
        copy.group.as_mut().unwrap().nodes.pop();
        copy.group.as_mut().unwrap().name = "Other".into();
        assert_eq!(group.group.as_ref().unwrap().nodes.len(), 3);
        assert_eq!(group.label(), "Group");
        assert_eq!(copy.label(), "Other");
    }

    #[test]
    fn a_group_with_an_empty_name_is_labeled_group() {
        let mut group = node(NodeKind::Group);
        group.group.as_mut().unwrap().name = "  ".into();
        assert_eq!(group.label(), "Group");
        assert_eq!(node(NodeKind::Filter).label(), "Filter");
    }

    #[test]
    fn edge_nodes_cannot_be_bypassed_or_copied_in_pieces() {
        for kind in [NodeKind::GroupInput, NodeKind::GroupOutput] {
            let mut part = node(kind);
            assert!(kind.is_group_part());
            assert!(!part.toggle_bypass());
        }
        assert!(node(NodeKind::Group).can_duplicate());
        assert!(!node(NodeKind::GroupInput).can_duplicate());
        assert!(!node(NodeKind::GroupOutput).can_duplicate());
    }

    /// A group can be switched off, like a processor.
    #[test]
    fn a_group_can_be_switched_off() {
        let mut group = node(NodeKind::Group);
        assert!(group.toggle_bypass());
        assert!(group.bypassed);
    }

    /// An output takes the input of its signal kind that holds the same place, and one with no
    /// such input goes silent.
    #[test]
    fn an_output_passes_through_the_input_in_its_place() {
        let mut group = node(NodeKind::Group);
        group.inputs = vec![
            GroupPin::new("Freq", Signal::Control),
            GroupPin::new("L", Signal::Audio),
            GroupPin::new("R", Signal::Audio),
        ];
        group.outputs = vec![
            GroupPin::new("A", Signal::Audio),
            GroupPin::new("B", Signal::Audio),
            GroupPin::new("C", Signal::Audio),
            GroupPin::new("Shape", Signal::Control),
        ];
        assert_eq!(group.pass_through_input(0), Some(1));
        assert_eq!(group.pass_through_input(1), Some(2));
        // There is no third audio input.
        assert_eq!(group.pass_through_input(2), None);
        assert_eq!(group.pass_through_input(3), Some(0));
        assert_eq!(group.pass_through_input(9), None);
    }

    /// A switched-off group says "bypass" if anything passes through, and "mute" if nothing does.
    #[test]
    fn the_header_names_how_a_group_is_off() {
        let mut group = node(NodeKind::Group);
        group.toggle_bypass();
        // No pins at all, so nothing passes through.
        assert_eq!(group.bypass_label(), Some("mute"));
        group.inputs = vec![GroupPin::new("In", Signal::Audio)];
        group.outputs = vec![GroupPin::new("Out", Signal::Audio)];
        assert_eq!(group.bypass_label(), Some("bypass"));
        group.toggle_bypass();
        assert_eq!(group.bypass_label(), None);
    }
}
