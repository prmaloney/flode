//! The filter stage: a state-variable filter on one audio input. Every voice runs its own, so a
//! cutoff driven by an envelope follows each note's own envelope.

use super::{AudioStage, StartContext, Tick};
use crate::dsp::svf::Svf;
use crate::patch::stage::{FilterParams, MAX_INPUTS};

pub struct Filter;

/// What one voice remembers of a filter: its integrators, and the cutoff and resonance it has
/// eased to. Easing keeps a knob move from clicking.
#[derive(Clone, Copy)]
pub struct FilterState {
    svf: Svf,
    /// The cutoff as log2 of Hz, so the easing is even across octaves.
    cutoff_log2: f32,
    resonance: f32,
}

#[cfg(test)]
impl FilterState {
    /// The cutoff the filter has eased to, in Hz.
    pub fn cutoff_hz(&self) -> f32 {
        self.cutoff_log2.exp2()
    }
}

impl AudioStage for Filter {
    type Params = FilterParams;
    type State = FilterState;

    /// Starts at the knob values, not easing in from zero.
    fn start(p: &FilterParams, ctx: &mut StartContext) -> FilterState {
        FilterState {
            svf: Svf::new(),
            cutoff_log2: p.cutoff.plain(ctx.levels).log2(),
            resonance: p.resonance.plain(ctx.levels),
        }
    }

    fn process(
        p: &FilterParams,
        state: &mut FilterState,
        inputs: &[f32; MAX_INPUTS],
        tick: &mut Tick,
    ) -> f32 {
        let cutoff_target = p.cutoff.plain(tick.levels);
        let resonance_target = p.resonance.plain(tick.levels);
        state.cutoff_log2 += (cutoff_target.log2() - state.cutoff_log2) * tick.smooth;
        state.resonance += (resonance_target - state.resonance) * tick.smooth;
        state.svf.process(
            inputs[0],
            state.cutoff_log2.exp2(),
            state.resonance,
            p.kind,
            tick.sample_rate,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::svf::FilterType;
    use crate::engine::stage::testing::Rig;
    use crate::nodes::knobs::KnobId;
    use crate::patch::control::Param;
    use std::f32::consts::TAU;

    /// The peak output for a sine at `freq`, after the filter has settled.
    fn peak(params: &FilterParams, freq: f32) -> f32 {
        let mut rig = Rig::new();
        let mut state = Filter::start(params, &mut rig.start_context());
        let mut peak = 0.0f32;
        for n in 0..44100 {
            let input = (TAU * freq * n as f32 / 44100.0).sin();
            let out = Filter::process(params, &mut state, &[input, 0.0], &mut rig.tick());
            if n > 22050 {
                peak = peak.max(out.abs());
            }
        }
        peak
    }

    fn low_pass(cutoff: f32) -> FilterParams {
        FilterParams {
            kind: FilterType::LowPass,
            cutoff: Param::fixed(KnobId::Cutoff, cutoff),
            resonance: Param::fixed(KnobId::Resonance, 0.0),
        }
    }

    #[test]
    fn a_low_pass_stage_passes_lows_and_cuts_highs() {
        assert!(peak(&low_pass(1000.0), 100.0) > 0.9);
        assert!(peak(&low_pass(1000.0), 10000.0) < 0.2);
    }

    /// Two voices' filters never share memory.
    #[test]
    fn each_voice_has_its_own_filter_state() {
        let mut rig = Rig::new();
        let params = low_pass(1000.0);
        let mut a = Filter::start(&params, &mut rig.start_context());
        let mut b = Filter::start(&params, &mut rig.start_context());
        for _ in 0..100 {
            Filter::process(&params, &mut a, &[1.0, 0.0], &mut rig.tick());
        }
        assert_eq!(
            Filter::process(&params, &mut b, &[0.0, 0.0], &mut rig.tick()),
            0.0
        );
    }

    /// The cutoff eases toward a new knob value instead of jumping to it.
    #[test]
    fn the_cutoff_eases_toward_its_target() {
        let mut rig = Rig::new();
        let mut state = Filter::start(&low_pass(1000.0), &mut rig.start_context());
        let moved = low_pass(4000.0);
        Filter::process(&moved, &mut state, &[0.0, 0.0], &mut rig.tick());
        let after_one = state.cutoff_log2.exp2();
        assert!(after_one > 1000.0 && after_one < 1100.0, "{after_one}");
        for _ in 0..2000 {
            Filter::process(&moved, &mut state, &[0.0, 0.0], &mut rig.tick());
        }
        assert!((state.cutoff_log2.exp2() - 4000.0).abs() < 40.0);
    }
}
