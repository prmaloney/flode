//! The gain stage: a level, a balance, and a polarity flip on one audio input. Every voice runs
//! its own, so a gain that an envelope drives follows each note's own envelope. The path eases
//! toward its settings, which is why it keeps state. See `dsp::gain::Utility`.

use super::{AudioStage, StartContext, Tick};
use crate::dsp::frame::Frame;
use crate::dsp::gain::Utility;
use crate::patch::stage::{GainParams, MAX_INPUTS};

pub struct Gain;

impl AudioStage for Gain {
    type Params = GainParams;
    type State = Utility;

    /// Starts at the knob values, not easing in from silence.
    fn start(p: &GainParams, ctx: &mut StartContext) -> Utility {
        Utility::new(p.gain.plain(ctx.levels), p.pan.plain(ctx.levels), p.invert)
    }

    fn process(
        p: &GainParams,
        state: &mut Utility,
        inputs: &[Frame; MAX_INPUTS],
        tick: &mut Tick,
    ) -> Frame {
        state.tick(
            inputs[0],
            p.gain.plain(tick.levels),
            p.pan.plain(tick.levels),
            p.invert,
            tick.smooth,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::stage::testing::Rig;
    use crate::nodes::knobs::KnobId;
    use crate::patch::control::Param;

    fn params(gain_db: f32, pan: f32, invert: bool) -> GainParams {
        GainParams {
            gain: Param::fixed(KnobId::UtilGain, gain_db),
            pan: Param::fixed(KnobId::Pan, pan),
            invert,
        }
    }

    fn run(p: &GainParams, input: Frame) -> Frame {
        let mut rig = Rig::new();
        let mut state = Gain::start(p, &mut rig.start_context());
        Gain::process(p, &mut state, &[input, Frame::SILENT], &mut rig.tick())
    }

    #[test]
    fn a_neutral_stage_passes_the_signal() {
        assert_eq!(
            run(&params(0.0, 0.0, false), Frame::new(0.3, -0.6)),
            Frame::new(0.3, -0.6)
        );
    }

    #[test]
    fn the_stage_scales_pans_and_inverts() {
        let out = run(&params(6.0, 0.5, true), Frame::mono(0.5));
        let gain = 1.9953;
        assert!((out.l + 0.5 * gain * 0.5).abs() < 0.01, "{}", out.l);
        assert!((out.r + 0.5 * gain).abs() < 0.01, "{}", out.r);
    }

    /// Two voices' stages never share memory. One at full level does not leak into one at the
    /// minimum.
    #[test]
    fn each_voice_has_its_own_state() {
        let mut rig = Rig::new();
        let loud = params(0.0, 0.0, false);
        let quiet = params(-60.0, 0.0, false);
        let mut a = Gain::start(&loud, &mut rig.start_context());
        let mut b = Gain::start(&quiet, &mut rig.start_context());
        let input = [Frame::mono(1.0), Frame::SILENT];
        assert_eq!(Gain::process(&loud, &mut a, &input, &mut rig.tick()).l, 1.0);
        assert_eq!(
            Gain::process(&quiet, &mut b, &input, &mut rig.tick()).l,
            0.0
        );
    }
}
