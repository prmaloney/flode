//! The graph model and its editing state: nodes, wires, and what the mouse is doing to them.

use crate::editor::piano::Piano;
use crate::nodes::connection::Connection;
use crate::nodes::filter;
use crate::nodes::knobs::KnobId;
use crate::nodes::mix;
use crate::nodes::oscillator;
use crate::nodes::output::{slot_knob, MAX_SLOTS};
use crate::nodes::{Category, FlodeNode, NodeKind, Port, NODE_WIDTH};
use crate::patch::control::{ControlLevels, KnobValue, Route, MAX_ENVELOPES, MAX_LFOS};
use crate::patch::stage::LaneGraph;
use crate::patch::wiring::Wiring;
use crate::routing::{Branch, EnvelopeSettings, Globals, LaneChain, LfoSettings, MixChain};
use nih_plug_iced::{Point, Vector};

use super::{draw::*, menus::*, saved::*, style::*, view::*};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Terminal {
    Out(usize, Port),
    In(usize, Port),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Interaction {
    Idle,
    /// Dragging a node. `grab` is the offset from the node's corner to the cursor.
    MovingNode {
        node: usize,
        grab: Vector,
    },
    /// A new node follows the cursor until the next press places it.
    Placing {
        node: usize,
        grab: Vector,
    },
    /// Dragging a wire from an output port. `cursor` is the current end of the wire.
    Wiring {
        from: usize,
        out: Port,
        cursor: Point,
    },
    /// Turning a knob on a node.
    Knob {
        node: usize,
        id: KnobId,
    },
    /// Dragging one end of a map's transfer curve. `end` is 0 for the low end, 1 for the high.
    MapHandle {
        node: usize,
        end: usize,
    },
}

/// Graph state that persists between frames.
pub struct Graph {
    pub(super) nodes: Vec<FlodeNode>,
    pub(super) connections: Vec<Connection>,
    pub(super) interaction: Interaction,
    pub(super) piano: Piano,
    pub(super) selected: Option<usize>,
    pub(super) menu: Option<ContextMenu>,
    /// The node whose dropdown list is open.
    pub(super) dropdown_menu: Option<(usize, usize)>,
    /// The audio connection a dragged node would be inserted into, if dropped now.
    pub(super) link_target: Option<usize>,
    /// The key on the strip the mouse holds down, and so the note it is playing.
    pub(super) mouse_note: Option<u8>,
    /// What the cursor is over, if it is over a dropdown or one of its open rows.
    pub(super) hover: Option<Hover>,
    /// The category whose list is open under the pill.
    pub(super) category_menu: Option<Category>,
    pub(super) view: View,
    /// The last screen position the cursor was seen at. A release uses it, since the cursor can be
    /// outside the window when the button comes up.
    pub(super) last_screen: Point,
}

/// The part of a dropdown under the cursor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Hover {
    /// A dropdown button: which dropdown of which node.
    Button { node: usize, which: usize },
    /// A row of an open dropdown list.
    Row {
        node: usize,
        which: usize,
        row: usize,
    },
}

/// Where the starting nodes sit: the oscillator at the far left and the output at the far right of
/// the default 1024 px editor, with a 24 px margin.
pub(super) const DEFAULT_OSC_X: f32 = 24.0;
pub(super) const DEFAULT_OUTPUT_X: f32 = 1024.0 - 24.0 - NODE_WIDTH;

impl Default for Graph {
    fn default() -> Self {
        let mut graph = Self::with(
            vec![
                FlodeNode::new(NodeKind::Oscillator, Point::new(DEFAULT_OSC_X, 80.0)),
                FlodeNode::new(NodeKind::Output, Point::new(DEFAULT_OUTPUT_X, 80.0)),
            ],
            vec![Connection {
                from: 0,
                out: Port::Audio,
                to: 1,
                input: Port::Slot(0),
            }],
        );
        graph.refresh_slots();
        graph
    }
}

impl Graph {
    pub(super) fn with(nodes: Vec<FlodeNode>, connections: Vec<Connection>) -> Self {
        Self {
            nodes,
            connections,
            interaction: Interaction::Idle,
            piano: Piano::default(),
            selected: None,
            menu: None,
            dropdown_menu: None,
            link_target: None,
            hover: None,
            mouse_note: None,
            category_menu: None,
            view: View::default(),
            last_screen: Point::new(0.0, 0.0),
        }
    }

    /// The dropdown part under `point`. An open list is checked first, since it covers the
    /// buttons below it.
    pub(super) fn hover_at(&self, point: Point) -> Option<Hover> {
        if let Some((index, which)) = self.dropdown_menu {
            let node = &self.nodes[index];
            if let Some(dropdown) = node.dropdowns().into_iter().find(|d| d.which == which) {
                let rows = dropdown.labels.len();
                let list = dropdown_list_bounds(dropdown.bounds, rows);
                if let Some(row) = list_row_at(list, point, rows) {
                    return Some(Hover::Row {
                        node: index,
                        which,
                        row,
                    });
                }
            }
        }
        (0..self.nodes.len()).find_map(|index| {
            self.nodes[index]
                .dropdown_at(point)
                .map(|which| Hover::Button { node: index, which })
        })
    }

    /// Returns the terminal under `point`, if any.
    pub(super) fn terminal_at(&self, point: Point) -> Option<Terminal> {
        for (index, node) in self.nodes.iter().enumerate() {
            for port in node.output_ports() {
                if node.output_point(port).is_some_and(|p| near(p, point)) {
                    return Some(Terminal::Out(index, port));
                }
            }
            for port in node.input_ports() {
                if node.input_point(port).is_some_and(|p| near(p, point)) {
                    return Some(Terminal::In(index, port));
                }
            }
        }
        None
    }

    /// Returns the top-most node under `point`, if any.
    pub(super) fn node_at(&self, point: Point) -> Option<usize> {
        (0..self.nodes.len())
            .rev()
            .find(|&index| self.nodes[index].bounds().contains(point))
    }

    /// The graph's wires as the compiler reads them.
    pub fn wiring(&self) -> Wiring<'_> {
        Wiring::new(&self.nodes, &self.connections)
    }

    /// True if a control wire drives this knob. A driven knob can't be dragged.
    pub(super) fn is_driven(&self, node: usize, id: KnobId) -> bool {
        self.wiring().is_driven(node, id)
    }

    /// True if any knob in the graph is driven by a control wire.
    pub fn any_driven(&self) -> bool {
        self.wiring().any_driven()
    }

    /// The control route that drives a knob, if one does.
    pub fn control_route(&self, node: usize, id: KnobId) -> Option<Route> {
        self.wiring().control_route(node, id)
    }

    /// The level arriving at a map's input, if a control source feeds it.
    pub(super) fn map_input_level(&self, node: usize, levels: &ControlLevels) -> Option<f32> {
        self.wiring().map_input_level(node, levels)
    }

    /// The envelopes the audio thread runs, in graph order.
    pub fn envelope_settings(&self) -> [EnvelopeSettings; MAX_ENVELOPES] {
        self.wiring().envelope_settings()
    }

    /// The LFOs the audio thread runs, in graph order.
    pub fn lfo_settings(&self) -> [LfoSettings; MAX_LFOS] {
        self.wiring().lfo_settings()
    }

    /// The settings of the output node that apply to every slot.
    pub fn globals(&self) -> Globals {
        self.wiring().globals()
    }

    /// The audio stages behind each output slot.
    #[allow(dead_code)] // Used once the engine runs the stage arena.
    pub fn lane_graphs(&self) -> [LaneGraph; MAX_SLOTS] {
        self.wiring().lane_graphs()
    }

    // The three below belong to the old single-chain path, which the engine's switch to stages
    // removes.

    pub(super) fn knob_value(&self, node: usize, id: KnobId) -> KnobValue {
        self.wiring().knob_value(node, id)
    }

    pub(super) fn amp_envelope(&self, oscillator: usize) -> Option<usize> {
        self.wiring().amp_envelope(oscillator)
    }

    pub(super) fn first_output(&self) -> Option<usize> {
        self.wiring().first_output()
    }

    /// Rebuilds the graph from saved state. Empty state gives the default graph, and connections
    /// that no longer fit the node kinds are dropped.
    pub fn from_saved(saved: &SavedGraph) -> Self {
        if saved.nodes.is_empty() {
            return Self::default();
        }
        let nodes: Vec<FlodeNode> = saved.nodes.iter().map(SavedNode::into_node).collect();
        let links: Vec<Connection> = saved
            .links
            .iter()
            .copied()
            .chain(saved.connections.iter().map(|&(from, to)| Connection {
                from,
                out: Port::Audio,
                to,
                input: Port::Audio,
            }))
            .collect();
        // Envelopes used to shape the sound through audio pins. Those pins are gone.
        let mut links = migrate_envelope_audio(&nodes, links);
        // An output node's inputs used to be one audio pin. Those links go to the next free slot.
        for index in 0..links.len() {
            let to = links[index].to;
            if links[index].input == Port::Audio
                && nodes.get(to).is_some_and(|n| n.kind == NodeKind::Output)
            {
                let taken = links
                    .iter()
                    .filter(|c| c.to == to && matches!(c.input, Port::Slot(_)))
                    .count();
                links[index].input = Port::Slot(taken);
            }
        }
        // Knob links name a node's own knobs. Wires drop out if their pins are gone.
        let connections = links
            .into_iter()
            .filter(|link| link.is_valid(&nodes))
            .collect();
        let mut graph = Self::with(nodes, connections);
        graph.refresh_slots();
        graph
    }

    pub fn to_saved(&self) -> SavedGraph {
        SavedGraph {
            nodes: self.nodes.iter().map(SavedNode::new).collect(),
            connections: Vec::new(),
            links: self.connections.clone(),
        }
    }

    /// The chain behind each output slot of the first output node. A slot with no chain is silent.
    pub fn lane_chains(&self) -> [LaneChain; MAX_SLOTS] {
        let output = self.first_output();
        std::array::from_fn(|slot| {
            let Some(output) = output else {
                return LaneChain::SILENT;
            };
            if slot >= self.nodes[output].used {
                return LaneChain::SILENT;
            }
            self.lane_chain(output, slot).unwrap_or(LaneChain::SILENT)
        })
    }

    /// Copies the knobs of one node into a knob table, such as a lane's or a mix branch's.
    pub(super) fn copy_knobs(&self, knobs: &mut [KnobValue; KnobId::COUNT], node: usize) {
        for id in self.nodes[node].rows().iter().filter_map(|row| row.knob) {
            knobs[id as usize] = self.knob_value(node, id);
        }
    }

    /// An oscillator's settings and knobs, as one of a mix node's two branches.
    pub(super) fn oscillator_branch(&self, oscillator: usize) -> Branch {
        let mut knobs = [KnobValue::ZERO; KnobId::COUNT];
        self.copy_knobs(&mut knobs, oscillator);
        let node = &self.nodes[oscillator];
        Branch {
            active: true,
            waveform: self::oscillator::waveform(node),
            noise: self::oscillator::noise_type(node),
            warp: self::oscillator::warp_mode(node),
            envelope: self.amp_envelope(oscillator),
            knobs,
        }
    }

    /// One of a mix node's two branches. Only a bare oscillator can feed one, so an input wired
    /// to anything else, or left unwired, contributes silence.
    pub(super) fn mix_branch(&self, mix_node: usize, which: usize) -> Branch {
        let Some(link) = self
            .connections
            .iter()
            .find(|c| c.to == mix_node && c.input == Port::AudioIn(which))
        else {
            return Branch::SILENT;
        };
        if self.nodes[link.from].kind != NodeKind::Oscillator {
            return Branch::SILENT;
        }
        self.oscillator_branch(link.from)
    }

    /// Walks the audio connections back from one slot of `output` to an oscillator. The lane
    /// takes its knob values from the nodes on the way, so each lane has its own.
    pub(super) fn lane_chain(&self, output: usize, slot: usize) -> Option<LaneChain> {
        let mut chain = LaneChain::SILENT;
        chain.active = true;
        let gain = slot_knob(slot);
        chain.knobs[gain as usize] = self.knob_value(output, gain);

        let mut current = output;
        let mut input = Port::Slot(slot);
        // A graph can't have more links than nodes, so a longer walk means a loop.
        for _ in 0..self.nodes.len() {
            let from = self
                .connections
                .iter()
                .find(|c| c.to == current && c.input == input)?
                .from;
            input = Port::Audio;
            let node = &self.nodes[from];
            match node.kind {
                NodeKind::Oscillator => {
                    chain.waveform = oscillator::waveform(node);
                    chain.noise = oscillator::noise_type(node);
                    chain.warp = oscillator::warp_mode(node);
                    // An envelope wired to the oscillator's input shapes its amplitude.
                    chain.envelope = self.amp_envelope(from);
                    self.copy_knobs(&mut chain.knobs, from);
                    return Some(chain);
                }
                // A mix node stands in for the single oscillator above, combining two of its own.
                NodeKind::Mix => {
                    chain.mix = Some(MixChain {
                        mode: mix::mix_mode(node),
                        mix: self.knob_value(from, KnobId::Mix),
                        a: self.mix_branch(from, 0),
                        b: self.mix_branch(from, 1),
                    });
                    return Some(chain);
                }
                // The walk starts at the output, so the first filter found is the last in the chain.
                NodeKind::Filter => {
                    if chain.filter.is_none() {
                        chain.filter = Some(filter::filter_type(node));
                        self.copy_knobs(&mut chain.knobs, from);
                    }
                }
                NodeKind::Map
                | NodeKind::Output
                | NodeKind::Lfo
                | NodeKind::Input
                | NodeKind::Envelope => return None,
            }
            current = from;
        }
        None
    }

    /// Removes the connection on an input port and returns it.
    pub(super) fn disconnect_input(&mut self, to: usize, input: Port) -> Option<Connection> {
        let index = self
            .connections
            .iter()
            .position(|c| c.to == to && c.input == input)?;
        let link = self.connections.remove(index);
        self.refresh_slots();
        Some(link)
    }

    /// The four points of a connection's curve, or None if its ports no longer exist.
    pub(super) fn link_curve(&self, link: &Connection) -> Option<[Point; 4]> {
        let from = self.nodes.get(link.from)?.output_point(link.out)?;
        let to = self.nodes.get(link.to)?.input_point(link.input)?;
        Some(wire_curve(from, to))
    }

    /// Sets how many slot pins each output node shows. A pin shows for each connected slot, and
    /// one empty pin sits below the last of them.
    pub(super) fn refresh_slots(&mut self) {
        let used: Vec<usize> = (0..self.nodes.len())
            .map(|index| {
                self.connections
                    .iter()
                    .filter(|c| c.to == index)
                    .filter_map(|c| match c.input {
                        Port::Slot(slot) => Some(slot + 1),
                        _ => None,
                    })
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        for (node, used) in self.nodes.iter_mut().zip(used) {
            if node.kind == NodeKind::Output {
                node.used = used.min(MAX_SLOTS);
                node.slots = (node.used + 1).min(MAX_SLOTS);
            }
        }
    }

    /// The audio connection that a node would be inserted into if dropped here. Only an
    /// unconnected node with an audio input and an audio output qualifies. The connection's curve
    /// must pass through the node. A wire into an output slot counts, since that is where
    /// the chain's last wire ends.
    pub(super) fn link_under(&self, node: usize) -> Option<usize> {
        let dragged = &self.nodes[node];
        let behavior = dragged.kind.behavior();
        // Audio nodes splice into audio wires. A node with more than one audio input, such as a
        // mix node, has nowhere obvious to splice a single wire, so it is wired by hand instead.
        // A node with a control input and output, such as a map, splices into a wire that drives
        // a knob.
        let audio_inputs = dragged
            .input_ports()
            .iter()
            .filter(|p| p.is_audio())
            .count();
        let audio_node = behavior.audio_in() && behavior.audio_out() && audio_inputs <= 1;
        let control_node = dragged.input_ports().contains(&Port::Shape)
            && dragged.output_ports().contains(&Port::Shape);
        if !(audio_node || control_node) {
            return None;
        }
        if self
            .connections
            .iter()
            .any(|c| c.from == node || c.to == node)
        {
            return None;
        }
        let bounds = dragged.bounds();
        self.connections.iter().position(|c| {
            let fits = if audio_node {
                c.out == Port::Audio && c.input.is_audio()
            } else {
                c.out.is_control_out() && matches!(c.input, Port::Knob(_))
            };
            fits && self.link_curve(c).is_some_and(|curve| {
                (0..=WIRE_SAMPLES)
                    .any(|i| bounds.contains(bezier_point(&curve, i as f32 / WIRE_SAMPLES as f32)))
            })
        })
    }

    /// Splices `node` into the connection `link`: the old source feeds the node, and the node
    /// feeds the old target. Control wires are spliced through the node's shape pins.
    pub(super) fn insert_into(&mut self, node: usize, link: usize) {
        let old = self.connections[link];
        let (input, out) = if old.out.is_control_out() {
            (Port::Shape, Port::Shape)
        } else {
            (Port::Audio, Port::Audio)
        };
        self.connections[link] = Connection {
            from: old.from,
            out: old.out,
            to: node,
            input,
        };
        self.connections.push(Connection {
            from: node,
            out,
            to: old.to,
            input: old.input,
        });
    }

    /// Connects an output port to an input port, if the two match. An input holds one
    /// connection, so this replaces any connection already on that input.
    pub(super) fn connect(&mut self, from: usize, out: Port, to: usize, input: Port) {
        let link = Connection {
            from,
            out,
            to,
            input,
        };
        if !link.is_valid(&self.nodes) {
            return;
        }
        self.connections
            .retain(|c| !(c.to == to && c.input == input));
        self.connections.push(link);
        self.refresh_slots();
    }

    /// Adds a node and selects it. Returns its index.
    pub(super) fn add_node(&mut self, kind: NodeKind, position: Point) -> usize {
        self.nodes.push(FlodeNode::new(kind, position));
        let index = self.nodes.len() - 1;
        self.selected = Some(index);
        index
    }

    /// Removes a node and its connections. Connections and the selection after it shift down
    /// to match the new node indices.
    pub(super) fn delete_node(&mut self, index: usize) {
        self.nodes.remove(index);
        self.connections
            .retain(|c| c.from != index && c.to != index);
        for connection in &mut self.connections {
            if connection.from > index {
                connection.from -= 1;
            }
            if connection.to > index {
                connection.to -= 1;
            }
        }
        self.refresh_slots();
        self.selected = match self.selected {
            Some(selected) if selected == index => None,
            Some(selected) if selected > index => Some(selected - 1),
            other => other,
        };
        self.menu = None;
        self.dropdown_menu = None;
        self.interaction = Interaction::Idle;
    }
}

pub(super) fn near(a: Point, b: Point) -> bool {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy <= TERMINAL_HIT_RADIUS * TERMINAL_HIT_RADIUS
}
