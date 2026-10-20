//! The factory presets: a few starting patches, built in code with a `Builder` so each one is
//! plain to read and to change. Each preset lives in its own file and has a `build` function.
//! Nothing here runs in the plugin. The ignored test `write_factory_presets` writes the files
//! into the preset folder:
//!
//! `cargo test --lib write_factory_presets -- --ignored`
//!
//! The other tests check every preset on its own: it loads without losing a wire, it makes sound
//! at a sane level, it fits the editor window, and it takes its notes from a MIDI input.

mod bell_lead;
mod dusk_pad;
mod pulse_lead;
mod reese;
mod sub_bass;
mod warm_pad;
mod wobble_riot;

use super::graph::Graph;
use crate::dsp::wavetable::Waveform;
use crate::nodes::connection::Connection;
use crate::nodes::knobs::KnobId;
use crate::nodes::oscillator::WARP_LABELS;
use crate::nodes::output::slot_knob;
use crate::nodes::{FlodeNode, NodeKind, Port};
use crate::presets::Preset;
use nih_plug_iced::Point;

/// Every factory preset, basses first, then leads, then pads. Each is filed under its kind, which
/// is the folder it is written into.
pub(super) fn all() -> Vec<Preset> {
    let filed = |category: &str, mut preset: Preset| {
        preset.category = category.to_string();
        preset
    };
    vec![
        filed("Bass", sub_bass::build()),
        filed("Bass", reese::build()),
        filed("Bass", wobble_riot::build()),
        filed("Lead", pulse_lead::build()),
        filed("Lead", bell_lead::build()),
        filed("Pad", warm_pad::build()),
        filed("Pad", dusk_pad::build()),
    ]
}

/// The left edge of each column of nodes. A patch reads left to right: the MIDI input and the
/// envelope, the oscillators, the filter and what drives it, the effects, and the output.
const COLUMN_X: [f32; 5] = [24.0, 224.0, 424.0, 624.0, 840.0];
/// Where the top node of a column sits, and the gap between stacked nodes.
const TOP: f32 = 80.0;
const GAP: f32 = 12.0;

/// The column of the MIDI input and the amplitude envelope.
pub(super) const CONTROLS: usize = 0;
/// The column of the oscillators.
pub(super) const SOURCES: usize = 1;
/// The column of filters, maps, LFOs, and mixes.
pub(super) const SHAPING: usize = 2;
/// The column of effects.
pub(super) const EFFECTS: usize = 3;
/// The column of the output node, with room under it.
pub(super) const OUTPUT: usize = 4;

/// Builds one preset. Add nodes with `add` into a column, set them, wire them, and call `finish`.
/// Nodes in a column are stacked top to bottom in the order they were added, spaced by their
/// real heights, so a patch lays itself out.
pub(super) struct Builder {
    graph: Graph,
    /// The column each node is in, by node index.
    column: Vec<usize>,
    /// The macro knobs' values the preset loads with, from 0 to 1.
    pub(super) macros: [f32; 4],
}

impl Builder {
    pub(super) fn new() -> Self {
        Self {
            graph: Graph::with(Vec::new(), Vec::new()),
            column: Vec::new(),
            macros: [0.0; 4],
        }
    }

    /// Adds a node to a column and returns its index.
    pub(super) fn add(&mut self, kind: NodeKind, column: usize) -> usize {
        self.graph
            .nodes
            .push(FlodeNode::new(kind, Point::new(0.0, 0.0)));
        self.column.push(column);
        self.graph.nodes.len() - 1
    }

    /// Sets a knob to a value in its own units, such as Hz or dB.
    pub(super) fn set(&mut self, node: usize, id: KnobId, plain: f32) {
        self.graph.nodes[node].set_value(id, id.to_normalized(plain));
    }

    /// Picks row `row` of dropdown `which` on a node.
    pub(super) fn choose(&mut self, node: usize, which: usize, row: usize) {
        self.graph.nodes[node].choose(which, row);
    }

    /// Turns switch `index` of a node on or off.
    pub(super) fn switch(&mut self, node: usize, index: usize, on: bool) {
        self.graph.nodes[node].set_switch(index, on);
    }

    /// Folds or opens section `section` of a node. A folded section hides its knobs, so open the
    /// ones the preset sets, as long as the patch still fits the window.
    pub(super) fn fold(&mut self, node: usize, section: usize, folded: bool) {
        self.graph.nodes[node].sections_collapsed[section] = folded;
    }

    /// Sets an oscillator's waveform.
    pub(super) fn waveform(&mut self, oscillator: usize, waveform: Waveform) {
        self.choose(oscillator, 0, waveform.index());
    }

    /// Sets an oscillator's warp mode by the name on its dropdown, such as "PWM".
    pub(super) fn warp(&mut self, oscillator: usize, name: &str) {
        let row = WARP_LABELS
            .iter()
            .position(|label| *label == name)
            .unwrap_or_else(|| panic!("no warp mode called {name}"));
        self.choose(oscillator, 1, row);
    }

    /// Wires an output port to an input port. A wire that is not allowed is a mistake in the
    /// preset, so it panics.
    pub(super) fn wire(&mut self, from: usize, out: Port, to: usize, input: Port) {
        let link = Connection {
            from,
            out,
            to,
            input,
        };
        assert!(link.is_valid(&self.graph.nodes), "bad wire {link:?}");
        self.graph.connect(from, out, to, input);
    }

    /// Wires audio from one node into the first audio input of another.
    pub(super) fn audio(&mut self, from: usize, to: usize) {
        self.wire(from, Port::Audio, to, Port::Audio);
    }

    /// Wires audio into the input of a mix node, `0` for signal A and `1` for signal B.
    pub(super) fn audio_in(&mut self, from: usize, mix: usize, input: usize) {
        self.wire(from, Port::Audio, mix, Port::AudioIn(input));
    }

    /// Wires a node's audio to output slot `slot`. Slots are wired in order from 0.
    pub(super) fn slot(&mut self, from: usize, slot: usize, output: usize) {
        self.wire(from, Port::Audio, output, Port::Slot(slot));
    }

    /// Sets the gain of output slot `slot`, in dB.
    pub(super) fn slot_gain(&mut self, output: usize, slot: usize, db: f32) {
        self.set(output, slot_knob(slot), db);
    }

    /// Wires a MIDI input's notes to an oscillator.
    pub(super) fn notes(&mut self, input: usize, oscillator: usize) {
        self.wire(input, Port::Notes, oscillator, Port::Notes);
    }

    /// Wires an envelope to an oscillator's amplitude.
    pub(super) fn amplitude(&mut self, envelope: usize, oscillator: usize) {
        self.wire(envelope, Port::Shape, oscillator, Port::Shape);
    }

    /// Drives a knob from a control source through a new map node. The source's whole range goes
    /// to the knob's range from `low` to `high`, both in the knob's own units. `source` is the
    /// node and its output pin, such as `(lfo, Port::Shape)` or `(macros, Port::Macro(0))`.
    /// Returns the map's node index.
    pub(super) fn modulate(
        &mut self,
        source: (usize, Port),
        target: usize,
        knob: KnobId,
        range: (f32, f32),
        column: usize,
    ) -> usize {
        let map = self.add(NodeKind::Map, column);
        self.graph.nodes[map].set_value(KnobId::MapMin, knob.to_normalized(range.0));
        self.graph.nodes[map].set_value(KnobId::MapMax, knob.to_normalized(range.1));
        self.wire(source.0, source.1, map, Port::Shape);
        self.wire(map, Port::Shape, target, Port::Knob(knob));
        map
    }

    /// The MIDI input, in the first column. `mono` plays one voice at a time, and `glide` is the
    /// glide time in seconds.
    pub(super) fn voice_input(&mut self, mono: bool, glide: f32) -> usize {
        let input = self.add(NodeKind::Input, CONTROLS);
        self.switch(input, 0, mono);
        self.set(input, KnobId::Glide, glide);
        input
    }

    /// An envelope with these times in seconds and this sustain level from 0 to 1.
    pub(super) fn envelope(
        &mut self,
        column: usize,
        attack: f32,
        decay: f32,
        sustain: f32,
        release: f32,
    ) -> usize {
        let envelope = self.add(NodeKind::Envelope, column);
        self.set(envelope, KnobId::Attack, attack);
        self.set(envelope, KnobId::Decay, decay);
        self.set(envelope, KnobId::Sustain, sustain);
        self.set(envelope, KnobId::Release, release);
        envelope
    }

    /// The output node, in the last column, with its master gain in dB.
    pub(super) fn output(&mut self, gain_db: f32) -> usize {
        let output = self.add(NodeKind::Output, OUTPUT);
        self.set(output, KnobId::Gain, gain_db);
        output
    }

    /// Lays the nodes out, names the preset, and returns it.
    pub(super) fn finish(mut self, name: &str) -> Preset {
        let mut next = [TOP; COLUMN_X.len()];
        for index in 0..self.graph.nodes.len() {
            let column = self.column[index];
            self.graph.nodes[index].position = Point::new(COLUMN_X[column], next[column]);
            next[column] += self.graph.nodes[index].height() + GAP;
        }
        self.graph.preset_name = Some(name.to_string());
        Preset {
            name: name.to_string(),
            category: String::new(),
            graph: self.graph.to_saved(),
            macros: self.macros,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::wavetable::WaveTables;
    use crate::engine::{Engine, Transport};
    use crate::nodes::connection::Connection;
    use crate::patch::Patch;
    use std::sync::Arc;

    const SAMPLE_RATE: usize = 44100;

    fn loaded(preset: &Preset) -> Graph {
        Graph::from_saved(&preset.graph)
    }

    /// Plays `notes` together for `seconds` and returns the peak and whether every sample was
    /// finite. The macros are set the way the preset loads them.
    fn play(preset: &Preset, patch: &Patch, notes: &[u8], seconds: f32) -> (f32, bool) {
        let mut engine = Engine::new(Arc::new(WaveTables::new()));
        engine.begin_block(patch, Transport::default());
        engine.set_macros(preset.macros);
        for &note in notes {
            engine.note_on(note, 1.0);
        }
        let mut peak = 0.0f32;
        let mut finite = true;
        for n in 0..(seconds * SAMPLE_RATE as f32) as usize {
            let frame = engine.process_sample(n);
            finite &= frame.l.is_finite() && frame.r.is_finite();
            peak = peak.max(frame.peak());
        }
        (peak, finite)
    }

    /// The presets the checks run on. Set `FLODE_PRESET` to a preset's name, such as `Reese`, to
    /// check that one alone while it is being built.
    fn presets() -> Vec<Preset> {
        let only = std::env::var("FLODE_PRESET").ok();
        all()
            .into_iter()
            .filter(|preset| only.as_deref().is_none_or(|name| preset.name == name))
            .collect()
    }

    #[test]
    fn the_presets_have_distinct_clean_names() {
        let presets = all();
        assert_eq!(presets.len(), 7);
        for (index, preset) in presets.iter().enumerate() {
            assert!(!preset.name.is_empty() && preset.name == preset.name.trim());
            assert!(presets[..index].iter().all(|p| p.name != preset.name));
        }
        // Three basses, two leads, two pads, each filed in the folder that names it.
        for (kind, expected) in [("Bass", 3), ("Lead", 2), ("Pad", 2)] {
            let count = presets.iter().filter(|p| p.category == kind).count();
            assert_eq!(count, expected, "{kind}");
        }
    }

    /// Loading a preset keeps every wire, so each one compiles to the sound it was built for.
    #[test]
    fn every_preset_loads_without_losing_a_wire() {
        for preset in presets() {
            let wires = |graph: &Graph| graph.connections.len();
            let built = preset.graph.links.len();
            assert_eq!(wires(&loaded(&preset)), built, "{}", preset.name);
            assert!(built > 0, "{}", preset.name);
        }
    }

    /// Every oscillator takes its notes from a MIDI input, and the preset plays.
    #[test]
    fn every_preset_is_wired_to_play() {
        for preset in presets() {
            let graph = loaded(&preset);
            for (index, node) in graph.nodes.iter().enumerate() {
                if node.kind == NodeKind::Oscillator {
                    let fed = graph.connections.iter().any(|c: &Connection| {
                        c.to == index
                            && c.input == Port::Notes
                            && graph.nodes[c.from].kind == NodeKind::Input
                    });
                    assert!(fed, "{}: oscillator {index} has no MIDI input", preset.name);
                }
            }
            assert!(graph.patch().lanes[0].is_active(), "{}", preset.name);
            assert_eq!(
                graph
                    .nodes
                    .iter()
                    .filter(|n| n.kind == NodeKind::Output)
                    .count(),
                1,
                "{}",
                preset.name
            );
        }
    }

    /// One note is audible but not loud, and a chord of three does not clip. The output meter
    /// reads these peaks, so they are the levels a player sees.
    #[test]
    fn every_preset_plays_at_a_sane_level() {
        for preset in presets() {
            let patch = loaded(&preset).patch();
            let (single, finite) = play(&preset, &patch, &[48], 3.0);
            assert!(finite, "{}", preset.name);
            assert!(
                (0.2..=0.8).contains(&single),
                "{}: one note peaks at {single}",
                preset.name
            );
            let (chord, finite) = play(&preset, &patch, &[48, 55, 60], 3.0);
            assert!(finite, "{}", preset.name);
            assert!(chord <= 0.98, "{}: a chord peaks at {chord}", preset.name);
        }
    }

    /// Every node is inside the default window's width, none overlaps another, and each node's
    /// header and first row sit above the key strip. The viewers make some nodes tall enough that
    /// the bottom of a node can run under the keys, and the canvas pans to reach it.
    #[test]
    fn every_preset_fits_the_window() {
        let strip_top = 712.0 - crate::editor::piano::STRIP_HEIGHT;
        for preset in presets() {
            let graph = loaded(&preset);
            let bounds: Vec<_> = graph.nodes.iter().map(|n| n.bounds()).collect();
            for (i, a) in bounds.iter().enumerate() {
                assert!(
                    a.x >= 0.0 && a.x + a.width <= 1024.0,
                    "{}: node {i} is off the side",
                    preset.name
                );
                assert!(
                    a.y + a
                        .height
                        .min(crate::nodes::HEADER_HEIGHT + crate::nodes::ROW_PITCH)
                        <= strip_top,
                    "{}: node {i} starts under the keys",
                    preset.name
                );
                for b in &bounds[i + 1..] {
                    let apart = a.x + a.width <= b.x
                        || b.x + b.width <= a.x
                        || a.y + a.height <= b.y
                        || b.y + b.height <= a.y;
                    assert!(apart, "{}: {a:?} overlaps {b:?}", preset.name);
                }
            }
        }
    }

    /// Writes the factory presets into the preset folder, each in its category's folder. It
    /// replaces presets of the same names and leaves any others alone.
    #[test]
    #[ignore = "writes into the user's preset folder"]
    fn write_factory_presets() {
        let dir = crate::presets::directory().expect("a home folder");
        for preset in all() {
            // These were first written with no category, straight into the folder. The copy
            // there would show up twice, so it goes.
            let stale = dir.join(format!("{}.{}", preset.name, crate::presets::EXTENSION));
            let _ = std::fs::remove_file(stale);
            crate::presets::save_in(&dir, &preset).unwrap();
            println!("wrote {}/{}", preset.category, preset.name);
        }
    }
}
