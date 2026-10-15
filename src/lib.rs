// The nih_export_vst3! macro (via vst3-com) expands with a trailing semicolon that
// newer rustc flags as a future-incompat lint. It's upstream in nih-plug, not our code.
#![allow(unknown_lints, semicolon_in_expressions_from_non_local_macros)]

use nih_plug::prelude::*;
use nih_plug_iced::IcedState;
use rtrb::{Consumer, Producer, RingBuffer};
use std::f32::consts::PI;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

mod dsp;
mod editor;
mod knob;
mod node_editor;
mod nodes;
mod piano;
mod routing;

use dsp::noise::{NoiseState, NoiseType};
use dsp::rng::Rng;
use dsp::warp::{warp_phase, warp_value, WarpMode};
use dsp::wavetable::{WaveTables, Waveform};
use nodes::filter::FilterType;
use nodes::input::{MidiSource, MIDI_SOURCE_COUNT};
use nodes::knobs::{KnobId, GAIN_MIN_DB, MAX_UNISON};
use nodes::lfo;
use nodes::oscillator::transpose_ratio;
use nodes::output::{slot_knob, MAX_SLOTS};
use piano::PianoEvent;
use routing::{
    ControlLevels, EnvelopeSettings, KnobValue, LaneChain, LfoSettings, ENV_SLOTS, MAX_ENVELOPES,
    MAX_LFOS,
};

/// Set by the standalone binary before it starts. Only standalone routes the computer keyboard to
/// the synth.
pub static STANDALONE: AtomicBool = AtomicBool::new(false);

/// Maximum number of notes that can sound at once in one output slot.
const NUM_VOICES: usize = 16;
/// Scale applied to the summed voices so chords don't clip (1 / sqrt(NUM_VOICES)).
const VOICE_MIX: f32 = 0.25;
/// Room for queued computer-keyboard notes. A full queue drops new notes.
const NOTE_QUEUE_LEN: usize = 256;
/// The release used when no envelope node is in the chain. It avoids clicks on note off.
const RELEASE_SECONDS: f32 = 0.01;
/// How long gains, the cutoff, and the resonance take to settle after a knob moves. Knob values
/// change once per block, so without this they would step and click.
const SMOOTH_SECONDS: f32 = 0.01;

/// Where a voice is in its amplitude envelope.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Attack,
    Decay,
    Sustain,
    Release,
}

/// The oscillator state for one of a voice's sources: its own source, or one branch of a mix.
/// Index 0 is the lane's source when there is no mix, or branch A's when there is. Index 1 is
/// branch B's, unused without a mix.
#[derive(Clone, Copy)]
struct OscVoice {
    /// This oscillator's noise, when its waveform is noise.
    noise: NoiseState,
    /// One phase per unison sub-oscillator. Only the first `unison_count` are used.
    phases: [f32; MAX_UNISON],
    /// Each sub-oscillator's detune, as a frequency ratio from the base pitch.
    detune_ratios: [f32; MAX_UNISON],
}

#[derive(Clone, Copy)]
struct Voice {
    note: u8,
    oscillators: [OscVoice; 2],
    /// The pitch the voice is moving toward, and the pitch it is at, as phase steps per sample.
    target_delta: f32,
    base_delta: f32,
    velocity: f32,
    /// One copy of each envelope node, then the bypass copy last.
    envs: [Adsr; ENV_SLOTS],
    /// Monotonic stamp of when this voice started, used to steal the oldest voice.
    age: u64,
}

/// One envelope generator: an attack, decay, sustain, and release, run per voice.
#[derive(Clone, Copy)]
struct Adsr {
    stage: Stage,
    /// The current level, from 0.0 to 1.0.
    level: f32,
    /// Level lost per sample while releasing.
    release_step: f32,
}

impl Adsr {
    const START: Adsr = Adsr {
        stage: Stage::Attack,
        level: 0.0,
        release_step: 0.0,
    };

    /// Moves the envelope on by one sample.
    fn advance(&mut self, env: &EnvConfig) {
        match self.stage {
            Stage::Attack => {
                self.level += env.attack_step;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level -= env.decay_step;
                if self.level <= env.sustain {
                    self.level = env.sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => self.level = env.sustain,
            // A release stops at zero. Otherwise a copy that finished early keeps falling below zero
            // while the voice waits on a longer envelope, and a negative amplitude flips the sound.
            Stage::Release => self.level = (self.level - self.release_step).max(0.0),
        }
    }

    /// Starts the release from the current level, falling to zero over `seconds`.
    fn release(&mut self, seconds: f32, sample_rate: f32) {
        if self.stage != Stage::Release {
            self.stage = Stage::Release;
            let samples = (seconds * sample_rate).max(1.0);
            self.release_step = self.level / samples;
        }
    }

    fn finished(&self) -> bool {
        self.stage == Stage::Release && self.level <= 0.0
    }
}

impl Voice {
    /// Moves every envelope copy on by one sample, each with its own times.
    fn advance_all(&mut self, configs: &[EnvConfig; ENV_SLOTS]) {
        for (env, config) in self.envs.iter_mut().zip(configs) {
            env.advance(config);
        }
    }

    /// This voice's level for each envelope node.
    fn env_levels(&self) -> [f32; MAX_ENVELOPES] {
        std::array::from_fn(|i| self.envs[i].level)
    }

    /// True once every envelope has finished its release.
    fn releasing(&self) -> bool {
        self.envs.iter().all(|env| env.stage == Stage::Release)
    }

    /// True once every envelope copy has run out. A voice lives until its longest release ends,
    /// so a driver envelope's release is heard even when the amplitude is the short bypass fade.
    fn finished(&self) -> bool {
        self.envs.iter().all(|env| env.finished())
    }

    /// Moves the pitch a fraction `coef` of the way to the target. A coef of 1.0 jumps there.
    fn glide(&mut self, coef: f32) {
        self.base_delta += (self.target_delta - self.base_delta) * coef;
    }

    /// One sample from oscillator slot `which`: 0 for the lane's own source or mix branch A, 1
    /// for branch B. Noise is one unpitched stream, and any other waveform is its unison
    /// sub-oscillators.
    fn branch_sample(
        &mut self,
        which: usize,
        tables: &WaveTables,
        waveform: Waveform,
        noise: NoiseType,
        warp: WarpMode,
        warp_amount: f32,
        count: usize,
        transpose: f32,
        rng: &mut Rng,
    ) -> f32 {
        if waveform == Waveform::Noise {
            self.oscillators[which].noise.sample(noise, rng)
        } else {
            let base_delta = self.base_delta;
            let osc = &mut self.oscillators[which];
            let mut sum = 0.0;
            for i in 0..count {
                // The phase step, in cycles per sample. The waveform band-limits its edges by it.
                let step = base_delta * osc.detune_ratios[i] * transpose;
                let phase = warp_phase(warp, osc.phases[i], warp_amount);
                sum += warp_value(warp, tables.sample(waveform, phase, step), warp_amount);
                osc.phases[i] += step;
                if osc.phases[i] >= 1.0 {
                    osc.phases[i] -= 1.0;
                }
            }
            sum
        }
    }

    /// One source's full contribution to a lane: its sample, scaled by its own unison
    /// normalization, this voice's velocity, its amplitude envelope, and its own gain knob.
    #[allow(clippy::too_many_arguments)]
    fn branch_contribution(
        &mut self,
        which: usize,
        tables: &WaveTables,
        waveform: Waveform,
        noise: NoiseType,
        warp: WarpMode,
        knobs: &[KnobValue; KnobId::COUNT],
        prep: &BranchPrep,
        voice_levels: &ControlLevels,
        rng: &mut Rng,
    ) -> f32 {
        let osc_gain =
            db_to_linear(knobs[KnobId::OscGain as usize].plain(KnobId::OscGain, voice_levels));
        let warp_amount =
            knobs[KnobId::WarpAmount as usize].plain(KnobId::WarpAmount, voice_levels);
        let amplitude = if prep.bypass_only {
            1.0
        } else {
            self.envs[prep.amp_slot].level
        };
        self.branch_sample(
            which,
            tables,
            waveform,
            noise,
            warp,
            warp_amount,
            prep.unison_count,
            prep.transpose,
            rng,
        ) * prep.source_norm
            * self.velocity
            * amplitude
            * osc_gain
    }
}

/// A source's lane-rate settings: the ones that stay fixed across a block instead of following a
/// per-voice control level. Shared by the lane's own oscillator and by each of a mix's branches.
struct BranchPrep {
    /// Unison voices add up, so the sum is scaled down. Noise is one stream and needs no scale.
    source_norm: f32,
    /// The envelope copy that shapes this source's amplitude, or the bypass copy's index.
    amp_slot: usize,
    /// True when this source has no envelope of its own, so the bypass fade is its only shaping.
    bypass_only: bool,
    transpose: f32,
    unison_count: usize,
}

impl BranchPrep {
    fn new(
        waveform: Waveform,
        envelope: Option<usize>,
        knobs: &[KnobValue; KnobId::COUNT],
        unison_count: usize,
        envelopes_active: bool,
        levels: &ControlLevels,
    ) -> Self {
        let unison_norm = 1.0 / (unison_count as f32).sqrt();
        let source_norm = if waveform == Waveform::Noise {
            1.0
        } else {
            unison_norm
        };
        let transpose = transpose_ratio(
            knobs[KnobId::Octave as usize].plain(KnobId::Octave, levels),
            knobs[KnobId::Coarse as usize].plain(KnobId::Coarse, levels),
            knobs[KnobId::Fine as usize].plain(KnobId::Fine, levels),
        );
        Self {
            source_norm,
            amp_slot: envelope.unwrap_or(MAX_ENVELOPES),
            bypass_only: envelope.is_none() && envelopes_active,
            transpose,
            unison_count,
        }
    }
}

/// One output slot's voices, filter state, and smoothed levels. Every slot plays the notes through
/// its own chain.
#[derive(Clone, Copy)]
struct Lane {
    voices: [Option<Voice>; NUM_VOICES],
    /// State of this slot's filter, a state-variable filter with two integrators.
    filter_ic1: f32,
    filter_ic2: f32,
    /// The slot's gain, the filter cutoff as log2 of Hz, and the resonance, each easing toward
    /// the value the knobs ask for.
    gain: f32,
    cutoff_log2: f32,
    resonance: f32,
    /// False until the smoothed levels have a first value. They then start at the knob values
    /// instead of easing in from zero.
    settled: bool,
}

impl Lane {
    const EMPTY: Lane = Lane {
        voices: [None; NUM_VOICES],
        filter_ic1: 0.0,
        filter_ic2: 0.0,
        gain: 0.0,
        cutoff_log2: 0.0,
        resonance: 0.0,
        settled: false,
    };
}

/// A slot's settings for the current block, read from the knobs of the nodes in its chain.
#[derive(Clone, Copy)]
struct LaneSettings {
    /// Number of unison sub-oscillators per voice, and their total detune spread.
    unison_count: usize,
    detune_cents: f32,
    /// How far the starting phases move from an even spread toward random, from 0.0 to 1.0.
    phase_randomness: f32,
}

impl LaneSettings {
    const DEFAULT: LaneSettings = LaneSettings {
        unison_count: 1,
        detune_cents: 0.0,
        phase_randomness: 0.0,
    };

    /// Reads one source's settings from its own knob table: the lane's, or a mix branch's.
    fn from_knobs(knobs: &[KnobValue; KnobId::COUNT], levels: &ControlLevels) -> Self {
        let plain = |id: KnobId| knobs[id as usize].plain(id, levels);
        Self {
            unison_count: (plain(KnobId::Unison).round() as usize).clamp(1, MAX_UNISON),
            detune_cents: plain(KnobId::Detune),
            phase_randomness: plain(KnobId::PhaseRandom),
        }
    }
}

/// Per-sample envelope steps, recomputed once per block from the ADSR parameters.
#[derive(Clone, Copy)]
struct EnvConfig {
    attack_step: f32,
    decay_step: f32,
    sustain: f32,
    release_seconds: f32,
}

impl EnvConfig {
    /// No envelope in the chain: full level while held, with the short fade on release.
    const BYPASS: EnvConfig = EnvConfig {
        attack_step: 1.0,
        decay_step: 0.0,
        sustain: 1.0,
        release_seconds: RELEASE_SECONDS,
    };

    fn from_values(attack: f32, decay: f32, sustain: f32, release: f32, sample_rate: f32) -> Self {
        let attack_samples = (attack * sample_rate).max(1.0);
        let decay_samples = (decay * sample_rate).max(1.0);
        Self {
            attack_step: 1.0 / attack_samples,
            decay_step: (1.0 - sustain) / decay_samples,
            sustain,
            release_seconds: release,
        }
    }
}

pub struct Flode {
    params: Arc<FlodeParams>,
    sample_rate: f32,
    /// One lane per output slot, each with its own voices and filter.
    lanes: [Lane; MAX_SLOTS],
    /// The chain of each slot, read from the routing at the start of each block.
    lane_chains: [LaneChain; MAX_SLOTS],
    /// Each LFO's settings for the block, and its phase and current level.
    lfo_settings: [LfoSettings; MAX_LFOS],
    lfo_phase: [f32; MAX_LFOS],
    lfo_levels: [f32; MAX_LFOS],
    /// The random value a noise LFO holds, and the phase it last saw, to spot each new cycle.
    lfo_hold: [f32; MAX_LFOS],
    lfo_prev: [f32; MAX_LFOS],
    /// The MIDI sources' latest values, from 0.0 to 1.0.
    midi_levels: [f32; MIDI_SOURCE_COUNT],
    age_counter: u64,
    /// The settings of each slot for the current block: index 0 is the lane's own oscillator, or
    /// mix branch A; index 1 is branch B, unused without a mix.
    lane_settings: [[LaneSettings; 2]; MAX_SLOTS],
    /// The master gain, easing toward the knob. It applies after the slots are mixed.
    master_gain: f32,
    master_settled: bool,
    /// The share of the gap a smoothed value closes each sample.
    smooth_coef: f32,
    /// True while the output is mono, so one voice sounds at a time.
    mono: bool,
    /// Portamento: the glide time from the knob, the per-sample step toward the target pitch,
    /// and the pitch of the last note played, which the next mono note glides from. Glide runs on
    /// every mono note when `glide_always` is set, otherwise only while another note is held.
    glide_seconds: f32,
    glide_always: bool,
    /// False until a note has played. The first note has no pitch to glide from.
    has_played: bool,
    glide_coef: f32,
    mono_delta: f32,
    /// Notes currently held, oldest first, with their velocity. Mono mode recalls from the end.
    held: [(u8, f32); 128],
    held_len: usize,
    /// State of the random number generator for starting phases.
    rng: Rng,
    /// Each envelope node's times for the block, with the bypass copy last.
    env_configs: [EnvConfig; ENV_SLOTS],
    /// The level of each envelope's newest voice. Lane-wide knobs read these.
    env_levels: [f32; MAX_ENVELOPES],
    /// True while any envelope node is in the patch. Then the amplitude is the envelopes' job.
    envelopes_active: bool,
    /// Notes from the editor's computer keyboard. Drained on the audio thread in `process`.
    note_tx: Arc<Mutex<Producer<PianoEvent>>>,
    note_rx: Consumer<PianoEvent>,
    /// Band-limited waveforms for every pitch range. Built once, read on the audio thread.
    tables: Arc<WaveTables>,
    /// Set by the editor. Says whether the output is connected and which chain to play.
    routing: Arc<routing::AudioRouting>,
}

impl Flode {
    /// Every control source's level right now. Knobs read their route from these.
    fn control_levels(&self) -> ControlLevels {
        ControlLevels {
            envelope: self.env_levels,
            lfo: self.lfo_levels,
            midi: self.midi_levels,
        }
    }

    fn note_on(&mut self, note: u8, velocity: f32) {
        self.held_push(note, velocity);
        self.midi_levels[MidiSource::Note.index()] = f32::from(note) / 127.0;
        self.midi_levels[MidiSource::Velocity.index()] = velocity;

        // Portamento in mono mode starts the new note at the last pitch, while any voice still
        // sounds. Otherwise the voice starts at its own pitch.
        let target_delta = util::midi_note_to_freq(note) / self.sample_rate;
        let sounding = self
            .lanes
            .iter()
            .any(|lane| lane.voices.iter().any(|v| v.is_some()));
        let gliding = self.mono
            && self.glide_seconds > 0.0
            && self.has_played
            && (self.glide_always || sounding);
        self.has_played = true;
        let base_delta = if gliding {
            self.mono_delta
        } else {
            target_delta
        };
        self.mono_delta = target_delta;

        for lane in 0..MAX_SLOTS {
            if self.lane_chains[lane].active {
                self.lane_note_on(lane, note, velocity, base_delta, target_delta);
            }
        }
    }

    /// Starts a voice in one slot. In mono mode the slot's sounding voices fade out first.
    fn lane_note_on(
        &mut self,
        lane: usize,
        note: u8,
        velocity: f32,
        base_delta: f32,
        target_delta: f32,
    ) {
        // In mono mode the sounding voices fade out as the new note starts, so only one stays
        // loud. The fade uses the short release, the same as a note off with no envelope.
        if self.mono {
            for voice in self.lanes[lane].voices.iter_mut().flatten() {
                for env in voice.envs.iter_mut() {
                    env.release(RELEASE_SECONDS, self.sample_rate);
                }
            }
        }

        // Prefer a free slot, then a voice already fading out, then the oldest voice.
        let index = {
            let voices = &self.lanes[lane].voices;
            voices
                .iter()
                .position(|v| v.is_none())
                .or_else(|| {
                    voices
                        .iter()
                        .position(|v| v.map_or(false, |v| v.releasing()))
                })
                .unwrap_or_else(|| {
                    voices
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, v)| v.map_or(u64::MAX, |v| v.age))
                        .map(|(i, _)| i)
                        .unwrap_or(0)
                })
        };

        // Spread each oscillator's sub-oscillators from the lowest detune to the highest. Starting
        // the phases apart keeps them from cancelling. Slot 0 is the lane's own source, or mix
        // branch A; slot 1 is branch B, set up the same way from its own settings.
        let branch_settings = self.lane_settings[lane];
        let oscillators = std::array::from_fn(|which| {
            let settings = branch_settings[which];
            let count = settings.unison_count;
            let mut phases = [0.0; MAX_UNISON];
            let mut detune_ratios = [1.0; MAX_UNISON];
            for i in 0..count {
                let spread = if count > 1 {
                    2.0 * i as f32 / (count - 1) as f32 - 1.0
                } else {
                    0.0
                };
                let even = i as f32 / count as f32;
                let random = self.rng.unit();
                phases[i] = even + settings.phase_randomness * (random - even);
                detune_ratios[i] = 2.0f32.powf(spread * settings.detune_cents / 1200.0);
            }
            OscVoice {
                noise: NoiseState::default(),
                phases,
                detune_ratios,
            }
        });

        self.age_counter += 1;
        self.lanes[lane].voices[index] = Some(Voice {
            note,
            oscillators,
            target_delta,
            base_delta,
            velocity,
            envs: [Adsr::START; ENV_SLOTS],
            age: self.age_counter,
        });
    }

    /// Adds a note to the end of the held list. A repeat moves it to the end.
    fn held_push(&mut self, note: u8, velocity: f32) {
        self.held_remove(note);
        self.held[self.held_len] = (note, velocity);
        self.held_len += 1;
    }

    fn held_remove(&mut self, note: u8) {
        if let Some(index) = self.held[..self.held_len]
            .iter()
            .position(|&(held, _)| held == note)
        {
            self.held.copy_within(index + 1..self.held_len, index);
            self.held_len -= 1;
        }
    }

    fn note_off(&mut self, note: u8) {
        // Only the note that is sounding matters for recall. A note that was already fading
        // doesn't bring anything back.
        let sounding = self
            .lanes
            .iter()
            .flat_map(|lane| lane.voices.iter())
            .flatten()
            .any(|v| v.note == note && !v.releasing());
        self.held_remove(note);

        // Each envelope copy releases over its own release time. Without any envelope driving the
        // sound, the bypass copy takes the short fade. With one, it lasts as long as the longest
        // release, so the voice ends when the envelopes do.
        let bypass_release = if self.envelopes_active {
            self.env_configs[..MAX_ENVELOPES]
                .iter()
                .map(|config| config.release_seconds)
                .fold(RELEASE_SECONDS, f32::max)
        } else {
            RELEASE_SECONDS
        };
        for lane in self.lanes.iter_mut() {
            for voice in lane.voices.iter_mut().flatten() {
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
        if self.mono && sounding && self.held_len > 0 {
            let (next, velocity) = self.held[self.held_len - 1];
            self.note_on(next, velocity);
        }
    }

    /// One sample of the state-variable low-pass filter. Returns the low-pass output.
    fn filter_sample(
        &mut self,
        lane: usize,
        input: f32,
        cutoff: f32,
        resonance: f32,
        filter_type: FilterType,
    ) -> f32 {
        let cutoff = cutoff.clamp(20.0, self.sample_rate * 0.49);
        let g = (PI * cutoff / self.sample_rate).tan();
        // Damping: 2.0 is no resonance, and the floor keeps the filter stable.
        let k = 2.0 - 1.9 * resonance.clamp(0.0, 1.0);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        let v3 = input - self.lanes[lane].filter_ic2;
        let v1 = a1 * self.lanes[lane].filter_ic1 + a2 * v3;
        let v2 = self.lanes[lane].filter_ic2 + a2 * self.lanes[lane].filter_ic1 + a3 * v3;
        self.lanes[lane].filter_ic1 = 2.0 * v1 - self.lanes[lane].filter_ic1;
        self.lanes[lane].filter_ic2 = 2.0 * v2 - self.lanes[lane].filter_ic2;

        match filter_type {
            FilterType::LowPass => v2,
            // The high pass is what the bass and band pass leave out. It is zero at DC.
            FilterType::HighPass => input - k * v1 - v2,
            // Scaling by k keeps the peak near 1 at every resonance.
            FilterType::BandPass => k * v1,
        }
    }
}

impl Default for Flode {
    fn default() -> Self {
        let (note_tx, note_rx) = RingBuffer::new(NOTE_QUEUE_LEN);
        Self {
            params: Arc::new(FlodeParams::default()),
            sample_rate: 44100.0,
            lanes: [Lane::EMPTY; MAX_SLOTS],
            lane_chains: [LaneChain::SILENT; MAX_SLOTS],
            lfo_settings: [LfoSettings::OFF; MAX_LFOS],
            lfo_phase: [0.0; MAX_LFOS],
            lfo_levels: [0.0; MAX_LFOS],
            lfo_hold: [0.5; MAX_LFOS],
            lfo_prev: [0.0; MAX_LFOS],
            midi_levels: MidiSource::ALL.map(|source| source.default_level()),
            age_counter: 0,
            lane_settings: [[LaneSettings::DEFAULT; 2]; MAX_SLOTS],
            master_gain: 0.0,
            master_settled: false,
            smooth_coef: 1.0 - (-1.0 / (SMOOTH_SECONDS * 44100.0f32)).exp(),
            glide_seconds: 0.0,
            glide_always: false,
            has_played: false,
            glide_coef: 1.0,
            mono_delta: 0.0,
            mono: false,
            held: [(0, 0.0); 128],
            held_len: 0,
            rng: Rng::default(),
            env_configs: [EnvConfig::BYPASS; ENV_SLOTS],
            env_levels: [0.0; MAX_ENVELOPES],
            envelopes_active: false,
            note_tx: Arc::new(Mutex::new(note_tx)),
            note_rx,
            routing: Arc::new(routing::AudioRouting::new()),
            tables: Arc::new(WaveTables::new()),
        }
    }
}

/// Every knob lives on a node in the graph, so the project saves the graph instead of host
/// parameters. Two nodes of one kind keep their own values that way.
#[derive(Params)]
struct FlodeParams {
    /// Saves the editor window size with the project.
    #[persist = "editor-state"]
    editor_state: Arc<IcedState>,

    /// The node graph, so the wiring, the node positions, and every node's knobs survive save and
    /// reload.
    #[persist = "graph"]
    graph: Mutex<node_editor::SavedGraph>,
}

impl Default for FlodeParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            graph: Mutex::new(node_editor::SavedGraph::default()),
        }
    }
}

/// The linear gain for a level in dB. A gain at its minimum is exactly silent.
fn db_to_linear(db: f32) -> f32 {
    if db <= GAIN_MIN_DB {
        0.0
    } else {
        util::db_to_gain_fast(db)
    }
}

impl Plugin for Flode {
    const NAME: &'static str = "Flode";
    const VENDOR: &'static str = "Patrick Maloney";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let note_tx = STANDALONE
            .load(Ordering::Relaxed)
            .then(|| self.note_tx.clone());
        editor::create(
            self.params.clone(),
            self.params.editor_state.clone(),
            note_tx,
            self.routing.clone(),
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        self.smooth_coef = 1.0 - (-1.0 / (SMOOTH_SECONDS * self.sample_rate)).exp();
        // Set the audio chain from the saved graph. The editor keeps it current while it is open.
        if let Ok(saved) = self.params.graph.lock() {
            let graph = node_editor::Graph::from_saved(&saved);
            self.routing.publish(&graph);
        }
        true
    }

    fn reset(&mut self) {
        self.held_len = 0;
        self.lanes = [Lane::EMPTY; MAX_SLOTS];
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // The chain of each output slot and the output's own settings, as the editor last
        // published them. Each chain carries the knob values of its own nodes.
        self.lane_chains = std::array::from_fn(|lane| self.routing.lane(lane));
        self.lfo_settings = std::array::from_fn(|lfo| self.routing.lfo(lfo));
        let globals = self.routing.globals();
        let audible = self.lane_chains.iter().any(|chain| chain.active);
        // Every control source's level right now. Knobs read their route from these.
        let levels = self.control_levels();

        // Block-rate values. A knob wired to the shape takes it through its maps instead of its
        // own value, and updates at the start of each block.
        let sample_rate = self.sample_rate;
        // Each envelope node's times, read once per block. The bypass copy uses the short fade.
        let env_settings: [EnvelopeSettings; MAX_ENVELOPES] =
            std::array::from_fn(|index| self.routing.envelope(index));
        self.envelopes_active = env_settings.iter().any(|settings| settings.active);
        self.env_configs = std::array::from_fn(|slot| {
            let settings = env_settings
                .get(slot)
                .copied()
                .unwrap_or(EnvelopeSettings::OFF);
            if !settings.active {
                return EnvConfig::BYPASS;
            }
            EnvConfig::from_values(
                settings.attack.plain(KnobId::Attack, &levels),
                settings.decay.plain(KnobId::Decay, &levels),
                settings.sustain.plain(KnobId::Sustain, &levels),
                settings.release.plain(KnobId::Release, &levels),
                sample_rate,
            )
        });
        self.lane_settings = std::array::from_fn(|lane| {
            let chain = &self.lane_chains[lane];
            match &chain.mix {
                Some(mix) => [
                    LaneSettings::from_knobs(&mix.a.knobs, &levels),
                    LaneSettings::from_knobs(&mix.b.knobs, &levels),
                ],
                None => [
                    LaneSettings::from_knobs(&chain.knobs, &levels),
                    LaneSettings::DEFAULT,
                ],
            }
        });
        let lane_settings = self.lane_settings;
        // The host's transport, copied out so the context can be used for events below.
        let (beat0, tempo, playing) = {
            let transport = context.transport();
            (transport.pos_beats(), transport.tempo, transport.playing)
        };
        let tables = Arc::clone(&self.tables);
        self.mono = globals.mono;
        self.glide_always = globals.glide_always;
        let glide_seconds = globals.glide.plain(KnobId::Glide, &levels);
        self.glide_seconds = glide_seconds;
        // Each sample closes a fixed share of the gap, so the pitch is within 1% of the target
        // after the glide time.
        self.glide_coef = if glide_seconds <= 0.0 {
            1.0
        } else {
            1.0 - (-5.0 / (glide_seconds * self.sample_rate)).exp()
        };
        let glide_coef = self.glide_coef;
        let smooth = self.smooth_coef;

        while let Ok(note) = self.note_rx.pop() {
            match note {
                PianoEvent::NoteOn { note, velocity } => self.note_on(note, velocity),
                PianoEvent::NoteOff { note } => self.note_off(note),
            }
        }

        let mut next_event = context.next_event();

        for (sample_id, channel_samples) in buffer.iter_samples().enumerate() {
            // Handle every MIDI event scheduled for this sample.
            while let Some(event) = next_event {
                if event.timing() > sample_id as u32 {
                    break;
                }

                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => self.note_on(note, velocity),
                    NoteEvent::NoteOff { note, .. } => self.note_off(note),
                    NoteEvent::MidiPitchBend { value, .. } => {
                        self.midi_levels[MidiSource::PitchBend.index()] = value;
                    }
                    NoteEvent::MidiCC { cc: 1, value, .. } => {
                        self.midi_levels[MidiSource::ModWheel.index()] = value;
                    }
                    NoteEvent::MidiChannelPressure { pressure, .. } => {
                        self.midi_levels[MidiSource::Aftertouch.index()] = pressure;
                    }
                    _ => {}
                }

                next_event = context.next_event();
            }

            let levels = self.control_levels();

            // A gain is exactly silent at its minimum. The master gain eases toward its knob.
            let master_target = db_to_linear(globals.gain.plain(KnobId::Gain, &levels));
            if self.master_settled {
                self.master_gain += (master_target - self.master_gain) * smooth;
            } else {
                self.master_gain = master_target;
                self.master_settled = true;
            }

            // Each active slot sums its voices and runs its own filter. The newest voice's level
            // across all slots is the envelope shape for the next sample.
            // Each active slot sums its voices and runs its own filter. Each envelope's newest voice
            // level is what the lane-wide knobs read for the next sample.
            let mut newest_age = [0u64; MAX_ENVELOPES];
            let mut newest_level = [0.0f32; MAX_ENVELOPES];
            let mut out = 0.0;
            for lane in 0..MAX_SLOTS {
                let chain = self.lane_chains[lane];
                if !chain.active {
                    self.lanes[lane].settled = false;
                    continue;
                }
                let branch_settings = lane_settings[lane];
                // With a mix node, the lane runs two independent sources and combines their
                // samples. Without one, it runs a single source, same as before.
                let prep_a = BranchPrep::new(
                    chain.mix.map_or(chain.waveform, |mix| mix.a.waveform),
                    chain.mix.map_or(chain.envelope, |mix| mix.a.envelope),
                    chain.mix.as_ref().map_or(&chain.knobs, |mix| &mix.a.knobs),
                    branch_settings[0].unison_count,
                    self.envelopes_active,
                    &levels,
                );
                let prep_b = chain.mix.map(|mix| {
                    BranchPrep::new(
                        mix.b.waveform,
                        mix.b.envelope,
                        &mix.b.knobs,
                        branch_settings[1].unison_count,
                        self.envelopes_active,
                        &levels,
                    )
                });

                let mut lane_out = 0.0;
                for slot in self.lanes[lane].voices.iter_mut() {
                    let finished = match slot.as_mut() {
                        Some(voice) => {
                            for index in 0..MAX_ENVELOPES {
                                if voice.age > newest_age[index] {
                                    newest_age[index] = voice.age;
                                    newest_level[index] = voice.envs[index].level;
                                }
                            }
                            // A voice's own envelopes drive its oscillator gain and warp.
                            let voice_levels = ControlLevels {
                                envelope: voice.env_levels(),
                                lfo: self.lfo_levels,
                                midi: self.midi_levels,
                            };
                            voice.glide(glide_coef);

                            let sample = match (&chain.mix, &prep_b) {
                                (Some(mix), Some(prep_b)) => {
                                    let a = voice.branch_contribution(
                                        0,
                                        &tables,
                                        mix.a.waveform,
                                        mix.a.noise,
                                        mix.a.warp,
                                        &mix.a.knobs,
                                        &prep_a,
                                        &voice_levels,
                                        &mut self.rng,
                                    );
                                    let b = voice.branch_contribution(
                                        1,
                                        &tables,
                                        mix.b.waveform,
                                        mix.b.noise,
                                        mix.b.warp,
                                        &mix.b.knobs,
                                        prep_b,
                                        &voice_levels,
                                        &mut self.rng,
                                    );
                                    let mix_amount = mix.mix.plain(KnobId::Mix, &voice_levels);
                                    mix.mode.combine(a, b, mix_amount)
                                }
                                _ => voice.branch_contribution(
                                    0,
                                    &tables,
                                    chain.waveform,
                                    chain.noise,
                                    chain.warp,
                                    &chain.knobs,
                                    &prep_a,
                                    &voice_levels,
                                    &mut self.rng,
                                ),
                            };
                            lane_out += sample;

                            voice.advance_all(&self.env_configs);
                            voice.finished()
                        }
                        None => false,
                    };
                    if finished {
                        *slot = None;
                    }
                }

                // The slot's gain, and its filter's cutoff and resonance, follow their knobs.
                let gain_id = slot_knob(lane);
                let gain_target = db_to_linear(chain.knob(gain_id).plain(gain_id, &levels));
                let cutoff_target = chain.knob(KnobId::Cutoff).plain(KnobId::Cutoff, &levels);
                let resonance_target = chain
                    .knob(KnobId::Resonance)
                    .plain(KnobId::Resonance, &levels);
                let state = &mut self.lanes[lane];
                if state.settled {
                    state.gain += (gain_target - state.gain) * smooth;
                    state.cutoff_log2 += (cutoff_target.log2() - state.cutoff_log2) * smooth;
                    state.resonance += (resonance_target - state.resonance) * smooth;
                } else {
                    state.gain = gain_target;
                    state.cutoff_log2 = cutoff_target.log2();
                    state.resonance = resonance_target;
                    state.settled = true;
                }
                let gain = state.gain;
                let cutoff = state.cutoff_log2.exp2();
                let resonance = state.resonance;

                let lane_out = match chain.filter {
                    Some(filter_type) => {
                        self.filter_sample(lane, lane_out, cutoff, resonance, filter_type)
                    }
                    None => {
                        self.lanes[lane].filter_ic1 = 0.0;
                        self.lanes[lane].filter_ic2 = 0.0;
                        lane_out
                    }
                };
                out += lane_out * gain;
            }

            self.env_levels = newest_level;

            // Each LFO advances once per sample. Its level is its waveform, from 0.0 to 1.0.
            for lfo in 0..MAX_LFOS {
                let settings = self.lfo_settings[lfo];
                if settings.active {
                    // A Sync step past free locks the LFO to the playhead, so it follows the song
                    // while it plays. Otherwise, or without a host tempo, it runs at its rate.
                    let sync = settings.sync.plain(KnobId::LfoSync, &levels).round() as usize;
                    let beats = match (playing, beat0, tempo) {
                        (true, Some(beat), Some(bpm)) if sync >= 1 => {
                            Some(beat + sample_id as f64 * bpm / (60.0 * self.sample_rate as f64))
                        }
                        _ => None,
                    };
                    let phase = match beats {
                        Some(beats) => {
                            let cycles = beats * lfo::cycles_per_beat(sync) as f64;
                            let phase = cycles.rem_euclid(1.0) as f32;
                            self.lfo_phase[lfo] = phase;
                            phase
                        }
                        None => {
                            let phase = self.lfo_phase[lfo];
                            let rate = settings.rate.plain(KnobId::LfoRate, &levels);
                            self.lfo_phase[lfo] = (phase + rate / self.sample_rate).fract();
                            phase
                        }
                    };
                    // Noise holds a random value for each cycle, so its rate is how often it changes.
                    let wrapped = phase < self.lfo_prev[lfo];
                    self.lfo_prev[lfo] = phase;
                    self.lfo_levels[lfo] = if settings.waveform == Waveform::Noise {
                        if wrapped {
                            self.lfo_hold[lfo] = self.rng.unit();
                        }
                        self.lfo_hold[lfo]
                    } else {
                        (settings.waveform.naive(phase) + 1.0) * 0.5
                    };
                } else {
                    self.lfo_levels[lfo] = 0.0;
                }
            }

            let out = if audible {
                out * VOICE_MIX * self.master_gain
            } else {
                0.0
            };
            for sample in channel_samples {
                *sample = out;
            }
        }

        self.routing.set_levels(self.control_levels());

        // Publish which notes are sounding, for the keys on the editor's strip.
        let mut notes = [0u64; 2];
        for voice in self
            .lanes
            .iter()
            .flat_map(|lane| lane.voices.iter())
            .flatten()
        {
            notes[(voice.note / 64) as usize] |= 1u64 << (voice.note % 64);
        }
        self.routing.set_active_notes(notes);

        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for Flode {
    const CLAP_ID: &'static str = "app.flode.flode";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A MIDI-driven synth");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for Flode {
    // Must be exactly 16 bytes and unique to this plugin.
    const VST3_CLASS_ID: [u8; 16] = *b"FlodeSineSynth02";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nih_export_clap!(Flode);
nih_export_vst3!(Flode);

#[cfg(test)]
mod filter_tests {
    use super::*;

    /// The peak output for a sine at `freq`, after the filter has settled.
    fn peak(filter_type: FilterType, cutoff: f32, freq: f32) -> f32 {
        let mut flode = Flode::default();
        let mut peak = 0.0f32;
        for n in 0..44100 {
            let input = (std::f32::consts::TAU * freq * n as f32 / 44100.0).sin();
            let out = flode.filter_sample(0, input, cutoff, 0.0, filter_type);
            if n > 22050 {
                peak = peak.max(out.abs());
            }
        }
        peak
    }

    #[test]
    fn each_type_passes_its_own_band() {
        assert!(peak(FilterType::LowPass, 1000.0, 100.0) > 0.9);
        assert!(peak(FilterType::LowPass, 1000.0, 10000.0) < 0.2);

        assert!(peak(FilterType::HighPass, 1000.0, 10000.0) > 0.9);
        assert!(peak(FilterType::HighPass, 1000.0, 100.0) < 0.2);

        assert!(peak(FilterType::BandPass, 1000.0, 1000.0) > 0.9);
        assert!(peak(FilterType::BandPass, 1000.0, 100.0) < 0.2);
        assert!(peak(FilterType::BandPass, 1000.0, 10000.0) < 0.2);
    }
}

#[cfg(test)]
mod glide_tests {
    use super::*;

    /// In mono mode, a second note starts at the first note's pitch and glides to its own.
    #[test]
    fn mono_note_glides_from_the_previous_pitch() {
        let mut flode = Flode::default();
        flode.mono = true;
        flode.glide_seconds = 0.1;
        flode.glide_coef = 1.0 - (-5.0 / (0.1 * flode.sample_rate)).exp();

        flode.lane_chains[0] = LaneChain {
            active: true,
            ..LaneChain::SILENT
        };
        flode.note_on(60, 1.0);
        let first_target = util::midi_note_to_freq(60) / flode.sample_rate;
        flode.note_on(72, 1.0);
        let second_target = util::midi_note_to_freq(72) / flode.sample_rate;

        let newest = flode.lanes[0]
            .voices
            .iter()
            .flatten()
            .max_by_key(|v| v.age)
            .unwrap();
        assert!((newest.base_delta - first_target).abs() < 1e-9);
        assert!((newest.target_delta - second_target).abs() < 1e-9);

        let coef = flode.glide_coef;
        let newest = flode.lanes[0]
            .voices
            .iter_mut()
            .flatten()
            .max_by_key(|v| v.age)
            .unwrap();
        for _ in 0..(0.1 * flode.sample_rate) as usize {
            newest.glide(coef);
        }
        assert!((newest.base_delta - second_target).abs() / second_target < 0.01);
    }
}

#[cfg(test)]
mod lane_tests {
    use super::*;

    /// Two active slots start their voices with their own settings, so the slots do not share
    /// a detune, a unison count, or a phase spread.
    #[test]
    fn each_slot_uses_its_own_oscillator_settings() {
        let mut flode = Flode::default();
        for lane in 0..2 {
            flode.lane_chains[lane] = LaneChain {
                active: true,
                ..LaneChain::SILENT
            };
        }
        flode.lane_settings[0] = [
            LaneSettings {
                unison_count: 3,
                detune_cents: 50.0,
                ..LaneSettings::DEFAULT
            },
            LaneSettings::DEFAULT,
        ];
        flode.lane_settings[1] = [
            LaneSettings {
                unison_count: 1,
                detune_cents: 0.0,
                ..LaneSettings::DEFAULT
            },
            LaneSettings::DEFAULT,
        ];

        flode.note_on(60, 1.0);

        let first = flode.lanes[0].voices.iter().flatten().next().unwrap();
        let second = flode.lanes[1].voices.iter().flatten().next().unwrap();
        // The first slot spreads three voices. The second plays one, with no detune.
        assert!(
            first.oscillators[0].detune_ratios[0] < 1.0
                && first.oscillators[0].detune_ratios[2] > 1.0
        );
        assert_eq!(second.oscillators[0].detune_ratios[0], 1.0);
        // A slot that is not active starts no voice.
        assert!(flode.lanes[2].voices.iter().all(|v| v.is_none()));
    }
}

#[cfg(test)]
mod always_glide_tests {
    use super::*;

    fn mono_flode(always: bool) -> Flode {
        let mut flode = Flode::default();
        flode.mono = true;
        flode.glide_seconds = 0.1;
        flode.glide_always = always;
        flode.lane_chains[0] = LaneChain {
            active: true,
            ..LaneChain::SILENT
        };
        flode
    }

    /// With Always on, a note glides from the last pitch even when no voice is still sounding.
    #[test]
    fn always_glides_after_the_previous_note_has_died() {
        let mut flode = mono_flode(true);
        flode.note_on(60, 1.0);
        let last = util::midi_note_to_freq(60) / flode.sample_rate;
        for lane in flode.lanes.iter_mut() {
            lane.voices = [None; NUM_VOICES];
        }
        flode.note_on(72, 1.0);
        let newest = flode.lanes[0].voices.iter().flatten().next().unwrap();
        assert!((newest.base_delta - last).abs() < 1e-9);
    }

    /// Without Always, the same note starts at its own pitch.
    #[test]
    fn without_always_a_dead_previous_note_is_not_glided_from() {
        let mut flode = mono_flode(false);
        flode.note_on(60, 1.0);
        for lane in flode.lanes.iter_mut() {
            lane.voices = [None; NUM_VOICES];
        }
        flode.note_on(72, 1.0);
        let newest = flode.lanes[0].voices.iter().flatten().next().unwrap();
        assert!((newest.base_delta - newest.target_delta).abs() < 1e-9);
    }

    /// The first note never glides, even with Always on, because there is no pitch to glide from.
    #[test]
    fn first_note_never_glides() {
        let mut flode = mono_flode(true);
        flode.note_on(60, 1.0);
        let newest = flode.lanes[0].voices.iter().flatten().next().unwrap();
        assert!((newest.base_delta - newest.target_delta).abs() < 1e-9);
    }
}

#[cfg(test)]
mod release_tests {
    use super::*;

    /// A note with an envelope releases over the envelope's release time after its note off.
    #[test]
    fn note_off_releases_over_the_envelope_release_time() {
        let mut flode = Flode::default();
        flode.sample_rate = 44100.0;
        flode.lane_chains[0] = LaneChain {
            active: true,
            envelope: Some(0),
            ..LaneChain::SILENT
        };
        flode.env_configs[0] = EnvConfig::from_values(0.001, 0.001, 1.0, 0.5, 44100.0);
        flode.note_on(60, 1.0);
        for _ in 0..2000 {
            for voice in flode.lanes[0].voices.iter_mut().flatten() {
                voice.advance_all(&flode.env_configs);
            }
        }
        flode.note_off(60);
        let voice = flode.lanes[0].voices.iter().flatten().next().unwrap();
        assert_eq!(voice.envs[0].stage, Stage::Release);
        for _ in 0..(0.25 * 44100.0) as usize {
            for voice in flode.lanes[0].voices.iter_mut().flatten() {
                voice.advance_all(&flode.env_configs);
            }
        }
        let voice = flode.lanes[0].voices.iter().flatten().next().unwrap();
        assert!(
            voice.envs[0].level > 0.1 && voice.envs[0].level < 0.6,
            "level {}",
            voice.envs[0].level
        );

        // The voice stays alive through the whole release, then the engine drops it.
        for _ in 0..(0.3 * 44100.0) as usize {
            for voice in flode.lanes[0].voices.iter_mut().flatten() {
                voice.advance_all(&flode.env_configs);
            }
        }
        assert!(flode.lanes[0].voices.iter().flatten().all(|v| v.finished()));
    }

    /// A release never takes an envelope below zero, even while the voice waits on a longer one.
    #[test]
    fn a_finished_release_stays_at_zero() {
        let mut env = Adsr {
            stage: Stage::Release,
            level: 0.1,
            release_step: 0.01,
        };
        let config = EnvConfig::BYPASS;
        for _ in 0..100 {
            env.advance(&config);
        }
        assert_eq!(env.level, 0.0);
    }

    /// With an envelope driving the sound, a note off keeps the voice alive for the envelope's
    /// release, rather than cutting it in the short bypass fade.
    #[test]
    fn a_driven_note_keeps_its_voice_through_the_envelope_release() {
        let mut flode = Flode::default();
        flode.sample_rate = 44100.0;
        flode.envelopes_active = true;
        flode.env_configs[0] = EnvConfig::from_values(0.001, 0.001, 0.5, 0.5, 44100.0);
        flode.lane_chains[0] = LaneChain {
            active: true,
            ..LaneChain::SILENT
        };
        flode.note_on(60, 1.0);
        for _ in 0..4410 {
            for voice in flode.lanes[0].voices.iter_mut().flatten() {
                voice.advance_all(&flode.env_configs);
            }
        }
        flode.note_off(60);
        for _ in 0..(0.1 * 44100.0) as usize {
            for voice in flode.lanes[0].voices.iter_mut().flatten() {
                voice.advance_all(&flode.env_configs);
            }
        }
        let voice = flode.lanes[0].voices.iter().flatten().next().unwrap();
        assert!(!voice.finished());
        assert!(voice.envs[MAX_ENVELOPES].level > 0.5);
    }
}

#[cfg(test)]
mod mix_tests {
    use super::*;
    use nodes::mix::MixMode;
    use routing::{Branch, MixChain};

    fn active_branch(waveform: Waveform) -> Branch {
        Branch {
            active: true,
            waveform,
            ..Branch::SILENT
        }
    }

    /// A mix lane sets up two independent oscillators at note-on. Stepping one branch many times
    /// leaves the other branch's phase untouched.
    #[test]
    fn a_mix_voice_keeps_each_branchs_phase_independent() {
        let mut flode = Flode::default();
        flode.sample_rate = 44100.0;
        flode.lane_settings[0] = [
            LaneSettings {
                unison_count: 1,
                ..LaneSettings::DEFAULT
            },
            LaneSettings {
                unison_count: 1,
                ..LaneSettings::DEFAULT
            },
        ];
        flode.lane_chains[0] = LaneChain {
            active: true,
            mix: Some(MixChain {
                mode: MixMode::Crossfade,
                mix: KnobValue::ZERO,
                a: active_branch(Waveform::Sine),
                b: active_branch(Waveform::Sine),
            }),
            ..LaneChain::SILENT
        };
        flode.note_on(60, 1.0);

        let prep = BranchPrep::new(
            Waveform::Sine,
            None,
            &[KnobValue::ZERO; KnobId::COUNT],
            1,
            false,
            &ControlLevels::default(),
        );
        let tables = Arc::clone(&flode.tables);
        let voice = flode.lanes[0].voices[0].as_mut().unwrap();
        for _ in 0..1000 {
            voice.branch_contribution(
                1,
                &tables,
                Waveform::Sine,
                NoiseType::White,
                WarpMode::None,
                &[KnobValue::ZERO; KnobId::COUNT],
                &prep,
                &ControlLevels::default(),
                &mut Rng::default(),
            );
        }

        assert_eq!(voice.oscillators[0].phases[0], 0.0);
        assert_ne!(voice.oscillators[1].phases[0], 0.0);
    }

    /// A crossfade at zero mix passes branch A through unchanged, matching `MixMode::combine`.
    #[test]
    fn crossfade_mode_combines_two_branch_samples() {
        let a = 0.6;
        let b = -0.2;
        assert_eq!(MixMode::Crossfade.combine(a, b, 0.0), a);
        assert!((MixMode::Crossfade.combine(a, b, 1.0) - b).abs() < 1e-6);
    }
}
