//! The envelope node. It runs an attack, decay, sustain, and release for every voice, and sends
//! that level out as a control signal on its shape pin. It has no audio pins. To shape a sound,
//! drive an oscillator's gain from it.

use super::{FlodeNode, KnobId, NodeBehavior, Port, Row};

pub struct Envelope;

impl NodeBehavior for Envelope {
    fn title(&self) -> &'static str {
        "Envelope"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::driven_knob(KnobId::Attack).with_output(Port::Shape),
            Row::driven_knob(KnobId::Decay),
            Row::driven_knob(KnobId::Sustain),
            Row::driven_knob(KnobId::Release),
        ]
    }
}
