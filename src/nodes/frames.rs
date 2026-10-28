//! The nodes that make frames for a wavetable: a drawn curve, a list of harmonics, a sweep that
//! makes many frames at once, and frames cut from an audio file. Each has one table output, which
//! goes to a frame input of a wavetable node. Their knobs have no pins, since a frame is worked
//! out when the patch is built, not while the sound plays.

use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};

/// The row every frame node starts with: the table output.
fn output_row() -> Row {
    Row::default().with_output(Port::Table)
}

pub struct CurveFrame;

impl NodeBehavior for CurveFrame {
    fn title(&self) -> &'static str {
        "Curve frame"
    }

    /// The drawn cycle is a viewer with points to drag, like an LFO's custom wave.
    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![output_row(), Row::control(Control::Viewer)]
    }
}

pub struct Harmonics;

impl NodeBehavior for Harmonics {
    fn title(&self) -> &'static str {
        "Harmonics"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            output_row(),
            Row::control(Control::Viewer),
            Row::plain_knob(KnobId::HarmCount),
            Row::plain_knob(KnobId::HarmTilt),
            Row::plain_knob(KnobId::HarmOddEven),
        ]
    }
}

/// The Kind dropdown's rows for a sweep: what changes from the first frame to the last.
pub const SWEEP_LABELS: [&str; 4] = ["Harmonics", "PWM", "Sync", "Fold"];

pub struct Sweep;

impl NodeBehavior for Sweep {
    fn title(&self) -> &'static str {
        "Sweep"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            output_row(),
            Row::control(Control::Viewer),
            Row::control(Control::Dropdown(0)),
            Row::plain_knob(KnobId::SweepFrames),
            Row::plain_knob(KnobId::SweepAmount),
        ]
    }

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&SWEEP_LABELS]
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Kind"]
    }
}

/// The Slice dropdown's rows for a file: cut one pitch period per frame, or a fixed number of
/// samples.
pub const SLICE_LABELS: [&str; 4] = ["Cycles", "1024", "2048", "4096"];

pub struct WaveFile;

impl NodeBehavior for WaveFile {
    fn title(&self) -> &'static str {
        "Wave file"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            output_row(),
            Row::control(Control::LoadSample),
            Row::control(Control::Viewer),
            Row::control(Control::Dropdown(0)),
        ]
    }

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&SLICE_LABELS]
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Slice"]
    }
}

/// The sweep a node has chosen, as an index into `SWEEP_LABELS`.
pub fn sweep_index(node: &FlodeNode) -> usize {
    node.choice(0).min(SWEEP_LABELS.len() - 1)
}

/// The slice mode a file node has chosen, as an index into `SLICE_LABELS`.
pub fn slice_index(node: &FlodeNode) -> usize {
    node.choice(0).min(SLICE_LABELS.len() - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    fn node(kind: NodeKind) -> FlodeNode {
        FlodeNode::new(kind, Point::new(0.0, 0.0))
    }

    /// Every frame node makes a table and takes no signal of its own.
    #[test]
    fn frame_nodes_make_a_table_and_take_no_wires() {
        for kind in [
            NodeKind::CurveFrame,
            NodeKind::Harmonics,
            NodeKind::Sweep,
            NodeKind::WaveFile,
        ] {
            let node = node(kind);
            assert_eq!(node.output_ports(), vec![Port::Table], "{kind:?}");
            assert!(node.input_ports().is_empty(), "{kind:?}");
        }
    }

    #[test]
    fn the_dropdowns_have_a_row_for_each_choice() {
        assert_eq!(SWEEP_LABELS.len(), 4);
        assert_eq!(SLICE_LABELS.len(), 4);
        assert_eq!(sweep_index(&node(NodeKind::Sweep)), 0);
        assert_eq!(slice_index(&node(NodeKind::WaveFile)), 0);
    }
}
