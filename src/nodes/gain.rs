//! The gain node. A utility with three controls. The Gain knob sets the level from -60 dB, which
//! is silent, up to +24 dB. The Pan knob balances a stereo signal, turning one side down and
//! leaving the other. The Invert switch flips the polarity of both sides.

use super::{Bypass, Control, FlodeNode, KnobId, NodeBehavior, Port, Row};

pub struct Gain;

impl NodeBehavior for Gain {
    fn title(&self) -> &'static str {
        "Gain"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::input(Port::Audio).with_output(Port::Audio),
            Row::driven_knob(KnobId::UtilGain),
            Row::driven_knob(KnobId::Pan),
            Row::control(Control::Switch(0)),
        ]
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

    fn switch_labels(&self) -> &'static [&'static str] {
        &["Invert"]
    }
}

/// True when the node flips the polarity of both sides.
pub fn invert(node: &FlodeNode) -> bool {
    node.switch(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    fn node() -> FlodeNode {
        FlodeNode::new(NodeKind::Gain, Point::new(0.0, 0.0))
    }

    /// A new node passes the signal unchanged.
    #[test]
    fn a_new_gain_node_is_neutral() {
        let node = node();
        assert_eq!(node.plain(KnobId::UtilGain), 0.0);
        assert_eq!(node.plain(KnobId::Pan), 0.0);
        assert!(!invert(&node));
    }

    #[test]
    fn the_rows_are_an_audio_pair_two_driven_knobs_and_a_switch() {
        let rows = Gain.rows(&node());
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].input, Some(Port::Audio));
        assert_eq!(rows[0].output, Some(Port::Audio));
        assert_eq!(rows[1].knob, Some(KnobId::UtilGain));
        assert_eq!(rows[2].knob, Some(KnobId::Pan));
        assert!(rows[1].input.is_some() && rows[2].input.is_some());
        assert_eq!(rows[3].control, Some(Control::Switch(0)));
        assert_eq!(Gain.switch_labels(), &["Invert"]);
    }

    #[test]
    fn invert_follows_the_switch() {
        let mut node = node();
        node.set_switch(0, true);
        assert!(invert(&node));
    }
}
