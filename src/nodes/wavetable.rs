//! The wavetable node. It collects the frames its inputs make into one table. Each connected frame
//! input is a region of the node's ramp, and the handles on the ramp set how much of the table each
//! region gets. Position moves along the ramp, and the node hands the table and the position to the
//! oscillator it feeds. A pin shows for each connected input, with one empty pin below the last.

use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};

pub struct Wavetable;

/// The most frame inputs a wavetable node has, which is also how many regions its ramp has.
pub const MAX_FRAMES: usize = 8;

/// The Blend dropdown's rows: how neighbouring regions meet.
pub const BLEND_LABELS: [&str; 3] = ["Linear", "Smooth", "Step"];

impl NodeBehavior for Wavetable {
    fn title(&self) -> &'static str {
        "Wavetable"
    }

    /// The table output, then Position, the ramp, how regions blend, and a pin for each frame
    /// input. The shares the ramp's handles set have no row of their own.
    fn rows(&self, node: &FlodeNode) -> Vec<Row> {
        let mut rows = vec![
            Row::default().with_output(Port::Table),
            Row::driven_knob(KnobId::WtPosition),
            Row::control(Control::Ramp),
            Row::control(Control::Dropdown(0)),
            Row::plain_knob(KnobId::WtBlendWidth),
        ];
        for slot in 0..node.slots.min(MAX_FRAMES) {
            rows.push(Row::input(Port::Frame(slot)));
        }
        rows
    }

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&BLEND_LABELS]
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Blend"]
    }
}

/// The row of the Blend dropdown a wavetable node has chosen.
pub fn blend_index(node: &FlodeNode) -> usize {
    node.choice(0).min(BLEND_LABELS.len() - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    fn node() -> FlodeNode {
        FlodeNode::new(NodeKind::Wavetable, Point::new(0.0, 0.0))
    }

    /// A new node shows one frame pin, and a pin shows for each slot after that.
    #[test]
    fn a_pin_shows_for_each_slot() {
        let mut node = node();
        let frames = |node: &FlodeNode| {
            node.input_ports()
                .into_iter()
                .filter(|port| matches!(port, Port::Frame(_)))
                .count()
        };
        assert_eq!(frames(&node), 1);
        node.slots = 3;
        assert_eq!(frames(&node), 3);
        node.slots = 100;
        assert_eq!(frames(&node), MAX_FRAMES);
    }

    /// The output is a table, Position can be driven, and the shares and blend width have no pin.
    #[test]
    fn the_node_makes_a_table_and_takes_a_position() {
        let node = node();
        assert_eq!(node.output_ports(), vec![Port::Table]);
        let ports = node.input_ports();
        assert!(ports.contains(&Port::Knob(KnobId::WtPosition)));
        assert!(!ports.contains(&Port::Knob(KnobId::WtBlendWidth)));
        assert!(!ports
            .iter()
            .any(|port| matches!(port, Port::Knob(KnobId::Share1))));
    }

    #[test]
    fn the_blend_dropdown_lists_three_modes() {
        assert_eq!(BLEND_LABELS.len(), 3);
        assert_eq!(blend_index(&node()), 0);
    }
}
