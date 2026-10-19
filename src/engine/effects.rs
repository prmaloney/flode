//! The bus of every slot: the stages that run once per sample on the slot's summed voices, after
//! the per-voice stages. These are the effects, and any filter or mix that something on the bus
//! feeds. The engine keeps a few instances of each effect kind and of the filter, built when the
//! sample rate is set, since their memory can't be allocated on the audio thread. A patch hands
//! each such stage an instance by its `pool` number.
//!
//! This module reads the knobs and passes plain numbers to the effects in `dsp::effects`. It does
//! no signal processing of its own, apart from combining the two signals of a mix.

use crate::dsp::analyzer::{BINS as SPECTRUM_BINS, FLOOR_DB};
use crate::dsp::effects::chorus::{Chorus, ChorusParams};
use crate::dsp::effects::compressor::{Compressor, CompressorParams};
use crate::dsp::effects::delay::{Delay, DelayParams};
use crate::dsp::effects::distortion::{Distortion, DistortionParams};
use crate::dsp::effects::eq::{Eq, EqBand, EqParams, BANDS};
use crate::dsp::effects::flanger::{Flanger, FlangerParams};
use crate::dsp::effects::phaser::{Phaser, PhaserParams};
use crate::dsp::effects::reverb::{Reverb, ReverbParams};
use crate::dsp::frame::Frame;
use crate::dsp::mix::MixMode;
use crate::engine::stage::FilterState;
use crate::nodes::lfo::cycles_per_beat;
use crate::patch::control::ControlLevels;
use crate::patch::stage::{LaneGraph, StageParams, MAX_INPUTS, MAX_STAGES};

/// How many instances of each effect kind the engine keeps. An effect stage past this many, in
/// graph order across all slots, passes its input through.
pub const POOL: usize = 4;

/// The longest delay time, in ms, which the delay's Time knob also stops at.
const MAX_DELAY_MS: f32 = 2000.0;

pub struct Effects {
    sample_rate: f32,
    distortion: Vec<Distortion>,
    compressor: Vec<Compressor>,
    reverb: Vec<Reverb>,
    delay: Vec<Delay>,
    chorus: Vec<Chorus>,
    phaser: Vec<Phaser>,
    flanger: Vec<Flanger>,
    eq: Vec<Eq>,
    /// The filters that run on the bus. One starts at its knob values the first time it plays,
    /// so it holds `None` until then.
    filter: Vec<Option<FilterState>>,
}

impl Effects {
    /// Builds every instance for this sample rate. It allocates, so it runs when the sample rate
    /// is set and never on the audio thread.
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            filter: vec![None; POOL],
            distortion: (0..POOL).map(|_| Distortion::new(sample_rate)).collect(),
            compressor: (0..POOL).map(|_| Compressor::new(sample_rate)).collect(),
            reverb: (0..POOL).map(|_| Reverb::new(sample_rate)).collect(),
            delay: (0..POOL).map(|_| Delay::new(sample_rate)).collect(),
            chorus: (0..POOL).map(|_| Chorus::new(sample_rate)).collect(),
            phaser: (0..POOL).map(|_| Phaser::new(sample_rate)).collect(),
            flanger: (0..POOL).map(|_| Flanger::new(sample_rate)).collect(),
            eq: (0..POOL).map(|_| Eq::new(sample_rate)).collect(),
        }
    }

    /// The spectrum each EQ instance last measured, by pool number. An instance that is not
    /// analyzing reads the floor.
    pub fn spectra(&self) -> [[f32; SPECTRUM_BINS]; POOL] {
        std::array::from_fn(|pool| {
            self.eq
                .get(pool)
                .map_or([FLOOR_DB; SPECTRUM_BINS], |eq| *eq.spectrum())
        })
    }

    /// Clears every tail and filter memory.
    pub fn reset(&mut self) {
        self.distortion.iter_mut().for_each(Distortion::reset);
        self.compressor.iter_mut().for_each(Compressor::reset);
        self.reverb.iter_mut().for_each(Reverb::reset);
        self.delay.iter_mut().for_each(Delay::reset);
        self.chorus.iter_mut().for_each(Chorus::reset);
        self.phaser.iter_mut().for_each(Phaser::reset);
        self.flanger.iter_mut().for_each(Flanger::reset);
        self.eq.iter_mut().for_each(Eq::reset);
        self.filter.iter_mut().for_each(|filter| *filter = None);
    }

    /// Runs the bus stages of one slot for one sample and returns what the last stage makes, or
    /// None when that stage runs per voice. `taps` holds the summed voices at each per-voice stage
    /// that a bus stage reads, `smooth` is how far a filter eases toward its knobs per sample, and
    /// `tempo` is the host's, for a synced delay.
    pub fn run(
        &mut self,
        graph: &LaneGraph,
        taps: &[Frame; MAX_STAGES],
        levels: &ControlLevels,
        tempo: Option<f64>,
        smooth: f32,
    ) -> Option<Frame> {
        // A stage that reads the bus is on the bus too, so if any stage is, the last one is.
        let root = graph.stages.len().checked_sub(1)?;
        if !graph.stages[root].bus {
            return None;
        }
        let mut outputs = *taps;
        for (index, spec) in graph.stages.iter().enumerate() {
            if !spec.bus {
                continue;
            }
            let inputs = spec
                .inputs
                .map(|source| source.map_or(Frame::SILENT, |i| outputs[i as usize]));
            outputs[index] = self.process(
                &spec.params,
                usize::from(spec.pool),
                inputs,
                levels,
                tempo,
                smooth,
            );
        }
        Some(outputs[root])
    }

    /// One sample through one bus stage. An instance past the pool passes the input through.
    fn process(
        &mut self,
        params: &StageParams,
        pool: usize,
        inputs: [Frame; MAX_INPUTS],
        levels: &ControlLevels,
        tempo: Option<f64>,
        smooth: f32,
    ) -> Frame {
        let input = inputs[0];
        match params {
            StageParams::Distortion(p) => match self.distortion.get_mut(pool) {
                Some(fx) => fx.process(
                    input,
                    &DistortionParams {
                        kind: p.kind,
                        drive: p.drive.plain(levels),
                        tone: p.tone.plain(levels),
                        level_db: p.level.plain(levels),
                        mix: p.mix.plain(levels),
                    },
                ),
                None => input,
            },
            StageParams::Compressor(p) => match self.compressor.get_mut(pool) {
                Some(fx) => fx.process(
                    input,
                    &CompressorParams {
                        threshold_db: p.threshold.plain(levels),
                        ratio: p.ratio.plain(levels),
                        attack_ms: p.attack.plain(levels),
                        release_ms: p.release.plain(levels),
                        makeup_db: p.makeup.plain(levels),
                        mix: p.mix.plain(levels),
                        multiband: p.multiband,
                        depth: p.depth.plain(levels),
                    },
                ),
                None => input,
            },
            StageParams::Reverb(p) => match self.reverb.get_mut(pool) {
                Some(fx) => fx.process(
                    input,
                    &ReverbParams {
                        decay_s: p.decay.plain(levels),
                        size: p.size.plain(levels),
                        damp: p.damp.plain(levels),
                        mix: p.mix.plain(levels),
                    },
                ),
                None => input,
            },
            StageParams::Delay(p) => match self.delay.get_mut(pool) {
                Some(fx) => fx.process(
                    input,
                    &DelayParams {
                        time_ms: delay_time_ms(
                            p.time.plain(levels),
                            p.sync.plain(levels).round() as usize,
                            tempo,
                        ),
                        feedback: p.feedback.plain(levels),
                        damp: p.damp.plain(levels),
                        mix: p.mix.plain(levels),
                        ping_pong: p.ping_pong,
                    },
                ),
                None => input,
            },
            StageParams::Chorus(p) => match self.chorus.get_mut(pool) {
                Some(fx) => fx.process(
                    input,
                    &ChorusParams {
                        rate_hz: p.rate.plain(levels),
                        depth: p.depth.plain(levels),
                        spread: p.spread.plain(levels),
                        mix: p.mix.plain(levels),
                    },
                ),
                None => input,
            },
            StageParams::Phaser(p) => match self.phaser.get_mut(pool) {
                Some(fx) => fx.process(
                    input,
                    &PhaserParams {
                        rate_hz: p.rate.plain(levels),
                        depth: p.depth.plain(levels),
                        feedback: p.feedback.plain(levels),
                        center_hz: p.center.plain(levels),
                        stages: p.stages,
                        spread: p.spread.plain(levels),
                        mix: p.mix.plain(levels),
                    },
                ),
                None => input,
            },
            StageParams::Flanger(p) => match self.flanger.get_mut(pool) {
                Some(fx) => fx.process(
                    input,
                    &FlangerParams {
                        rate_hz: p.rate.plain(levels),
                        depth: p.depth.plain(levels),
                        feedback: p.feedback.plain(levels),
                        delay_ms: p.delay.plain(levels),
                        spread: p.spread.plain(levels),
                        mix: p.mix.plain(levels),
                    },
                ),
                None => input,
            },
            StageParams::Eq(p) => match self.eq.get_mut(pool) {
                Some(fx) => fx.process(
                    input,
                    &EqParams {
                        bands: std::array::from_fn::<_, BANDS, _>(|band| EqBand {
                            kind: p.bands[band].kind,
                            freq_hz: p.bands[band].freq.plain(levels),
                            gain_db: p.bands[band].gain.plain(levels),
                            q: p.bands[band].q.plain(levels),
                        }),
                        analyzer: p.analyzer,
                    },
                ),
                None => input,
            },
            // A filter on the bus is one shared filter over the summed voices, so a knob that an
            // envelope drives follows the newest note, not each note's own.
            StageParams::Filter(p) => match self.filter.get_mut(pool) {
                Some(state) => state
                    .get_or_insert_with(|| FilterState::new(p, levels))
                    .tick(p, input, levels, smooth, self.sample_rate),
                None => input,
            },
            StageParams::Mix(p) => {
                let amount = p.amount.plain(levels);
                // FM reads signal A at a delay that follows B, scaled by the note's pitch, and the
                // sum of voices has no one pitch. So on the bus an FM mix passes A through.
                if p.mode == MixMode::Fm {
                    inputs[0]
                } else {
                    inputs[0].zip(inputs[1], |a, b| p.mode.combine(a, b, amount))
                }
            }
            // A source never runs on the bus.
            StageParams::Oscillator(_) => input,
        }
    }
}

/// The delay time to use, in ms. A Sync step past zero is a note division at the host's tempo. It
/// falls back to the Time knob when the host gives no tempo. The result stays inside the delay's
/// range.
fn delay_time_ms(time_ms: f32, sync: usize, tempo: Option<f64>) -> f32 {
    let ms = match tempo {
        Some(bpm) if sync >= 1 && bpm > 0.0 => 60_000.0 / bpm as f32 / cycles_per_beat(sync),
        _ => time_ms,
    };
    ms.clamp(1.0, MAX_DELAY_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With no sync, the Time knob sets the delay.
    #[test]
    fn free_time_uses_the_knob() {
        assert_eq!(delay_time_ms(350.0, 0, Some(120.0)), 350.0);
        assert_eq!(delay_time_ms(350.0, 0, None), 350.0);
    }

    /// A quarter note at 120 bpm is 500 ms, and an eighth is 250 ms.
    #[test]
    fn a_synced_time_follows_the_tempo() {
        assert!((delay_time_ms(350.0, 3, Some(120.0)) - 500.0).abs() < 1e-3);
        assert!((delay_time_ms(350.0, 4, Some(120.0)) - 250.0).abs() < 1e-3);
    }

    /// Without a tempo, a synced delay falls back to the knob. A long division stops at the
    /// longest delay.
    #[test]
    fn sync_falls_back_without_a_tempo_and_clamps() {
        assert_eq!(delay_time_ms(350.0, 3, None), 350.0);
        assert_eq!(delay_time_ms(350.0, 1, Some(60.0)), MAX_DELAY_MS);
    }
}
