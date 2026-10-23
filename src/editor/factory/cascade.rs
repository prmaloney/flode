//! Cascade. One key opens a sus2 chord, and an arpeggiator chained after it steps through that
//! chord up and down, so a single held note turns into a running line. The same per-wire notes
//! signal that lets a chord feed an oscillator also lets a chord feed an arpeggiator, which is
//! what this preset leans on.

use super::{Builder, CONTROLS, EFFECTS, SHAPING, SOURCES};
use crate::dsp::wavetable::Waveform;
use crate::nodes::chord::ChordType;
use crate::nodes::knobs::KnobId;
use crate::nodes::{NodeKind, Port};
use crate::presets::Preset;

/// The oscillator's Warp section, by number.
const WARP: usize = 2;
/// The step of `LfoSync` that reads "1/16".
const SYNC_SIXTEENTH: f32 = 5.0;

pub(super) fn build() -> Preset {
    let mut b = Builder::new();
    let input = b.voice_input(true, 0.0);
    // The input is set once and forgotten, so it is folded to leave room for the chord and the
    // arpeggiator stacked beneath it in the same column.
    b.graph.nodes[input].collapsed = true;

    let chord = b.add(NodeKind::Chord, CONTROLS);
    b.wire(input, Port::Notes, chord, Port::Notes);
    b.choose(chord, 0, ChordType::Sus2 as usize);

    let arp = b.add(NodeKind::Arpeggiator, SHAPING);
    b.wire(chord, Port::Notes, arp, Port::Notes);
    b.choose(arp, 0, 2); // Up-down
    b.set(arp, KnobId::LfoSync, SYNC_SIXTEENTH);
    b.set(arp, KnobId::ArpOctaves, 2.0);
    b.set(arp, KnobId::ArpGate, 0.7);

    let envelope = b.envelope(CONTROLS, 0.002, 0.2, 0.0, 0.1);

    let osc = b.add(NodeKind::Oscillator, SOURCES);
    b.waveform(osc, Waveform::Saw);
    b.set(osc, KnobId::Unison, 2.0);
    b.set(osc, KnobId::Detune, 10.0);
    b.fold(osc, WARP, true);
    b.wire(arp, Port::Notes, osc, Port::Notes);
    b.amplitude(envelope, osc);

    let filter = b.add(NodeKind::Filter, SHAPING);
    b.set(filter, KnobId::Cutoff, 2000.0);
    b.set(filter, KnobId::Resonance, 0.3);
    b.audio(osc, filter);

    let delay = b.add(NodeKind::Delay, EFFECTS);
    b.set(delay, KnobId::LfoSync, 4.0); // 1/8, against the arp's 1/16 for movement
    b.set(delay, KnobId::Feedback, 0.4);
    b.set(delay, KnobId::Mix, 0.35);
    b.switch(delay, 0, true);
    b.audio(filter, delay);

    let gain = b.add(NodeKind::Gain, EFFECTS);
    b.set(gain, KnobId::UtilGain, 6.0);
    b.audio(delay, gain);

    let output = b.output(0.0);
    b.slot(gain, 0, output);
    b.finish("Cascade")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::graph::Graph;
    use crate::nodes::arpeggiator::{pattern, ArpPattern};
    use crate::nodes::chord::{chord_type, ChordType as CT};

    fn graph() -> Graph {
        Graph::from_saved(&build().graph)
    }

    /// The chord feeds the arpeggiator, and the arpeggiator feeds the oscillator: neither reads
    /// notes straight from the MIDI input.
    #[test]
    fn the_note_sources_chain_input_to_chord_to_arpeggiator_to_oscillator() {
        let graph = graph();
        let feeder_of = |to: usize| {
            graph
                .connections
                .iter()
                .find(|c| c.to == to && c.input == Port::Notes)
                .map(|c| (c.from, graph.nodes[c.from].kind))
        };
        let osc = graph
            .nodes
            .iter()
            .position(|n| n.kind == NodeKind::Oscillator)
            .unwrap();
        let (arp, arp_kind) = feeder_of(osc).expect("the oscillator reads notes from somewhere");
        assert_eq!(arp_kind, NodeKind::Arpeggiator);
        let (_, chord_kind) = feeder_of(arp).expect("the arpeggiator reads notes from somewhere");
        assert_eq!(chord_kind, NodeKind::Chord);
    }

    #[test]
    fn the_chord_is_a_sus2_and_the_pattern_is_up_down() {
        let graph = graph();
        let chord = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Chord)
            .unwrap();
        assert_eq!(chord_type(chord), CT::Sus2);
        let arp = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Arpeggiator)
            .unwrap();
        assert_eq!(pattern(arp), ArpPattern::UpDown);
    }
}
