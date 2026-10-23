//! Chord Stab. One key triggers a full dominant 7th, kept tight and punchy with a fast decay and a
//! touch of tube drive. The input is polyphonic, since the chord plays several notes from a single
//! key press.

use super::{Builder, CONTROLS, EFFECTS, OUTPUT, SHAPING, SOURCES};
use crate::dsp::wavetable::Waveform;
use crate::nodes::chord::ChordType;
use crate::nodes::knobs::KnobId;
use crate::nodes::{NodeKind, Port};
use crate::presets::Preset;

/// The oscillator's Warp section, by number.
const WARP: usize = 2;
/// The row of "Tube" on the distortion's type dropdown.
const TUBE: usize = 2;

pub(super) fn build() -> Preset {
    let mut b = Builder::new();
    let input = b.voice_input(false, 0.0);

    let chord = b.add(NodeKind::Chord, CONTROLS);
    b.wire(input, Port::Notes, chord, Port::Notes);
    b.choose(chord, 0, ChordType::Dominant7 as usize);

    // Short and percussive, with no sustain, so the stack reads as one hit.
    let envelope = b.envelope(CONTROLS, 0.001, 0.15, 0.0, 0.05);

    let osc = b.add(NodeKind::Oscillator, SOURCES);
    b.waveform(osc, Waveform::Saw);
    b.set(osc, KnobId::Unison, 3.0);
    b.set(osc, KnobId::Detune, 15.0);
    b.set(osc, KnobId::Spread, 0.4);
    b.fold(osc, WARP, true);
    b.wire(chord, Port::Notes, osc, Port::Notes);
    b.amplitude(envelope, osc);

    let filter = b.add(NodeKind::Filter, SHAPING);
    b.set(filter, KnobId::Cutoff, 3200.0);
    b.set(filter, KnobId::Resonance, 0.25);
    b.audio(osc, filter);

    let drive = b.add(NodeKind::Distortion, EFFECTS);
    b.choose(drive, 0, TUBE);
    b.set(drive, KnobId::Drive, 0.3);
    b.set(drive, KnobId::Mix, 0.4);
    b.audio(filter, drive);

    let compressor = b.add(NodeKind::Compressor, OUTPUT);
    b.set(compressor, KnobId::Threshold, -14.0);
    b.set(compressor, KnobId::Ratio, 4.0);
    b.set(compressor, KnobId::Makeup, 3.0);
    b.audio(drive, compressor);

    let output = b.output(-6.0);
    b.slot(compressor, 0, output);
    b.finish("Chord Stab")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::graph::Graph;
    use crate::nodes::chord::chord_type;

    fn graph() -> Graph {
        Graph::from_saved(&build().graph)
    }

    #[test]
    fn the_oscillator_reads_notes_from_the_chord_not_the_input() {
        let graph = graph();
        let osc = graph
            .nodes
            .iter()
            .position(|n| n.kind == NodeKind::Oscillator)
            .unwrap();
        let feeder = graph
            .connections
            .iter()
            .find(|c| c.to == osc && c.input == Port::Notes)
            .map(|c| graph.nodes[c.from].kind);
        assert_eq!(feeder, Some(NodeKind::Chord));
    }

    #[test]
    fn the_chord_is_a_dominant_seventh() {
        let graph = graph();
        let chord = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Chord)
            .unwrap();
        assert_eq!(chord_type(chord), ChordType::Dominant7);
    }

    #[test]
    fn the_input_is_polyphonic() {
        let graph = graph();
        let input = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Input)
            .unwrap();
        assert!(!input.switch(0), "a chord needs more than one voice");
    }
}
