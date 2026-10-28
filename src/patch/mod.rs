//! What the editor hands the audio thread: the patch, as plain data the engine can run. The
//! editor compiles its graph into a `Patch` whenever the graph changes and publishes it through a
//! `PatchLink`. The audio thread reads the newest one at the start of each block.

pub mod control;
pub mod flatten;
pub mod link;
pub mod stage;
pub mod tables;
pub mod wiring;

use crate::dsp::curve::Curve;
use crate::dsp::wavetable::Waveform;
use crate::nodes::arpeggiator::ArpPattern;
use crate::nodes::chord::ChordType;
use crate::nodes::knobs::KnobId;
use crate::nodes::output::MAX_SLOTS;
use control::{Param, SignalTable, MAX_ENVELOPES, MAX_LFOS, MAX_NOTE_SOURCES};
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
    /// Restart the cycle at every note on. It applies to a free running LFO. A synced LFO follows
    /// the playhead and ignores it.
    pub retrigger: bool,
    /// Play one cycle from each note on, then hold the end value until the next note. It restarts
    /// at every note whatever `retrigger` says, and it ignores the playhead lock of Sync.
    pub one_shot: bool,
    /// The drawn wave. Some only when the shape is Custom, and then `waveform` is not read.
    pub curve: Option<Curve>,
}

impl LfoSettings {
    pub fn new() -> Self {
        Self {
            waveform: Waveform::Sine,
            rate: Param::default_of(KnobId::LfoRate),
            sync: Param::default_of(KnobId::LfoSync),
            retrigger: false,
            one_shot: false,
            curve: None,
        }
    }
}

impl Default for LfoSettings {
    fn default() -> Self {
        Self::new()
    }
}

/// An arpeggiator node's settings. It reads the chord held at its own `input` note-source slot and
/// re-emits a pattern of its own, which other note sources can read the same way they would a MIDI
/// input's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArpeggiatorSettings {
    /// The note-source slot that feeds this arpeggiator's chord, if its Notes pin is wired to one.
    /// `None` means nothing feeds it, so it plays no chord and emits nothing.
    pub input: Option<usize>,
    pub pattern: ArpPattern,
    /// Free running in Hz, or synced to the host tempo through `sync`, the same pair as an LFO's
    /// Rate and Sync.
    pub rate: Param,
    pub sync: Param,
    /// How many octaves the pattern climbs before it cycles back, from 1 to 4.
    pub octaves: Param,
    /// How much of each step's duration the note sounds before its note off, from 0.0 to 1.0.
    pub gate: Param,
}

/// A chord node's settings. It reads the notes held at its own `input` note-source slot and turns
/// each one into a stack of notes, the root plus the chosen chord's intervals, which other note
/// sources can read the same way they would a MIDI input's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChordSettings {
    /// The note-source slot that feeds this chord, if its Notes pin is wired to one. `None` means
    /// nothing feeds it, so it plays no notes.
    pub input: Option<usize>,
    pub chord: ChordType,
    /// How far the chord's notes climb apart, from 0.0 to 1.0. See `ChordType` and the Chord node
    /// for what it does to each interval.
    pub spread: Param,
}

/// One slot in the note-routing table: a node in the graph that emits notes, in the order the
/// compiler found them. A downstream oscillator's `notes` field names a slot here instead of the
/// boolean "is a MIDI input wired in" the compiler used to carry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoteSourceSpec {
    /// A MIDI input node, which passes the host's notes straight through. Every `Input` node in the
    /// patch compiles to one of these, and they all carry the same host events today, since there
    /// is only one physical note source: the host, or the key strip in standalone. That is
    /// intentional. Giving each Input its own slot is what lets a transform sitting on one Input's
    /// output leave another Input's own state alone; it is not an attempt to filter by channel.
    Input,
    /// An arpeggiator, reading the chord held at its own slot and emitting a pattern of its own.
    Arpeggiator(ArpeggiatorSettings),
    /// A chord, reading the notes held at its own slot and emitting a stack of notes for each one.
    Chord(ChordSettings),
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
    /// The note-emitting nodes in graph order, or None past the last one. Built the compile-order
    /// way `envelopes` and `lfos` are, with the same limitation: deleting an earlier node can
    /// reassign a later one's slot, since nodes have no stable id that survives an edit.
    pub note_sources: [Option<NoteSourceSpec>; MAX_NOTE_SOURCES],
    pub globals: Globals,
    /// The map and math nodes, in the order to run them. A route can read any of them as a source.
    pub signals: SignalTable,
}

impl Clone for Patch {
    fn clone(&self) -> Self {
        Self {
            lanes: self.lanes.clone(),
            envelopes: self.envelopes,
            lfos: self.lfos,
            note_sources: self.note_sources,
            globals: self.globals,
            signals: self.signals,
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
        self.note_sources = source.note_sources;
        self.globals = source.globals;
        self.signals = source.signals;
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
            note_sources: [None; MAX_NOTE_SOURCES],
            globals: Globals::default(),
            signals: SignalTable::EMPTY,
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

    /// The audio thread copies the patch every block, on a stack of about 2 MB that the rest of
    /// the process call shares. A patch is about 73 KB. The copy has to go straight into the
    /// target, since a temporary patch for it to be built in would overflow a stack this small.
    /// The patches sit on the heap here, so only the copy itself uses the thread's stack.
    #[test]
    fn copying_a_patch_into_another_needs_little_stack() {
        let source = Box::new(Patch::silent());
        let mut target = Box::new(Patch::silent());
        target.lanes[0].gain = Param::fixed(crate::nodes::knobs::KnobId::Slot1, -6.0);
        let target = std::thread::Builder::new()
            .stack_size(96 * 1024)
            .spawn(move || {
                target.clone_from(&source);
                (target, source)
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(target.0, target.1);
    }
}
