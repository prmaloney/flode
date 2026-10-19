//! The two channels between the editor and the audio thread. Each is a triple buffer: one side
//! writes whenever it likes, the other reads the newest value whenever it likes, and neither ever
//! waits for the other. The audio thread never locks or allocates.
//!
//! The editor and the plugin's setup both publish patches, and the editor reads meters, so those
//! ends sit behind a mutex that only those non-audio threads take. The audio thread owns its ends
//! outright.

use super::control::ControlLevels;
use super::Patch;
use crate::dsp::analyzer::{BINS, FLOOR_DB};
use crate::engine::POOL;
use std::sync::{Arc, Mutex};
use triple_buffer::{triple_buffer, Input, Output};

/// What the audio thread reports back, for the editor to show.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Meters {
    /// The live level of every control source, shown on the knobs they drive.
    pub levels: ControlLevels,
    /// The notes with a voice sounding, as two 64-bit words, shown on the key strip.
    pub active_notes: [u64; 2],
    /// The loudest left and right output sample of the last block, as linear amplitude, shown on
    /// the output meter.
    pub output: [f32; 2],
    /// How many note-ons the audio thread has received, from the host or the key strip. It wraps.
    /// The editor lights its MIDI indicator when this changes, so it shows that notes arrive
    /// whatever the patch does with them.
    pub notes_in: u32,
    /// What each EQ instance measured, in dB per band, by pool number. An EQ node whose analyzer
    /// is on draws its instance's.
    pub spectra: [[f32; BINS]; POOL],
}

impl Default for Meters {
    fn default() -> Self {
        Self {
            levels: ControlLevels::default(),
            active_notes: [0; 2],
            output: [0.0; 2],
            notes_in: 0,
            spectra: [[FLOOR_DB; BINS]; POOL],
        }
    }
}

/// The editor's ends of the two channels.
pub struct PatchLink {
    patches: Mutex<Input<Patch>>,
    meters: Mutex<Output<Meters>>,
}

/// The audio thread's ends of the two channels.
pub struct AudioEnd {
    pub patches: Output<Patch>,
    pub meters: Input<Meters>,
}

impl PatchLink {
    /// Makes both channels, and returns the editor's ends and the audio thread's.
    pub fn new() -> (Arc<PatchLink>, AudioEnd) {
        let (patch_in, patch_out) = triple_buffer(&Patch::silent());
        let (meters_in, meters_out) = triple_buffer(&Meters::default());
        let link = Arc::new(PatchLink {
            patches: Mutex::new(patch_in),
            meters: Mutex::new(meters_out),
        });
        let audio = AudioEnd {
            patches: patch_out,
            meters: meters_in,
        };
        (link, audio)
    }

    /// Sends a patch to the audio thread, which picks it up at the start of its next block.
    pub fn publish(&self, patch: Patch) {
        if let Ok(mut patches) = self.patches.lock() {
            patches.write(patch);
        }
    }

    /// The newest meters the audio thread wrote.
    pub fn meters(&self) -> Meters {
        match self.meters.lock() {
            Ok(mut meters) => *meters.read(),
            Err(_) => Meters::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::knobs::KnobId;
    use crate::patch::control::Param;
    use crate::patch::Globals;

    /// The audio end starts on a silent patch, and then sees what the editor publishes.
    #[test]
    fn the_audio_end_reads_the_newest_published_patch() {
        let (link, mut audio) = PatchLink::new();
        assert!(!audio.patches.read().is_audible());

        let mut patch = Patch::silent();
        patch.globals.mono = true;
        link.publish(patch.clone());
        assert!(audio.patches.read().globals.mono);

        patch.globals.glide_always = true;
        link.publish(patch);
        assert!(audio.patches.read().globals.glide_always);
    }

    /// Several publishes between two reads leave only the newest.
    #[test]
    fn only_the_latest_of_several_publishes_is_read() {
        let (link, mut audio) = PatchLink::new();
        for glide in [0.1, 0.2, 0.3] {
            let mut patch = Patch::silent();
            patch.globals.glide = Param::fixed(KnobId::Glide, glide);
            link.publish(patch);
        }
        let read = audio.patches.read().globals.glide;
        let levels = ControlLevels::default();
        assert!((read.plain(&levels) - 0.3).abs() < 1e-3);
    }

    #[test]
    fn the_editor_reads_the_meters_the_audio_thread_wrote() {
        let (link, mut audio) = PatchLink::new();
        assert_eq!(link.meters(), Meters::default());
        audio.meters.write(Meters {
            active_notes: [0b101, 0],
            ..Meters::default()
        });
        assert_eq!(link.meters().active_notes, [0b101, 0]);
    }

    /// A writer on one thread and a reader on another never see a torn patch.
    #[test]
    fn a_reader_on_another_thread_never_sees_a_torn_patch() {
        let (link, mut audio) = PatchLink::new();
        let writer = std::thread::spawn(move || {
            for n in 0..2000u32 {
                let mut patch = Patch::silent();
                // The same value goes in two places, so a torn read would show a mismatch.
                let level = (n % 30) as f32;
                patch.globals.gain = Param::fixed(KnobId::Gain, -level);
                patch.globals.glide = Param::fixed(KnobId::Glide, level / 30.0);
                link.publish(patch);
            }
        });
        let levels = ControlLevels::default();
        for _ in 0..2000 {
            let globals = audio.patches.read().globals;
            // Before the first publish the reader sees the silent patch, which is not torn.
            if globals == Globals::default() {
                continue;
            }
            let gain = globals.gain.plain(&levels);
            let glide = globals.glide.plain(&levels);
            assert!((gain.abs() / 30.0 - glide).abs() < 0.02, "{gain} {glide}");
        }
        writer.join().unwrap();
    }
}
