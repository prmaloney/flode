//! The node types. Every node in the graph is a `FlodeNode`. What differs between kinds is its
//! `NodeBehavior`: the rows it shows, the extra control under them, and its name. Each kind lives
//! in its own module and holds nothing but that behavior and the typed reads the audio engine
//! needs. A node owns its own knob values, so two nodes of one kind never share a value.
//!
//! A node is a header over a stack of rows. A row can have an input pin on the left edge, a knob
//! beside it, and an output pin on the right edge. Under the rows a node can have one extra
//! control, which is a dropdown or a switch.

pub mod envelope;
pub mod filter;
pub mod input;
pub mod knobs;
pub mod lfo;
pub mod map;
pub mod mix;
pub mod oscillator;
pub mod output;

use crate::editor::knob;
use nih_plug_iced::{Point, Rectangle};
use serde::{Deserialize, Serialize};

pub use knobs::KnobId;

pub const NODE_WIDTH: f32 = 160.0;
pub const HEADER_HEIGHT: f32 = 28.0;
/// The collapse arrow sits in the header, and the title starts to its right.
pub const ARROW_CENTER_X: f32 = 20.0;
pub const ARROW_HIT_RADIUS: f32 = 8.0;
pub const TITLE_INSET: f32 = 36.0;

/// Pins run down the node edges, one row each. A knob sits on the same row as its pin.
pub const PIN_ROW0_Y: f32 = HEADER_HEIGHT + 22.0;
pub const ROW_PITCH: f32 = 44.0;
/// Knobs sit right of the labels, at the same height as their pin.
pub const ROW_KNOB_DIAMETER: f32 = 32.0;
pub const ROW_KNOB_CENTER_X: f32 = 84.0;

pub const DROPDOWN_HEIGHT: f32 = 24.0;
pub const DROPDOWN_INSET: f32 = 12.0;
/// The switch at the bottom of the output node.
pub const SWITCH_ROW_HEIGHT: f32 = 22.0;
/// The transfer curve of a map node: a box about as wide as the node's body.
pub const CURVE_HEIGHT: f32 = 96.0;

/// Vertical offset of a pin row, measured from the top of the node.
pub fn pin_y(row: usize) -> f32 {
    PIN_ROW0_Y + row as f32 * ROW_PITCH
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    Oscillator,
    Envelope,
    Filter,
    Lfo,
    Input,
    Map,
    Mix,
    Output,
}

/// The groups the add menu sorts the node kinds into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// Nodes that make control signals.
    Driver,
    /// Nodes that route or shape signals without a sound of their own.
    Util,
    /// Nodes that change the sound of a signal. Filter so far.
    Effects,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Driver => "Driver",
            Category::Util => "Util",
            Category::Effects => "Effects",
        }
    }

    /// The kinds in this category, in the order the menu lists them.
    pub fn kinds(self) -> &'static [NodeKind] {
        match self {
            Category::Driver => &[NodeKind::Envelope, NodeKind::Lfo],
            Category::Util => &[
                NodeKind::Output,
                NodeKind::Map,
                NodeKind::Input,
                NodeKind::Mix,
            ],
            Category::Effects => &[NodeKind::Filter],
        }
    }
}

impl NodeKind {
    /// Every kind. The tests walk them.
    #[cfg(test)]
    pub const ALL: [NodeKind; 8] = [
        NodeKind::Oscillator,
        NodeKind::Envelope,
        NodeKind::Filter,
        NodeKind::Lfo,
        NodeKind::Input,
        NodeKind::Map,
        NodeKind::Mix,
        NodeKind::Output,
    ];

    /// What sets this kind apart from the others.
    pub fn behavior(self) -> &'static dyn NodeBehavior {
        match self {
            NodeKind::Oscillator => &oscillator::Oscillator,
            NodeKind::Envelope => &envelope::Envelope,
            NodeKind::Filter => &filter::Filter,
            NodeKind::Lfo => &lfo::Lfo,
            NodeKind::Input => &input::Input,
            NodeKind::Map => &map::Map,
            NodeKind::Mix => &mix::Mix,
            NodeKind::Output => &output::Output,
        }
    }

    pub fn title(self) -> &'static str {
        self.behavior().title()
    }
}

/// A socket on a node. Audio ports carry sound. The shape port carries a control signal. A knob
/// port takes a control signal in place of the knob's own value. A slot port is an audio input
/// on an output node, and each slot plays its own chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Port {
    Audio,
    /// One of several audio inputs on a node, such as the two signals a mix node combines.
    AudioIn(usize),
    Shape,
    Knob(KnobId),
    Slot(usize),
    /// A MIDI source on an input node, sent as a control signal.
    Midi(input::MidiSource),
}

impl Port {
    /// True for an audio input, which takes sound.
    pub fn is_audio(self) -> bool {
        matches!(self, Port::Audio | Port::AudioIn(_) | Port::Slot(_))
    }

    /// True for a control output: a shape pin, or a MIDI source.
    pub fn is_control_out(self) -> bool {
        matches!(self, Port::Shape | Port::Midi(_))
    }

    /// The name drawn beside an output pin, for the ones that need one.
    pub fn output_label(self) -> Option<&'static str> {
        match self {
            Port::Shape => Some("Shape"),
            Port::Midi(source) => Some(source.label()),
            _ => None,
        }
    }

    /// The name drawn beside an input pin that has no knob of its own to name it.
    pub fn input_label(self) -> Option<&'static str> {
        match self {
            Port::Shape => Some("Shape"),
            Port::AudioIn(0) => Some("A"),
            Port::AudioIn(1) => Some("B"),
            _ => None,
        }
    }
}

/// One row of a node: an input pin, a knob, and an output pin, any of which can be missing.
#[derive(Debug, Clone, Copy, Default)]
pub struct Row {
    /// The input pin on the left edge.
    pub input: Option<Port>,
    /// The knob on this row. Its name and value sit beside the pin, and the knob sits right of them.
    pub knob: Option<KnobId>,
    /// The output pin on the right edge.
    pub output: Option<Port>,
    /// A control on this row, in place of pins and a knob: a dropdown, a switch, or a curve.
    pub control: Option<Control>,
}

/// A control that takes a whole row, in the order the node lists its rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// Dropdown number `n` of the node.
    Dropdown(usize),
    /// Switch number `n` of the node.
    Switch(usize),
    /// The transfer curve of a map.
    Curve,
}

impl Row {
    /// A knob with its own control pin. A control wire can drive it.
    pub fn driven_knob(id: KnobId) -> Self {
        Self {
            input: Some(Port::Knob(id)),
            knob: Some(id),
            output: None,
            control: None,
        }
    }

    /// A knob with no pin, which a control wire can't drive.
    pub fn plain_knob(id: KnobId) -> Self {
        Self {
            input: None,
            knob: Some(id),
            output: None,
            control: None,
        }
    }

    pub fn input(port: Port) -> Self {
        Self {
            input: Some(port),
            knob: None,
            output: None,
            control: None,
        }
    }

    /// A row that holds a control and nothing else.
    pub fn control(control: Control) -> Self {
        Self {
            input: None,
            knob: None,
            output: None,
            control: Some(control),
        }
    }

    pub fn with_output(mut self, port: Port) -> Self {
        self.output = Some(port);
        self
    }
}

/// One dropdown on a node: its place in the node, the labels it offers, and the one that is set.
pub struct Dropdown {
    /// Which of the node's dropdowns this is, from 0.
    pub which: usize,
    /// The name drawn on the dropdown, such as "Wave" or "Warp".
    pub name: &'static str,
    pub labels: &'static [&'static str],
    pub selected: usize,
    /// Where its button sits on the node, in editor coordinates.
    pub bounds: Rectangle,
}

/// What sets one kind of node apart. Everything else about a node is shared in `FlodeNode`.
pub trait NodeBehavior: Sync {
    fn title(&self) -> &'static str;

    /// The rows from top to bottom. Some kinds depend on the node's own state, such as the output
    /// node's slot count.
    fn rows(&self, node: &FlodeNode) -> Vec<Row>;

    /// True if sound enters this kind of node.
    fn audio_in(&self) -> bool {
        false
    }

    /// True if sound leaves this kind of node.
    fn audio_out(&self) -> bool {
        false
    }

    /// The labels of the node's switches, top to bottom.
    fn switch_labels(&self) -> &'static [&'static str] {
        &[]
    }

    /// The name of each dropdown, indexed the same way as `dropdowns`.
    fn dropdown_names(&self) -> &'static [&'static str] {
        &[]
    }

    /// The node's dropdowns, from top to bottom. A kind can change which ones it shows with its
    /// own state, such as the oscillator's noise type under the noise waveform.
    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        Vec::new()
    }
}

/// Where a knob is drawn on its node.
#[derive(Debug, Clone, Copy)]
pub struct KnobSlot {
    pub id: KnobId,
    pub center: Point,
    /// True if the knob has a control pin, so a wire can drive it.
    pub pinned: bool,
}

/// A node in the graph. Every kind of node is this type.
pub struct FlodeNode {
    pub kind: NodeKind,
    /// Top-left corner in editor coordinates.
    pub position: Point,
    /// A collapsed node shows only its header. Its pins sit on the header's edges.
    pub collapsed: bool,
    /// Normalized knob values, indexed by `KnobId as usize`. A node reads the ones its rows show.
    values: [f32; KnobId::COUNT],
    /// The row chosen in each dropdown, indexed by dropdown.
    pub choices: Vec<usize>,
    /// The switches, for kinds that have them. Each one is off until it is clicked.
    pub switches: Vec<bool>,
    /// Output nodes only. The slot pins shown: the connected ones and one empty pin below them.
    pub slots: usize,
    /// Output nodes only. The number of slots with a chain wired in.
    pub used: usize,
    /// Drag state for each knob.
    pub knob_states: [knob::State; KnobId::COUNT],
}

impl FlodeNode {
    pub fn new(kind: NodeKind, position: Point) -> Self {
        Self {
            kind,
            position,
            collapsed: false,
            values: std::array::from_fn(|i| KnobId::ALL[i].default_normalized()),
            choices: Vec::new(),
            switches: Vec::new(),
            slots: 1,
            used: 0,
            knob_states: Default::default(),
        }
    }

    pub fn title(&self) -> &'static str {
        self.kind.title()
    }

    /// The node's rows, top to bottom.
    pub fn rows(&self) -> Vec<Row> {
        self.kind.behavior().rows(self)
    }

    /// A knob's value, normalized from 0.0 to 1.0.
    pub fn value(&self, id: KnobId) -> f32 {
        self.values[id as usize]
    }

    pub fn set_value(&mut self, id: KnobId, normalized: f32) {
        self.values[id as usize] = id.snap(normalized);
    }

    /// A knob's value in its own units.
    pub fn plain(&self, id: KnobId) -> f32 {
        id.to_plain(self.value(id))
    }

    pub fn reset_value(&mut self, id: KnobId) {
        self.values[id as usize] = id.default_normalized();
    }

    /// The labels of this node's switches, top to bottom.
    pub fn switch_labels(&self) -> &'static [&'static str] {
        self.kind.behavior().switch_labels()
    }

    /// True if this node draws a transfer curve.
    pub fn has_curve(&self) -> bool {
        self.rows()
            .iter()
            .any(|row| row.control == Some(Control::Curve))
    }

    /// The node's dropdowns, in the order its rows list them, with where each one sits.
    pub fn dropdowns(&self) -> Vec<Dropdown> {
        let labels = self.kind.behavior().dropdowns(self);
        self.rows()
            .iter()
            .enumerate()
            .filter_map(|(index, row)| match row.control {
                Some(Control::Dropdown(which)) => Some((index, which)),
                _ => None,
            })
            .filter_map(|(index, which)| {
                let labels = *labels.get(which)?;
                let name = self
                    .kind
                    .behavior()
                    .dropdown_names()
                    .get(which)
                    .copied()
                    .unwrap_or("");
                Some(Dropdown {
                    which,
                    name,
                    labels,
                    selected: self.choice(which).min(labels.len().saturating_sub(1)),
                    bounds: Rectangle {
                        x: self.position.x + DROPDOWN_INSET,
                        y: self.position.y + pin_y(index) - DROPDOWN_HEIGHT / 2.0,
                        width: NODE_WIDTH - 2.0 * DROPDOWN_INSET,
                        height: DROPDOWN_HEIGHT,
                    },
                })
            })
            .collect()
    }

    /// The dropdown under `point`, if the node has one there and isn't collapsed.
    pub fn dropdown_at(&self, point: Point) -> Option<usize> {
        if self.collapsed {
            return None;
        }
        self.dropdowns()
            .iter()
            .find(|dropdown| dropdown.bounds.contains(point))
            .map(|dropdown| dropdown.which)
    }

    /// The row chosen in dropdown `which`. A dropdown nobody has touched is at its first row.
    pub fn choice(&self, which: usize) -> usize {
        self.choices.get(which).copied().unwrap_or(0)
    }

    /// Whether switch `index` is on.
    pub fn switch(&self, index: usize) -> bool {
        self.switches.get(index).copied().unwrap_or(false)
    }

    /// Turns switch `index` on or off, growing the switch list if needed.
    pub fn set_switch(&mut self, index: usize, on: bool) {
        if self.switches.len() <= index {
            self.switches.resize(index + 1, false);
        }
        self.switches[index] = on;
    }

    /// Sets dropdown `which` to the row at `row`.
    pub fn choose(&mut self, which: usize, row: usize) {
        let labels = self.kind.behavior().dropdowns(self);
        if let Some(labels) = labels.get(which) {
            if row < labels.len() {
                if self.choices.len() <= which {
                    self.choices.resize(which + 1, 0);
                }
                self.choices[which] = row;
            }
        }
    }

    /// Every input port, top to bottom.
    pub fn input_ports(&self) -> Vec<Port> {
        self.rows()
            .into_iter()
            .filter_map(|row| row.input)
            .collect()
    }

    /// Every output port, top to bottom.
    pub fn output_ports(&self) -> Vec<Port> {
        self.rows()
            .into_iter()
            .filter_map(|row| row.output)
            .collect()
    }

    /// Full height of the node. A collapsed node is only its header. Each row's control takes the
    /// space of its row, so the height runs to the lowest one.
    pub fn height(&self) -> f32 {
        if self.collapsed {
            return HEADER_HEIGHT;
        }
        let bottom = self
            .rows()
            .iter()
            .enumerate()
            .map(|(index, row)| match row.control {
                Some(Control::Curve) => pin_y(index) - ROW_PITCH / 2.0 + CURVE_HEIGHT,
                Some(Control::Dropdown(_)) | Some(Control::Switch(_)) => {
                    pin_y(index) + SWITCH_ROW_HEIGHT / 2.0
                }
                None => pin_y(index) + ROW_PITCH / 2.0,
            })
            .fold(pin_y(0), f32::max);
        bottom + 8.0
    }

    pub fn bounds(&self) -> Rectangle {
        Rectangle {
            x: self.position.x,
            y: self.position.y,
            width: NODE_WIDTH,
            height: self.height(),
        }
    }

    pub fn header_bounds(&self) -> Rectangle {
        Rectangle {
            x: self.position.x,
            y: self.position.y,
            width: NODE_WIDTH,
            height: HEADER_HEIGHT,
        }
    }

    /// The transfer curve's box, for a map node. It sits on the row that holds the curve.
    pub fn curve_bounds(&self) -> Rectangle {
        let index = self
            .rows()
            .iter()
            .position(|row| row.control == Some(Control::Curve))
            .unwrap_or(0);
        Rectangle {
            x: self.position.x + DROPDOWN_INSET,
            y: self.position.y + pin_y(index) - ROW_PITCH / 2.0,
            width: NODE_WIDTH - 2.0 * DROPDOWN_INSET,
            height: CURVE_HEIGHT,
        }
    }

    /// The row of switch `index`, on the row that holds it.
    pub fn switch_bounds(&self, index: usize) -> Rectangle {
        let row = self
            .rows()
            .iter()
            .position(|row| row.control == Some(Control::Switch(index)))
            .unwrap_or(0);
        Rectangle {
            x: self.position.x + DROPDOWN_INSET,
            y: self.position.y + pin_y(row) - SWITCH_ROW_HEIGHT / 2.0,
            width: NODE_WIDTH - 2.0 * DROPDOWN_INSET,
            height: SWITCH_ROW_HEIGHT,
        }
    }

    /// The collapse arrow's hit area in the header.
    pub fn toggle_bounds(&self) -> Rectangle {
        Rectangle {
            x: self.position.x + ARROW_CENTER_X - ARROW_HIT_RADIUS,
            y: self.position.y + HEADER_HEIGHT / 2.0 - ARROW_HIT_RADIUS,
            width: 2.0 * ARROW_HIT_RADIUS,
            height: 2.0 * ARROW_HIT_RADIUS,
        }
    }

    /// The height of a pin's row. A collapsed node puts every pin at the header's center.
    pub fn pin_offset(&self, row: usize) -> f32 {
        if self.collapsed {
            HEADER_HEIGHT / 2.0
        } else {
            pin_y(row)
        }
    }

    /// Where a port's wire attaches for an input, or None if this node has no such input.
    pub fn input_point(&self, port: Port) -> Option<Point> {
        let row = self.rows().iter().position(|row| row.input == Some(port))?;
        Some(Point::new(
            self.position.x,
            self.position.y + self.pin_offset(row),
        ))
    }

    /// Where a port's wire attaches for an output, or None if this node has no such output.
    pub fn output_point(&self, port: Port) -> Option<Point> {
        let row = self
            .rows()
            .iter()
            .position(|row| row.output == Some(port))?;
        Some(Point::new(
            self.position.x + NODE_WIDTH,
            self.position.y + self.pin_offset(row),
        ))
    }

    /// Every knob on this node with where it is drawn. A collapsed node has no knobs on show.
    pub fn knob_slots(&self) -> Vec<KnobSlot> {
        if self.collapsed {
            return Vec::new();
        }
        self.rows()
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                let id = row.knob?;
                Some(KnobSlot {
                    id,
                    center: Point::new(
                        self.position.x + ROW_KNOB_CENTER_X,
                        self.position.y + pin_y(index),
                    ),
                    pinned: row.input == Some(Port::Knob(id)),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two nodes of one kind keep separate knob values.
    #[test]
    fn nodes_of_one_kind_do_not_share_knobs() {
        let mut first = FlodeNode::new(NodeKind::Oscillator, Point::new(0.0, 0.0));
        let second = FlodeNode::new(NodeKind::Oscillator, Point::new(0.0, 0.0));
        first.set_value(KnobId::Detune, 1.0);
        assert_eq!(first.plain(KnobId::Detune), 100.0);
        assert_eq!(second.plain(KnobId::Detune), KnobId::Detune.default_plain());
    }

    /// Every kind lists each knob at most once, and every pin sits on a row.
    #[test]
    fn every_kind_has_rows() {
        for kind in NodeKind::ALL {
            let node = FlodeNode::new(kind, Point::new(0.0, 0.0));
            assert!(!node.rows().is_empty(), "{kind:?}");
            let knobs: Vec<KnobId> = node.rows().iter().filter_map(|row| row.knob).collect();
            for (i, id) in knobs.iter().enumerate() {
                assert!(!knobs[..i].contains(id), "{kind:?} lists {id:?} twice");
            }
        }
    }
}
