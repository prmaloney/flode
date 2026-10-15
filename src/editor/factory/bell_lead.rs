//! Bell Lead. A glassy FM pluck. A sine carrier is phase modulated by a sine tuned up a little
//! past a ratio of 3, so the sidebands are slightly inharmonic like a struck bell. A second
//! envelope drives the FM depth, so the tone is brightest at the start of the note and mellows
//! as it rings. Chorus and reverb sit after the mix.

use super::{Builder, CONTROLS, EFFECTS, OUTPUT, SHAPING, SOURCES};
use crate::nodes::knobs::KnobId;
use crate::nodes::{NodeKind, Port};
use crate::presets::Preset;

/// The row of the Mix node's dropdown that picks FM.
const FM: usize = 3;

pub(super) fn build() -> Preset {
    let mut b = Builder::new();
    let input = b.voice_input(true, 0.03);
    // The input is set once and forgotten, so it is folded to leave room in its column.
    b.graph.nodes[input].collapsed = true;
    let amp = b.envelope(CONTROLS, 0.002, 0.6, 0.35, 0.5);
    let bright = b.envelope(CONTROLS, 0.002, 0.35, 0.1, 0.2);

    let carrier = b.add(NodeKind::Oscillator, SOURCES);
    let modulator = b.add(NodeKind::Oscillator, SHAPING);
    for osc in [carrier, modulator] {
        b.waveform(osc, crate::dsp::wavetable::Waveform::Sine);
        b.notes(input, osc);
    }
    b.amplitude(amp, carrier);
    // The modulator is an octave and seven semitones up, with a little detune. Its Pitch section
    // is open so the tuning shows. Unison is folded on both to fit the column.
    b.fold(modulator, 0, false);
    b.fold(modulator, 1, true);
    b.fold(carrier, 1, true);
    b.set(modulator, KnobId::Octave, 1.0);
    b.set(modulator, KnobId::Coarse, 7.0);
    b.set(modulator, KnobId::Fine, 20.0);

    let fm = b.add(NodeKind::Mix, EFFECTS);
    b.choose(fm, 0, FM);
    b.audio_in(carrier, fm, 0);
    b.audio_in(modulator, fm, 1);
    b.modulate(
        (bright, Port::Shape),
        fm,
        KnobId::Mix,
        (0.08, 0.55),
        SOURCES,
    );

    let chorus = b.add(NodeKind::Chorus, EFFECTS);
    b.set(chorus, KnobId::LfoRate, 0.5);
    b.set(chorus, KnobId::Depth, 0.35);
    b.set(chorus, KnobId::Mix, 0.3);
    b.audio(fm, chorus);

    let reverb = b.add(NodeKind::Reverb, OUTPUT);
    b.set(reverb, KnobId::ReverbDecay, 2.2);
    b.set(reverb, KnobId::Size, 0.5);
    b.set(reverb, KnobId::Mix, 0.25);
    b.audio(chorus, reverb);

    // The master gain is at its top. One note peaks near 0.33, since the FM and the effects
    // already keep the level down.
    let output = b.output(0.0);
    b.slot(reverb, 0, output);
    b.finish("Bell Lead")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::mix::MixMode;
    use crate::editor::graph::Graph;
    use crate::nodes::mix::mix_mode;

    fn loaded() -> Graph {
        Graph::from_saved(&build().graph)
    }

    fn find(graph: &Graph, kind: NodeKind) -> Vec<usize> {
        (0..graph.nodes.len())
            .filter(|&i| graph.nodes[i].kind == kind)
            .collect()
    }

    /// The carrier and the modulator are both sines, and the Mix node does FM between them.
    #[test]
    fn the_mix_is_fm() {
        let graph = loaded();
        let mix = find(&graph, NodeKind::Mix);
        assert_eq!(mix.len(), 1);
        assert_eq!(mix_mode(&graph.nodes[mix[0]]), MixMode::Fm);
    }

    /// An envelope drives the FM depth through a map, so the tone is brightest at the attack.
    #[test]
    fn an_envelope_drives_the_fm_amount() {
        let graph = loaded();
        let mix = find(&graph, NodeKind::Mix)[0];
        let into_mix = graph
            .connections
            .iter()
            .find(|c| c.to == mix && c.input == Port::Knob(KnobId::Mix))
            .expect("the Mix knob is driven");
        assert_eq!(graph.nodes[into_mix.from].kind, NodeKind::Map);
        let into_map = graph
            .connections
            .iter()
            .find(|c| c.to == into_mix.from)
            .expect("the map has a source");
        assert_eq!(graph.nodes[into_map.from].kind, NodeKind::Envelope);
    }

    /// The modulator is tuned up an octave and seven semitones, and slightly sharp.
    #[test]
    fn the_modulator_is_tuned_up() {
        let graph = loaded();
        let mix = find(&graph, NodeKind::Mix)[0];
        let b_input = graph
            .connections
            .iter()
            .find(|c| c.to == mix && c.input == Port::AudioIn(1))
            .expect("the modulator feeds input B");
        let modulator = &graph.nodes[b_input.from];
        assert_eq!(modulator.kind, NodeKind::Oscillator);
        let plain = |id: KnobId| id.to_plain(modulator.value(id));
        assert_eq!(plain(KnobId::Octave), 1.0);
        assert_eq!(plain(KnobId::Coarse), 7.0);
        assert!((plain(KnobId::Fine) - 20.0).abs() < 0.5);
    }
}
