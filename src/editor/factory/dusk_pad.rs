//! Dusk Pad. A dark, slow triangle pad. Macro 1 opens the filter and Macro 2 adds reverb.

use super::{Builder, CONTROLS, EFFECTS, OUTPUT, SHAPING, SOURCES};
use crate::dsp::wavetable::Waveform;
use crate::nodes::knobs::KnobId;
use crate::nodes::{NodeKind, Port};
use crate::presets::Preset;

pub(super) fn build() -> Preset {
    let mut b = Builder::new();
    let input = b.voice_input(false, 0.0);
    let envelope = b.envelope(CONTROLS, 1.1, 1.5, 0.9, 2.6);
    let macros = b.add(NodeKind::Macro, SHAPING);

    let osc = b.add(NodeKind::Oscillator, SOURCES);
    b.waveform(osc, Waveform::Triangle);
    b.set(osc, KnobId::Unison, 7.0);
    b.set(osc, KnobId::Detune, 32.0);
    b.set(osc, KnobId::Spread, 1.0);
    b.set(osc, KnobId::OscGain, 0.0);
    b.notes(input, osc);
    b.amplitude(envelope, osc);

    let filter = b.add(NodeKind::Filter, SHAPING);
    b.set(filter, KnobId::Resonance, 0.15);
    b.audio(osc, filter);
    let cutoff_map = b.modulate(
        (macros, Port::Macro(0)),
        filter,
        KnobId::Cutoff,
        (300.0, 9000.0),
        SHAPING,
    );

    let phaser = b.add(NodeKind::Phaser, EFFECTS);
    b.set(phaser, KnobId::LfoRate, 0.15);
    b.set(phaser, KnobId::Depth, 0.6);
    b.set(phaser, KnobId::Feedback, 0.35);
    b.set(phaser, KnobId::Mix, 0.5);
    b.audio(filter, phaser);

    let reverb = b.add(NodeKind::Reverb, OUTPUT);
    b.set(reverb, KnobId::ReverbDecay, 6.5);
    b.set(reverb, KnobId::Size, 0.85);
    b.audio(phaser, reverb);
    let mix_map = b.modulate(
        (macros, Port::Macro(1)),
        reverb,
        KnobId::Mix,
        (0.1, 0.8),
        EFFECTS,
    );
    // The five columns are too short for ten open nodes, so the two maps are collapsed. Their
    // ranges are set once and the macro knobs are the ones to play.
    b.graph.nodes[cutoff_map].collapsed = true;
    b.graph.nodes[mix_map].collapsed = true;

    let output = b.output(0.0);
    b.slot(reverb, 0, output);

    b.macros = [0.45, 0.4, 0.0, 0.0];
    b.finish("Dusk Pad")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::wavetable::WaveTables;
    use crate::editor::graph::Graph;
    use crate::engine::{Engine, Transport};
    use std::sync::Arc;

    const SAMPLE_RATE: usize = 44100;

    /// Renders one note for `seconds` with these macro values and returns the left channel.
    fn render(macros: [f32; 4], seconds: f32) -> Vec<f32> {
        let preset = build();
        let patch = Graph::from_saved(&preset.graph).patch();
        let mut engine = Engine::new(Arc::new(WaveTables::new()));
        engine.begin_block(&patch, Transport::default());
        engine.set_macros(macros);
        engine.note_on(48, 1.0);
        (0..(seconds * SAMPLE_RATE as f32) as usize)
            .map(|n| engine.process_sample(n).l)
            .collect()
    }

    fn energy(samples: &[f32]) -> f32 {
        samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32
    }

    /// The sum of squared second differences, which grows with high frequency content.
    fn roughness(samples: &[f32]) -> f32 {
        samples
            .windows(3)
            .map(|w| (w[2] - 2.0 * w[1] + w[0]).powi(2))
            .sum()
    }

    #[test]
    fn the_macros_drive_the_cutoff_and_the_reverb_mix() {
        let preset = build();
        let graph = Graph::from_saved(&preset.graph);
        let driven = |kind: NodeKind, knob: KnobId| {
            graph.connections.iter().any(|c| {
                c.to < graph.nodes.len()
                    && graph.nodes[c.to].kind == kind
                    && c.input == Port::Knob(knob)
                    && graph.connections.iter().any(|m| {
                        m.to == c.from
                            && graph.nodes[m.from].kind == NodeKind::Macro
                            && m.out == Port::Macro(if knob == KnobId::Cutoff { 0 } else { 1 })
                    })
            })
        };
        assert!(driven(NodeKind::Filter, KnobId::Cutoff));
        assert!(driven(NodeKind::Reverb, KnobId::Mix));
        assert_eq!(preset.macros, [0.45, 0.4, 0.0, 0.0]);
    }

    #[test]
    fn macro_one_changes_the_sound() {
        let dark = render([0.0, 0.4, 0.0, 0.0], 3.0);
        let bright = render([1.0, 0.4, 0.0, 0.0], 3.0);
        assert!(roughness(&bright) > 1.5 * roughness(&dark));
        let difference: Vec<f32> = dark.iter().zip(&bright).map(|(a, b)| a - b).collect();
        assert!(energy(&difference) > 0.1 * energy(&dark));
    }

    #[test]
    fn macro_two_changes_the_sound() {
        let dry = render([0.45, 0.0, 0.0, 0.0], 3.0);
        let wet = render([0.45, 1.0, 0.0, 0.0], 3.0);
        let difference: Vec<f32> = dry.iter().zip(&wet).map(|(a, b)| a - b).collect();
        assert!(energy(&difference) > 0.1 * energy(&dry));
    }
}
