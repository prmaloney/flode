//! The mix stage: two audio inputs combined into one. Crossfade, AM, and ring mod work sample by
//! sample. FM needs memory. Phase modulation of a signal is the same as reading it at a delay that
//! moves, so the stage keeps a delay line of signal A and lets signal B move the read point.
//!
//! The delay is scaled by the played note's period, so the depth in cycles does not depend on the
//! pitch, the way the index of an FM voice does not. A delay can't be negative, so the read point
//! rests behind the newest sample by a centre that leaves room for the swing.
//!
//! The centre is fixed when the note starts, sized for the full depth. A centre that moved with
//! the depth knob would bend the pitch while it moved, since a delay that grows or shrinks
//! stretches or squeezes the signal the way a moving source does. With the centre fixed, the knob
//! only scales the swing around it. The price is a delay of a few cycles of the note on this
//! stage's output, a few milliseconds at middle C and more on low notes.
//!
//! Signal B is delayed by the same centre. Without that, A would lag B by the centre, and the
//! phase between them would change the sound whenever their pitches are in a whole-number ratio.
//! With it, the stage hears A and B as they were at the same moment, so the result matches an
//! oscillator that phase-modulates itself.

use super::{AudioStage, StartContext, Tick};
use crate::dsp::delay::DELAY_CAPACITY;
use crate::dsp::frame::Frame;
use crate::dsp::mix::{fm_depth, MixMode};
use crate::patch::stage::{MixParams, MAX_INPUTS};

pub struct Mix;

/// How much louder than full scale the modulator may get before the delay would have to go
/// negative. A unison sum can pass 1.
const HEADROOM: f32 = 1.5;
/// The most the delay swings per unit of modulator, so the centre and the swing both fit the line.
const MAX_SWING: f32 = (DELAY_CAPACITY as f32 - 4.0) / (2.0 * HEADROOM);

/// What one voice remembers of a mix.
#[derive(Clone, Copy)]
pub struct MixState {
    /// The delay lines for the left and right of signals A and B, in that order, which this stage
    /// took at the start of the note if the voice had four free.
    delays: Option<[usize; 4]>,
    /// Where the read point rests, in samples behind the newest. It never moves during a note.
    center: f32,
    /// The widest the swing can get without the delay running out of room behind the centre.
    max_swing: f32,
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

    /// Takes four delay lines for FM, and fixes the centre for the full depth at the note's pitch.
    fn start(_: &MixParams, ctx: &mut StartContext) -> MixState {
        let max_swing = swing(1.0, ctx.pitch_step);
        MixState {
            delays: ctx.take_delays(),
            center: center_for(max_swing),
            max_swing,
        }
    }

    fn process(
        p: &MixParams,
        state: &mut MixState,
        inputs: &[Frame; MAX_INPUTS],
        tick: &mut Tick,
    ) -> Frame {
        let amount = p.amount.plain(tick.levels);
        if p.mode != MixMode::Fm {
            return inputs[0].zip(inputs[1], |a, b| p.mode.combine(a, b, amount));
        }
        // A voice with no free delay lines can't run FM, so it passes A through.
        let Some([a_left, a_right, b_left, b_right]) = state.delays else {
            return inputs[0];
        };

        // A pitch that bends below the note's own would swing wider than the centre has room for,
        // so the swing stops at what fits.
        let swing = swing(amount, tick.pitch_step).min(state.max_swing);

        tick.delays[a_left].push(inputs[0].l);
        tick.delays[a_right].push(inputs[0].r);
        tick.delays[b_left].push(inputs[1].l);
        tick.delays[b_right].push(inputs[1].r);
        // Each side of the carrier follows the same side of the modulator, as it was when the
        // carrier sample being read was current.
        let center = state.center;
        let side = |carrier: usize, modulator: usize| {
            let wave = tick.delays[modulator]
                .read(center)
                .clamp(-HEADROOM, HEADROOM);
            tick.delays[carrier].read(center + swing * wave)
        };
        Frame::new(side(a_left, b_left), side(a_right, b_right))
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
        Mix::process(
            &params,
            &mut state,
            &[Frame::mono(a), Frame::mono(b)],
            &mut rig.tick(),
        )
        .l
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
                Mix::process(
                    &params,
                    &mut state,
                    &[Frame::mono(a), Frame::mono(b)],
                    &mut rig.tick(),
                )
                .l
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

    /// With no depth, FM is signal A delayed by the centre, whatever the modulator does.
    #[test]
    fn fm_with_no_depth_passes_a_through_delayed_by_the_centre() {
        let mut rig = Rig::new();
        rig.pitch_step = 440.0 / RATE;
        let params = params(MixMode::Fm, 0.0);
        let mut state = Mix::start(&params, &mut rig.start_context());
        let centre = state.center;
        let out: Vec<f32> = (0..2000)
            .map(|n| {
                let a = (TAU * 440.0 * n as f32 / RATE).sin();
                Mix::process(
                    &params,
                    &mut state,
                    &[Frame::mono(a), Frame::mono(0.8)],
                    &mut rig.tick(),
                )
                .l
            })
            .collect();
        for n in 1000..1100 {
            let expected = (TAU * 440.0 * (n as f32 - centre) / RATE).sin();
            assert!((out[n] - expected).abs() < 0.01, "sample {n}");
        }
    }

    /// The centre is set when the note starts and never moves, however the knob turns.
    #[test]
    fn the_centre_stays_put_when_the_depth_changes() {
        let mut rig = Rig::new();
        let mut state = Mix::start(&params(MixMode::Fm, 0.1), &mut rig.start_context());
        let start = state.center;
        for n in 0..2000 {
            let amount = if n % 100 < 50 { 0.1 } else { 0.9 };
            Mix::process(
                &params(MixMode::Fm, amount),
                &mut state,
                &[Frame::SILENT; 2],
                &mut rig.tick(),
            );
            assert_eq!(state.center, start);
        }
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

    /// A voice with no delay lines left plays signal A unchanged. Each mix takes four lines.
    #[test]
    fn a_stage_with_no_free_delay_lines_passes_a_through() {
        let mut rig = Rig::new();
        let params = params(MixMode::Fm, 0.8);
        let mut ctx = rig.start_context();
        let fits = MAX_DELAY_LINES / 4;
        let states: Vec<MixState> = (0..=fits).map(|_| Mix::start(&params, &mut ctx)).collect();
        assert!(states[..fits].iter().all(|s| s.delays.is_some()));
        let mut last = states[fits];
        assert!(last.delays.is_none());
        let out = Mix::process(
            &params,
            &mut last,
            &[Frame::mono(0.42), Frame::mono(1.0)],
            &mut rig.tick(),
        )
        .l;
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
        let mut all: Vec<usize> = first.into_iter().chain(second).collect();
        all.sort();
        assert_eq!(all, (0..MAX_DELAY_LINES).collect::<Vec<_>>());
    }

    /// Each side of the carrier follows the same side of the modulator, so a signal that lives
    /// on one side stays there. Here the carrier is on the left only, and the modulator on the
    /// right only, so the right of the output stays silent and the left is not bent by it.
    #[test]
    fn fm_keeps_each_side_with_its_own_modulator() {
        let mut rig = Rig::new();
        rig.pitch_step = 440.0 / RATE;
        let params = params(MixMode::Fm, 0.8);
        let mut state = Mix::start(&params, &mut rig.start_context());
        let (mut left, mut right) = (Vec::new(), Vec::new());
        for n in 0..4000 {
            let carrier = (TAU * 440.0 * n as f32 / RATE).sin();
            let modulator = (TAU * 440.0 * n as f32 / RATE).sin();
            let out = Mix::process(
                &params,
                &mut state,
                &[Frame::new(carrier, 0.0), Frame::new(0.0, modulator)],
                &mut rig.tick(),
            );
            left.push(out.l);
            right.push(out.r);
        }
        assert!(right.iter().all(|&x| x == 0.0));
        // The left modulator is silent, so the left side is the carrier, one centre late.
        let delay = state.center.round() as usize;
        for n in 3000..3100 {
            let expected = (TAU * 440.0 * (n - delay) as f32 / RATE).sin();
            assert!((left[n] - expected).abs() < 0.05, "sample {n}");
        }
    }

    /// Switching to another mode in the middle of a note works, since every mix takes a line.
    #[test]
    fn other_modes_ignore_the_delay_line() {
        let mut rig = Rig::new();
        let fm = params(MixMode::Fm, 0.5);
        let mut state = Mix::start(&fm, &mut rig.start_context());
        let ring = params(MixMode::RingMod, 1.0);
        let out = Mix::process(
            &ring,
            &mut state,
            &[Frame::mono(0.5), Frame::mono(0.4)],
            &mut rig.tick(),
        )
        .l;
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
            let out = Mix::process(
                &params,
                &mut state,
                &[Frame::mono(a), Frame::mono(b)],
                &mut rig.tick(),
            )
            .l;
            assert!(out.is_finite() && out.abs() <= 1.01, "sample {n}: {out}");
        }
    }

    /// The frequency of `signal` in Hz, from its first and last upward zero crossings.
    fn frequency(signal: &[f32]) -> f32 {
        let crossings: Vec<f32> = signal
            .windows(2)
            .enumerate()
            .filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0)
            .map(|(n, w)| n as f32 + -w[0] / (w[1] - w[0]))
            .collect();
        (crossings.len() - 1) as f32 * RATE / (crossings[crossings.len() - 1] - crossings[0])
    }

    /// Turning the depth knob while a note sounds does not bend its pitch. With a silent
    /// modulator the stage is a plain delay of A, so any drift in the output's frequency comes
    /// from the delay time moving.
    #[test]
    fn turning_the_depth_does_not_bend_the_pitch() {
        let mut rig = Rig::new();
        rig.pitch_step = 440.0 / RATE;
        let mut state = Mix::start(&params(MixMode::Fm, 0.1), &mut rig.start_context());
        let out: Vec<f32> = (0..40000)
            .map(|n| {
                // The knob sweeps from 0.1 to 0.9 over the first half, then holds.
                let amount = 0.1 + 0.8 * (n as f32 / 20000.0).min(1.0);
                let a = Frame::mono((TAU * 440.0 * n as f32 / RATE).sin());
                Mix::process(
                    &params(MixMode::Fm, amount),
                    &mut state,
                    &[a, Frame::SILENT],
                    &mut rig.tick(),
                )
                .l
            })
            .collect();
        for window in out.chunks(4000).skip(1) {
            let cents = 1200.0 * (frequency(window) / 440.0).log2();
            assert!(cents.abs() < 3.0, "{cents} cents off");
        }
    }

    /// The same holds for a knob that jumps, as a block-rate knob does.
    #[test]
    fn a_jump_in_the_depth_does_not_bend_the_pitch() {
        let mut rig = Rig::new();
        rig.pitch_step = 110.0 / RATE;
        let mut state = Mix::start(&params(MixMode::Fm, 0.1), &mut rig.start_context());
        let out: Vec<f32> = (0..40000)
            .map(|n| {
                let amount = if n < 10000 { 0.1 } else { 0.9 };
                let a = Frame::mono((TAU * 110.0 * n as f32 / RATE).sin());
                Mix::process(
                    &params(MixMode::Fm, amount),
                    &mut state,
                    &[a, Frame::SILENT],
                    &mut rig.tick(),
                )
                .l
            })
            .collect();
        for window in out.chunks(5000).skip(1) {
            let cents = 1200.0 * (frequency(window) / 110.0).log2();
            assert!(cents.abs() < 3.0, "{cents} cents off");
        }
    }
}
