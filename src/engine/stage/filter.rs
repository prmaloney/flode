//! The filter stage: a ladder for the low and high pass, and a state-variable filter for the band
//! pass, on one audio input. Every voice runs its own, so a cutoff driven by an envelope follows
//! each note's own envelope.

use super::{AudioStage, StartContext, Tick};
use crate::dsp::frame::Frame;
use crate::dsp::svf::{Coefficients, FilterType, Ladder, LadderCoefficients, Slope, Svf};
use crate::patch::control::ControlLevels;
use crate::patch::stage::{FilterParams, MAX_INPUTS};

pub struct Filter;

/// What one voice remembers of a filter: its integrators, and the cutoff and resonance it has
/// eased to. Easing keeps a knob move from clicking.
#[derive(Clone, Copy)]
pub struct FilterState {
    /// The band pass, one per side.
    svf: [Svf; 2],
    /// The low and high pass, one ladder per side. Its four stages give every slope.
    ladder: [Ladder; 2],
    /// The cutoff as log2 of Hz, so the easing is even across octaves.
    cutoff_log2: f32,
    resonance: f32,
}

impl FilterState {
    /// A filter at the knob values, not easing in from zero.
    pub fn new(p: &FilterParams, levels: &ControlLevels) -> Self {
        Self {
            svf: [Svf::new(); 2],
            ladder: [Ladder::new(); 2],
            cutoff_log2: p.cutoff.plain(levels).log2(),
            resonance: p.resonance.plain(levels),
        }
    }

    /// One frame through the filter. The cutoff and resonance ease a fraction `smooth` of the way
    /// to the knob values each frame. The bus uses this directly, and a voice's filter stage goes
    /// through it too.
    pub fn tick(
        &mut self,
        p: &FilterParams,
        input: Frame,
        levels: &ControlLevels,
        smooth: f32,
        sample_rate: f32,
    ) -> Frame {
        let cutoff_target = p.cutoff.plain(levels);
        let resonance_target = p.resonance.plain(levels);
        self.cutoff_log2 += (cutoff_target.log2() - self.cutoff_log2) * smooth;
        self.resonance += (resonance_target - self.resonance) * smooth;
        // Both sides run the same response, so they share one set of coefficients.
        let cutoff = self.cutoff_log2.exp2();
        if !p.kind.is_ladder() {
            // The band pass, notch and peak have no slope to choose, so they run on the
            // state-variable filter.
            let coefficients = Coefficients::new(cutoff, self.resonance, sample_rate);
            let [left, right] = &mut self.svf;
            return Frame::new(
                left.tick(input.l, &coefficients, p.kind),
                right.tick(input.r, &coefficients, p.kind),
            );
        }
        let slope = Slope::from_step(p.slope.plain(levels));
        let coefficients = LadderCoefficients::new(cutoff, self.resonance, sample_rate);
        let high = p.kind == FilterType::HighPass;
        let [left, right] = &mut self.ladder;
        Frame::new(
            left.tick(input.l, &coefficients, slope, high),
            right.tick(input.r, &coefficients, slope, high),
        )
    }
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
        FilterState::new(p, ctx.levels)
    }

    fn process(
        p: &FilterParams,
        state: &mut FilterState,
        inputs: &[Frame; MAX_INPUTS],
        tick: &mut Tick,
    ) -> Frame {
        state.tick(p, inputs[0], tick.levels, tick.smooth, tick.sample_rate)
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
            let out = Filter::process(
                params,
                &mut state,
                &[Frame::mono(input), Frame::SILENT],
                &mut rig.tick(),
            );
            if n > 22050 {
                peak = peak.max(out.l.abs());
            }
        }
        peak
    }

    fn low_pass(cutoff: f32) -> FilterParams {
        FilterParams {
            kind: FilterType::LowPass,
            cutoff: Param::fixed(KnobId::Cutoff, cutoff),
            resonance: Param::fixed(KnobId::Resonance, 0.0),
            slope: Param::default_of(KnobId::Slope),
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
            Filter::process(
                &params,
                &mut a,
                &[Frame::mono(1.0), Frame::SILENT],
                &mut rig.tick(),
            );
        }
        assert_eq!(
            Filter::process(&params, &mut b, &[Frame::SILENT; 2], &mut rig.tick()),
            Frame::SILENT
        );
    }

    /// Each side has its own integrators, so a signal on the left leaves the right silent.
    #[test]
    fn the_two_sides_filter_independently() {
        let mut rig = Rig::new();
        let params = low_pass(1000.0);
        let mut state = Filter::start(&params, &mut rig.start_context());
        let mut right_peak = 0.0f32;
        let mut left_peak = 0.0f32;
        for n in 0..2000 {
            let input = Frame::new((TAU * 200.0 * n as f32 / 44100.0).sin(), 0.0);
            let out = Filter::process(
                &params,
                &mut state,
                &[input, Frame::SILENT],
                &mut rig.tick(),
            );
            left_peak = left_peak.max(out.l.abs());
            right_peak = right_peak.max(out.r.abs());
        }
        assert!(left_peak > 0.5);
        assert_eq!(right_peak, 0.0);
    }

    /// The cutoff eases toward a new knob value instead of jumping to it.
    #[test]
    fn the_cutoff_eases_toward_its_target() {
        let mut rig = Rig::new();
        let mut state = Filter::start(&low_pass(1000.0), &mut rig.start_context());
        let moved = low_pass(4000.0);
        Filter::process(&moved, &mut state, &[Frame::SILENT; 2], &mut rig.tick());
        let after_one = state.cutoff_log2.exp2();
        assert!(after_one > 1000.0 && after_one < 1100.0, "{after_one}");
        for _ in 0..2000 {
            Filter::process(&moved, &mut state, &[Frame::SILENT; 2], &mut rig.tick());
        }
        assert!((state.cutoff_log2.exp2() - 4000.0).abs() < 40.0);
    }
}
