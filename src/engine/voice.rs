//! One sounding note: its pitch, its envelopes, and the state of every stage in its slot.

use super::stage::{evaluate, StageState, Tick};
use crate::dsp::delay::DelayLine;
use crate::dsp::envelope::{Adsr, AdsrConfig};
use crate::dsp::frame::Frame;
use crate::dsp::rng::Rng;
use crate::dsp::wavetable::WaveTables;
use crate::nodes::input::MIDI_SOURCE_COUNT;
use crate::nodes::macros::MACRO_COUNT;
use crate::patch::control::{ControlLevels, SignalTable, ENV_SLOTS, MAX_ENVELOPES, MAX_LFOS};
use crate::patch::stage::{LaneGraph, MAX_STAGES};

/// What a voice reads from the engine for one sample.
pub struct RenderEnv<'a> {
    pub tables: &'a WaveTables,
    pub lfo_levels: [f32; MAX_LFOS],
    pub midi_levels: [f32; MIDI_SOURCE_COUNT],
    pub macro_levels: [f32; MACRO_COUNT],
    /// The patch's map and math nodes, which each voice works out again from its own envelopes.
    pub signals: &'a SignalTable,
    /// True while any envelope node is in the patch.
    pub envelopes_active: bool,
    pub sample_rate: f32,
    /// The share of the gap a smoothed value closes each sample.
    pub smooth: f32,
    pub rng: &'a mut Rng,
}

pub struct Voice {
    pub(super) note: u8,
    /// The pitch the voice is moving toward, and the pitch it is at, as phase steps per sample.
    pub(super) target_step: f32,
    pub(super) step: f32,
    pub(super) velocity: f32,
    /// One copy of each envelope node, then the bypass copy last.
    pub(super) envs: [Adsr; ENV_SLOTS],
    /// Monotonic stamp of when this voice started, used to steal the oldest voice.
    pub(super) age: u64,
    /// What each stage of the voice's slot remembers.
    pub(super) stages: [StageState; MAX_STAGES],
}

impl Voice {
    pub fn new(
        note: u8,
        velocity: f32,
        step: f32,
        target_step: f32,
        age: u64,
        stages: [StageState; MAX_STAGES],
    ) -> Self {
        Self {
            note,
            target_step,
            step,
            velocity,
            envs: [Adsr::START; ENV_SLOTS],
            age,
            stages,
        }
    }

    /// Moves every envelope copy on by one sample, each with its own times.
    pub fn advance(&mut self, configs: &[AdsrConfig; ENV_SLOTS]) {
        for (env, config) in self.envs.iter_mut().zip(configs) {
            env.advance(config);
        }
    }

    /// This voice's level for each envelope node.
    pub fn env_levels(&self) -> [f32; MAX_ENVELOPES] {
        std::array::from_fn(|i| self.envs[i].level)
    }

    /// True once every envelope has started its release.
    pub fn releasing(&self) -> bool {
        self.envs.iter().all(Adsr::releasing)
    }

    /// True once every envelope copy has run out. A voice lives until its longest release ends,
    /// so a driver envelope's release is heard even when the amplitude is the short bypass fade.
    pub fn finished(&self) -> bool {
        self.envs.iter().all(Adsr::finished)
    }

    /// Moves the pitch a fraction `coef` of the way to the target. A coef of 1.0 jumps there.
    pub fn glide(&mut self, coef: f32) {
        self.step += (self.target_step - self.step) * coef;
    }

    /// One sample of the voice: its slot's stages, run in signal order. `delays` are the voice's
    /// own delay lines. `taps` collects what the effects in the slot read from the voices.
    pub fn render(
        &mut self,
        graph: &LaneGraph,
        env: &mut RenderEnv,
        delays: &mut [DelayLine],
        taps: &mut [Frame; MAX_STAGES],
    ) -> Frame {
        // A voice's own envelopes drive its knobs, so each note moves on its own.
        let mut levels = ControlLevels {
            envelope: self.env_levels(),
            lfo: env.lfo_levels,
            midi: env.midi_levels,
            macros: env.macro_levels,
            ..ControlLevels::default()
        };
        // A map or math node that reads an envelope follows this voice's own envelope.
        env.signals.evaluate(&mut levels);
        let mut tick = Tick {
            levels: &levels,
            envs: &self.envs,
            velocity: self.velocity,
            pitch_step: self.step,
            envelopes_active: env.envelopes_active,
            tables: env.tables,
            sample_rate: env.sample_rate,
            smooth: env.smooth,
            rng: env.rng,
            delays,
        };
        evaluate(graph, &mut self.stages, &mut tick, taps)
    }
}
