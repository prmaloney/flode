//! The oscillator stage: a waveform or noise, with unison, detune, warp, transpose, and the
//! amplitude envelope. It is a source with no audio inputs.

use super::{AudioStage, StartContext, Tick};
use crate::dsp::frame::{pan_gains, Frame};
use crate::dsp::gain::db_to_linear;
use crate::dsp::noise::NoiseState;
use crate::dsp::pitch::{ratio_of_semitones, semitones_of};
use crate::dsp::rng::Rng;
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
    /// One phase per unison sub-oscillator.
    phases: [f32; MAX_UNISON],
    /// Each sub-oscillator's detune, as a frequency ratio from the voice's pitch.
    detune_ratios: [f32; MAX_UNISON],
    /// The unison count the knob asks for. Copies past it fade out, and a copy that joins fades in.
    count: usize,
    /// Unison copies add up, so the sum is scaled down. Noise is one stream and needs no scale.
    norm: f32,
    /// Where each sub-oscillator sits across the unison spread now, from -1.0 to 1.0. The lowest
    /// detune sits at the left end. Noise has one stream and sits at the centre. A change in the
    /// count moves the copies toward their new places over a few milliseconds.
    positions: [f32; MAX_UNISON],
    /// The places `positions` is easing toward.
    goals: [f32; MAX_UNISON],
    /// How much of each copy sounds, from 0.0 to 1.0. A copy that joins or leaves eases, so the
    /// change does not click.
    fades: [f32; MAX_UNISON],
    /// How many copies run: the count, and any copy past it that is still fading out.
    running: usize,
    /// The left and right gains of each sub-oscillator, for the pan and spread below.
    gains: [Frame; MAX_UNISON],
    /// The pan, spread, detune and noise setting that `gains` and `detune_ratios` were made for.
    /// Placing a copy takes a sine and a cosine and a detune takes a power, so they are made
    /// again only when a knob moves or the copies are easing.
    placed_at: (f32, f32),
    placed_cents: f32,
    placed_noise: bool,
    /// True when nothing is easing, so a sample needs no upkeep.
    settled: bool,
    /// The octave, coarse and fine transpose, as a sum of semitones, eased toward the knob values.
    /// Easing in semitones keeps the step even in frequency, since a transpose is a log-frequency
    /// quantity. A step on Coarse or Octave still lands on a whole semitone, but reaching it now
    /// takes a few milliseconds instead of jumping, so it does not click or zipper.
    transpose_semitones: f32,
}

/// Where copy `i` of `count` sits across the spread: -1.0 for the first copy, 1.0 for the last,
/// and the middle for a single copy.
fn spread_of(i: usize, count: usize) -> f32 {
    if count > 1 {
        2.0 * i as f32 / (count - 1) as f32 - 1.0
    } else {
        0.0
    }
}

/// How close a fade or a position gets to its goal before it lands on it.
const EASE_DONE: f32 = 1e-3;

impl OscillatorState {
    /// Moves the unison count to `count`. A copy that joins starts at its own place and phase and
    /// fades in. Copies that stay ease toward their new places, and copies that leave fade out
    /// where they are.
    fn recount(&mut self, count: usize, randomness: f32, rng: &mut Rng) {
        for i in 0..MAX_UNISON {
            if i < count {
                let goal = spread_of(i, count);
                if i >= self.count && self.fades[i] <= 0.0 {
                    let even = i as f32 / count as f32;
                    self.phases[i] = even + randomness * (rng.unit() - even);
                    self.positions[i] = goal;
                }
                self.goals[i] = goal;
            } else {
                self.goals[i] = self.positions[i];
            }
        }
        self.count = count;
        self.settled = false;
    }

    /// Eases the fades and places toward their goals, then makes the gains, detune ratios and
    /// scale for the knobs. `smooth` is the share of the gap that each sample closes.
    fn update(&mut self, pan: f32, spread: f32, cents: f32, noise: bool, smooth: f32) {
        let mut moving = false;
        for i in 0..MAX_UNISON {
            let fade_goal = if i < self.count { 1.0 } else { 0.0 };
            if self.fades[i] != fade_goal {
                self.fades[i] += (fade_goal - self.fades[i]) * smooth;
                if (fade_goal - self.fades[i]).abs() < EASE_DONE {
                    self.fades[i] = fade_goal;
                } else {
                    moving = true;
                }
            }
            if self.positions[i] != self.goals[i] {
                self.positions[i] += (self.goals[i] - self.positions[i]) * smooth;
                if (self.goals[i] - self.positions[i]).abs() < EASE_DONE {
                    self.positions[i] = self.goals[i];
                } else {
                    moving = true;
                }
            }
        }
        self.running = (0..MAX_UNISON)
            .rev()
            .find(|&i| self.fades[i] > 0.0)
            .map_or(0, |i| i + 1)
            .max(self.count);
        let mut power = 0.0;
        for i in 0..self.running {
            let position = if noise { 0.0 } else { self.positions[i] };
            self.gains[i] = pan_gains(pan + spread * position);
            self.detune_ratios[i] = 2.0f32.powf(position * cents / 1200.0);
            power += self.fades[i] * self.fades[i];
        }
        self.norm = if noise || power <= 0.0 {
            1.0
        } else {
            1.0 / power.sqrt()
        };
        self.placed_at = (pan, spread);
        self.placed_cents = cents;
        self.placed_noise = noise;
        self.settled = !moving;
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
    /// apart keeps them from cancelling. The phase spread is read once here, since it only sets
    /// where the copies begin. The unison count and the detune are read again every sample, so a
    /// held note follows them.
    fn start(p: &OscillatorParams, ctx: &mut StartContext) -> OscillatorState {
        let count = (p.unison.plain(ctx.levels).round() as usize).clamp(1, MAX_UNISON);
        let randomness = p.phase_random.plain(ctx.levels);

        let noise = p.waveform == Waveform::Noise;
        let mut phases = [0.0; MAX_UNISON];
        let mut goals = [0.0; MAX_UNISON];
        let mut fades = [0.0; MAX_UNISON];
        for i in 0..count {
            let even = i as f32 / count as f32;
            let random = ctx.rng.unit();
            phases[i] = even + randomness * (random - even);
            goals[i] = spread_of(i, count);
            fades[i] = 1.0;
        }
        let mut state = OscillatorState {
            noise: NoiseState::default(),
            phases,
            detune_ratios: [1.0; MAX_UNISON],
            count,
            norm: 1.0,
            positions: goals,
            goals,
            fades,
            running: count,
            gains: [Frame::SILENT; MAX_UNISON],
            placed_at: (0.0, 0.0),
            placed_cents: 0.0,
            placed_noise: noise,
            settled: true,
            transpose_semitones: semitones_of(
                p.octave.plain(ctx.levels),
                p.coarse.plain(ctx.levels),
                p.fine.plain(ctx.levels),
            ),
        };
        state.update(
            p.pan.plain(ctx.levels),
            p.spread.plain(ctx.levels),
            p.detune.plain(ctx.levels),
            noise,
            0.0,
        );
        state
    }

    fn process(
        p: &OscillatorParams,
        state: &mut OscillatorState,
        _inputs: &[Frame; MAX_INPUTS],
        tick: &mut Tick,
    ) -> Frame {
        let levels = tick.levels;
        let noise = p.waveform == Waveform::Noise;
        let count = (p.unison.plain(levels).round() as usize).clamp(1, MAX_UNISON);
        if count != state.count {
            let randomness = p.phase_random.plain(levels);
            state.recount(count, randomness, tick.rng);
        }
        let placed_at = (p.pan.plain(levels), p.spread.plain(levels));
        let cents = p.detune.plain(levels);
        if !state.settled
            || placed_at != state.placed_at
            || cents != state.placed_cents
            || noise != state.placed_noise
        {
            state.update(placed_at.0, placed_at.1, cents, noise, tick.smooth);
        }
        let raw = if noise {
            state.gains[0] * state.noise.sample(p.noise, tick.rng)
        } else {
            let semitones_target = semitones_of(
                p.octave.plain(levels),
                p.coarse.plain(levels),
                p.fine.plain(levels),
            );
            state.transpose_semitones +=
                (semitones_target - state.transpose_semitones) * tick.smooth;
            let transpose = ratio_of_semitones(state.transpose_semitones);
            let warp_amount = p.warp_amount.plain(levels);

            let mut sum = Frame::SILENT;
            for i in 0..state.running {
                // The phase step, in cycles per sample. The waveform band-limits its edges by it.
                let step = tick.pitch_step * state.detune_ratios[i] * transpose;
                let phase = warp_phase(p.warp, state.phases[i], warp_amount);
                let sample = warp_value(
                    p.warp,
                    tick.tables.sample(p.waveform, phase, step),
                    warp_amount,
                );
                sum += state.gains[i] * (sample * state.fades[i]);
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

    /// Runs `samples` of `params` on a state that began under other settings, and returns the
    /// output.
    fn run_on(
        state: &mut OscillatorState,
        rig: &mut Rig,
        params: &OscillatorParams,
        samples: usize,
    ) -> Vec<Frame> {
        (0..samples)
            .map(|_| {
                Oscillator::process(params, state, &[Frame::SILENT; MAX_INPUTS], &mut rig.tick())
            })
            .collect()
    }

    fn stack(unison: f32, detune: f32) -> OscillatorParams {
        OscillatorParams {
            unison: Param::fixed(KnobId::Unison, unison),
            detune: Param::fixed(KnobId::Detune, detune),
            ..OscillatorParams::default()
        }
    }

    /// The largest jump between neighbouring samples on the left channel.
    fn biggest_step(out: &[Frame]) -> f32 {
        out.windows(2)
            .map(|pair| (pair[1].l - pair[0].l).abs())
            .fold(0.0f32, f32::max)
    }

    /// Turning the detune up while a note sounds spreads the copies at once.
    #[test]
    fn a_held_note_follows_the_detune() {
        let mut rig = Rig::new();
        let mut state = Oscillator::start(&stack(3.0, 0.0), &mut rig.start_context());
        assert!(state.detune_ratios().iter().all(|&ratio| ratio == 1.0));
        run_on(&mut state, &mut rig, &stack(3.0, 50.0), 10);
        let ratios = state.detune_ratios();
        assert!(ratios[0] < 1.0 && ratios[1] == 1.0 && ratios[2] > 1.0);
        let expected = 2.0f32.powf(50.0 / 1200.0);
        assert!((ratios[2] - expected).abs() < 1e-6);
    }

    /// More copies join a held note, and the sum settles at the scale for the new count.
    #[test]
    fn a_held_note_follows_a_higher_unison_count() {
        let mut rig = Rig::new();
        let mut state = Oscillator::start(&stack(1.0, 30.0), &mut rig.start_context());
        run_on(&mut state, &mut rig, &stack(4.0, 30.0), 3000);
        assert_eq!(state.detune_ratios().len(), 4);
        assert!(state.detune_ratios()[0] < 1.0 && state.detune_ratios()[3] > 1.0);
        assert!((state.norm - 0.5).abs() < 1e-5, "{}", state.norm);
        assert!(state.settled);
    }

    /// Copies leave a held note when the count falls, and the rest are scaled back up.
    #[test]
    fn a_held_note_follows_a_lower_unison_count() {
        let mut rig = Rig::new();
        let mut state = Oscillator::start(&stack(5.0, 30.0), &mut rig.start_context());
        run_on(&mut state, &mut rig, &stack(2.0, 30.0), 3000);
        assert_eq!(state.detune_ratios().len(), 2);
        assert_eq!(state.running, 2);
        assert!((state.norm - 1.0 / 2.0f32.sqrt()).abs() < 1e-5);
        assert!(state.settled);
    }

    /// A copy that joins or leaves fades over a few milliseconds, so the count changing does not
    /// click. The biggest step after the change stays near the steps of a steady stack.
    #[test]
    fn a_unison_change_does_not_click() {
        let steady = {
            let mut rig = Rig::new();
            let mut state = Oscillator::start(&stack(5.0, 0.0), &mut rig.start_context());
            biggest_step(&run_on(&mut state, &mut rig, &stack(5.0, 0.0), 3000))
        };
        for (from, to) in [(1.0, 5.0), (5.0, 1.0), (2.0, 7.0), (7.0, 3.0)] {
            let mut rig = Rig::new();
            let mut state = Oscillator::start(&stack(from, 0.0), &mut rig.start_context());
            run_on(&mut state, &mut rig, &stack(from, 0.0), 500);
            let after = run_on(&mut state, &mut rig, &stack(to, 0.0), 3000);
            let step = biggest_step(&after);
            assert!(
                step < steady * 1.5 + 0.05,
                "{from} to {to}: step {step}, steady {steady}"
            );
        }
    }

    /// A count that moves back before the fade ends picks the copy up where it is. It does not
    /// start it over.
    #[test]
    fn a_copy_that_returns_midway_keeps_its_phase() {
        let mut rig = Rig::new();
        let mut state = Oscillator::start(&stack(3.0, 0.0), &mut rig.start_context());
        run_on(&mut state, &mut rig, &stack(1.0, 0.0), 100);
        assert!(state.fades[2] > 0.0 && state.fades[2] < 1.0);
        let phase = state.phases[2];
        run_on(&mut state, &mut rig, &stack(3.0, 0.0), 1);
        // One sample of travel and no restart.
        assert!((state.phases[2] - phase).abs() < 0.05);
    }

    /// With nothing easing, a sample does no upkeep.
    #[test]
    fn a_steady_state_stays_settled() {
        let mut rig = Rig::new();
        let params = stack(5.0, 20.0);
        let mut state = Oscillator::start(&params, &mut rig.start_context());
        assert!(state.settled);
        run_on(&mut state, &mut rig, &params, 100);
        assert!(state.settled);
    }

    /// A jump on the coarse knob while a note sounds eases toward the new pitch instead of
    /// snapping, so the step does not click like an instant pitch jump would.
    #[test]
    fn a_coarse_jump_eases_instead_of_snapping() {
        let mut rig = Rig::new();
        let low = OscillatorParams::default();
        let high = OscillatorParams {
            coarse: Param::fixed(KnobId::Coarse, 12.0),
            ..OscillatorParams::default()
        };
        let mut state = Oscillator::start(&low, &mut rig.start_context());
        run_on(&mut state, &mut rig, &low, 100);
        assert_eq!(state.transpose_semitones, 0.0);
        run_on(&mut state, &mut rig, &high, 1);
        // One sample of easing moves toward the target but does not reach it outright.
        assert!(state.transpose_semitones > 0.0 && state.transpose_semitones < 12.0);
        run_on(&mut state, &mut rig, &high, 10000);
        assert!((state.transpose_semitones - 12.0).abs() < 1e-3);
    }

    /// Noise ignores the count and the detune, and a switch to a wave still places its copies.
    #[test]
    fn switching_from_noise_to_a_wave_places_the_copies() {
        let mut rig = Rig::new();
        let noise = OscillatorParams {
            waveform: Waveform::Noise,
            unison: Param::fixed(KnobId::Unison, 3.0),
            detune: Param::fixed(KnobId::Detune, 40.0),
            ..OscillatorParams::default()
        };
        let mut state = Oscillator::start(&noise, &mut rig.start_context());
        run_on(&mut state, &mut rig, &noise, 50);
        run_on(&mut state, &mut rig, &stack(3.0, 40.0), 50);
        let ratios = state.detune_ratios();
        assert!(ratios[0] < 1.0 && ratios[2] > 1.0);
    }
}
