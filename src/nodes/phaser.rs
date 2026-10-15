//! The phaser node. A dropdown picks how many all-pass stages it runs.

use super::{Bypass, Control, FlodeNode, KnobId, NodeBehavior, Port, Row, Section};

pub struct Phaser;

/// The stage counts the dropdown offers, in the order it lists them.
pub const STAGE_COUNTS: [usize; 4] = [4, 6, 8, 12];
const STAGE_LABELS: [&str; 4] = ["4", "6", "8", "12"];

impl NodeBehavior for Phaser {
    fn title(&self) -> &'static str {
        "Phaser"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::input(Port::Audio).with_output(Port::Audio),
            Row::driven_knob(KnobId::LfoRate),
            Row::driven_knob(KnobId::Depth),
            Row::driven_knob(KnobId::Mix),
            Row::control(Control::Dropdown(0)),
            // The rest of the sweep's shape folds away.
            Row::section(0),
            Row::driven_knob(KnobId::Feedback),
            Row::driven_knob(KnobId::Center),
            Row::driven_knob(KnobId::Spread),
        ]
    }

    fn sections(&self) -> &'static [Section] {
        &[Section {
            title: "Sweep",
            collapsed: true,
        }]
    }

    fn audio_in(&self) -> bool {
        true
    }

    fn audio_out(&self) -> bool {
        true
    }

    fn bypass(&self) -> Bypass {
        Bypass::PassThrough
    }

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&STAGE_LABELS]
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Stages"]
    }
}

/// The number of all-pass stages a phaser node runs.
pub fn stages(node: &FlodeNode) -> usize {
    STAGE_COUNTS
        .get(node.choice(0))
        .copied()
        .unwrap_or(STAGE_COUNTS[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    #[test]
    fn the_dropdown_picks_the_stage_count() {
        let mut node = FlodeNode::new(NodeKind::Phaser, Point::new(0.0, 0.0));
        assert_eq!(stages(&node), 4);
        node.choose(0, 3);
        assert_eq!(stages(&node), 12);
        assert_eq!(STAGE_LABELS.len(), STAGE_COUNTS.len());
    }
}
