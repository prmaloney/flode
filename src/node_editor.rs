//! A freeform node graph for the plugin UI. Click a pill button at the top to add a node. It
//! follows the mouse until the next click places it. Drag a node by its header, and click one to
//! select it. Right-click a node to delete it. The arrow in a header collapses the node. Pins sit
//! on the node edges, with labels. Inputs are on the left and outputs are on the right. Audio
//! wires are grey. Control wires are green. A knob that a control wire drives is greyed out and
//! shows the live signal. Scrolling pans the canvas, and a pinch zooms it.
//!
//! The nodes themselves are in `nodes`. This module holds the graph, the wiring between nodes,
//! and the editor widget that draws the graph and handles the mouse.
//!
//! The audio chain runs from an oscillator through any filter and envelope nodes to an output slot.

use crate::dsp::wavetable::Waveform;
use crate::knob::{self, Press};
use crate::nodes::filter::{self, FilterType};
use crate::nodes::knobs::KnobId;
use crate::nodes::lfo;
use crate::nodes::map;
use crate::nodes::mix;
use crate::nodes::oscillator;
use crate::nodes::output::{slot_knob, MAX_SLOTS};
use crate::nodes::{
    pin_y, Category, FlodeNode, NodeKind, Port, ARROW_CENTER_X, HEADER_HEIGHT, NODE_WIDTH,
    ROW_KNOB_DIAMETER, TITLE_INSET,
};
use crate::piano::{self, Piano, PianoEvent};
use crate::routing::{
    Branch, ControlLevels, EnvelopeSettings, Globals, KnobValue, LaneChain, LfoSettings, MapStage,
    MixChain, Route, Source, MAX_ENVELOPES, MAX_LFOS, MAX_MAPS,
};
use nih_plug_iced::widget::canvas::{
    event::{self, Event},
    Cursor, Frame, Geometry, LineCap, Path, Program, Stroke, Text,
};
use nih_plug_iced::{alignment, mouse, Color, Point, Rectangle, Vector};
use serde::{Deserialize, Serialize};

const CORNER_RADIUS: f32 = 8.0;
const TERMINAL_HIT_RADIUS: f32 = 10.0;
const PIN_RADIUS: f32 = 5.0;
/// Control pins are smaller than audio pins.
const CONTROL_PIN_RADIUS: f32 = 4.0;
const PIN_LABEL_INSET: f32 = 10.0;
const PIN_LABEL_SIZE: f32 = 11.0;
/// The name and value under a pin sit this far above and below the pin's row center.
const NAME_OFFSET: f32 = -7.0;
const VALUE_OFFSET: f32 = 7.0;

const DROPDOWN_CORNER_RADIUS: f32 = 4.0;
const WAVEFORM_ITEM_HEIGHT: f32 = 26.0;
/// Gap between the dropdown and the list that opens below it.
const LIST_GAP: f32 = 2.0;

const CHECKBOX_SIZE: f32 = 12.0;

const PILL_WIDTH: f32 = 480.0;
const PILL_HEIGHT: f32 = 36.0;
const PILL_TOP: f32 = 12.0;

/// The closest and farthest the canvas zooms.
const ZOOM_MIN: f32 = 0.25;
const ZOOM_MAX: f32 = 3.0;

/// The canvas view: nodes are drawn at `zoom` and moved by `offset`. A screen point is
/// `world * zoom + offset`. The pill and the mouse position stay in screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
struct View {
    zoom: f32,
    offset: Vector,
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            offset: Vector::new(0.0, 0.0),
        }
    }
}

impl View {
    /// The editor point under a screen point.
    fn to_world(&self, screen: Point) -> Point {
        Point::new(
            (screen.x - self.offset.x) / self.zoom,
            (screen.y - self.offset.y) / self.zoom,
        )
    }

    /// The editor rectangle that covers a screen rectangle.
    fn rect_to_world(&self, rect: Rectangle) -> Rectangle {
        let origin = self.to_world(Point::new(rect.x, rect.y));
        Rectangle {
            x: origin.x,
            y: origin.y,
            width: rect.width / self.zoom,
            height: rect.height / self.zoom,
        }
    }

    /// Moves the content by a screen offset.
    fn pan(&mut self, by: Vector) {
        self.offset = Vector::new(self.offset.x + by.x, self.offset.y + by.y);
    }

    /// Scales by `factor` around a screen point. The editor point under that point stays put.
    fn zoom_at(&mut self, screen: Point, factor: f32) {
        let world = self.to_world(screen);
        self.zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        self.offset = Vector::new(
            screen.x - world.x * self.zoom,
            screen.y - world.y * self.zoom,
        );
    }
}

/// A pixel count for one line of a mouse wheel. Trackpads already report pixels.
const LINE_SCROLL_PIXELS: f32 = 24.0;

/// What a wheel or trackpad event does to the view.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ViewScroll {
    /// Scale by `1 + magnification`.
    Zoom(f32),
    /// Move the content by this many screen pixels.
    Pan(Vector),
}

/// Reads a scroll event as a zoom or a pan. The baseview fork sends a pinch as a line delta with
/// a NaN x, because iced has no pinch event. Any other scroll pans.
fn view_scroll(delta: mouse::ScrollDelta) -> ViewScroll {
    match delta {
        mouse::ScrollDelta::Lines { x, y } if x.is_nan() => ViewScroll::Zoom(y),
        mouse::ScrollDelta::Lines { x, y } => {
            ViewScroll::Pan(Vector::new(x * LINE_SCROLL_PIXELS, y * LINE_SCROLL_PIXELS))
        }
        mouse::ScrollDelta::Pixels { x, y } => ViewScroll::Pan(Vector::new(x, y)),
    }
}

/// The add buttons on the pill, from left to right.
/// What a pill button does: add a node kind, or open a category's list below the pill.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PillItem {
    Kind(NodeKind),
    Category(Category),
}

/// The pill's buttons, from left to right.
const PILL_ITEMS: [PillItem; 4] = [
    PillItem::Kind(NodeKind::Oscillator),
    PillItem::Category(Category::Driver),
    PillItem::Category(Category::Util),
    PillItem::Category(Category::Effects),
];
/// The narrowest a category list is, so short labels still get a usable row.
const CATEGORY_MIN_WIDTH: f32 = 160.0;

const MENU_WIDTH: f32 = 150.0;
const MENU_ITEM_HEIGHT: f32 = 32.0;
const MENU_CORNER_RADIUS: f32 = 6.0;

const NODE_BODY_COLOR: Color = Color::from_rgb(0.24, 0.24, 0.27);
const NODE_HEADER_COLOR: Color = Color::from_rgb(0.30, 0.30, 0.36);
const SELECTION_COLOR: Color = Color::WHITE;
const SELECTION_WIDTH: f32 = 2.0;
const TEXT_COLOR: Color = Color::WHITE;
const DIM_TEXT_COLOR: Color = Color::from_rgb(0.55, 0.55, 0.58);
const TERMINAL_COLOR: Color = Color::from_rgb(0.75, 0.75, 0.80);
const WIRE_COLOR: Color = Color::from_rgb(0.85, 0.85, 0.90);
/// Every non-audio connection is this green.
const CONTROL_COLOR: Color = Color::from_rgb(0.35, 0.85, 0.45);
const PENDING_WIRE_COLOR: Color = Color::from_rgb(0.50, 0.50, 0.55);
const PILL_COLOR: Color = Color::from_rgb(0.18, 0.18, 0.21);
const PILL_DIVIDER_COLOR: Color = Color::from_rgb(0.40, 0.40, 0.45);
const MENU_COLOR: Color = Color::from_rgb(0.20, 0.20, 0.24);
const DROPDOWN_COLOR: Color = Color::from_rgb(0.16, 0.16, 0.19);
/// A dropdown and its list rows lighten under the cursor.
const DROPDOWN_HOVER_COLOR: Color = Color::from_rgb(0.23, 0.23, 0.28);
const LIST_ROW_HOVER_COLOR: Color = Color::from_rgb(0.26, 0.26, 0.32);
const HIGHLIGHT_WIRE_COLOR: Color = Color::from_rgb(1.0, 0.75, 0.25);
/// Points sampled along a wire when checking whether a node sits on it.
const WIRE_SAMPLES: usize = 24;
const CHECKBOX_CHECKED_COLOR: Color = Color::from_rgb(0.19, 0.50, 0.90);

/// The list that opens below a dropdown, with one row per label.
fn dropdown_list_bounds(dropdown: Rectangle, rows: usize) -> Rectangle {
    Rectangle {
        x: dropdown.x,
        y: dropdown.y + dropdown.height + LIST_GAP,
        width: dropdown.width,
        height: rows as f32 * WAVEFORM_ITEM_HEIGHT,
    }
}

/// The index of the list row under `point`, if any.
fn list_row_at(list: Rectangle, point: Point, rows: usize) -> Option<usize> {
    if !list.contains(point) {
        return None;
    }
    let index = ((point.y - list.y) / WAVEFORM_ITEM_HEIGHT) as usize;
    (index < rows).then_some(index)
}

/// A wire from an output port on one node to an input port on another.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub from: usize,
    pub out: Port,
    pub to: usize,
    pub input: Port,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Terminal {
    Out(usize, Port),
    In(usize, Port),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Interaction {
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

/// The right-click menu for one node. `position` is its top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ContextMenu {
    node: usize,
    position: Point,
}

/// Graph state that persists between frames.
pub struct Graph {
    nodes: Vec<FlodeNode>,
    connections: Vec<Connection>,
    interaction: Interaction,
    piano: Piano,
    selected: Option<usize>,
    menu: Option<ContextMenu>,
    /// The node whose dropdown list is open.
    dropdown_menu: Option<(usize, usize)>,
    /// The audio connection a dragged node would be inserted into, if dropped now.
    link_target: Option<usize>,
    /// The key on the strip the mouse holds down, and so the note it is playing.
    mouse_note: Option<u8>,
    /// What the cursor is over, if it is over a dropdown or one of its open rows.
    hover: Option<Hover>,
    /// The category whose list is open under the pill.
    category_menu: Option<Category>,
    view: View,
    /// The last screen position the cursor was seen at. A release uses it, since the cursor can be
    /// outside the window when the button comes up.
    last_screen: Point,
}

/// The part of a dropdown under the cursor.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Hover {
    /// A dropdown button: which dropdown of which node.
    Button { node: usize, which: usize },
    /// A row of an open dropdown list.
    Row {
        node: usize,
        which: usize,
        row: usize,
    },
}

/// The node graph as saved with the project.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedGraph {
    nodes: Vec<SavedNode>,
    /// Audio connections as (output node index, input node index). Graphs saved before ports
    /// existed use this. New saves leave it empty.
    #[serde(default)]
    connections: Vec<(usize, usize)>,
    #[serde(default)]
    links: Vec<Connection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedNode {
    kind: NodeKind,
    x: f32,
    y: f32,
    /// Missing in graphs saved before collapsing existed. Those load expanded.
    #[serde(default)]
    collapsed: bool,
    /// The dropdown rows, for oscillators, filters, and LFOs. Each one is a dropdown in order.
    #[serde(default)]
    choices: Vec<usize>,
    /// Graphs saved before there were several dropdowns kept one, which loads as `choices[0]`.
    #[serde(default, skip_serializing)]
    choice: usize,
    /// The switches, for the output node's mono and always-glide.
    #[serde(default)]
    switches: Vec<bool>,
    /// Graphs saved before there were switches kept one, for mono. It loads into `switches`.
    #[serde(default, skip_serializing)]
    switched: bool,
    /// The node's knob values, normalized. A knob that is missing keeps its default. Graphs saved
    /// before knobs lived on nodes have none, because those values were host parameters.
    #[serde(default)]
    knobs: Vec<(KnobId, f32)>,
    /// Fields from before the dropdown row and the knobs moved onto every node. They are read
    /// when loading and never written.
    #[serde(default, skip_serializing)]
    waveform: Option<Waveform>,
    #[serde(default, skip_serializing)]
    filter_type: Option<FilterType>,
    #[serde(default, skip_serializing)]
    out_min: Option<f32>,
    #[serde(default, skip_serializing)]
    out_max: Option<f32>,
}

impl SavedNode {
    fn new(node: &FlodeNode) -> Self {
        Self {
            kind: node.kind,
            x: node.position.x,
            y: node.position.y,
            collapsed: node.collapsed,
            choices: node.choices.to_vec(),
            choice: 0,
            switches: node.switches.to_vec(),
            switched: false,
            knobs: KnobId::ALL.iter().map(|&id| (id, node.value(id))).collect(),
            waveform: None,
            filter_type: None,
            out_min: None,
            out_max: None,
        }
    }

    fn into_node(&self) -> FlodeNode {
        let mut node = FlodeNode::new(self.kind, Point::new(self.x, self.y));
        node.collapsed = self.collapsed;
        node.choices = self.choices.clone();
        if self.choices.is_empty() {
            node.choices.push(self.choice);
        }
        node.switches = self.switches.clone();
        if self.switched {
            node.set_switch(0, true);
        }
        match self.kind {
            NodeKind::Oscillator => {
                if let Some(waveform) = self.waveform {
                    if node.choices.is_empty() {
                        node.choices.push(0);
                    }
                    node.choices[0] = waveform.index();
                }
            }
            NodeKind::Filter => {
                if let Some(filter_type) = self.filter_type {
                    if node.choices.is_empty() {
                        node.choices.push(0);
                    }
                    node.choices[0] = filter_type.index();
                }
            }
            NodeKind::Map => {
                if let Some(min) = self.out_min {
                    node.set_value(KnobId::MapMin, KnobId::MapMin.to_normalized(min));
                }
                if let Some(max) = self.out_max {
                    node.set_value(KnobId::MapMax, KnobId::MapMax.to_normalized(max));
                }
            }
            _ => {}
        }
        for &(id, value) in &self.knobs {
            node.set_value(id, value);
        }
        node
    }
}

/// Where the starting nodes sit: the oscillator at the far left and the output at the far right of
/// the default 1024 px editor, with a 24 px margin.
const DEFAULT_OSC_X: f32 = 24.0;
const DEFAULT_OUTPUT_X: f32 = 1024.0 - 24.0 - NODE_WIDTH;

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

/// Rewires old saves whose envelopes sat in the sound chain. The audio goes straight from the
/// envelope's input to its output, and the envelope drives the oscillator's gain instead, so the
/// sound stays the same.
fn migrate_envelope_audio(nodes: &[FlodeNode], links: Vec<Connection>) -> Vec<Connection> {
    let is_envelope = |index: usize| {
        nodes
            .get(index)
            .is_some_and(|n| n.kind == NodeKind::Envelope)
    };
    let mut result = Vec::new();
    let mut audio = Vec::new();
    for link in links {
        let touches_envelope_audio = (is_envelope(link.to) && link.input.is_audio())
            || (is_envelope(link.from) && link.out == Port::Audio);
        if touches_envelope_audio {
            audio.push(link);
        } else {
            result.push(link);
        }
    }
    for envelope in (0..nodes.len()).filter(|&index| is_envelope(index)) {
        let Some(input) = audio
            .iter()
            .find(|c| c.to == envelope && c.input.is_audio())
            .copied()
        else {
            continue;
        };
        for output in audio
            .iter()
            .filter(|c| c.from == envelope && c.out == Port::Audio)
        {
            result.push(Connection {
                from: input.from,
                out: input.out,
                to: output.to,
                input: output.input,
            });
        }
        // The oscillator's envelope input is the amplitude, so the envelope drives that.
        let amp = Port::Shape;
        if nodes[input.from].kind == NodeKind::Oscillator
            && !result.iter().any(|c| c.to == input.from && c.input == amp)
        {
            result.push(Connection {
                from: envelope,
                out: Port::Shape,
                to: input.from,
                input: amp,
            });
        }
    }
    result
}

/// True if a connection joins ports that exist and carry the same kind of signal. Audio goes to
/// audio, and control goes to a knob.
fn link_is_valid(nodes: &[FlodeNode], link: &Connection) -> bool {
    let (Some(from), Some(to)) = (nodes.get(link.from), nodes.get(link.to)) else {
        return false;
    };
    link.from != link.to
        && (link.out == Port::Audio) == link.input.is_audio()
        && from.output_point(link.out).is_some()
        && to.input_point(link.input).is_some()
}

/// The map knobs for the low and high ends of the curve: the input and output of each.
const MAP_IN: [KnobId; 2] = [KnobId::MapInMin, KnobId::MapInMax];
const MAP_OUT: [KnobId; 2] = [KnobId::MapMin, KnobId::MapMax];
/// How close a press must be to a curve handle to grab it.
const HANDLE_HIT_RADIUS: f32 = 9.0;
/// How many points the curve is drawn with.
const CURVE_SAMPLES: usize = 32;

/// Where the two handles of a map's curve sit, in editor coordinates.
fn map_handles(node: &FlodeNode) -> [Point; 2] {
    let bounds = node.curve_bounds();
    std::array::from_fn(|end| {
        let x = node.value(MAP_IN[end]);
        let y = node.value(MAP_OUT[end]);
        Point::new(
            bounds.x + x * bounds.width,
            bounds.y + (1.0 - y) * bounds.height,
        )
    })
}

/// The handle of a map under `point`, if any.
fn map_handle_at(node: &FlodeNode, point: Point) -> Option<usize> {
    if node.collapsed || !node.has_curve() {
        return None;
    }
    map_handles(node).iter().position(|handle| {
        let dx = handle.x - point.x;
        let dy = handle.y - point.y;
        dx * dx + dy * dy <= HANDLE_HIT_RADIUS * HANDLE_HIT_RADIUS
    })
}

impl Graph {
    fn with(nodes: Vec<FlodeNode>, connections: Vec<Connection>) -> Self {
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
    fn hover_at(&self, point: Point) -> Option<Hover> {
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
    fn terminal_at(&self, point: Point) -> Option<Terminal> {
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
    fn node_at(&self, point: Point) -> Option<usize> {
        (0..self.nodes.len())
            .rev()
            .find(|&index| self.nodes[index].bounds().contains(point))
    }

    /// True if the envelope shape drives this knob. A driven knob can't be dragged.
    fn is_driven(&self, node: usize, id: KnobId) -> bool {
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

    /// The envelope shape's path to a knob, as the linear map `value = a + b * shape`. Each map
    /// on the path rescales the value it receives into its own min and max. The path is only
    /// valid if it ends at an envelope's shape pin. Returns None for a knob without one.
    pub fn control_route(&self, node: usize, id: KnobId) -> Option<Route> {
        let mut link = self
            .connections
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
                    return Self::compose(&self.nodes, &maps, Source::Envelope(index));
                }
                NodeKind::Lfo => {
                    let index = self.lfo_index(source)?;
                    return Self::compose(&self.nodes, &maps, Source::Lfo(index));
                }
                NodeKind::Input => {
                    // An input node's pins are the MIDI sources, so the wire says which one.
                    let Port::Midi(midi) = link.out else {
                        return None;
                    };
                    return Self::compose(&self.nodes, &maps, Source::Midi(midi));
                }
                NodeKind::Map => {
                    maps.push(source);
                    link = self
                        .connections
                        .iter()
                        .find(|c| c.to == source && c.input == Port::Shape)?;
                }
                _ => return None,
            }
        }
        None
    }

    /// Applies the maps to a source in signal order, starting from the source.
    fn compose(nodes: &[FlodeNode], maps: &[usize], source: Source) -> Option<Route> {
        if maps.len() > MAX_MAPS {
            return None;
        }
        // The maps are stored from the knob back toward the source, so reverse them to signal order.
        let mut stages = [MapStage::IDENTITY; MAX_MAPS];
        for (slot, &index) in maps.iter().rev().enumerate() {
            stages[slot] = map::stage(&nodes[index]);
        }
        Some(Route {
            source,
            stages,
            count: maps.len(),
        })
    }

    /// The level of the signal on a control output, as the live control levels give it. Maps on
    /// the way apply their curves.
    fn control_level(
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
                let link = self
                    .connections
                    .iter()
                    .find(|c| c.to == from && c.input == Port::Shape)?;
                let input = self.control_level(link.from, link.out, levels, depth + 1)?;
                Some(map::stage(&self.nodes[from]).apply(input))
            }
            _ => None,
        }
    }

    /// The level arriving at a map's input, if a control source feeds it.
    fn map_input_level(&self, node: usize, levels: &ControlLevels) -> Option<f32> {
        let link = self
            .connections
            .iter()
            .find(|c| c.to == node && c.input == Port::Shape)?;
        self.control_level(link.from, link.out, levels, 0)
    }

    /// The index of the envelope wired to an oscillator's input, if one is.
    fn amp_envelope(&self, oscillator: usize) -> Option<usize> {
        let link = self
            .connections
            .iter()
            .find(|c| c.to == oscillator && c.input == Port::Shape)?;
        if self.nodes.get(link.from)?.kind != NodeKind::Envelope {
            return None;
        }
        self.envelope_index(link.from)
    }

    /// The index of an envelope among the envelope nodes in graph order, or None past the
    /// envelopes the audio thread runs.
    fn envelope_index(&self, node: usize) -> Option<usize> {
        let index = self.nodes[..node]
            .iter()
            .filter(|n| n.kind == NodeKind::Envelope)
            .count();
        (index < MAX_ENVELOPES).then_some(index)
    }

    /// The envelopes the audio thread runs, in graph order. Each one's knob values are read per
    /// block, so an envelope can have its times driven too.
    pub fn envelope_settings(&self) -> [EnvelopeSettings; MAX_ENVELOPES] {
        let mut out = [EnvelopeSettings::OFF; MAX_ENVELOPES];
        let envelopes = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.kind == NodeKind::Envelope);
        for (slot, (index, _)) in envelopes.take(MAX_ENVELOPES).enumerate() {
            out[slot] = EnvelopeSettings {
                active: true,
                attack: self.knob_value(index, KnobId::Attack),
                decay: self.knob_value(index, KnobId::Decay),
                sustain: self.knob_value(index, KnobId::Sustain),
                release: self.knob_value(index, KnobId::Release),
            };
        }
        out
    }

    /// The index of an LFO among the LFO nodes in graph order, or None past the LFOs the audio
    /// thread runs.
    fn lfo_index(&self, node: usize) -> Option<usize> {
        let index = self.nodes[..node]
            .iter()
            .filter(|n| n.kind == NodeKind::Lfo)
            .count();
        (index < MAX_LFOS).then_some(index)
    }

    /// The LFOs the audio thread runs, in graph order.
    pub fn lfo_settings(&self) -> [LfoSettings; MAX_LFOS] {
        let mut out = [LfoSettings::OFF; MAX_LFOS];
        let lfos = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.kind == NodeKind::Lfo);
        for (slot, (index, node)) in lfos.take(MAX_LFOS).enumerate() {
            out[slot] = LfoSettings {
                active: true,
                waveform: lfo::waveform(node),
                rate: self.knob_value(index, KnobId::LfoRate),
                sync: self.knob_value(index, KnobId::LfoSync),
            };
        }
        out
    }

    /// A knob as the audio thread reads it: the node's own value and the route that drives it.
    fn knob_value(&self, node: usize, id: KnobId) -> KnobValue {
        KnobValue {
            value: self.nodes[node].value(id),
            route: self.control_route(node, id),
        }
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
            .filter(|link| link_is_valid(&nodes, link))
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

    fn first_output(&self) -> Option<usize> {
        self.nodes.iter().position(|n| n.kind == NodeKind::Output)
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

    /// The settings of the output node that apply to every slot.
    pub fn globals(&self) -> Globals {
        match self.first_output() {
            Some(output) => Globals {
                mono: self.nodes[output].switch(0),
                glide_always: self.nodes[output].switch(1),
                gain: self.knob_value(output, KnobId::Gain),
                glide: self.knob_value(output, KnobId::Glide),
            },
            None => Globals::SILENT,
        }
    }

    /// Copies the knobs of one node into a knob table, such as a lane's or a mix branch's.
    fn copy_knobs(&self, knobs: &mut [KnobValue; KnobId::COUNT], node: usize) {
        for id in self.nodes[node].rows().iter().filter_map(|row| row.knob) {
            knobs[id as usize] = self.knob_value(node, id);
        }
    }

    /// An oscillator's settings and knobs, as one of a mix node's two branches.
    fn oscillator_branch(&self, oscillator: usize) -> Branch {
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
    fn mix_branch(&self, mix_node: usize, which: usize) -> Branch {
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
    fn lane_chain(&self, output: usize, slot: usize) -> Option<LaneChain> {
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
    fn disconnect_input(&mut self, to: usize, input: Port) -> Option<Connection> {
        let index = self
            .connections
            .iter()
            .position(|c| c.to == to && c.input == input)?;
        let link = self.connections.remove(index);
        self.refresh_slots();
        Some(link)
    }

    /// The four points of a connection's curve, or None if its ports no longer exist.
    fn link_curve(&self, link: &Connection) -> Option<[Point; 4]> {
        let from = self.nodes.get(link.from)?.output_point(link.out)?;
        let to = self.nodes.get(link.to)?.input_point(link.input)?;
        Some(wire_curve(from, to))
    }

    /// Sets how many slot pins each output node shows. A pin shows for each connected slot, and
    /// one empty pin sits below the last of them.
    fn refresh_slots(&mut self) {
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
    fn link_under(&self, node: usize) -> Option<usize> {
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
    fn insert_into(&mut self, node: usize, link: usize) {
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
    fn connect(&mut self, from: usize, out: Port, to: usize, input: Port) {
        let link = Connection {
            from,
            out,
            to,
            input,
        };
        if !link_is_valid(&self.nodes, &link) {
            return;
        }
        self.connections
            .retain(|c| !(c.to == to && c.input == input));
        self.connections.push(link);
        self.refresh_slots();
    }

    /// Adds a node and selects it. Returns its index.
    fn add_node(&mut self, kind: NodeKind, position: Point) -> usize {
        self.nodes.push(FlodeNode::new(kind, position));
        let index = self.nodes.len() - 1;
        self.selected = Some(index);
        index
    }

    /// Removes a node and its connections. Connections and the selection after it shift down
    /// to match the new node indices.
    fn delete_node(&mut self, index: usize) {
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

fn near(a: Point, b: Point) -> bool {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy <= TERMINAL_HIT_RADIUS * TERMINAL_HIT_RADIUS
}

/// The pill at the top center of the editor.
fn pill_bounds(bounds: Rectangle) -> Rectangle {
    Rectangle {
        x: (bounds.width - PILL_WIDTH) / 2.0,
        y: PILL_TOP,
        width: PILL_WIDTH,
        height: PILL_HEIGHT,
    }
}

/// The part of the pill under `point`.
fn pill_button_at(bounds: Rectangle, point: Point) -> Option<PillItem> {
    let pill = pill_bounds(bounds);
    if !pill.contains(point) {
        return None;
    }
    let segment = pill.width / PILL_ITEMS.len() as f32;
    let index = (((point.x - pill.x) / segment) as usize).min(PILL_ITEMS.len() - 1);
    Some(PILL_ITEMS[index])
}

/// The rectangle of one pill button.
fn pill_segment(bounds: Rectangle, index: usize) -> Rectangle {
    let pill = pill_bounds(bounds);
    let segment = pill.width / PILL_ITEMS.len() as f32;
    Rectangle {
        x: pill.x + index as f32 * segment,
        y: pill.y,
        width: segment,
        height: pill.height,
    }
}

/// The list that opens below a category's pill button.
fn category_list_bounds(bounds: Rectangle, category: Category) -> Rectangle {
    let index = PILL_ITEMS
        .iter()
        .position(|item| *item == PillItem::Category(category))
        .unwrap_or(0);
    let segment = pill_segment(bounds, index);
    Rectangle {
        x: segment.x,
        y: segment.y + segment.height + LIST_GAP,
        width: segment.width.max(CATEGORY_MIN_WIDTH),
        height: category.kinds().len().max(1) as f32 * MENU_ITEM_HEIGHT,
    }
}

/// The node kind for the row of a category's list under `point`, if any.
fn category_row_at(bounds: Rectangle, category: Category, point: Point) -> Option<NodeKind> {
    let list = category_list_bounds(bounds, category);
    if !list.contains(point) {
        return None;
    }
    let index = ((point.y - list.y) / MENU_ITEM_HEIGHT) as usize;
    category.kinds().get(index).copied()
}

fn menu_bounds(menu: ContextMenu) -> Rectangle {
    Rectangle {
        x: menu.position.x,
        y: menu.position.y,
        width: MENU_WIDTH,
        height: MENU_ITEM_HEIGHT,
    }
}

/// Borrows the graph for one frame of the editor.
pub struct NodeEditor<'a> {
    graph: &'a mut Graph,
    /// The live control levels, shown on knobs that a control wire drives.
    levels: ControlLevels,
    /// The notes the synth is playing, shown as dots on the key strip.
    active_notes: [u64; 2],
}

impl<'a> NodeEditor<'a> {
    pub fn new(graph: &'a mut Graph, levels: ControlLevels, active_notes: [u64; 2]) -> Self {
        Self {
            graph,
            levels,
            active_notes,
        }
    }

    /// `point` is in editor coordinates. `screen` is the mouse position, which knob drags use so
    /// their speed does not change with the zoom.
    fn press(
        &mut self,
        point: Point,
        screen: Point,
        bounds: Rectangle,
    ) -> (event::Status, Option<Message>) {
        // A press while placing sets the node down, and nothing else happens on that press.
        if let Interaction::Placing { node, .. } = self.graph.interaction {
            self.graph.interaction = Interaction::Idle;
            if let Some(link) = self.graph.link_target.take() {
                self.graph.insert_into(node, link);
            }
            return (event::Status::Captured, Some(Message::GraphChanged));
        }

        // An open dropdown list takes the press. Its row picks the value. Any other press closes
        // the list and goes through as usual.
        if let Some((index, which)) = self.graph.dropdown_menu.take() {
            let node = &self.graph.nodes[index];
            if let Some(dropdown) = node.dropdowns().into_iter().find(|d| d.which == which) {
                let rows = dropdown.labels.len();
                let list = dropdown_list_bounds(dropdown.bounds, rows);
                if let Some(row) = list_row_at(list, point, rows) {
                    self.graph.nodes[index].choose(which, row);
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
            }
        }

        // An open menu takes the press. Its item deletes the node. A press anywhere else closes
        // the menu and then goes through as usual.
        if let Some(menu) = self.graph.menu.take() {
            if menu_bounds(menu).contains(point) {
                self.graph.delete_node(menu.node);
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
        }

        // An open category list takes the press. Its row adds the node. Any other press closes it.
        let open_category = self.graph.category_menu.take();
        if let Some(category) = open_category {
            if let Some(kind) = category_row_at(bounds, category, screen) {
                self.place(kind, point);
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
        }

        match pill_button_at(bounds, screen) {
            Some(PillItem::Kind(kind)) => {
                self.place(kind, point);
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            Some(PillItem::Category(category)) => {
                // Pressing the same category again closes its list.
                if open_category != Some(category) {
                    self.graph.category_menu = Some(category);
                }
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            None => {}
        }

        match self.graph.terminal_at(point) {
            Some(Terminal::Out(from, out)) => {
                self.graph.interaction = Interaction::Wiring {
                    from,
                    out,
                    cursor: point,
                };
                return (event::Status::Captured, None);
            }
            // Pressing a connected input detaches its wire and drags it from the original output.
            // Releasing on empty space leaves the input disconnected.
            Some(Terminal::In(to, input)) => {
                if let Some(link) = self.graph.disconnect_input(to, input) {
                    self.graph.interaction = Interaction::Wiring {
                        from: link.from,
                        out: link.out,
                        cursor: point,
                    };
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
            }
            None => {}
        }

        // Walk nodes from the last drawn (top) to the first.
        for index in (0..self.graph.nodes.len()).rev() {
            let node = &self.graph.nodes[index];
            if let Some(end) = map_handle_at(node, point) {
                self.graph.selected = Some(index);
                self.graph.interaction = Interaction::MapHandle { node: index, end };
                let node = &mut self.graph.nodes[index];
                let current = node.value(MAP_IN[end]);
                let state = &mut node.knob_states[MAP_IN[end] as usize];
                if let Press::Reset = knob::press(state, current, screen) {
                    for id in MAP_IN.into_iter().chain(MAP_OUT) {
                        node.reset_value(id);
                    }
                }
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            for slot in node.knob_slots() {
                // A driven knob is greyed out, so pressing it does nothing to the knob.
                let driven = slot.pinned && self.graph.is_driven(index, slot.id);
                if !driven && knob::contains(slot.center, ROW_KNOB_DIAMETER, point) {
                    self.graph.selected = Some(index);
                    self.graph.interaction = Interaction::Knob {
                        node: index,
                        id: slot.id,
                    };
                    let node = &mut self.graph.nodes[index];
                    let current = node.value(slot.id);
                    let state = &mut node.knob_states[slot.id as usize];
                    if let Press::Reset = knob::press(state, current, screen) {
                        node.reset_value(slot.id);
                    }
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
            }
            if node.toggle_bounds().contains(point) {
                self.graph.selected = Some(index);
                self.graph.dropdown_menu = None;
                self.graph.nodes[index].collapsed = !self.graph.nodes[index].collapsed;
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            {
                let labels = node.switch_labels();
                let hit = (0..labels.len()).find(|&i| node.switch_bounds(i).contains(point));
                if let (Some(hit), false) = (hit, node.collapsed) {
                    self.graph.selected = Some(index);
                    let node = &mut self.graph.nodes[index];
                    let on = !node.switch(hit);
                    node.set_switch(hit, on);
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
            }
            if let Some(which) = node.dropdown_at(point) {
                self.graph.selected = Some(index);
                self.graph.dropdown_menu = Some((index, which));
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            if node.bounds().contains(point) {
                self.graph.selected = Some(index);
                if node.header_bounds().contains(point) {
                    let grab = Vector::new(point.x - node.position.x, point.y - node.position.y);
                    self.graph.interaction = Interaction::MovingNode { node: index, grab };
                }
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
        }

        // Empty canvas clears the selection.
        self.graph.selected = None;
        (event::Status::Captured, Some(Message::GraphChanged))
    }

    /// Adds a node of `kind` centered on `point` and makes it follow the cursor until the next
    /// press places it.
    fn place(&mut self, kind: NodeKind, point: Point) {
        let grab = Vector::new(NODE_WIDTH / 2.0, HEADER_HEIGHT / 2.0);
        let node = self
            .graph
            .add_node(kind, Point::new(point.x - grab.x, point.y - grab.y));
        self.graph.interaction = Interaction::Placing { node, grab };
    }

    /// Right press: selects the node under the cursor and opens its menu. A right press
    /// anywhere else only closes the menus.
    fn secondary_press(&mut self, point: Point) -> (event::Status, Option<Message>) {
        self.graph.menu = None;
        self.graph.dropdown_menu = None;
        if let Some(index) = self.graph.node_at(point) {
            self.graph.selected = Some(index);
            self.graph.menu = Some(ContextMenu {
                node: index,
                position: point,
            });
        }
        (event::Status::Captured, Some(Message::GraphChanged))
    }

    fn release(&mut self, point: Point) -> (event::Status, Option<Message>) {
        // Placing ends on the next press, not on the release of the press that started it.
        if let Interaction::Placing { .. } = self.graph.interaction {
            return (event::Status::Captured, None);
        }
        let interaction = std::mem::replace(&mut self.graph.interaction, Interaction::Idle);
        match interaction {
            Interaction::Wiring { from, out, .. } => {
                if let Some(Terminal::In(to, input)) = self.graph.terminal_at(point) {
                    self.graph.connect(from, out, to, input);
                }
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::Knob { node, id } => {
                knob::release(&mut self.graph.nodes[node].knob_states[id as usize]);
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::MapHandle { node, end } => {
                knob::release(&mut self.graph.nodes[node].knob_states[MAP_IN[end] as usize]);
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::MovingNode { node, .. } => {
                if let Some(link) = self.graph.link_target.take() {
                    self.graph.insert_into(node, link);
                }
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::Placing { .. } => (event::Status::Captured, None),
            Interaction::Idle => (event::Status::Ignored, None),
        }
    }

    fn drag(&mut self, point: Point, screen: Point) -> (event::Status, Option<Message>) {
        match self.graph.interaction {
            Interaction::MovingNode { node, grab } | Interaction::Placing { node, grab } => {
                self.graph.nodes[node].position = Point::new(point.x - grab.x, point.y - grab.y);
                self.graph.link_target = self.graph.link_under(node);
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::Wiring { from, out, .. } => {
                self.graph.interaction = Interaction::Wiring {
                    from,
                    out,
                    cursor: point,
                };
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::MapHandle { node, end } => {
                // The handle follows the cursor over the curve box. Its x is the input, and its y
                // is the output, so both ends move together.
                let bounds = self.graph.nodes[node].curve_bounds();
                let x = ((point.x - bounds.x) / bounds.width).clamp(0.0, 1.0);
                let y = (1.0 - (point.y - bounds.y) / bounds.height).clamp(0.0, 1.0);
                let node = &mut self.graph.nodes[node];
                node.set_value(MAP_IN[end], x);
                node.set_value(MAP_OUT[end], y);
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::Knob { node, id } => {
                let state = &self.graph.nodes[node].knob_states[id as usize];
                if let Some(value) = knob::drag(state, screen) {
                    self.graph.nodes[node].set_value(id, value);
                }
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::Idle => (event::Status::Ignored, None),
        }
    }
}

/// Messages from the node editor. Graph changes republish the audio routing and save the graph.
#[derive(Debug, Clone, Copy)]
pub enum Message {
    /// A note from the computer keyboard. The editor forwards it only in standalone mode.
    Note(PianoEvent),
    GraphChanged,
    /// The cursor moved onto or off a dropdown. Only a redraw is needed.
    Hovered,
}

impl<'a> Program<Message> for NodeEditor<'a> {
    fn update(
        &mut self,
        event: Event,
        bounds: Rectangle,
        cursor: Cursor,
    ) -> (event::Status, Option<Message>) {
        let mouse_event = match event {
            Event::Mouse(mouse_event) => mouse_event,
            Event::Keyboard(key_event) => {
                let status = if Piano::uses(&key_event) {
                    event::Status::Captured
                } else {
                    event::Status::Ignored
                };
                let note = self.graph.piano.handle(key_event);
                return (status, note.map(Message::Note));
            }
        };
        // Positions are relative to the editor's top-left corner. Drags keep working when the
        // cursor leaves the editor, so only the press checks the bounds. When the cursor is above or
        // beside the window its position is unknown, but a release must still end the drag. So a
        // release uses the last position seen, and a move with no position is ignored.
        let position = cursor.position_from(bounds.position());
        if let Some(position) = position {
            self.graph.last_screen = position;
        }
        let released = matches!(
            mouse_event,
            mouse::Event::ButtonReleased(mouse::Button::Left)
        );
        if position.is_none() && !released {
            return (event::Status::Ignored, None);
        }
        let screen = self.graph.last_screen;
        let point = self.graph.view.to_world(screen);

        // The key strip takes the mouse before the graph does. A key plays while the mouse holds it.
        let strip = piano::strip_bounds(bounds);
        match mouse_event {
            mouse::Event::ButtonPressed(mouse::Button::Left) if strip.contains(screen) => {
                let note = piano::key_at(strip, screen);
                self.graph.mouse_note = note;
                return match note {
                    Some(note) => (
                        event::Status::Captured,
                        Some(Message::Note(PianoEvent::NoteOn {
                            note,
                            velocity: piano::CLICK_VELOCITY,
                        })),
                    ),
                    None => (event::Status::Captured, None),
                };
            }
            mouse::Event::ButtonReleased(mouse::Button::Left)
                if self.graph.mouse_note.is_some() =>
            {
                let note = self.graph.mouse_note.take();
                return (
                    event::Status::Captured,
                    note.map(|note| Message::Note(PianoEvent::NoteOff { note })),
                );
            }
            _ => {}
        }

        match mouse_event {
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                if !cursor.is_over(&bounds) {
                    return (event::Status::Ignored, None);
                }
                self.press(point, screen, bounds)
            }
            mouse::Event::ButtonPressed(mouse::Button::Right) => {
                if !cursor.is_over(&bounds) {
                    return (event::Status::Ignored, None);
                }
                self.secondary_press(point)
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) => self.release(point),
            mouse::Event::CursorMoved { .. } => {
                let hover = self.graph.hover_at(point);
                let changed = hover != self.graph.hover;
                self.graph.hover = hover;
                match self.drag(point, screen) {
                    (event::Status::Ignored, None) if changed => {
                        (event::Status::Ignored, Some(Message::Hovered))
                    }
                    other => other,
                }
            }
            mouse::Event::WheelScrolled { delta } => {
                if !cursor.is_over(&bounds) {
                    return (event::Status::Ignored, None);
                }
                match view_scroll(delta) {
                    ViewScroll::Zoom(magnification) => {
                        self.graph.view.zoom_at(screen, 1.0 + magnification)
                    }
                    ViewScroll::Pan(by) => self.graph.view.pan(by),
                }
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            _ => (event::Status::Ignored, None),
        }
    }

    fn draw(&self, bounds: Rectangle, _cursor: Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(bounds.size());
        let graph = &*self.graph;
        let view = graph.view;
        let offset = view.offset;

        // Iced draws all text above all shapes in a frame. Node text under an open overlay would
        // show through it, so node text is hidden inside the overlays. The overlays are kept in
        // editor coordinates. The pill is in screen coordinates, so it is converted.
        // The key strip covers nodes too, so its area is an overlay as well.
        let mut overlays = vec![
            view.rect_to_world(pill_bounds(bounds)),
            view.rect_to_world(piano::strip_bounds(bounds)),
        ];
        if let Some((index, which)) = graph.dropdown_menu {
            let node = &graph.nodes[index];
            if let Some(dropdown) = node.dropdowns().into_iter().find(|d| d.which == which) {
                // The pin labels sit left of the list, so the cover spans the node's full width.
                let list = dropdown_list_bounds(dropdown.bounds, dropdown.labels.len());
                overlays.push(Rectangle {
                    x: node.position.x,
                    y: list.y,
                    width: NODE_WIDTH,
                    height: list.height,
                });
            }
        }
        if let Some(menu) = graph.menu {
            overlays.push(menu_bounds(menu));
        }
        if let Some(category) = graph.category_menu {
            overlays.push(view.rect_to_world(category_list_bounds(bounds, category)));
        }

        // The nodes and wires are drawn in editor coordinates, then the view scales and moves them.
        frame.with_save(|frame| {
            frame.translate(offset);
            frame.scale(view.zoom);

            // Ordinary wires go under the nodes. The wire a dragged node would splice into, and the
            // wire being dragged, go over them so they stay visible.
            for (index, connection) in graph.connections.iter().enumerate() {
                if graph.link_target == Some(index) {
                    continue;
                }
                if let Some(curve) = graph.link_curve(connection) {
                    draw_curve(frame, &curve, wire_color(connection.out), 2.0);
                }
            }

            for index in 0..graph.nodes.len() {
                draw_node(frame, index, graph, self.levels, &overlays);
            }

            if let Some(index) = graph.link_target {
                if let Some(curve) = graph.link_curve(&graph.connections[index]) {
                    draw_curve(frame, &curve, HIGHLIGHT_WIRE_COLOR, 4.0);
                }
            }

            if let Interaction::Wiring { from, out, cursor } = graph.interaction {
                if let Some(start) = graph.nodes[from].output_point(out) {
                    let color = if out == Port::Audio {
                        PENDING_WIRE_COLOR
                    } else {
                        CONTROL_COLOR
                    };
                    draw_curve(frame, &wire_curve(start, cursor), color, 2.0);
                }
            }

            if let Some(index) = graph.selected {
                draw_selection(frame, &graph.nodes[index]);
            }
        });

        // The pill stays fixed on screen. It is drawn after the nodes, so it covers them.
        draw_pill(&mut frame, bounds);

        let strip = piano::strip_bounds(bounds);
        piano::draw_strip(&mut frame, strip, self.active_notes, graph.mouse_note);

        // The open lists and the menu are drawn over the pill, in editor coordinates like the nodes.
        frame.with_save(|frame| {
            frame.translate(offset);
            frame.scale(view.zoom);

            if let Some((index, which)) = graph.dropdown_menu {
                let node = &graph.nodes[index];
                if let Some(dropdown) = node.dropdowns().into_iter().find(|d| d.which == which) {
                    let list = dropdown_list_bounds(dropdown.bounds, dropdown.labels.len());
                    let hovered_row = match graph.hover {
                        Some(Hover::Row {
                            node,
                            which: hovered,
                            row,
                        }) if node == index && hovered == which => Some(row),
                        _ => None,
                    };
                    draw_list(
                        frame,
                        list,
                        &dropdown.labels,
                        dropdown.selected,
                        hovered_row,
                        view.zoom,
                    );
                }
            }

            if let Some(menu) = graph.menu {
                draw_menu(frame, menu, view.zoom);
            }
        });

        if let Some(category) = graph.category_menu {
            draw_category_list(&mut frame, bounds, category);
        }

        vec![frame.into_geometry()]
    }
}

/// Audio is grey. Every other signal is green.
fn wire_color(out: Port) -> Color {
    if out == Port::Audio {
        WIRE_COLOR
    } else {
        CONTROL_COLOR
    }
}

/// The four points of a wire's cubic curve. Control points stay at least 60 pixels out so short
/// wires still curve.
fn wire_curve(from: Point, to: Point) -> [Point; 4] {
    let reach = ((to.x - from.x).abs() * 0.5).max(60.0);
    [
        from,
        Point::new(from.x + reach, from.y),
        Point::new(to.x - reach, to.y),
        to,
    ]
}

fn bezier_point(curve: &[Point; 4], t: f32) -> Point {
    let u = 1.0 - t;
    let a = u * u * u;
    let b = 3.0 * u * u * t;
    let c = 3.0 * u * t * t;
    let d = t * t * t;
    Point::new(
        a * curve[0].x + b * curve[1].x + c * curve[2].x + d * curve[3].x,
        a * curve[0].y + b * curve[1].y + c * curve[2].y + d * curve[3].y,
    )
}

fn draw_curve(frame: &mut Frame, curve: &[Point; 4], color: Color, width: f32) {
    let [p0, p1, p2, p3] = *curve;
    let path = Path::new(|builder| {
        builder.move_to(p0);
        builder.bezier_curve_to(p1, p2, p3);
    });
    frame.stroke(
        &path,
        Stroke::default()
            .with_color(color)
            .with_width(width)
            .with_line_cap(LineCap::Round),
    );
}

fn draw_node(
    frame: &mut Frame,
    index: usize,
    graph: &Graph,
    levels: ControlLevels,
    covered: &[Rectangle],
) {
    let node = &graph.nodes[index];
    let zoom = graph.view.zoom;
    let bounds = node.bounds();
    frame.fill(&rounded_rect(bounds, CORNER_RADIUS), NODE_BODY_COLOR);

    // The header is a rounded rectangle with its bottom corners squared off.
    let header = node.header_bounds();
    frame.fill(&rounded_rect(header, CORNER_RADIUS), NODE_HEADER_COLOR);
    frame.fill(
        &Path::rectangle(
            Point::new(header.x, header.y + header.height - CORNER_RADIUS),
            nih_plug_iced::Size::new(header.width, CORNER_RADIUS),
        ),
        NODE_HEADER_COLOR,
    );

    draw_text(
        frame,
        covered,
        zoom,
        Text {
            content: node.title().to_string(),
            position: Point::new(header.x + TITLE_INSET, header.y + HEADER_HEIGHT / 2.0),
            color: TEXT_COLOR,
            size: 14.0,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        },
    );

    draw_collapse_arrow(frame, node);

    // A collapsed node shows only its header. Its pins are dots at the header's edges, and wires
    // still attach to them.
    if node.collapsed {
        for port in node.input_ports() {
            if let Some(point) = node.input_point(port) {
                draw_pin_dot(frame, point, port);
            }
        }
        for port in node.output_ports() {
            if let Some(point) = node.output_point(port) {
                draw_pin_dot(frame, point, port);
            }
        }
        return;
    }

    // Each row has an optional input pin on the left edge, a knob with its name and value, and an
    // optional output pin on the right edge.
    for (row_index, row) in node.rows().iter().enumerate() {
        let y = node.position.y + pin_y(row_index);
        let left = node.position.x;
        let right = node.position.x + NODE_WIDTH;

        if let Some(id) = row.knob {
            let driven = graph.is_driven(index, id);
            let text_color = if driven { DIM_TEXT_COLOR } else { TEXT_COLOR };
            draw_text(
                frame,
                covered,
                zoom,
                Text {
                    content: id.name().to_string(),
                    position: Point::new(left + PIN_LABEL_INSET, y + NAME_OFFSET),
                    color: text_color,
                    size: PIN_LABEL_SIZE,
                    vertical_alignment: alignment::Vertical::Center,
                    ..Text::default()
                },
            );
            draw_text(
                frame,
                covered,
                zoom,
                Text {
                    content: id.format(node.value(id)),
                    position: Point::new(left + PIN_LABEL_INSET, y + VALUE_OFFSET),
                    color: text_color,
                    size: PIN_LABEL_SIZE,
                    vertical_alignment: alignment::Vertical::Center,
                    ..Text::default()
                },
            );
        }

        if let Some(port) = row.input {
            let point = Point::new(left, y);
            draw_pin_dot(frame, point, port);
            // A pin with no knob to name it draws its own label, such as "Shape" or "A".
            if row.knob.is_none() {
                if let Some(label) = port.input_label() {
                    draw_text(
                        frame,
                        covered,
                        zoom,
                        Text {
                            content: label.to_string(),
                            position: Point::new(point.x + PIN_LABEL_INSET, point.y),
                            color: TEXT_COLOR,
                            size: PIN_LABEL_SIZE,
                            vertical_alignment: alignment::Vertical::Center,
                            ..Text::default()
                        },
                    );
                }
            }
        }

        if let Some(port) = row.output {
            let point = Point::new(right, y);
            draw_pin_dot(frame, point, port);
            if let Some(label) = port.output_label() {
                draw_text(
                    frame,
                    covered,
                    zoom,
                    Text {
                        content: label.to_string(),
                        position: Point::new(point.x - PIN_LABEL_INSET, point.y),
                        color: TEXT_COLOR,
                        size: PIN_LABEL_SIZE,
                        horizontal_alignment: alignment::Horizontal::Right,
                        vertical_alignment: alignment::Vertical::Center,
                        ..Text::default()
                    },
                );
            }
        }
    }

    {
        let labels = node.switch_labels();
        for (index, label) in labels.iter().enumerate() {
            draw_switch(frame, node, index, label, covered, zoom);
        }
    }

    for dropdown in node.dropdowns() {
        let hovered = graph.hover
            == Some(Hover::Button {
                node: index,
                which: dropdown.which,
            });
        draw_dropdown(
            frame,
            dropdown.bounds,
            dropdown.name,
            dropdown.labels[dropdown.selected],
            hovered,
            zoom,
            covered,
        );
    }

    if node.has_curve() {
        draw_map_curve(frame, index, graph, levels);
    }

    for slot in node.knob_slots() {
        let route = if slot.pinned {
            graph.control_route(index, slot.id)
        } else {
            None
        };
        // A driven knob shows the value its control route sets for the live signal. Its own value
        // is ignored while it's driven.
        let value = match route {
            Some(route) => route.apply(&levels),
            None => node.value(slot.id),
        };
        knob::draw(
            frame,
            slot.center,
            ROW_KNOB_DIAMETER,
            value,
            route.is_some(),
        );
    }
}

/// The arrow in the header. It points down when the node is open and right when it is collapsed.
fn draw_collapse_arrow(frame: &mut Frame, node: &FlodeNode) {
    let x = node.position.x + ARROW_CENTER_X;
    let y = node.position.y + HEADER_HEIGHT / 2.0;
    let path = if node.collapsed {
        Path::new(|builder| {
            builder.move_to(Point::new(x - 2.5, y - 4.0));
            builder.line_to(Point::new(x + 3.0, y));
            builder.line_to(Point::new(x - 2.5, y + 4.0));
            builder.close();
        })
    } else {
        Path::new(|builder| {
            builder.move_to(Point::new(x - 4.0, y - 2.5));
            builder.line_to(Point::new(x + 4.0, y - 2.5));
            builder.line_to(Point::new(x, y + 3.0));
            builder.close();
        })
    };
    frame.fill(&path, TEXT_COLOR);
}

/// A pin without its label. Control pins are green and audio pins are grey.
fn draw_pin_dot(frame: &mut Frame, point: Point, port: Port) {
    if port.is_audio() {
        frame.fill(&Path::circle(point, PIN_RADIUS), TERMINAL_COLOR);
    } else {
        frame.fill(&Path::circle(point, CONTROL_PIN_RADIUS), CONTROL_COLOR);
    }
}

/// Draws node text unless its position is inside an overlay that covers the node.
fn draw_text(frame: &mut Frame, covered: &[Rectangle], zoom: f32, mut text: Text) {
    text.size *= zoom;
    if !covered.iter().any(|rect| rect.contains(text.position)) {
        frame.fill_text(text);
    }
}

/// The transfer curve of a map: the mapping line, its two handles, and a dot where the live input
/// lands on the curve.
fn draw_map_curve(frame: &mut Frame, index: usize, graph: &Graph, levels: ControlLevels) {
    let node = &graph.nodes[index];
    let bounds = node.curve_bounds();
    frame.fill(
        &rounded_rect(bounds, DROPDOWN_CORNER_RADIUS),
        DROPDOWN_COLOR,
    );

    let stage = map::stage(node);
    let line = Path::new(|builder| {
        for i in 0..=CURVE_SAMPLES {
            let x = i as f32 / CURVE_SAMPLES as f32;
            let y = stage.apply(x);
            let point = Point::new(
                bounds.x + x * bounds.width,
                bounds.y + (1.0 - y) * bounds.height,
            );
            if i == 0 {
                builder.move_to(point);
            } else {
                builder.line_to(point);
            }
        }
    });
    frame.stroke(
        &line,
        Stroke::default()
            .with_color(TEXT_COLOR)
            .with_width(1.5)
            .with_line_cap(LineCap::Round),
    );

    for handle in map_handles(node) {
        frame.fill(&Path::circle(handle, 4.0), TEXT_COLOR);
    }

    if let Some(input) = graph.map_input_level(index, &levels) {
        let output = stage.apply(input);
        let dot = Point::new(
            bounds.x + input * bounds.width,
            bounds.y + (1.0 - output) * bounds.height,
        );
        frame.fill(&Path::circle(dot, 3.5), piano::SOUNDING_DOT_COLOR);
    }
}

/// A switch: a box with a check when on, and its label. The output's "Always" is dimmed while
/// mono is off, since it has no effect then.
fn draw_switch(
    frame: &mut Frame,
    node: &FlodeNode,
    index: usize,
    label: &str,
    covered: &[Rectangle],
    zoom: f32,
) {
    let row = node.switch_bounds(index);
    let enabled = index == 0 || node.switch(0);
    let text_color = if enabled { TEXT_COLOR } else { DIM_TEXT_COLOR };
    let box_x = row.x + 2.0;
    let box_y = row.y + (row.height - CHECKBOX_SIZE) / 2.0;
    let outline = Rectangle {
        x: box_x,
        y: box_y,
        width: CHECKBOX_SIZE,
        height: CHECKBOX_SIZE,
    };
    frame.fill(&rounded_rect(outline, 2.0), DROPDOWN_COLOR);
    if node.switch(index) {
        frame.fill(
            &rounded_rect(
                Rectangle {
                    x: box_x + 3.0,
                    y: box_y + 3.0,
                    width: CHECKBOX_SIZE - 6.0,
                    height: CHECKBOX_SIZE - 6.0,
                },
                1.0,
            ),
            CHECKBOX_CHECKED_COLOR,
        );
    }
    draw_text(
        frame,
        covered,
        zoom,
        Text {
            content: label.to_string(),
            position: Point::new(box_x + CHECKBOX_SIZE + 8.0, row.y + row.height / 2.0),
            color: text_color,
            size: 13.0,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        },
    );
}

/// The closed dropdown: the current label and a chevron.
fn draw_dropdown(
    frame: &mut Frame,
    bounds: Rectangle,
    name: &str,
    label: &str,
    hovered: bool,
    zoom: f32,
    covered: &[Rectangle],
) {
    let color = if hovered {
        DROPDOWN_HOVER_COLOR
    } else {
        DROPDOWN_COLOR
    };
    frame.fill(&rounded_rect(bounds, DROPDOWN_CORNER_RADIUS), color);
    // The labels go through `draw_text`, so an open list over this node hides them. The name sits
    // dimmed on the left, and the chosen value sits right of centre, before the chevron.
    draw_text(
        frame,
        covered,
        zoom,
        Text {
            content: name.to_string(),
            position: Point::new(bounds.x + 8.0, bounds.y + bounds.height / 2.0),
            color: DIM_TEXT_COLOR,
            size: 11.0,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        },
    );
    draw_text(
        frame,
        covered,
        zoom,
        Text {
            content: label.to_string(),
            position: Point::new(
                bounds.x + bounds.width - 22.0,
                bounds.y + bounds.height / 2.0,
            ),
            color: TEXT_COLOR,
            size: 13.0,
            horizontal_alignment: alignment::Horizontal::Right,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        },
    );

    let chevron_x = bounds.x + bounds.width - 12.0;
    let chevron_y = bounds.y + bounds.height / 2.0;
    let chevron = Path::new(|builder| {
        builder.move_to(Point::new(chevron_x - 4.0, chevron_y - 2.0));
        builder.line_to(Point::new(chevron_x, chevron_y + 2.0));
        builder.line_to(Point::new(chevron_x + 4.0, chevron_y - 2.0));
    });
    frame.stroke(
        &chevron,
        Stroke::default()
            .with_color(TEXT_COLOR)
            .with_width(1.5)
            .with_line_cap(LineCap::Round),
    );
}

/// The open dropdown list. The selected row is highlighted.
fn draw_list(
    frame: &mut Frame,
    list: Rectangle,
    labels: &[&str],
    selected: usize,
    hovered_row: Option<usize>,
    zoom: f32,
) {
    frame.fill(&rounded_rect(list, MENU_CORNER_RADIUS), MENU_COLOR);

    for (row, label) in labels.iter().enumerate() {
        let row_y = list.y + row as f32 * WAVEFORM_ITEM_HEIGHT;
        if hovered_row == Some(row) {
            frame.fill(
                &rounded_rect(
                    Rectangle {
                        x: list.x + 4.0,
                        y: row_y + 2.0,
                        width: list.width - 8.0,
                        height: WAVEFORM_ITEM_HEIGHT - 4.0,
                    },
                    4.0,
                ),
                LIST_ROW_HOVER_COLOR,
            );
        }
        if row == selected {
            frame.fill(
                &rounded_rect(
                    Rectangle {
                        x: list.x + 4.0,
                        y: row_y + 2.0,
                        width: list.width - 8.0,
                        height: WAVEFORM_ITEM_HEIGHT - 4.0,
                    },
                    4.0,
                ),
                NODE_HEADER_COLOR,
            );
        }
        frame.fill_text(Text {
            content: label.to_string(),
            position: Point::new(list.x + 12.0, row_y + WAVEFORM_ITEM_HEIGHT / 2.0),
            color: TEXT_COLOR,
            size: 13.0 * zoom,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
    }
}

/// A white outline just outside the selected node.
fn draw_selection(frame: &mut Frame, node: &FlodeNode) {
    let bounds = node.bounds();
    let outline = Rectangle {
        x: bounds.x - SELECTION_WIDTH,
        y: bounds.y - SELECTION_WIDTH,
        width: bounds.width + 2.0 * SELECTION_WIDTH,
        height: bounds.height + 2.0 * SELECTION_WIDTH,
    };
    frame.stroke(
        &rounded_rect(outline, CORNER_RADIUS + SELECTION_WIDTH),
        Stroke::default()
            .with_color(SELECTION_COLOR)
            .with_width(SELECTION_WIDTH),
    );
}

/// The pill of add buttons at the top of the editor.
fn draw_pill(frame: &mut Frame, bounds: Rectangle) {
    let pill = pill_bounds(bounds);
    frame.fill(&rounded_rect(pill, PILL_HEIGHT / 2.0), PILL_COLOR);

    let segment = pill.width / PILL_ITEMS.len() as f32;
    for divider in 1..PILL_ITEMS.len() {
        let x = pill.x + divider as f32 * segment;
        frame.stroke(
            &Path::line(
                Point::new(x, pill.y + 8.0),
                Point::new(x, pill.y + pill.height - 8.0),
            ),
            Stroke::default()
                .with_color(PILL_DIVIDER_COLOR)
                .with_width(1.0),
        );
    }

    for (index, item) in PILL_ITEMS.iter().enumerate() {
        let center_x = pill.x + (index as f32 + 0.5) * segment;
        let center_y = pill.y + pill.height / 2.0;
        let label = match item {
            PillItem::Kind(kind) => kind.title(),
            PillItem::Category(category) => category.label(),
        };
        frame.fill_text(Text {
            content: label.to_string(),
            position: Point::new(center_x, center_y),
            color: TEXT_COLOR,
            size: 14.0,
            horizontal_alignment: alignment::Horizontal::Center,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
        // A category opens a list, so its button has a chevron.
        if let PillItem::Category(_) = item {
            let chevron_x = center_x + 36.0;
            let chevron = Path::new(|builder| {
                builder.move_to(Point::new(chevron_x - 3.0, center_y - 2.0));
                builder.line_to(Point::new(chevron_x, center_y + 2.0));
                builder.line_to(Point::new(chevron_x + 3.0, center_y - 2.0));
            });
            frame.stroke(
                &chevron,
                Stroke::default()
                    .with_color(TEXT_COLOR)
                    .with_width(1.5)
                    .with_line_cap(LineCap::Round),
            );
        }
    }
}

/// The list under an open category. Its rows are the category's node kinds, or a note when the
/// category has none yet.
fn draw_category_list(frame: &mut Frame, bounds: Rectangle, category: Category) {
    let list = category_list_bounds(bounds, category);
    frame.fill(&rounded_rect(list, MENU_CORNER_RADIUS), MENU_COLOR);

    let labels: Vec<&str> = if category.kinds().is_empty() {
        vec!["Empty for now"]
    } else {
        category.kinds().iter().map(|kind| kind.title()).collect()
    };
    for (row, label) in labels.iter().enumerate() {
        frame.fill_text(Text {
            content: label.to_string(),
            position: Point::new(
                list.x + 12.0,
                list.y + row as f32 * MENU_ITEM_HEIGHT + MENU_ITEM_HEIGHT / 2.0,
            ),
            color: if category.kinds().is_empty() {
                DIM_TEXT_COLOR
            } else {
                TEXT_COLOR
            },
            size: 13.0,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
    }
}

/// The right-click menu. It has one item for now.
fn draw_menu(frame: &mut Frame, menu: ContextMenu, zoom: f32) {
    let bounds = menu_bounds(menu);
    frame.fill(&rounded_rect(bounds, MENU_CORNER_RADIUS), MENU_COLOR);
    frame.fill_text(Text {
        content: "Delete node".to_string(),
        position: Point::new(bounds.x + 12.0, bounds.y + MENU_ITEM_HEIGHT / 2.0),
        color: TEXT_COLOR,
        size: 14.0 * zoom,
        vertical_alignment: alignment::Vertical::Center,
        ..Text::default()
    });
}

/// A rectangle with rounded corners of radius `r`. Each corner is a quadratic curve whose control
/// point is the sharp corner. Iced's arc builders produced skewed corners here.
fn rounded_rect(bounds: Rectangle, r: f32) -> Path {
    let x0 = bounds.x;
    let y0 = bounds.y;
    let x1 = bounds.x + bounds.width;
    let y1 = bounds.y + bounds.height;
    Path::new(|builder| {
        builder.move_to(Point::new(x0 + r, y0));
        builder.line_to(Point::new(x1 - r, y0));
        builder.quadratic_curve_to(Point::new(x1, y0), Point::new(x1, y0 + r));
        builder.line_to(Point::new(x1, y1 - r));
        builder.quadratic_curve_to(Point::new(x1, y1), Point::new(x1 - r, y1));
        builder.line_to(Point::new(x0 + r, y1));
        builder.quadratic_curve_to(Point::new(x0, y1), Point::new(x0, y1 - r));
        builder.line_to(Point::new(x0, y0 + r));
        builder.quadratic_curve_to(Point::new(x0, y0), Point::new(x0 + r, y0));
        builder.close();
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn knob_at(id: KnobId, plain: f32) -> f32 {
        id.to_normalized(plain)
    }

    /// Envelope into a map that ranges 0.2 to 0.6, into a second map that ranges 0.5 to 1.0,
    /// into the gain. The shape 0 reaches 0.5 + 0.5 * 0.2 and the shape 1 reaches 0.5 + 0.5 * 0.6.
    #[test]
    fn chained_maps_compose_into_one_range() {
        let mut graph = Graph::default();
        let env = graph.add_node(NodeKind::Envelope, Point::new(0.0, 0.0));
        let first = graph.add_node(NodeKind::Map, Point::new(0.0, 0.0));
        let second = graph.add_node(NodeKind::Map, Point::new(0.0, 0.0));
        graph.nodes[first].set_value(KnobId::MapMin, knob_at(KnobId::MapMin, 0.2));
        graph.nodes[first].set_value(KnobId::MapMax, knob_at(KnobId::MapMax, 0.6));
        graph.nodes[second].set_value(KnobId::MapMin, knob_at(KnobId::MapMin, 0.5));
        graph.nodes[second].set_value(KnobId::MapMax, knob_at(KnobId::MapMax, 1.0));
        graph.connect(env, Port::Shape, first, Port::Shape);
        graph.connect(first, Port::Shape, second, Port::Shape);
        graph.connect(second, Port::Shape, 1, Port::Knob(KnobId::Gain));

        // The source's level passes through the first map, then the second.
        let route = graph.control_route(1, KnobId::Gain).unwrap();
        let at = |envelope: f32| {
            route.apply(&ControlLevels {
                envelope: [envelope; MAX_ENVELOPES],
                ..ControlLevels::default()
            })
        };
        assert!((at(0.0) - 0.6).abs() < 1e-6);
        assert!((at(1.0) - 0.8).abs() < 1e-6);
        assert!(graph.is_driven(1, KnobId::Gain));
        assert!(graph.any_driven());
    }

    /// A map with nothing upstream does not drive its knob.
    #[test]
    fn map_without_envelope_does_not_drive() {
        let mut graph = Graph::default();
        let map = graph.add_node(NodeKind::Map, Point::new(0.0, 0.0));
        graph.connect(map, Port::Shape, 1, Port::Knob(KnobId::Gain));
        assert_eq!(graph.control_route(1, KnobId::Gain), None);
        assert!(!graph.is_driven(1, KnobId::Gain));
    }

    /// Two filters keep separate knobs. A wire into one's cutoff drives only that filter.
    #[test]
    fn a_control_wire_drives_only_its_own_node() {
        let mut graph = Graph::default();
        let env = graph.add_node(NodeKind::Envelope, Point::new(0.0, 0.0));
        let first = graph.add_node(NodeKind::Filter, Point::new(0.0, 0.0));
        let second = graph.add_node(NodeKind::Filter, Point::new(0.0, 0.0));
        graph.connect(env, Port::Shape, first, Port::Knob(KnobId::Cutoff));
        assert!(graph.is_driven(first, KnobId::Cutoff));
        assert!(!graph.is_driven(second, KnobId::Cutoff));
    }
}

#[cfg(test)]
mod slot_tests {
    use super::*;

    /// The default graph has one slot in use, and one empty pin below it.
    #[test]
    fn default_output_has_one_slot_and_an_empty_pin() {
        let graph = Graph::default();
        let output = &graph.nodes[1];
        assert_eq!(output.used, 1);
        assert_eq!(output.slots, 2);
        assert!(graph.lane_chains()[0].active);
        assert!(!graph.lane_chains()[1].active);
    }

    /// Wiring the empty pin adds a slot and a new empty pin below it.
    #[test]
    fn wiring_the_empty_pin_adds_a_slot() {
        let mut graph = Graph::default();
        let second = graph.add_node(NodeKind::Oscillator, Point::new(0.0, 200.0));
        graph.connect(second, Port::Audio, 1, Port::Slot(1));
        let output = &graph.nodes[1];
        assert_eq!(output.used, 2);
        assert_eq!(output.slots, 3);
        assert!(graph.lane_chains()[1].active);
        assert!(graph.nodes[1]
            .rows()
            .iter()
            .any(|row| row.knob == Some(KnobId::Slot2)));
    }

    /// Two oscillators wired to two slots keep their own knobs. Turning one does not move the other.
    #[test]
    fn two_oscillators_keep_separate_knobs() {
        let mut graph = Graph::default();
        let second = graph.add_node(NodeKind::Oscillator, Point::new(0.0, 200.0));
        graph.connect(second, Port::Audio, 1, Port::Slot(1));

        graph.nodes[0].set_value(KnobId::Detune, KnobId::Detune.to_normalized(60.0));
        let chains = graph.lane_chains();
        let first_detune = chains[0]
            .knob(KnobId::Detune)
            .plain(KnobId::Detune, &ControlLevels::default());
        let second_detune = chains[1]
            .knob(KnobId::Detune)
            .plain(KnobId::Detune, &ControlLevels::default());
        assert!((first_detune - 60.0).abs() < 0.01);
        assert!((second_detune - KnobId::Detune.default_plain()).abs() < 0.01);
    }

    /// Saves from before slots wired the output's single audio pin, which loads into slot 0.
    #[test]
    fn old_output_links_load_into_slot_zero() {
        let node = |kind| SavedNode {
            kind,
            x: 0.0,
            y: 0.0,
            collapsed: false,
            choices: Vec::new(),
            choice: 0,
            switches: Vec::new(),
            switched: false,
            knobs: Vec::new(),
            waveform: None,
            filter_type: None,
            out_min: None,
            out_max: None,
        };
        let saved = SavedGraph {
            nodes: vec![node(NodeKind::Oscillator), node(NodeKind::Output)],
            connections: vec![(0, 1)],
            links: Vec::new(),
        };
        let graph = Graph::from_saved(&saved);
        assert_eq!(graph.connections.len(), 1);
        assert_eq!(graph.connections[0].input, Port::Slot(0));
        assert!(graph.lane_chains()[0].active);
    }

    /// Saved knob values and the dropdown come back, and old fields still load.
    #[test]
    fn saving_and_loading_keeps_node_state() {
        let mut graph = Graph::default();
        graph.nodes[0].set_value(KnobId::Unison, KnobId::Unison.to_normalized(5.0));
        graph.nodes[0].choose(0, 2);
        graph.nodes[1].switches = vec![true, true];
        let loaded = Graph::from_saved(&graph.to_saved());
        assert_eq!(loaded.nodes[0].plain(KnobId::Unison), 5.0);
        assert_eq!(oscillator::waveform(&loaded.nodes[0]), Waveform::Square);
        assert_eq!(loaded.nodes[1].switches, vec![true, true]);

        let legacy = SavedNode {
            kind: NodeKind::Oscillator,
            x: 0.0,
            y: 0.0,
            collapsed: false,
            choices: Vec::new(),
            choice: 0,
            switches: Vec::new(),
            switched: false,
            knobs: Vec::new(),
            waveform: Some(Waveform::Triangle),
            filter_type: None,
            out_min: None,
            out_max: None,
        };
        assert_eq!(
            oscillator::waveform(&legacy.into_node()),
            Waveform::Triangle
        );
    }

    /// A node dropped on the wire into an output slot splices into that wire.
    #[test]
    fn a_node_dropped_on_a_slot_wire_is_spliced_in() {
        let mut graph = Graph::default();
        let curve = graph.link_curve(&graph.connections[0]).unwrap();
        let middle = bezier_point(&curve, 0.5);
        let filter = graph.add_node(
            NodeKind::Filter,
            Point::new(middle.x - 80.0, middle.y - 20.0),
        );

        assert_eq!(graph.link_under(filter), Some(0));
        graph.insert_into(filter, 0);
        graph.refresh_slots();

        assert_eq!(graph.connections.len(), 2);
        assert_eq!(graph.connections[0].to, filter);
        assert_eq!(graph.connections[1].from, filter);
        assert_eq!(graph.connections[1].input, Port::Slot(0));
        assert!(graph.lane_chains()[0].filter.is_some());
    }
}

#[cfg(test)]
mod lfo_and_category_tests {
    use super::*;

    /// An LFO wired into the output's gain drives that knob from LFO 0.
    #[test]
    fn an_lfo_drives_a_knob_as_its_own_source() {
        let mut graph = Graph::default();
        let lfo = graph.add_node(NodeKind::Lfo, Point::new(0.0, 0.0));
        graph.connect(lfo, Port::Shape, 1, Port::Knob(KnobId::Gain));

        let route = graph.control_route(1, KnobId::Gain).unwrap();
        assert_eq!(route.source, Source::Lfo(0));
        let at = |lfo: f32| {
            route.apply(&ControlLevels {
                lfo: [lfo, 0.0],
                ..ControlLevels::default()
            })
        };
        assert!((at(0.25) - 0.25).abs() < 1e-6);

        let settings = graph.lfo_settings();
        assert!(settings[0].active);
        assert!(!settings[1].active);
        assert!((settings[0].rate.value - KnobId::LfoRate.default_normalized()).abs() < 1e-6);
    }

    /// The first LFO is the one the audio thread runs. Later LFOs make no signal.
    #[test]
    fn only_the_first_lfos_are_run() {
        let mut graph = Graph::default();
        let mut lfos = Vec::new();
        for x in 0..3 {
            let lfo = graph.add_node(NodeKind::Lfo, Point::new(x as f32 * 10.0, 200.0));
            graph.connect(lfo, Port::Shape, 1, Port::Knob(KnobId::Gain));
            lfos.push(lfo);
        }
        // The third LFO is past the two the audio thread runs.
        assert_eq!(graph.lfo_index(lfos[2]), None);
        assert_eq!(graph.lfo_index(lfos[1]), Some(1));
        assert!(graph.lfo_settings().iter().all(|s| s.active));
    }

    /// The pill's buttons open the Driver, Util, and Effects lists, and each list holds its kinds.
    #[test]
    fn categories_list_their_kinds() {
        let bounds = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        };
        assert_eq!(
            pill_button_at(bounds, Point::new(450.0, 30.0)),
            Some(PillItem::Category(Category::Driver))
        );
        assert_eq!(
            category_row_at(bounds, Category::Driver, Point::new(400.0, 98.0)),
            Some(NodeKind::Lfo)
        );
        assert!(Category::Effects.kinds().contains(&NodeKind::Filter));
        assert!(!Category::Driver.kinds().contains(&NodeKind::Filter));
        assert!(Category::Util.kinds().contains(&NodeKind::Output));
        assert!(Category::Util.kinds().contains(&NodeKind::Map));
    }

    /// Every kind is in exactly one category, except the oscillator, which has its own button.
    #[test]
    fn every_kind_is_categorised_or_on_the_pill() {
        for kind in NodeKind::ALL {
            let in_category = [Category::Driver, Category::Util, Category::Effects]
                .iter()
                .any(|c| c.kinds().contains(&kind));
            let on_pill = PILL_ITEMS.contains(&PillItem::Kind(kind));
            assert!(in_category ^ on_pill, "{kind:?}");
        }
    }

    /// An envelope can drive an LFO's rate, and the LFO reports that route.
    #[test]
    fn an_envelope_can_drive_an_lfo_rate() {
        let mut graph = Graph::default();
        let env = graph.add_node(NodeKind::Envelope, Point::new(0.0, 0.0));
        let lfo = graph.add_node(NodeKind::Lfo, Point::new(0.0, 0.0));
        graph.connect(env, Port::Shape, lfo, Port::Knob(KnobId::LfoRate));
        let route = graph.control_route(lfo, KnobId::LfoRate).unwrap();
        assert_eq!(route.source, Source::Envelope(0));
        assert!(graph.is_driven(lfo, KnobId::LfoRate));
    }
}

#[cfg(test)]
mod input_node_tests {
    use super::*;
    use crate::nodes::input::MidiSource;

    /// The mod wheel pin of an input node drives a knob as its own source.
    #[test]
    fn a_midi_pin_drives_a_knob() {
        let mut graph = Graph::default();
        let input = graph.add_node(NodeKind::Input, Point::new(0.0, 0.0));
        graph.connect(
            input,
            Port::Midi(MidiSource::ModWheel),
            1,
            Port::Knob(KnobId::Gain),
        );
        let route = graph.control_route(1, KnobId::Gain).unwrap();
        assert_eq!(route.source, Source::Midi(MidiSource::ModWheel));
        assert!(graph.is_driven(1, KnobId::Gain));
    }

    /// A MIDI source goes through a map like any other control signal.
    #[test]
    fn a_midi_pin_can_feed_a_map() {
        let mut graph = Graph::default();
        let input = graph.add_node(NodeKind::Input, Point::new(0.0, 0.0));
        let map = graph.add_node(NodeKind::Map, Point::new(0.0, 0.0));
        graph.connect(input, Port::Midi(MidiSource::Velocity), map, Port::Shape);
        graph.connect(map, Port::Shape, 1, Port::Knob(KnobId::Gain));
        let route = graph.control_route(1, KnobId::Gain).unwrap();
        assert_eq!(route.source, Source::Midi(MidiSource::Velocity));
    }

    /// Every source survives a trip through the saved byte, so routes keep their meaning.
    #[test]
    fn every_source_round_trips() {
        for source in [
            Source::Envelope(0),
            Source::Lfo(0),
            Source::Lfo(1),
            Source::Midi(MidiSource::Note),
            Source::Midi(MidiSource::Aftertouch),
        ] {
            assert_eq!(Source::from_u8(source.to_u8()), source);
        }
    }

    /// A map dropped on a wire that drives a knob splices into it. The knob keeps its route, now
    /// through the map.
    #[test]
    fn a_map_dropped_on_a_control_wire_is_spliced_in() {
        let mut graph = Graph::default();
        let env = graph.add_node(NodeKind::Envelope, Point::new(0.0, 0.0));
        graph.connect(env, Port::Shape, 1, Port::Knob(KnobId::Gain));
        let link = graph.connections.len() - 1;
        let curve = graph.link_curve(&graph.connections[link]).unwrap();
        let middle = bezier_point(&curve, 0.5);
        let map = graph.add_node(NodeKind::Map, Point::new(middle.x - 80.0, middle.y - 20.0));

        assert_eq!(graph.link_under(map), Some(link));
        graph.insert_into(map, link);
        assert_eq!(graph.connections[link].to, map);
        assert_eq!(graph.connections[link].input, Port::Shape);
        let route = graph.control_route(1, KnobId::Gain).unwrap();
        assert_eq!(route.source, Source::Envelope(0));
    }
}

#[cfg(test)]
mod map_curve_tests {
    use super::*;

    /// Inputs outside the input range hold at the ends, so the output is flat there.
    #[test]
    fn a_map_clamps_outside_its_input_range() {
        let stage = MapStage {
            in_lo: 0.25,
            in_hi: 0.75,
            out_lo: 0.0,
            out_hi: 1.0,
        };
        assert_eq!(stage.apply(0.1), 0.0);
        assert!((stage.apply(0.5) - 0.5).abs() < 1e-6);
        assert_eq!(stage.apply(0.9), 1.0);
    }

    /// A handle's position on the curve follows its input and output values.
    #[test]
    fn handles_sit_on_the_curve() {
        let mut node = FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0));
        node.set_value(KnobId::MapInMin, 0.0);
        node.set_value(KnobId::MapMin, 0.0);
        node.set_value(KnobId::MapInMax, 1.0);
        node.set_value(KnobId::MapMax, 1.0);
        let bounds = node.curve_bounds();
        let handles = map_handles(&node);
        assert!((handles[0].x - bounds.x).abs() < 1e-3);
        assert!((handles[1].y - bounds.y).abs() < 1e-3);
    }
}

#[cfg(test)]
mod envelope_migration_tests {
    use super::*;

    /// An old save with an envelope in the sound chain loads with the envelope driving the
    /// oscillator's gain, and the audio running straight to the output.
    #[test]
    fn an_envelope_in_the_chain_becomes_a_gain_driver() {
        let node = |kind, x| SavedNode {
            kind,
            x,
            y: 0.0,
            collapsed: false,
            choices: Vec::new(),
            choice: 0,
            switches: Vec::new(),
            switched: false,
            knobs: Vec::new(),
            waveform: None,
            filter_type: None,
            out_min: None,
            out_max: None,
        };
        let saved = SavedGraph {
            nodes: vec![
                node(NodeKind::Oscillator, 0.0),
                node(NodeKind::Envelope, 100.0),
                node(NodeKind::Output, 200.0),
            ],
            connections: Vec::new(),
            links: vec![
                Connection {
                    from: 0,
                    out: Port::Audio,
                    to: 1,
                    input: Port::Audio,
                },
                Connection {
                    from: 1,
                    out: Port::Audio,
                    to: 2,
                    input: Port::Slot(0),
                },
            ],
        };
        let graph = Graph::from_saved(&saved);
        // The oscillator now feeds the output directly.
        assert!(graph
            .connections
            .iter()
            .any(|c| c.from == 0 && c.out == Port::Audio && c.to == 2 && c.input == Port::Slot(0)));
        // The envelope drives the oscillator's amplitude.
        assert_eq!(graph.lane_chains()[0].envelope, Some(0));
        assert!(graph.lane_chains()[0].active);
    }
}

#[cfg(test)]
mod mix_tests {
    use super::*;

    /// A mix node between two oscillators and the output. One branch has a wire, the other
    /// doesn't, to check each resolves on its own.
    fn chain() -> (Graph, usize, usize, usize) {
        let mut graph = Graph::default();
        graph.connections.clear();
        let a = graph.add_node(NodeKind::Oscillator, Point::new(0.0, 0.0));
        let b = graph.add_node(NodeKind::Oscillator, Point::new(0.0, 0.0));
        let mix = graph.add_node(NodeKind::Mix, Point::new(0.0, 0.0));
        graph.connect(a, Port::Audio, mix, Port::AudioIn(0));
        graph.connect(b, Port::Audio, mix, Port::AudioIn(1));
        graph.connect(mix, Port::Audio, 1, Port::Slot(0));
        graph.refresh_slots();
        (graph, a, b, mix)
    }

    #[test]
    fn a_mix_node_combines_its_two_branches() {
        let (graph, _, _, _) = chain();
        let lane = graph.lane_chains()[0];
        let mix = lane.mix.expect("mix chain");
        assert!(mix.a.active);
        assert!(mix.b.active);
        assert!(lane.active);
    }

    /// An unwired input contributes silence instead of breaking the chain.
    #[test]
    fn an_unwired_branch_is_silent() {
        let mut graph = Graph::default();
        graph.connections.clear();
        let a = graph.add_node(NodeKind::Oscillator, Point::new(0.0, 0.0));
        let mix = graph.add_node(NodeKind::Mix, Point::new(0.0, 0.0));
        graph.connect(a, Port::Audio, mix, Port::AudioIn(0));
        graph.connect(mix, Port::Audio, 1, Port::Slot(0));
        graph.refresh_slots();

        let lane = graph.lane_chains()[0];
        let mix = lane.mix.expect("mix chain");
        assert!(mix.a.active);
        assert!(!mix.b.active);
    }

    /// Each branch keeps its own oscillator's waveform and knobs.
    #[test]
    fn branches_keep_their_own_oscillator_settings() {
        let (mut graph, a, b, _) = chain();
        graph.nodes[a].choices = vec![Waveform::Square.index()];
        graph.nodes[b].choices = vec![Waveform::Triangle.index()];
        graph.nodes[a].set_value(KnobId::Detune, KnobId::Detune.to_normalized(40.0));

        let lane = graph.lane_chains()[0];
        let mix = lane.mix.unwrap();
        assert_eq!(mix.a.waveform, Waveform::Square);
        assert_eq!(mix.b.waveform, Waveform::Triangle);
        assert_eq!(
            mix.a.knobs[KnobId::Detune as usize].plain(KnobId::Detune, &ControlLevels::default()),
            40.0
        );
    }

    /// The mix node's own dropdown and knob reach the lane.
    #[test]
    fn the_mix_type_and_amount_reach_the_lane() {
        let (mut graph, _, _, mix_node) = chain();
        graph.nodes[mix_node].choices = vec![2];
        graph.nodes[mix_node].set_value(KnobId::Mix, KnobId::Mix.to_normalized(0.75));

        let lane = graph.lane_chains()[0];
        let mix = lane.mix.unwrap();
        assert_eq!(mix.mode, crate::nodes::mix::MixMode::RingMod);
        assert_eq!(mix.mix.plain(KnobId::Mix, &ControlLevels::default()), 0.75);
    }

    /// A chain without a mix node behaves as before.
    #[test]
    fn a_plain_oscillator_chain_has_no_mix() {
        let lane = Graph::default().lane_chains()[0];
        assert!(lane.mix.is_none());
    }
}
