//! The mix node. It combines two audio signals into one. The dropdown picks how: a crossfade
//! between them, one modulating the other's amplitude, or true ring modulation. The Mix knob sets
//! how far from plain signal A the result sits, from 0.0 (just A) to 1.0 (fully the chosen effect,
//! or all B for a crossfade).

use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};
use serde::{Deserialize, Serialize};

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

/// How a mix node combines its two input signals.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum MixMode {
    #[default]
    Crossfade = 0,
    Am = 1,
    RingMod = 2,
}

impl MixMode {
    /// Every mode, in the order the dropdown lists them.
    pub const ALL: [MixMode; 3] = [MixMode::Crossfade, MixMode::Am, MixMode::RingMod];

    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// Unknown values fall back to the crossfade.
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => MixMode::Am,
            2 => MixMode::RingMod,
            _ => MixMode::Crossfade,
        }
    }

    /// Combines two already-scaled samples. `mix` runs from 0.0 to 1.0.
    pub fn combine(self, a: f32, b: f32, mix: f32) -> f32 {
        match self {
            // An equal-power blend, so the perceived loudness stays level across the sweep.
            MixMode::Crossfade => {
                let angle = mix * std::f32::consts::FRAC_PI_2;
                a * angle.cos() + b * angle.sin()
            }
            // B modulates A's amplitude. At 0, A passes through unchanged. At 1, B's swing from
            // -1 to 1 takes A's amplitude from zero to double.
            MixMode::Am => a * (1.0 + mix * b),
            // The product of the two signals, faded in as the mix knob rises.
            MixMode::RingMod => {
                let ring = a * b;
                a * (1.0 - mix) + ring * mix
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossfade_at_the_ends_is_just_one_signal() {
        assert_eq!(MixMode::Crossfade.combine(1.0, 0.5, 0.0), 1.0);
        assert!((MixMode::Crossfade.combine(1.0, 0.5, 1.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn am_at_zero_mix_passes_a_through() {
        assert_eq!(MixMode::Am.combine(0.3, 0.9, 0.0), 0.3);
    }

    #[test]
    fn ring_mod_at_full_mix_is_the_plain_product() {
        assert_eq!(MixMode::RingMod.combine(0.5, 0.4, 1.0), 0.2);
        assert_eq!(MixMode::RingMod.combine(0.5, 0.4, 0.0), 0.5);
    }
}
