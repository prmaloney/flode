//! Chord Swell. A minor 7th from one held key, spread into an open voicing, with a slow filter
//! sweep. A compressor tames the extra voices a full chord adds over a single note, so three held
//! keys swell without clipping.

use super::{Builder, CONTROLS, EFFECTS, OUTPUT, SHAPING, SOURCES};
use crate::dsp::wavetable::Waveform;
use crate::nodes::chord::ChordType;
use crate::nodes::knobs::KnobId;
use crate::nodes::{NodeKind, Port};
use crate::presets::Preset;

pub(super) fn build() -> Preset {
    let mut b = Builder::new();
    let input = b.voice_input(false, 0.0);

    let chord = b.add(NodeKind::Chord, CONTROLS);
    b.wire(input, Port::Notes, chord, Port::Notes);
    b.choose(chord, 0, ChordType::Minor7 as usize);
    // Spread the stack across octaves instead of stacking it tight, for an open, airy voicing.
    b.set(chord, KnobId::ChordSpread, 0.3);

    let envelope = b.envelope(CONTROLS, 1.2, 1.0, 0.85, 2.0);

    let osc = b.add(NodeKind::Oscillator, SOURCES);
    b.waveform(osc, Waveform::Triangle);
    b.set(osc, KnobId::Unison, 3.0);
    b.set(osc, KnobId::Detune, 15.0);
    b.set(osc, KnobId::Spread, 0.6);
    b.wire(chord, Port::Notes, osc, Port::Notes);
    b.amplitude(envelope, osc);

    let filter = b.add(NodeKind::Filter, SHAPING);
    b.set(filter, KnobId::Resonance, 0.2);
    b.audio(osc, filter);
    let cutoff_lfo = b.add(NodeKind::Lfo, SHAPING);
    b.set(cutoff_lfo, KnobId::LfoRate, 0.08);
    let cutoff_map = b.modulate(
        (cutoff_lfo, Port::Shape),
        filter,
        KnobId::Cutoff,
        (700.0, 3000.0),
        EFFECTS,
    );
    b.graph.nodes[cutoff_map].collapsed = true;

    // A single note sits under the threshold and plays untouched; a full chord's extra voices
    // push it over, where the compressor brings them back down.
    let compressor = b.add(NodeKind::Compressor, EFFECTS);
    b.set(compressor, KnobId::Threshold, -16.0);
    b.set(compressor, KnobId::Ratio, 5.0);
    b.set(compressor, KnobId::Makeup, 2.0);
    b.audio(filter, compressor);

    let reverb = b.add(NodeKind::Reverb, OUTPUT);
    b.set(reverb, KnobId::ReverbDecay, 4.5);
    b.set(reverb, KnobId::Size, 0.7);
    b.set(reverb, KnobId::Mix, 0.3);
    b.audio(compressor, reverb);

    let output = b.output(-4.0);
    b.slot(reverb, 0, output);
    b.finish("Chord Swell")
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
    fn the_chord_is_a_spread_minor_seventh() {
        let graph = graph();
        let chord = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Chord)
            .unwrap();
        assert_eq!(chord_type(chord), ChordType::Minor7);
        assert!(chord.plain(KnobId::ChordSpread) > 0.0);
    }

    #[test]
    fn an_lfo_drives_the_cutoff_before_the_compressor() {
        let graph = graph();
        let driven = graph.connections.iter().any(|c| {
            graph.nodes[c.to].kind == NodeKind::Filter
                && c.input == Port::Knob(KnobId::Cutoff)
                && graph.connections.iter().any(|m| {
                    m.to == c.from
                        && m.input == Port::Shape
                        && graph.nodes[m.from].kind == NodeKind::Lfo
                })
        });
        assert!(driven, "no LFO on the cutoff");
        let compressor = graph
            .nodes
            .iter()
            .position(|n| n.kind == NodeKind::Compressor)
            .unwrap();
        let fed_by_filter = graph
            .connections
            .iter()
            .any(|c| c.to == compressor && graph.nodes[c.from].kind == NodeKind::Filter);
        assert!(fed_by_filter, "the compressor does not follow the filter");
    }
}
