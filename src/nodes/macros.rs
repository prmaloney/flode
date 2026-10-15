//! The macro node. It has four knobs, and each one is a host parameter, so the DAW can automate
//! it. Each knob has a control pin that can drive any knob in the graph. The knobs show the host
//! parameters and do not keep a value of their own, so a graph has only one macro node.

use super::{FlodeNode, KnobId, NodeBehavior, Port, Row};

pub struct Macros;

/// How many macro knobs there are, and so how many host parameters.
pub const MACRO_COUNT: usize = 4;

impl NodeBehavior for Macros {
    fn title(&self) -> &'static str {
        "Macro"
    }

    /// One knob per macro, each with a control pin on the right edge.
    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        KnobId::MACROS
            .iter()
            .enumerate()
            .map(|(index, &id)| Row::plain_knob(id).with_output(Port::Macro(index)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    #[test]
    fn each_macro_has_a_knob_and_a_pin() {
        let node = FlodeNode::new(NodeKind::Macro, Point::new(0.0, 0.0));
        let rows = node.rows();
        assert_eq!(rows.len(), MACRO_COUNT);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row.knob, Some(KnobId::MACROS[index]));
            assert_eq!(row.output, Some(Port::Macro(index)));
            assert_eq!(KnobId::MACROS[index].macro_index(), Some(index));
        }
    }

    #[test]
    fn a_macro_node_cannot_be_copied() {
        let node = FlodeNode::new(NodeKind::Macro, Point::new(0.0, 0.0));
        assert!(!node.can_duplicate());
    }
}
