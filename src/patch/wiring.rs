//! A read-only view of the node graph: the nodes and the wires between them, and what the wires
//! mean. It answers which control route drives a knob, which envelope shapes an oscillator, and
//! what audio stages each output slot plays. The editor draws from it, and the compiler turns it
//! into the patch for the audio thread. It holds no editing state.

use super::control::{
    ControlLevels, KnobValue, MapNode, MapValue, MathNode, Param, Route, SignalNode, SignalTable,
    Source, MAX_ENVELOPES, MAX_LFOS,
};
use super::stage::{
    ChorusParams, CompressorParams, DelayParams, DistortionParams, EqBandParams, EqParams,
    FilterParams, FlangerParams, LaneGraph, MixParams, MonitorParams, OscillatorParams,
    PhaserParams, ReverbParams, StageParams, StageSpec, MAX_INPUTS, MAX_STAGES,
};
use super::{EnvelopeSettings, Globals, LfoSettings, Patch};
use crate::nodes::connection::Connection;
use crate::nodes::knobs::KnobId;
use crate::nodes::output::{slot_knob, MAX_SLOTS};
use crate::nodes::{
    compressor, delay, distortion, eq, filter, input, lfo, map, math, mix, monitor, oscillator,
    phaser, Bypass, FlodeNode, NodeKind, Port,
};
use arrayvec::ArrayVec;
use std::cell::OnceCell;

/// The nodes and wires of a graph, borrowed for a query.
#[derive(Clone)]
pub struct Wiring<'a> {
    nodes: &'a [FlodeNode],
    links: &'a [Connection],
    /// The patch's map and math nodes, worked out the first time a query needs one.
    signals: OnceCell<SignalBuild>,
}

/// The map and math nodes of a graph as the patch runs them, and which node is which.
#[derive(Clone, Default)]
struct SignalBuild {
    table: SignalTable,
    /// The graph node and its place in `table`, for each one that makes a signal.
    nodes: Vec<(usize, u8)>,
}

impl SignalBuild {
    fn index_of(&self, node: usize) -> Option<u8> {
        self.nodes
            .iter()
            .find(|(found, _)| *found == node)
            .map(|&(_, at)| at)
    }
}

impl<'a> Wiring<'a> {
    pub fn new(nodes: &'a [FlodeNode], links: &'a [Connection]) -> Self {
        Self {
            nodes,
            links,
            signals: OnceCell::new(),
        }
    }

    /// The wire into an input port. An input holds one wire.
    pub fn input_link(&self, node: usize, port: Port) -> Option<&'a Connection> {
        self.links.iter().find(|c| c.to == node && c.input == port)
    }

    /// True if `node` makes a stage that runs on the bus: it is an effect, or an effect feeds it,
    /// directly or through other audio nodes. This follows the same rules as the compiler. A node
    /// that is switched off makes no stage, so a muted source has no effect to pass on, and a
    /// bypassed processor is whatever feeds its first audio input.
    pub fn runs_on_bus(&self, node: usize) -> bool {
        self.bus_from(node, &mut Vec::new())
    }

    fn bus_from(&self, node: usize, path: &mut Vec<usize>) -> bool {
        if path.contains(&node) {
            return false;
        }
        let this = &self.nodes[node];
        let follow_first_only = match (this.bypassed, this.bypass()) {
            (true, Bypass::Mute) => return false,
            (true, Bypass::PassThrough) => true,
            _ => {
                if self.stage_params(node).is_some_and(|p| p.is_effect()) {
                    return true;
                }
                false
            }
        };
        path.push(node);
        let mut ports = this
            .input_ports()
            .into_iter()
            .filter(|port| port.is_audio());
        let found = if follow_first_only {
            ports
                .next()
                .into_iter()
                .any(|port| self.feeds_bus(node, port, path))
        } else {
            ports.any(|port| self.feeds_bus(node, port, path))
        };
        path.pop();
        found
    }

    /// True if the node wired into `port` of `node` runs on the bus.
    fn feeds_bus(&self, node: usize, port: Port, path: &mut Vec<usize>) -> bool {
        self.input_link(node, port)
            .is_some_and(|link| self.bus_from(link.from, path))
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

    /// The control route to a knob: the source that drives it. The source can be an envelope, an
    /// LFO, a MIDI source, a macro, a map, or a math node, and a map or math node takes its own
    /// input from any of those, so the chain behind a knob can be as long as the graph. Returns
    /// None for a knob with no wire, or one whose chain does not end at a source.
    pub fn control_route(&self, node: usize, id: KnobId) -> Option<Route> {
        let link = self.control_link(node, id)?;
        self.source_at(link.from, link.out)
            .map(|source| Route { source })
    }

    /// The control wire into a knob's pin.
    fn control_link(&self, node: usize, id: KnobId) -> Option<&'a Connection> {
        self.links
            .iter()
            .find(|c| c.out.is_control_out() && c.to == node && c.input == Port::Knob(id))
    }

    /// The source on a control output, or None if the output makes no signal. A map or math node
    /// is the one the patch runs, found by its node.
    fn source_at(&self, from: usize, out: Port) -> Option<Source> {
        match self.nodes.get(from)?.kind {
            NodeKind::Map | NodeKind::Math => self.signals().index_of(from).map(Source::Signal),
            _ => self.leaf_source(from, out),
        }
    }

    /// The source a node makes, for every kind of node that is a source and not a map or math
    /// node.
    fn leaf_source(&self, from: usize, out: Port) -> Option<Source> {
        match self.nodes.get(from)?.kind {
            NodeKind::Envelope => self.envelope_index(from).map(|i| Source::Envelope(i as u8)),
            NodeKind::Lfo => self.lfo_index(from).map(|i| Source::Lfo(i as u8)),
            // An input node's pins are the MIDI sources, so the wire says which one.
            NodeKind::Input => match out {
                Port::Midi(midi) => Some(Source::Midi(midi)),
                _ => None,
            },
            NodeKind::Macro => match out {
                Port::Macro(index) => Some(Source::Macro(index as u8)),
                _ => None,
            },
            _ => None,
        }
    }

    /// The patch's map and math nodes. They are worked out once for this view of the graph.
    fn signals(&self) -> &SignalBuild {
        self.signals.get_or_init(|| {
            let mut build = SignalBuild::default();
            let mut path = Vec::new();
            for node in 0..self.nodes.len() {
                if matches!(self.nodes[node].kind, NodeKind::Map | NodeKind::Math) {
                    self.compile_signal(node, &mut build, &mut path);
                }
            }
            build
        })
    }

    /// Adds map or math node `node` to the table, after the nodes that feed it, and returns its
    /// place. A node already added keeps its place. None means the node makes no signal: a map
    /// has nothing wired to its input, what feeds it is not a source, it is part of a loop, or the
    /// table is full. A math node needs no wire, since an input with none is the knob's number.
    fn compile_signal(
        &self,
        node: usize,
        build: &mut SignalBuild,
        path: &mut Vec<usize>,
    ) -> Option<u8> {
        if let Some(at) = build.index_of(node) {
            return Some(at);
        }
        if path.contains(&node) {
            return None;
        }
        path.push(node);
        // What a wire into `from` carries: the place in the table of a map or math node, which
        // compiles it first, or any other source.
        let mut source_from = |from: usize, out: Port| match self.nodes[from].kind {
            NodeKind::Map | NodeKind::Math => {
                self.compile_signal(from, build, path).map(Source::Signal)
            }
            _ => self.leaf_source(from, out),
        };
        let value = |id: KnobId, source_from: &mut dyn FnMut(usize, Port) -> Option<Source>| {
            MapValue {
                value: self.nodes[node].plain(id),
                // A value with a wire that makes no signal holds its own number.
                source: self
                    .control_link(node, id)
                    .and_then(|link| source_from(link.from, link.out)),
            }
        };
        let made: Option<SignalNode> = match self.nodes[node].kind {
            NodeKind::Map => {
                let input = self
                    .input_link(node, Port::Shape)
                    .and_then(|link| source_from(link.from, link.out));
                let in_lo = value(KnobId::MapInMin, &mut source_from);
                let in_hi = value(KnobId::MapInMax, &mut source_from);
                // A bipolar map has the Center and Range knobs on show, and neither the output
                // ends nor the offset are read, since Center does the offset's job. Otherwise the
                // ends and the offset are read, and the other two are not.
                let (out_lo, out_hi, offset, center_range) = if map::bipolar(&self.nodes[node]) {
                    let center = value(KnobId::MapCenter, &mut source_from);
                    let range = value(KnobId::MapRange, &mut source_from);
                    (
                        MapValue::fixed(self.nodes[node].plain(KnobId::MapMin)),
                        MapValue::fixed(self.nodes[node].plain(KnobId::MapMax)),
                        MapValue::fixed(0.0),
                        Some((center, range)),
                    )
                } else {
                    (
                        value(KnobId::MapMin, &mut source_from),
                        value(KnobId::MapMax, &mut source_from),
                        value(KnobId::MapOffset, &mut source_from),
                        None,
                    )
                };
                input.map(|input| {
                    SignalNode::Map(MapNode {
                        input,
                        in_lo,
                        in_hi,
                        out_lo,
                        out_hi,
                        offset,
                        center_range,
                    })
                })
            }
            NodeKind::Math => Some(SignalNode::Math(MathNode {
                op: math::operation(&self.nodes[node]),
                a: value(KnobId::MathA, &mut source_from),
                b: value(KnobId::MathB, &mut source_from),
            })),
            _ => None,
        };
        path.pop();
        let at = build.table.push(made?)?;
        build.nodes.push((node, at as u8));
        Some(at as u8)
    }

    /// The level of the signal on a control output, as the live control levels give it. The levels
    /// must have the map and math nodes worked out, as the engine's do.
    pub fn control_level(&self, from: usize, out: Port, levels: &ControlLevels) -> Option<f32> {
        self.source_at(from, out).map(|source| levels.level(source))
    }

    /// `levels` with every map and math node's output worked out from its sources, as the engine
    /// does. A test sets the sources it wants and calls this to read what a route gives.
    #[cfg(test)]
    pub fn with_signals(&self, mut levels: ControlLevels) -> ControlLevels {
        self.signals().table.evaluate(&mut levels);
        levels
    }

    /// The level arriving at a map's input, if a control source feeds it.
    pub fn map_input_level(&self, node: usize, levels: &ControlLevels) -> Option<f32> {
        let link = self.input_link(node, Port::Shape)?;
        self.control_level(link.from, link.out, levels)
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
                retrigger: node.switch(0),
                one_shot: node.switch(1),
                curve: lfo::is_custom(node).then(|| lfo::curve(node)),
            });
        }
        out
    }

    /// The first output node, which decides what the plugin plays.
    pub fn first_output(&self) -> Option<usize> {
        self.nodes.iter().position(|n| n.kind == NodeKind::Output)
    }

    /// The first MIDI input node, which sets the voice behaviour: mono and glide.
    pub fn first_input(&self) -> Option<usize> {
        self.nodes.iter().position(|n| n.kind == NodeKind::Input)
    }

    /// The settings that apply to every slot: the master gain from the first output node, and
    /// mono and glide from the first MIDI input node. A graph without one has the defaults.
    pub fn globals(&self) -> Globals {
        let mut globals = Globals::default();
        if let Some(output) = self.first_output() {
            globals.gain = self.param(output, KnobId::Gain);
        }
        if let Some(input) = self.first_input() {
            globals.mono = input::mono(&self.nodes[input]);
            globals.glide_always = input::glide_always(&self.nodes[input]);
            globals.glide = self.param(input, KnobId::Glide);
        }
        globals
    }

    /// True if a MIDI input node's notes pin is wired to this oscillator.
    pub fn has_notes(&self, oscillator: usize) -> bool {
        self.input_link(oscillator, Port::Notes)
            .and_then(|link| self.nodes.get(link.from))
            .is_some_and(|node| node.kind == NodeKind::Input)
    }

    /// Compiles the whole graph for the audio thread.
    pub fn patch(&self) -> Patch {
        Patch {
            lanes: self.lane_graphs(),
            signals: self.signals().table,
            envelopes: self.envelope_settings(),
            lfos: self.lfo_settings(),
            globals: self.globals(),
        }
    }

    /// The audio stages behind each output slot of the first output node. A slot with nothing
    /// wired in is silent.
    pub fn lane_graphs(&self) -> [LaneGraph; MAX_SLOTS] {
        let output = self.first_output();
        // Each effect takes the next free instance of its kind, counted across every slot.
        let mut pools = [0u8; EFFECT_KINDS];
        std::array::from_fn(|slot| match output {
            Some(output) if slot < self.nodes[output].used => {
                self.lane_graph(output, slot, &mut pools)
            }
            _ => LaneGraph::silent(self.slot_gain(output, slot)),
        })
    }

    /// Which engine instance each EQ node runs on, as (node, pool number) pairs, in the order the
    /// compiler hands them out. The editor reads an EQ node's spectrum from its instance. A node
    /// that plays in no slot, or a bypassed one, has no entry. An EQ node that two slots both play
    /// has an entry for each.
    pub fn eq_pools(&self) -> Vec<(usize, u8)> {
        let Some(output) = self.first_output() else {
            return Vec::new();
        };
        let mut pools = [0u8; EFFECT_KINDS];
        let mut found = Vec::new();
        for slot in 0..MAX_SLOTS.min(self.nodes[output].used) {
            let Some(link) = self.input_link(output, Port::Slot(slot)) else {
                continue;
            };
            if let Some(built) = self.build_lane(link.from, &mut pools) {
                found.extend(built.eq_pools);
            }
        }
        found
    }

    /// Which engine instance each monitor node runs on, as (node, pool number) pairs, in the order
    /// the compiler hands them out. The editor reads a monitor node's measurement from its
    /// instance. A monitor that listens to nothing that plays in a slot, or that the slot had no
    /// room for, has no entry. A monitor that two slots both play has an entry for each.
    pub fn monitor_pools(&self) -> Vec<(usize, u8)> {
        let Some(output) = self.first_output() else {
            return Vec::new();
        };
        let mut pools = [0u8; EFFECT_KINDS];
        let mut found = Vec::new();
        for slot in 0..MAX_SLOTS.min(self.nodes[output].used) {
            let Some(link) = self.input_link(output, Port::Slot(slot)) else {
                continue;
            };
            if let Some(built) = self.build_lane(link.from, &mut pools) {
                found.extend(built.monitor_pools);
            }
        }
        found
    }

    /// Builds the arena of the slot fed by `from`, and hands out instances from `pools`. A slot
    /// that only overflows because of its monitors is built again without them, so a monitor never
    /// silences a slot. None means the slot is silent.
    fn build_lane(&self, from: usize, pools: &mut [u8; EFFECT_KINDS]) -> Option<BuiltLane> {
        let (built, used) = self.build_with(from, *pools, true);
        let (built, used) = match built {
            None if used.1 => self.build_with(from, *pools, false),
            _ => (built, used),
        };
        *pools = used.0;
        built
    }

    /// One try at a slot's arena. Returns the arena if it fits, the pools after the try, and
    /// whether the arena overflowed.
    fn build_with(
        &self,
        from: usize,
        mut pools: [u8; EFFECT_KINDS],
        monitors: bool,
    ) -> (Option<BuiltLane>, ([u8; EFFECT_KINDS], bool)) {
        let mut builder = LaneBuilder::new(&mut pools, monitors);
        let placed = builder.place(self, from).is_some();
        let overflowed = builder.overflowed;
        let built = (placed && !overflowed).then(|| BuiltLane {
            stages: builder.stages,
            eq_pools: builder.eq_pools,
            monitor_pools: builder.monitor_pools,
        });
        (built, (pools, overflowed))
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
    fn lane_graph(&self, output: usize, slot: usize, pools: &mut [u8; EFFECT_KINDS]) -> LaneGraph {
        let mut lane = LaneGraph::silent(self.slot_gain(Some(output), slot));
        let Some(link) = self.input_link(output, Port::Slot(slot)) else {
            return lane;
        };
        if let Some(built) = self.build_lane(link.from, pools) {
            lane.stages = built.stages;
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
            NodeKind::Distortion => Some(StageParams::Distortion(DistortionParams {
                kind: distortion::kind(n),
                drive: knob(KnobId::Drive),
                tone: knob(KnobId::Tone),
                level: knob(KnobId::OscGain),
                mix: knob(KnobId::Mix),
            })),
            NodeKind::Compressor => Some(StageParams::Compressor(CompressorParams {
                threshold: knob(KnobId::Threshold),
                ratio: knob(KnobId::Ratio),
                attack: knob(KnobId::CompAttack),
                release: knob(KnobId::CompRelease),
                makeup: knob(KnobId::Makeup),
                mix: knob(KnobId::Mix),
                multiband: compressor::multiband(n),
                depth: knob(KnobId::Depth),
            })),
            NodeKind::Reverb => Some(StageParams::Reverb(ReverbParams {
                decay: knob(KnobId::ReverbDecay),
                size: knob(KnobId::Size),
                damp: knob(KnobId::Damp),
                mix: knob(KnobId::Mix),
            })),
            NodeKind::Delay => Some(StageParams::Delay(DelayParams {
                time: knob(KnobId::DelayTime),
                sync: knob(KnobId::LfoSync),
                feedback: knob(KnobId::Feedback),
                damp: knob(KnobId::Damp),
                mix: knob(KnobId::Mix),
                ping_pong: delay::ping_pong(n),
            })),
            NodeKind::Chorus => Some(StageParams::Chorus(ChorusParams {
                rate: knob(KnobId::LfoRate),
                depth: knob(KnobId::Depth),
                spread: knob(KnobId::Spread),
                mix: knob(KnobId::Mix),
            })),
            NodeKind::Phaser => Some(StageParams::Phaser(PhaserParams {
                rate: knob(KnobId::LfoRate),
                depth: knob(KnobId::Depth),
                feedback: knob(KnobId::Feedback),
                center: knob(KnobId::Center),
                stages: phaser::stages(n),
                spread: knob(KnobId::Spread),
                mix: knob(KnobId::Mix),
            })),
            NodeKind::Flanger => Some(StageParams::Flanger(FlangerParams {
                rate: knob(KnobId::LfoRate),
                depth: knob(KnobId::Depth),
                feedback: knob(KnobId::Feedback),
                delay: knob(KnobId::FlangerDelay),
                spread: knob(KnobId::Spread),
                mix: knob(KnobId::Mix),
            })),
            NodeKind::Eq => Some(StageParams::Eq(EqParams {
                analyzer: eq::analyzer(n),
                bands: std::array::from_fn(|band| EqBandParams {
                    kind: eq::kind(n, band),
                    freq: knob(eq::FREQ[band]),
                    gain: knob(eq::GAIN[band]),
                    q: knob(eq::Q[band]),
                }),
            })),
            NodeKind::Monitor => Some(StageParams::Monitor(MonitorParams {
                mode: monitor::mode(n),
            })),
            NodeKind::Map
            | NodeKind::Math
            | NodeKind::Output
            | NodeKind::Lfo
            | NodeKind::Input
            | NodeKind::Macro
            | NodeKind::Envelope => None,
        }
    }
}

/// The kinds of bus stage that keep memory, for counting how many of each a patch uses: the eight
/// effects, the monitor, and the filter, which only runs on the bus when something on the bus
/// feeds it.
pub const EFFECT_KINDS: usize = 10;
/// The counter the monitors use.
const MONITOR_KIND: usize = 9;

/// Which counter a bus stage uses, or None for a stage with no memory, such as a mix.
fn pool_kind(params: &StageParams) -> Option<usize> {
    match params {
        StageParams::Distortion(_) => Some(0),
        StageParams::Compressor(_) => Some(1),
        StageParams::Reverb(_) => Some(2),
        StageParams::Delay(_) => Some(3),
        StageParams::Chorus(_) => Some(4),
        StageParams::Phaser(_) => Some(5),
        StageParams::Flanger(_) => Some(6),
        StageParams::Filter(_) => Some(7),
        StageParams::Eq(_) => Some(8),
        StageParams::Monitor(_) => Some(MONITOR_KIND),
        StageParams::Oscillator(_) | StageParams::Mix(_) => None,
    }
}

/// What a slot compiles to, and the instances its EQ and monitor nodes took.
struct BuiltLane {
    stages: ArrayVec<StageSpec, MAX_STAGES>,
    eq_pools: Vec<(usize, u8)>,
    monitor_pools: Vec<(usize, u8)>,
}

/// The arena of one slot while it is built.
struct LaneBuilder<'p> {
    /// How many of each effect kind earlier slots and this one have placed, for the next instance.
    pools: &'p mut [u8; EFFECT_KINDS],
    stages: ArrayVec<StageSpec, MAX_STAGES>,
    /// The arena position of each node already placed, so a node feeding two inputs runs once.
    placed: Vec<(usize, u8)>,
    /// The nodes on the path being placed, to spot a loop.
    path: Vec<usize>,
    /// Set when the slot needs more stages than fit.
    overflowed: bool,
    /// The EQ nodes placed so far and the instance each one took.
    eq_pools: Vec<(usize, u8)>,
    /// True to give each monitor a stage after the node it listens to.
    monitors: bool,
    /// The nodes whose monitors are already placed.
    attached: Vec<usize>,
    /// The monitor nodes placed so far and the instance each one took.
    monitor_pools: Vec<(usize, u8)>,
}

impl<'p> LaneBuilder<'p> {
    fn new(pools: &'p mut [u8; EFFECT_KINDS], monitors: bool) -> Self {
        Self {
            pools,
            stages: ArrayVec::new(),
            placed: Vec::new(),
            path: Vec::new(),
            overflowed: false,
            eq_pools: Vec::new(),
            monitors,
            attached: Vec::new(),
            monitor_pools: Vec::new(),
        }
    }

    /// Places a node's stage, and then a stage for each monitor wired to it. A monitor has no
    /// output, so nothing else would pull it in.
    fn place(&mut self, wiring: &Wiring, node: usize) -> Option<u8> {
        let position = self.place_node(wiring, node)?;
        if self.monitors && !self.attached.contains(&node) {
            self.attached.push(node);
            self.place_monitors(wiring, node, position);
        }
        Some(position)
    }

    /// Places a stage for every monitor that listens to `node`, whose own stage or whose upstream
    /// stage, for a bypassed node, is at `source`. A monitor passes its input on unchanged, so it
    /// never changes the sound, even as the last stage of the slot. One that does not fit is
    /// skipped, and the slot keeps what it has.
    fn place_monitors(&mut self, wiring: &Wiring, node: usize, source: u8) {
        for monitor in 0..wiring.nodes.len() {
            if wiring.nodes[monitor].kind != NodeKind::Monitor {
                continue;
            }
            let listens = wiring
                .input_link(monitor, Port::Audio)
                .is_some_and(|link| link.from == node);
            let Some(params) = wiring.stage_params(monitor).filter(|_| listens) else {
                continue;
            };
            if self.stages.is_full() {
                return;
            }
            let pool = self.pools[MONITOR_KIND];
            self.pools[MONITOR_KIND] = pool.saturating_add(1);
            self.monitor_pools.push((monitor, pool));
            let feeder = &mut self.stages[source as usize];
            if !feeder.bus {
                feeder.tap = true;
            }
            self.stages.push(StageSpec {
                params,
                inputs: [Some(source), None],
                bus: true,
                tap: false,
                pool,
            });
        }
    }

    /// Places a node's stage after the stages that feed it, and returns its arena position. None
    /// means the node makes no audio, is part of a loop, or would not fit.
    fn place_node(&mut self, wiring: &Wiring, node: usize) -> Option<u8> {
        if let Some(&(_, position)) = self.placed.iter().find(|(placed, _)| *placed == node) {
            return Some(position);
        }
        if self.path.contains(&node) {
            return None;
        }

        // A switched-off node makes no stage. A muted source is silence on its output, and a
        // bypassed processor is whatever feeds its first audio input.
        let switched_off = &wiring.nodes[node];
        if switched_off.bypassed {
            match switched_off.bypass() {
                Bypass::Mute => return None,
                Bypass::PassThrough => {
                    let first_audio = switched_off
                        .input_ports()
                        .into_iter()
                        .find(|port| port.is_audio());
                    let link = first_audio.and_then(|port| wiring.input_link(node, port));
                    self.path.push(node);
                    let source = link.and_then(|link| self.place(wiring, link.from));
                    self.path.pop();
                    return source;
                }
                Bypass::None => {}
            }
        }

        // An oscillator plays only the notes wired into it. With no MIDI input on its notes pin
        // it is silent.
        if wiring.nodes[node].kind == NodeKind::Oscillator && !wiring.has_notes(node) {
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

        // A stage runs on the bus if it is an effect, or if anything on the bus feeds it. The
        // voices are added together by then, so it can only run once, on the sum.
        let bus = params.is_effect()
            || inputs
                .iter()
                .flatten()
                .any(|&at| self.stages[at as usize].bus);

        let mut pool = 0;
        if bus {
            if let Some(kind) = pool_kind(&params) {
                pool = self.pools[kind];
                self.pools[kind] = pool.saturating_add(1);
                if matches!(params, StageParams::Eq(_)) {
                    self.eq_pools.push((node, pool));
                }
            }
            // A per-voice stage that feeds the bus is added across the voices to feed it.
            for source in inputs.iter().flatten() {
                let stage = &mut self.stages[*source as usize];
                if !stage.bus {
                    stage.tap = true;
                }
            }
        }

        let position = self.stages.len() as u8;
        let spec = StageSpec {
            params,
            inputs,
            bus,
            tap: false,
            pool,
        };
        if self.stages.try_push(spec).is_err() {
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

    /// Adds a MIDI input node at the end and wires its notes to every oscillator, so the
    /// oscillators sound. The nodes already there keep their indices.
    pub(super) fn with_notes(nodes: &mut Vec<FlodeNode>, links: &mut Vec<Connection>) {
        nodes.push(node(NodeKind::Input));
        let input = nodes.len() - 1;
        for oscillator in 0..input {
            if nodes[oscillator].kind == NodeKind::Oscillator {
                links.push(notes(input, oscillator));
            }
        }
    }

    pub(super) fn notes(from: usize, to: usize) -> Connection {
        Connection {
            from,
            out: Port::Notes,
            to,
            input: Port::Notes,
        }
    }

    /// The lane for slot 0 of a graph whose output is the node at `output`. Every oscillator
    /// gets its notes from a MIDI input.
    fn lane(mut nodes: Vec<FlodeNode>, mut links: Vec<Connection>, output: usize) -> LaneGraph {
        nodes[output].used = 1;
        with_notes(&mut nodes, &mut links);
        Wiring::new(&nodes, &links).lane_graphs()[0].clone()
    }

    /// An oscillator plays only the notes wired into it.
    #[test]
    fn an_oscillator_with_no_notes_wired_is_silent() {
        let mut nodes = vec![node(NodeKind::Oscillator), node(NodeKind::Output)];
        nodes[1].used = 1;
        let links = vec![audio(0, 1, Port::Slot(0))];
        assert!(!Wiring::new(&nodes, &links).lane_graphs()[0].is_active());

        // A notes wire from a node that is not a MIDI input does not count.
        nodes.push(node(NodeKind::Filter));
        let links = vec![audio(0, 1, Port::Slot(0)), notes(2, 0)];
        assert!(!Wiring::new(&nodes, &links).lane_graphs()[0].is_active());

        nodes[2] = node(NodeKind::Input);
        assert!(Wiring::new(&nodes, &links).lane_graphs()[0].is_active());
    }

    /// Mono and glide come from the first MIDI input, and the master gain from the output.
    #[test]
    fn the_voice_settings_come_from_the_midi_input() {
        let mut input = node(NodeKind::Input);
        input.set_switch(0, true);
        input.set_switch(1, true);
        input.set_value(KnobId::Glide, KnobId::Glide.to_normalized(0.5));
        let mut output = node(NodeKind::Output);
        output.set_value(KnobId::Gain, KnobId::Gain.to_normalized(-3.0));
        let nodes = vec![output, input];
        let globals = Wiring::new(&nodes, &[]).globals();
        assert!(globals.mono && globals.glide_always);
        let plain = |param: Param| param.plain(&ControlLevels::default());
        assert!((plain(globals.glide) - 0.5).abs() < 1e-4);
        assert!((plain(globals.gain) + 3.0).abs() < 1e-3);

        // With no input node the voices are poly with no glide.
        let nodes = vec![node(NodeKind::Output)];
        let globals = Wiring::new(&nodes, &[]).globals();
        assert!(!globals.mono && !globals.glide_always);
        assert_eq!(globals.glide.plain(&ControlLevels::default()), 0.0);
    }

    /// A macro knob drives a wired knob from the macro's level, through any maps on the way.
    #[test]
    fn a_macro_drives_a_knob() {
        let nodes = vec![node(NodeKind::Macro), node(NodeKind::Filter)];
        let links = vec![Connection {
            from: 0,
            out: Port::Macro(2),
            to: 1,
            input: Port::Knob(KnobId::Cutoff),
        }];
        let wiring = Wiring::new(&nodes, &links);
        let route = wiring.control_route(1, KnobId::Cutoff).expect("a route");
        assert_eq!(route.source, Source::Macro(2));
        let mut levels = ControlLevels::default();
        levels.macros[2] = 1.0;
        assert_eq!(route.apply(&levels), 1.0);
        assert_eq!(wiring.control_level(0, Port::Macro(2), &levels), Some(1.0));
        assert_eq!(wiring.control_level(0, Port::Macro(1), &levels), Some(0.0));
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
        let mut links = vec![audio(0, 1, Port::Slot(0))];
        let mut with_output = nodes;
        with_output[1].used = 1;
        with_notes(&mut with_output, &mut links);
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

#[cfg(test)]
mod effect_tests {
    use super::*;
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

    fn lanes(
        nodes: &mut Vec<FlodeNode>,
        links: &[Connection],
        output: usize,
    ) -> [LaneGraph; MAX_SLOTS] {
        nodes[output].used = MAX_SLOTS;
        let mut links = links.to_vec();
        super::tests::with_notes(nodes, &mut links);
        Wiring::new(nodes, &links).lane_graphs()
    }

    /// Every effect node makes an effect stage with one input, and no other node does.
    #[test]
    fn effect_nodes_make_effect_stages() {
        let effects = [
            NodeKind::Distortion,
            NodeKind::Compressor,
            NodeKind::Reverb,
            NodeKind::Delay,
            NodeKind::Chorus,
            NodeKind::Phaser,
            NodeKind::Flanger,
            NodeKind::Eq,
            NodeKind::Monitor,
        ];
        for kind in NodeKind::ALL {
            let nodes = vec![node(kind)];
            let params = Wiring::new(&nodes, &[]).stage_params(0);
            let is_effect = effects.contains(&kind);
            assert_eq!(params.is_some_and(|p| p.is_effect()), is_effect, "{kind:?}");
            if is_effect {
                assert_eq!(params.unwrap().input_count(), 1, "{kind:?}");
                let ports = nodes[0].input_ports();
                assert_eq!(ports.iter().filter(|p| p.is_audio()).count(), 1, "{kind:?}");
            }
        }
    }

    /// An EQ node's stage carries each band's kind from its toggle and each setting from its
    /// knobs, and a Q that a wire drives has a route.
    #[test]
    fn an_eq_stage_carries_its_bands() {
        let mut nodes = vec![node(NodeKind::Eq), node(NodeKind::Lfo)];
        nodes[0].set_switch(eq::band_switch(0), true);
        nodes[0].set_value(eq::GAIN[1], eq::GAIN[1].to_normalized(6.0));
        let links = [Connection {
            from: 1,
            out: Port::Shape,
            to: 0,
            input: Port::Knob(eq::Q[2]),
        }];
        let Some(StageParams::Eq(params)) = Wiring::new(&nodes, &links).stage_params(0) else {
            panic!("an EQ node makes an EQ stage");
        };
        let levels = ControlLevels::default();
        assert_eq!(
            params.bands[0].kind,
            crate::dsp::effects::eq::EqKind::HighPass
        );
        assert_eq!(params.bands[1].kind, crate::dsp::effects::eq::EqKind::Peak);
        assert_eq!(
            params.bands[3].kind,
            crate::dsp::effects::eq::EqKind::HighShelf
        );
        assert!((params.bands[1].gain.plain(&levels) - 6.0).abs() < 1e-3);
        assert!((params.bands[3].freq.plain(&levels) - eq::FREQ[3].default_plain()).abs() < 0.1);
        assert_ne!(
            params.bands[2].q,
            Wiring::new(&nodes, &[])
                .stage_params(0)
                .map_or(params.bands[2].q, |p| match p {
                    StageParams::Eq(p) => p.bands[2].q,
                    _ => unreachable!(),
                })
        );
    }

    /// The compiler's pool numbers for EQ nodes are what the editor reads spectra by. Two EQ nodes
    /// in a chain take instances 0 and 1 in signal order, and an EQ wired to nothing takes none.
    #[test]
    fn eq_nodes_report_their_pool_instances() {
        let mut nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Eq),
            node(NodeKind::Eq),
            node(NodeKind::Eq),
            node(NodeKind::Output),
        ];
        let links = [
            audio(0, 1, Port::Audio),
            audio(1, 2, Port::Audio),
            audio(2, 4, Port::Slot(0)),
        ];
        nodes[4].used = 1;
        let mut links = links.to_vec();
        super::tests::with_notes(&mut nodes, &mut links);
        let pools = Wiring::new(&nodes, &links).eq_pools();
        assert_eq!(pools, vec![(1, 0), (2, 1)]);
    }

    /// An oscillator through a reverb is two stages. The oscillator is tapped, so the engine sums
    /// its voices for the reverb, and the reverb is the root.
    #[test]
    fn an_effect_taps_the_stage_that_feeds_it() {
        let mut nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Reverb),
            node(NodeKind::Output),
        ];
        let links = [audio(0, 1, Port::Audio), audio(1, 2, Port::Slot(0))];
        let lane = &lanes(&mut nodes, &links, 2)[0];
        assert_eq!(lane.stages.len(), 2);
        assert!(lane.stages[0].tap);
        assert!(!lane.stages[1].tap);
        assert_eq!(lane.stages[1].inputs[0], Some(0));
        assert!(lane.stages[1].params.is_effect());
    }

    /// A chain of effects taps only the voice stage at its head.
    #[test]
    fn a_chain_of_effects_taps_only_the_voice_stage() {
        let mut nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Distortion),
            node(NodeKind::Delay),
            node(NodeKind::Output),
        ];
        let links = [
            audio(0, 1, Port::Audio),
            audio(1, 2, Port::Audio),
            audio(2, 3, Port::Slot(0)),
        ];
        let lane = &lanes(&mut nodes, &links, 3)[0];
        let taps: Vec<bool> = lane.stages.iter().map(|s| s.tap).collect();
        assert_eq!(taps, [true, false, false]);
    }

    /// A stage that is not an effect keeps its tap flag clear.
    #[test]
    fn a_plain_chain_has_no_taps() {
        let mut nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Filter),
            node(NodeKind::Output),
        ];
        let links = [audio(0, 1, Port::Audio), audio(1, 2, Port::Slot(0))];
        let lane = &lanes(&mut nodes, &links, 2)[0];
        assert!(lane.stages.iter().all(|s| !s.tap));
    }

    /// Each effect of a kind takes the next instance, across slots.
    #[test]
    fn effects_of_one_kind_take_different_instances() {
        let mut nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Reverb),
            node(NodeKind::Oscillator),
            node(NodeKind::Reverb),
            node(NodeKind::Output),
        ];
        let links = [
            audio(0, 1, Port::Audio),
            audio(1, 4, Port::Slot(0)),
            audio(2, 3, Port::Audio),
            audio(3, 4, Port::Slot(1)),
        ];
        let lanes = lanes(&mut nodes, &links, 4);
        assert_eq!(lanes[0].stages[1].pool, 0);
        assert_eq!(lanes[1].stages[1].pool, 1);
    }

    /// A per-voice stage can't read an effect. A save that wires one that way reads silence there.
    #[test]
    fn a_filter_after_an_effect_runs_on_the_bus() {
        let mut nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Reverb),
            node(NodeKind::Filter),
            node(NodeKind::Output),
        ];
        let links = [
            audio(0, 1, Port::Audio),
            audio(1, 2, Port::Audio),
            audio(2, 3, Port::Slot(0)),
        ];
        let lane = &lanes(&mut nodes, &links, 3)[0];
        let filter = lane.stages.last().unwrap();
        assert!(matches!(filter.params, StageParams::Filter(_)));
        assert_eq!(filter.inputs[0], Some(1));
        assert!(filter.bus);
        assert!(!filter.tap);
        // The oscillator stays per voice, and the voices are summed for the reverb.
        assert!(!lane.stages[0].bus && lane.stages[0].tap);
        assert!(lane.stages[1].bus);
    }

    /// Two effect chains from one oscillator meet in a mix. The oscillator is placed and tapped
    /// once, each chain is on the bus, and the mix reads the end of each.
    #[test]
    fn two_effect_chains_meet_in_a_mix() {
        let mut nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Reverb),
            node(NodeKind::Delay),
            node(NodeKind::Mix),
            node(NodeKind::Output),
        ];
        let links = [
            audio(0, 1, Port::Audio),
            audio(0, 2, Port::Audio),
            audio(1, 3, Port::AudioIn(0)),
            audio(2, 3, Port::AudioIn(1)),
            audio(3, 4, Port::Slot(0)),
        ];
        let lane = &lanes(&mut nodes, &links, 4)[0];
        assert_eq!(lane.stages.len(), 4);
        let kinds: Vec<bool> = lane.stages.iter().map(|s| s.bus).collect();
        assert_eq!(kinds, [false, true, true, true]);
        assert!(lane.stages[0].tap);
        let mix = &lane.stages[3];
        assert!(matches!(mix.params, StageParams::Mix(_)));
        assert_eq!(mix.inputs, [Some(1), Some(2)]);
        assert!(!mix.tap);
    }

    /// A dry path and an effect path meet in a mix. The dry stage is per voice, and the mix taps
    /// it to sum the voices.
    #[test]
    fn a_dry_path_and_an_effect_path_meet_in_a_mix() {
        let mut nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Reverb),
            node(NodeKind::Mix),
            node(NodeKind::Output),
        ];
        let links = [
            audio(0, 1, Port::Audio),
            audio(0, 2, Port::AudioIn(0)),
            audio(1, 2, Port::AudioIn(1)),
            audio(2, 3, Port::Slot(0)),
        ];
        let lane = &lanes(&mut nodes, &links, 3)[0];
        assert_eq!(lane.stages.len(), 3);
        assert!(lane.stages[0].tap, "the oscillator feeds the bus twice");
        assert!(lane.stages[2].bus);
        assert_eq!(lane.stages[2].inputs, [Some(0), Some(1)]);
    }

    /// A mix of two per-voice stages stays per voice, as it did before.
    #[test]
    fn a_mix_of_voice_stages_stays_per_voice() {
        let mut nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Oscillator),
            node(NodeKind::Mix),
            node(NodeKind::Output),
        ];
        let links = [
            audio(0, 2, Port::AudioIn(0)),
            audio(1, 2, Port::AudioIn(1)),
            audio(2, 3, Port::Slot(0)),
        ];
        let lane = &lanes(&mut nodes, &links, 3)[0];
        assert!(lane.stages.iter().all(|s| !s.bus && !s.tap));
    }

    /// Filters on the bus take instances of their own, counted across slots, and a mix takes
    /// none.
    #[test]
    fn bus_filters_take_their_own_instances() {
        let mut nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Reverb),
            node(NodeKind::Filter),
            node(NodeKind::Oscillator),
            node(NodeKind::Reverb),
            node(NodeKind::Filter),
            node(NodeKind::Output),
        ];
        let links = [
            audio(0, 1, Port::Audio),
            audio(1, 2, Port::Audio),
            audio(2, 6, Port::Slot(0)),
            audio(3, 4, Port::Audio),
            audio(4, 5, Port::Audio),
            audio(5, 6, Port::Slot(1)),
        ];
        let lanes = lanes(&mut nodes, &links, 6);
        assert_eq!(lanes[0].stages[2].pool, 0);
        assert_eq!(lanes[1].stages[2].pool, 1);
    }

    /// A per-voice filter, with no effect before it, runs per voice and takes no pool.
    #[test]
    fn a_filter_before_the_effects_stays_per_voice() {
        let mut nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Filter),
            node(NodeKind::Reverb),
            node(NodeKind::Output),
        ];
        let links = [
            audio(0, 1, Port::Audio),
            audio(1, 2, Port::Audio),
            audio(2, 3, Port::Slot(0)),
        ];
        let lane = &lanes(&mut nodes, &links, 3)[0];
        assert!(!lane.stages[1].bus);
        assert!(lane.stages[1].tap);
        assert_eq!(lane.stages[1].pool, 0);
    }

    /// The monitor pools of a graph with every slot on, after adding notes.
    fn monitor_pools(
        nodes: &mut Vec<FlodeNode>,
        links: &[Connection],
        output: usize,
    ) -> Vec<(usize, u8)> {
        nodes[output].used = MAX_SLOTS;
        let mut links = links.to_vec();
        super::tests::with_notes(nodes, &mut links);
        Wiring::new(nodes, &links).monitor_pools()
    }

    /// One wire feeds the output and a monitor. The monitor is a bus stage after the oscillator,
    /// which it taps, and it is the last stage, so it passes the oscillator on to the slot.
    #[test]
    fn a_monitor_on_an_oscillator_is_a_bus_stage() {
        let mut nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Monitor),
            node(NodeKind::Output),
        ];
        let links = [audio(0, 1, Port::Audio), audio(0, 2, Port::Slot(0))];
        let lane = &lanes(&mut nodes, &links, 2)[0];
        assert_eq!(lane.stages.len(), 2);
        assert!(lane.stages[0].tap && !lane.stages[0].bus);
        let monitor = lane.root().unwrap();
        assert!(matches!(monitor.params, StageParams::Monitor(_)));
        assert!(monitor.bus && !monitor.tap);
        assert_eq!(monitor.inputs, [Some(0), None]);
        assert_eq!(monitor.pool, 0);
        assert_eq!(monitor_pools(&mut nodes, &links, 2), [(1, 0)]);
    }

    /// A monitor on an effect that feeds the output reads the effect's output.
    #[test]
    fn a_monitor_on_an_effect_reads_its_output() {
        let mut nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Reverb),
            node(NodeKind::Monitor),
            node(NodeKind::Output),
        ];
        let links = [
            audio(0, 1, Port::Audio),
            audio(1, 2, Port::Audio),
            audio(1, 3, Port::Slot(0)),
        ];
        let lane = &lanes(&mut nodes, &links, 3)[0];
        assert_eq!(lane.stages.len(), 3);
        assert!(matches!(lane.stages[1].params, StageParams::Reverb(_)));
        assert_eq!(lane.stages[2].inputs, [Some(1), None]);
        assert!(matches!(lane.stages[2].params, StageParams::Monitor(_)));
    }

    /// A monitor on a node in the middle of a per-voice chain sits where it was placed. The root
    /// stays per voice, and the monitor's source is tapped.
    #[test]
    fn a_monitor_in_a_voice_chain_leaves_the_root_per_voice() {
        let mut nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Filter),
            node(NodeKind::Monitor),
            node(NodeKind::Output),
        ];
        let links = [
            audio(0, 1, Port::Audio),
            audio(0, 2, Port::Audio),
            audio(1, 3, Port::Slot(0)),
        ];
        let lane = &lanes(&mut nodes, &links, 3)[0];
        assert_eq!(lane.stages.len(), 3);
        assert!(lane.stages[0].tap);
        assert!(matches!(lane.stages[1].params, StageParams::Monitor(_)));
        let root = lane.root().unwrap();
        assert!(matches!(root.params, StageParams::Filter(_)));
        assert!(!root.bus);
        assert_eq!(root.inputs[0], Some(0));
    }

    /// A monitor on a bypassed node listens to what the node passes through.
    #[test]
    fn a_monitor_on_a_bypassed_node_reads_the_node_upstream() {
        let mut nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Filter),
            node(NodeKind::Monitor),
            node(NodeKind::Output),
        ];
        nodes[1].bypassed = true;
        let links = [
            audio(0, 1, Port::Audio),
            audio(1, 2, Port::Audio),
            audio(1, 3, Port::Slot(0)),
        ];
        let lane = &lanes(&mut nodes, &links, 3)[0];
        assert_eq!(lane.stages.len(), 2);
        assert!(matches!(lane.stages[0].params, StageParams::Oscillator(_)));
        assert_eq!(lane.stages[1].inputs, [Some(0), None]);
        assert!(matches!(lane.stages[1].params, StageParams::Monitor(_)));
    }

    /// A monitor on a muted node has nothing to hear and gets no stage.
    #[test]
    fn a_monitor_on_a_muted_node_gets_no_stage() {
        let mut nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Oscillator),
            node(NodeKind::Monitor),
            node(NodeKind::Output),
        ];
        nodes[1].bypassed = true;
        let links = [audio(1, 2, Port::Audio), audio(0, 3, Port::Slot(0))];
        let lane = &lanes(&mut nodes, &links, 3)[0];
        assert_eq!(lane.stages.len(), 1);
        assert!(monitor_pools(&mut nodes, &links, 3).is_empty());
    }

    /// A monitor on a source that plays in no slot gets no stage and no instance.
    #[test]
    fn a_monitor_on_an_unplayed_source_gets_no_stage() {
        let mut nodes = vec![
            node(NodeKind::Oscillator),
            node(NodeKind::Oscillator),
            node(NodeKind::Monitor),
            node(NodeKind::Output),
        ];
        let links = [audio(1, 2, Port::Audio), audio(0, 3, Port::Slot(0))];
        let lane = &lanes(&mut nodes, &links, 3)[0];
        assert_eq!(lane.stages.len(), 1);
        assert!(monitor_pools(&mut nodes, &links, 3).is_empty());
    }

    /// A monitor that does not fit is skipped, and the slot keeps playing.
    #[test]
    fn a_monitor_past_the_stage_limit_is_skipped() {
        // A full arena, with a monitor on its last stage.
        let mut nodes = vec![node(NodeKind::Oscillator)];
        let mut links = Vec::new();
        for i in 0..MAX_STAGES - 1 {
            nodes.push(node(NodeKind::Filter));
            links.push(audio(i, i + 1, Port::Audio));
        }
        nodes.push(node(NodeKind::Monitor));
        nodes.push(node(NodeKind::Output));
        let (monitor, output) = (MAX_STAGES, MAX_STAGES + 1);
        links.push(audio(MAX_STAGES - 1, monitor, Port::Audio));
        links.push(audio(MAX_STAGES - 1, output, Port::Slot(0)));
        let lane = &lanes(&mut nodes, &links, output)[0];
        assert_eq!(lane.stages.len(), MAX_STAGES);
        assert!(lane
            .stages
            .iter()
            .all(|s| !matches!(s.params, StageParams::Monitor(_))));
        assert!(monitor_pools(&mut nodes, &links, output).is_empty());

        // A monitor early in the chain would take the room the later filters need. The slot
        // plays without it.
        links[MAX_STAGES - 1] = audio(0, monitor, Port::Audio);
        let lane = &lanes(&mut nodes, &links, output)[0];
        assert_eq!(lane.stages.len(), MAX_STAGES);
        assert!(lane
            .stages
            .iter()
            .all(|s| !matches!(s.params, StageParams::Monitor(_))));
        assert!(monitor_pools(&mut nodes, &links, output).is_empty());
    }

    /// Monitors take instances in the order the slots place them, and the lane graphs hand out
    /// the same numbers as `monitor_pools`.
    #[test]
    fn monitor_pools_number_monitors_across_slots() {
        let mut nodes = vec![
            node(NodeKind::Oscillator), // 0
            node(NodeKind::Monitor),    // 1
            node(NodeKind::Monitor),    // 2
            node(NodeKind::Oscillator), // 3
            node(NodeKind::Monitor),    // 4
            node(NodeKind::Output),     // 5
        ];
        let links = [
            audio(0, 1, Port::Audio),
            audio(0, 2, Port::Audio),
            audio(0, 5, Port::Slot(0)),
            audio(3, 4, Port::Audio),
            audio(3, 5, Port::Slot(1)),
        ];
        assert_eq!(
            monitor_pools(&mut nodes, &links, 5),
            [(1, 0), (2, 1), (4, 2)]
        );
        let lanes = lanes(&mut nodes, &links, 5);
        let pools = |lane: &LaneGraph| -> Vec<u8> {
            lane.stages
                .iter()
                .filter(|s| matches!(s.params, StageParams::Monitor(_)))
                .map(|s| s.pool)
                .collect()
        };
        assert_eq!(pools(&lanes[0]), [0, 1]);
        assert_eq!(pools(&lanes[1]), [2]);
    }
}
