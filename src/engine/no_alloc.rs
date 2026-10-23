//! Proves the audio path never allocates. `nih-plug` installs an allocator that aborts on any
//! allocation inside `assert_no_alloc`, the same guard the host-facing `process` runs under in
//! debug builds. This test plays a busy patch under it.

use super::*;
use crate::dsp::curve::Curve;
use crate::dsp::effects::distortion::DistortionKind;
use crate::dsp::effects::eq::EqKind;
use crate::dsp::mix::MixMode;
use crate::dsp::monitor::MonitorMode;
use crate::dsp::sample::SampleData;
use crate::dsp::warp::WarpMode;
use crate::dsp::wavetable::Waveform;
use crate::nodes::arpeggiator::ArpPattern;
use crate::nodes::chord::ChordType;
use crate::nodes::eq;
use crate::nodes::knobs::KnobId;
use crate::nodes::math::MathOp;
use crate::nodes::sampler::LoopMode;
use crate::patch::control::{MapNode, MapValue, MathNode, Param, Source};
use crate::patch::stage::{
    ChorusParams, CompressorParams, DelayParams, DistortionParams, EqBandParams, EqParams,
    FilterParams, FlangerParams, GainParams, LaneGraph, MixParams, MonitorParams, OscillatorParams,
    PhaserParams, ReverbParams, SamplerParams, StageParams, StageSpec,
};
use crate::patch::{
    ArpeggiatorSettings, ChordSettings, EnvelopeSettings, LfoSettings, NoteSourceSpec,
};
use assert_no_alloc::assert_no_alloc;
use std::sync::Arc;

/// A patch that uses every stage kind: two oscillators with unison, an envelope, noise, a filter
/// inside one mix branch, a second filter after the mix, and an LFO. Slot 0's notes come from an
/// arpeggiator reading a MIDI input slot, rather than straight from the host, so the guard also
/// covers that path: the arpeggiator's own step clock and the notes it dispatches into a lane. A
/// third note-source slot is a chord reading the same MIDI input directly, so the guard covers its
/// per-sample diff and the chord it dispatches too, even though nothing plays from it today.
fn busy_patch() -> Patch {
    let saw = OscillatorParams {
        waveform: Waveform::Saw,
        unison: Param::fixed(KnobId::Unison, 5.0),
        envelope: Some(0),
        notes: Some(1),
        ..OscillatorParams::default()
    };
    let noise = OscillatorParams {
        waveform: Waveform::Noise,
        notes: Some(1),
        ..OscillatorParams::default()
    };
    // A small sample, built here like `WaveTables::new()`, before the audio thread ever touches
    // it. The stage only ever holds an `Arc` to it afterward.
    let sample = Arc::new(SampleData {
        sample_rate: 44100.0,
        channels: 2,
        frames: (0..200)
            .flat_map(|i| {
                let v = (i as f32 / 200.0) * 2.0 - 1.0;
                [v, -v]
            })
            .collect(),
    });
    let sampler = SamplerParams {
        data: Some(sample),
        envelope: Some(0),
        root: Param::fixed(KnobId::SamplerRoot, 48.0),
        loop_mode: LoopMode::PingPong,
        loop_start: Param::fixed(KnobId::SamplerLoopStart, 10.0),
        loop_end: Param::fixed(KnobId::SamplerLoopEnd, 90.0),
        reverse: true,
        ..SamplerParams::default()
    };

    let mut patch = Patch::silent();
    patch.note_sources[0] = Some(NoteSourceSpec::Input);
    patch.note_sources[1] = Some(NoteSourceSpec::Arpeggiator(ArpeggiatorSettings {
        input: Some(0),
        pattern: ArpPattern::Up,
        rate: Param::fixed(KnobId::LfoRate, 8.0),
        sync: Param::fixed(KnobId::LfoSync, 0.0),
        octaves: Param::fixed(KnobId::ArpOctaves, 2.0),
        gate: Param::fixed(KnobId::ArpGate, 0.8),
    }));
    patch.note_sources[2] = Some(NoteSourceSpec::Chord(ChordSettings {
        input: Some(0),
        chord: ChordType::Major7,
        spread: Param::fixed(KnobId::ChordSpread, 0.5),
    }));
    patch.lanes[0].notes_slot = Some(1);
    patch.lanes[0].stages.extend([
        StageSpec::new(StageParams::Oscillator(saw)),
        StageSpec::new(StageParams::Oscillator(noise)),
        StageSpec::new(StageParams::Filter(FilterParams::default())).with_input(0, 0),
        StageSpec::new(StageParams::Mix(MixParams::default()))
            .with_input(0, 2)
            .with_input(1, 1),
        StageSpec::new(StageParams::Filter(FilterParams::default())).with_input(0, 3),
        StageSpec::new(StageParams::Sampler(sampler)),
        StageSpec::new(StageParams::Mix(MixParams {
            mode: MixMode::Add,
            amount: Param::fixed(KnobId::Mix, 1.0),
        }))
        .with_input(0, 4)
        .with_input(1, 5),
    ]);
    // A second slot plays a warped, panned saw into two parallel effect chains. One goes through a
    // filter on the bus, and a mix on the bus adds the two. This covers the bus filter and mix.
    let warped = OscillatorParams {
        waveform: Waveform::Saw,
        warp: WarpMode::Pwm,
        warp_amount: Param::fixed(KnobId::WarpAmount, 0.8),
        pan: Param::fixed(KnobId::Pan, -0.5),
        ..OscillatorParams::default()
    };
    let mut warped_source = StageSpec::new(StageParams::Oscillator(warped));
    warped_source.tap = true;
    let mut chorus = StageSpec::new(StageParams::Chorus(ChorusParams {
        rate: Param::default_of(KnobId::LfoRate),
        depth: Param::default_of(KnobId::Depth),
        spread: Param::default_of(KnobId::Spread),
        mix: Param::default_of(KnobId::Mix),
    }))
    .with_input(0, 0);
    chorus.pool = 1;
    let mut delay = StageSpec::new(StageParams::Delay(DelayParams {
        time: Param::default_of(KnobId::DelayTime),
        sync: Param::default_of(KnobId::LfoSync),
        feedback: Param::default_of(KnobId::Feedback),
        damp: Param::default_of(KnobId::Damp),
        mix: Param::default_of(KnobId::Mix),
        ping_pong: false,
    }))
    .with_input(0, 0);
    delay.pool = 1;
    let bus_filter = StageSpec::new(StageParams::Filter(FilterParams::default()))
        .with_input(0, 1)
        .on_bus();
    let add = StageSpec::new(StageParams::Mix(MixParams {
        mode: MixMode::Add,
        amount: Param::fixed(KnobId::Mix, 1.0),
    }))
    .with_input(0, 3)
    .with_input(1, 2)
    .on_bus();
    // A gain on the bus after the mix, and one per voice after the FM mix of the next slot. Both
    // pan, invert, and ease, so the guard covers the pooled and the per-voice gain.
    let gain = |source| {
        StageSpec::new(StageParams::Gain(GainParams {
            gain: Param::fixed(KnobId::UtilGain, 3.0),
            pan: Param::fixed(KnobId::Pan, -0.3),
            invert: true,
        }))
        .with_input(0, source)
    };
    patch.lanes[1].stages.extend([
        warped_source,
        chorus,
        delay,
        bus_filter,
        add,
        gain(4).on_bus(),
    ]);
    // A third slot mixes two oscillators by FM, which runs a delay line per voice.
    let fm_mix = MixParams {
        mode: MixMode::Fm,
        amount: Param::fixed(KnobId::Mix, 0.7),
    };
    patch.lanes[2].stages.extend([
        StageSpec::new(StageParams::Oscillator(OscillatorParams::default())),
        StageSpec::new(StageParams::Oscillator(OscillatorParams {
            waveform: Waveform::Triangle,
            ..OscillatorParams::default()
        })),
        StageSpec::new(StageParams::Mix(fm_mix))
            .with_input(0, 0)
            .with_input(1, 1),
        gain(2),
    ]);
    // A fourth slot runs a saw through all seven effects, with the two switches on.
    let chain = [
        StageParams::Distortion(DistortionParams {
            kind: DistortionKind::Fold,
            drive: Param::fixed(KnobId::Drive, 0.7),
            tone: Param::default_of(KnobId::Tone),
            level: Param::default_of(KnobId::OscGain),
            mix: Param::fixed(KnobId::Mix, 1.0),
        }),
        StageParams::Compressor(CompressorParams {
            threshold: Param::default_of(KnobId::Threshold),
            ratio: Param::default_of(KnobId::Ratio),
            attack: Param::default_of(KnobId::CompAttack),
            release: Param::default_of(KnobId::CompRelease),
            makeup: Param::default_of(KnobId::Makeup),
            mix: Param::fixed(KnobId::Mix, 1.0),
            multiband: true,
            depth: Param::fixed(KnobId::Depth, 1.0),
        }),
        StageParams::Reverb(ReverbParams {
            decay: Param::default_of(KnobId::ReverbDecay),
            size: Param::default_of(KnobId::Size),
            damp: Param::default_of(KnobId::Damp),
            mix: Param::default_of(KnobId::Mix),
        }),
        StageParams::Delay(DelayParams {
            time: Param::default_of(KnobId::DelayTime),
            sync: Param::default_of(KnobId::LfoSync),
            feedback: Param::default_of(KnobId::Feedback),
            damp: Param::default_of(KnobId::Damp),
            mix: Param::default_of(KnobId::Mix),
            ping_pong: true,
        }),
        StageParams::Chorus(ChorusParams {
            rate: Param::default_of(KnobId::LfoRate),
            depth: Param::default_of(KnobId::Depth),
            spread: Param::default_of(KnobId::Spread),
            mix: Param::default_of(KnobId::Mix),
        }),
        StageParams::Phaser(PhaserParams {
            rate: Param::default_of(KnobId::LfoRate),
            depth: Param::default_of(KnobId::Depth),
            feedback: Param::default_of(KnobId::Feedback),
            center: Param::default_of(KnobId::Center),
            stages: 12,
            spread: Param::default_of(KnobId::Spread),
            mix: Param::default_of(KnobId::Mix),
        }),
        StageParams::Eq(EqParams {
            // The analyzer runs under the guard too.
            analyzer: true,
            bands: std::array::from_fn(|band| EqBandParams {
                kind: if band % 2 == 0 {
                    EqKind::HighShelf
                } else {
                    EqKind::Peak
                },
                freq: Param::default_of(eq::FREQ[band]),
                gain: Param::fixed(eq::GAIN[band], 9.0),
                q: Param::default_of(eq::Q[band]),
            }),
        }),
        StageParams::Flanger(FlangerParams {
            rate: Param::default_of(KnobId::LfoRate),
            depth: Param::default_of(KnobId::Depth),
            feedback: Param::default_of(KnobId::Feedback),
            delay: Param::default_of(KnobId::FlangerDelay),
            spread: Param::default_of(KnobId::Spread),
            mix: Param::default_of(KnobId::Mix),
        }),
    ];
    let mut source = StageSpec::new(StageParams::Oscillator(OscillatorParams {
        waveform: Waveform::Saw,
        ..OscillatorParams::default()
    }));
    source.tap = true;
    patch.lanes[3].stages.push(source);
    for (index, params) in chain.into_iter().enumerate() {
        let mut spec = StageSpec::new(params).with_input(0, index as u8);
        spec.pool = 0;
        patch.lanes[3].stages.push(spec);
    }
    // Maps chain and read an envelope and an LFO, so the guard covers working them out per voice.
    for map in [
        MapNode {
            input: Source::Envelope(0),
            in_lo: MapValue::fixed(0.0),
            in_hi: MapValue::fixed(1.0),
            out_lo: MapValue::fixed(0.0),
            out_hi: MapValue::fixed(1.0),
            offset: MapValue {
                value: 0.0,
                source: Some(Source::Lfo(0)),
            },
            center_range: None,
        },
        // A bipolar map, with its range driven by the LFO.
        MapNode {
            input: Source::Signal(0),
            in_lo: MapValue::fixed(0.0),
            in_hi: MapValue::fixed(1.0),
            out_lo: MapValue::fixed(0.0),
            out_hi: MapValue::fixed(1.0),
            offset: MapValue::fixed(0.0),
            center_range: Some((
                MapValue::fixed(0.5),
                MapValue {
                    value: 1.0,
                    source: Some(Source::Lfo(0)),
                },
            )),
        },
    ] {
        patch.signals.push(map);
    }
    // A math node that reads the second map and the LFO, so the guard covers it too.
    patch.signals.push(MathNode {
        op: MathOp::Power,
        a: MapValue {
            value: 0.0,
            source: Some(Source::Signal(1)),
        },
        b: MapValue {
            value: 0.5,
            source: Some(Source::Lfo(0)),
        },
    });
    // A monitor in each slot, in a different mode. Three listen to the end of the slot and one to
    // a source.
    let watch = |lane: &mut LaneGraph, mode, source: u8, pool: u8| {
        let mut spec =
            StageSpec::new(StageParams::Monitor(MonitorParams { mode })).with_input(0, source);
        spec.pool = pool;
        if !lane.stages[source as usize].bus {
            lane.stages[source as usize].tap = true;
        }
        lane.stages.push(spec);
    };
    watch(&mut patch.lanes[0], MonitorMode::Scope, 4, 0);
    watch(&mut patch.lanes[1], MonitorMode::Levels, 0, 1);
    watch(&mut patch.lanes[2], MonitorMode::Vectorscope, 2, 2);
    watch(&mut patch.lanes[3], MonitorMode::Spectrum, 8, 3);
    patch.envelopes[0] = Some(EnvelopeSettings::new());
    // A drawn curve in one shot mode, which the notes in the test restart.
    patch.lfos[0] = Some(LfoSettings {
        curve: Curve::from_points(&[[0.0, 0.1], [0.3, 0.9], [0.6, 0.4], [1.0, 0.7]]),
        one_shot: true,
        rate: Param::fixed(KnobId::LfoRate, 20.0),
        ..LfoSettings::new()
    });
    patch
}

#[test]
fn the_audio_path_never_allocates() {
    let patch = busy_patch();
    let mut engine = Engine::new(Arc::new(WaveTables::new()));
    engine.begin_block(&patch, Transport::default());

    let mut peak = 0.0f32;
    assert_no_alloc(|| {
        for block in 0..6 {
            engine.begin_block(&patch, Transport::default());
            if block == 0 {
                for note in [60, 64, 67, 72] {
                    engine.note_on(note, 0.8);
                }
            }
            if block == 2 {
                engine.note_off(60);
                engine.note_off(64);
            }
            engine.pitch_bend(0.7);
            for n in 0..512 {
                peak = peak.max(engine.process_sample(n).peak());
            }
            engine.levels();
            engine.active_notes();
            engine.monitors();
        }
    });

    // The patch really played, so the guard covered the voices, the stages, and the LFO.
    assert!(peak > 0.01, "peak {peak}");
}

/// Every effect stays finite and bounded through a long run that includes the tails after the
/// notes end. A chord of saws through all seven effects at once is a hard case for a feedback
/// loop.
#[test]
fn the_effect_chain_stays_finite_and_bounded() {
    let patch = busy_patch();
    let mut engine = Engine::new(Arc::new(WaveTables::new()));
    let mut worst = 0.0f32;
    for block in 0..400 {
        engine.begin_block(&patch, Transport::default());
        if block == 0 {
            for note in [36, 48, 60, 72] {
                engine.note_on(note, 1.0);
            }
        }
        if block == 100 {
            for note in [36, 48, 60, 72] {
                engine.note_off(note);
            }
        }
        for n in 0..512 {
            let out = engine.process_sample(n);
            assert!(out.l.is_finite() && out.r.is_finite(), "block {block}");
            worst = worst.max(out.peak());
        }
    }
    assert!(worst < 8.0, "peak {worst}");
}
