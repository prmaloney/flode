//! The arpeggiators. Each one is a small step sequencer: once a sample it advances a clock, and
//! when the clock crosses a step, it looks at the chord currently held at its input slot, works out
//! the next note of its pattern, and reports a note on for it. A separate check inside the step
//! finds the note off once the Gate knob's share of the step has passed.
//!
//! This mirrors `Lfos` in shape, but where an LFO's result is a continuous level, an arpeggiator's
//! is a stream of note changes, so `advance` reports events instead of holding a readable level.
//!
//! The clock here is a simpler cousin of `Lfos`' free/synced rate: it runs at a frequency, free or
//! from the host tempo, rather than locking its absolute phase to the song position the way a synced
//! LFO does. A tempo change or a host seek nudges the step clock's rate rather than snapping it to a
//! beat boundary. That is enough to keep a pattern in time with the tempo, which is what Sync is for
//! here; it does not guarantee a step always lands exactly on a beat after the host has jumped.

use super::notes::{HeldNotes, NoteBuffer, NoteEvent};
use crate::dsp::rng::Rng;
use crate::engine::Transport;
use crate::nodes::arpeggiator::ArpPattern;
use crate::nodes::lfo::cycles_per_beat;
use crate::patch::control::{ControlLevels, MAX_NOTE_SOURCES};
use crate::patch::{ArpeggiatorSettings, NoteSourceSpec};
use arrayvec::ArrayVec;

/// The most notes one arpeggiator's pattern can hold: a chord's worth of notes, repeated across up
/// to four octaves. Real chords are far smaller than this; it is a backstop, not a target.
const ARP_PATTERN_CAP: usize = 64;

/// What one arpeggiator remembers between samples.
#[derive(Debug, Clone, Copy, Default)]
struct ArpVoice {
    phase: f32,
    /// The step the pattern is on. It only ever counts up, so the pattern keeps moving forward
    /// through a chord that changes size, rather than restarting at step 0 on every note.
    step: usize,
    /// The note currently sounding, if the gate has not released it yet.
    sounding: Option<u8>,
    /// True once this step's gate release has already fired, so it does not fire twice.
    released: bool,
    /// True while the clock is running. It goes false whenever the chord is empty, so the pattern
    /// always starts its first step right away the next time a note is held, rather than waiting
    /// out whatever was left of a cycle that began before anything was down to play.
    started: bool,
}

pub struct Arpeggiators {
    state: [ArpVoice; MAX_NOTE_SOURCES],
}

impl Arpeggiators {
    pub const fn new() -> Self {
        Self {
            state: [ArpVoice {
                phase: 0.0,
                step: 0,
                sounding: None,
                released: false,
                started: false,
            }; MAX_NOTE_SOURCES],
        }
    }

    /// The step rate in cycles (steps) per second: the host tempo's note division with Sync on, or
    /// the Rate knob running free.
    fn cycles_per_second(
        settings: &ArpeggiatorSettings,
        levels: &ControlLevels,
        transport: &Transport,
    ) -> f32 {
        let sync = settings.sync.plain(levels).round() as usize;
        match (sync >= 1, transport.tempo) {
            (true, Some(bpm)) => bpm as f32 / 60.0 * cycles_per_beat(sync),
            _ => settings.rate.plain(levels),
        }
    }

    /// Moves every arpeggiator on by one sample. Each arpeggiator slot's `NoteBuffer` in `events` is
    /// the signal this call puts on its Notes wire for the sample: cleared first, then given the
    /// note on or note off, or both, that the slot's step clock makes this sample. The engine
    /// dispatches host notes and arpeggiator steps as it reaches them rather than batching a whole
    /// block ahead of time, so in practice at most one on and one off ever land in a buffer here,
    /// both carrying offset 0 within this sample; see `NoteBuffer` for why the type still carries a
    /// sample offset.
    pub fn advance(
        &mut self,
        note_sources: &[Option<NoteSourceSpec>; MAX_NOTE_SOURCES],
        held: &[HeldNotes; MAX_NOTE_SOURCES],
        levels: &ControlLevels,
        transport: &Transport,
        sample_rate: f32,
        rng: &mut Rng,
        events: &mut [NoteBuffer; MAX_NOTE_SOURCES],
    ) {
        for buffer in events.iter_mut() {
            buffer.clear();
        }
        for slot in 0..MAX_NOTE_SOURCES {
            let Some(NoteSourceSpec::Arpeggiator(settings)) = &note_sources[slot] else {
                // Not an arpeggiator this block: drop any state, so a slot that changes kind, or
                // stops existing, does not leave a stuck note behind it.
                self.state[slot] = ArpVoice::default();
                continue;
            };
            let voice = &mut self.state[slot];
            let chord: &[(u8, f32)] = match settings.input {
                Some(input) if input < MAX_NOTE_SOURCES => held[input].notes(),
                _ => &[],
            };

            if chord.is_empty() {
                // Nothing held: the clock stops, and whatever was sounding stops with it, so the
                // next chord starts its pattern fresh and right away instead of mid cycle.
                voice.started = false;
                voice.phase = 0.0;
                if let Some(note) = voice.sounding.take() {
                    events[slot].push(NoteEvent {
                        sample_offset: 0,
                        note,
                        velocity: 0.0,
                        on: false,
                    });
                }
                continue;
            }

            let cycles = Self::cycles_per_second(settings, levels, transport);
            let before = voice.phase;
            voice.phase = (before + cycles / sample_rate).fract();
            // The first sample a chord is held steps right away; a stopped clock has nothing to
            // wrap from.
            let first = !voice.started;
            voice.started = true;
            let wrapped = first || voice.phase < before;

            if wrapped {
                // A step always starts clean: whatever was sounding stops first, so steps never
                // overlap even if the gate had not yet released it.
                if let Some(note) = voice.sounding.take() {
                    events[slot].push(NoteEvent {
                        sample_offset: 0,
                        note,
                        velocity: 0.0,
                        on: false,
                    });
                }
                voice.released = false;
                let octaves = (settings.octaves.plain(levels).round() as usize).clamp(1, 4);
                let next = next_note(chord, settings.pattern, octaves, voice.step, rng);
                voice.step = voice.step.wrapping_add(1);
                if let Some((note, velocity)) = next {
                    voice.sounding = Some(note);
                    events[slot].push(NoteEvent {
                        sample_offset: 0,
                        note,
                        velocity,
                        on: true,
                    });
                }
            } else if let Some(note) = voice.sounding {
                if !voice.released && voice.phase >= settings.gate.plain(levels).clamp(0.0, 1.0) {
                    voice.released = true;
                    voice.sounding = None;
                    events[slot].push(NoteEvent {
                        sample_offset: 0,
                        note,
                        velocity: 0.0,
                        on: false,
                    });
                }
            }
        }
    }
}

impl Default for Arpeggiators {
    fn default() -> Self {
        Self::new()
    }
}

/// The chord held, repeated across `octaves` octaves, in as-played order: every note of the chord
/// at the base octave, then every note a further octave up, and so on. A note that climbs past the
/// top of the MIDI range is dropped, and the pattern stops growing once it is full.
fn build_pattern(chord: &[(u8, f32)], octaves: usize) -> ArrayVec<(u8, f32), ARP_PATTERN_CAP> {
    let mut built = ArrayVec::new();
    'octaves: for octave in 0..octaves {
        for &(note, velocity) in chord {
            let shifted = i16::from(note) + 12 * octave as i16;
            if shifted > i16::from(u8::MAX) {
                continue;
            }
            if built.try_push((shifted as u8, velocity)).is_err() {
                break 'octaves;
            }
        }
    }
    built
}

/// `up`, sorted ascending, followed by its reverse with both ends left off, so a two-way sweep of a
/// chord does not repeat its highest or lowest note.
fn up_down_pattern(
    mut up: ArrayVec<(u8, f32), ARP_PATTERN_CAP>,
) -> ArrayVec<(u8, f32), ARP_PATTERN_CAP> {
    up.sort_by_key(|&(note, _)| note);
    let mut result = up.clone();
    let middle = up.len().saturating_sub(2);
    for &note in up.iter().rev().skip(1).take(middle) {
        if result.try_push(note).is_err() {
            break;
        }
    }
    result
}

/// The note the pattern plays at `step`, or None for an empty chord. Random draws a fresh note from
/// the chord each call rather than following `step`.
fn next_note(
    chord: &[(u8, f32)],
    pattern: ArpPattern,
    octaves: usize,
    step: usize,
    rng: &mut Rng,
) -> Option<(u8, f32)> {
    if chord.is_empty() {
        return None;
    }
    let built = build_pattern(chord, octaves);
    if built.is_empty() {
        return None;
    }
    match pattern {
        ArpPattern::AsPlayed => Some(built[step % built.len()]),
        ArpPattern::Up => {
            let mut sorted = built;
            sorted.sort_by_key(|&(note, _)| note);
            Some(sorted[step % sorted.len()])
        }
        ArpPattern::Down => {
            let mut sorted = built;
            sorted.sort_by_key(|&(note, _)| std::cmp::Reverse(note));
            Some(sorted[step % sorted.len()])
        }
        ArpPattern::UpDown => {
            let sweep = up_down_pattern(built);
            Some(sweep[step % sweep.len()])
        }
        ArpPattern::Random => {
            let index = ((rng.unit() * built.len() as f32) as usize).min(built.len() - 1);
            Some(built[index])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn up_sorts_the_chord_ascending() {
        let chord = [(67, 1.0), (60, 1.0), (64, 1.0)];
        let mut rng = Rng::default();
        let steps: Vec<u8> = (0..3)
            .map(|step| {
                next_note(&chord, ArpPattern::Up, 1, step, &mut rng)
                    .unwrap()
                    .0
            })
            .collect();
        assert_eq!(steps, [60, 64, 67]);
        // The pattern wraps back to the start on the step after the last note.
        assert_eq!(
            next_note(&chord, ArpPattern::Up, 1, 3, &mut rng).unwrap().0,
            60
        );
    }

    #[test]
    fn down_sorts_the_chord_descending() {
        let chord = [(60, 1.0), (64, 1.0), (67, 1.0)];
        let mut rng = Rng::default();
        let steps: Vec<u8> = (0..3)
            .map(|step| {
                next_note(&chord, ArpPattern::Down, 1, step, &mut rng)
                    .unwrap()
                    .0
            })
            .collect();
        assert_eq!(steps, [67, 64, 60]);
    }

    #[test]
    fn as_played_keeps_press_order() {
        let chord = [(67, 1.0), (60, 1.0), (64, 1.0)];
        let mut rng = Rng::default();
        let steps: Vec<u8> = (0..3)
            .map(|step| {
                next_note(&chord, ArpPattern::AsPlayed, 1, step, &mut rng)
                    .unwrap()
                    .0
            })
            .collect();
        assert_eq!(steps, [67, 60, 64]);
    }

    /// Up-down sweeps up then back down without repeating either end.
    #[test]
    fn up_down_does_not_repeat_its_ends() {
        let chord = [(60, 1.0), (64, 1.0), (67, 1.0), (72, 1.0)];
        let mut rng = Rng::default();
        let steps: Vec<u8> = (0..6)
            .map(|step| {
                next_note(&chord, ArpPattern::UpDown, 1, step, &mut rng)
                    .unwrap()
                    .0
            })
            .collect();
        assert_eq!(steps, [60, 64, 67, 72, 67, 64]);
    }

    /// Octaves repeats the chord an octave higher for every step past the first lap.
    #[test]
    fn octaves_repeats_the_chord_higher() {
        let chord = [(60, 1.0)];
        let mut rng = Rng::default();
        assert_eq!(
            next_note(&chord, ArpPattern::Up, 2, 0, &mut rng).unwrap().0,
            60
        );
        assert_eq!(
            next_note(&chord, ArpPattern::Up, 2, 1, &mut rng).unwrap().0,
            72
        );
    }

    #[test]
    fn an_empty_chord_plays_nothing() {
        let mut rng = Rng::default();
        assert_eq!(next_note(&[], ArpPattern::Up, 1, 0, &mut rng), None);
    }

    /// A free running arpeggiator steps at its Rate and reports a note on at every step, with a
    /// note off first once a note is already sounding.
    #[test]
    fn a_free_running_arpeggiator_steps_at_its_rate() {
        use crate::nodes::knobs::KnobId;
        use crate::patch::control::Param;

        let mut arps = Arpeggiators::new();
        let mut sources = [None; MAX_NOTE_SOURCES];
        sources[0] = Some(NoteSourceSpec::Input);
        sources[1] = Some(NoteSourceSpec::Arpeggiator(ArpeggiatorSettings {
            input: Some(0),
            pattern: ArpPattern::Up,
            rate: Param::fixed(KnobId::LfoRate, 10.0),
            sync: Param::fixed(KnobId::LfoSync, 0.0),
            octaves: Param::fixed(KnobId::ArpOctaves, 1.0),
            gate: Param::fixed(KnobId::ArpGate, 1.0),
        }));
        let mut held: [HeldNotes; MAX_NOTE_SOURCES] = std::array::from_fn(|_| HeldNotes::new());
        held[0].push(60, 0.8);
        held[0].push(64, 0.8);
        let levels = ControlLevels::default();
        let transport = Transport::default();
        let mut rng = Rng::default();

        // 10 Hz at 44100 Hz is one step every 4410 samples.
        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        let mut ons = Vec::new();
        for sample in 0..4410 * 3 {
            arps.advance(
                &sources,
                &held,
                &levels,
                &transport,
                44100.0,
                &mut rng,
                &mut events,
            );
            for event in events[1].as_slice() {
                if event.on {
                    ons.push((sample, event.note));
                }
            }
        }
        // The pattern steps the moment the chord is held, then once every 4410 samples after.
        assert_eq!(ons.len(), 3, "{ons:?}");
        assert_eq!(ons[0], (0, 60));
        assert_eq!(
            ons.iter().map(|&(_, note)| note).collect::<Vec<_>>(),
            [60, 64, 60]
        );
    }

    /// Less than full gate releases the note before the next step starts.
    #[test]
    fn the_gate_releases_the_note_early() {
        use crate::nodes::knobs::KnobId;
        use crate::patch::control::Param;

        let mut arps = Arpeggiators::new();
        let mut sources = [None; MAX_NOTE_SOURCES];
        sources[0] = Some(NoteSourceSpec::Input);
        sources[1] = Some(NoteSourceSpec::Arpeggiator(ArpeggiatorSettings {
            input: Some(0),
            pattern: ArpPattern::Up,
            rate: Param::fixed(KnobId::LfoRate, 10.0),
            sync: Param::fixed(KnobId::LfoSync, 0.0),
            octaves: Param::fixed(KnobId::ArpOctaves, 1.0),
            gate: Param::fixed(KnobId::ArpGate, 0.5),
        }));
        let mut held: [HeldNotes; MAX_NOTE_SOURCES] = std::array::from_fn(|_| HeldNotes::new());
        held[0].push(60, 0.8);
        let levels = ControlLevels::default();
        let transport = Transport::default();
        let mut rng = Rng::default();

        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        let mut off_at = None;
        for sample in 0..4410 {
            arps.advance(
                &sources,
                &held,
                &levels,
                &transport,
                44100.0,
                &mut rng,
                &mut events,
            );
            for event in events[1].as_slice() {
                if !event.on && off_at.is_none() {
                    off_at = Some(sample);
                }
            }
        }
        // Half of a 4410 sample step is about 2205 samples in.
        let off_at = off_at.expect("the gate releases the note");
        assert!((2190..2220).contains(&off_at), "{off_at}");
    }
}
