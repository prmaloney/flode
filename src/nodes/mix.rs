//! The mix node. It combines two audio signals into one. The dropdown picks how: a crossfade
//! between them, one modulating the other's amplitude, or true ring modulation. The Mix knob sets
//! how far from plain signal A the result sits, from 0.0 (just A) to 1.0 (fully the chosen effect,
//! or all B for a crossfade).

use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};
use crate::dsp::mix::MixMode;

pub struct Mix;

const MIX_TYPE_LABELS: [&str; 3] = ["Crossfade", "AM", "Ring Mod"];

impl NodeBehavior for Mix {
    fn title(&self) -> &'static str {
        "Mix"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::control(Control::Dropdown(0)),
            Row::input(Port::AudioIn(0)).with_output(Port::Audio),
            Row::input(Port::AudioIn(1)),
            Row::driven_knob(KnobId::Mix),
        ]
    }

    fn audio_in(&self) -> bool {
        true
    }

    fn audio_out(&self) -> bool {
        true
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Type"]
    }

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&MIX_TYPE_LABELS]
    }
}

/// The way a mix node is set to combine its two signals.
pub fn mix_mode(node: &FlodeNode) -> MixMode {
    MixMode::ALL[node.choice(0).min(MixMode::ALL.len() - 1)]
}
