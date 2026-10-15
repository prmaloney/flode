//! One output slot: its voices and its smoothed gain.

use super::voice::Voice;

/// The most notes that can sound at once in one output slot.
pub const NUM_VOICES: usize = 16;

pub struct Lane {
    pub(super) voices: [Option<Voice>; NUM_VOICES],
    /// The slot's gain, easing toward the knob so a move does not click.
    gain: f32,
    /// False until the gain has a first value. It then starts at the knob instead of easing in
    /// from zero.
    settled: bool,
}

impl Lane {
    pub fn new() -> Self {
        Self {
            voices: std::array::from_fn(|_| None),
            gain: 0.0,
            settled: false,
        }
    }

    /// Drops every voice.
    pub fn clear(&mut self) {
        self.voices = std::array::from_fn(|_| None);
    }

    pub fn voices(&self) -> impl Iterator<Item = &Voice> {
        self.voices.iter().flatten()
    }

    pub fn voices_mut(&mut self) -> impl Iterator<Item = &mut Voice> {
        self.voices.iter_mut().flatten()
    }

    pub fn is_sounding(&self) -> bool {
        self.voices().next().is_some()
    }

    /// Where a new voice goes: a free slot, then a voice already fading out, then the oldest.
    pub fn voice_slot(&self) -> usize {
        self.voices
            .iter()
            .position(Option::is_none)
            .or_else(|| {
                self.voices
                    .iter()
                    .position(|voice| voice.as_ref().is_some_and(Voice::releasing))
            })
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, voice)| voice.as_ref().map_or(u64::MAX, |v| v.age))
                    .map_or(0, |(index, _)| index)
            })
    }

    /// Eases the gain toward `target` and returns it.
    pub fn ease_gain(&mut self, target: f32, smooth: f32) -> f32 {
        if self.settled {
            self.gain += (target - self.gain) * smooth;
        } else {
            self.gain = target;
            self.settled = true;
        }
        self.gain
    }

    /// Called while the slot plays nothing, so its gain starts fresh the next time.
    pub fn go_quiet(&mut self) {
        self.settled = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::stage::StageState;
    use crate::patch::stage::MAX_STAGES;

    fn voice(age: u64) -> Voice {
        Voice::new(60, 1.0, 0.01, 0.01, age, [StageState::Empty; MAX_STAGES])
    }

    #[test]
    fn a_free_slot_is_preferred() {
        let mut lane = Lane::new();
        lane.voices[0] = Some(voice(1));
        assert_eq!(lane.voice_slot(), 1);
    }

    /// With every slot taken, a voice that is already fading goes first.
    #[test]
    fn a_fading_voice_is_stolen_before_a_held_one() {
        let mut lane = Lane::new();
        for (i, slot) in lane.voices.iter_mut().enumerate() {
            *slot = Some(voice(i as u64 + 1));
        }
        lane.voices[5]
            .as_mut()
            .unwrap()
            .envs
            .iter_mut()
            .for_each(|e| e.release(0.1, 1000.0));
        assert_eq!(lane.voice_slot(), 5);
    }

    /// With none fading, the oldest voice goes.
    #[test]
    fn the_oldest_voice_is_stolen_last() {
        let mut lane = Lane::new();
        for (i, slot) in lane.voices.iter_mut().enumerate() {
            *slot = Some(voice(100 - i as u64));
        }
        assert_eq!(lane.voice_slot(), NUM_VOICES - 1);
    }

    #[test]
    fn the_gain_starts_at_its_target_and_then_eases() {
        let mut lane = Lane::new();
        assert_eq!(lane.ease_gain(1.0, 0.1), 1.0);
        let next = lane.ease_gain(0.0, 0.1);
        assert!((next - 0.9).abs() < 1e-6);
        lane.go_quiet();
        assert_eq!(lane.ease_gain(0.5, 0.1), 0.5);
    }
}
