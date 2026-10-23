//! The chords. A chord is not a clock the way an arpeggiator is: it has nothing to step through, it
//! only reacts to notes changing at its input. Each sample it compares what is currently held at
//! its input slot against what it saw last sample. A note that is new gets its chord expanded and
//! turned on right away. A note that is gone gets exactly the notes that were turned on for it
//! turned off, whatever the chord type or the spread knob now say, so a knob moved while a note is
//! still held never releases the wrong notes or leaves one stuck.
//!
//! This mirrors `Arpeggiators` in shape: one `advance` call a sample, reading `note_sources` and
//! `held` and writing into a `NoteBuffer` per slot, called from the same place in the engine.

use super::notes::{HeldNotes, NoteBuffer, NoteEvent};
use crate::nodes::chord::MAX_CHORD_NOTES;
use crate::patch::control::{ControlLevels, MAX_NOTE_SOURCES};
use crate::patch::{ChordSettings, NoteSourceSpec};

/// The most input notes one chord slot remembers at once. Real playing rarely holds more than a
/// handful of notes into a chord at a time; this is a backstop, not a target, the same role
/// `NUM_VOICES` plays for a lane's voices.
const MAX_CHORD_VOICES: usize = 16;

/// The notes a chord turned on for one input note, so its note off releases exactly those, not
/// whatever the current knobs would produce.
#[derive(Debug, Clone, Copy)]
struct ChordVoice {
    /// The input note this chord's notes were built from.
    input: u8,
    notes: [u8; MAX_CHORD_NOTES],
    count: usize,
}

/// What one chord node remembers between samples: one `ChordVoice` for every input note currently
/// held, or None in the rest.
#[derive(Debug, Clone, Copy)]
struct ChordSlot {
    voices: [Option<ChordVoice>; MAX_CHORD_VOICES],
}

impl Default for ChordSlot {
    fn default() -> Self {
        Self {
            voices: [None; MAX_CHORD_VOICES],
        }
    }
}

pub struct Chords {
    state: [ChordSlot; MAX_NOTE_SOURCES],
}

impl Chords {
    pub const fn new() -> Self {
        Self {
            state: [ChordSlot {
                voices: [None; MAX_CHORD_VOICES],
            }; MAX_NOTE_SOURCES],
        }
    }

    /// Moves every chord on by this sample: releases the notes of any input note that let go since
    /// last sample, then turns on a fresh chord for any input note that is new. Each chord slot's
    /// `NoteBuffer` in `events` is cleared first, the same contract `Arpeggiators::advance` keeps.
    pub fn advance(
        &mut self,
        note_sources: &[Option<NoteSourceSpec>; MAX_NOTE_SOURCES],
        held: &[HeldNotes; MAX_NOTE_SOURCES],
        levels: &ControlLevels,
        events: &mut [NoteBuffer; MAX_NOTE_SOURCES],
    ) {
        for buffer in events.iter_mut() {
            buffer.clear();
        }
        for slot in 0..MAX_NOTE_SOURCES {
            let Some(NoteSourceSpec::Chord(settings)) = &note_sources[slot] else {
                // Not a chord this block: drop any state, so a slot that changes kind, or stops
                // existing, does not leave a stuck note behind it.
                self.state[slot] = ChordSlot::default();
                continue;
            };
            let current: &[(u8, f32)] = match settings.input {
                Some(input) if input < MAX_NOTE_SOURCES => held[input].notes(),
                _ => &[],
            };
            let buffer = &mut events[slot];
            Self::release_gone(&mut self.state[slot], current, buffer);
            Self::sound_new(&mut self.state[slot], current, settings, levels, buffer);
        }
    }

    /// Releases every remembered voice whose input note is no longer in `current`.
    fn release_gone(slot: &mut ChordSlot, current: &[(u8, f32)], buffer: &mut NoteBuffer) {
        for voice in slot.voices.iter_mut() {
            let gone =
                voice.is_some_and(|voice| !current.iter().any(|&(note, _)| note == voice.input));
            if gone {
                let taken = voice.take().expect("checked above");
                for &note in &taken.notes[..taken.count] {
                    buffer.push(NoteEvent {
                        sample_offset: 0,
                        note,
                        velocity: 0.0,
                        on: false,
                    });
                }
            }
        }
    }

    /// Expands and sounds the chord for every note in `current` that has no voice yet.
    fn sound_new(
        slot: &mut ChordSlot,
        current: &[(u8, f32)],
        settings: &ChordSettings,
        levels: &ControlLevels,
        buffer: &mut NoteBuffer,
    ) {
        let spread = settings.spread.plain(levels).clamp(0.0, 1.0);
        for &(note, velocity) in current {
            if slot.voices.iter().flatten().any(|v| v.input == note) {
                continue;
            }
            let Some(free) = slot.voices.iter().position(Option::is_none) else {
                // Every voice slot is taken: drop this note rather than sound it with nowhere to
                // remember it, which would leave its note off with nothing to release.
                continue;
            };
            let mut voice = ChordVoice {
                input: note,
                notes: [0; MAX_CHORD_NOTES],
                count: 0,
            };
            for (index, &interval) in settings.chord.intervals().iter().enumerate() {
                // Every note past the root climbs `index` extra octaves at full spread, and none at
                // zero, interpolated continuously in between.
                let extra = (index as f32 * spread * 12.0).round() as i32;
                let out_note = (i32::from(note) + i32::from(interval) + extra).clamp(0, 127) as u8;
                voice.notes[voice.count] = out_note;
                voice.count += 1;
                buffer.push(NoteEvent {
                    sample_offset: 0,
                    note: out_note,
                    velocity,
                    on: true,
                });
            }
            slot.voices[free] = Some(voice);
        }
    }
}

impl Default for Chords {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::chord::ChordType;
    use crate::nodes::knobs::KnobId;
    use crate::patch::control::Param;

    fn settings(chord: ChordType, spread: f32) -> ChordSettings {
        ChordSettings {
            input: Some(0),
            chord,
            spread: Param::fixed(KnobId::ChordSpread, spread),
        }
    }

    fn node_sources(settings: ChordSettings) -> [Option<NoteSourceSpec>; MAX_NOTE_SOURCES] {
        let mut sources = [None; MAX_NOTE_SOURCES];
        sources[0] = Some(NoteSourceSpec::Input);
        sources[1] = Some(NoteSourceSpec::Chord(settings));
        sources
    }

    fn held_with(note: u8, velocity: f32) -> [HeldNotes; MAX_NOTE_SOURCES] {
        let mut held: [HeldNotes; MAX_NOTE_SOURCES] = std::array::from_fn(|_| HeldNotes::new());
        held[0].push(note, velocity);
        held
    }

    /// A note on at the chord's input immediately produces a note on for the root plus every
    /// interval, in the same sample.
    #[test]
    fn a_held_note_sounds_its_whole_chord_at_once() {
        let mut chords = Chords::new();
        let sources = node_sources(settings(ChordType::Major, 0.0));
        let held = held_with(60, 0.8);
        let levels = ControlLevels::default();
        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        chords.advance(&sources, &held, &levels, &mut events);
        let ons: Vec<u8> = events[1]
            .as_slice()
            .iter()
            .filter(|e| e.on)
            .map(|e| e.note)
            .collect();
        assert_eq!(ons, [60, 64, 67]);
    }

    /// Spread at 0.0 plays the chord's own listed intervals unchanged.
    #[test]
    fn spread_at_zero_keeps_the_chords_own_voicing() {
        let mut chords = Chords::new();
        let sources = node_sources(settings(ChordType::Major7, 0.0));
        let held = held_with(60, 0.8);
        let levels = ControlLevels::default();
        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        chords.advance(&sources, &held, &levels, &mut events);
        let ons: Vec<u8> = events[1]
            .as_slice()
            .iter()
            .filter(|e| e.on)
            .map(|e| e.note)
            .collect();
        assert_eq!(ons, [60, 64, 67, 71]);
    }

    /// Spread at 1.0 raises each successive note by one extra octave in sequence: a three-note
    /// chord's top note ends up two octaves higher than at spread 0.0, and a four-note chord's top
    /// note ends up three octaves higher.
    #[test]
    fn spread_at_one_raises_each_note_an_extra_octave() {
        let mut chords = Chords::new();
        let levels = ControlLevels::default();

        let sources = node_sources(settings(ChordType::Major, 1.0));
        let held = held_with(60, 0.8);
        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        chords.advance(&sources, &held, &levels, &mut events);
        let ons: Vec<u8> = events[1]
            .as_slice()
            .iter()
            .filter(|e| e.on)
            .map(|e| e.note)
            .collect();
        // Root unmoved, +1 octave, +2 octaves.
        assert_eq!(ons, [60, 76, 91]);

        let mut chords = Chords::new();
        let sources = node_sources(settings(ChordType::Major7, 1.0));
        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        chords.advance(&sources, &held, &levels, &mut events);
        let ons: Vec<u8> = events[1]
            .as_slice()
            .iter()
            .filter(|e| e.on)
            .map(|e| e.note)
            .collect();
        // Root unmoved, +1, +2, +3 octaves.
        assert_eq!(ons, [60, 76, 91, 107]);
    }

    /// The note off releases exactly the notes that were turned on for it, not whatever the chord
    /// type or spread knob now say, even after both change while the note is still held.
    #[test]
    fn a_note_off_releases_what_was_remembered_not_what_is_now_set() {
        let mut chords = Chords::new();
        let mut sources = node_sources(settings(ChordType::Major, 0.0));
        let held = held_with(60, 0.8);
        let levels = ControlLevels::default();
        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        chords.advance(&sources, &held, &levels, &mut events);

        // The chord type and spread change while the note is still held.
        let Some(NoteSourceSpec::Chord(settings)) = &mut sources[1] else {
            panic!("expected a chord");
        };
        settings.chord = ChordType::Minor7;
        settings.spread = Param::fixed(KnobId::ChordSpread, 1.0);

        // Nothing held changes, so no new chord is sounded, and nothing releases yet.
        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        chords.advance(&sources, &held, &levels, &mut events);
        assert!(events[1].as_slice().is_empty());

        // Letting go releases exactly the original major triad, not a minor 7th spread apart.
        let empty: [HeldNotes; MAX_NOTE_SOURCES] = std::array::from_fn(|_| HeldNotes::new());
        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        chords.advance(&sources, &empty, &levels, &mut events);
        let offs: Vec<u8> = events[1]
            .as_slice()
            .iter()
            .filter(|e| !e.on)
            .map(|e| e.note)
            .collect();
        assert_eq!(offs, [60, 64, 67]);
    }

    /// Two notes held at once track independently: releasing one leaves the other's chord sounding.
    #[test]
    fn two_held_notes_release_independently() {
        let mut chords = Chords::new();
        let sources = node_sources(settings(ChordType::Power, 0.0));
        let levels = ControlLevels::default();
        let mut held: [HeldNotes; MAX_NOTE_SOURCES] = std::array::from_fn(|_| HeldNotes::new());
        held[0].push(60, 0.8);
        held[0].push(64, 0.8);
        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        chords.advance(&sources, &held, &levels, &mut events);

        held[0].remove(60);
        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        chords.advance(&sources, &held, &levels, &mut events);
        let offs: Vec<u8> = events[1]
            .as_slice()
            .iter()
            .filter(|e| !e.on)
            .map(|e| e.note)
            .collect();
        assert_eq!(offs, [60, 67]);

        // The other note's chord is untouched: letting it go still releases its own notes.
        held[0].remove(64);
        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        chords.advance(&sources, &held, &levels, &mut events);
        let offs: Vec<u8> = events[1]
            .as_slice()
            .iter()
            .filter(|e| !e.on)
            .map(|e| e.note)
            .collect();
        assert_eq!(offs, [64, 71]);
    }

    /// A note off the chord never saw a note on for does nothing and does not panic: nothing is
    /// held, so nothing releases.
    #[test]
    fn an_unseen_note_off_does_nothing() {
        let mut chords = Chords::new();
        let sources = node_sources(settings(ChordType::Major, 0.0));
        let levels = ControlLevels::default();
        let empty: [HeldNotes; MAX_NOTE_SOURCES] = std::array::from_fn(|_| HeldNotes::new());
        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        chords.advance(&sources, &empty, &levels, &mut events);
        assert!(events[1].as_slice().is_empty());
    }

    /// A slot that stops being a chord drops its remembered voices instead of leaving them stuck.
    #[test]
    fn a_slot_that_changes_kind_drops_its_state() {
        let mut chords = Chords::new();
        let mut sources = node_sources(settings(ChordType::Major, 0.0));
        let held = held_with(60, 0.8);
        let levels = ControlLevels::default();
        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        chords.advance(&sources, &held, &levels, &mut events);

        sources[1] = None;
        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        chords.advance(&sources, &held, &levels, &mut events);
        assert!(events[1].as_slice().is_empty());

        // Were the old voice still remembered, this would wrongly release the old chord.
        sources[1] = Some(NoteSourceSpec::Chord(settings(ChordType::Major, 0.0)));
        let empty: [HeldNotes; MAX_NOTE_SOURCES] = std::array::from_fn(|_| HeldNotes::new());
        let mut events: [NoteBuffer; MAX_NOTE_SOURCES] = std::array::from_fn(|_| NoteBuffer::new());
        chords.advance(&sources, &empty, &levels, &mut events);
        assert!(events[1].as_slice().is_empty());
    }
}
