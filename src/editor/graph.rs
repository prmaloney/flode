//! The graph model and its editing state: nodes, wires, and what the mouse is doing to them.

use crate::editor::piano::Piano;
use crate::nodes::connection::Connection;
use crate::nodes::knobs::KnobId;
use crate::nodes::output::MAX_SLOTS;
use crate::nodes::{Category, FlodeNode, NodeKind, Port, HEADER_HEIGHT, NODE_WIDTH};
use crate::patch::control::{ControlLevels, Route};
use crate::patch::wiring::Wiring;
use crate::patch::Patch;
use nih_plug_iced::keyboard::KeyCode;
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
    /// The key of the shortcut that last ran, while it is still held. Holding a key repeats its
    /// press, and a shortcut runs once per press.
    pub(super) shortcut_held: Option<KeyCode>,
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

/// Sets how many slot pins each output node shows for these wires. A wire to a pin that is not
/// shown is invalid, so a load calls this before it checks the wires it read.
fn show_slots(nodes: &mut [FlodeNode], connections: &[Connection]) {
    for (index, node) in nodes.iter_mut().enumerate() {
        if node.kind != NodeKind::Output {
            continue;
        }
        let used = connections
            .iter()
            .filter(|c| c.to == index)
            .filter_map(|c| match c.input {
                Port::Slot(slot) => Some(slot + 1),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        node.used = used.min(MAX_SLOTS);
        node.slots = (node.used + 1).min(MAX_SLOTS);
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
            shortcut_held: None,
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
        self.draw_order()
            .into_iter()
            .rev()
            .find(|&index| self.nodes[index].bounds().contains(point))
    }

    /// The nodes from the one drawn first (at the back) to the one drawn last (on top). Nodes
    /// keep their own order, except that the selected node, or the one being placed, goes last,
    /// so it sits above any node it overlaps.
    pub(super) fn draw_order(&self) -> Vec<usize> {
        let top = match self.interaction {
            Interaction::Placing { node, .. } => Some(node),
            _ => self.selected,
        }
        .filter(|&index| index < self.nodes.len());
        let mut order: Vec<usize> = (0..self.nodes.len())
            .filter(|&index| Some(index) != top)
            .collect();
        order.extend(top);
        order
    }

    /// The graph's wires as the compiler reads them.
    pub fn wiring(&self) -> Wiring<'_> {
        Wiring::new(&self.nodes, &self.connections)
    }

    /// True if a control wire drives this knob. A driven knob can't be dragged.
    pub(super) fn is_driven(&self, node: usize, id: KnobId) -> bool {
        self.wiring().is_driven(node, id)
    }

    /// The control route that drives a knob, if one does.
    pub fn control_route(&self, node: usize, id: KnobId) -> Option<Route> {
        self.wiring().control_route(node, id)
    }

    /// The level arriving at a map's input, if a control source feeds it.
    pub(super) fn map_input_level(&self, node: usize, levels: &ControlLevels) -> Option<f32> {
        self.wiring().map_input_level(node, levels)
    }

    /// Compiles the graph into the patch the audio thread plays.
    pub fn patch(&self) -> Patch {
        self.wiring().patch()
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
        // Slot pins show only for connected slots, so they are counted before the wires are
        // checked. Otherwise every wire past the first slot would look like it has no pin.
        let mut nodes = nodes;
        show_slots(&mut nodes, &links);
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
        show_slots(&mut self.nodes, &self.connections);
    }

    /// The audio connection that a node would be inserted into if dropped here. Only an
    /// unconnected node with an audio input and an audio output qualifies. The connection's curve
    /// must pass through the node. A wire into an output slot counts, since that is where
    /// the chain's last wire ends.
    pub(super) fn link_under(&self, node: usize) -> Option<usize> {
        let dragged = &self.nodes[node];
        let behavior = dragged.kind.behavior();
        // Audio nodes splice into audio wires. A node with several audio inputs, such as a mix
        // node, takes the wire on its first one. A node with a control input and output, such as a
        // map, splices into a wire that drives a knob.
        let audio_node = behavior.audio_in() && behavior.audio_out();
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
                // The two wires the splice would make have to be allowed: an effect's output
                // only goes to another effect or an output slot.
                c.out == Port::Audio
                    && c.input.is_audio()
                    && NodeKind::may_feed(self.nodes[c.from].kind, dragged.kind)
                    && NodeKind::may_feed(dragged.kind, self.nodes[c.to].kind)
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
    /// feeds the old target. Control wires are spliced through the node's shape pins. An audio
    /// wire goes into the node's first audio input, which is the top one on a node with several,
    /// such as signal A on a mix.
    pub(super) fn insert_into(&mut self, node: usize, link: usize) {
        let old = self.connections[link];
        let (input, out) = if old.out.is_control_out() {
            (Port::Shape, Port::Shape)
        } else {
            let first_audio = self.nodes[node]
                .input_ports()
                .into_iter()
                .find(|port| port.is_audio());
            (first_audio.unwrap_or(Port::Audio), Port::Audio)
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

    /// Makes `node` follow the cursor, centred on `point`, until the next press sets it down. A
    /// press that lands while a wire is under the node splices the node into that wire.
    pub(super) fn start_placing(&mut self, node: usize, point: Point) {
        let grab = Vector::new(NODE_WIDTH / 2.0, HEADER_HEIGHT / 2.0);
        self.nodes[node].position = Point::new(point.x - grab.x, point.y - grab.y);
        self.selected = Some(node);
        self.interaction = Interaction::Placing { node, grab };
    }

    /// How far a copy of a node sits from the original, so it does not hide it.
    const DUPLICATE_OFFSET: f32 = 32.0;

    /// Copies a node, offset down and to the right, and selects the copy. The copy keeps the
    /// original's settings and has no wires. Returns its index, or None for a node that can't be
    /// copied.
    pub(super) fn duplicate_node(&mut self, index: usize) -> Option<usize> {
        let original = self.nodes.get(index)?;
        if !original.can_duplicate() {
            return None;
        }
        let at = Point::new(
            original.position.x + Self::DUPLICATE_OFFSET,
            original.position.y + Self::DUPLICATE_OFFSET,
        );
        self.nodes.push(original.duplicate(at));
        let copy = self.nodes.len() - 1;
        self.selected = Some(copy);
        Some(copy)
    }

    /// Switches a node off or on, if its kind can be. Returns whether anything changed.
    pub(super) fn toggle_bypass(&mut self, index: usize) -> bool {
        self.nodes
            .get_mut(index)
            .is_some_and(FlodeNode::toggle_bypass)
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
