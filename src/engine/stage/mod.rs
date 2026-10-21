//! Audio stages at run time. Each kind of stage implements `AudioStage`: it builds its per-voice
//! state when a note starts, and makes one output sample per tick from its input samples. The
//! patch describes a slot's stages as a flat arena, and `evaluate` runs the arena for one voice.
//!
//! Dispatch goes through `StageParams` and `StageState`, two closed enums, so the audio thread
//! never allocates or calls through a trait object.

mod filter;
mod gain;
mod mix;
mod oscillator;

use crate::dsp::delay::DelayLine;
use crate::dsp::envelope::Adsr;
use crate::dsp::frame::Frame;
use crate::dsp::gain::Utility;
use crate::dsp::rng::Rng;
use crate::dsp::wavetable::WaveTables;
use crate::patch::control::{ControlLevels, ENV_SLOTS};
use crate::patch::stage::{LaneGraph, StageParams, MAX_INPUTS, MAX_STAGES};

pub use filter::{Filter, FilterState};
pub use gain::Gain;
pub use mix::{Mix, MixState};
pub use oscillator::{Oscillator, OscillatorState};

/// The delay lines each voice owns. An FM mix stage takes four at the start of a note, a pair for
/// each of its two signals, so a voice runs FM on a quarter this many mix stages. A mix past that
/// passes signal A through. The lines are built before the audio thread runs, since a delay line
/// allocates.
pub const MAX_DELAY_LINES: usize = 8;

/// One kind of audio node, run once per voice. `Params` is what the patch says about the stage,
/// and `State` is what a voice remembers between samples.
pub trait AudioStage {
    type Params;
    type State;

    /// Builds a voice's state for this stage when a note starts.
    fn start(params: &Self::Params, ctx: &mut StartContext) -> Self::State;

    /// One output frame. `inputs` holds the frames on the stage's audio inputs, with silence on
    /// an input that has no wire.
    fn process(
        params: &Self::Params,
        state: &mut Self::State,
        inputs: &[Frame; MAX_INPUTS],
        tick: &mut Tick,
    ) -> Frame;
}

/// What a stage reads when a note starts.
pub struct StartContext<'a> {
    pub levels: &'a ControlLevels,
    pub rng: &'a mut Rng,
    /// The pitch the voice starts at, as a phase step per sample.
    pub pitch_step: f32,
    /// The delay lines of the voice being started.
    delays: &'a mut [DelayLine],
    /// How many of them stages have taken so far.
    next_delay: usize,
}

impl<'a> StartContext<'a> {
    pub fn new(
        levels: &'a ControlLevels,
        rng: &'a mut Rng,
        pitch_step: f32,
        delays: &'a mut [DelayLine],
    ) -> Self {
        Self {
            levels,
            rng,
            pitch_step,
            delays,
            next_delay: 0,
        }
    }

    /// Hands out the next `N` free delay lines, cleared, or None if the voice has fewer than `N`
    /// left. It takes all of them or none.
    pub fn take_delays<const N: usize>(&mut self) -> Option<[usize; N]> {
        let first = self.next_delay;
        let lines = self.delays.get_mut(first..first + N)?;
        lines.iter_mut().for_each(DelayLine::clear);
        self.next_delay += N;
        Some(std::array::from_fn(|i| first + i))
    }
}

/// What every stage reads while it runs for one voice and one sample.
pub struct Tick<'a> {
    /// The control levels this voice sees: its own envelopes, and the shared LFO and MIDI levels.
    pub levels: &'a ControlLevels,
    /// The voice's envelope copies, one per envelope node and then the bypass copy.
    pub envs: &'a [Adsr; ENV_SLOTS],
    pub velocity: f32,
    /// The voice's pitch, as a phase step per sample.
    pub pitch_step: f32,
    /// True while any envelope node is in the patch.
    pub envelopes_active: bool,
    pub tables: &'a WaveTables,
    pub sample_rate: f32,
    /// The share of the gap a smoothed value closes each sample.
    pub smooth: f32,
    pub rng: &'a mut Rng,
    /// The voice's delay lines, which stages index by the number they took at the start.
    pub delays: &'a mut [DelayLine],
}

/// A voice's memory for one stage.
#[derive(Clone, Copy)]
pub enum StageState {
    /// A slot of the arena that holds no stage.
    Empty,
    Oscillator(OscillatorState),
    Filter(FilterState),
    Mix(MixState),
    Gain(Utility),
}

impl StageState {
    /// The state of a voice's whole arena before any note has started.
    pub const ARENA: [StageState; MAX_STAGES] = [StageState::Empty; MAX_STAGES];

    /// Builds the state a stage starts a note with.
    pub fn start(params: &StageParams, ctx: &mut StartContext) -> Self {
        match params {
            StageParams::Oscillator(p) => Self::Oscillator(Oscillator::start(p, ctx)),
            StageParams::Filter(p) => Self::Filter(Filter::start(p, ctx)),
            StageParams::Mix(p) => Self::Mix(Mix::start(p, ctx)),
            StageParams::Gain(p) => Self::Gain(Gain::start(p, ctx)),
            // An effect runs once on the summed voices, so a voice keeps nothing for it.
            _ => Self::Empty,
        }
    }

    /// One frame from a stage. A state that does not match its stage makes silence, which only
    /// happens if a patch changes under a voice. The next note starts clean.
    pub fn process(
        &mut self,
        params: &StageParams,
        inputs: &[Frame; MAX_INPUTS],
        tick: &mut Tick,
    ) -> Frame {
        match (params, self) {
            (StageParams::Oscillator(p), Self::Oscillator(s)) => {
                Oscillator::process(p, s, inputs, tick)
            }
            (StageParams::Filter(p), Self::Filter(s)) => Filter::process(p, s, inputs, tick),
            (StageParams::Mix(p), Self::Mix(s)) => Mix::process(p, s, inputs, tick),
            (StageParams::Gain(p), Self::Gain(s)) => Gain::process(p, s, inputs, tick),
            _ => Frame::SILENT,
        }
    }
}

/// Starts every stage of a slot for a new voice.
pub fn start_arena(graph: &LaneGraph, ctx: &mut StartContext) -> [StageState; MAX_STAGES] {
    let mut states = StageState::ARENA;
    for (state, spec) in states.iter_mut().zip(&graph.stages) {
        // A bus stage runs once on the summed voices, so a voice keeps nothing for it. Skipping
        // it also keeps a bus mix from taking delay lines the voice's own stages could use.
        if !spec.bus {
            *state = StageState::start(&spec.params, ctx);
        }
    }
    states
}

/// Runs a slot's per-voice stages for one voice and one sample, and returns what the last stage
/// makes. The arena is in signal order, so each stage finds its inputs already computed.
///
/// Stages on the bus are skipped, since they run once on the summed voices. A stage that a bus
/// stage reads adds its output to `taps`, which holds the sum over the voices so far. When the
/// last stage is on the bus, the voice itself makes silence, and its sound reaches the output
/// through the taps.
pub fn evaluate(
    graph: &LaneGraph,
    states: &mut [StageState; MAX_STAGES],
    tick: &mut Tick,
    taps: &mut [Frame; MAX_STAGES],
) -> Frame {
    let mut outputs = [Frame::SILENT; MAX_STAGES];
    for (index, spec) in graph.stages.iter().enumerate() {
        if spec.bus {
            continue;
        }
        let inputs = spec
            .inputs
            .map(|source| source.map_or(Frame::SILENT, |i| outputs[i as usize]));
        outputs[index] = states[index].process(&spec.params, &inputs, tick);
        if spec.tap {
            taps[index] += outputs[index];
        }
    }
    graph
        .stages
        .len()
        .checked_sub(1)
        .map_or(Frame::SILENT, |root| outputs[root])
}

#[cfg(test)]
pub(crate) mod testing {
    //! Owns what a `Tick` borrows, so a test can run a stage without an engine around it.

    use super::*;
    use crate::dsp::envelope::Adsr;

    pub struct Rig {
        pub levels: ControlLevels,
        pub envs: [Adsr; ENV_SLOTS],
        pub tables: WaveTables,
        pub rng: Rng,
        pub pitch_step: f32,
        pub delays: Vec<DelayLine>,
    }

    impl Rig {
        /// A rig at 440 Hz and 44.1 kHz, with the bypass envelope open.
        pub fn new() -> Self {
            let mut envs = [Adsr::START; ENV_SLOTS];
            envs[ENV_SLOTS - 1].level = 1.0;
            Self {
                levels: ControlLevels::default(),
                envs,
                tables: WaveTables::new(),
                rng: Rng::default(),
                pitch_step: 440.0 / 44100.0,
                delays: (0..MAX_DELAY_LINES).map(|_| DelayLine::new()).collect(),
            }
        }

        pub fn start_context(&mut self) -> StartContext<'_> {
            StartContext::new(
                &self.levels,
                &mut self.rng,
                self.pitch_step,
                &mut self.delays,
            )
        }

        pub fn tick(&mut self) -> Tick<'_> {
            Tick {
                levels: &self.levels,
                envs: &self.envs,
                velocity: 1.0,
                pitch_step: self.pitch_step,
                envelopes_active: false,
                tables: &self.tables,
                sample_rate: 44100.0,
                smooth: 0.01,
                rng: &mut self.rng,
                delays: &mut self.delays,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::Rig;
    use super::*;
    use crate::dsp::mix::MixMode;
    use crate::dsp::wavetable::Waveform;
    use crate::nodes::knobs::KnobId;
    use crate::patch::control::Param;
    use crate::patch::stage::{FilterParams, GainParams, MixParams, OscillatorParams, StageSpec};

    fn osc(waveform: Waveform) -> StageSpec {
        StageSpec::new(StageParams::Oscillator(OscillatorParams {
            waveform,
            ..OscillatorParams::default()
        }))
    }

    fn low_pass(cutoff: f32, source: u8) -> StageSpec {
        StageSpec::new(StageParams::Filter(FilterParams {
            cutoff: Param::fixed(KnobId::Cutoff, cutoff),
            resonance: Param::fixed(KnobId::Resonance, 0.0),
            ..FilterParams::default()
        }))
        .with_input(0, source)
    }

    fn mix(mode: MixMode, amount: f32, a: u8, b: u8) -> StageSpec {
        StageSpec::new(StageParams::Mix(MixParams {
            mode,
            amount: Param::fixed(KnobId::Mix, amount),
        }))
        .with_input(0, a)
        .with_input(1, b)
    }

    fn lane(stages: &[StageSpec]) -> LaneGraph {
        let mut lane = LaneGraph::silent(Param::default_of(KnobId::Slot1));
        lane.stages.extend(stages.iter().copied());
        lane
    }

    /// Runs one voice of a lane for `samples` and returns the peak after the first half.
    fn peak(lane: &LaneGraph, samples: usize) -> f32 {
        let mut rig = Rig::new();
        let mut states = start_arena(lane, &mut rig.start_context());
        let mut peak = 0.0f32;
        for n in 0..samples {
            let out = evaluate(
                lane,
                &mut states,
                &mut rig.tick(),
                &mut [Frame::SILENT; MAX_STAGES],
            );
            if n >= samples / 2 {
                peak = peak.max(out.peak());
            }
        }
        peak
    }

    #[test]
    fn an_empty_lane_is_silent() {
        assert_eq!(peak(&lane(&[]), 1000), 0.0);
    }

    /// A single oscillator reaches the output.
    #[test]
    fn a_lone_oscillator_plays() {
        assert!(peak(&lane(&[osc(Waveform::Sine)]), 4000) > 0.9);
    }

    /// A filter sits inside one branch of a mix. The old chain could not do this. At zero mix the
    /// output is branch A, which the low pass has all but removed.
    #[test]
    fn a_filter_can_sit_inside_a_mix_branch() {
        let stages = |amount| {
            lane(&[
                osc(Waveform::Sine),
                osc(Waveform::Square),
                low_pass(60.0, 0),
                mix(MixMode::Crossfade, amount, 2, 1),
            ])
        };
        let filtered_a = peak(&stages(0.0), 8000);
        let plain_b = peak(&stages(1.0), 8000);
        assert!(filtered_a < 0.1, "{filtered_a}");
        assert!(plain_b > 0.9, "{plain_b}");
    }

    /// Two filters in a row cut more than one.
    #[test]
    fn filters_in_series_both_run() {
        let one = lane(&[osc(Waveform::Sine), low_pass(200.0, 0)]);
        let two = lane(&[osc(Waveform::Sine), low_pass(200.0, 0), low_pass(200.0, 1)]);
        assert!(peak(&two, 8000) < peak(&one, 8000) * 0.7);
    }

    /// An input with no wire is silence, so a crossfade to it fades the other signal out.
    #[test]
    fn an_unwired_input_is_silent() {
        let unwired = StageSpec::new(StageParams::Mix(MixParams {
            mode: MixMode::Crossfade,
            amount: Param::fixed(KnobId::Mix, 1.0),
        }))
        .with_input(0, 0);
        assert!(peak(&lane(&[osc(Waveform::Sine), unwired]), 4000) < 1e-3);
    }

    /// One oscillator wired to both inputs runs once and feeds both. Ring modulating it with
    /// itself can only be positive.
    #[test]
    fn one_stage_can_feed_two_inputs() {
        let lane = lane(&[osc(Waveform::Sine), mix(MixMode::RingMod, 1.0, 0, 0)]);
        let mut rig = Rig::new();
        let mut states = start_arena(&lane, &mut rig.start_context());
        let lowest = (0..2000)
            .map(|_| {
                evaluate(
                    &lane,
                    &mut states,
                    &mut rig.tick(),
                    &mut [Frame::SILENT; MAX_STAGES],
                )
                .l
            })
            .fold(f32::MAX, f32::min);
        assert!(lowest >= 0.0, "{lowest}");
    }

    /// A voice's state for a stage that no longer matches its params makes silence, not a panic.
    #[test]
    fn a_mismatched_state_is_silent() {
        let mut rig = Rig::new();
        let params = StageParams::Mix(MixParams::default());
        let mut state = StageState::Empty;
        assert_eq!(
            state.process(&params, &[Frame::mono(1.0); 2], &mut rig.tick()),
            Frame::SILENT
        );
    }

    /// Every stage kind reports how many inputs the compiler should wire.
    #[test]
    fn each_stage_declares_its_inputs() {
        let osc = StageParams::Oscillator(OscillatorParams::default());
        let filter = StageParams::Filter(FilterParams::default());
        let mix = StageParams::Mix(MixParams::default());
        let gain = StageParams::Gain(GainParams::default());
        assert_eq!(osc.input_count(), 0);
        assert_eq!(gain.input_count(), 1);
        assert_eq!(filter.input_count(), 1);
        assert_eq!(mix.input_count(), 2);
    }
}
