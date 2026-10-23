//! The arpeggiator node. It sits between a MIDI input and an oscillator: it reads the chord held at
//! its Notes input and re-emits a pattern of its own on its Notes output, which an oscillator or
//! another arpeggiator can read in place of the raw chord. It makes no sound by itself.

use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};

pub struct Arpeggiator;

/// The order an arpeggiator steps through the held chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArpPattern {
    Up,
    Down,
    UpDown,
    /// The order the notes were pressed in, oldest first.
    AsPlayed,
    /// A new, random note of the chord each step.
    Random,
}

impl ArpPattern {
    pub const ALL: [ArpPattern; 5] = [
        ArpPattern::Up,
        ArpPattern::Down,
        ArpPattern::UpDown,
        ArpPattern::AsPlayed,
        ArpPattern::Random,
    ];

    pub const LABELS: [&'static str; 5] = ["Up", "Down", "Up-down", "As played", "Random"];
}

impl NodeBehavior for Arpeggiator {
    fn title(&self) -> &'static str {
        "Arpeggiator"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::input(Port::Notes).with_output(Port::Notes),
            Row::driven_knob(KnobId::LfoRate),
            Row::plain_knob(KnobId::LfoSync),
            Row::plain_knob(KnobId::ArpOctaves),
            Row::plain_knob(KnobId::ArpGate),
            Row::control(Control::Dropdown(0)),
        ]
    }

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&ArpPattern::LABELS]
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Pattern"]
    }
}

/// The pattern the node's dropdown chooses.
pub fn pattern(node: &FlodeNode) -> ArpPattern {
    ArpPattern::ALL[node.choice(0).min(ArpPattern::ALL.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    #[test]
    fn the_notes_pin_is_the_first_row_both_ways() {
        let node = FlodeNode::new(NodeKind::Arpeggiator, Point::new(0.0, 0.0));
        assert_eq!(node.input_ports()[0], Port::Notes);
        assert_eq!(node.output_ports()[0], Port::Notes);
    }

    #[test]
    fn the_pattern_dropdown_starts_at_up() {
        let node = FlodeNode::new(NodeKind::Arpeggiator, Point::new(0.0, 0.0));
        assert_eq!(pattern(&node), ArpPattern::Up);
    }
}
