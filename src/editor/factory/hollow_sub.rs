//! Hollow Sub. Two detuned sine subs, three octaves down, one run through a Gain node with its
//! polarity flipped before they sum. The slow beating between the two, pushed in and out of
//! cancellation by the inverted side, hollows the sub out instead of just doubling it.

use super::{Builder, EFFECTS, OUTPUT, SHAPING, SOURCES};
use crate::dsp::wavetable::Waveform;
use crate::nodes::knobs::KnobId;
use crate::nodes::{NodeKind, Port};
use crate::presets::Preset;

/// The Mix node dropdown's row for plain addition.
const ADD: usize = 4;
/// The oscillator's Unison and Warp sections, by number.
const UNISON: usize = 1;
const WARP: usize = 2;

pub(super) fn build() -> Preset {
    let mut b = Builder::new();
    let input = b.voice_input(true, 0.02);
    let envelope = b.envelope(super::CONTROLS, 0.005, 0.3, 0.9, 0.15);

    let osc = b.add(NodeKind::Oscillator, SOURCES);
    b.waveform(osc, Waveform::Sine);
    b.set(osc, KnobId::Octave, -3.0);
    b.fold(osc, UNISON, true);
    b.fold(osc, WARP, true);
    b.notes(input, osc);
    b.amplitude(envelope, osc);

    let detuned = b.add(NodeKind::Oscillator, SOURCES);
    b.waveform(detuned, Waveform::Sine);
    b.set(detuned, KnobId::Octave, -3.0);
    b.set(detuned, KnobId::Fine, 6.0);
    b.fold(detuned, UNISON, true);
    b.fold(detuned, WARP, true);
    b.notes(input, detuned);
    b.amplitude(envelope, detuned);

    // The detuned side runs through an inverted Gain before it joins the first, so the two drift
    // in and out of phase as the detune beats, instead of just reinforcing each other.
    let invert = b.add(NodeKind::Gain, SHAPING);
    b.switch(invert, 0, true);
    b.audio(detuned, invert);

    let mix = b.add(NodeKind::Mix, SHAPING);
    b.choose(mix, 0, ADD);
    b.audio_in(osc, mix, 0);
    b.audio_in(invert, mix, 1);

    let filter = b.add(NodeKind::Filter, EFFECTS);
    b.set(filter, KnobId::Cutoff, 900.0);
    b.audio(mix, filter);

    let compressor = b.add(NodeKind::Compressor, OUTPUT);
    b.set(compressor, KnobId::Threshold, -16.0);
    b.set(compressor, KnobId::Ratio, 4.0);
    b.set(compressor, KnobId::Makeup, 4.0);
    b.audio(filter, compressor);

    let output = b.output(-4.0);
    b.slot(compressor, 0, output);
    b.finish("Hollow Sub")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::mix::MixMode;
    use crate::editor::graph::Graph;
    use crate::nodes::gain::invert;
    use crate::nodes::mix::mix_mode;

    fn graph() -> Graph {
        Graph::from_saved(&build().graph)
    }

    #[test]
    fn one_side_of_the_mix_runs_through_an_inverted_gain() {
        let graph = graph();
        let mix = graph
            .nodes
            .iter()
            .position(|n| n.kind == NodeKind::Mix)
            .unwrap();
        assert_eq!(mix_mode(&graph.nodes[mix]), MixMode::Add);
        let b_input = graph
            .connections
            .iter()
            .find(|c| c.to == mix && c.input == Port::AudioIn(1))
            .expect("input B is wired");
        let gain = &graph.nodes[b_input.from];
        assert_eq!(gain.kind, NodeKind::Gain);
        assert!(invert(gain), "the gain stage is not inverted");
    }

    #[test]
    fn both_oscillators_sit_three_octaves_down_and_one_is_detuned() {
        let graph = graph();
        let oscillators: Vec<_> = graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Oscillator)
            .collect();
        assert_eq!(oscillators.len(), 2);
        for osc in &oscillators {
            assert_eq!(osc.plain(KnobId::Octave), -3.0);
        }
        assert!(oscillators.iter().any(|osc| osc.plain(KnobId::Fine) != 0.0));
    }
}
