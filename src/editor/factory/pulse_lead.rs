//! Pulse Lead. A bright pulse-wave lead. A slow LFO sweeps the pulse width, three detuned voices
//! thicken it, and a short echo and a small room sit behind it.

use super::{Builder, CONTROLS, EFFECTS, OUTPUT, SHAPING, SOURCES};
use crate::dsp::wavetable::Waveform;
use crate::nodes::knobs::KnobId;
use crate::nodes::{NodeKind, Port};
use crate::presets::Preset;

pub(super) fn build() -> Preset {
    let mut b = Builder::new();
    let input = b.voice_input(true, 0.04);
    let envelope = b.envelope(CONTROLS, 0.004, 0.3, 0.7, 0.25);

    let osc = b.add(NodeKind::Oscillator, SOURCES);
    b.waveform(osc, Waveform::Square);
    b.warp(osc, "PWM");
    // Section 2 is the warp section. It starts folded.
    b.fold(osc, 2, false);
    // The unison knobs are set below, so the section can stay folded to fit the window.
    b.fold(osc, 1, true);
    b.set(osc, KnobId::Unison, 3.0);
    b.set(osc, KnobId::Detune, 14.0);
    b.set(osc, KnobId::Spread, 0.7);
    b.set(osc, KnobId::OscGain, 0.0);
    b.notes(input, osc);
    b.amplitude(envelope, osc);

    let lfo = b.add(NodeKind::Lfo, SHAPING);
    b.choose(lfo, 0, Waveform::Sine.index());
    b.set(lfo, KnobId::LfoRate, 0.7);

    let filter = b.add(NodeKind::Filter, SHAPING);
    b.set(filter, KnobId::Cutoff, 4200.0);
    b.set(filter, KnobId::Resonance, 0.2);
    b.audio(osc, filter);

    let delay = b.add(NodeKind::Delay, EFFECTS);
    b.set(delay, KnobId::DelayTime, 360.0);
    b.set(delay, KnobId::Feedback, 0.35);
    b.set(delay, KnobId::Damp, 0.4);
    b.set(delay, KnobId::Mix, 0.22);
    b.audio(filter, delay);

    // The effects column has room for the delay and the map, not the reverb, so the reverb sits
    // under the output. The LFO moves the pulse width between a quarter and three quarters.
    let output = b.output(0.0);
    let reverb = b.add(NodeKind::Reverb, OUTPUT);
    b.set(reverb, KnobId::ReverbDecay, 1.6);
    b.set(reverb, KnobId::Size, 0.45);
    b.set(reverb, KnobId::Mix, 0.18);
    b.audio(delay, reverb);
    b.slot(reverb, 0, output);
    b.modulate(
        (lfo, Port::Shape),
        osc,
        KnobId::WarpAmount,
        (0.25, 0.75),
        EFFECTS,
    );
    b.finish("Pulse Lead")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::warp::WarpMode;
    use crate::editor::graph::Graph;
    use crate::nodes::oscillator::warp_mode;

    #[test]
    fn the_lfo_drives_the_pulse_width() {
        let graph = Graph::from_saved(&build().graph);
        let osc = graph
            .nodes
            .iter()
            .position(|n| n.kind == NodeKind::Oscillator)
            .unwrap();
        assert_eq!(warp_mode(&graph.nodes[osc]), WarpMode::Pwm);
        let map = graph
            .connections
            .iter()
            .find(|c| c.to == osc && c.input == Port::Knob(KnobId::WarpAmount))
            .expect("nothing drives the warp amount")
            .from;
        assert!(graph
            .connections
            .iter()
            .any(|c| { c.to == map && graph.nodes[c.from].kind == NodeKind::Lfo }));
    }

    #[test]
    fn the_input_is_mono() {
        let graph = Graph::from_saved(&build().graph);
        let input = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Input)
            .unwrap();
        assert!(input.switch(0));
    }
}
