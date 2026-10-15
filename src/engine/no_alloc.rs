//! Proves the audio path never allocates. `nih-plug` installs an allocator that aborts on any
//! allocation inside `assert_no_alloc`, the same guard the host-facing `process` runs under in
//! debug builds. This test plays a busy patch under it.

use super::*;
use crate::dsp::mix::MixMode;
use crate::dsp::warp::WarpMode;
use crate::dsp::wavetable::Waveform;
use crate::nodes::knobs::KnobId;
use crate::patch::control::Param;
use crate::patch::stage::{FilterParams, MixParams, OscillatorParams, StageParams, StageSpec};
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
    // A second slot plays an FM pair: a sine modulator feeding a saw carrier.
    let carrier = OscillatorParams {
        waveform: Waveform::Saw,
        warp: WarpMode::Fm,
        warp_amount: Param::fixed(KnobId::WarpAmount, 0.8),
        ..OscillatorParams::default()
    };
    patch.lanes[1].stages.extend([
        StageSpec::new(StageParams::Oscillator(OscillatorParams::default())),
        StageSpec::new(StageParams::Oscillator(carrier)).with_input(0, 0),
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
    ]);
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
                peak = peak.max(engine.process_sample(n).abs());
            }
            engine.levels();
            engine.active_notes();
        }
    });

    // The patch really played, so the guard covered the voices, the stages, and the LFO.
    assert!(peak > 0.01, "peak {peak}");
}
