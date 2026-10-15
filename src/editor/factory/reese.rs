//! Reese. A detuned saw bass whose filter cutoff drifts slowly, pushed through tube drive and a
//! compressor so it growls.

use super::{Builder, CONTROLS, EFFECTS, OUTPUT, SHAPING, SOURCES};
use crate::dsp::wavetable::Waveform;
use crate::nodes::knobs::KnobId;
use crate::nodes::{NodeKind, Port};
use crate::presets::Preset;

/// The oscillator's Pitch and Unison sections, by number.
const PITCH: usize = 0;
const UNISON: usize = 1;
/// The row of "Tube" on the distortion's type dropdown.
const TUBE: usize = 2;

pub(super) fn build() -> Preset {
    let mut b = Builder::new();
    let input = b.voice_input(true, 0.08);
    let envelope = b.envelope(CONTROLS, 0.005, 0.4, 0.9, 0.2);

    // Four saws an octave down, spread a little wide, make the beating Reese sound.
    let osc = b.add(NodeKind::Oscillator, SOURCES);
    b.waveform(osc, Waveform::Saw);
    b.fold(osc, PITCH, true);
    b.fold(osc, UNISON, false);
    b.set(osc, KnobId::Octave, -1.0);
    b.set(osc, KnobId::Unison, 4.0);
    b.set(osc, KnobId::Detune, 28.0);
    b.set(osc, KnobId::Spread, 0.5);
    b.notes(input, osc);
    b.amplitude(envelope, osc);

    // A slow free running sine sweeps the cutoff between 350 and 2200 Hz. The map sits in the
    // effects column because the filter's column is full.
    let filter = b.add(NodeKind::Filter, SHAPING);
    b.set(filter, KnobId::Cutoff, 1000.0);
    b.set(filter, KnobId::Resonance, 0.3);
    b.audio(osc, filter);
    let lfo = b.add(NodeKind::Lfo, SHAPING);
    b.set(lfo, KnobId::LfoRate, 0.35);
    b.switch(lfo, 0, false);
    b.modulate(
        (lfo, Port::Shape),
        filter,
        KnobId::Cutoff,
        (350.0, 2200.0),
        EFFECTS,
    );

    let drive = b.add(NodeKind::Distortion, EFFECTS);
    b.choose(drive, 0, TUBE);
    b.set(drive, KnobId::Drive, 0.4);
    b.set(drive, KnobId::Mix, 0.6);
    b.audio(filter, drive);
    // The compressor is tall, so it goes above the output node instead of under the distortion.
    let compressor = b.add(NodeKind::Compressor, OUTPUT);
    b.set(compressor, KnobId::Threshold, -18.0);
    b.set(compressor, KnobId::Ratio, 3.0);
    b.set(compressor, KnobId::Makeup, 3.0);
    b.audio(drive, compressor);

    let output = b.output(-4.0);
    b.slot(compressor, 0, output);
    b.finish("Reese")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::graph::Graph;

    #[test]
    fn the_lfo_drives_the_filter_cutoff() {
        let preset = build();
        let graph = Graph::from_saved(&preset.graph);
        let kind = |i: usize| graph.nodes[i].kind;
        let driven = graph.connections.iter().any(|c| {
            kind(c.to) == NodeKind::Filter
                && c.input == Port::Knob(KnobId::Cutoff)
                && kind(c.from) == NodeKind::Map
                && graph.connections.iter().any(|m| {
                    m.to == c.from && m.input == Port::Shape && kind(m.from) == NodeKind::Lfo
                })
        });
        assert!(driven, "no LFO to map to cutoff path");
    }

    #[test]
    fn four_detuned_voices_play_mono() {
        let preset = build();
        let graph = Graph::from_saved(&preset.graph);
        let osc = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Oscillator)
            .unwrap();
        assert_eq!(osc.plain(KnobId::Unison).round(), 4.0);
        assert_eq!(osc.plain(KnobId::Octave).round(), -1.0);
        let input = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Input)
            .unwrap();
        assert!(input.switch(0));
    }
}
