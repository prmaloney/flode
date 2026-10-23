//! The sampler stage: a source with no audio inputs, like an oscillator, but reading a loaded
//! audio file instead of a wavetable. Each voice keeps its own fractional read position, so held
//! notes each play their own place in the file.

use super::{AudioStage, StartContext, Tick};
use crate::dsp::frame::{pan_gains, Frame};
use crate::dsp::gain::{balance, db_to_linear};
use crate::dsp::sample::SampleData;
use crate::nodes::sampler::LoopMode;
use crate::patch::control::{ControlLevels, MAX_ENVELOPES};
use crate::patch::stage::{SamplerParams, MAX_INPUTS};
use nih_plug::util;

pub struct Sampler;

/// What one voice remembers of a sampler: its place in the file and, for ping-pong looping, which
/// way it is currently reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplerState {
    /// A fractional frame index into the loaded file.
    position: f32,
    /// 1.0 forward, -1.0 backward. Only ping-pong flips it after it starts; `Reverse` folds into
    /// the step instead, so this always starts at 1.0.
    direction: f32,
    /// True once a non-looping voice has run off the end (or, in reverse, off the start). A
    /// finished voice stays silent, the same as a voice whose envelope has fully released.
    finished: bool,
}

#[cfg(test)]
impl SamplerState {
    pub fn position(&self) -> f32 {
        self.position
    }
}

/// The frame at index `i` of `data`, or silence for an index before the start of the file. `data`
/// itself already returns silence for an index past its end.
fn frame_at(data: &SampleData, i: i64) -> (f32, f32) {
    if i < 0 {
        (0.0, 0.0)
    } else {
        data.frame(i as usize)
    }
}

/// Linear interpolation between the two nearest frames at fractional position `pos`.
fn read(data: &SampleData, pos: f32) -> (f32, f32) {
    let i0 = pos.floor();
    let frac = pos - i0;
    let (l0, r0) = frame_at(data, i0 as i64);
    let (l1, r1) = frame_at(data, i0 as i64 + 1);
    (l0 + (l1 - l0) * frac, r0 + (r1 - r0) * frac)
}

/// The loop's bounds in frames, defensively clamped so the start never sits past the end even if
/// the knobs or a saved patch would otherwise put it there.
fn loop_bounds(p: &SamplerParams, levels: &ControlLevels, len: f32) -> (f32, f32) {
    let start = (p.loop_start.plain(levels).clamp(0.0, 100.0) / 100.0 * len).min(len);
    let end = (p.loop_end.plain(levels).clamp(0.0, 100.0) / 100.0 * len)
        .max(start + 1.0)
        .min(len);
    (start, end)
}

impl AudioStage for Sampler {
    type Params = SamplerParams;
    type State = SamplerState;

    fn start(p: &SamplerParams, ctx: &mut StartContext) -> SamplerState {
        let len = p.data.as_ref().map_or(0.0, |d| d.len() as f32);
        let start_pct = p.start.plain(ctx.levels).clamp(0.0, 100.0) / 100.0;
        SamplerState {
            position: start_pct * len,
            direction: 1.0,
            finished: len <= 0.0,
        }
    }

    fn process(
        p: &SamplerParams,
        state: &mut SamplerState,
        _inputs: &[Frame; MAX_INPUTS],
        tick: &mut Tick,
    ) -> Frame {
        let levels = tick.levels;
        let Some(data) = &p.data else {
            return Frame::SILENT;
        };
        if data.is_empty() || state.finished {
            return Frame::SILENT;
        }
        let len = data.len() as f32;

        let (l, r) = read(data, state.position);

        // The step per output sample, in file frames. Pitch tracking scales it by how far the
        // voice's note sits from the root key. Off, it always plays at the file's own speed.
        let native_step = data.sample_rate / tick.sample_rate;
        let rate = if p.pitch_tracking {
            let root =
                util::midi_note_to_freq(p.root.plain(levels).clamp(0.0, 127.0).round() as u8);
            let voice_freq = tick.pitch_step * tick.sample_rate;
            native_step * voice_freq / root
        } else {
            native_step
        };
        let step = rate * state.direction * if p.reverse { -1.0 } else { 1.0 };

        let (loop_start, loop_end) = loop_bounds(p, levels, len);
        state.position += step;

        match p.loop_mode {
            LoopMode::Off => {
                if state.position >= len || state.position < 0.0 {
                    state.finished = true;
                }
            }
            LoopMode::Forward => {
                if state.position >= loop_end {
                    state.position = loop_start + (state.position - loop_end);
                } else if state.position < loop_start {
                    state.position = loop_end - (loop_start - state.position);
                }
            }
            LoopMode::PingPong => {
                if state.position >= loop_end {
                    state.position = loop_end - (state.position - loop_end);
                    state.direction = -state.direction;
                } else if state.position < loop_start {
                    state.position = loop_start + (loop_start - state.position);
                    state.direction = -state.direction;
                }
            }
        }

        // One shot ignores the envelope entirely, so a note off never cuts it short. Gated, and
        // an unwired sampler, follow the envelope exactly as an oscillator does.
        let amplitude = if p.one_shot {
            1.0
        } else {
            match p.envelope {
                Some(index) => tick.envs[index].level,
                None if tick.envelopes_active => 1.0,
                None => tick.envs[MAX_ENVELOPES].level,
            }
        };
        let gain = tick.velocity * amplitude * db_to_linear(p.level.plain(levels));

        if data.channels <= 1 {
            pan_gains(p.pan.plain(levels)) * (l * gain)
        } else {
            let (bl, br) = balance(p.pan.plain(levels));
            Frame::new(l * bl, r * br) * gain
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::stage::testing::Rig;
    use crate::nodes::knobs::KnobId;
    use crate::patch::control::Param;
    use std::sync::Arc;

    /// A mono file whose frames rise from just above 0.0 to 1.0, so a linear read at a fractional
    /// position is easy to check against its neighbors, and a test that looks for the first exact
    /// silence never mistakes a frame of real data for the end (the real data is never exactly
    /// 0.0; only a read past the end of the file is).
    fn ramp(frames: usize) -> Arc<SampleData> {
        Arc::new(SampleData {
            sample_rate: 44100.0,
            channels: 1,
            frames: (0..frames)
                .map(|i| (i + 1) as f32 / frames as f32)
                .collect(),
        })
    }

    fn params(data: Arc<SampleData>) -> SamplerParams {
        SamplerParams {
            data: Some(data),
            ..SamplerParams::default()
        }
    }

    fn run(p: &SamplerParams, samples: usize) -> Vec<Frame> {
        let mut rig = Rig::new();
        let mut state = Sampler::start(p, &mut rig.start_context());
        (0..samples)
            .map(|_| Sampler::process(p, &mut state, &[Frame::SILENT; MAX_INPUTS], &mut rig.tick()))
            .collect()
    }

    /// The rig's voice plays at 440 Hz (A4, note 69). Setting the root to the same note makes the
    /// pitch ratio exactly 1, so the file reads at its own recorded speed: one frame per sample.
    #[test]
    fn a_note_at_the_root_plays_at_the_files_own_speed() {
        let data = ramp(44100);
        let p = SamplerParams {
            root: Param::fixed(KnobId::SamplerRoot, 69.0),
            ..params(data)
        };
        let out = run(&p, 1000);
        // One file frame per output sample: after 1000 samples the ramp has advanced by 1000 of
        // 44100 frames, which is close to the raw ramp value at index 1000.
        let expected = 1000.0 / 44099.0;
        assert!((out[999].l - expected).abs() < 0.01, "{}", out[999].l);
    }

    /// An octave above the root reads the file about twice as fast, so a fixed-length buffer runs
    /// out in about half the samples.
    #[test]
    fn an_octave_up_plays_about_twice_as_fast() {
        let frames = 2000;
        let data = ramp(frames);
        // Root at one octave below the rig's 440 Hz voice, so the voice note is an octave above
        // the root and the sample reads at roughly double speed.
        let p = SamplerParams {
            root: Param::fixed(KnobId::SamplerRoot, 57.0), // A3, 220 Hz
            loop_mode: LoopMode::Off,
            ..params(data)
        };
        let out = run(&p, frames * 2);
        let silent_from = out
            .iter()
            .position(|f| f.peak() == 0.0)
            .unwrap_or(out.len());
        // At roughly double speed the buffer is consumed in about half the frames.
        assert!(
            (silent_from as isize - frames as isize / 2).abs() < 100,
            "{silent_from}"
        );
    }

    /// An octave below the root plays about half as fast: twice the output samples pass before
    /// the buffer runs out, compared to the root rate.
    #[test]
    fn an_octave_down_plays_about_half_as_fast() {
        let frames = 1000;
        let data = ramp(frames);
        let at_root = SamplerParams {
            root: Param::fixed(KnobId::SamplerRoot, 69.0),
            loop_mode: LoopMode::Off,
            ..params(data.clone())
        };
        let an_octave_down = SamplerParams {
            root: Param::fixed(KnobId::SamplerRoot, 81.0), // an octave above the rig's note
            loop_mode: LoopMode::Off,
            ..params(data)
        };
        let silent_at = |p: &SamplerParams| {
            run(p, frames * 4)
                .iter()
                .position(|f| f.peak() == 0.0)
                .unwrap_or(frames * 4)
        };
        let root_end = silent_at(&at_root);
        let down_end = silent_at(&an_octave_down);
        assert!(down_end > root_end * 3 / 2, "{root_end} {down_end}");
    }

    /// With pitch tracking off, every note plays at the same, file-native speed.
    #[test]
    fn pitch_tracking_off_ignores_the_note() {
        let data = ramp(2000);
        let low = SamplerParams {
            root: Param::fixed(KnobId::SamplerRoot, 36.0),
            pitch_tracking: false,
            loop_mode: LoopMode::Off,
            ..params(data.clone())
        };
        let high = SamplerParams {
            root: Param::fixed(KnobId::SamplerRoot, 96.0),
            pitch_tracking: false,
            loop_mode: LoopMode::Off,
            ..params(data)
        };
        let end = |p: &SamplerParams| {
            run(p, 4000)
                .iter()
                .position(|f| f.peak() == 0.0)
                .unwrap_or(4000)
        };
        assert_eq!(end(&low), end(&high));
    }

    /// Reading at a fractional position blends the two nearest frames, instead of repeating the
    /// nearest one, so consecutive reads a half frame apart differ smoothly.
    #[test]
    fn linear_interpolation_smooths_fractional_reads() {
        let data = SampleData {
            sample_rate: 44100.0,
            channels: 1,
            frames: vec![0.0, 1.0, 0.0],
        };
        let (l, _) = read(&data, 0.5);
        assert!((l - 0.5).abs() < 1e-6);
        let (l, _) = read(&data, 0.25);
        assert!((l - 0.25).abs() < 1e-6);
    }

    /// Start offset begins playback partway through the file.
    #[test]
    fn start_offset_begins_partway_through() {
        let data = ramp(1000);
        let p = SamplerParams {
            start: Param::fixed(KnobId::SamplerStart, 50.0),
            ..params(data)
        };
        let mut rig = Rig::new();
        let state = Sampler::start(&p, &mut rig.start_context());
        assert!((state.position - 500.0).abs() < 1.0);
    }

    /// With looping off, the voice goes silent once it reaches the end, and stays silent.
    #[test]
    fn loop_off_goes_silent_at_the_end_and_stays_there() {
        let data = ramp(200);
        let p = SamplerParams {
            root: Param::fixed(KnobId::SamplerRoot, 69.0),
            loop_mode: LoopMode::Off,
            ..params(data)
        };
        let out = run(&p, 400);
        assert!(out[199].peak() > 0.0);
        assert!(out[250].peak() == 0.0);
        assert!(out[399].peak() == 0.0);
    }

    /// Forward looping wraps from loop end back to loop start and keeps playing, never reading
    /// outside the loop region once it has wrapped.
    #[test]
    fn forward_loop_wraps_and_keeps_playing() {
        let data = ramp(1000);
        let p = SamplerParams {
            root: Param::fixed(KnobId::SamplerRoot, 69.0),
            loop_mode: LoopMode::Forward,
            loop_start: Param::fixed(KnobId::SamplerLoopStart, 10.0),
            loop_end: Param::fixed(KnobId::SamplerLoopEnd, 20.0),
            ..params(data)
        };
        let mut rig = Rig::new();
        let mut state = Sampler::start(&p, &mut rig.start_context());
        for _ in 0..5000 {
            Sampler::process(
                &p,
                &mut state,
                &[Frame::SILENT; MAX_INPUTS],
                &mut rig.tick(),
            );
            assert!(
                state.position >= 95.0 && state.position <= 205.0,
                "{}",
                state.position
            );
        }
    }

    /// Reverse plays the file backward from where it starts.
    #[test]
    fn reverse_reads_backward() {
        let data = ramp(1000);
        let p = SamplerParams {
            root: Param::fixed(KnobId::SamplerRoot, 69.0),
            start: Param::fixed(KnobId::SamplerStart, 50.0),
            loop_mode: LoopMode::Off,
            reverse: true,
            ..params(data)
        };
        let mut rig = Rig::new();
        let mut state = Sampler::start(&p, &mut rig.start_context());
        let before = state.position;
        Sampler::process(
            &p,
            &mut state,
            &[Frame::SILENT; MAX_INPUTS],
            &mut rig.tick(),
        );
        assert!(state.position < before);
    }

    /// Gated playback stops shaping amplitude when the envelope releases: with no envelope wired
    /// and no envelope node in the patch, the bypass copy sets it, the same as an oscillator.
    #[test]
    fn gated_follows_the_bypass_envelope() {
        let data = ramp(44100);
        let p = params(data);
        let mut rig = Rig::new();
        rig.envs[MAX_ENVELOPES].level = 0.0;
        let mut state = Sampler::start(&p, &mut rig.start_context());
        let mut peak = 0.0f32;
        for _ in 0..200 {
            peak = peak.max(
                Sampler::process(
                    &p,
                    &mut state,
                    &[Frame::SILENT; MAX_INPUTS],
                    &mut rig.tick(),
                )
                .peak(),
            );
        }
        assert_eq!(peak, 0.0);
    }

    /// A wired envelope shapes a gated sampler's amplitude from its own copy.
    #[test]
    fn a_wired_envelope_shapes_the_amplitude() {
        let data = ramp(44100);
        let p = SamplerParams {
            envelope: Some(0),
            ..params(data)
        };
        let mut rig = Rig::new();
        rig.envs[0].level = 0.5;
        let mut state = Sampler::start(&p, &mut rig.start_context());
        let mut peak = 0.0f32;
        for _ in 0..200 {
            peak = peak.max(
                Sampler::process(
                    &p,
                    &mut state,
                    &[Frame::SILENT; MAX_INPUTS],
                    &mut rig.tick(),
                )
                .peak(),
            );
        }
        assert!(peak > 0.0 && peak <= 0.6, "{peak}");
    }

    /// One shot ignores the envelope entirely, so the bypass envelope's release does not silence
    /// it, unlike a gated sampler under the same conditions.
    #[test]
    fn one_shot_ignores_the_envelope() {
        let data = ramp(44100);
        let p = SamplerParams {
            one_shot: true,
            ..params(data)
        };
        let mut rig = Rig::new();
        rig.envs[MAX_ENVELOPES].level = 0.0;
        let mut state = Sampler::start(&p, &mut rig.start_context());
        let mut peak = 0.0f32;
        for _ in 0..200 {
            peak = peak.max(
                Sampler::process(
                    &p,
                    &mut state,
                    &[Frame::SILENT; MAX_INPUTS],
                    &mut rig.tick(),
                )
                .peak(),
            );
        }
        assert!(peak > 0.0);
    }

    /// With no data loaded, the stage is silent and never panics.
    #[test]
    fn no_data_is_silent() {
        let p = SamplerParams::default();
        let out = run(&p, 100);
        assert!(out.iter().all(|f| f.peak() == 0.0));
    }

    /// Two voices of one sampler keep independent read positions.
    #[test]
    fn voices_keep_independent_positions() {
        let data = ramp(1000);
        let p = params(data);
        let mut rig = Rig::new();
        let mut a = Sampler::start(&p, &mut rig.start_context());
        let b = Sampler::start(&p, &mut rig.start_context());
        let before = b.position;
        for _ in 0..50 {
            Sampler::process(&p, &mut a, &[Frame::SILENT; MAX_INPUTS], &mut rig.tick());
        }
        assert_eq!(b.position, before);
        assert_ne!(a.position, before);
    }
}
