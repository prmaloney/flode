//! The synth engine: what runs on the audio thread. It knows nothing about the plugin API, so a
//! test can play notes into it and read samples out. The plugin feeds it events, and it runs the
//! stages of the current patch for every sounding voice.

mod effects;
mod lane;
mod lfo;
#[cfg(test)]
mod no_alloc;
mod notes;
pub mod stage;
#[cfg(test)]
mod tests;
mod voice;

use crate::dsp::envelope::{AdsrConfig, FADE_SECONDS};
use crate::dsp::frame::Frame;
use crate::dsp::gain::db_to_linear;
use crate::dsp::rng::Rng;
use crate::dsp::wavetable::WaveTables;
use crate::nodes::input::{MidiSource, MIDI_SOURCE_COUNT};
use crate::nodes::macros::MACRO_COUNT;
use crate::nodes::output::MAX_SLOTS;
use crate::patch::control::{ControlLevels, ENV_SLOTS, MAX_ENVELOPES};
use crate::patch::stage::MAX_STAGES;
use crate::patch::Patch;
use effects::Effects;
use lane::Lane;
use lfo::Lfos;
use nih_plug::util;
use notes::HeldNotes;
use stage::{start_arena, StartContext, MAX_DELAY_LINES};
use std::sync::Arc;
use voice::{RenderEnv, Voice};

/// Scale applied to the summed voices so chords don't clip (1 / sqrt(NUM_VOICES)).
const VOICE_MIX: f32 = 0.25;
/// How long gains and cutoffs take to settle after a knob moves. Knob values change once per
/// block, so without this they would step and click.
const SMOOTH_SECONDS: f32 = 0.01;

/// Where the host's playhead is. Synced LFOs follow it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Transport {
    pub beats: Option<f64>,
    pub tempo: Option<f64>,
    pub playing: bool,
}

pub struct Engine {
    sample_rate: f32,
    /// The patch for the current block.
    patch: Patch,
    lanes: [Lane; MAX_SLOTS],
    /// The effects of every slot, which run on the slot's summed voices.
    effects: Effects,
    lfos: Lfos,
    transport: Transport,
    /// The MIDI sources' latest values, from 0.0 to 1.0.
    midi_levels: [f32; MIDI_SOURCE_COUNT],
    /// The macro knobs' latest values, from 0.0 to 1.0.
    macro_levels: [f32; MACRO_COUNT],
    /// Each envelope node's times for the block, with the bypass copy last.
    env_configs: [AdsrConfig; ENV_SLOTS],
    /// The level of each envelope's newest voice. Knobs that no single voice owns read these.
    env_levels: [f32; MAX_ENVELOPES],
    /// True while any envelope node is in the patch. Then the amplitude is the envelopes' job.
    envelopes_active: bool,
    age_counter: u64,
    /// The master gain, easing toward its knob. It applies after the slots are mixed.
    master_gain: f32,
    master_settled: bool,
    /// The loudest left and right output sample since the meter last read it.
    output_peak: [f32; 2],
    /// The share of the gap a smoothed value closes each sample.
    smooth: f32,
    /// True while the output is mono, so one voice sounds at a time.
    mono: bool,
    /// Glide on every mono note, not only while another note is held.
    glide_always: bool,
    glide_seconds: f32,
    glide_coef: f32,
    /// False until a note has played. The first note has no pitch to glide from.
    has_played: bool,
    /// The pitch of the last note played, which the next mono note glides from.
    mono_step: f32,
    held: HeldNotes,
    rng: Rng,
    /// Band-limited waveforms for every pitch range. Built once, read on the audio thread.
    tables: Arc<WaveTables>,
}

impl Engine {
    pub fn new(tables: Arc<WaveTables>) -> Self {
        let mut engine = Self {
            sample_rate: 44100.0,
            patch: Patch::silent(),
            lanes: std::array::from_fn(|_| Lane::new()),
            effects: Effects::new(44100.0),
            lfos: Lfos::new(),
            transport: Transport::default(),
            midi_levels: MidiSource::ALL.map(|source| source.default_level()),
            macro_levels: [0.0; MACRO_COUNT],
            env_configs: [AdsrConfig::BYPASS; ENV_SLOTS],
            env_levels: [0.0; MAX_ENVELOPES],
            envelopes_active: false,
            age_counter: 0,
            master_gain: 0.0,
            master_settled: false,
            output_peak: [0.0; 2],
            smooth: 0.0,
            mono: false,
            glide_always: false,
            glide_seconds: 0.0,
            glide_coef: 1.0,
            has_played: false,
            mono_step: 0.0,
            held: HeldNotes::new(),
            rng: Rng::default(),
            tables,
        };
        engine.set_sample_rate(44100.0);
        engine
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        if sample_rate != self.sample_rate {
            // The effects size their memory by the rate, so they are built again.
            self.effects = Effects::new(sample_rate);
        }
        self.sample_rate = sample_rate;
        self.smooth = 1.0 - (-1.0 / (SMOOTH_SECONDS * sample_rate)).exp();
    }

    /// Stops every note.
    pub fn reset(&mut self) {
        self.held.clear();
        self.lanes.iter_mut().for_each(Lane::clear);
        self.effects.reset();
    }

    /// Every control source's level right now. Knobs read their route from these.
    pub fn levels(&self) -> ControlLevels {
        ControlLevels {
            envelope: self.env_levels,
            lfo: self.lfos.levels(),
            midi: self.midi_levels,
            macros: self.macro_levels,
        }
    }

    /// Sets the macro knobs' levels, from the host parameters. The plugin calls it each sample.
    pub fn set_macros(&mut self, levels: [f32; MACRO_COUNT]) {
        self.macro_levels = levels;
    }

    /// The notes with a voice sounding, as two 64-bit words.
    pub fn active_notes(&self) -> [u64; 2] {
        let mut notes = [0u64; 2];
        for voice in self.lanes.iter().flat_map(Lane::voices) {
            notes[(voice.note / 64) as usize] |= 1u64 << (voice.note % 64);
        }
        notes
    }

    /// Takes the patch and the playhead for the next block. Block-rate values, such as envelope
    /// times and the glide, are read here.
    pub fn begin_block(&mut self, patch: &Patch, transport: Transport) {
        self.patch.clone_from(patch);
        self.transport = transport;
        let levels = self.levels();

        self.envelopes_active = patch.has_envelopes();
        self.env_configs = std::array::from_fn(|slot| match patch.envelopes.get(slot) {
            Some(Some(settings)) => AdsrConfig::from_values(
                settings.attack.plain(&levels),
                settings.decay.plain(&levels),
                settings.sustain.plain(&levels),
                settings.release.plain(&levels),
                self.sample_rate,
            ),
            // No envelope node here, and the bypass copy past the last of them.
            _ => AdsrConfig::BYPASS,
        });

        self.mono = patch.globals.mono;
        self.glide_always = patch.globals.glide_always;
        self.glide_seconds = patch.globals.glide.plain(&levels);
        // Each sample closes a fixed share of the gap, so the pitch is within 1% of the target
        // after the glide time.
        self.glide_coef = if self.glide_seconds <= 0.0 {
            1.0
        } else {
            1.0 - (-5.0 / (self.glide_seconds * self.sample_rate)).exp()
        };
    }

    pub fn pitch_bend(&mut self, value: f32) {
        self.midi_levels[MidiSource::PitchBend.index()] = value;
    }

    pub fn mod_wheel(&mut self, value: f32) {
        self.midi_levels[MidiSource::ModWheel.index()] = value;
    }

    pub fn aftertouch(&mut self, value: f32) {
        self.midi_levels[MidiSource::Aftertouch.index()] = value;
    }

    pub fn note_on(&mut self, note: u8, velocity: f32) {
        self.held.push(note, velocity);
        self.midi_levels[MidiSource::Note.index()] = f32::from(note) / 127.0;
        self.midi_levels[MidiSource::Velocity.index()] = velocity;
        let levels = self.levels();
        self.lfos
            .retrigger(&self.patch.lfos, &levels, &self.transport, &mut self.rng);

        // Portamento in mono mode starts the new note at the last pitch, while any voice still
        // sounds. Otherwise the voice starts at its own pitch.
        let target_step = util::midi_note_to_freq(note) / self.sample_rate;
        let sounding = self.lanes.iter().any(Lane::is_sounding);
        let gliding = self.mono
            && self.glide_seconds > 0.0
            && self.has_played
            && (self.glide_always || sounding);
        self.has_played = true;
        let step = if gliding { self.mono_step } else { target_step };
        self.mono_step = target_step;

        for lane in 0..MAX_SLOTS {
            if self.patch.lanes[lane].is_active() {
                self.lane_note_on(lane, note, velocity, step, target_step);
            }
        }
    }

    /// Starts a voice in one slot. In mono mode the slot's sounding voices fade out first.
    fn lane_note_on(&mut self, lane: usize, note: u8, velocity: f32, step: f32, target_step: f32) {
        // In mono mode the sounding voices fade out as the new note starts, so only one stays
        // loud. The fade uses the short release, the same as a note off with no envelope.
        if self.mono {
            for voice in self.lanes[lane].voices_mut() {
                for env in voice.envs.iter_mut() {
                    env.release(FADE_SECONDS, self.sample_rate);
                }
            }
        }

        let slot = self.lanes[lane].voice_slot();
        let levels = self.levels();
        let stages = start_arena(
            &self.patch.lanes[lane],
            &mut StartContext::new(
                &levels,
                &mut self.rng,
                step,
                self.lanes[lane].delays_of(slot),
            ),
        );
        self.age_counter += 1;
        self.lanes[lane].voices[slot] = Some(Voice::new(
            note,
            velocity,
            step,
            target_step,
            self.age_counter,
            stages,
        ));
    }

    pub fn note_off(&mut self, note: u8) {
        // Only the note that is sounding matters for recall. A note that was already fading
        // doesn't bring anything back.
        let sounding = self
            .lanes
            .iter()
            .flat_map(Lane::voices)
            .any(|voice| voice.note == note && !voice.releasing());
        self.held.remove(note);

        // Each envelope copy releases over its own release time. Without any envelope driving the
        // sound, the bypass copy takes the short fade. With one, it lasts as long as the longest
        // release, so the voice ends when the envelopes do.
        let bypass_release = if self.envelopes_active {
            self.env_configs[..MAX_ENVELOPES]
                .iter()
                .map(|config| config.release_seconds)
                .fold(FADE_SECONDS, f32::max)
        } else {
            FADE_SECONDS
        };
        for lane in self.lanes.iter_mut() {
            for voice in lane.voices_mut() {
                if voice.note == note && !voice.releasing() {
                    for (index, env) in voice.envs.iter_mut().enumerate() {
                        let seconds = if index == MAX_ENVELOPES {
                            bypass_release
                        } else {
                            self.env_configs[index].release_seconds
                        };
                        env.release(seconds, self.sample_rate);
                    }
                }
            }
        }

        // Mono recall: if the sounding note was let go and another key is still down, play that
        // key again, starting from its most recent press.
        if self.mono && sounding {
            if let Some((next, velocity)) = self.held.newest() {
                self.note_on(next, velocity);
            }
        }
    }

    /// One stereo frame of the whole synth. `sample_id` is the sample's place in the block, which
    /// a synced LFO needs to find the playhead.
    pub fn process_sample(&mut self, sample_id: usize) -> Frame {
        let levels = self.levels();

        // A gain is exactly silent at its minimum. The master gain eases toward its knob.
        let master_target = db_to_linear(self.patch.globals.gain.plain(&levels));
        if self.master_settled {
            self.master_gain += (master_target - self.master_gain) * self.smooth;
        } else {
            self.master_gain = master_target;
            self.master_settled = true;
        }

        // Each active slot sums its voices. The newest voice's level of each envelope is what
        // knobs that no single voice owns read for the next sample.
        let mut newest_age = [0u64; MAX_ENVELOPES];
        let mut newest_level = [0.0f32; MAX_ENVELOPES];
        let mut out = Frame::SILENT;
        for (lane, graph) in self.lanes.iter_mut().zip(&self.patch.lanes) {
            if !graph.is_active() {
                lane.go_quiet();
                continue;
            }
            let mut env = RenderEnv {
                tables: &self.tables,
                lfo_levels: levels.lfo,
                midi_levels: levels.midi,
                macro_levels: levels.macros,
                envelopes_active: self.envelopes_active,
                sample_rate: self.sample_rate,
                smooth: self.smooth,
                rng: &mut self.rng,
            };
            let mut lane_out = Frame::SILENT;
            lane.taps = [Frame::SILENT; MAX_STAGES];
            for (index, slot) in lane.voices.iter_mut().enumerate() {
                let Some(voice) = slot else { continue };
                for index in 0..MAX_ENVELOPES {
                    if voice.age > newest_age[index] {
                        newest_age[index] = voice.age;
                        newest_level[index] = voice.envs[index].level;
                    }
                }
                voice.glide(self.glide_coef);
                let delays =
                    &mut lane.delays[index * MAX_DELAY_LINES..(index + 1) * MAX_DELAY_LINES];
                lane_out += voice.render(graph, &mut env, delays, &mut lane.taps);
                voice.advance(&self.env_configs);
                if voice.finished() {
                    *slot = None;
                }
            }
            // The voices are scaled before the effects, so an effect sees the level the output
            // would have without it. The slot's gain comes after, like a fader.
            lane_out = lane_out * VOICE_MIX;
            lane.taps.iter_mut().for_each(|tap| *tap = *tap * VOICE_MIX);
            if let Some(effected) =
                self.effects
                    .run(graph, &lane.taps, &levels, self.transport.tempo)
            {
                lane_out = effected;
            }
            let gain = lane.ease_gain(db_to_linear(graph.gain.plain(&levels)), self.smooth);
            out += lane_out * gain;
        }
        self.env_levels = newest_level;

        self.lfos.advance(
            &self.patch.lfos,
            &levels,
            &self.transport,
            sample_id,
            self.sample_rate,
            &mut self.rng,
        );

        let result = if self.patch.is_audible() {
            out * self.master_gain
        } else {
            Frame::SILENT
        };
        self.output_peak[0] = self.output_peak[0].max(result.l.abs());
        self.output_peak[1] = self.output_peak[1].max(result.r.abs());
        result
    }

    /// The loudest left and right sample since the last call, as linear amplitude. The meter
    /// reads it once per block.
    pub fn take_output_peak(&mut self) -> [f32; 2] {
        std::mem::take(&mut self.output_peak)
    }
}
