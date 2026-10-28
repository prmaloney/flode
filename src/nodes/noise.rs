//! The noise node. A source of white, pink, brown or Geiger noise, played per voice from the notes
//! wired into it, shaped by an envelope like an oscillator is. It has no pitch, so it has no
//! transpose, unison or warp.

use super::viewer::{sample, Live, Trace};
use super::{Bypass, Control, FlodeNode, KnobId, NodeBehavior, Port, Row};
use crate::dsp::noise::NoiseType;

pub struct Noise;

/// The Type dropdown's rows.
pub const NOISE_LABELS: [&str; 4] = ["White", "Pink", "Brown", "Geiger"];

impl NodeBehavior for Noise {
    fn title(&self) -> &'static str {
        "Noise"
    }

    /// The notes pin leads, since with no wire here the node is silent. The shape row has the
    /// envelope input and the audio output. Level and pan follow, as on an oscillator.
    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::input(Port::Notes),
            Row::control(Control::Dropdown(0)),
            Row::control(Control::Viewer),
            Row::input(Port::Shape).with_output(Port::Audio),
            Row::driven_knob(KnobId::OscGain),
            Row::driven_knob(KnobId::Pan),
        ]
    }

    fn audio_out(&self) -> bool {
        true
    }

    fn bypass(&self) -> Bypass {
        Bypass::Mute
    }

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&NOISE_LABELS]
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Type"]
    }

    /// Noise has no cycle, so the box draws a fixed jagged line that does not change between
    /// frames.
    fn trace(&self, _node: &FlodeNode, _live: &Live, samples: usize) -> Option<Trace> {
        Some(Trace {
            points: sample(samples, super::oscillator::noise_shape),
            ..Trace::default()
        })
    }
}

/// The noise a node makes.
pub fn noise_type(node: &FlodeNode) -> NoiseType {
    NoiseType::ALL[node.choice(0).min(NoiseType::ALL.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    fn node() -> FlodeNode {
        FlodeNode::new(NodeKind::Noise, Point::new(0.0, 0.0))
    }

    #[test]
    fn the_node_takes_notes_and_an_envelope_and_makes_audio() {
        let node = node();
        assert_eq!(node.input_ports()[0], Port::Notes);
        assert!(node.input_ports().contains(&Port::Shape));
        assert_eq!(node.output_ports(), vec![Port::Audio]);
    }

    #[test]
    fn the_dropdown_picks_the_type() {
        let mut node = node();
        assert_eq!(noise_type(&node), NoiseType::White);
        for (row, expected) in [
            (1, NoiseType::Pink),
            (2, NoiseType::Brown),
            (3, NoiseType::Geiger),
        ] {
            node.choose(0, row);
            assert_eq!(noise_type(&node), expected);
        }
        assert_eq!(NOISE_LABELS.len(), 4);
    }

    #[test]
    fn the_trace_is_fixed() {
        let node = node();
        let live = Live::of(&node);
        let a = Noise.trace(&node, &live, 32).unwrap();
        let b = Noise.trace(&node, &live, 32).unwrap();
        assert_eq!(a, b);
        assert!(a.points.iter().all(|p| (0.0..=1.0).contains(p)));
    }
}
