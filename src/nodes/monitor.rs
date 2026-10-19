//! The monitor node. It listens to one audio wire and shows what it hears on a screen under its
//! pin. A dropdown picks what the screen shows. The node has no output and no knobs, and it
//! changes nothing in the sound. It only shows a wire that reaches an output slot.

use super::{Control, FlodeNode, NodeBehavior, Port, Row, MONITOR_WIDTH};
use crate::dsp::monitor::MonitorMode;

pub struct Monitor;

impl NodeBehavior for Monitor {
    fn title(&self) -> &'static str {
        "Monitor"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::input(Port::Audio),
            Row::control(Control::Dropdown(0)),
            Row::control(Control::Monitor),
        ]
    }

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&MonitorMode::LABELS]
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Mode"]
    }

    fn width(&self) -> f32 {
        MONITOR_WIDTH
    }

    fn audio_in(&self) -> bool {
        true
    }
}

/// What the node's screen shows.
pub fn mode(node: &FlodeNode) -> MonitorMode {
    MonitorMode::ALL[node.choice(0).min(MonitorMode::ALL.len() - 1)]
}
