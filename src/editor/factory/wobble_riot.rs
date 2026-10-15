//! Wobble Riot. A detuned growl saw torn apart by a square LFO synced to the host tempo, both the
//! filter cutoff and a hard crush riding the same wobble, with a dry sub underneath that never
//! gets touched by the chop. The kind of dubstep bass built to clip a room, not sit in a mix.

use super::{Builder, CONTROLS, EFFECTS, OUTPUT, SHAPING, SOURCES};
use crate::dsp::wavetable::Waveform;
use crate::nodes::knobs::KnobId;
use crate::nodes::{NodeKind, Port};
use crate::presets::Preset;

/// The oscillator's Pitch, Unison, and Warp sections, by number.
const PITCH: usize = 0;
const UNISON: usize = 1;
const WARP: usize = 2;
/// The row of "Crush" on the distortion's type dropdown.
const CRUSH: usize = 4;

pub(super) fn build() -> Preset {
    let mut b = Builder::new();
    let input = b.voice_input(true, 0.02);
    let amp = b.envelope(CONTROLS, 0.002, 0.3, 0.95, 0.08);

    // Six detuned saws, wide and biting, make the growl. Folded away so it fits its column
    // beside the filter; the values still apply with the sections closed.
    let growl = b.add(NodeKind::Oscillator, SHAPING);
    b.waveform(growl, Waveform::Saw);
    b.set(growl, KnobId::Octave, -1.0);
    b.set(growl, KnobId::Unison, 6.0);
    b.set(growl, KnobId::Detune, 32.0);
    b.set(growl, KnobId::Spread, 0.6);
    b.fold(growl, PITCH, true);
    b.fold(growl, UNISON, true);
    b.fold(growl, WARP, true);
    b.notes(input, growl);
    b.amplitude(amp, growl);

    // A plain sine, two octaves down, skips the wobble and the crush entirely so the low end
    // stays solid no matter how hard the growl is torn up.
    let sub = b.add(NodeKind::Oscillator, SOURCES);
    b.waveform(sub, Waveform::Sine);
    b.set(sub, KnobId::Octave, -2.0);
    b.fold(sub, PITCH, true);
    b.fold(sub, UNISON, true);
    b.fold(sub, WARP, true);
    b.notes(input, sub);
    b.amplitude(amp, sub);

    // The wobble: a square LFO synced to 1/16 notes swings the cutoff between a growl and a
    // near close, the classic dubstep chop.
    let filter = b.add(NodeKind::Filter, SHAPING);
    b.set(filter, KnobId::Resonance, 0.6);
    b.audio(growl, filter);
    let wobble = b.add(NodeKind::Lfo, SOURCES);
    b.waveform(wobble, Waveform::Square);
    b.set(wobble, KnobId::LfoSync, 4.0);
    b.switch(wobble, 0, true);
    b.modulate(
        (wobble, Port::Shape),
        filter,
        KnobId::Cutoff,
        (140.0, 3800.0),
        EFFECTS,
    );

    // One hard crush does the grinding, pushed fully wet.
    let crush = b.add(NodeKind::Distortion, EFFECTS);
    b.choose(crush, 0, CRUSH);
    b.set(crush, KnobId::Drive, 0.6);
    b.set(crush, KnobId::Mix, 0.75);
    b.audio(filter, crush);

    // The compressor is tall, so it sits above the output node like Reese's does.
    let compressor = b.add(NodeKind::Compressor, OUTPUT);
    b.set(compressor, KnobId::Threshold, -16.0);
    b.set(compressor, KnobId::Ratio, 4.0);
    b.set(compressor, KnobId::Makeup, 4.0);
    b.audio(crush, compressor);

    let output = b.output(-6.0);
    b.slot(compressor, 0, output);
    b.slot(sub, 1, output);
    b.slot_gain(output, 0, 0.0);
    b.slot_gain(output, 1, -2.0);
    b.finish("Wobble Riot")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::graph::Graph;

    fn graph() -> Graph {
        Graph::from_saved(&build().graph)
    }

    #[test]
    fn the_lfo_drives_the_filter_cutoff() {
        let graph = graph();
        let lfos = graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Lfo)
            .count();
        assert_eq!(lfos, 1);
        let cutoff_driven = graph.connections.iter().any(|c| {
            graph.nodes[c.to].kind == NodeKind::Filter && c.input == Port::Knob(KnobId::Cutoff)
        });
        assert!(cutoff_driven, "no wobble on the cutoff");
    }

    #[test]
    fn the_wobble_is_tempo_synced() {
        let graph = graph();
        let lfo = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Lfo)
            .unwrap();
        assert!(lfo.plain(KnobId::LfoSync) > 0.0, "the wobble free-runs");
    }

    #[test]
    fn the_sub_skips_the_grind() {
        let graph = graph();
        let sub = graph
            .nodes
            .iter()
            .position(|n| n.kind == NodeKind::Oscillator && n.plain(KnobId::Octave) == -2.0)
            .unwrap();
        // The sub's only audio connection is straight to an output slot.
        let destinations: Vec<_> = graph
            .connections
            .iter()
            .filter(|c| c.from == sub && c.out == Port::Audio)
            .map(|c| graph.nodes[c.to].kind)
            .collect();
        assert_eq!(destinations, [NodeKind::Output]);
    }

    #[test]
    fn the_growl_runs_through_a_crush() {
        let graph = graph();
        let crush = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Distortion)
            .unwrap();
        assert_eq!(crush.choice(0), CRUSH);
    }
}
