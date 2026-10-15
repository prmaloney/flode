//! The flanger node.

use super::{FlodeNode, KnobId, NodeBehavior, Port, Row};

pub struct Flanger;

impl NodeBehavior for Flanger {
    fn title(&self) -> &'static str {
        "Flanger"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::input(Port::Audio).with_output(Port::Audio),
            Row::driven_knob(KnobId::LfoRate),
            Row::driven_knob(KnobId::Depth),
            Row::driven_knob(KnobId::Feedback),
            Row::driven_knob(KnobId::FlangerDelay),
            Row::driven_knob(KnobId::Spread),
            Row::driven_knob(KnobId::Mix),
        ]
    }

    fn audio_in(&self) -> bool {
        true
    }

    fn audio_out(&self) -> bool {
        true
    }
}
