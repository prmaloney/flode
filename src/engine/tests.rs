//! Tests that play notes into the engine and read what it does.

use super::stage::StageState;
use super::*;
use crate::dsp::envelope::EnvPhase;
use crate::dsp::mix::MixMode;
use crate::dsp::warp::WarpMode;
use crate::nodes::knobs::KnobId;
use crate::patch::control::{KnobValue, MapStage, Param, Route, Source, MAX_MAPS};
use crate::patch::stage::{FilterParams, MixParams, OscillatorParams, StageParams, StageSpec};
use crate::patch::EnvelopeSettings;
use std::sync::OnceLock;

/// The wavetables are slow to build, so every test shares one set.
fn tables() -> Arc<WaveTables> {
    static TABLES: OnceLock<Arc<WaveTables>> = OnceLock::new();
    TABLES.get_or_init(|| Arc::new(WaveTables::new())).clone()
}

fn osc(params: OscillatorParams) -> StageSpec {
    StageSpec::new(StageParams::Oscillator(params))
}

/// A patch that plays `stages` in output slot `slot`.
fn patch_with(slot: usize, stages: &[StageSpec]) -> Patch {
    let mut patch = Patch::silent();
    patch.lanes[slot].stages.extend(stages.iter().copied());
    patch
}

/// An engine that has begun a block on `patch`.
fn engine_on(patch: &Patch) -> Engine {
    let mut engine = Engine::new(tables());
    engine.begin_block(patch, Transport::default());
    engine
}

fn plain_oscillator_patch() -> Patch {
    patch_with(0, &[osc(OscillatorParams::default())])
}

fn mono_patch(glide_seconds: f32, always: bool) -> Patch {
    let mut patch = plain_oscillator_patch();
    patch.globals.mono = true;
    patch.globals.glide_always = always;
    patch.globals.glide = Param::fixed(KnobId::Glide, glide_seconds);
    patch
}

/// The newest voice of slot 0.
fn newest(engine: &Engine) -> &Voice {
    engine.lanes[0].voices().max_by_key(|v| v.age).unwrap()
}

/// The left channel of `samples` frames. A patch with nothing panned sounds the same on both.
fn render(engine: &mut Engine, samples: usize) -> Vec<f32> {
    render_stereo(engine, samples).iter().map(|f| f.l).collect()
}

fn render_stereo(engine: &mut Engine, samples: usize) -> Vec<Frame> {
    (0..samples).map(|n| engine.process_sample(n)).collect()
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0, |peak, s| f32::max(peak, s.abs()))
}

#[test]
fn a_silent_patch_makes_no_sound() {
    let mut engine = engine_on(&Patch::silent());
    engine.note_on(60, 1.0);
    assert_eq!(peak(&render(&mut engine, 1000)), 0.0);
}

#[test]
fn a_note_makes_sound_through_the_stages() {
    let mut engine = engine_on(&plain_oscillator_patch());
    engine.note_on(60, 1.0);
    assert!(peak(&render(&mut engine, 4410)) > 0.05);
}

/// Once the note off has run its short fade, the voice is gone and the key strip shows nothing.
#[test]
fn a_released_note_ends_and_leaves_the_strip() {
    let mut engine = engine_on(&plain_oscillator_patch());
    engine.note_on(60, 1.0);
    render(&mut engine, 2000);
    assert_eq!(engine.active_notes()[0] & (1 << 60), 1 << 60);
    engine.note_off(60);
    render(&mut engine, 2000);
    assert_eq!(engine.active_notes(), [0, 0]);
}

#[test]
fn a_note_sets_the_note_and_velocity_sources() {
    let mut engine = engine_on(&plain_oscillator_patch());
    engine.note_on(64, 0.5);
    let midi = engine.levels().midi;
    assert!((midi[MidiSource::Note.index()] - 64.0 / 127.0).abs() < 1e-6);
    assert_eq!(midi[MidiSource::Velocity.index()], 0.5);
}

/// In mono mode, a second note starts at the first note's pitch and glides to its own.
#[test]
fn mono_note_glides_from_the_previous_pitch() {
    let mut engine = engine_on(&mono_patch(0.1, false));
    engine.note_on(60, 1.0);
    let first = util::midi_note_to_freq(60) / engine.sample_rate;
    engine.note_on(72, 1.0);
    let second = util::midi_note_to_freq(72) / engine.sample_rate;

    let voice = newest(&engine);
    assert!((voice.step - first).abs() < 1e-9);
    assert!((voice.target_step - second).abs() < 1e-9);

    let coef = engine.glide_coef;
    let voice = engine.lanes[0].voices_mut().max_by_key(|v| v.age).unwrap();
    for _ in 0..(0.1 * 44100.0) as usize {
        voice.glide(coef);
    }
    assert!((voice.step - second).abs() / second < 0.01);
}

/// With Always on, a note glides from the last pitch even when no voice is still sounding.
#[test]
fn always_glides_after_the_previous_note_has_died() {
    let mut engine = engine_on(&mono_patch(0.1, true));
    engine.note_on(60, 1.0);
    let last = util::midi_note_to_freq(60) / engine.sample_rate;
    engine.lanes.iter_mut().for_each(Lane::clear);
    engine.note_on(72, 1.0);
    assert!((newest(&engine).step - last).abs() < 1e-9);
}

/// Without Always, the same note starts at its own pitch.
#[test]
fn without_always_a_dead_previous_note_is_not_glided_from() {
    let mut engine = engine_on(&mono_patch(0.1, false));
    engine.note_on(60, 1.0);
    engine.lanes.iter_mut().for_each(Lane::clear);
    engine.note_on(72, 1.0);
    let voice = newest(&engine);
    assert!((voice.step - voice.target_step).abs() < 1e-9);
}

/// The first note never glides, even with Always on, because there is no pitch to glide from.
#[test]
fn first_note_never_glides() {
    let mut engine = engine_on(&mono_patch(0.1, true));
    engine.note_on(60, 1.0);
    let voice = newest(&engine);
    assert!((voice.step - voice.target_step).abs() < 1e-9);
}

/// Letting go of the newest mono note plays the note still held, from its own press.
#[test]
fn mono_recalls_the_held_note() {
    let mut engine = engine_on(&mono_patch(0.0, false));
    engine.note_on(60, 1.0);
    engine.note_on(64, 1.0);
    engine.note_off(64);
    assert_eq!(newest(&engine).note, 60);
}

/// Two active slots start their voices with their own settings, so the slots do not share a
/// detune or a unison count.
#[test]
fn each_slot_uses_its_own_oscillator_settings() {
    let mut patch = Patch::silent();
    let spread = OscillatorParams {
        unison: Param::fixed(KnobId::Unison, 3.0),
        detune: Param::fixed(KnobId::Detune, 50.0),
        ..OscillatorParams::default()
    };
    patch.lanes[0].stages.push(osc(spread));
    patch.lanes[1].stages.push(osc(OscillatorParams::default()));
    let mut engine = engine_on(&patch);
    engine.note_on(60, 1.0);

    let ratios = |lane: usize| -> Vec<f32> {
        match engine.lanes[lane].voices().next().unwrap().stages[0] {
            StageState::Oscillator(state) => state.detune_ratios().to_vec(),
            _ => panic!("not an oscillator"),
        }
    };
    let first = ratios(0);
    assert_eq!(first.len(), 3);
    assert!(first[0] < 1.0 && first[2] > 1.0);
    assert_eq!(ratios(1), vec![1.0]);
    // A slot that is not wired starts no voice.
    assert!(engine.lanes[2].voices().next().is_none());
}

fn envelope(release: f32, sustain: f32) -> EnvelopeSettings {
    EnvelopeSettings {
        attack: Param::fixed(KnobId::Attack, 0.001),
        decay: Param::fixed(KnobId::Decay, 0.001),
        sustain: Param::fixed(KnobId::Sustain, sustain),
        release: Param::fixed(KnobId::Release, release),
    }
}

fn advance_voices(engine: &mut Engine, samples: usize) {
    for _ in 0..samples {
        for voice in engine.lanes[0].voices_mut() {
            voice.advance(&engine.env_configs);
        }
    }
}

/// A note with an envelope releases over the envelope's release time after its note off.
#[test]
fn note_off_releases_over_the_envelope_release_time() {
    let mut patch = patch_with(
        0,
        &[osc(OscillatorParams {
            envelope: Some(0),
            ..OscillatorParams::default()
        })],
    );
    patch.envelopes[0] = Some(envelope(0.5, 1.0));
    let mut engine = engine_on(&patch);
    engine.note_on(60, 1.0);
    advance_voices(&mut engine, 2000);
    engine.note_off(60);
    assert_eq!(newest(&engine).envs[0].phase, EnvPhase::Release);

    advance_voices(&mut engine, (0.25 * 44100.0) as usize);
    let level = newest(&engine).envs[0].level;
    assert!(level > 0.1 && level < 0.6, "level {level}");

    // The voice stays alive through the whole release, then the engine drops it.
    advance_voices(&mut engine, (0.3 * 44100.0) as usize);
    assert!(engine.lanes[0].voices().all(Voice::finished));
}

/// With an envelope driving a knob and none on the oscillator, a note off keeps the voice alive
/// for the envelope's release, rather than cutting it in the short bypass fade.
#[test]
fn a_driver_envelope_keeps_the_voice_through_its_release() {
    let mut patch = plain_oscillator_patch();
    patch.envelopes[0] = Some(envelope(0.5, 0.5));
    let mut engine = engine_on(&patch);
    engine.note_on(60, 1.0);
    advance_voices(&mut engine, 4410);
    engine.note_off(60);
    advance_voices(&mut engine, (0.1 * 44100.0) as usize);
    let voice = newest(&engine);
    assert!(!voice.finished());
    assert!(voice.envs[MAX_ENVELOPES].level > 0.5);
}

/// A filter's cutoff driven by an envelope follows each voice's own envelope. The old engine
/// ran one filter over the summed voices, so it could only follow one of them.
#[test]
fn a_driven_cutoff_follows_each_voices_own_envelope() {
    let driven = KnobValue {
        value: 0.0,
        route: Some(Route {
            source: Source::Envelope(0),
            stages: [MapStage::IDENTITY; MAX_MAPS],
            count: 0,
        }),
    };
    let filter = StageSpec::new(StageParams::Filter(FilterParams {
        cutoff: Param::new(KnobId::Cutoff, driven),
        ..FilterParams::default()
    }))
    .with_input(0, 0);
    let mut patch = patch_with(0, &[osc(OscillatorParams::default()), filter]);
    // A slow attack, so a note a moment old sits far below one a second old.
    patch.envelopes[0] = Some(EnvelopeSettings {
        attack: Param::fixed(KnobId::Attack, 4.0),
        ..envelope(0.5, 1.0)
    });
    let mut engine = engine_on(&patch);

    engine.note_on(60, 1.0);
    render(&mut engine, 44100);
    engine.note_on(64, 1.0);
    render(&mut engine, 4410);

    let cutoffs: Vec<f32> = engine.lanes[0]
        .voices()
        .map(|voice| match voice.stages[1] {
            StageState::Filter(state) => state.cutoff_hz(),
            _ => panic!("not a filter"),
        })
        .collect();
    assert_eq!(cutoffs.len(), 2);
    assert!(cutoffs[0] > cutoffs[1] * 2.0, "{cutoffs:?}");
}

/// The power of `signal` at `freq`, from the Goertzel recurrence.
fn power_at(signal: &[f32], freq: f32) -> f32 {
    let coef = 2.0 * (std::f32::consts::TAU * freq / 44100.0).cos();
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    for &x in signal {
        let s0 = x + coef * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    s1 * s1 + s2 * s2 - coef * s1 * s2
}

/// FM done in the mix node and FM done by the oscillator's own warp are the same modulation, so
/// the same two sines give the same spectrum. The mix reads signal A from a delay line, and the
/// oscillator reads its table at a moved phase.
#[test]
fn mix_fm_matches_the_oscillators_fm_warp() {
    const DEPTH: f32 = 0.5;
    let modulator = || osc(OscillatorParams::default());

    // The oscillator way: the carrier warps its own phase by the modulator.
    let warped = OscillatorParams {
        warp: WarpMode::Fm,
        warp_amount: Param::fixed(KnobId::WarpAmount, DEPTH),
        ..OscillatorParams::default()
    };
    let by_warp = patch_with(
        0,
        &[
            modulator(),
            StageSpec::new(StageParams::Oscillator(warped)).with_input(0, 0),
        ],
    );

    // The mix way: a plain carrier and a plain modulator go into an FM mix.
    let fm_mix = StageSpec::new(StageParams::Mix(MixParams {
        mode: MixMode::Fm,
        amount: Param::fixed(KnobId::Mix, DEPTH),
    }))
    .with_input(0, 0)
    .with_input(1, 1);
    let by_mix = patch_with(0, &[osc(OscillatorParams::default()), modulator(), fm_mix]);

    // Both play A4, so the sidebands land on the harmonics of 440 Hz.
    let harmonics = |patch: &Patch| -> Vec<f32> {
        let mut engine = engine_on(patch);
        engine.note_on(69, 1.0);
        let out = render(&mut engine, 14000);
        let tail = &out[3000..];
        (1..=4)
            .map(|k| power_at(tail, 440.0 * k as f32))
            .collect::<Vec<_>>()
    };
    let (warp, mix) = (harmonics(&by_warp), harmonics(&by_mix));
    for k in 1..4 {
        // Each harmonic's share of the carrier's power agrees to within a quarter.
        let warp_share = warp[k] / warp[0];
        let mix_share = mix[k] / mix[0];
        assert!(
            (mix_share / warp_share - 1.0).abs() < 0.25,
            "harmonic {}: warp {warp_share}, mix {mix_share}",
            k + 1
        );
    }
}

/// An FM pair plays through the engine: the modulator runs first in each voice, the carrier
/// reads it, and the result differs from the same carrier with no modulator.
#[test]
fn an_fm_pair_sounds_different_from_the_plain_carrier() {
    let carrier = |warp| OscillatorParams {
        warp,
        warp_amount: Param::fixed(KnobId::WarpAmount, 0.6),
        ..OscillatorParams::default()
    };
    let modulator = osc(OscillatorParams {
        octave: Param::fixed(KnobId::Octave, 1.0),
        ..OscillatorParams::default()
    });
    let fm = StageSpec::new(StageParams::Oscillator(carrier(WarpMode::Fm))).with_input(0, 0);

    let mut with_fm = engine_on(&patch_with(0, &[modulator, fm]));
    let mut plain = engine_on(&patch_with(0, &[osc(carrier(WarpMode::None))]));
    with_fm.note_on(60, 1.0);
    plain.note_on(60, 1.0);

    let fm_out = render(&mut with_fm, 4410);
    let plain_out = render(&mut plain, 4410);
    assert!(peak(&fm_out) > 0.05);
    let difference: f32 = fm_out
        .iter()
        .zip(&plain_out)
        .map(|(a, b)| (a - b).abs())
        .sum();
    assert!(difference > 1.0, "{difference}");
}

/// A wide unison stack through a filter, then a mix, keeps its stereo image to the output.
#[test]
fn a_unison_stack_stays_wide_through_a_filter_and_a_mix() {
    let stack = OscillatorParams {
        unison: Param::fixed(KnobId::Unison, 5.0),
        detune: Param::fixed(KnobId::Detune, 30.0),
        spread: Param::fixed(KnobId::Spread, 1.0),
        ..OscillatorParams::default()
    };
    let through = patch_with(
        0,
        &[
            osc(stack),
            StageSpec::new(StageParams::Filter(FilterParams::default())).with_input(0, 0),
            StageSpec::new(StageParams::Mix(MixParams {
                mode: MixMode::Crossfade,
                amount: Param::fixed(KnobId::Mix, 0.0),
            }))
            .with_input(0, 1)
            .with_input(1, 0),
        ],
    );
    let mut engine = engine_on(&through);
    engine.note_on(57, 1.0);
    let out = render_stereo(&mut engine, 6000);
    let difference = out[2000..]
        .iter()
        .map(|f| (f.l - f.r).abs())
        .fold(0.0f32, f32::max);
    assert!(difference > 0.02, "{difference}");
}

/// The default patch has nothing panned or spread, so it stays identical on both channels.
#[test]
fn a_plain_oscillator_stays_centred_through_the_engine() {
    let mut engine = engine_on(&plain_oscillator_patch());
    engine.note_on(60, 1.0);
    let out = render_stereo(&mut engine, 4000);
    assert!(out.iter().any(|f| f.l.abs() > 0.05));
    assert!(out.iter().all(|f| f.l == f.r));
}

/// Two slots panned to opposite sides come out on opposite channels.
#[test]
fn slots_panned_apart_come_out_on_separate_channels() {
    let panned = |pan| {
        osc(OscillatorParams {
            pan: Param::fixed(KnobId::Pan, pan),
            ..OscillatorParams::default()
        })
    };
    let mut patch = patch_with(0, &[panned(-1.0)]);
    patch.lanes[1].stages.extend([panned(1.0)]);
    let mut engine = engine_on(&patch);
    engine.note_on(60, 1.0);
    let out = render_stereo(&mut engine, 4000);
    assert!(out.iter().map(|f| f.l.abs()).fold(0.0f32, f32::max) > 0.05);
    assert!(out.iter().map(|f| f.r.abs()).fold(0.0f32, f32::max) > 0.05);
}
