//! The chorus node.

use super::{FlodeNode, KnobId, NodeBehavior, Port, Row};

pub struct Chorus;

impl NodeBehavior for Chorus {
    fn title(&self) -> &'static str {
        "Chorus"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::input(Port::Audio).with_output(Port::Audio),
            Row::driven_knob(KnobId::LfoRate),
            Row::driven_knob(KnobId::Depth),
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

    /// A chorus starts at a gentle rate.
    fn defaults(&self) -> &'static [(KnobId, f32)] {
        &[(KnobId::LfoRate, 0.8)]
    }
}
