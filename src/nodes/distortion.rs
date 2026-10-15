//! The distortion node. A waveshaper with a choice of curve.

use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};
use crate::dsp::effects::distortion::DistortionKind;

pub struct Distortion;

const TYPE_LABELS: [&str; 5] = ["Soft", "Hard", "Tube", "Fold", "Crush"];

impl NodeBehavior for Distortion {
    fn title(&self) -> &'static str {
        "Distortion"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::input(Port::Audio).with_output(Port::Audio),
            Row::driven_knob(KnobId::Drive),
            Row::driven_knob(KnobId::Tone),
            Row::driven_knob(KnobId::OscGain),
            Row::driven_knob(KnobId::Mix),
            Row::control(Control::Dropdown(0)),
        ]
    }

    fn audio_in(&self) -> bool {
        true
    }

    fn audio_out(&self) -> bool {
        true
    }

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&TYPE_LABELS]
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Type"]
    }

    /// A distortion starts fully wet.
    fn defaults(&self) -> &'static [(KnobId, f32)] {
        &[(KnobId::Mix, 1.0)]
    }
}

/// The curve a distortion node applies.
pub fn kind(node: &FlodeNode) -> DistortionKind {
    DistortionKind::ALL
        .get(node.choice(0))
        .copied()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    #[test]
    fn the_labels_match_the_kinds() {
        assert_eq!(TYPE_LABELS.len(), DistortionKind::ALL.len());
    }

    #[test]
    fn a_new_distortion_starts_fully_wet() {
        let node = FlodeNode::new(NodeKind::Distortion, Point::new(0.0, 0.0));
        assert_eq!(node.plain(KnobId::Mix), 1.0);
        assert_eq!(kind(&node), DistortionKind::Soft);
    }
}
