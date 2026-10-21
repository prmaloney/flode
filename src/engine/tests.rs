//! Tests that play notes into the engine and read what it does.

use super::stage::StageState;
use super::*;
use crate::dsp::envelope::EnvPhase;
use crate::dsp::mix::MixMode;
use crate::dsp::monitor::{MonitorData, MonitorMode};
use crate::nodes::knobs::KnobId;
use crate::patch::control::{KnobValue, MapNode, MapValue, Param, Route, Source};
use crate::patch::stage::{
    FilterParams, MixParams, MonitorParams, OscillatorParams, StageParams, StageSpec,
};
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

/// The macro levels the plugin sets are the macro source, and a knob driven by one follows it.
#[test]
fn a_macro_level_drives_a_knob() {
    let driven = KnobValue {
        value: 0.0,
        route: Some(Route {
            source: Source::Macro(2),
        }),
    };
    let quiet_then_loud = OscillatorParams {
        level: Param::new(KnobId::OscGain, driven),
        ..OscillatorParams::default()
    };
    let patch = patch_with(0, &[osc(quiet_then_loud)]);
    let mut engine = engine_on(&patch);
    engine.note_on(60, 1.0);

    // OscGain at zero is its floor, which is silent.
    engine.set_macros([0.0; 4]);
    assert_eq!(engine.levels().macros[2], 0.0);
    let quiet = peak(&render(&mut engine, 2000));
    engine.set_macros([0.0, 0.0, 1.0, 0.0]);
    assert_eq!(engine.levels().macros[2], 1.0);
    let loud = peak(&render(&mut engine, 2000));
    assert!(loud > 0.05 && quiet < loud * 0.1, "{quiet} {loud}");
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

/// A note let go with a long release is still ringing when the next mono note starts. The new note
/// cuts that tail short, so one voice sounds, not a tail under it.
#[test]
fn a_mono_note_cuts_the_release_tail_of_the_one_before() {
    let mut patch = mono_patch(0.0, false);
    patch.envelopes[0] = Some(envelope(5.0, 1.0));
    patch.lanes[0].stages[0] = osc(OscillatorParams {
        envelope: Some(0),
        ..OscillatorParams::default()
    });
    let mut engine = engine_on(&patch);

    engine.note_on(60, 1.0);
    render(&mut engine, 4410);
    engine.note_off(60);
    render(&mut engine, 4410);
    // The first note is half a second into a five second tail.
    assert_eq!(engine.lanes[0].voices().count(), 1);

    engine.note_on(64, 1.0);
    // The fade is 10 ms, so well within 50 ms the old voice is gone.
    render(&mut engine, 2205);
    assert_eq!(engine.lanes[0].voices().count(), 1);
    assert_eq!(newest(&engine).note, 64);
}

/// In poly mode the same tail rings on under the next note.
#[test]
fn a_poly_note_leaves_the_release_tail_alone() {
    let mut patch = plain_oscillator_patch();
    patch.envelopes[0] = Some(envelope(5.0, 1.0));
    patch.lanes[0].stages[0] = osc(OscillatorParams {
        envelope: Some(0),
        ..OscillatorParams::default()
    });
    let mut engine = engine_on(&patch);

    engine.note_on(60, 1.0);
    render(&mut engine, 4410);
    engine.note_off(60);
    render(&mut engine, 4410);
    engine.note_on(64, 1.0);
    render(&mut engine, 2205);
    assert_eq!(engine.lanes[0].voices().count(), 2);
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

/// A note that is held follows the unison count and the detune when a later block brings new
/// values. It does not wait for a new note.
#[test]
fn a_held_note_follows_unison_and_detune_changes() {
    let stack = |unison: f32, detune: f32| {
        patch_with(
            0,
            &[osc(OscillatorParams {
                unison: Param::fixed(KnobId::Unison, unison),
                detune: Param::fixed(KnobId::Detune, detune),
                ..OscillatorParams::default()
            })],
        )
    };
    let mut engine = engine_on(&stack(1.0, 0.0));
    engine.note_on(60, 1.0);
    render(&mut engine, 500);

    let ratios = |engine: &Engine| -> Vec<f32> {
        match engine.lanes[0].voices().next().unwrap().stages[0] {
            StageState::Oscillator(state) => state.detune_ratios().to_vec(),
            _ => panic!("not an oscillator"),
        }
    };
    assert_eq!(ratios(&engine), vec![1.0]);

    engine.begin_block(&stack(3.0, 50.0), Transport::default());
    render(&mut engine, 4000);
    let held = ratios(&engine);
    assert_eq!(held.len(), 3);
    assert!(held[0] < 1.0 && held[1] == 1.0 && held[2] > 1.0);
    // Still one voice, the same note.
    assert_eq!(engine.lanes[0].voices().count(), 1);
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

/// A map that inverts its input: 1.0 in gives 0.0 out.
fn inverting_map(input: Source) -> MapNode {
    MapNode {
        input,
        in_lo: MapValue::fixed(0.0),
        in_hi: MapValue::fixed(1.0),
        out_lo: MapValue::fixed(1.0),
        out_hi: MapValue::fixed(0.0),
        offset: MapValue::fixed(0.0),
        center_range: None,
    }
}

/// A knob driven through a map follows the map's curve, not the source. With the macro up the
/// inverted map is down, so the oscillator is quiet then and loud with the macro down.
#[test]
fn a_knob_driven_through_a_map_follows_the_maps_curve() {
    let driven = KnobValue {
        value: 0.0,
        route: Some(Route {
            source: Source::Signal(0),
        }),
    };
    let params = OscillatorParams {
        level: Param::new(KnobId::OscGain, driven),
        ..OscillatorParams::default()
    };
    let mut patch = patch_with(0, &[osc(params)]);
    patch.signals.push(inverting_map(Source::Macro(2)));
    let mut engine = engine_on(&patch);
    engine.note_on(60, 1.0);

    engine.set_macros([0.0, 0.0, 1.0, 0.0]);
    assert_eq!(engine.levels().signals[0], 0.0);
    let quiet = peak(&render(&mut engine, 2000));
    engine.set_macros([0.0; 4]);
    assert_eq!(engine.levels().signals[0], 1.0);
    let loud = peak(&render(&mut engine, 2000));
    assert!(loud > 0.05 && quiet < loud * 0.1, "{quiet} {loud}");
}

/// A map can read another map, so two inverting maps in a row give the source back.
#[test]
fn maps_chain_in_the_engine() {
    let mut patch = Patch::silent();
    patch.signals.push(inverting_map(Source::Macro(1)));
    patch.signals.push(inverting_map(Source::Signal(0)));
    let mut engine = engine_on(&patch);
    for level in [0.0, 0.25, 1.0] {
        engine.set_macros([0.0, level, 0.0, 0.0]);
        let levels = engine.levels();
        assert!((levels.signals[0] - (1.0 - level)).abs() < 1e-6);
        assert!((levels.signals[1] - level).abs() < 1e-6, "{level}");
    }
}

/// A math node in the engine combines two sources, and a map can read it. Macro 1 plus macro 2,
/// then inverted by a map.
#[test]
fn a_math_node_runs_in_the_engine_and_a_map_reads_it() {
    use crate::nodes::math::MathOp;
    use crate::patch::control::MathNode;
    let mut patch = Patch::silent();
    patch.signals.push(MathNode {
        op: MathOp::Add,
        a: MapValue {
            value: 0.0,
            source: Some(Source::Macro(1)),
        },
        b: MapValue {
            value: 0.0,
            source: Some(Source::Macro(2)),
        },
    });
    patch.signals.push(inverting_map(Source::Signal(0)));
    let mut engine = engine_on(&patch);
    for (a, b) in [(0.0, 0.0), (0.25, 0.5), (0.75, 0.75)] {
        engine.set_macros([0.0, a, b, 0.0]);
        let levels = engine.levels();
        let sum = (a + b).min(1.0);
        assert!((levels.signals[0] - sum).abs() < 1e-6, "{a} + {b}");
        assert!((levels.signals[1] - (1.0 - sum)).abs() < 1e-6, "{a} + {b}");
    }
}

/// A math node that reads an envelope follows each voice's own envelope too, since the voice works
/// the node out from its own levels.
#[test]
fn a_math_node_of_an_envelope_follows_each_voices_own_envelope() {
    use crate::nodes::math::MathOp;
    use crate::patch::control::MathNode;
    let driven = KnobValue {
        value: 0.0,
        route: Some(Route {
            source: Source::Signal(0),
        }),
    };
    let filter = StageSpec::new(StageParams::Filter(FilterParams {
        cutoff: Param::new(KnobId::Cutoff, driven),
        ..FilterParams::default()
    }))
    .with_input(0, 0);
    let mut patch = patch_with(0, &[osc(OscillatorParams::default()), filter]);
    // The envelope times one, so the node passes the envelope through.
    patch.signals.push(MathNode {
        op: MathOp::Multiply,
        a: MapValue {
            value: 0.0,
            source: Some(Source::Envelope(0)),
        },
        b: MapValue::fixed(1.0),
    });
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

/// A map that reads an envelope follows each voice's own envelope, as a knob wired to the envelope
/// directly does.
#[test]
fn a_map_of_an_envelope_follows_each_voices_own_envelope() {
    let driven = KnobValue {
        value: 0.0,
        route: Some(Route {
            source: Source::Signal(0),
        }),
    };
    let filter = StageSpec::new(StageParams::Filter(FilterParams {
        cutoff: Param::new(KnobId::Cutoff, driven),
        ..FilterParams::default()
    }))
    .with_input(0, 0);
    let mut patch = patch_with(0, &[osc(OscillatorParams::default()), filter]);
    // A map that passes the envelope through.
    patch.signals.push(MapNode {
        input: Source::Envelope(0),
        in_lo: MapValue::fixed(0.0),
        in_hi: MapValue::fixed(1.0),
        out_lo: MapValue::fixed(0.0),
        out_hi: MapValue::fixed(1.0),
        offset: MapValue::fixed(0.0),
        center_range: None,
    });
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

/// An LFO with Retrigger on starts its cycle again at every note, through the engine.
#[test]
fn a_note_retriggers_the_lfo_through_the_engine() {
    let mut patch = plain_oscillator_patch();
    let mut lfo = crate::patch::LfoSettings::new();
    lfo.waveform = crate::dsp::wavetable::Waveform::Saw;
    lfo.rate = Param::fixed(KnobId::LfoRate, 3.0);
    lfo.retrigger = true;
    patch.lfos[0] = Some(lfo);
    let mut engine = engine_on(&patch);
    engine.note_on(60, 1.0);
    render(&mut engine, 20000);
    let before = engine.levels().lfo[0];
    engine.note_on(64, 1.0);
    render(&mut engine, 1);
    let after = engine.levels().lfo[0];
    assert!(before > 0.1, "{before}");
    assert!(after < 0.01, "{after}");
}

/// The engine reports the stage of the newest voice's envelope, and nothing while no voice runs.
#[test]
fn the_engine_reports_the_envelope_stage() {
    let mut patch = plain_oscillator_patch();
    patch.envelopes[0] = Some(envelope(0.5, 0.5));
    let mut engine = engine_on(&patch);
    render(&mut engine, 16);
    assert_eq!(engine.levels().envelope_phase[0], None);

    engine.note_on(60, 1.0);
    render(&mut engine, 4410);
    assert_eq!(engine.levels().envelope_phase[0], Some(EnvPhase::Sustain));

    engine.note_off(60);
    render(&mut engine, 4410);
    assert_eq!(engine.levels().envelope_phase[0], Some(EnvPhase::Release));
}

/// The engine reports where each LFO is in its cycle.
#[test]
fn the_engine_reports_the_lfo_phase() {
    let mut patch = plain_oscillator_patch();
    let mut lfo = crate::patch::LfoSettings::new();
    lfo.rate = Param::fixed(KnobId::LfoRate, 1.0);
    patch.lfos[0] = Some(lfo);
    let mut engine = engine_on(&patch);
    render(&mut engine, 11025);
    let phase = engine.levels().lfo_phase[0];
    assert!((phase - 0.25).abs() < 0.01, "{phase}");
}

/// An effect at the end of a chain still lets the voices through. The oscillator is tapped, its
/// voices are summed for the effect, and the effect's output is what the slot plays.
#[test]
fn an_effect_at_the_end_of_a_chain_plays_the_voices() {
    use crate::patch::stage::ChorusParams;
    let mut source = osc(OscillatorParams::default());
    source.tap = true;
    let dry_chorus = StageSpec::new(StageParams::Chorus(ChorusParams {
        rate: Param::default_of(KnobId::LfoRate),
        depth: Param::default_of(KnobId::Depth),
        spread: Param::default_of(KnobId::Spread),
        mix: Param::fixed(KnobId::Mix, 0.0),
    }))
    .with_input(0, 0);
    let mut engine = engine_on(&patch_with(0, &[source, dry_chorus]));
    engine.note_on(60, 1.0);
    let out = render(&mut engine, 4410);
    assert!(peak(&out) > 0.05, "{}", peak(&out));
}

/// A chorus with none of its effect mixed in. It passes its input through, so a test can use it
/// as a bus stage that changes nothing.
fn clear_effect(source: u8) -> StageSpec {
    use crate::patch::stage::ChorusParams;
    StageSpec::new(StageParams::Chorus(ChorusParams {
        rate: Param::default_of(KnobId::LfoRate),
        depth: Param::default_of(KnobId::Depth),
        spread: Param::default_of(KnobId::Spread),
        mix: Param::fixed(KnobId::Mix, 0.0),
    }))
    .with_input(0, source)
}

/// A mix on the bus, with signal `a` on its first input and `b` on its second.
fn bus_mix(mode: MixMode, amount: f32, a: u8, b: u8) -> StageSpec {
    StageSpec::new(StageParams::Mix(MixParams {
        mode,
        amount: Param::fixed(KnobId::Mix, amount),
    }))
    .with_input(0, a)
    .with_input(1, b)
    .on_bus()
}

/// The oscillator that feeds the bus stages in these tests, tapped so the voices are summed.
fn tapped_source() -> StageSpec {
    let mut source = osc(OscillatorParams::default());
    source.tap = true;
    source
}

/// The left channel of one note played through `stages`, for a second.
fn play_through(stages: &[StageSpec]) -> Vec<f32> {
    let mut engine = engine_on(&patch_with(0, stages));
    engine.note_on(60, 1.0);
    render(&mut engine, 44100)
}

/// Two effect chains from one oscillator, added in a bus mix, sound as the two summed. Two copies
/// of the same signal are twice as loud as one.
#[test]
fn a_bus_mix_adds_two_effect_chains() {
    let one = peak(&play_through(&[tapped_source(), clear_effect(0)]));
    let two = peak(&play_through(&[
        tapped_source(),
        clear_effect(0),
        clear_effect(0),
        bus_mix(MixMode::Add, 1.0, 1, 2),
    ]));
    assert!(one > 0.05, "{one}");
    assert!((two / one - 2.0).abs() < 0.02, "{two} against {one}");
}

/// A crossfade with none of signal B is signal A, so the second chain does not change the sound.
#[test]
fn a_bus_crossfade_at_zero_is_the_first_chain() {
    let one = play_through(&[tapped_source(), clear_effect(0)]);
    let mixed = play_through(&[
        tapped_source(),
        clear_effect(0),
        clear_effect(0),
        bus_mix(MixMode::Crossfade, 0.0, 1, 2),
    ]);
    assert_eq!(one, mixed);
}

/// A dry path and an effect path can meet. The mix reads the summed oscillator on one input.
#[test]
fn a_dry_path_and_a_bus_path_meet_in_a_bus_mix() {
    let one = peak(&play_through(&[tapped_source(), clear_effect(0)]));
    let both = peak(&play_through(&[
        tapped_source(),
        clear_effect(0),
        bus_mix(MixMode::Add, 1.0, 0, 1),
    ]));
    assert!((both / one - 2.0).abs() < 0.02, "{both} against {one}");
}

/// A mix with an FM mode on the bus has no note pitch to scale by, so it passes signal A through.
#[test]
fn an_fm_mix_on_the_bus_passes_signal_a_through() {
    let plain = play_through(&[tapped_source(), clear_effect(0)]);
    let fm = play_through(&[
        tapped_source(),
        clear_effect(0),
        bus_mix(MixMode::Fm, 1.0, 1, 1),
    ]);
    assert_eq!(plain, fm);
}

/// A filter after an effect filters the summed signal. A low pass well below the note leaves
/// little of it.
#[test]
fn a_filter_on_the_bus_filters_the_sum() {
    let filter = |source: u8| {
        StageSpec::new(StageParams::Filter(FilterParams {
            cutoff: Param::fixed(KnobId::Cutoff, 100.0),
            resonance: Param::fixed(KnobId::Resonance, 0.0),
            ..FilterParams::default()
        }))
        .with_input(0, source)
        .on_bus()
    };
    let open = peak(&play_through(&[tapped_source(), clear_effect(0)]));
    let closed = peak(&play_through(&[
        tapped_source(),
        clear_effect(0),
        filter(1),
    ]));
    assert!(open > 0.05, "{open}");
    assert!(closed < open * 0.4, "{closed} against {open}");
}

/// The voices of the bus mix's source are summed once for the bus, not once per bus stage that
/// reads them, so the dry path is the same level however many stages read it.
#[test]
fn the_taps_sum_the_voices_once() {
    let one = peak(&play_through(&[tapped_source(), clear_effect(0)]));
    let two = peak(&play_through(&[
        tapped_source(),
        clear_effect(0),
        clear_effect(0),
        bus_mix(MixMode::Crossfade, 0.0, 1, 2),
    ]));
    assert!((one - two).abs() < 1e-6, "{one} {two}");
}

/// The output peak is the loudest sample on each side since it was last read, and reading it
/// starts a new window.
#[test]
fn the_output_peak_reports_and_resets() {
    let panned = osc(OscillatorParams {
        pan: Param::fixed(KnobId::Pan, -1.0),
        ..OscillatorParams::default()
    });
    let mut engine = engine_on(&patch_with(0, &[panned]));
    assert_eq!(engine.take_output_peak(), [0.0, 0.0]);

    engine.note_on(60, 1.0);
    let out = render_stereo(&mut engine, 4410);
    let [left, right] = engine.take_output_peak();
    let expected = out.iter().map(|f| f.l.abs()).fold(0.0f32, f32::max);
    assert_eq!(left, expected);
    assert!(left > 0.05 && right < 1e-5, "{left} {right}");

    // The window starts over, and silence after the note reads as zero.
    engine.note_off(60);
    render_stereo(&mut engine, 44100);
    engine.take_output_peak();
    render_stereo(&mut engine, 100);
    assert_eq!(engine.take_output_peak(), [0.0, 0.0]);
}

/// A monitor stage that listens to the stage at `source`.
fn monitor_stage(mode: MonitorMode, source: u8) -> StageSpec {
    StageSpec::new(StageParams::Monitor(MonitorParams { mode })).with_input(0, source)
}

/// A chord through `stages` in slot 0, with a release, as stereo frames.
fn play_chord(stages: &[StageSpec]) -> Vec<Frame> {
    let mut engine = engine_on(&patch_with(0, stages));
    for note in [48, 55, 64] {
        engine.note_on(note, 0.9);
    }
    let mut out = render_stereo(&mut engine, 6000);
    engine.note_off(55);
    out.extend(render_stereo(&mut engine, 6000));
    out
}

fn wide_saw() -> StageSpec {
    osc(OscillatorParams {
        waveform: crate::dsp::wavetable::Waveform::Saw,
        unison: Param::fixed(KnobId::Unison, 3.0),
        pan: Param::fixed(KnobId::Pan, 0.4),
        ..OscillatorParams::default()
    })
}

/// A monitor never changes the sound, wherever it sits: after a per-voice root, in the middle of
/// a per-voice chain, or after a bus effect.
#[test]
fn a_monitor_leaves_the_sound_unchanged() {
    let filter = || StageSpec::new(StageParams::Filter(FilterParams::default())).with_input(0, 0);
    let tapped = |mut spec: StageSpec| {
        spec.tap = true;
        spec
    };

    // A per-voice root with a monitor after it, which the bus then plays.
    let plain = play_chord(&[wide_saw(), filter()]);
    let on_root = play_chord(&[
        wide_saw(),
        tapped(filter()),
        monitor_stage(MonitorMode::Levels, 1),
    ]);
    assert_eq!(plain, on_root);

    // A monitor in the middle of a per-voice chain, off to the side of the root.
    let beside = play_chord(&[
        tapped(wide_saw()),
        monitor_stage(MonitorMode::Scope, 0),
        filter(),
    ]);
    assert_eq!(plain, beside);

    // A bus effect as the root, and a monitor on it and on the source.
    let dry = play_chord(&[tapped(wide_saw()), clear_effect(0)]);
    let watched = play_chord(&[
        tapped(wide_saw()),
        monitor_stage(MonitorMode::Spectrum, 0),
        clear_effect(0),
        monitor_stage(MonitorMode::Vectorscope, 2),
    ]);
    assert_eq!(dry, watched);
    assert!(peak(&plain.iter().map(|f| f.l).collect::<Vec<_>>()) > 0.05);
}

/// The engine hands out what each monitor instance measured, by pool number, and an instance with
/// no stage keeps its starting values.
#[test]
fn the_engine_reports_what_a_monitor_measured() {
    let mut watcher = monitor_stage(MonitorMode::Levels, 0);
    watcher.pool = 1;
    let mut engine = engine_on(&patch_with(0, &[tapped_source(), watcher]));
    engine.note_on(60, 1.0);
    render_stereo(&mut engine, 8192);
    let monitors = engine.monitors();
    assert!(monitors[1].levels[0] > -40.0, "{}", monitors[1].levels[0]);
    assert_eq!(monitors[0], MonitorData::default());
    engine.reset();
    assert_eq!(engine.monitors()[1], MonitorData::default());
}

/// A gain stage fed by `source`.
fn gain_stage(gain_db: f32, pan: f32, invert: bool, source: u8) -> StageSpec {
    use crate::patch::stage::GainParams;
    StageSpec::new(StageParams::Gain(GainParams {
        gain: Param::fixed(KnobId::UtilGain, gain_db),
        pan: Param::fixed(KnobId::Pan, pan),
        invert,
    }))
    .with_input(0, source)
}

/// Both channels of one note played through `stages`.
fn play_stereo_through(stages: &[StageSpec]) -> Vec<Frame> {
    let mut engine = engine_on(&patch_with(0, stages));
    engine.note_on(60, 1.0);
    render_stereo(&mut engine, 8000)
}

/// Checks that `out` is `dry` scaled by 6 dB, panned half right, and inverted.
fn assert_inverted_scaled_and_panned(dry: &[Frame], out: &[Frame]) {
    let gain = crate::dsp::gain::utility_db_to_linear(6.0);
    assert!(peak(&dry.iter().map(|f| f.l).collect::<Vec<_>>()) > 0.05);
    for (dry, out) in dry.iter().zip(out) {
        assert!((out.l + dry.l * gain * 0.5).abs() < 1e-4, "{dry:?} {out:?}");
        assert!((out.r + dry.r * gain).abs() < 1e-4, "{dry:?} {out:?}");
    }
}

/// A played note comes out of a per-voice gain stage inverted, scaled, and panned as set.
#[test]
fn a_per_voice_gain_inverts_scales_and_pans_a_note() {
    let source = osc(OscillatorParams::default());
    let dry = play_stereo_through(&[source]);
    let out = play_stereo_through(&[source, gain_stage(6.0, 0.5, true, 0)]);
    assert_inverted_scaled_and_panned(&dry, &out);
}

/// The same stage on the bus, after an effect that changes nothing, does the same.
#[test]
fn a_bus_gain_inverts_scales_and_pans_a_note() {
    let dry = play_stereo_through(&[tapped_source(), clear_effect(0)]);
    let stages = [
        tapped_source(),
        clear_effect(0),
        gain_stage(6.0, 0.5, true, 1).on_bus(),
    ];
    assert!(stages[2].bus);
    let out = play_stereo_through(&stages);
    assert_inverted_scaled_and_panned(&dry, &out);
}

/// A gain at its minimum silences the note, per voice and on the bus.
#[test]
fn a_gain_at_its_minimum_is_silent() {
    let source = osc(OscillatorParams::default());
    let per_voice = play_stereo_through(&[source, gain_stage(-60.0, 0.0, false, 0)]);
    assert!(per_voice.iter().all(|f| *f == Frame::SILENT));
    let bus = play_stereo_through(&[
        tapped_source(),
        clear_effect(0),
        gain_stage(-60.0, 0.0, false, 1).on_bus(),
    ]);
    assert!(bus.iter().all(|f| *f == Frame::SILENT));
}

/// A gain that is bypassed compiles away, so the signal passes through. In the engine that is a
/// slot with no gain stage in it, and a neutral gain stage sounds the same.
#[test]
fn a_neutral_gain_passes_the_signal_through() {
    let source = osc(OscillatorParams::default());
    let dry = play_stereo_through(&[source]);
    let through = play_stereo_through(&[source, gain_stage(0.0, 0.0, false, 0)]);
    assert_eq!(dry, through);
}

/// Moving the knob while a note plays eases. No two neighboring samples jump by more than the
/// note itself could move them, so there is no click.
#[test]
fn a_gain_move_while_playing_does_not_click() {
    let source = osc(OscillatorParams::default());
    let loud = patch_with(0, &[source, gain_stage(0.0, 0.0, false, 0)]);
    let quiet = patch_with(0, &[source, gain_stage(-40.0, 0.0, false, 0)]);
    let mut engine = engine_on(&loud);
    engine.note_on(60, 1.0);
    render(&mut engine, 4000);
    engine.begin_block(&quiet, Transport::default());
    let out = render(&mut engine, 4000);
    let biggest_step = out
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(biggest_step < 0.1, "{biggest_step}");
    assert!(peak(&out[3000..]) < peak(&out[..200]) * 0.2);
}
