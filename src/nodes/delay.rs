//! The delay node. The Sync knob sets the time from the host's tempo, and Ping pong bounces the
//! repeats between the sides.

use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};

pub struct Delay;

impl NodeBehavior for Delay {
    fn title(&self) -> &'static str {
        "Delay"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::input(Port::Audio).with_output(Port::Audio),
            Row::driven_knob(KnobId::DelayTime),
            Row::plain_knob(KnobId::LfoSync),
            Row::driven_knob(KnobId::Feedback),
            Row::driven_knob(KnobId::Damp),
            Row::driven_knob(KnobId::Mix),
            Row::control(Control::Switch(0)),
        ]
    }

    fn audio_in(&self) -> bool {
        true
    }

    fn audio_out(&self) -> bool {
        true
    }

    fn switch_labels(&self) -> &'static [&'static str] {
        &["Ping pong"]
    }

    /// A delay starts mostly dry.
    fn defaults(&self) -> &'static [(KnobId, f32)] {
        &[(KnobId::Mix, 0.3)]
    }
}

/// True when the repeats bounce between the sides.
pub fn ping_pong(node: &FlodeNode) -> bool {
    node.switch(0)
}
