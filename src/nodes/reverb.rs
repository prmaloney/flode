//! The reverb node. A basic algorithmic reverb.

use super::{Bypass, FlodeNode, KnobId, NodeBehavior, Port, Row};

pub struct Reverb;

impl NodeBehavior for Reverb {
    fn title(&self) -> &'static str {
        "Reverb"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::input(Port::Audio).with_output(Port::Audio),
            Row::driven_knob(KnobId::ReverbDecay),
            Row::driven_knob(KnobId::Size),
            Row::driven_knob(KnobId::Damp),
            Row::driven_knob(KnobId::Mix),
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

    /// A reverb starts mostly dry.
    fn defaults(&self) -> &'static [(KnobId, f32)] {
        &[(KnobId::Mix, 0.3)]
    }
}
