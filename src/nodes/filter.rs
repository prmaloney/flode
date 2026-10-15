//! The filter node. A state-variable filter with a low, high, or band pass response.

use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};
use serde::{Deserialize, Serialize};

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

/// The filter responses. The audio thread's state-variable filter computes all three outputs from
/// the same state, so switching the type only picks which output to return.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum FilterType {
    #[default]
    LowPass = 0,
    HighPass = 1,
    BandPass = 2,
}

impl FilterType {
    /// Every type, in the order the dropdown lists them.
    pub const ALL: [FilterType; 3] = [
        FilterType::LowPass,
        FilterType::HighPass,
        FilterType::BandPass,
    ];

    /// The dropdown row for this type.
    pub fn index(self) -> usize {
        self as usize
    }

    /// Stored as a u8 so the editor can publish the type to the audio thread atomically.
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// Unknown values fall back to the low pass.
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => FilterType::HighPass,
            2 => FilterType::BandPass,
            _ => FilterType::LowPass,
        }
    }
}
