//! The audio stages of a patch, as plain data. A stage is one audio node: it has some audio
//! inputs and one output. A slot's stages sit in an arena in signal order, and each stage names
//! the stages that feed it by position. The engine runs the per-voice stages of the arena once
//! per voice. The stages on the bus run once, on the sum of the voices.

use super::control::Param;
use crate::dsp::effects::distortion::DistortionKind;
use crate::dsp::effects::eq::{EqKind, BANDS};
use crate::dsp::mix::MixMode;
use crate::dsp::monitor::MonitorMode;
use crate::dsp::noise::NoiseType;
use crate::dsp::sample::SampleData;
use crate::dsp::svf::FilterType;
use crate::dsp::warp::WarpMode;
use crate::dsp::wavetable::Waveform;
use crate::nodes::knobs::KnobId;
use crate::nodes::sampler::LoopMode;
use arrayvec::ArrayVec;
use std::sync::Arc;

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
    /// The note-source slot that feeds this oscillator, if its Notes pin is wired. It can be a
    /// MIDI input passed straight through or an arpeggiator reading one. None means nothing is
    /// wired to the Notes pin, and the oscillator compiles out entirely rather than playing silent.
    pub notes: Option<usize>,
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
            notes: None,
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

/// A sampler stage: a source with no audio inputs, like an oscillator, but reading from a loaded
/// file instead of a wavetable.
#[derive(Debug, Clone)]
pub struct SamplerParams {
    /// The loaded audio, shared with the editor's copy through an `Arc`, so a patch clone never
    /// copies the samples themselves. None plays silence.
    pub data: Option<Arc<SampleData>>,
    /// The envelope node that shapes this voice's amplitude, if one is wired in.
    pub envelope: Option<usize>,
    /// The note-source slot that feeds this sampler, if its notes pin is wired to one. See
    /// `OscillatorParams.notes`.
    pub notes: Option<usize>,
    /// This Sampler's place among the Sampler nodes in graph order, or None past the Samplers the
    /// engine tracks a playhead for. Used only to report the newest voice's position back to the
    /// viewer; it plays either way.
    pub sampler_slot: Option<usize>,
    /// The root key: the MIDI note the file plays back at its recorded pitch.
    pub root: Param,
    /// True to change playback speed with the note. False always plays at the file's own speed.
    pub pitch_tracking: bool,
    /// Where playback starts on every new note, from 0.0 to 100.0 percent of the file's length.
    pub start: Param,
    pub loop_mode: LoopMode,
    /// The loop's bounds, each from 0.0 to 100.0 percent of the file's length.
    pub loop_start: Param,
    pub loop_end: Param,
    /// True to ignore note off and play through to the end or loop forever. False stops on the
    /// envelope's release, like an oscillator.
    pub one_shot: bool,
    /// True to play the file backward.
    pub reverse: bool,
    pub level: Param,
    pub pan: Param,
}

impl PartialEq for SamplerParams {
    /// `SampleData` is large, so instead of comparing its contents, two params are equal only if
    /// they share the same loaded data, by `Arc` pointer, or both have none.
    fn eq(&self, other: &Self) -> bool {
        let same_data = match (&self.data, &other.data) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        same_data
            && self.envelope == other.envelope
            && self.notes == other.notes
            && self.sampler_slot == other.sampler_slot
            && self.root == other.root
            && self.pitch_tracking == other.pitch_tracking
            && self.start == other.start
            && self.loop_mode == other.loop_mode
            && self.loop_start == other.loop_start
            && self.loop_end == other.loop_end
            && self.one_shot == other.one_shot
            && self.reverse == other.reverse
            && self.level == other.level
            && self.pan == other.pan
    }
}

impl Default for SamplerParams {
    fn default() -> Self {
        Self {
            data: None,
            envelope: None,
            notes: None,
            sampler_slot: None,
            root: Param::default_of(KnobId::SamplerRoot),
            pitch_tracking: true,
            start: Param::default_of(KnobId::SamplerStart),
            loop_mode: LoopMode::Off,
            loop_start: Param::default_of(KnobId::SamplerLoopStart),
            loop_end: Param::default_of(KnobId::SamplerLoopEnd),
            one_shot: false,
            reverse: false,
            level: Param::default_of(KnobId::OscGain),
            pan: Param::default_of(KnobId::Pan),
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

/// A gain stage: one audio input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GainParams {
    /// The level in dB.
    pub gain: Param,
    /// The balance, from -1.0 (left) to 1.0 (right).
    pub pan: Param,
    pub invert: bool,
}

impl Default for GainParams {
    fn default() -> Self {
        Self {
            gain: Param::default_of(KnobId::UtilGain),
            pan: Param::default_of(KnobId::Pan),
            invert: false,
        }
    }
}

/// A distortion stage: one audio input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistortionParams {
    pub kind: DistortionKind,
    pub drive: Param,
    pub tone: Param,
    pub level: Param,
    pub mix: Param,
}

/// A compressor stage: one audio input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompressorParams {
    pub threshold: Param,
    pub ratio: Param,
    pub attack: Param,
    pub release: Param,
    pub makeup: Param,
    pub mix: Param,
    pub multiband: bool,
    pub depth: Param,
}

/// A reverb stage: one audio input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReverbParams {
    pub decay: Param,
    pub size: Param,
    pub damp: Param,
    pub mix: Param,
}

/// A delay stage: one audio input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DelayParams {
    pub time: Param,
    /// The Sync knob. Zero uses the time. A step past it is a note division at the host's tempo.
    pub sync: Param,
    pub feedback: Param,
    pub damp: Param,
    pub mix: Param,
    pub ping_pong: bool,
}

/// A chorus stage: one audio input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChorusParams {
    pub rate: Param,
    pub depth: Param,
    pub spread: Param,
    pub mix: Param,
}

/// A phaser stage: one audio input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhaserParams {
    pub rate: Param,
    pub depth: Param,
    pub feedback: Param,
    pub center: Param,
    pub stages: usize,
    pub spread: Param,
    pub mix: Param,
}

/// A flanger stage: one audio input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlangerParams {
    pub rate: Param,
    pub depth: Param,
    pub feedback: Param,
    pub delay: Param,
    pub spread: Param,
    pub mix: Param,
}

/// One band of an EQ stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqBandParams {
    pub kind: EqKind,
    pub freq: Param,
    pub gain: Param,
    pub q: Param,
}

/// An EQ stage: one audio input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqParams {
    pub bands: [EqBandParams; BANDS],
    /// True to measure the output for the node's spectrum display.
    pub analyzer: bool,
}

/// A monitor stage: one audio input and no output. It passes its input on unchanged and measures
/// it for the node's display.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MonitorParams {
    pub mode: MonitorMode,
}

/// What kind of stage this is, with its settings. Adding an audio node means adding a variant
/// here and its implementation in `engine::stage`, or in `engine::effects` for an effect.
/// `StageParams` holds a `SamplerParams`, which holds an `Option<Arc<SampleData>>`, so the whole
/// enum can only be `Clone`, not `Copy`. Cloning it is still cheap: every other variant is plain
/// numbers, and a Sampler's clone is only an `Arc` pointer copy.
#[derive(Debug, Clone, PartialEq)]
pub enum StageParams {
    Oscillator(OscillatorParams),
    Sampler(SamplerParams),
    Filter(FilterParams),
    Mix(MixParams),
    Gain(GainParams),
    Distortion(DistortionParams),
    Compressor(CompressorParams),
    Reverb(ReverbParams),
    Delay(DelayParams),
    Chorus(ChorusParams),
    Phaser(PhaserParams),
    Flanger(FlangerParams),
    Eq(EqParams),
    Monitor(MonitorParams),
}

impl StageParams {
    /// How many audio inputs the stage reads. The compiler wires that many.
    pub fn input_count(&self) -> usize {
        match self {
            // An oscillator and a sampler are sources.
            StageParams::Oscillator(_) | StageParams::Sampler(_) => 0,
            StageParams::Filter(_) => 1,
            StageParams::Mix(_) => 2,
            _ => 1,
        }
    }

    /// True for the effects. They always run once on a slot's summed voices, on the bus. A filter,
    /// a mix, or a gain runs per voice, unless something on the bus feeds it. Then it runs on the
    /// bus too.
    pub fn is_effect(&self) -> bool {
        !matches!(
            self,
            StageParams::Oscillator(_)
                | StageParams::Sampler(_)
                | StageParams::Filter(_)
                | StageParams::Mix(_)
                | StageParams::Gain(_)
        )
    }
}

/// One stage in a slot's arena, and the stages that feed it.
#[derive(Debug, Clone, PartialEq)]
pub struct StageSpec {
    pub params: StageParams,
    /// The arena position of the stage on each audio input, or `None` for an input with no wire.
    /// Every position is lower than this stage's own, since the arena is in signal order.
    pub inputs: [Option<u8>; MAX_INPUTS],
    /// True for a stage that runs once on the summed voices, on the bus, in place of once per
    /// voice. Every effect is on the bus. So is any stage that reads one, directly or through
    /// other stages, since by then the voices are already added together. An oscillator is never
    /// on the bus.
    pub bus: bool,
    /// True for a per-voice stage that a bus stage reads. The engine adds its output across the
    /// voices to feed the bus stage.
    pub tap: bool,
    /// Which instance of its kind a bus stage uses. The engine keeps a few of each effect kind,
    /// of the filter, and of the gain, and a stage past the last one passes its input through.
    /// Unused by the per-voice stages and by a bus mix, which has no memory.
    pub pool: u8,
}

#[cfg(test)]
impl StageSpec {
    /// A stage with nothing wired to its inputs. An effect starts on the bus, and the other kinds
    /// run per voice until `on_bus` moves them.
    pub fn new(params: StageParams) -> Self {
        let bus = params.is_effect();
        Self {
            params,
            inputs: [None; MAX_INPUTS],
            bus,
            tap: false,
            pool: 0,
        }
    }

    /// The same stage run on the bus.
    pub fn on_bus(mut self) -> Self {
        self.bus = true;
        self
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
    /// The note-source slot that triggers this slot's voices, if the compiler could tell. It comes
    /// from the first oscillator in the slot whose Notes pin is wired; a slot that mixes oscillators
    /// from different note sources only follows the first one found, which is a known limitation.
    /// `None` means the compiler made no such call, which happens for a slot built by hand rather
    /// than by `Wiring`, such as in a test or the no-alloc check: then the engine falls back to its
    /// old behavior of triggering the slot from every host note, same as before this field existed.
    pub notes_slot: Option<usize>,
}

impl Clone for LaneGraph {
    fn clone(&self) -> Self {
        Self {
            stages: self.stages.clone(),
            gain: self.gain,
            notes_slot: self.notes_slot,
        }
    }

    /// Copies the stages one at a time into `self`, with no whole-lane temporary on the stack.
    fn clone_from(&mut self, source: &Self) {
        self.stages.clear();
        self.stages.extend(source.stages.iter().cloned());
        self.gain = source.gain;
        self.notes_slot = source.notes_slot;
    }
}

impl LaneGraph {
    /// A slot with nothing wired in. It makes no sound.
    pub fn silent(gain: Param) -> Self {
        Self {
            stages: ArrayVec::new(),
            gain,
            notes_slot: None,
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
