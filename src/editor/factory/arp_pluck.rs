//! Arp Pluck. A short square pluck stepped by an arpeggiator synced to 1/16 notes, bounced through
//! a ping-pong delay so the steps land on alternating sides.

use super::{Builder, CONTROLS, EFFECTS, SHAPING, SOURCES};
use crate::dsp::wavetable::Waveform;
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
    // The input is set once and forgotten, so it is folded to leave room in its column.
    b.graph.nodes[input].collapsed = true;

    let arp = b.add(NodeKind::Arpeggiator, CONTROLS);
    b.wire(input, Port::Notes, arp, Port::Notes);
    b.set(arp, KnobId::LfoSync, SYNC_SIXTEENTH);
    b.set(arp, KnobId::ArpOctaves, 2.0);
    b.set(arp, KnobId::ArpGate, 0.55);

    // Fully closed, with no sustain, so every step is a clean, separated pluck.
    let envelope = b.envelope(CONTROLS, 0.001, 0.1, 0.0, 0.05);

    let osc = b.add(NodeKind::Oscillator, SOURCES);
    b.waveform(osc, Waveform::Square);
    b.set(osc, KnobId::Unison, 2.0);
    b.set(osc, KnobId::Detune, 8.0);
    b.set(osc, KnobId::Spread, 0.3);
    b.fold(osc, WARP, true);
    b.wire(arp, Port::Notes, osc, Port::Notes);
    b.amplitude(envelope, osc);

    let filter = b.add(NodeKind::Filter, SHAPING);
    b.set(filter, KnobId::Cutoff, 2600.0);
    b.set(filter, KnobId::Resonance, 0.35);
    b.audio(osc, filter);

    // The delay repeats the same 1/16 grid the arpeggiator steps on, so the echoes feel like part
    // of the pattern instead of a wash behind it.
    let delay = b.add(NodeKind::Delay, EFFECTS);
    b.set(delay, KnobId::LfoSync, SYNC_SIXTEENTH);
    b.set(delay, KnobId::Feedback, 0.3);
    b.set(delay, KnobId::Mix, 0.3);
    b.switch(delay, 0, true);
    b.audio(filter, delay);

    let gain = b.add(NodeKind::Gain, EFFECTS);
    b.set(gain, KnobId::UtilGain, 3.0);
    b.audio(delay, gain);

    let output = b.output(0.0);
    b.slot(gain, 0, output);
    b.finish("Arp Pluck")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::graph::Graph;

    fn graph() -> Graph {
        Graph::from_saved(&build().graph)
    }

    #[test]
    fn the_oscillator_reads_notes_from_the_arpeggiator_not_the_input() {
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
        assert_eq!(feeder, Some(NodeKind::Arpeggiator));
    }

    #[test]
    fn the_arpeggiator_is_synced_and_wide() {
        let graph = graph();
        let arp = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Arpeggiator)
            .unwrap();
        assert_eq!(arp.plain(KnobId::LfoSync), SYNC_SIXTEENTH);
        assert_eq!(arp.plain(KnobId::ArpOctaves), 2.0);
    }

    #[test]
    fn the_delay_bounces_between_sides() {
        let graph = graph();
        let delay = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Delay)
            .unwrap();
        assert!(delay.switch(0), "the delay is not ping-ponging");
    }
}
