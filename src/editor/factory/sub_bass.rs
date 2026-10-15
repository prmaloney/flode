//! Sub Bass. A mono sine and a filtered, tube-warmed saw, both an octave down.

use super::{Builder, CONTROLS, EFFECTS, OUTPUT, SHAPING, SOURCES};
use crate::dsp::wavetable::Waveform;
use crate::nodes::knobs::KnobId;
use crate::nodes::{NodeKind, Port};
use crate::presets::Preset;

/// The oscillator's sections, as `Builder::fold` numbers them.
const PITCH: usize = 0;
const UNISON: usize = 1;
const WARP: usize = 2;

/// The dropdown row of the distortion's Tube type.
const TUBE: usize = 2;

pub(super) fn build() -> Preset {
    let mut b = Builder::new();
    let input = b.voice_input(true, 0.05);
    let amp = b.envelope(CONTROLS, 0.003, 0.35, 0.85, 0.15);
    // Two oscillators are too tall to share a column, so the saw sits in the shaping column
    // above its filter, and the filter's envelope goes under the output.
    let output = b.output(2.0);
    let sweep = b.envelope(OUTPUT, 0.002, 0.22, 0.15, 0.1);

    // The sine is the weight underneath and goes straight to the output.
    let sine = b.add(NodeKind::Oscillator, SOURCES);
    b.waveform(sine, Waveform::Sine);
    b.set(sine, KnobId::Octave, -1.0);
    b.fold(sine, PITCH, false);
    b.fold(sine, UNISON, true);
    b.fold(sine, WARP, true);
    b.notes(input, sine);
    b.amplitude(amp, sine);

    // The saw is the bite. It is filtered, then warmed, and the envelope opens the filter on
    // each note.
    let saw = b.add(NodeKind::Oscillator, SHAPING);
    b.waveform(saw, Waveform::Saw);
    b.set(saw, KnobId::Octave, -1.0);
    b.set(saw, KnobId::OscGain, -9.0);
    b.fold(saw, PITCH, true);
    b.fold(saw, UNISON, true);
    b.fold(saw, WARP, true);
    b.notes(input, saw);
    b.amplitude(amp, saw);

    let filter = b.add(NodeKind::Filter, SHAPING);
    b.choose(filter, 0, 0);
    b.set(filter, KnobId::Cutoff, 450.0);
    b.set(filter, KnobId::Resonance, 0.25);
    b.audio(saw, filter);
    b.modulate(
        (sweep, Port::Shape),
        filter,
        KnobId::Cutoff,
        (200.0, 1800.0),
        EFFECTS,
    );

    let tube = b.add(NodeKind::Distortion, EFFECTS);
    b.choose(tube, 0, TUBE);
    b.set(tube, KnobId::Drive, 0.3);
    b.set(tube, KnobId::Mix, 0.4);
    b.audio(filter, tube);

    b.slot(sine, 0, output);
    b.slot(tube, 1, output);
    b.slot_gain(output, 0, 0.0);
    b.slot_gain(output, 1, -3.0);
    b.finish("Sub Bass")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::graph::Graph;

    fn graph() -> Graph {
        Graph::from_saved(&build().graph)
    }

    #[test]
    fn the_input_is_mono_and_both_oscillators_are_an_octave_down() {
        let graph = graph();
        let input = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Input)
            .unwrap();
        assert!(input.switch(0));
        let oscillators: Vec<_> = graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Oscillator)
            .collect();
        assert_eq!(oscillators.len(), 2);
        for osc in oscillators {
            assert_eq!(osc.plain(KnobId::Octave), -1.0);
        }
    }

    #[test]
    fn the_second_envelope_drives_the_filter_cutoff() {
        let graph = graph();
        let filter = graph
            .nodes
            .iter()
            .position(|n| n.kind == NodeKind::Filter)
            .unwrap();
        let map = graph
            .connections
            .iter()
            .find(|c| c.to == filter && c.input == Port::Knob(KnobId::Cutoff))
            .expect("the cutoff has a wire")
            .from;
        assert_eq!(graph.nodes[map].kind, NodeKind::Map);
        let source = graph
            .connections
            .iter()
            .find(|c| c.to == map)
            .expect("the map has a source")
            .from;
        assert_eq!(graph.nodes[source].kind, NodeKind::Envelope);
        assert!((graph.nodes[source].plain(KnobId::Decay) - 0.22).abs() < 1e-4);
    }
}
