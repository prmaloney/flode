//! The mix stage: two audio inputs combined into one.

use super::{AudioStage, StartContext, Tick};
use crate::patch::stage::{MixParams, MAX_INPUTS};

pub struct Mix;

impl AudioStage for Mix {
    type Params = MixParams;
    /// A mix remembers nothing between samples.
    type State = ();
    const INPUTS: usize = 2;

    fn start(_: &MixParams, _: &mut StartContext) {}

    fn process(p: &MixParams, _: &mut (), inputs: &[f32; MAX_INPUTS], tick: &mut Tick) -> f32 {
        p.mode
            .combine(inputs[0], inputs[1], p.amount.plain(tick.levels))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::mix::MixMode;
    use crate::engine::stage::testing::Rig;
    use crate::nodes::knobs::KnobId;
    use crate::patch::control::Param;

    fn run(mode: MixMode, amount: f32, a: f32, b: f32) -> f32 {
        let mut rig = Rig::new();
        let params = MixParams {
            mode,
            amount: Param::fixed(KnobId::Mix, amount),
        };
        Mix::process(&params, &mut (), &[a, b], &mut rig.tick())
    }

    #[test]
    fn the_stage_applies_its_mode_and_amount() {
        assert_eq!(run(MixMode::Crossfade, 0.0, 0.8, 0.2), 0.8);
        assert!((run(MixMode::Crossfade, 1.0, 0.8, 0.2) - 0.2).abs() < 1e-6);
        assert!((run(MixMode::RingMod, 1.0, 0.5, 0.4) - 0.2).abs() < 1e-6);
    }
}
