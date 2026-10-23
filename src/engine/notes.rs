//! The notes currently held down.

/// Notes held down, oldest first, with their velocity. Mono mode recalls from the end.
pub struct HeldNotes {
    notes: [(u8, f32); 128],
    len: usize,
}

impl HeldNotes {
    pub const fn new() -> Self {
        Self {
            notes: [(0, 0.0); 128],
            len: 0,
        }
    }

    /// Adds a note to the end. A repeat moves it to the end.
    pub fn push(&mut self, note: u8, velocity: f32) {
        self.remove(note);
        self.notes[self.len] = (note, velocity);
        self.len += 1;
    }

    pub fn remove(&mut self, note: u8) {
        if let Some(index) = self.notes[..self.len]
            .iter()
            .position(|&(held, _)| held == note)
        {
            self.notes.copy_within(index + 1..self.len, index);
            self.len -= 1;
        }
    }

    /// The most recently pressed note that is still down.
    pub fn newest(&self) -> Option<(u8, f32)> {
        self.len.checked_sub(1).map(|last| self.notes[last])
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Every note held down, oldest first, with its velocity. This is the chord an arpeggiator
    /// reads, already in "as played" order.
    pub fn notes(&self) -> &[(u8, f32)] {
        &self.notes[..self.len]
    }
}

/// The most events one block's worth of a Notes wire can carry. A fast arpeggiator step emits a
/// note off and a note on close together, so this leaves plenty of headroom above what a block of
/// a few hundred samples could realistically need, while staying small on the stack.
pub const NOTE_BUFFER_CAPACITY: usize = 32;

/// One note changing on a Notes wire: a note turning on or off, at a sample offset within the
/// block it happened in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteEvent {
    /// The sample within the block the event happens at. The engine dispatches host notes and
    /// arpeggiator steps sample-accurately as it reaches them, so in practice every event a caller
    /// builds today carries the offset of the sample it is dispatched on; the field is here so a
    /// batched caller could fill a whole block of them at once.
    pub sample_offset: u16,
    pub note: u8,
    pub velocity: f32,
    pub on: bool,
}

/// The signal on a Notes wire for one block: a fixed-capacity list of note events, analogous to how
/// a `Frame` is what flows on an audio wire. It holds no heap allocation, so it is safe to build on
/// the audio thread. An event past the capacity is dropped; nothing sane plays that many note
/// changes in one block.
#[derive(Debug, Clone, Copy)]
pub struct NoteBuffer {
    events: [NoteEvent; NOTE_BUFFER_CAPACITY],
    len: usize,
}

impl NoteBuffer {
    pub const EMPTY_EVENT: NoteEvent = NoteEvent {
        sample_offset: 0,
        note: 0,
        velocity: 0.0,
        on: false,
    };

    pub const fn new() -> Self {
        Self {
            events: [Self::EMPTY_EVENT; NOTE_BUFFER_CAPACITY],
            len: 0,
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Adds an event at the end. Does nothing once the buffer is full.
    pub fn push(&mut self, event: NoteEvent) {
        if self.len < NOTE_BUFFER_CAPACITY {
            self.events[self.len] = event;
            self.len += 1;
        }
    }

    pub fn as_slice(&self) -> &[NoteEvent] {
        &self.events[..self.len]
    }
}

impl Default for NoteBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_note_is_the_last_one_pressed() {
        let mut held = HeldNotes::new();
        assert_eq!(held.newest(), None);
        held.push(60, 0.5);
        held.push(64, 0.7);
        assert_eq!(held.newest(), Some((64, 0.7)));
    }

    /// Releasing the newest note recalls the one before it.
    #[test]
    fn removing_the_newest_recalls_the_previous() {
        let mut held = HeldNotes::new();
        held.push(60, 0.5);
        held.push(64, 0.7);
        held.remove(64);
        assert_eq!(held.newest(), Some((60, 0.5)));
    }

    /// Pressing a note that is already down moves it to the end instead of listing it twice.
    #[test]
    fn a_repeat_moves_to_the_end() {
        let mut held = HeldNotes::new();
        held.push(60, 0.5);
        held.push(64, 0.7);
        held.push(60, 0.9);
        assert_eq!(held.newest(), Some((60, 0.9)));
        held.remove(60);
        assert_eq!(held.newest(), Some((64, 0.7)));
    }

    #[test]
    fn removing_a_note_that_is_not_held_changes_nothing() {
        let mut held = HeldNotes::new();
        held.push(60, 0.5);
        held.remove(72);
        assert_eq!(held.newest(), Some((60, 0.5)));
        held.clear();
        assert_eq!(held.newest(), None);
    }

    /// `notes()` lists the chord oldest first, which is the order an arpeggiator reads.
    #[test]
    fn notes_lists_the_chord_oldest_first() {
        let mut held = HeldNotes::new();
        held.push(60, 0.5);
        held.push(64, 0.6);
        held.push(67, 0.7);
        assert_eq!(held.notes(), &[(60, 0.5), (64, 0.6), (67, 0.7)]);
        held.remove(64);
        assert_eq!(held.notes(), &[(60, 0.5), (67, 0.7)]);
    }

    #[test]
    fn a_note_buffer_drops_events_past_its_capacity() {
        let mut buffer = NoteBuffer::new();
        for note in 0..NOTE_BUFFER_CAPACITY + 5 {
            buffer.push(NoteEvent {
                sample_offset: 0,
                note: note as u8,
                velocity: 1.0,
                on: true,
            });
        }
        assert_eq!(buffer.as_slice().len(), NOTE_BUFFER_CAPACITY);
        assert_eq!(buffer.as_slice()[0].note, 0);
    }
}
