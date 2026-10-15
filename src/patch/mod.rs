//! What the editor hands the audio thread: the patch, as plain data the engine can run. The
//! editor compiles its graph into a `Patch` whenever the graph changes and publishes it through a
//! `PatchLink`. The audio thread reads the newest one at the start of each block.

pub mod control;
pub mod link;
pub mod stage;
pub mod wiring;

use crate::dsp::wavetable::Waveform;
use crate::nodes::knobs::KnobId;
use crate::nodes::output::MAX_SLOTS;
use control::{Param, MAX_ENVELOPES, MAX_LFOS};
use stage::LaneGraph;

/// An envelope node's settings. Each voice's copy of the envelope is set up from these once per
/// block, so a control wire can drive the times.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvelopeSettings {
    pub attack: Param,
    pub decay: Param,
    pub sustain: Param,
    pub release: Param,
}

impl EnvelopeSettings {
    /// An envelope at its knobs' default values.
    pub fn new() -> Self {
        Self {
            attack: Param::default_of(KnobId::Attack),
            decay: Param::default_of(KnobId::Decay),
            sustain: Param::default_of(KnobId::Sustain),
            release: Param::default_of(KnobId::Release),
        }
    }
}

impl Default for EnvelopeSettings {
    fn default() -> Self {
        Self::new()
    }
}

/// An LFO's settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LfoSettings {
    pub waveform: Waveform,
    /// The rate knob, in cycles per second. A control wire can drive it.
    pub rate: Param,
    /// The Sync knob. Zero is free running, and each step past it is a note division.
    pub sync: Param,
}

impl LfoSettings {
    pub fn new() -> Self {
        Self {
            waveform: Waveform::Sine,
            rate: Param::default_of(KnobId::LfoRate),
            sync: Param::default_of(KnobId::LfoSync),
        }
    }
}

impl Default for LfoSettings {
    fn default() -> Self {
        Self::new()
    }
}

/// The output node's settings that apply to every slot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Globals {
    pub mono: bool,
    /// Glide on every mono note, not only while another note is still held.
    pub glide_always: bool,
    pub gain: Param,
    pub glide: Param,
}

impl Default for Globals {
    fn default() -> Self {
        Self {
            mono: false,
            glide_always: false,
            gain: Param::default_of(KnobId::Gain),
            glide: Param::default_of(KnobId::Glide),
        }
    }
}

/// Everything the audio thread needs from the graph.
#[derive(Debug, PartialEq)]
pub struct Patch {
    /// The stages behind each output slot. A slot with no stages is silent.
    pub lanes: [LaneGraph; MAX_SLOTS],
    /// The envelope nodes in graph order, or None past the last one. Each voice runs a copy of
    /// every envelope that is there.
    pub envelopes: [Option<EnvelopeSettings>; MAX_ENVELOPES],
    /// The LFO nodes in graph order, or None past the last one.
    pub lfos: [Option<LfoSettings>; MAX_LFOS],
    pub globals: Globals,
}

impl Clone for Patch {
    fn clone(&self) -> Self {
        Self {
            lanes: self.lanes.clone(),
            envelopes: self.envelopes,
            lfos: self.lfos,
            globals: self.globals,
        }
    }

    /// Copies field by field into `self`. The audio thread calls this every block, and the default
    /// builds a whole patch on the stack first, which is too big for a 2 MB audio stack in a debug
    /// build.
    fn clone_from(&mut self, source: &Self) {
        for (lane, from) in self.lanes.iter_mut().zip(&source.lanes) {
            lane.clone_from(from);
        }
        self.envelopes = source.envelopes;
        self.lfos = source.lfos;
        self.globals = source.globals;
    }
}

impl Patch {
    /// A patch that makes no sound.
    pub fn silent() -> Self {
        Self {
            lanes: std::array::from_fn(|slot| {
                LaneGraph::silent(Param::default_of(crate::nodes::output::slot_knob(slot)))
            }),
            envelopes: [None; MAX_ENVELOPES],
            lfos: [None; MAX_LFOS],
            globals: Globals::default(),
        }
    }

    /// True if any slot has a stage to play.
    pub fn is_audible(&self) -> bool {
        self.lanes.iter().any(LaneGraph::is_active)
    }

    /// True while any envelope node is in the patch. Then the amplitude is the envelopes' job.
    pub fn has_envelopes(&self) -> bool {
        self.envelopes.iter().any(Option::is_some)
    }
}

impl Default for Patch {
    fn default() -> Self {
        Self::silent()
    }
}

#[cfg(test)]
mod clone_tests {
    use super::*;

    /// The audio thread copies the patch every block on a stack of about 2 MB, shared with the
    /// whole process call. Copying in place needs almost none of it, so a thread with a small
    /// stack can do it. A copy that builds a temporary patch first would overflow here.
    #[test]
    fn copying_a_patch_into_another_needs_little_stack() {
        let source = Patch::silent();
        let copied = std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(move || {
                let mut target = Patch::silent();
                target.clone_from(&source);
                target == source
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(copied);
    }
}
