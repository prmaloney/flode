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
}
