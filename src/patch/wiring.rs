//! A read-only view of the node graph: the nodes and the wires between them, and what the wires
//! mean. It answers which control route drives a knob, which envelope shapes an oscillator, and
//! what audio stages each output slot plays. The editor draws from it, and the compiler turns it
//! into the patch for the audio thread. It holds no editing state.

use super::control::{
    ControlLevels, KnobValue, MapStage, Param, Route, Source, MAX_ENVELOPES, MAX_LFOS, MAX_MAPS,
};
use super::stage::{
    FilterParams, LaneGraph, MixParams, OscillatorParams, StageParams, StageSpec, MAX_INPUTS,
    MAX_STAGES,
};
use super::{EnvelopeSettings, Globals, LfoSettings, Patch};
use crate::nodes::connection::Connection;
use crate::nodes::knobs::KnobId;
use crate::nodes::output::{slot_knob, MAX_SLOTS};
use crate::nodes::{filter, lfo, map, mix, oscillator, FlodeNode, NodeKind, Port};
use arrayvec::ArrayVec;

/// The nodes and wires of a graph, borrowed for a query.
#[derive(Clone, Copy)]
pub struct Wiring<'a> {
    nodes: &'a [FlodeNode],
    links: &'a [Connection],
}

impl<'a> Wiring<'a> {
    pub fn new(nodes: &'a [FlodeNode], links: &'a [Connection]) -> Self {
        Self { nodes, links }
    }

    /// The wire into an input port. An input holds one wire.
    pub fn input_link(&self, node: usize, port: Port) -> Option<&'a Connection> {
        self.links.iter().find(|c| c.to == node && c.input == port)
    }

    /// A knob as the audio thread reads it: the node's own value and the route that drives it.
    pub fn knob_value(&self, node: usize, id: KnobId) -> KnobValue {
        KnobValue {
            value: self.nodes[node].value(id),
            route: self.control_route(node, id),
        }
    }

    /// A knob as a stage reads it.
    pub fn param(&self, node: usize, id: KnobId) -> Param {
        Param::new(id, self.knob_value(node, id))
    }

    /// True if a control wire drives this knob. A driven knob can't be dragged.
    pub fn is_driven(&self, node: usize, id: KnobId) -> bool {
        self.control_route(node, id).is_some()
    }

    /// True if any knob in the graph is driven by a control wire.
    pub fn any_driven(&self) -> bool {
        self.nodes.iter().enumerate().any(|(index, node)| {
            node.rows()
                .iter()
                .filter_map(|row| row.knob)
                .any(|id| self.is_driven(index, id))
        })
    }

    /// The control path to a knob: a source, and the maps its level passes through on the way.
    /// The path is only valid if it ends at an envelope, an LFO, or a MIDI source. Returns None
    /// for a knob without one.
    pub fn control_route(&self, node: usize, id: KnobId) -> Option<Route> {
        let mut link = self
            .links
            .iter()
            .find(|c| c.out.is_control_out() && c.to == node && c.input == Port::Knob(id))?;
        // The maps on the path, from the knob back toward the source.
        let mut maps: Vec<usize> = Vec::new();
        // A graph can't have more links than nodes, so a longer walk means a loop.
        for _ in 0..self.nodes.len() {
            let source = link.from;
            match self.nodes[source].kind {
                NodeKind::Envelope => {
                    let index = self.envelope_index(source)?;
                    return self.compose(&maps, Source::Envelope(index));
                }
                NodeKind::Lfo => {
                    let index = self.lfo_index(source)?;
                    return self.compose(&maps, Source::Lfo(index));
                }
                NodeKind::Input => {
                    // An input node's pins are the MIDI sources, so the wire says which one.
                    let Port::Midi(midi) = link.out else {
                        return None;
                    };
                    return self.compose(&maps, Source::Midi(midi));
                }
                NodeKind::Map => {
                    maps.push(source);
                    link = self.input_link(source, Port::Shape)?;
                }
                _ => return None,
            }
        }
        None
    }

    /// Applies the maps to a source in signal order, starting from the source.
    fn compose(&self, maps: &[usize], source: Source) -> Option<Route> {
        if maps.len() > MAX_MAPS {
            return None;
        }
        // The maps are stored from the knob back toward the source, so reverse them to signal order.
        let mut stages = [MapStage::IDENTITY; MAX_MAPS];
        for (slot, &index) in maps.iter().rev().enumerate() {
            stages[slot] = map::stage(&self.nodes[index]);
        }
        Some(Route {
            source,
            stages,
            count: maps.len(),
        })
    }

    /// The level of the signal on a control output, as the live control levels give it. Maps on
    /// the way apply their curves.
    pub fn control_level(
        &self,
        from: usize,
        out: Port,
        levels: &ControlLevels,
        depth: usize,
    ) -> Option<f32> {
        if depth > self.nodes.len() {
            return None;
        }
        match self.nodes.get(from)?.kind {
            NodeKind::Envelope => {
                let index = self.envelope_index(from)?;
                Some(levels.envelope[index])
            }
            NodeKind::Lfo => self.lfo_index(from).map(|index| levels.lfo[index]),
            NodeKind::Input => match out {
                Port::Midi(source) => Some(levels.midi[source.index()]),
                _ => None,
            },
            NodeKind::Map => {
                let link = self.input_link(from, Port::Shape)?;
                let input = self.control_level(link.from, link.out, levels, depth + 1)?;
                Some(map::stage(&self.nodes[from]).apply(input))
            }
            _ => None,
        }
    }

    /// The level arriving at a map's input, if a control source feeds it.
    pub fn map_input_level(&self, node: usize, levels: &ControlLevels) -> Option<f32> {
        let link = self.input_link(node, Port::Shape)?;
        self.control_level(link.from, link.out, levels, 0)
    }

    /// The index of the envelope wired to an oscillator's input, if one is.
    pub fn amp_envelope(&self, oscillator: usize) -> Option<usize> {
        let link = self.input_link(oscillator, Port::Shape)?;
        if self.nodes.get(link.from)?.kind != NodeKind::Envelope {
            return None;
        }
        self.envelope_index(link.from)
    }

    /// The index of an envelope among the envelope nodes in graph order, or None past the
    /// envelopes the audio thread runs.
    pub fn envelope_index(&self, node: usize) -> Option<usize> {
        let index = self.nodes[..node]
            .iter()
            .filter(|n| n.kind == NodeKind::Envelope)
            .count();
        (index < MAX_ENVELOPES).then_some(index)
    }

    /// The envelopes the audio thread runs, in graph order. Each one's knob values are read per
    /// block, so an envelope can have its times driven too.
    pub fn envelope_settings(&self) -> [Option<EnvelopeSettings>; MAX_ENVELOPES] {
        let mut out = [None; MAX_ENVELOPES];
        let envelopes = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.kind == NodeKind::Envelope);
        for (slot, (index, _)) in envelopes.take(MAX_ENVELOPES).enumerate() {
            out[slot] = Some(EnvelopeSettings {
                attack: self.param(index, KnobId::Attack),
                decay: self.param(index, KnobId::Decay),
                sustain: self.param(index, KnobId::Sustain),
                release: self.param(index, KnobId::Release),
            });
        }
        out
    }

    /// The index of an LFO among the LFO nodes in graph order, or None past the LFOs the audio
    /// thread runs.
    pub fn lfo_index(&self, node: usize) -> Option<usize> {
        let index = self.nodes[..node]
            .iter()
            .filter(|n| n.kind == NodeKind::Lfo)
            .count();
        (index < MAX_LFOS).then_some(index)
    }

    /// The LFOs the audio thread runs, in graph order.
    pub fn lfo_settings(&self) -> [Option<LfoSettings>; MAX_LFOS] {
        let mut out = [None; MAX_LFOS];
        let lfos = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.kind == NodeKind::Lfo);
        for (slot, (index, node)) in lfos.take(MAX_LFOS).enumerate() {
            out[slot] = Some(LfoSettings {
                waveform: lfo::waveform(node),
                rate: self.param(index, KnobId::LfoRate),
                sync: self.param(index, KnobId::LfoSync),
            });
        }
        out
    }

    /// The first output node, which decides what the plugin plays.
    pub fn first_output(&self) -> Option<usize> {
        self.nodes.iter().position(|n| n.kind == NodeKind::Output)
    }

    /// The settings of the output node that apply to every slot.
    pub fn globals(&self) -> Globals {
        match self.first_output() {
            Some(output) => Globals {
                mono: self.nodes[output].switch(0),
                glide_always: self.nodes[output].switch(1),
                gain: self.param(output, KnobId::Gain),
                glide: self.param(output, KnobId::Glide),
            },
            None => Globals::default(),
        }
    }

    /// Compiles the whole graph for the audio thread.
    pub fn patch(&self) -> Patch {
        Patch {
            lanes: self.lane_graphs(),
            envelopes: self.envelope_settings(),
            lfos: self.lfo_settings(),
            globals: self.globals(),
        }
    }

    /// The audio stages behind each output slot of the first output node. A slot with nothing
    /// wired in is silent.
    pub fn lane_graphs(&self) -> [LaneGraph; MAX_SLOTS] {
        let output = self.first_output();
        std::array::from_fn(|slot| match output {
            Some(output) if slot < self.nodes[output].used => self.lane_graph(output, slot),
            _ => LaneGraph::silent(self.slot_gain(output, slot)),
        })
    }

    /// The gain knob of a slot, as a param.
    fn slot_gain(&self, output: Option<usize>, slot: usize) -> Param {
        let id = slot_knob(slot);
        match output {
            Some(output) => self.param(output, id),
            None => Param::default_of(id),
        }
    }

    /// Compiles what is wired into one slot of `output` to an arena of stages in signal order.
    /// Each audio node becomes a stage, and each audio input finds its stage by position. A
    /// stage fed by two inputs from one node is placed once. An input with no wire, a wire to a
    /// node that makes no audio, or a loop makes silence on that input. A slot that needs more
    /// than `MAX_STAGES` stages is silent.
    fn lane_graph(&self, output: usize, slot: usize) -> LaneGraph {
        let mut lane = LaneGraph::silent(self.slot_gain(Some(output), slot));
        let Some(link) = self.input_link(output, Port::Slot(slot)) else {
            return lane;
        };
        let mut builder = LaneBuilder::default();
        if builder.place(self, link.from).is_some() && !builder.overflowed {
            lane.stages = builder.stages;
        }
        lane
    }

    /// The stage a node makes, or None for a node that makes no audio. Every audio node is
    /// listed here once.
    fn stage_params(&self, node: usize) -> Option<StageParams> {
        let n = &self.nodes[node];
        let knob = |id| self.param(node, id);
        match n.kind {
            NodeKind::Oscillator => Some(StageParams::Oscillator(OscillatorParams {
                waveform: oscillator::waveform(n),
                noise: oscillator::noise_type(n),
                warp: oscillator::warp_mode(n),
                envelope: self.amp_envelope(node),
                coarse: knob(KnobId::Coarse),
                fine: knob(KnobId::Fine),
                octave: knob(KnobId::Octave),
                unison: knob(KnobId::Unison),
                detune: knob(KnobId::Detune),
                phase_random: knob(KnobId::PhaseRandom),
                warp_amount: knob(KnobId::WarpAmount),
                level: knob(KnobId::OscGain),
                pan: knob(KnobId::Pan),
                spread: knob(KnobId::Spread),
            })),
            NodeKind::Filter => Some(StageParams::Filter(FilterParams {
                kind: filter::filter_type(n),
                cutoff: knob(KnobId::Cutoff),
                resonance: knob(KnobId::Resonance),
            })),
            NodeKind::Mix => Some(StageParams::Mix(MixParams {
                mode: mix::mix_mode(n),
                amount: knob(KnobId::Mix),
            })),
            NodeKind::Map
            | NodeKind::Output
            | NodeKind::Lfo
            | NodeKind::Input
            | NodeKind::Envelope => None,
        }
    }
}

/// The arena of one slot while it is built.
#[derive(Default)]
struct LaneBuilder {
    stages: ArrayVec<StageSpec, MAX_STAGES>,
    /// The arena position of each node already placed, so a node feeding two inputs runs once.
    placed: Vec<(usize, u8)>,
    /// The nodes on the path being placed, to spot a loop.
    path: Vec<usize>,
    /// Set when the slot needs more stages than fit.
    overflowed: bool,
}

impl LaneBuilder {
    /// Places a node's stage after the stages that feed it, and returns its arena position. None
    /// means the node makes no audio, is part of a loop, or would not fit.
    fn place(&mut self, wiring: &Wiring, node: usize) -> Option<u8> {
        if let Some(&(_, position)) = self.placed.iter().find(|(placed, _)| *placed == node) {
            return Some(position);
        }
        if self.path.contains(&node) {
            return None;
        }
        let params = wiring.stage_params(node)?;

        self.path.push(node);
        let mut inputs = [None; MAX_INPUTS];
        let ports = wiring.nodes[node]
            .input_ports()
            .into_iter()
            .filter(|port| port.is_audio());
        for (input, port) in ports.take(params.input_count()).enumerate() {
            if let Some(link) = wiring.input_link(node, port) {
                inputs[input] = self.place(wiring, link.from);
            }
        }
        self.path.pop();

        let position = self.stages.len() as u8;
        if self.stages.try_push(StageSpec { params, inputs }).is_err() {
            self.overflowed = true;
            return None;
        }
        self.placed.push((node, position));
        Some(position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::mix::MixMode;
    use crate::dsp::warp::WarpMode;
    use nih_plug_iced::Point;

    fn node(kind: NodeKind) -> FlodeNode {
        FlodeNode::new(kind, Point::new(0.0, 0.0))
    }

    fn audio(from: usize, to: usize, input: Port) -> Connection {
        Connection {
            from,
            out: Port::Audio,
            to,
            input,
        }
    }

    /// The lane for slot 0 of a graph whose output is the node at `output`.
    fn lane(mut nodes: Vec<FlodeNode>, links: Vec<Connection>, output: usize) -> LaneGraph {
        nodes[output].used = 1;
        Wiring::new(&nodes, &links).lane_graphs()[0].clone()
    }

    /// Every stage reads only stages placed before it.
    fn in_signal_order(lane: &LaneGraph) -> bool {
        lane.stages.iter().enumerate().all(|(index, spec)| {
            spec.inputs
                .iter()
                .flatten()
                .all(|&source| (source as usize) < index)
        })
    }

    #[test]
    fn an_oscillator_into_the_output_is_one_stage() {
        let nodes = vec![node(NodeKind::Oscillator), node(NodeKind::Output)];
        let lane = lane(nodes, vec![audio(0, 1, Port::Slot(0))], 1);
        assert_eq!(lane.stages.len(), 1);
        assert!(matches!(lane.stages[0].params, StageParams::Oscillator(_)));
        assert!(lane.is_active());
    }

    #[test]
    fn a_filter_comes_after_the_oscillator_that_feeds_it() {
        let nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Filter),
            node(NodeKind::Output),
        ];
        let links = vec![audio(0, 1, Port::Audio), audio(1, 2, Port::Slot(0))];
        let lane = lane(nodes, links, 2);
        assert_eq!(lane.stages.len(), 2);
        assert!(matches!(lane.stages[1].params, StageParams::Filter(_)));
        assert_eq!(lane.stages[1].inputs, [Some(0), None]);
        assert!(in_signal_order(&lane));
    }

    /// A filter sits inside one branch of a mix. The old chain could not compile this.
    #[test]
    fn a_filter_can_sit_inside_a_mix_branch() {
        let nodes = vec![
            node(NodeKind::Oscillator), // 0: branch A's source
            node(NodeKind::Oscillator), // 1: branch B's source
            node(NodeKind::Filter),     // 2: filters A
            node(NodeKind::Mix),        // 3
            node(NodeKind::Output),     // 4
        ];
        let links = vec![
            audio(0, 2, Port::Audio),
            audio(2, 3, Port::AudioIn(0)),
            audio(1, 3, Port::AudioIn(1)),
            audio(3, 4, Port::Slot(0)),
        ];
        let lane = lane(nodes, links, 4);
        assert_eq!(lane.stages.len(), 4);
        assert!(in_signal_order(&lane));
        let root = lane.root().unwrap();
        assert!(matches!(root.params, StageParams::Mix(_)));
        let [a, b] = root.inputs.map(|i| &lane.stages[i.unwrap() as usize]);
        assert!(matches!(a.params, StageParams::Filter(_)));
        assert!(matches!(b.params, StageParams::Oscillator(_)));
    }

    /// Every filter in a row is a stage. The old chain ran only the first it found.
    #[test]
    fn filters_in_series_all_compile() {
        let nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Filter),
            node(NodeKind::Filter),
            node(NodeKind::Output),
        ];
        let links = vec![
            audio(0, 1, Port::Audio),
            audio(1, 2, Port::Audio),
            audio(2, 3, Port::Slot(0)),
        ];
        let lane = lane(nodes, links, 3);
        assert_eq!(lane.stages.len(), 3);
        assert_eq!(lane.stages[2].inputs[0], Some(1));
    }

    /// One oscillator wired to both mix inputs is one stage that feeds both.
    #[test]
    fn a_node_feeding_two_inputs_is_placed_once() {
        let nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Mix),
            node(NodeKind::Output),
        ];
        let links = vec![
            audio(0, 1, Port::AudioIn(0)),
            audio(0, 1, Port::AudioIn(1)),
            audio(1, 2, Port::Slot(0)),
        ];
        let lane = lane(nodes, links, 2);
        assert_eq!(lane.stages.len(), 2);
        assert_eq!(lane.stages[1].inputs, [Some(0), Some(0)]);
    }

    #[test]
    fn an_unwired_input_is_none() {
        let nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Mix),
            node(NodeKind::Output),
        ];
        let links = vec![audio(0, 1, Port::AudioIn(0)), audio(1, 2, Port::Slot(0))];
        let lane = lane(nodes, links, 2);
        assert_eq!(lane.stages[1].inputs, [Some(0), None]);
    }

    /// Two filters wired in a loop make no sound and no infinite walk.
    #[test]
    fn a_loop_compiles_to_silence_without_hanging() {
        let nodes = vec![
            node(NodeKind::Filter),
            node(NodeKind::Filter),
            node(NodeKind::Output),
        ];
        let links = vec![
            audio(0, 1, Port::Audio),
            audio(1, 0, Port::Audio),
            audio(1, 2, Port::Slot(0)),
        ];
        let lane = lane(nodes, links, 2);
        // Both filters are placed, and the one that closes the loop has no input.
        assert_eq!(lane.stages.len(), 2);
        assert_eq!(lane.stages[0].inputs, [None, None]);
        assert!(in_signal_order(&lane));
    }

    /// A slot that needs more stages than fit goes silent instead of playing part of its chain.
    #[test]
    fn too_many_stages_make_a_silent_slot() {
        let mut nodes = vec![node(NodeKind::Oscillator)];
        let mut links = Vec::new();
        for i in 0..MAX_STAGES {
            nodes.push(node(NodeKind::Filter));
            links.push(audio(i, i + 1, Port::Audio));
        }
        nodes.push(node(NodeKind::Output));
        links.push(audio(MAX_STAGES, MAX_STAGES + 1, Port::Slot(0)));
        let lane = lane(nodes, links, MAX_STAGES + 1);
        assert!(!lane.is_active());
    }

    /// An envelope wired to the oscillator's input shapes its amplitude.
    #[test]
    fn a_wired_envelope_reaches_the_oscillator() {
        let nodes = vec![
            node(NodeKind::Envelope),
            node(NodeKind::Oscillator),
            node(NodeKind::Output),
        ];
        let links = vec![
            Connection {
                from: 0,
                out: Port::Shape,
                to: 1,
                input: Port::Shape,
            },
            audio(1, 2, Port::Slot(0)),
        ];
        let lane = lane(nodes, links, 2);
        let StageParams::Oscillator(osc) = lane.stages[0].params else {
            panic!("not an oscillator");
        };
        assert_eq!(osc.envelope, Some(0));
    }

    /// An LFO wired to a filter's cutoff drives the stage's param from the live level.
    #[test]
    fn a_driven_knob_follows_its_source() {
        let nodes = vec![
            node(NodeKind::Lfo),
            node(NodeKind::Oscillator),
            node(NodeKind::Filter),
            node(NodeKind::Output),
        ];
        let links = vec![
            Connection {
                from: 0,
                out: Port::Shape,
                to: 2,
                input: Port::Knob(KnobId::Cutoff),
            },
            audio(1, 2, Port::Audio),
            audio(2, 3, Port::Slot(0)),
        ];
        let lane = lane(nodes, links, 3);
        let StageParams::Filter(filter) = lane.stages[1].params else {
            panic!("not a filter");
        };
        let mut levels = ControlLevels::default();
        levels.lfo[0] = 1.0;
        assert_eq!(filter.cutoff.plain(&levels), 20000.0);
        levels.lfo[0] = 0.0;
        assert_eq!(filter.cutoff.plain(&levels), 20.0);
    }

    /// A mix stage carries its dropdown and its knob.
    #[test]
    fn the_mix_mode_and_amount_reach_the_stage() {
        let mut mix_node = node(NodeKind::Mix);
        mix_node.choices = vec![2];
        mix_node.set_value(KnobId::Mix, KnobId::Mix.to_normalized(0.75));
        let nodes = vec![node(NodeKind::Oscillator), mix_node, node(NodeKind::Output)];
        let links = vec![audio(0, 1, Port::AudioIn(0)), audio(1, 2, Port::Slot(0))];
        let lane = lane(nodes, links, 2);
        let StageParams::Mix(mix) = lane.stages[1].params else {
            panic!("not a mix");
        };
        assert_eq!(mix.mode, MixMode::RingMod);
        assert_eq!(mix.amount.plain(&ControlLevels::default()), 0.75);
    }

    /// Slots past the used ones, and a graph with no output, are silent.
    #[test]
    fn unused_slots_and_a_missing_output_are_silent() {
        let nodes = vec![node(NodeKind::Oscillator), node(NodeKind::Output)];
        let links = vec![audio(0, 1, Port::Slot(0))];
        let mut with_output = nodes;
        with_output[1].used = 1;
        let lanes = Wiring::new(&with_output, &links).lane_graphs();
        assert!(lanes[0].is_active());
        assert!(lanes[1..].iter().all(|lane| !lane.is_active()));

        let no_output = vec![node(NodeKind::Oscillator)];
        let lanes = Wiring::new(&no_output, &[]).lane_graphs();
        assert!(lanes.iter().all(|lane| !lane.is_active()));
    }

    /// A node that makes audio has as many audio inputs as its stage reads.
    #[test]
    fn each_audio_node_wires_as_many_inputs_as_its_stage_reads() {
        for kind in [NodeKind::Oscillator, NodeKind::Filter, NodeKind::Mix] {
            let nodes = vec![node(kind)];
            let wiring = Wiring::new(&nodes, &[]);
            let params = wiring.stage_params(0).expect("an audio node makes a stage");
            let ports = nodes[0].input_ports();
            let audio_ports = ports.iter().filter(|port| port.is_audio()).count();
            assert_eq!(audio_ports, params.input_count(), "{kind:?}");
        }
    }

    /// One oscillator wired to another's FM input feeds it, and the modulator is placed first.
    #[test]
    fn an_oscillator_can_feed_anothers_fm_input() {
        let mut carrier = node(NodeKind::Oscillator);
        carrier.choose(1, WarpMode::Fm as usize);
        let nodes = vec![node(NodeKind::Oscillator), carrier, node(NodeKind::Output)];
        let links = vec![audio(0, 1, Port::AudioIn(0)), audio(1, 2, Port::Slot(0))];
        let lane = lane(nodes, links, 2);
        assert_eq!(lane.stages.len(), 2);
        assert_eq!(lane.stages[1].inputs, [Some(0), None]);
        assert!(in_signal_order(&lane));
    }

    /// The same wire does nothing once the warp is not FM, and the modulator drops out of the
    /// slot, since nothing else reads it.
    #[test]
    fn the_fm_wire_is_ignored_under_other_warps() {
        let nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Oscillator),
            node(NodeKind::Output),
        ];
        let links = vec![audio(0, 1, Port::AudioIn(0)), audio(1, 2, Port::Slot(0))];
        let lane = lane(nodes, links, 2);
        assert_eq!(lane.stages.len(), 1);
        assert_eq!(lane.stages[0].inputs, [None, None]);
    }

    /// A modulator can also play: wired to the FM input and to a mix, it runs once and feeds both.
    #[test]
    fn a_modulator_can_also_be_heard() {
        let mut carrier = node(NodeKind::Oscillator);
        carrier.choose(1, WarpMode::Fm as usize);
        let nodes = vec![
            node(NodeKind::Oscillator), // 0: modulator
            carrier,                    // 1
            node(NodeKind::Mix),        // 2
            node(NodeKind::Output),     // 3
        ];
        let links = vec![
            audio(0, 1, Port::AudioIn(0)),
            audio(0, 2, Port::AudioIn(0)),
            audio(1, 2, Port::AudioIn(1)),
            audio(2, 3, Port::Slot(0)),
        ];
        let lane = lane(nodes, links, 3);
        assert_eq!(lane.stages.len(), 3);
        assert_eq!(lane.stages[1].inputs[0], Some(0));
        assert_eq!(lane.stages[2].inputs, [Some(0), Some(1)]);
    }

    /// A node that makes no audio makes no stage.
    #[test]
    fn control_nodes_make_no_stage() {
        for kind in [
            NodeKind::Envelope,
            NodeKind::Lfo,
            NodeKind::Input,
            NodeKind::Map,
            NodeKind::Output,
        ] {
            let nodes = vec![node(kind)];
            assert!(
                Wiring::new(&nodes, &[]).stage_params(0).is_none(),
                "{kind:?}"
            );
        }
    }
}
