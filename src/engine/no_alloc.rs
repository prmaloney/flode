//! Proves the audio path never allocates. `nih-plug` installs an allocator that aborts on any
//! allocation inside `assert_no_alloc`, the same guard the host-facing `process` runs under in
//! debug builds. This test plays a busy patch under it.

use super::*;
use crate::dsp::effects::distortion::DistortionKind;
use crate::dsp::mix::MixMode;
use crate::dsp::warp::WarpMode;
use crate::dsp::wavetable::Waveform;
use crate::nodes::knobs::KnobId;
use crate::patch::control::Param;
use crate::patch::stage::{
    ChorusParams, CompressorParams, DelayParams, DistortionParams, FilterParams, FlangerParams,
    MixParams, OscillatorParams, PhaserParams, ReverbParams, StageParams, StageSpec,
};
use crate::patch::{EnvelopeSettings, LfoSettings};
use assert_no_alloc::assert_no_alloc;

/// A patch that uses every stage kind: two oscillators with unison, an envelope, noise, a filter
/// inside one mix branch, a second filter after the mix, and an LFO.
fn busy_patch() -> Patch {
    let saw = OscillatorParams {
        waveform: Waveform::Saw,
        unison: Param::fixed(KnobId::Unison, 5.0),
        envelope: Some(0),
        ..OscillatorParams::default()
    };
    let noise = OscillatorParams {
        waveform: Waveform::Noise,
        ..OscillatorParams::default()
    };
    let mut patch = Patch::silent();
    patch.lanes[0].stages.extend([
        StageSpec::new(StageParams::Oscillator(saw)),
        StageSpec::new(StageParams::Oscillator(noise)),
        StageSpec::new(StageParams::Filter(FilterParams::default())).with_input(0, 0),
        StageSpec::new(StageParams::Mix(MixParams::default()))
            .with_input(0, 2)
            .with_input(1, 1),
        StageSpec::new(StageParams::Filter(FilterParams::default())).with_input(0, 3),
    ]);
    // A second slot plays a warped, panned saw.
    let warped = OscillatorParams {
        waveform: Waveform::Saw,
        warp: WarpMode::Pwm,
        warp_amount: Param::fixed(KnobId::WarpAmount, 0.8),
        pan: Param::fixed(KnobId::Pan, -0.5),
        ..OscillatorParams::default()
    };
    patch.lanes[1]
        .stages
        .push(StageSpec::new(StageParams::Oscillator(warped)));
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
    patch.envelopes[0] = Some(EnvelopeSettings::new());
    patch.lfos[0] = Some(LfoSettings::new());
    patch
}

#[test]
fn the_audio_path_never_allocates() {
    let patch = busy_patch();
    let mut engine = Engine::new(Arc::new(WaveTables::new()));
    engine.begin_block(&patch, Transport::default());

    let mut peak = 0.0f32;
    assert_no_alloc(|| {
        for block in 0..4 {
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
