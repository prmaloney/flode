//! Tearout. A brutal dubstep bass. A table of a wavefolded sweep, a saw and a square is raked by
//! a 1/16 saw LFO that moves the Position and the filter cutoff together, then folded, overdriven,
//! flanged and crushed in a row before a hard compressor. A clean sine sub sits under the whole
//! mess and skips every effect, so the low end stays solid.

use super::{Builder, CONTROLS, EFFECTS, OUTPUT, SHAPING, SOURCES};
use crate::dsp::wavetable::Waveform;
use crate::nodes::knobs::KnobId;
use crate::nodes::{NodeKind, Port};
use crate::presets::Preset;

/// The oscillator's Pitch, Unison, and Warp sections, by number.
const PITCH: usize = 0;
const UNISON: usize = 1;
const WARP: usize = 2;
/// The rows of the distortion's type dropdown.
const FOLD: usize = 3;
const CRUSH: usize = 4;
const OVERDRIVE: usize = 5;
/// The row of "Fold" on the sweep's kind dropdown.
const SWEEP_FOLD: usize = 3;
/// The row of "Band pass" on the filter's type dropdown.
const BAND_PASS: usize = 2;

pub(super) fn build() -> Preset {
    let mut b = Builder::new();
    let input = b.voice_input(true, 0.03);
    let amp = b.envelope(CONTROLS, 0.001, 0.4, 0.9, 0.06);

    // Three frames: a sine folded harder and harder across 24 steps, then a saw, then a square.
    // The sweep gets the biggest share of the ramp, so most of Position is the fold opening up.
    let table = b.add(NodeKind::Wavetable, SHAPING);
    let sweep = b.add(NodeKind::Sweep, SOURCES);
    b.choose(sweep, 0, SWEEP_FOLD);
    b.set(sweep, KnobId::SweepFrames, 24.0);
    b.set(sweep, KnobId::SweepAmount, 1.0);
    let saw = b.add(NodeKind::SawWave, SOURCES);
    let square = b.add(NodeKind::SquareWave, SOURCES);
    for (slot, frame) in [sweep, saw, square].into_iter().enumerate() {
        b.wire(frame, Port::Table, table, Port::Frame(slot));
    }
    b.set(table, KnobId::share(0), 1.0);
    b.set(table, KnobId::share(1), 0.3);
    b.set(table, KnobId::share(2), 0.3);

    // One saw LFO at 1/16 notes rakes the table and the filter cutoff together.
    let rake = b.add(NodeKind::Lfo, SOURCES);
    b.waveform(rake, Waveform::Saw);
    b.set(rake, KnobId::LfoSync, 4.0);
    b.switch(rake, 0, true);
    b.modulate(
        (rake, Port::Shape),
        table,
        KnobId::WtPosition,
        (0.05, 1.0),
        SOURCES,
    );

    let growl = b.add(NodeKind::Oscillator, SHAPING);
    b.wire(table, Port::Table, growl, Port::TableIn);
    b.set(growl, KnobId::Octave, -1.0);
    b.set(growl, KnobId::Unison, 4.0);
    b.set(growl, KnobId::Detune, 38.0);
    b.set(growl, KnobId::Spread, 0.5);
    b.fold(growl, PITCH, true);
    b.fold(growl, UNISON, true);
    b.fold(growl, WARP, true);
    b.notes(input, growl);
    b.amplitude(amp, growl);

    // A resonant band pass, opened and closed by the same LFO.
    let filter = b.add(NodeKind::Filter, EFFECTS);
    b.choose(filter, 0, BAND_PASS);
    b.set(filter, KnobId::Resonance, 0.55);
    b.audio(growl, filter);
    b.modulate(
        (rake, Port::Shape),
        filter,
        KnobId::Cutoff,
        (250.0, 5200.0),
        SOURCES,
    );

    // The destruction, in order: fold, overdrive, flange, crush.
    let fold = b.add(NodeKind::Distortion, EFFECTS);
    b.choose(fold, 0, FOLD);
    b.set(fold, KnobId::Drive, 0.65);
    b.set(fold, KnobId::Mix, 0.85);
    b.audio(filter, fold);

    let drive = b.add(NodeKind::Distortion, EFFECTS);
    b.choose(drive, 0, OVERDRIVE);
    b.set(drive, KnobId::Drive, 0.7);
    b.set(drive, KnobId::Mix, 1.0);
    b.audio(fold, drive);

    let flanger = b.add(NodeKind::Flanger, EFFECTS);
    b.set(flanger, KnobId::LfoRate, 0.4);
    b.set(flanger, KnobId::Feedback, 0.5);
    b.set(flanger, KnobId::Mix, 0.5);
    b.audio(drive, flanger);

    let crush = b.add(NodeKind::Distortion, EFFECTS);
    b.choose(crush, 0, CRUSH);
    b.set(crush, KnobId::Drive, 0.45);
    b.set(crush, KnobId::Mix, 0.5);
    b.audio(flanger, crush);

    let compressor = b.add(NodeKind::Compressor, OUTPUT);
    b.set(compressor, KnobId::Threshold, -20.0);
    b.set(compressor, KnobId::Ratio, 8.0);
    b.set(compressor, KnobId::Makeup, 5.0);
    b.audio(crush, compressor);

    // The sub plays a plain sine two octaves down and never touches the effects.
    let sub = b.add(NodeKind::Oscillator, EFFECTS);
    b.waveform(sub, Waveform::Sine);
    b.set(sub, KnobId::Octave, -2.0);
    b.fold(sub, PITCH, true);
    b.fold(sub, UNISON, true);
    b.fold(sub, WARP, true);
    b.notes(input, sub);
    b.amplitude(amp, sub);

    let output = b.output(-8.0);
    b.slot(compressor, 0, output);
    b.slot(sub, 1, output);
    b.slot_gain(output, 0, 0.0);
    b.slot_gain(output, 1, -3.0);

    b.finish("Tearout")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::graph::Graph;

    fn graph() -> Graph {
        Graph::from_saved(&build().graph)
    }

    #[test]
    fn one_lfo_drives_the_position_and_the_cutoff() {
        let graph = graph();
        let driven = |kind: NodeKind, knob: KnobId| {
            graph.connections.iter().any(|c| {
                graph.nodes[c.to].kind == kind
                    && c.input == Port::Knob(knob)
                    && graph
                        .connections
                        .iter()
                        .any(|m| m.to == c.from && graph.nodes[m.from].kind == NodeKind::Lfo)
            })
        };
        assert!(driven(NodeKind::Wavetable, KnobId::WtPosition));
        assert!(driven(NodeKind::Filter, KnobId::Cutoff));
    }

    #[test]
    fn the_rake_is_tempo_synced() {
        let graph = graph();
        let lfo = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Lfo)
            .unwrap();
        assert!(lfo.plain(KnobId::LfoSync) > 0.0, "the rake free-runs");
    }

    #[test]
    fn the_growl_is_fed_by_a_wavetable_of_three_frames() {
        let graph = graph();
        let table = graph
            .nodes
            .iter()
            .position(|n| n.kind == NodeKind::Wavetable)
            .unwrap();
        for slot in 0..3 {
            assert!(
                graph
                    .connections
                    .iter()
                    .any(|c| c.to == table && c.input == Port::Frame(slot)),
                "frame {slot} has no wire"
            );
        }
    }

    #[test]
    fn the_sub_skips_the_effects() {
        let graph = graph();
        let sub = graph
            .nodes
            .iter()
            .position(|n| n.kind == NodeKind::Oscillator && n.plain(KnobId::Octave) == -2.0)
            .unwrap();
        let destinations: Vec<_> = graph
            .connections
            .iter()
            .filter(|c| c.from == sub && c.out == Port::Audio)
            .map(|c| graph.nodes[c.to].kind)
            .collect();
        assert_eq!(destinations, [NodeKind::Output]);
    }

    #[test]
    fn four_effects_tear_the_growl_up() {
        let graph = graph();
        let count = |kind: NodeKind| graph.nodes.iter().filter(|n| n.kind == kind).count();
        assert_eq!(count(NodeKind::Distortion), 3);
        assert_eq!(count(NodeKind::Flanger), 1);
    }
}
