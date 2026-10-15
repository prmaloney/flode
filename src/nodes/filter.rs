//! The filter node. A state-variable filter with a low, high, or band pass response.

use super::{Bypass, Control, FlodeNode, KnobId, NodeBehavior, Port, Row};
use crate::dsp::svf::FilterType;

pub struct Filter;

const FILTER_LABELS: [&str; 3] = ["Low pass", "High pass", "Band pass"];

impl NodeBehavior for Filter {
    fn title(&self) -> &'static str {
        "Filter"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::input(Port::Audio).with_output(Port::Audio),
            Row::driven_knob(KnobId::Cutoff),
            Row::driven_knob(KnobId::Resonance),
            Row::control(Control::Dropdown(0)),
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

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&FILTER_LABELS]
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Type"]
    }
}

/// The response a filter node has.
pub fn filter_type(node: &FlodeNode) -> FilterType {
    FilterType::ALL[node.choice(0).min(FilterType::ALL.len() - 1)]
}
