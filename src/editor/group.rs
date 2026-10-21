//! Node groups in the editor. A group node holds a graph of its own. The commands here make a
//! group from selected nodes, break one back into nodes, open one so the editor shows its graph,
//! and edit the pins where wires cross its boundary.
//!
//! Opening a group swaps its graph into `Graph::nodes` and `Graph::connections`, so everything
//! else in the editor keeps working on whatever graph is shown. The graph outside goes on the
//! `open_groups` stack until the group closes.
//!
//! The audio engine never sees a group. `Graph::patch` flattens the root graph first, and the
//! queries that ask about a node on show read the same flat graph through a cache.

use super::graph::{show_slots, Graph, Interaction};
use super::group_ui::{canvas_rect, inner_bounds, pane_bounds};
use super::monitor::MonitorHistory;
use super::view::{View, ZOOM_MIN};
use crate::nodes::connection::Connection;
use crate::nodes::group::{GroupData, GroupPin};
use crate::nodes::{FlodeNode, KnobId, NodeKind, Port, Signal, NODE_WIDTH};
use crate::patch::flatten::{flatten, Flat};
use crate::patch::wiring::Wiring;
use nih_plug_iced::{Point, Vector};
use std::hash::{Hash, Hasher};
use std::sync::PoisonError;

/// Which edge of a group a pin sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinSide {
    /// A pin that takes a signal into the group. It is on the left of the group node, and it is
    /// an output of the group's input edge node inside.
    Input,
    /// A pin that sends a signal out of the group. It is on the right of the group node, and it
    /// is an input of the group's output edge node inside.
    Output,
}

impl PinSide {
    /// The kind of edge node inside the group that holds this side's pins.
    fn edge_kind(self) -> NodeKind {
        match self {
            PinSide::Input => NodeKind::GroupInput,
            PinSide::Output => NodeKind::GroupOutput,
        }
    }

    /// The pin list of an edge node on this side.
    fn edge_pins(self, node: &mut FlodeNode) -> &mut Vec<GroupPin> {
        match self {
            PinSide::Input => &mut node.outputs,
            PinSide::Output => &mut node.inputs,
        }
    }

    /// The pin list of the group node on this side. It copies the edge node's list.
    fn face_pins(self, node: &mut FlodeNode) -> &mut Vec<GroupPin> {
        match self {
            PinSide::Input => &mut node.inputs,
            PinSide::Output => &mut node.outputs,
        }
    }
}

/// The state of the graph outside an open group, kept while the group is on show.
pub(super) struct OpenGroup {
    pub(super) nodes: Vec<FlodeNode>,
    pub(super) connections: Vec<Connection>,
    pub(super) selected: Option<usize>,
    pub(super) also_selected: Vec<usize>,
    pub(super) view: View,
    pub(super) monitor_history: Vec<MonitorHistory>,
    /// The index in `nodes` of the group node that is open. Its graph is the one on show, so the
    /// graph inside it is empty until the group closes.
    pub(super) group: usize,
}

/// The graph compiled with its groups expanded, and where each node on show landed in it.
pub(super) struct FlatCache {
    /// A fingerprint of every graph this was built from.
    key: u64,
    pub(super) flat: Flat,
    /// The flat index of each node on show, or None for a group part, which has none.
    pub(super) shown: Vec<Option<usize>>,
    /// How many times the cache was built. Tests read it to see that a read reused it.
    pub(super) generation: usize,
}

/// A small fast hasher for the cache key. The key is hashed on every query, so the standard
/// hasher's cost would add up over a frame.
struct Mix(u64);

impl Mix {
    fn step(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl Hasher for Mix {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.step(u64::from(byte));
        }
    }

    fn write_u8(&mut self, value: u8) {
        self.step(u64::from(value));
    }

    fn write_u32(&mut self, value: u32) {
        self.step(u64::from(value));
    }

    fn write_u64(&mut self, value: u64) {
        self.step(value);
    }

    fn write_usize(&mut self, value: usize) {
        self.step(value as u64);
    }
}

fn hash_graph(nodes: &[FlodeNode], links: &[Connection], state: &mut Mix) {
    state.write_usize(nodes.len());
    for node in nodes {
        node.hash_compile_state(state);
    }
    links.hash(state);
}

/// The room left between nodes that a new group puts around the members, and the room the view
/// leaves around a group's graph when it fits the graph into the pane.
const EDGE_GAP: f32 = 48.0;
const FIT_MARGIN: f32 = 40.0;

/// A name for a pin that a port gives it, if the port has a natural one.
fn natural_name(port: Port) -> Option<String> {
    match port {
        Port::Knob(id) => Some(id.name().to_string()),
        Port::Shape => Some("Shape".to_string()),
        Port::Notes => Some("MIDI".to_string()),
        Port::Audio | Port::Slot(_) => Some("Audio".to_string()),
        Port::AudioIn(0) => Some("A".to_string()),
        Port::AudioIn(1) => Some("B".to_string()),
        Port::Midi(source) => Some(source.label().to_string()),
        Port::Macro(index) => Some(format!("Macro {}", index + 1)),
        Port::AudioIn(_) | Port::Pin(..) | Port::NewPin => None,
    }
}

/// True for a name that `add_pin` gave, such as "In 2" or "Out 1". A pin that still has one has
/// not been named by hand or by a wire.
fn is_default_name(name: &str) -> bool {
    let number = name
        .strip_prefix("In ")
        .or_else(|| name.strip_prefix("Out "));
    number.is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// `name`, or `name` with a number after it if a pin in `pins` already has it.
fn unique_name(pins: &[GroupPin], name: String) -> String {
    let taken = |candidate: &str| pins.iter().any(|pin| pin.name == candidate);
    if !taken(&name) {
        return name;
    }
    (2..)
        .map(|n| format!("{name} {n}"))
        .find(|candidate| !taken(candidate))
        .unwrap_or(name)
}

/// The number of a pin port.
fn pin_number(port: Port) -> Option<usize> {
    match port {
        Port::Pin(_, n) => Some(n),
        _ => None,
    }
}

/// Moves a pin port down one place if it is above `removed`.
fn close_gap(port: &mut Port, removed: usize) {
    if let Port::Pin(signal, n) = *port {
        if n > removed {
            *port = Port::Pin(signal, n - 1);
        }
    }
}

/// The first knob that pin `pin` of the input edge node `edge` feeds, with the value that knob
/// holds. Targets that are shape pins or pins of another group are not knobs.
fn first_knob(data: &GroupData, edge: usize, pin: usize) -> Option<(KnobId, f32)> {
    data.links.iter().find_map(|link| {
        let from_pin = link.from == edge && pin_number(link.out) == Some(pin);
        match (from_pin, link.input) {
            (true, Port::Knob(id)) => Some((id, data.nodes.get(link.to)?.value(id))),
            _ => None,
        }
    })
}

/// Makes the group node's face pins match its graph's edge nodes. The names and signals come from
/// the edge nodes. A control pin that feeds a knob inside takes that knob as its own, and keeps the
/// value it had if it already stood for a knob. Otherwise it starts at the value of that knob, so
/// making a group changes no sound. The edge node gets the same pins, so the two stay equal.
pub(super) fn sync_face(group: &mut FlodeNode) {
    let old = std::mem::take(&mut group.inputs);
    let Some(data) = group.group.as_mut() else {
        group.inputs = old;
        return;
    };
    if let Some(edge) = data.find(NodeKind::GroupInput) {
        let mut pins = data.nodes[edge].outputs.clone();
        for (index, pin) in pins.iter_mut().enumerate() {
            let before = old.get(index).filter(|before| before.signal == pin.signal);
            pin.knob = None;
            if pin.signal == Signal::Control {
                if let Some((knob, inside)) = first_knob(data, edge, index) {
                    pin.knob = Some(knob);
                    // The edge pin's own value is the one the knob on the input edge node
                    // edits while the group is open, and the face copy matches it otherwise.
                    if !before.is_some_and(|before| before.knob.is_some()) {
                        pin.value = inside;
                    }
                }
            }
        }
        data.nodes[edge].outputs = pins.clone();
        group.inputs = pins;
    } else {
        group.inputs = old;
    }
    if let Some(edge) = data.find(NodeKind::GroupOutput) {
        group.outputs = data.nodes[edge].inputs.clone();
    }
}

/// Puts a graph back inside its group node.
fn write_back(group: &mut FlodeNode, nodes: Vec<FlodeNode>, links: Vec<Connection>) {
    if let Some(data) = group.group.as_mut() {
        data.nodes = nodes;
        data.links = links;
    }
    sync_face(group);
}

/// The corner of the box around `nodes`, or None for no nodes.
fn top_left(nodes: &[FlodeNode]) -> Option<Point> {
    let first = nodes.first()?;
    Some(nodes.iter().fold(first.position, |corner, node| {
        Point::new(corner.x.min(node.position.x), corner.y.min(node.position.y))
    }))
}

impl Graph {
    /// True if any group is open, or the graph on show has a group node. Without one, nothing
    /// needs flattening.
    pub fn has_groups(&self) -> bool {
        !self.open_groups.is_empty() || self.nodes.iter().any(|node| node.kind == NodeKind::Group)
    }

    /// How many groups are open. Zero means the root graph is on show.
    pub fn group_depth(&self) -> usize {
        self.open_groups.len()
    }

    /// The names of the open groups, outermost first.
    pub fn group_path_names(&self) -> Vec<String> {
        self.open_groups
            .iter()
            .map(|open| open.nodes[open.group].label().into_owned())
            .collect()
    }

    /// Puts every editing state that names a node or a place back to rest. The commands here
    /// renumber nodes or swap the graph on show, which would leave these pointing at the wrong
    /// thing.
    fn reset_editing(&mut self) {
        self.finish_slides();
        self.interaction = Interaction::Idle;
        self.menu = None;
        self.dropdown_menu = None;
        self.knob_entry = None;
        self.quick_add = None;
        self.category_menu = None;
        self.group_list = None;
        self.hover = None;
        self.link_target = None;
        self.box_base.clear();
        self.viewer_click = None;
        self.rename = None;
        self.margin_press = None;
        self.header_click = None;
    }

    /// Makes a group of the nodes in `members` and returns the group node's index. Members that
    /// can't be grouped are dropped from the list: the output node, the macro node, and the edge
    /// nodes of an open group. A plain group can be a member. Returns None if no member is left.
    ///
    /// The group node takes the place of the first member in the node order, at the top left of
    /// the members. Each wire that crossed the edge of the selection goes through a pin. One
    /// outside source gets one input pin, however many members it feeds. One member output gets
    /// one output pin, however many outside inputs it feeds. A pin takes its name from the port
    /// it serves. The new node is selected.
    pub fn create_group(&mut self, members: &[usize]) -> Option<usize> {
        let mut inside: Vec<usize> = members
            .iter()
            .copied()
            .filter(|&index| {
                self.nodes.get(index).is_some_and(|node| {
                    !matches!(
                        node.kind,
                        NodeKind::Output
                            | NodeKind::Macro
                            | NodeKind::GroupInput
                            | NodeKind::GroupOutput
                    )
                })
            })
            .collect();
        inside.sort_unstable();
        inside.dedup();
        let first = *inside.first()?;
        self.reset_editing();

        let is_inside = |index: usize| inside.binary_search(&index).is_ok();
        // A member's place in the group's node list, after the two edge nodes.
        let inner_of = |index: usize| 2 + inside.partition_point(|&member| member < index);
        // A node that stays outside moves down by the members before it, and up by one for the
        // group node that now sits at `first`.
        let outer_of = |index: usize| {
            index - inside.partition_point(|&member| member < index) + usize::from(index > first)
        };

        let mut in_sources: Vec<(usize, Port)> = Vec::new();
        let mut in_pins: Vec<GroupPin> = Vec::new();
        let mut out_sources: Vec<(usize, Port)> = Vec::new();
        let mut out_pins: Vec<GroupPin> = Vec::new();
        let mut inner_links: Vec<Connection> = Vec::new();
        let mut outer_links: Vec<Connection> = Vec::new();
        for link in &self.connections {
            match (is_inside(link.from), is_inside(link.to)) {
                (true, true) => inner_links.push(Connection {
                    from: inner_of(link.from),
                    to: inner_of(link.to),
                    ..*link
                }),
                (false, true) => {
                    let signal = link.input.signal();
                    let key = (link.from, link.out);
                    let pin = match in_sources.iter().position(|&found| found == key) {
                        Some(pin) => pin,
                        None => {
                            let pin = in_sources.len();
                            let name = natural_name(link.input)
                                .or_else(|| natural_name(link.out))
                                .unwrap_or_else(|| format!("In {}", pin + 1));
                            in_pins.push(GroupPin::new(unique_name(&in_pins, name), signal));
                            in_sources.push(key);
                            outer_links.push(Connection {
                                from: outer_of(link.from),
                                out: link.out,
                                to: first,
                                input: Port::Pin(signal, pin),
                            });
                            pin
                        }
                    };
                    inner_links.push(Connection {
                        from: 0,
                        out: Port::Pin(signal, pin),
                        to: inner_of(link.to),
                        input: link.input,
                    });
                }
                (true, false) => {
                    let signal = link.out.signal();
                    let key = (link.from, link.out);
                    let pin = match out_sources.iter().position(|&found| found == key) {
                        Some(pin) => pin,
                        None => {
                            let pin = out_sources.len();
                            let name = natural_name(link.out)
                                .or_else(|| natural_name(link.input))
                                .unwrap_or_else(|| format!("Out {}", pin + 1));
                            out_pins.push(GroupPin::new(unique_name(&out_pins, name), signal));
                            out_sources.push(key);
                            inner_links.push(Connection {
                                from: inner_of(link.from),
                                out: link.out,
                                to: 1,
                                input: Port::Pin(signal, pin),
                            });
                            pin
                        }
                    };
                    outer_links.push(Connection {
                        from: first,
                        out: Port::Pin(signal, pin),
                        to: outer_of(link.to),
                        input: link.input,
                    });
                }
                (false, false) => outer_links.push(Connection {
                    from: outer_of(link.from),
                    to: outer_of(link.to),
                    ..*link
                }),
            }
        }

        let mut outer_nodes = Vec::new();
        let mut inner_nodes = Vec::new();
        for (index, node) in std::mem::take(&mut self.nodes).into_iter().enumerate() {
            if is_inside(index) {
                inner_nodes.push(node);
            } else {
                outer_nodes.push(node);
            }
        }
        let corner = top_left(&inner_nodes).unwrap_or(Point::new(0.0, 0.0));
        let right = inner_nodes
            .iter()
            .map(|node| node.position.x + node.width())
            .fold(corner.x, f32::max);
        let mut edge_in = FlodeNode::new(
            NodeKind::GroupInput,
            Point::new(corner.x - NODE_WIDTH - EDGE_GAP, corner.y),
        );
        edge_in.outputs = in_pins.clone();
        let mut edge_out = FlodeNode::new(
            NodeKind::GroupOutput,
            Point::new(right + EDGE_GAP, corner.y),
        );
        edge_out.inputs = out_pins.clone();
        inner_nodes.splice(0..0, [edge_in, edge_out]);

        let mut group = FlodeNode::new(NodeKind::Group, corner);
        group.inputs = in_pins;
        group.outputs = out_pins;
        group.group = Some(Box::new(GroupData {
            name: "Group".to_string(),
            nodes: inner_nodes,
            links: inner_links,
        }));
        // The pins that feed a knob start at that knob's value.
        sync_face(&mut group);
        outer_nodes.insert(first, group);
        self.nodes = outer_nodes;
        self.connections = outer_links;
        self.refresh_slots();

        // A history belongs to a monitor node. The ones that moved inside the group go with them.
        for mut history in std::mem::take(&mut self.monitor_history) {
            if !is_inside(history.node) {
                history.node = outer_of(history.node);
                self.monitor_history.push(history);
            }
        }
        self.select(first);
        Some(first)
    }

    /// Breaks the group node `group` back into its nodes and returns whether it did. The nodes
    /// take its place in the order, with the same layout they had inside, and the top left of
    /// that layout is where the group node was. A wire that ran through a pin goes straight from
    /// the node that made the signal to the node that takes it. Pins with no wire on one side
    /// drop their wire on the other. The edge nodes vanish. The nodes that came out are selected.
    pub fn ungroup(&mut self, group: usize) -> bool {
        if !self
            .nodes
            .get(group)
            .is_some_and(|node| node.kind == NodeKind::Group && node.group.is_some())
        {
            return false;
        }
        self.reset_editing();
        let mut nodes = std::mem::take(&mut self.nodes);
        let outer_links = std::mem::take(&mut self.connections);
        let removed = nodes.remove(group);
        let origin = removed.position;
        let Some(data) = removed.group else {
            return false;
        };
        let GroupData {
            nodes: inner_nodes,
            links: inner_links,
            ..
        } = *data;
        let edge_in = inner_nodes
            .iter()
            .position(|node| node.kind == NodeKind::GroupInput);
        let edge_out = inner_nodes
            .iter()
            .position(|node| node.kind == NodeKind::GroupOutput);

        // Where each node of the group's graph lands, and the members that come out.
        let mut landed: Vec<Option<usize>> = vec![None; inner_nodes.len()];
        let mut members: Vec<FlodeNode> = Vec::new();
        for (index, node) in inner_nodes.into_iter().enumerate() {
            if matches!(node.kind, NodeKind::GroupInput | NodeKind::GroupOutput) {
                continue;
            }
            landed[index] = Some(group + members.len());
            members.push(node);
        }
        let shift = top_left(&members).map_or(Vector::new(0.0, 0.0), |corner| {
            Vector::new(origin.x - corner.x, origin.y - corner.y)
        });
        for member in &mut members {
            member.position = Point::new(member.position.x + shift.x, member.position.y + shift.y);
        }
        let count = members.len();
        let outer_of = |index: usize| {
            if index < group {
                index
            } else {
                index - 1 + count
            }
        };
        let new_member = |index: usize| landed.get(index).copied().flatten();

        let mut links: Vec<Connection> = Vec::new();
        for link in outer_links
            .iter()
            .filter(|c| c.from != group && c.to != group)
        {
            links.push(Connection {
                from: outer_of(link.from),
                to: outer_of(link.to),
                ..*link
            });
        }
        for link in &inner_links {
            if let (Some(from), Some(to)) = (new_member(link.from), new_member(link.to)) {
                links.push(Connection { from, to, ..*link });
            }
        }
        let into_group = |pin: usize| {
            outer_links
                .iter()
                .find(|c| c.to == group && c.from != group && pin_number(c.input) == Some(pin))
        };
        // Pins that take a signal in: the outside source goes straight to each member it fed.
        for link in &inner_links {
            let (Some(pin), Some(to)) = (pin_number(link.out), new_member(link.to)) else {
                continue;
            };
            if Some(link.from) != edge_in {
                continue;
            }
            if let Some(source) = into_group(pin) {
                links.push(Connection {
                    from: outer_of(source.from),
                    out: source.out,
                    to,
                    input: link.input,
                });
            }
        }
        // Pins that send a signal out: the member behind the pin goes straight to each target.
        for target in outer_links
            .iter()
            .filter(|c| c.from == group && c.to != group)
        {
            let Some(pin) = pin_number(target.out) else {
                continue;
            };
            let Some(behind) = inner_links
                .iter()
                .find(|c| Some(c.to) == edge_out && pin_number(c.input) == Some(pin))
            else {
                continue;
            };
            let source = if Some(behind.from) == edge_in {
                // A pin wired straight across: the outside source of the matching input pin.
                pin_number(behind.out)
                    .and_then(into_group)
                    .map(|c| (outer_of(c.from), c.out))
            } else {
                new_member(behind.from).map(|from| (from, behind.out))
            };
            if let Some((from, out)) = source {
                links.push(Connection {
                    from,
                    out,
                    to: outer_of(target.to),
                    input: target.input,
                });
            }
        }

        nodes.splice(group..group, members);
        links.retain(|link| link.is_valid(&nodes));
        for mut history in std::mem::take(&mut self.monitor_history) {
            if history.node != group {
                history.node = outer_of(history.node);
                self.monitor_history.push(history);
            }
        }
        self.nodes = nodes;
        self.connections = links;
        self.refresh_slots();
        self.set_selection((group..group + count).collect());
        true
    }

    /// Shows the graph inside group node `group` and returns whether it did. The graph on show
    /// now goes on the stack with its selection, view, and monitor histories, and the editor
    /// works on the inner graph until `close_group`. The inner graph starts with nothing
    /// selected, and the view fits its nodes.
    pub fn open_group(&mut self, group: usize) -> bool {
        if !self
            .nodes
            .get(group)
            .is_some_and(|node| node.kind == NodeKind::Group && node.group.is_some())
        {
            return false;
        }
        self.reset_editing();
        let Some(data) = self.nodes[group].group.as_mut() else {
            return false;
        };
        let mut inner = std::mem::take(&mut data.nodes);
        let links = std::mem::take(&mut data.links);
        // A group from a damaged save may lack an edge node. The inner graph needs both.
        let corner = top_left(&inner).unwrap_or(Point::new(0.0, 0.0));
        for (kind, x) in [
            (NodeKind::GroupInput, corner.x - NODE_WIDTH - EDGE_GAP),
            (NodeKind::GroupOutput, corner.x + NODE_WIDTH + EDGE_GAP),
        ] {
            if !inner.iter().any(|node| node.kind == kind) {
                inner.push(FlodeNode::new(kind, Point::new(x, corner.y)));
            }
        }
        let outer = OpenGroup {
            nodes: std::mem::replace(&mut self.nodes, inner),
            connections: std::mem::replace(&mut self.connections, links),
            selected: self.selected.take(),
            also_selected: std::mem::take(&mut self.also_selected),
            view: self.view,
            monitor_history: std::mem::take(&mut self.monitor_history),
            group,
        };
        self.open_groups.push(outer);
        self.view = self.fit_view();
        true
    }

    /// Shows the graph outside the open group again and returns whether a group was open. The
    /// inner graph goes back into its group node, which takes its pins from the edge nodes. The
    /// selection, view, and monitor histories come back as they were.
    pub fn close_group(&mut self) -> bool {
        if self.open_groups.is_empty() {
            return false;
        }
        self.reset_editing();
        let Some(outer) = self.open_groups.pop() else {
            return false;
        };
        let inner = std::mem::replace(&mut self.nodes, outer.nodes);
        let links = std::mem::replace(&mut self.connections, outer.connections);
        write_back(&mut self.nodes[outer.group], inner, links);
        self.selected = outer.selected;
        self.also_selected = outer.also_selected;
        self.view = outer.view;
        self.monitor_history = outer.monitor_history;
        true
    }

    /// A view that shows every node in the pane, zoomed out if they are spread wider than the
    /// pane. The pane follows the editor's size as the last event reported it.
    fn fit_view(&self) -> View {
        let room = inner_bounds(pane_bounds(canvas_rect(self.canvas_size)));
        let Some(first) = self.nodes.first() else {
            return View::default();
        };
        let start = first.bounds();
        let (mut left, mut top, mut right, mut bottom) = (
            start.x,
            start.y,
            start.x + start.width,
            start.y + start.height,
        );
        for node in &self.nodes {
            let bounds = node.bounds();
            left = left.min(bounds.x);
            top = top.min(bounds.y);
            right = right.max(bounds.x + bounds.width);
            bottom = bottom.max(bounds.y + bounds.height);
        }
        let (width, height) = (right - left, bottom - top);
        let (room_x, room_y) = (
            (room.width - 2.0 * FIT_MARGIN).max(1.0),
            (room.height - 2.0 * FIT_MARGIN).max(1.0),
        );
        let zoom = (room_x / width).min(room_y / height).clamp(ZOOM_MIN, 1.0);
        View {
            zoom,
            offset: Vector::new(
                room.x + (room.width - width * zoom) / 2.0 - left * zoom,
                room.y + (room.height - height * zoom) / 2.0 - top * zoom,
            ),
        }
    }

    /// Fits the graph on show to the pane again. The editor calls it once the real pane size is
    /// known, in case the group opened before an event reported the size.
    pub(super) fn refit_view(&mut self) {
        if !self.open_groups.is_empty() {
            self.view = self.fit_view();
        }
    }

    /// The pins of group node `node` on the shown graph, or of the edge node on that side, on
    /// `side`. None if the node is not one of the group's parts.
    pub(super) fn pins_on(&self, node: usize, side: PinSide) -> Option<&[GroupPin]> {
        let node = self.nodes.get(node)?;
        match (node.kind, side) {
            (NodeKind::Group, PinSide::Input) | (NodeKind::GroupOutput, PinSide::Output) => {
                Some(&node.inputs)
            }
            (NodeKind::Group, PinSide::Output) | (NodeKind::GroupInput, PinSide::Input) => {
                Some(&node.outputs)
            }
            _ => None,
        }
    }

    /// Runs `edit` on the graph inside group node `node`, as if it were open, then puts the view
    /// back. It is how a command made for the open group acts on a group seen from outside.
    /// Returns None if the node is not a group.
    fn inside<R>(&mut self, node: usize, edit: impl FnOnce(&mut Graph) -> R) -> Option<R> {
        if !self.open_group(node) {
            return None;
        }
        let result = edit(self);
        self.close_group();
        Some(result)
    }

    /// Renames pin `index` on `side` of the group part `node` on show. On a group node that is the
    /// pin on its face, and on an edge node it is the pin of the open group. Both end up through
    /// `rename_pin`, so the two sides keep the same name.
    pub(super) fn rename_pin_of(
        &mut self,
        node: usize,
        side: PinSide,
        index: usize,
        name: &str,
    ) -> bool {
        match self.nodes.get(node).map(|n| n.kind) {
            Some(NodeKind::Group) => self
                .inside(node, |graph| graph.rename_pin(side, index, name))
                .unwrap_or(false),
            Some(NodeKind::GroupInput | NodeKind::GroupOutput) => {
                self.rename_pin(side, index, name)
            }
            _ => false,
        }
    }

    /// Removes pin `index` on `side` of the group part `node` on show, the way `rename_pin_of`
    /// renames one.
    pub(super) fn remove_pin_of(&mut self, node: usize, side: PinSide, index: usize) -> bool {
        match self.nodes.get(node).map(|n| n.kind) {
            Some(NodeKind::Group) => self
                .inside(node, |graph| graph.remove_pin(side, index))
                .unwrap_or(false),
            Some(NodeKind::GroupInput | NodeKind::GroupOutput) => self.remove_pin(side, index),
            _ => false,
        }
    }

    /// Joins an output pin to an input pin the way a wire drop does, and returns whether a wire
    /// was made. The empty pin on a group's edge node stands in for a pin that does not exist
    /// yet. A drop with it on one end first adds a pin that carries the signal of the other end.
    /// The new pin is named by `add_pin`. If the wire does not fit, the new pin goes again.
    pub(super) fn drop_wire(&mut self, from: usize, out: Port, to: usize, input: Port) -> bool {
        let (side, node, signal) = match (out, input) {
            (Port::NewPin, Port::NewPin) => return false,
            (Port::NewPin, _) => (PinSide::Input, from, input.signal()),
            (_, Port::NewPin) => (PinSide::Output, to, out.signal()),
            _ => {
                let link = Connection {
                    from,
                    out,
                    to,
                    input,
                };
                if !link.is_valid(&self.nodes) {
                    return false;
                }
                self.connect(from, out, to, input);
                return true;
            }
        };
        // The empty pin is only on the edge node of the group that is open.
        if self.nodes.get(node).map(|n| n.kind) != Some(side.edge_kind()) {
            return false;
        }
        let Some(index) = self.add_pin(side, "", signal) else {
            return false;
        };
        let pin = Port::Pin(signal, index);
        let link = match side {
            PinSide::Input => Connection {
                from,
                out: pin,
                to,
                input,
            },
            PinSide::Output => Connection {
                from,
                out,
                to,
                input: pin,
            },
        };
        if !link.is_valid(&self.nodes) {
            self.remove_pin(side, index);
            return false;
        }
        self.connect(link.from, link.out, link.to, link.input);
        true
    }

    /// Names a pin of the open group after the first wire that reaches it. A pin that feeds a
    /// filter's Cutoff becomes "Cutoff", and a pin that takes a shape becomes "Shape". Only a pin
    /// that still has the name `add_pin` gave it, and has just this one wire, is renamed, so a
    /// name typed by hand stays. A port with no natural name leaves the pin alone.
    pub(super) fn name_pin_after_wire(&mut self, link: Connection) {
        let kind = |node: usize| self.nodes.get(node).map(|n| n.kind);
        let (side, edge, index, port) = match (kind(link.from), link.out, kind(link.to), link.input)
        {
            (Some(NodeKind::GroupInput), Port::Pin(_, n), _, _) => {
                (PinSide::Input, link.from, n, link.input)
            }
            (_, _, Some(NodeKind::GroupOutput), Port::Pin(_, n)) => {
                (PinSide::Output, link.to, n, link.out)
            }
            _ => return,
        };
        let Some(natural) = natural_name(port) else {
            return;
        };
        let wires = self
            .connections
            .iter()
            .filter(|c| match side {
                PinSide::Input => c.from == edge && pin_number(c.out) == Some(index),
                PinSide::Output => c.to == edge && pin_number(c.input) == Some(index),
            })
            .count();
        if wires != 1 {
            return;
        }
        let pins = side.edge_pins(&mut self.nodes[edge]);
        let Some(pin) = pins.get(index) else {
            return;
        };
        if !is_default_name(&pin.name) {
            return;
        }
        let name = unique_name(pins, natural);
        self.rename_pin(side, index, &name);
    }

    /// Adds a pin to the open group and returns its number, or None if no group is open. The pin
    /// goes on the end of its side. It has no wires yet.
    pub fn add_pin(&mut self, side: PinSide, name: &str, signal: Signal) -> Option<usize> {
        let open = self.open_groups.last_mut()?;
        let edge = self
            .nodes
            .iter()
            .position(|node| node.kind == side.edge_kind())?;
        let pins = side.edge_pins(&mut self.nodes[edge]);
        let index = pins.len();
        let name = match name.trim() {
            "" => match side {
                PinSide::Input => format!("In {}", index + 1),
                PinSide::Output => format!("Out {}", index + 1),
            },
            name => name.to_string(),
        };
        let pin = GroupPin::new(name, signal);
        pins.push(pin.clone());
        side.face_pins(&mut open.nodes[open.group]).push(pin);
        Some(index)
    }

    /// Removes pin `index` from the open group and returns whether there was one. Every wire on
    /// it is dropped, on both sides of the boundary, and the pins above it move down one place
    /// with their wires.
    pub fn remove_pin(&mut self, side: PinSide, index: usize) -> bool {
        let Some(open) = self.open_groups.last_mut() else {
            return false;
        };
        let Some(edge) = self
            .nodes
            .iter()
            .position(|node| node.kind == side.edge_kind())
        else {
            return false;
        };
        let pins = side.edge_pins(&mut self.nodes[edge]);
        if index >= pins.len() {
            return false;
        }
        pins.remove(index);
        let face = side.face_pins(&mut open.nodes[open.group]);
        if index < face.len() {
            face.remove(index);
        }
        let group = open.group;
        match side {
            PinSide::Input => {
                // Inside the pin is an output of the edge node. Outside it is an input of the
                // group node.
                self.connections
                    .retain(|c| !(c.from == edge && pin_number(c.out) == Some(index)));
                for link in self.connections.iter_mut().filter(|c| c.from == edge) {
                    close_gap(&mut link.out, index);
                }
                open.connections
                    .retain(|c| !(c.to == group && pin_number(c.input) == Some(index)));
                for link in open.connections.iter_mut().filter(|c| c.to == group) {
                    close_gap(&mut link.input, index);
                }
            }
            PinSide::Output => {
                self.connections
                    .retain(|c| !(c.to == edge && pin_number(c.input) == Some(index)));
                for link in self.connections.iter_mut().filter(|c| c.to == edge) {
                    close_gap(&mut link.input, index);
                }
                open.connections
                    .retain(|c| !(c.from == group && pin_number(c.out) == Some(index)));
                for link in open.connections.iter_mut().filter(|c| c.from == group) {
                    close_gap(&mut link.out, index);
                }
            }
        }
        // A wire that left an output slot on the outside changes how many slots show.
        show_slots(&mut open.nodes, &open.connections);
        true
    }

    /// Renames pin `index` of the open group, on the edge node and on the group node. Returns
    /// whether there was such a pin. A name with nothing in it is refused.
    pub fn rename_pin(&mut self, side: PinSide, index: usize, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        let Some(open) = self.open_groups.last_mut() else {
            return false;
        };
        let Some(edge) = self
            .nodes
            .iter()
            .position(|node| node.kind == side.edge_kind())
        else {
            return false;
        };
        let Some(pin) = side.edge_pins(&mut self.nodes[edge]).get_mut(index) else {
            return false;
        };
        pin.name = name.to_string();
        if let Some(face) = side.face_pins(&mut open.nodes[open.group]).get_mut(index) {
            face.name = name.to_string();
        }
        true
    }

    /// Renames the open group. Returns false if no group is open or the name is empty.
    pub fn rename_group(&mut self, name: &str) -> bool {
        let Some(open) = self.open_groups.last_mut() else {
            return false;
        };
        let group = open.group;
        Self::set_group_name(&mut open.nodes, group, name)
    }

    /// Renames group node `node` of the graph on show. Returns false if it is not a group or the
    /// name is empty.
    pub fn rename_group_node(&mut self, node: usize, name: &str) -> bool {
        Self::set_group_name(&mut self.nodes, node, name)
    }

    fn set_group_name(nodes: &mut [FlodeNode], node: usize, name: &str) -> bool {
        let name = name.trim();
        match nodes.get_mut(node).and_then(|n| n.group.as_mut()) {
            Some(data) if !name.is_empty() => {
                data.name = name.to_string();
                true
            }
            _ => false,
        }
    }

    /// Opens the save dialog for the open group, as it stands now with the graph on show written
    /// back into it. Does nothing if no group is open.
    pub(super) fn start_open_group_save(&mut self) {
        let Some(open) = self.open_groups.last() else {
            return;
        };
        let mut node = open.nodes[open.group].duplicate(Point::new(0.0, 0.0));
        let nodes = self
            .nodes
            .iter()
            .map(|node| node.duplicate(node.position))
            .collect();
        write_back(&mut node, nodes, self.connections.clone());
        let name = node.label().into_owned();
        self.start_group_save_of(&node, &name);
    }

    /// The root graph as copies, with the graph on show and every open group written back into
    /// it. The graph on show is left alone.
    pub(super) fn root_snapshot(&self) -> (Vec<FlodeNode>, Vec<Connection>) {
        let copy = |nodes: &[FlodeNode]| -> Vec<FlodeNode> {
            nodes
                .iter()
                .map(|node| node.duplicate(node.position))
                .collect()
        };
        let mut nodes = copy(&self.nodes);
        let mut links = self.connections.clone();
        for open in self.open_groups.iter().rev() {
            let mut outer = copy(&open.nodes);
            write_back(&mut outer[open.group], nodes, links);
            nodes = outer;
            links = open.connections.clone();
        }
        (nodes, links)
    }

    /// A fingerprint of everything the compiler reads, over the graph on show and the graphs
    /// outside it.
    fn flat_key(&self) -> u64 {
        let mut state = Mix(0);
        hash_graph(&self.nodes, &self.connections, &mut state);
        for open in &self.open_groups {
            state.write_usize(open.group);
            hash_graph(&open.nodes, &open.connections, &mut state);
        }
        state.finish()
    }

    /// Compiles the root graph with its groups expanded, and finds where each node on show went.
    fn build_flat(&self, key: u64, generation: usize) -> FlatCache {
        let (nodes, links) = self.root_snapshot();
        let flat = flatten(&nodes, &links);
        let prefix: Vec<usize> = self.open_groups.iter().map(|open| open.group).collect();
        let shown = (0..self.nodes.len())
            .map(|index| {
                let mut path = prefix.clone();
                path.push(index);
                flat.paths.iter().position(|found| *found == path)
            })
            .collect();
        FlatCache {
            key,
            flat,
            shown,
            generation,
        }
    }

    /// Runs `read` on the flat graph, which is built the first time and then reused. Only call
    /// it when `has_groups` is true.
    ///
    /// The cache checks itself. Each read hashes the nodes and wires, with every knob value, and
    /// rebuilds if the hash differs from the one the cache was built from. Nodes and knobs change
    /// all over the editor, and this way no mutation has to remember to clear anything. The
    /// hash costs a pass over the nodes, which is much less than the flattening it saves.
    pub(super) fn with_flat_cache<R>(&self, read: impl FnOnce(&FlatCache) -> R) -> R {
        let mut cache = self
            .flat_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let key = self.flat_key();
        if cache.as_ref().is_none_or(|built| built.key != key) {
            let generation = cache.as_ref().map_or(1, |built| built.generation + 1);
            *cache = Some(self.build_flat(key, generation));
        }
        match cache.as_ref() {
            Some(built) => read(built),
            None => unreachable!("the cache was filled above"),
        }
    }

    /// Asks the compiler's view of the graph about node `node` of the graph on show. `read` gets
    /// the wiring and the node's index in it. With no groups that is the graph on show. With
    /// groups it is the flat graph, and a node with no place in it, such as a group node, gets
    /// None.
    pub(super) fn with_flat<R>(
        &self,
        node: usize,
        read: impl FnOnce(&Wiring<'_>, usize) -> R,
    ) -> Option<R> {
        if !self.has_groups() {
            return Some(read(&self.wiring(), node));
        }
        self.with_flat_cache(|cache| {
            let at = (*cache.shown.get(node)?)?;
            Some(read(&Wiring::new(&cache.flat.nodes, &cache.flat.links), at))
        })
    }
}
