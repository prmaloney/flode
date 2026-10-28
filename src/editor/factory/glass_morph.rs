//! Glass Morph. A wide pad that drifts through four waves, from a pure sine up through a triangle
//! and a saw to a bright square and back, as a slow LFO moves the wavetable's Position. The
//! frames are plain wave nodes, shrunk to their headers beside the table they feed.

use super::{Builder, CONTROLS, EFFECTS, OUTPUT, SHAPING, SOURCES};
use crate::nodes::knobs::KnobId;
use crate::nodes::{NodeKind, Port};
use crate::presets::Preset;

/// The oscillator's Pitch, Unison, and Warp sections, by number.
const PITCH: usize = 0;
const UNISON: usize = 1;
const WARP: usize = 2;
/// The row of "Smooth" on the wavetable's Blend dropdown.
const SMOOTH: usize = 1;

pub(super) fn build() -> Preset {
    let mut b = Builder::new();
    let input = b.voice_input(false, 0.0);
    let envelope = b.envelope(CONTROLS, 0.9, 1.4, 0.9, 2.6);

    // The wavetable gets its frames in order, dull to bright, so Position runs from the sine
    // at 0 to the square at 1. Smooth blending eases from one wave into the next.
    let table = b.add(NodeKind::Wavetable, SHAPING);
    b.choose(table, 0, SMOOTH);
    let frames = [
        NodeKind::SineWave,
        NodeKind::TriangleWave,
        NodeKind::SawWave,
        NodeKind::SquareWave,
    ];
    for (slot, kind) in frames.into_iter().enumerate() {
        let frame = b.add(kind, SOURCES);
        b.wire(frame, Port::Table, table, Port::Frame(slot));
    }

    // A slow sine LFO sweeps Position the whole way and back about every ten seconds.
    let drift = b.add(NodeKind::Lfo, SOURCES);
    b.set(drift, KnobId::LfoRate, 0.1);
    b.modulate(
        (drift, Port::Shape),
        table,
        KnobId::WtPosition,
        (0.0, 1.0),
        SOURCES,
    );

    let osc = b.add(NodeKind::Oscillator, EFFECTS);
    b.wire(table, Port::Table, osc, Port::TableIn);
    b.set(osc, KnobId::Unison, 5.0);
    b.set(osc, KnobId::Detune, 22.0);
    b.set(osc, KnobId::Spread, 1.0);
    b.fold(osc, PITCH, true);
    b.fold(osc, UNISON, true);
    b.fold(osc, WARP, true);
    b.notes(input, osc);
    b.amplitude(envelope, osc);

    let chorus = b.add(NodeKind::Chorus, EFFECTS);
    b.set(chorus, KnobId::LfoRate, 0.3);
    b.set(chorus, KnobId::Depth, 0.4);
    b.set(chorus, KnobId::Mix, 0.35);
    b.audio(osc, chorus);

    // A slow phaser, a long ping pong delay and a big reverb turn the drift into a wash.
    let phaser = b.add(NodeKind::Phaser, EFFECTS);
    b.set(phaser, KnobId::LfoRate, 0.12);
    b.set(phaser, KnobId::Depth, 0.7);
    b.set(phaser, KnobId::Feedback, 0.4);
    b.set(phaser, KnobId::Mix, 0.45);
    b.audio(chorus, phaser);

    let delay = b.add(NodeKind::Delay, OUTPUT);
    b.set(delay, KnobId::Feedback, 0.5);
    b.set(delay, KnobId::Mix, 0.3);
    b.switch(delay, 0, true);
    b.audio(phaser, delay);

    let reverb = b.add(NodeKind::Reverb, OUTPUT);
    b.set(reverb, KnobId::ReverbDecay, 7.0);
    b.set(reverb, KnobId::Size, 0.9);
    b.set(reverb, KnobId::Mix, 0.4);
    b.audio(delay, reverb);

    let output = b.output(-4.0);
    b.slot(reverb, 0, output);
    b.finish("Glass Morph")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::graph::Graph;

    fn graph() -> Graph {
        Graph::from_saved(&build().graph)
    }

    #[test]
    fn the_table_holds_four_waves_in_order() {
        let graph = graph();
        let table = graph
            .nodes
            .iter()
            .position(|n| n.kind == NodeKind::Wavetable)
            .unwrap();
        let kinds: Vec<_> = (0..4)
            .map(|slot| {
                let wire = graph
                    .connections
                    .iter()
                    .find(|c| c.to == table && c.input == Port::Frame(slot))
                    .expect("a frame pin with no wire");
                graph.nodes[wire.from].kind
            })
            .collect();
        assert_eq!(
            kinds,
            [
                NodeKind::SineWave,
                NodeKind::TriangleWave,
                NodeKind::SawWave,
                NodeKind::SquareWave
            ]
        );
    }

    #[test]
    fn an_lfo_drives_the_position() {
        let graph = graph();
        let moved = graph.connections.iter().any(|c| {
            graph.nodes[c.to].kind == NodeKind::Wavetable
                && c.input == Port::Knob(KnobId::WtPosition)
        });
        assert!(moved, "nothing moves the wavetable");
    }

    #[test]
    fn the_oscillator_plays_the_table_directly() {
        let graph = graph();
        let osc = graph
            .nodes
            .iter()
            .position(|n| n.kind == NodeKind::Oscillator)
            .unwrap();
        let fed_by = graph
            .connections
            .iter()
            .find(|c| c.to == osc && c.input == Port::TableIn)
            .map(|c| graph.nodes[c.from].kind);
        assert_eq!(fed_by, Some(NodeKind::Wavetable));
    }
}
