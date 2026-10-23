//! The synth engine: what runs on the audio thread. It knows nothing about the plugin API, so a
//! test can play notes into it and read samples out. The plugin feeds it events, and it runs the
//! stages of the current patch for every sounding voice.

mod arpeggiator;
mod chord;
#[cfg(test)]
mod chord_tests;
mod effects;
pub use effects::POOL;
mod lane;
mod lfo;
#[cfg(test)]
mod no_alloc;
mod notes;
#[cfg(test)]
mod regression_tests;
pub mod stage;
#[cfg(test)]
mod tests;
mod voice;

use crate::dsp::envelope::{AdsrConfig, EnvPhase, FADE_SECONDS};
use crate::dsp::frame::Frame;
use crate::dsp::gain::db_to_linear;
use crate::dsp::rng::Rng;
use crate::dsp::wavetable::WaveTables;
use crate::nodes::input::{MidiSource, MIDI_SOURCE_COUNT};
use crate::nodes::macros::MACRO_COUNT;
use crate::nodes::output::MAX_SLOTS;
use crate::patch::control::{
    ControlLevels, ENV_SLOTS, MAX_ENVELOPES, MAX_NOTE_SOURCES, MAX_SAMPLERS,
};
use crate::patch::stage::{StageParams, MAX_STAGES};
use crate::patch::{NoteSourceSpec, Patch};
use arpeggiator::Arpeggiators;
use chord::Chords;
use effects::Effects;
use lane::Lane;
use lfo::Lfos;
use nih_plug::util;
use notes::{HeldNotes, NoteBuffer};
use stage::{start_arena, StageState, StartContext, MAX_DELAY_LINES};
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
    /// The stage of each envelope's newest voice, or None with no voice. The editor shows it.
    env_phases: [Option<EnvPhase>; MAX_ENVELOPES],
    /// Where each Sampler's newest active voice is in its file, or None while none plays it. The
    /// editor's viewer shows it as the playhead.
    sampler_positions: [Option<f32>; MAX_SAMPLERS],
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
    /// Each note-source slot's own copy of the notes held at it. A slot fed straight from a MIDI
    /// input keeps every host note; an arpeggiator reads its input slot's copy here to build its
    /// chord. Separate from `held` above, which is the one legacy, engine-wide chord that mono
    /// recall and a hand-built patch with no note-routing table still use.
    note_held: [HeldNotes; MAX_NOTE_SOURCES],
    arpeggiators: Arpeggiators,
    chords: Chords,
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
            env_phases: [None; MAX_ENVELOPES],
            sampler_positions: [None; MAX_SAMPLERS],
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
            note_held: std::array::from_fn(|_| HeldNotes::new()),
            arpeggiators: Arpeggiators::new(),
            chords: Chords::new(),
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
        self.note_held.iter_mut().for_each(HeldNotes::clear);
        self.arpeggiators = Arpeggiators::new();
        self.chords = Chords::new();
        self.lanes.iter_mut().for_each(Lane::clear);
        self.effects.reset();
    }

    /// Every control source's level right now. Knobs read their route from these.
    pub fn levels(&self) -> ControlLevels {
        let mut levels = ControlLevels {
            envelope: self.env_levels,
            lfo: self.lfos.levels(),
            midi: self.midi_levels,
            macros: self.macro_levels,
            envelope_phase: self.env_phases,
            lfo_phase: self.lfos.phases(),
            sampler_position: self.sampler_positions,
            ..ControlLevels::default()
        };
        // The map and math nodes read the sources, so they come last.
        self.patch.signals.evaluate(&mut levels);
        levels
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

    /// A host note on (or the key strip's, in standalone). It reaches every MIDI input slot, since
    /// there is one physical note source behind all of them, and every lane that plays straight
    /// from one of those slots, or has no note-routing slot of its own at all because it was built
    /// by hand rather than compiled by `Wiring`.
    pub fn note_on(&mut self, note: u8, velocity: f32) {
        self.held.push(note, velocity);
        for slot in 0..MAX_NOTE_SOURCES {
            if matches!(self.patch.note_sources[slot], Some(NoteSourceSpec::Input)) {
                self.note_held[slot].push(note, velocity);
            }
        }
        self.midi_levels[MidiSource::Note.index()] = f32::from(note) / 127.0;
        self.midi_levels[MidiSource::Velocity.index()] = velocity;
        let levels = self.levels();
        self.lfos
            .retrigger(&self.patch.lfos, &levels, &self.transport, &mut self.rng);
        self.dispatch_note_on(None, note, velocity);
    }

    /// True if lane `lane` should sound for a note dispatched to `want`. `None` is a host event,
    /// which every lane with no note-routing slot of its own, or a slot fed straight from a MIDI
    /// input, takes directly. `Some(slot)` is an arpeggiator's own generated note, which only the
    /// lanes reading that exact slot take.
    fn wants_slot(&self, lane: usize, want: Option<usize>) -> bool {
        match want {
            None => match self.patch.lanes[lane].notes_slot {
                None => true,
                Some(slot) => matches!(self.patch.note_sources[slot], Some(NoteSourceSpec::Input)),
            },
            Some(target) => self.patch.lanes[lane].notes_slot == Some(target),
        }
    }

    /// Starts `note` in every active lane that wants a note dispatched to `want`. Shared by the
    /// host's `note_on` and by an arpeggiator's own generated note ons, which differ only in which
    /// lanes they reach; mono, glide and portamento are engine-wide either way, the same
    /// simplification `Wiring::globals()` already makes for mono and glide themselves.
    fn dispatch_note_on(&mut self, want: Option<usize>, note: u8, velocity: f32) {
        // Portamento in mono mode starts the new note at the last pitch, while any voice still
        // sounds. Otherwise the voice starts at its own pitch.
        let target_step = util::midi_note_to_freq(note) / self.sample_rate;
        let sounding = (0..MAX_SLOTS)
            .filter(|&lane| self.wants_slot(lane, want))
            .any(|lane| self.lanes[lane].is_sounding());
        let gliding = self.mono
            && self.glide_seconds > 0.0
            && self.has_played
            && (self.glide_always || sounding);
        self.has_played = true;
        let step = if gliding { self.mono_step } else { target_step };
        self.mono_step = target_step;

        for lane in 0..MAX_SLOTS {
            if self.patch.lanes[lane].is_active() && self.wants_slot(lane, want) {
                self.lane_note_on(lane, note, velocity, step, target_step);
            }
        }
    }

    /// Starts a voice in one slot. In mono mode the slot's sounding voices fade out first.
    fn lane_note_on(&mut self, lane: usize, note: u8, velocity: f32, step: f32, target_step: f32) {
        // In mono mode the sounding voices fade out as the new note starts, so only one stays
        // loud. The fade uses the short release, the same as a note off with no envelope. A voice
        // that is already in a long release tail is cut to the same fade, so no tail rings on
        // under the new note.
        if self.mono {
            for voice in self.lanes[lane].voices_mut() {
                for env in voice.envs.iter_mut() {
                    env.cut(FADE_SECONDS, self.sample_rate);
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

    /// A host note off. Like `note_on`, it reaches every MIDI input slot and every lane that plays
    /// straight from one, or has no note-routing slot at all.
    pub fn note_off(&mut self, note: u8) {
        // Only the note that is sounding matters for recall. A note that was already fading
        // doesn't bring anything back.
        let sounding = self
            .lanes
            .iter()
            .flat_map(Lane::voices)
            .any(|voice| voice.note == note && !voice.releasing());
        self.held.remove(note);
        for slot in 0..MAX_NOTE_SOURCES {
            if matches!(self.patch.note_sources[slot], Some(NoteSourceSpec::Input)) {
                self.note_held[slot].remove(note);
            }
        }
        self.dispatch_note_off(None, note);

        // Mono recall: if the sounding note was let go and another key is still down, play that
        // key again, starting from its most recent press. This reads the engine-wide `held`, the
        // same one `Wiring::globals()` already collapses mono and glide to the first MIDI input
        // for, so it is unaffected by notes arriving at more than one input slot.
        if self.mono && sounding {
            if let Some((next, velocity)) = self.held.newest() {
                self.note_on(next, velocity);
            }
        }
    }

    /// Releases `note` in every active lane that wants a note dispatched to `want`. See
    /// `dispatch_note_on` for what `want` means.
    fn dispatch_note_off(&mut self, want: Option<usize>, note: u8) {
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
        for lane in 0..MAX_SLOTS {
            if !self.wants_slot(lane, want) {
                continue;
            }
            for voice in self.lanes[lane].voices_mut() {
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
    }

    /// Moves every arpeggiator on by this sample, and dispatches whatever notes it turned on or off
    /// to the lanes that read its output slot. Called once per sample, before the voices render, so
    /// a note an arpeggiator starts this sample is already sounding in time for it.
    fn advance_arpeggiators(&mut self, levels: &ControlLevels) {
        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        self.arpeggiators.advance(
            &self.patch.note_sources,
            &self.note_held,
            levels,
            &self.transport,
            self.sample_rate,
            &mut self.rng,
            &mut events,
        );
        for (slot, buffer) in events.iter().enumerate() {
            self.dispatch_generated(slot, buffer);
        }
    }

    /// Dispatches the note events a note source generated this sample for its own slot to the
    /// lanes that read it, and records them in `note_held` too. Without this, a second note
    /// source reading this slot's chord, an arpeggiator stacked on a chord or a chord stacked on
    /// an arpeggiator, would see nothing there, since `note_held` otherwise only ever fills from
    /// the host's own note on and off.
    fn dispatch_generated(&mut self, slot: usize, buffer: &NoteBuffer) {
        for event in buffer.as_slice() {
            if event.on {
                self.note_held[slot].push(event.note, event.velocity);
                self.dispatch_note_on(Some(slot), event.note, event.velocity);
            } else {
                self.note_held[slot].remove(event.note);
                self.dispatch_note_off(Some(slot), event.note);
            }
        }
    }

    /// Moves every chord on by this sample, and dispatches whatever notes it turned on or off to
    /// the lanes that read its output slot. A chord has no clock: it reacts to a note arriving or
    /// letting go at its input, so this only ever has work to do on the sample a host note or an
    /// upstream note source's own event changes what is held there.
    fn advance_chords(&mut self, levels: &ControlLevels) {
        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        self.chords.advance(
            &self.patch.note_sources,
            &self.note_held,
            levels,
            &mut events,
        );
        for (slot, buffer) in events.iter().enumerate() {
            self.dispatch_generated(slot, buffer);
        }
    }

    /// One stereo frame of the whole synth. `sample_id` is the sample's place in the block, which
    /// a synced LFO needs to find the playhead.
    pub fn process_sample(&mut self, sample_id: usize) -> Frame {
        let levels = self.levels();
        self.advance_arpeggiators(&levels);
        self.advance_chords(&levels);

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
        let mut newest_phase = [None; MAX_ENVELOPES];
        // Unlike the envelope copies above, which every voice carries one of per patch, a Sampler
        // stage only exists in the voices of the one lane it was placed in. So the newest-active
        // voice per slot is tracked across lanes the same way, but only a Sampler's own lane ever
        // updates its slot.
        let mut newest_sampler_age = [0u64; MAX_SAMPLERS];
        let mut newest_sampler_position: [Option<f32>; MAX_SAMPLERS] = [None; MAX_SAMPLERS];
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
                signals: &self.patch.signals,
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
                        newest_phase[index] = Some(voice.envs[index].phase);
                    }
                }
                for (stage_index, spec) in graph.stages.iter().enumerate() {
                    let StageParams::Sampler(p) = &spec.params else {
                        continue;
                    };
                    let Some(slot) = p.sampler_slot else { continue };
                    let StageState::Sampler(state) = &voice.stages[stage_index] else {
                        continue;
                    };
                    let len = p.data.as_ref().map_or(0.0, |d| d.len() as f32);
                    if let Some(fraction) = state.fraction(len) {
                        if voice.age > newest_sampler_age[slot] {
                            newest_sampler_age[slot] = voice.age;
                            newest_sampler_position[slot] = Some(fraction);
                        }
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
            if let Some(effected) = self.effects.run(
                graph,
                &lane.taps,
                &levels,
                self.transport.tempo,
                self.smooth,
            ) {
                lane_out = effected;
            }
            let gain = lane.ease_gain(db_to_linear(graph.gain.plain(&levels)), self.smooth);
            out += lane_out * gain;
        }
        self.env_levels = newest_level;
        self.env_phases = newest_phase;
        self.sampler_positions = newest_sampler_position;

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

    /// The spectrum each EQ instance last measured, for the editor's analyzer display.
    pub fn spectra(&self) -> [[f32; crate::dsp::analyzer::BINS]; effects::POOL] {
        self.effects.spectra()
    }

    /// What each monitor instance last measured, for the editor's monitor displays.
    pub fn monitors(&self) -> [crate::dsp::monitor::MonitorData; effects::POOL] {
        self.effects.monitors()
    }

    /// The loudest left and right sample since the last call, as linear amplitude. The meter
    /// reads it once per block.
    pub fn take_output_peak(&mut self) -> [f32; 2] {
        std::mem::take(&mut self.output_peak)
    }
}
