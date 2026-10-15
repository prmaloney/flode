//! Warm Pad. A wide, slow saw pad. Six detuned saws spread across the stereo field go through a
//! low pass filter that a slow LFO sweeps, then a chorus and a reverb.

use super::{Builder, CONTROLS, EFFECTS, OUTPUT, SHAPING, SOURCES};
use crate::dsp::wavetable::Waveform;
use crate::nodes::knobs::KnobId;
use crate::nodes::{NodeKind, Port};
use crate::presets::Preset;

pub(super) fn build() -> Preset {
    let mut b = Builder::new();
    let input = b.voice_input(false, 0.0);
    let env = b.envelope(CONTROLS, 0.8, 1.2, 0.85, 1.6);

    let osc = b.add(NodeKind::Oscillator, SOURCES);
    b.waveform(osc, Waveform::Saw);
    b.set(osc, KnobId::Unison, 6.0);
    b.set(osc, KnobId::Detune, 24.0);
    b.set(osc, KnobId::Spread, 1.0);
    b.set(osc, KnobId::OscGain, 0.0);
    b.notes(input, osc);
    b.amplitude(env, osc);

    let filter = b.add(NodeKind::Filter, SHAPING);
    b.set(filter, KnobId::Cutoff, 2000.0);
    b.set(filter, KnobId::Resonance, 0.12);
    b.audio(osc, filter);

    // A slow sine, free running, breathes the cutoff between 1100 and 3200 Hz.
    let lfo = b.add(NodeKind::Lfo, SHAPING);
    b.waveform(lfo, Waveform::Sine);
    b.set(lfo, KnobId::LfoRate, 0.12);
    b.modulate(
        (lfo, Port::Shape),
        filter,
        KnobId::Cutoff,
        (1100.0, 3200.0),
        OUTPUT,
    );

    let chorus = b.add(NodeKind::Chorus, EFFECTS);
    b.set(chorus, KnobId::LfoRate, 0.5);
    b.set(chorus, KnobId::Depth, 0.45);
    b.set(chorus, KnobId::Spread, 0.9);
    b.set(chorus, KnobId::Mix, 0.45);
    b.audio(filter, chorus);

    let reverb = b.add(NodeKind::Reverb, EFFECTS);
    b.set(reverb, KnobId::ReverbDecay, 4.5);
    b.set(reverb, KnobId::Size, 0.75);
    b.set(reverb, KnobId::Mix, 0.35);
    b.audio(chorus, reverb);

    let output = b.output(-1.5);
    b.slot(reverb, 0, output);
    b.finish("Warm Pad")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::wavetable::WaveTables;
    use crate::editor::graph::Graph;
    use crate::engine::{Engine, Transport};
    use std::sync::Arc;

    /// The peak of `notes` held together for `seconds`.
    fn peak(preset: &Preset, notes: &[u8], seconds: usize) -> f32 {
        let patch = Graph::from_saved(&preset.graph).patch();
        let mut engine = Engine::new(Arc::new(WaveTables::new()));
        engine.begin_block(&patch, Transport::default());
        engine.set_macros(preset.macros);
        for &note in notes {
            engine.note_on(note, 1.0);
        }
        (0..seconds * 44100).fold(0.0f32, |peak, n| peak.max(engine.process_sample(n).peak()))
    }

    /// The LFO drives the filter's cutoff, and the oscillator runs six unison voices from a poly
    /// input.
    #[test]
    fn six_saws_through_a_filter_the_lfo_sweeps() {
        let graph = Graph::from_saved(&build().graph);
        let find = |kind| graph.nodes.iter().position(|n| n.kind == kind).unwrap();
        let (input, osc, filter) = (
            find(NodeKind::Input),
            find(NodeKind::Oscillator),
            find(NodeKind::Filter),
        );
        assert!(!graph.nodes[input].switch(0));
        assert_eq!(
            graph.nodes[osc].value(KnobId::Unison),
            KnobId::Unison.to_normalized(6.0)
        );
        let sweep = graph
            .connections
            .iter()
            .find(|c| c.to == filter && c.input == Port::Knob(KnobId::Cutoff))
            .expect("the cutoff has a control wire");
        assert_eq!(graph.nodes[sweep.from].kind, NodeKind::Map);
        assert!(graph
            .connections
            .iter()
            .any(|c| { c.to == sweep.from && graph.nodes[c.from].kind == NodeKind::Lfo }));
    }

    /// The chain ends in a chorus and a reverb, and the pad is loud enough to read on the meter
    /// even after the slow attack, without a chord clipping at the peak of the sweep.
    #[test]
    fn ends_in_a_reverb_and_peaks_after_the_attack() {
        let preset = build();
        let graph = Graph::from_saved(&preset.graph);
        let into_output = graph
            .connections
            .iter()
            .find(|c| graph.nodes[c.to].kind == NodeKind::Output)
            .unwrap();
        assert_eq!(graph.nodes[into_output.from].kind, NodeKind::Reverb);
        assert!(peak(&preset, &[48], 8) < 0.8);
        assert!(peak(&preset, &[48, 55, 60], 8) <= 0.98);
    }
}
