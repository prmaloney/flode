//! The mix stage: two audio inputs combined into one. Crossfade, AM, and ring mod work sample by
//! sample. FM needs memory. Phase modulation of a signal is the same as reading it at a delay that
//! moves, so the stage keeps a delay line of signal A and lets signal B move the read point.
//!
//! The delay is scaled by the played note's period, so the depth in cycles does not depend on the
//! pitch, the way the index of an FM voice does not. A delay can't be negative, so the read point
//! rests behind the newest sample by a centre that is as large as the swing needs. The centre
//! eases when the depth or the pitch changes, which keeps a knob move from bending the pitch.
//!
//! Signal B is delayed by the same centre. Without that, A would lag B by the centre, and the
//! phase between them would change the sound whenever their pitches are in a whole-number ratio.
//! With it, the stage hears A and B as they were at the same moment, so the result matches an
//! oscillator that phase-modulates itself.

use super::{AudioStage, StartContext, Tick};
use crate::dsp::delay::DELAY_CAPACITY;
use crate::dsp::mix::MixMode;
use crate::dsp::warp::fm_depth;
use crate::patch::stage::{MixParams, MAX_INPUTS};

pub struct Mix;

/// How much louder than full scale the modulator may get before the delay would have to go
/// negative. A unison sum can pass 1.
const HEADROOM: f32 = 1.5;
/// The most the centre delay moves per sample. Its move shifts the pitch by this much, so it stays
/// small.
const SLEW: f32 = 0.05;
/// The most the delay swings per unit of modulator, so the centre and the swing both fit the line.
const MAX_SWING: f32 = (DELAY_CAPACITY as f32 - 4.0) / (2.0 * HEADROOM);

/// What one voice remembers of a mix.
#[derive(Clone, Copy)]
pub struct MixState {
    /// The delay lines for signals A and B, which this stage took at the start of the note if the
    /// voice had two free.
    delays: Option<[usize; 2]>,
    /// Where the read point rests, in samples behind the newest.
    center: f32,
}

/// How far the delay swings per unit of modulator, in samples, at this depth and pitch.
fn swing(amount: f32, pitch_step: f32) -> f32 {
    (fm_depth(amount) / pitch_step.max(1e-5)).min(MAX_SWING)
}

/// The centre delay that leaves room for the swing, and one sample for the interpolation.
fn center_for(swing: f32) -> f32 {
    swing * HEADROOM + 1.0
}

impl AudioStage for Mix {
    type Params = MixParams;
    type State = MixState;

    /// Takes two delay lines for FM, and starts the centre where the depth needs it, so a note
    /// has no ramp in.
    fn start(p: &MixParams, ctx: &mut StartContext) -> MixState {
        let amount = p.amount.plain(ctx.levels);
        MixState {
            delays: ctx.take_delays(),
            center: center_for(swing(amount, ctx.pitch_step)),
        }
    }

    fn process(
        p: &MixParams,
        state: &mut MixState,
        inputs: &[f32; MAX_INPUTS],
        tick: &mut Tick,
    ) -> f32 {
        let amount = p.amount.plain(tick.levels);
        if p.mode != MixMode::Fm {
            return p.mode.combine(inputs[0], inputs[1], amount);
        }
        // A voice with no free delay lines can't run FM, so it passes A through.
        let Some([a, b]) = state.delays else {
            return inputs[0];
        };

        let swing = swing(amount, tick.pitch_step);
        let wanted = center_for(swing);
        state.center += (wanted - state.center).clamp(-SLEW, SLEW);

        tick.delays[a].push(inputs[0]);
        tick.delays[b].push(inputs[1]);
        // The modulator as it was when the carrier sample being read was current.
        let modulator = tick.delays[b].read(state.center);
        tick.delays[a].read(state.center + swing * modulator)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::stage::testing::Rig;
    use crate::engine::stage::MAX_DELAY_LINES;
    use crate::nodes::knobs::KnobId;
    use crate::patch::control::Param;
    use std::f32::consts::TAU;

    const RATE: f32 = 44100.0;

    fn params(mode: MixMode, amount: f32) -> MixParams {
        MixParams {
            mode,
            amount: Param::fixed(KnobId::Mix, amount),
        }
    }

    fn run(mode: MixMode, amount: f32, a: f32, b: f32) -> f32 {
        let mut rig = Rig::new();
        let params = params(mode, amount);
        let mut state = Mix::start(&params, &mut rig.start_context());
        Mix::process(&params, &mut state, &[a, b], &mut rig.tick())
    }

    #[test]
    fn the_stage_applies_its_mode_and_amount() {
        assert_eq!(run(MixMode::Crossfade, 0.0, 0.8, 0.2), 0.8);
        assert!((run(MixMode::Crossfade, 1.0, 0.8, 0.2) - 0.2).abs() < 1e-6);
        assert!((run(MixMode::RingMod, 1.0, 0.5, 0.4) - 0.2).abs() < 1e-6);
    }

    /// Renders an FM mix of two sines. The carrier plays the voice's pitch, and the modulator
    /// plays `ratio` times it.
    fn fm_render(pitch: f32, ratio: f32, amount: f32, samples: usize) -> Vec<f32> {
        let mut rig = Rig::new();
        rig.pitch_step = pitch / RATE;
        let params = params(MixMode::Fm, amount);
        let mut state = Mix::start(&params, &mut rig.start_context());
        (0..samples)
            .map(|n| {
                let a = (TAU * pitch * n as f32 / RATE).sin();
                let b = (TAU * pitch * ratio * n as f32 / RATE).sin();
                Mix::process(&params, &mut state, &[a, b], &mut rig.tick())
            })
            .collect()
    }

    /// The power of `signal` at `freq`, from the Goertzel recurrence.
    fn power_at(signal: &[f32], freq: f32) -> f32 {
        let coef = 2.0 * (TAU * freq / RATE).cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &x in signal {
            let s0 = x + coef * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        s1 * s1 + s2 * s2 - coef * s1 * s2
    }

    /// The share of the carrier's power that lands on its second harmonic.
    fn second_harmonic(pitch: f32, amount: f32) -> f32 {
        let out = fm_render(pitch, 1.0, amount, 12000);
        power_at(&out[2000..], pitch * 2.0) / power_at(&out[2000..], pitch)
    }

    /// With no depth, FM is signal A, one sample late: the read can't go closer than that.
    #[test]
    fn fm_with_no_depth_passes_a_through_one_sample_late() {
        let mut rig = Rig::new();
        let params = params(MixMode::Fm, 0.0);
        let mut state = Mix::start(&params, &mut rig.start_context());
        let input = [0.3, -0.7, 0.2, 0.9, -0.4];
        let out: Vec<f32> = input
            .iter()
            .map(|&a| Mix::process(&params, &mut state, &[a, 0.8], &mut rig.tick()))
            .collect();
        assert_eq!(out[0], 0.0);
        assert_eq!(&out[1..], &input[..4]);
    }

    /// The share of the signal's energy that is not on the carrier.
    fn off_carrier(pitch: f32, amount: f32) -> f32 {
        let out = &fm_render(pitch, 1.0, amount, 12000)[2000..];
        let total: f32 = out.iter().map(|x| x * x).sum();
        // A sine of amplitude A has energy A²N/2 and a Goertzel power of (AN/2)².
        let carrier = 2.0 * power_at(out, pitch) / out.len() as f32;
        1.0 - carrier / total
    }

    /// Depth adds sidebands to a pure sine, which it has none of without it. A little depth puts
    /// energy on the second harmonic, and a lot moves most of the energy off the carrier.
    #[test]
    fn fm_depth_adds_sidebands() {
        assert!(second_harmonic(440.0, 0.0) < 1e-4);
        assert!(second_harmonic(440.0, 0.5) > 0.05);
        assert!(off_carrier(440.0, 0.0) < 0.01);
        assert!(off_carrier(440.0, 0.5) > 0.2);
        assert!(off_carrier(440.0, 0.9) > 0.5);
    }

    /// The depth is in cycles of the played note, so an octave lower sounds the same.
    #[test]
    fn fm_depth_does_not_depend_on_the_pitch() {
        let high = second_harmonic(440.0, 0.5);
        let low = second_harmonic(220.0, 0.5);
        assert!((low / high - 1.0).abs() < 0.3, "{low} vs {high}");
    }

    /// A voice with no delay lines left plays signal A unchanged. Each mix takes two lines.
    #[test]
    fn a_stage_with_no_free_delay_lines_passes_a_through() {
        let mut rig = Rig::new();
        let params = params(MixMode::Fm, 0.8);
        let mut ctx = rig.start_context();
        let fits = MAX_DELAY_LINES / 2;
        let states: Vec<MixState> = (0..=fits).map(|_| Mix::start(&params, &mut ctx)).collect();
        assert!(states[..fits].iter().all(|s| s.delays.is_some()));
        let mut last = states[fits];
        assert!(last.delays.is_none());
        let out = Mix::process(&params, &mut last, &[0.42, 1.0], &mut rig.tick());
        assert_eq!(out, 0.42);
    }

    /// The two stages of one voice get different lines, so they never read each other's signal.
    #[test]
    fn two_stages_in_a_voice_take_different_lines() {
        let mut rig = Rig::new();
        let params = params(MixMode::Fm, 0.8);
        let mut ctx = rig.start_context();
        let first = Mix::start(&params, &mut ctx).delays.unwrap();
        let second = Mix::start(&params, &mut ctx).delays.unwrap();
        let mut all = [first[0], first[1], second[0], second[1]];
        all.sort();
        assert_eq!(all, [0, 1, 2, 3]);
    }

    /// Switching to another mode in the middle of a note works, since every mix takes a line.
    #[test]
    fn other_modes_ignore_the_delay_line() {
        let mut rig = Rig::new();
        let fm = params(MixMode::Fm, 0.5);
        let mut state = Mix::start(&fm, &mut rig.start_context());
        let ring = params(MixMode::RingMod, 1.0);
        let out = Mix::process(&ring, &mut state, &[0.5, 0.4], &mut rig.tick());
        assert!((out - 0.2).abs() < 1e-6);
    }

    /// At full depth on a very low note, with a modulator past full scale, the delay stays inside
    /// the line and the output stays finite.
    #[test]
    fn extreme_depth_stays_inside_the_line() {
        let mut rig = Rig::new();
        rig.pitch_step = 20.0 / RATE;
        let params = params(MixMode::Fm, 1.0);
        let mut state = Mix::start(&params, &mut rig.start_context());
        for n in 0..30000 {
            let a = (TAU * 20.0 * n as f32 / RATE).sin();
            let b = 2.0 * (TAU * 777.0 * n as f32 / RATE).sin();
            let out = Mix::process(&params, &mut state, &[a, b], &mut rig.tick());
            assert!(out.is_finite() && out.abs() <= 1.01, "sample {n}: {out}");
        }
    }

    /// Raising the depth while a note sounds moves the centre gradually, so the pitch does not
    /// jump. The centre never moves faster than the slew.
    #[test]
    fn the_centre_eases_when_the_depth_changes() {
        let mut rig = Rig::new();
        let low = params(MixMode::Fm, 0.1);
        let high = params(MixMode::Fm, 0.9);
        let mut state = Mix::start(&low, &mut rig.start_context());
        let mut last = state.center;
        for _ in 0..500 {
            Mix::process(&high, &mut state, &[0.0, 0.0], &mut rig.tick());
            assert!((state.center - last).abs() <= SLEW + 1e-6);
            last = state.center;
        }
        assert!(last > center_for(swing(0.1, 440.0 / RATE)));
    }
}
