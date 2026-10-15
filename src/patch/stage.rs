//! The audio stages of a patch, as plain data. A stage is one audio node: it has some audio
//! inputs and one output. A slot's stages sit in an arena in signal order, and each stage names
//! the stages that feed it by position. The engine runs the arena once per voice.

use super::control::Param;
use crate::dsp::mix::MixMode;
use crate::dsp::noise::NoiseType;
use crate::dsp::svf::FilterType;
use crate::dsp::warp::WarpMode;
use crate::dsp::wavetable::Waveform;
use crate::nodes::knobs::KnobId;
use arrayvec::ArrayVec;

/// The most stages one output slot can run. A slot with more goes silent.
pub const MAX_STAGES: usize = 16;
/// The most audio inputs one stage can have.
pub const MAX_INPUTS: usize = 2;

/// An oscillator stage: a source with no audio inputs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OscillatorParams {
    pub waveform: Waveform,
    /// The noise type, under the noise waveform.
    pub noise: NoiseType,
    pub warp: WarpMode,
    /// The envelope node that shapes this oscillator's amplitude, if one is wired in.
    pub envelope: Option<usize>,
    pub coarse: Param,
    pub fine: Param,
    pub octave: Param,
    pub unison: Param,
    pub detune: Param,
    pub phase_random: Param,
    pub warp_amount: Param,
    pub level: Param,
    /// Where the oscillator sits in the stereo field, from -1.0 (left) to 1.0 (right).
    pub pan: Param,
    /// How far the unison copies spread across the field around the pan, from 0.0 to 1.0.
    pub spread: Param,
}

impl Default for OscillatorParams {
    fn default() -> Self {
        Self {
            waveform: Waveform::default(),
            noise: NoiseType::default(),
            warp: WarpMode::default(),
            envelope: None,
            coarse: Param::default_of(KnobId::Coarse),
            fine: Param::default_of(KnobId::Fine),
            octave: Param::default_of(KnobId::Octave),
            unison: Param::default_of(KnobId::Unison),
            detune: Param::default_of(KnobId::Detune),
            phase_random: Param::default_of(KnobId::PhaseRandom),
            warp_amount: Param::default_of(KnobId::WarpAmount),
            level: Param::default_of(KnobId::OscGain),
            pan: Param::default_of(KnobId::Pan),
            spread: Param::default_of(KnobId::Spread),
        }
    }
}

/// A filter stage: one audio input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilterParams {
    pub kind: FilterType,
    pub cutoff: Param,
    pub resonance: Param,
}

impl Default for FilterParams {
    fn default() -> Self {
        Self {
            kind: FilterType::default(),
            cutoff: Param::default_of(KnobId::Cutoff),
            resonance: Param::default_of(KnobId::Resonance),
        }
    }
}

/// A mix stage: two audio inputs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MixParams {
    pub mode: MixMode,
    pub amount: Param,
}

impl Default for MixParams {
    fn default() -> Self {
        Self {
            mode: MixMode::default(),
            amount: Param::default_of(KnobId::Mix),
        }
    }
}

/// What kind of stage this is, with its settings. Adding an audio node means adding a variant
/// here and its implementation in `engine::stage`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StageParams {
    Oscillator(OscillatorParams),
    Filter(FilterParams),
    Mix(MixParams),
}

impl StageParams {
    /// How many audio inputs the stage reads. The compiler wires that many.
    pub fn input_count(&self) -> usize {
        match self {
            // An oscillator is a source, except under the FM warp, which reads a modulator.
            StageParams::Oscillator(p) => usize::from(p.warp == WarpMode::Fm),
            StageParams::Filter(_) => 1,
            StageParams::Mix(_) => 2,
        }
    }
}

/// One stage in a slot's arena, and the stages that feed it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StageSpec {
    pub params: StageParams,
    /// The arena position of the stage on each audio input, or `None` for an input with no wire.
    /// Every position is lower than this stage's own, since the arena is in signal order.
    pub inputs: [Option<u8>; MAX_INPUTS],
}

#[cfg(test)]
impl StageSpec {
    /// A stage with nothing wired to its inputs.
    pub fn new(params: StageParams) -> Self {
        Self {
            params,
            inputs: [None; MAX_INPUTS],
        }
    }

    /// The same stage with `source` wired to audio input `input`.
    pub fn with_input(mut self, input: usize, source: u8) -> Self {
        self.inputs[input] = Some(source);
        self
    }
}

/// Everything one output slot plays: its stages in signal order, ending at the stage that feeds
/// the slot, and the slot's gain.
#[derive(Debug, PartialEq)]
pub struct LaneGraph {
    pub stages: ArrayVec<StageSpec, MAX_STAGES>,
    pub gain: Param,
}

impl Clone for LaneGraph {
    fn clone(&self) -> Self {
        Self {
            stages: self.stages.clone(),
            gain: self.gain,
        }
    }

    /// Copies the stages one at a time into `self`, with no whole-lane temporary on the stack.
    fn clone_from(&mut self, source: &Self) {
        self.stages.clear();
        self.stages.extend(source.stages.iter().copied());
        self.gain = source.gain;
    }
}

impl LaneGraph {
    /// A slot with nothing wired in. It makes no sound.
    pub fn silent(gain: Param) -> Self {
        Self {
            stages: ArrayVec::new(),
            gain,
        }
    }

    /// True if the slot has any stage to run.
    pub fn is_active(&self) -> bool {
        !self.stages.is_empty()
    }

    /// The stage that feeds the slot, which is the last in the arena.
    #[cfg(test)]
    pub fn root(&self) -> Option<&StageSpec> {
        self.stages.last()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_silent_lane_has_no_root() {
        let lane = LaneGraph::silent(Param::default_of(KnobId::Slot1));
        assert!(!lane.is_active());
        assert!(lane.root().is_none());
    }

    #[test]
    fn a_spec_wires_inputs_by_position() {
        let spec = StageSpec::new(StageParams::Mix(MixParams::default()))
            .with_input(0, 2)
            .with_input(1, 5);
        assert_eq!(spec.inputs, [Some(2), Some(5)]);
    }
}
