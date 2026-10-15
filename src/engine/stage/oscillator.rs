//! The oscillator stage: a waveform or noise, with unison, detune, warp, transpose, and the
//! amplitude envelope. It is a source with no audio inputs.

use super::{AudioStage, StartContext, Tick};
use crate::dsp::frame::{pan_gains, Frame};
use crate::dsp::gain::db_to_linear;
use crate::dsp::noise::NoiseState;
use crate::dsp::pitch::transpose_ratio;
use crate::dsp::warp::{warp_phase, warp_value};
use crate::dsp::wavetable::Waveform;
use crate::nodes::knobs::MAX_UNISON;
use crate::patch::control::MAX_ENVELOPES;
use crate::patch::stage::{OscillatorParams, MAX_INPUTS};

pub struct Oscillator;

/// What one voice remembers of an oscillator.
#[derive(Clone, Copy)]
pub struct OscillatorState {
    noise: NoiseState,
    /// One phase per unison sub-oscillator. Only the first `count` are used.
    phases: [f32; MAX_UNISON],
    /// Each sub-oscillator's detune, as a frequency ratio from the voice's pitch.
    detune_ratios: [f32; MAX_UNISON],
    /// The unison voices this oscillator started with.
    count: usize,
    /// Unison copies add up, so the sum is scaled down. Noise is one stream and needs no scale.
    norm: f32,
    /// Where each sub-oscillator sits across the unison spread, from -1.0 to 1.0. The lowest
    /// detune sits at the left end. Noise has one stream and sits at the centre.
    positions: [f32; MAX_UNISON],
    /// The left and right gains of each sub-oscillator, for the pan and spread below.
    gains: [Frame; MAX_UNISON],
    /// The pan and spread `gains` was made for. Placing a copy takes a sine and a cosine, so the
    /// gains are made again only when a knob moves.
    placed_at: (f32, f32),
}

impl OscillatorState {
    /// Makes the gains of the first `count` sub-oscillators for a pan and a spread. The spread is
    /// how far the ends of the unison stack sit from the pan, as a share of the half field.
    fn place(&mut self, pan: f32, spread: f32) {
        for (gain, position) in self.gains.iter_mut().zip(&self.positions).take(self.count) {
            *gain = pan_gains(pan + spread * position);
        }
        self.placed_at = (pan, spread);
    }
}

#[cfg(test)]
impl OscillatorState {
    pub fn detune_ratios(&self) -> &[f32] {
        &self.detune_ratios[..self.count]
    }

    pub fn phases(&self) -> &[f32] {
        &self.phases[..self.count]
    }
}

impl AudioStage for Oscillator {
    type Params = OscillatorParams;
    type State = OscillatorState;

    /// Spreads the sub-oscillators from the lowest detune to the highest. Starting the phases
    /// apart keeps them from cancelling. The unison count, detune, and phase spread are read once
    /// here, so a note keeps the sound it started with.
    fn start(p: &OscillatorParams, ctx: &mut StartContext) -> OscillatorState {
        let count = (p.unison.plain(ctx.levels).round() as usize).clamp(1, MAX_UNISON);
        let detune_cents = p.detune.plain(ctx.levels);
        let randomness = p.phase_random.plain(ctx.levels);

        let noise = p.waveform == Waveform::Noise;
        let mut phases = [0.0; MAX_UNISON];
        let mut detune_ratios = [1.0; MAX_UNISON];
        let mut positions = [0.0; MAX_UNISON];
        for i in 0..count {
            let spread = if count > 1 {
                2.0 * i as f32 / (count - 1) as f32 - 1.0
            } else {
                0.0
            };
            let even = i as f32 / count as f32;
            let random = ctx.rng.unit();
            phases[i] = even + randomness * (random - even);
            detune_ratios[i] = 2.0f32.powf(spread * detune_cents / 1200.0);
            if !noise {
                positions[i] = spread;
            }
        }
        let norm = if noise {
            1.0
        } else {
            1.0 / (count as f32).sqrt()
        };
        let mut state = OscillatorState {
            noise: NoiseState::default(),
            phases,
            detune_ratios,
            count,
            norm,
            positions,
            gains: [Frame::SILENT; MAX_UNISON],
            placed_at: (0.0, 0.0),
        };
        state.place(p.pan.plain(ctx.levels), p.spread.plain(ctx.levels));
        state
    }

    fn process(
        p: &OscillatorParams,
        state: &mut OscillatorState,
        _inputs: &[Frame; MAX_INPUTS],
        tick: &mut Tick,
    ) -> Frame {
        let levels = tick.levels;
        let placed_at = (p.pan.plain(levels), p.spread.plain(levels));
        if placed_at != state.placed_at {
            state.place(placed_at.0, placed_at.1);
        }
        let raw = if p.waveform == Waveform::Noise {
            state.gains[0] * state.noise.sample(p.noise, tick.rng)
        } else {
            let transpose = transpose_ratio(
                p.octave.plain(levels),
                p.coarse.plain(levels),
                p.fine.plain(levels),
            );
            let warp_amount = p.warp_amount.plain(levels);

            let mut sum = Frame::SILENT;
            for i in 0..state.count {
                // The phase step, in cycles per sample. The waveform band-limits its edges by it.
                let step = tick.pitch_step * state.detune_ratios[i] * transpose;
                let phase = warp_phase(p.warp, state.phases[i], warp_amount);
                let sample = warp_value(
                    p.warp,
                    tick.tables.sample(p.waveform, phase, step),
                    warp_amount,
                );
                sum += state.gains[i] * sample;
                state.phases[i] += step;
                if state.phases[i] >= 1.0 {
                    state.phases[i] -= 1.0;
                }
            }
            sum
        };

        // With envelopes driving the sound, an oscillator with none of its own stays at full
        // amplitude. Otherwise the bypass copy gives it the short fade on release.
        let amplitude = match p.envelope {
            Some(index) => tick.envs[index].level,
            None if tick.envelopes_active => 1.0,
            None => tick.envs[MAX_ENVELOPES].level,
        };
        raw * (state.norm * tick.velocity * amplitude * db_to_linear(p.level.plain(levels)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::stage::testing::Rig;
    use crate::nodes::knobs::KnobId;
    use crate::patch::control::Param;

    /// Renders `samples` of an oscillator and counts the upward zero crossings.
    fn cycles(params: &OscillatorParams, rig: &mut Rig, samples: usize) -> usize {
        let mut state = Oscillator::start(params, &mut rig.start_context());
        let mut last = 0.0f32;
        let mut count = 0;
        for _ in 0..samples {
            let out = Oscillator::process(
                params,
                &mut state,
                &[Frame::SILENT; MAX_INPUTS],
                &mut rig.tick(),
            )
            .l;
            if last < 0.0 && out >= 0.0 {
                count += 1;
            }
            last = out;
        }
        count
    }

    /// A sine at 440 Hz completes about 440 cycles in a second.
    #[test]
    fn a_sine_runs_at_the_voices_pitch() {
        let mut rig = Rig::new();
        let n = cycles(&OscillatorParams::default(), &mut rig, 44100);
        assert!((438..=442).contains(&n), "{n} cycles");
    }

    /// An octave up doubles the frequency, and the transpose is read per sample.
    #[test]
    fn the_octave_param_doubles_the_frequency() {
        let mut rig = Rig::new();
        let params = OscillatorParams {
            octave: Param::fixed(KnobId::Octave, 1.0),
            ..OscillatorParams::default()
        };
        let n = cycles(&params, &mut rig, 44100);
        assert!((876..=884).contains(&n), "{n} cycles");
    }

    /// Unison copies spread by the detune, and the sum is scaled so it stays near full scale.
    #[test]
    fn unison_spreads_the_detune_across_copies() {
        let mut rig = Rig::new();
        let params = OscillatorParams {
            unison: Param::fixed(KnobId::Unison, 3.0),
            detune: Param::fixed(KnobId::Detune, 50.0),
            ..OscillatorParams::default()
        };
        let state = Oscillator::start(&params, &mut rig.start_context());
        let ratios = state.detune_ratios();
        assert_eq!(ratios.len(), 3);
        assert!(ratios[0] < 1.0 && ratios[1] == 1.0 && ratios[2] > 1.0);
        assert!((state.norm - 1.0 / 3.0f32.sqrt()).abs() < 1e-6);
    }

    /// Renders `samples` frames of an oscillator.
    fn frames(params: &OscillatorParams, samples: usize) -> Vec<Frame> {
        let mut rig = Rig::new();
        let mut state = Oscillator::start(params, &mut rig.start_context());
        (0..samples)
            .map(|_| {
                Oscillator::process(
                    params,
                    &mut state,
                    &[Frame::SILENT; MAX_INPUTS],
                    &mut rig.tick(),
                )
            })
            .collect()
    }

    /// A lone oscillator at the centre sounds the same on both sides, at the level it had before
    /// the engine was stereo.
    #[test]
    fn a_centred_oscillator_is_the_same_on_both_sides() {
        let out = frames(&OscillatorParams::default(), 500);
        assert!(out.iter().all(|f| f.l == f.r));
        let peak = out.iter().map(|f| f.peak()).fold(0.0f32, f32::max);
        assert!(peak > 0.95 && peak <= 1.01, "{peak}");
    }

    /// The pan knob moves the whole oscillator, and a hard pan leaves the other side silent.
    #[test]
    fn the_pan_knob_places_the_oscillator() {
        let at = |pan| {
            frames(
                &OscillatorParams {
                    pan: Param::fixed(KnobId::Pan, pan),
                    ..OscillatorParams::default()
                },
                500,
            )
        };
        assert!(at(-1.0).iter().all(|f| f.r.abs() < 1e-5));
        assert!(at(1.0).iter().all(|f| f.l.abs() < 1e-5));
        let left_heavy = at(-0.5).iter().fold((0.0f32, 0.0f32), |(l, r), f| {
            (l.max(f.l.abs()), r.max(f.r.abs()))
        });
        assert!(left_heavy.0 > left_heavy.1 * 1.5, "{left_heavy:?}");
    }

    /// Unison copies spread across the field, so the two sides differ. With no spread, or one
    /// copy, they do not.
    #[test]
    fn unison_spread_widens_the_image() {
        let stack = |unison, spread| {
            frames(
                &OscillatorParams {
                    unison: Param::fixed(KnobId::Unison, unison),
                    detune: Param::fixed(KnobId::Detune, 30.0),
                    spread: Param::fixed(KnobId::Spread, spread),
                    ..OscillatorParams::default()
                },
                2000,
            )
        };
        let difference =
            |out: &[Frame]| out.iter().map(|f| (f.l - f.r).abs()).fold(0.0f32, f32::max);
        assert!(difference(&stack(5.0, 1.0)) > 0.2);
        assert!(difference(&stack(5.0, 0.0)) < 1e-5);
        assert!(difference(&stack(1.0, 1.0)) < 1e-5);
    }

    /// With spread, the lowest copy sits to the left and the highest to the right, so a hard
    /// spread of two copies puts one on each side.
    #[test]
    fn a_full_spread_of_two_copies_puts_one_on_each_side() {
        let out = frames(
            &OscillatorParams {
                unison: Param::fixed(KnobId::Unison, 2.0),
                spread: Param::fixed(KnobId::Spread, 1.0),
                ..OscillatorParams::default()
            },
            500,
        );
        // At full spread, copy 0 is hard left and copy 1 is hard right, so each side holds one
        // copy at the unison scale.
        let peak =
            |side: fn(&Frame) -> f32| out.iter().map(|f| side(f).abs()).fold(0.0f32, f32::max);
        assert!((peak(|f| f.l) - 1.0).abs() < 0.05);
        assert!((peak(|f| f.r) - 1.0).abs() < 0.05);
    }

    /// Moving the pan while a note sounds takes effect on the next sample.
    #[test]
    fn the_pan_is_read_every_sample() {
        let mut rig = Rig::new();
        let centre = OscillatorParams::default();
        let left = OscillatorParams {
            pan: Param::fixed(KnobId::Pan, -1.0),
            ..OscillatorParams::default()
        };
        let mut state = Oscillator::start(&centre, &mut rig.start_context());
        let run = |params: &OscillatorParams, state: &mut OscillatorState, rig: &mut Rig| {
            (0..200)
                .map(|_| {
                    Oscillator::process(
                        params,
                        state,
                        &[Frame::SILENT; MAX_INPUTS],
                        &mut rig.tick(),
                    )
                })
                .fold(0.0f32, |r, f| r.max(f.r.abs()))
        };
        assert!(run(&centre, &mut state, &mut rig) > 0.5);
        assert!(run(&left, &mut state, &mut rig) < 1e-5);
    }

    /// With no envelope wired and none in the patch, the bypass copy sets the amplitude.
    #[test]
    fn the_bypass_envelope_shapes_an_unwired_oscillator() {
        let mut rig = Rig::new();
        let params = OscillatorParams::default();
        let mut state = Oscillator::start(&params, &mut rig.start_context());
        rig.envs[MAX_ENVELOPES].level = 0.0;
        let mut silent = 0.0f32;
        for _ in 0..200 {
            silent = silent.max(
                Oscillator::process(
                    &params,
                    &mut state,
                    &[Frame::SILENT; MAX_INPUTS],
                    &mut rig.tick(),
                )
                .l
                .abs(),
            );
        }
        assert_eq!(silent, 0.0);
    }

    /// A wired envelope sets the amplitude from its own copy.
    #[test]
    fn a_wired_envelope_sets_the_amplitude() {
        let mut rig = Rig::new();
        let params = OscillatorParams {
            envelope: Some(0),
            ..OscillatorParams::default()
        };
        let mut state = Oscillator::start(&params, &mut rig.start_context());
        rig.envs[0].level = 0.5;
        let mut peak = 0.0f32;
        for _ in 0..200 {
            peak = peak.max(
                Oscillator::process(
                    &params,
                    &mut state,
                    &[Frame::SILENT; MAX_INPUTS],
                    &mut rig.tick(),
                )
                .l
                .abs(),
            );
        }
        assert!(peak > 0.4 && peak <= 0.5, "{peak}");
    }

    /// Noise ignores the pitch and makes sound.
    #[test]
    fn noise_makes_sound_without_a_cycle() {
        let mut rig = Rig::new();
        let params = OscillatorParams {
            waveform: Waveform::Noise,
            ..OscillatorParams::default()
        };
        let mut state = Oscillator::start(&params, &mut rig.start_context());
        let peak = (0..1000)
            .map(|_| {
                Oscillator::process(
                    &params,
                    &mut state,
                    &[Frame::SILENT; MAX_INPUTS],
                    &mut rig.tick(),
                )
                .l
                .abs()
            })
            .fold(0.0f32, f32::max);
        assert!(peak > 0.5);
    }

    /// Two voices of the same oscillator keep their own phases.
    #[test]
    fn starting_twice_gives_independent_state() {
        let mut rig = Rig::new();
        let params = OscillatorParams::default();
        let mut a = Oscillator::start(&params, &mut rig.start_context());
        let b = Oscillator::start(&params, &mut rig.start_context());
        let before = b.phases()[0];
        for _ in 0..100 {
            Oscillator::process(
                &params,
                &mut a,
                &[Frame::SILENT; MAX_INPUTS],
                &mut rig.tick(),
            );
        }
        assert_eq!(b.phases()[0], before);
        assert_ne!(a.phases()[0], before);
    }
}
