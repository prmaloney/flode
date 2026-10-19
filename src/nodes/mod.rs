//! The node types. Every node in the graph is a `FlodeNode`. What differs between kinds is its
//! `NodeBehavior`: the rows it shows, the extra control under them, and its name. Each kind lives
//! in its own module and holds nothing but that behavior and the typed reads the audio engine
//! needs. A node owns its own knob values, so two nodes of one kind never share a value.
//!
//! A node is a header over a stack of rows. A row can have an input pin on the left edge, a knob
//! beside it, and an output pin on the right edge. Under the rows a node can have one extra
//! control, which is a dropdown or a switch.

pub mod chorus;
pub mod compressor;
pub mod connection;
pub mod delay;
pub mod distortion;
pub mod envelope;
pub mod eq;
pub mod filter;
pub mod flanger;
pub mod input;
pub mod knobs;
pub mod lfo;
pub mod macros;
pub mod map;
pub mod math;
pub mod mix;
pub mod monitor;
pub mod oscillator;
pub mod output;
pub mod phaser;
pub mod reverb;
pub mod viewer;

use crate::editor::knob;
use nih_plug_iced::{Point, Rectangle};
use serde::{Deserialize, Serialize};
use viewer::{Live, Trace, TRACE_SAMPLES};

pub use knobs::KnobId;

pub const NODE_WIDTH: f32 = 160.0;
/// The EQ node is wider, so its points can be placed more finely.
pub const EQ_WIDTH: f32 = 280.0;
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
/// The viewer on a node: a box that draws a shape. It is shorter than the curve, since nothing in
/// it is dragged.
pub const VIEWER_HEIGHT: f32 = 56.0;
/// The space a viewer row takes in the stack: the box and a margin above and below it. Rows after
/// it start below this.
pub const VIEWER_ROW: f32 = VIEWER_HEIGHT + 8.0;
/// The EQ node's graph, where its points are dragged. It is taller than the viewer so the points
/// have room.
pub const EQ_HEIGHT: f32 = 100.0;
/// The space the graph takes in the stack, with the same margin as a viewer.
pub const EQ_ROW: f32 = EQ_HEIGHT + 8.0;
/// The space the EQ's analyzer checkbox takes in the stack.
pub const EQ_SWITCH_ROW: f32 = 28.0;
/// The monitor node is wide and its screen tall, so a trace or a spectrum has room to read.
pub const MONITOR_WIDTH: f32 = 280.0;
pub const MONITOR_HEIGHT: f32 = 150.0;
/// The space the screen takes in the stack, with the same margin as a viewer.
pub const MONITOR_ROW: f32 = MONITOR_HEIGHT + 8.0;
/// The shelf, cut, peak, or notch toggle at the right end of each band's section header, and its
/// gap to the node's edge.
pub const EQ_TOGGLE_WIDTH: f32 = 34.0;
pub const EQ_TOGGLE_HEIGHT: f32 = 18.0;
pub const EQ_TOGGLE_INSET: f32 = 10.0;

/// The header row of a section: its arrow and title.
pub const SECTION_HEIGHT: f32 = 22.0;
/// The gap between pins that are packed together, in a collapsed section or a collapsed node.
pub const PACKED_PITCH: f32 = 9.0;
/// Space above and below the packed pins of a collapsed node.
pub const COLLAPSED_PADDING: f32 = 5.0;

/// A group of rows that can fold away. It starts at a row with `Control::Section` and runs to the
/// next one, or to the end of the node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
    pub title: &'static str,
    /// True if the section starts folded.
    pub collapsed: bool,
}

/// How a row is laid out right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowShape {
    /// The row at full size, with its knob, label, and controls.
    Full,
    /// A row folded away that still has pins. Its pins stay, close together, with no labels or
    /// knob.
    Packed,
    /// A row folded away that has no pins. It takes no space.
    Hidden,
    /// The header of section `n`.
    SectionHeader(usize),
}

/// Where a row sits in its node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowPlace {
    /// The height of the row's middle, measured from the top of the node. Pins and knobs sit
    /// here.
    pub y: f32,
    pub shape: RowShape,
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
    Distortion,
    Compressor,
    Reverb,
    Delay,
    Chorus,
    Phaser,
    Flanger,
    Eq,
    Math,
    Monitor,
    Macro,
}

/// The groups the add menu sorts the node kinds into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// Nodes that make control signals.
    Driver,
    /// Nodes that route or shape signals without a sound of their own.
    Util,
    /// Nodes that change the sound of a signal.
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
            Category::Driver => &[NodeKind::Envelope, NodeKind::Lfo, NodeKind::Macro],
            Category::Util => &[
                NodeKind::Output,
                NodeKind::Map,
                NodeKind::Math,
                NodeKind::Monitor,
                NodeKind::Input,
                NodeKind::Mix,
            ],
            Category::Effects => &[
                NodeKind::Filter,
                NodeKind::Eq,
                NodeKind::Distortion,
                NodeKind::Compressor,
                NodeKind::Reverb,
                NodeKind::Delay,
                NodeKind::Chorus,
                NodeKind::Phaser,
                NodeKind::Flanger,
            ],
        }
    }
}

impl NodeKind {
    /// Every kind. The quick add box searches them, and the tests walk them.
    pub const ALL: [NodeKind; 19] = [
        NodeKind::Oscillator,
        NodeKind::Envelope,
        NodeKind::Filter,
        NodeKind::Lfo,
        NodeKind::Input,
        NodeKind::Map,
        NodeKind::Mix,
        NodeKind::Output,
        NodeKind::Distortion,
        NodeKind::Compressor,
        NodeKind::Reverb,
        NodeKind::Delay,
        NodeKind::Chorus,
        NodeKind::Phaser,
        NodeKind::Flanger,
        NodeKind::Eq,
        NodeKind::Math,
        NodeKind::Monitor,
        NodeKind::Macro,
    ];

    /// True for a kind a graph holds at most one of. The output node decides what plays, and the
    /// macro knobs are one fixed set of host parameters.
    pub fn is_unique(self) -> bool {
        matches!(self, NodeKind::Output | NodeKind::Macro)
    }

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
            NodeKind::Distortion => &distortion::Distortion,
            NodeKind::Compressor => &compressor::Compressor,
            NodeKind::Reverb => &reverb::Reverb,
            NodeKind::Delay => &delay::Delay,
            NodeKind::Chorus => &chorus::Chorus,
            NodeKind::Phaser => &phaser::Phaser,
            NodeKind::Flanger => &flanger::Flanger,
            NodeKind::Eq => &eq::Eq,
            NodeKind::Math => &math::Math,
            NodeKind::Monitor => &monitor::Monitor,
            NodeKind::Macro => &macros::Macros,
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
    /// The notes themselves, from an input node to an oscillator. The same pin kind is on both
    /// ends, like `Audio`.
    Notes,
    /// Macro knob `n` on a macro node, sent as a control signal.
    Macro(usize),
}

/// The kinds of signal a wire can carry. A wire joins two ports of the same kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Sound.
    Audio,
    /// A level from 0.0 to 1.0 that drives a knob.
    Control,
    /// Which notes play, and how.
    Notes,
}

impl Port {
    /// The kind of signal this port carries or takes.
    pub fn signal(self) -> Signal {
        match self {
            Port::Audio | Port::AudioIn(_) | Port::Slot(_) => Signal::Audio,
            Port::Shape | Port::Knob(_) | Port::Midi(_) | Port::Macro(_) => Signal::Control,
            Port::Notes => Signal::Notes,
        }
    }

    /// True for an audio input, which takes sound.
    pub fn is_audio(self) -> bool {
        self.signal() == Signal::Audio
    }

    /// True for a control output: a shape pin, a MIDI source, or a macro.
    pub fn is_control_out(self) -> bool {
        matches!(self, Port::Shape | Port::Midi(_) | Port::Macro(_))
    }

    /// The name drawn beside an output pin, for the ones that need one.
    pub fn output_label(self) -> Option<&'static str> {
        match self {
            Port::Shape => Some("Shape"),
            Port::Midi(source) => Some(source.label()),
            Port::Notes => Some("MIDI"),
            _ => None,
        }
    }

    /// The name drawn beside an input pin that has no knob of its own to name it.
    pub fn input_label(self) -> Option<&'static str> {
        match self {
            Port::Shape => Some("Shape"),
            Port::AudioIn(0) => Some("A"),
            Port::AudioIn(1) => Some("B"),
            Port::Notes => Some("MIDI"),
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
    /// A box that draws the node's shape, such as its waveform. It cannot be edited. The kind's
    /// `trace` says what it shows.
    Viewer,
    /// The EQ node's graph. Its points are dragged, and the node's kind says what they set.
    Eq,
    /// The monitor node's screen. It draws what the audio thread measured and takes no input.
    Monitor,
    /// The header of section number `n`. The rows after it, up to the next header, fold with it.
    Section(usize),
}

impl Row {
    /// True if the row has a pin on either edge.
    pub fn has_pins(&self) -> bool {
        self.input.is_some() || self.output.is_some()
    }

    /// The header of section `n`.
    pub fn section(n: usize) -> Self {
        Self::control(Control::Section(n))
    }

    /// A knob with its own control pin. A control wire can drive it.
    pub fn driven_knob(id: KnobId) -> Self {
        Self {
            input: Some(Port::Knob(id)),
            knob: Some(id),
            ..Self::default()
        }
    }

    /// A knob with no pin, which a control wire can't drive.
    pub fn plain_knob(id: KnobId) -> Self {
        Self {
            knob: Some(id),
            ..Self::default()
        }
    }

    pub fn input(port: Port) -> Self {
        Self {
            input: Some(port),
            ..Self::default()
        }
    }

    /// A row that holds a control and nothing else.
    pub fn control(control: Control) -> Self {
        Self {
            control: Some(control),
            ..Self::default()
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

    /// The node's width. Most kinds are `NODE_WIDTH`.
    fn width(&self) -> f32 {
        NODE_WIDTH
    }

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

    /// What switching the node off does to its signal. Most kinds can't be switched off.
    fn bypass(&self) -> Bypass {
        Bypass::None
    }

    /// The sections the node's rows are grouped into, by number. A kind with none has fixed rows.
    fn sections(&self) -> &'static [Section] {
        &[]
    }

    /// Knobs this kind starts at a value other than the knob's own default, in plain units.
    fn defaults(&self) -> &'static [(KnobId, f32)] {
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

    /// What the node's viewer draws, with `samples` points. A kind with a `Control::Viewer` row
    /// overrides this. Read knobs through `live`, not the node, so a driven knob moves the shape.
    fn trace(&self, _node: &FlodeNode, _live: &Live, _samples: usize) -> Option<Trace> {
        None
    }
}

/// What a node does to its signal when it is switched off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bypass {
    /// The node can't be switched off.
    None,
    /// A source goes silent, as if it were not wired in.
    Mute,
    /// A processor hands its first audio input straight to its output.
    PassThrough,
}

impl Bypass {
    /// The word the node shows while it is switched off, or None if it can't be.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Bypass::None => None,
            Bypass::Mute => Some("mute"),
            Bypass::PassThrough => Some("bypass"),
        }
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
    /// True while the node is muted or bypassed, as its kind's `bypass` says. A kind that can't
    /// be bypassed ignores it.
    pub bypassed: bool,
    /// Whether each section of the node is folded, by section number.
    pub sections_collapsed: Vec<bool>,
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
        let mut node = Self {
            kind,
            position,
            collapsed: false,
            bypassed: false,
            sections_collapsed: kind
                .behavior()
                .sections()
                .iter()
                .map(|section| section.collapsed)
                .collect(),
            values: std::array::from_fn(|i| KnobId::ALL[i].default_normalized()),
            choices: Vec::new(),
            switches: Vec::new(),
            slots: 1,
            used: 0,
            knob_states: std::array::from_fn(|_| Default::default()),
        };
        // A kind can start a knob somewhere other than the knob's own default, such as a
        // distortion that starts fully wet.
        for &(id, plain) in kind.behavior().defaults() {
            node.values[id as usize] = id.to_normalized(plain);
        }
        node
    }

    /// True if the editor may copy this kind of node. A kind that is one per graph can't be.
    pub fn can_duplicate(&self) -> bool {
        !self.kind.is_unique()
    }

    /// A copy of the node at `position`, with the same knob values, dropdown choices, switches,
    /// and on/off state. It has no wires, since those belong to the graph.
    pub fn duplicate(&self, position: Point) -> FlodeNode {
        let mut copy = FlodeNode::new(self.kind, position);
        copy.collapsed = self.collapsed;
        copy.bypassed = self.bypassed;
        copy.sections_collapsed = self.sections_collapsed.clone();
        copy.values = self.values;
        copy.choices = self.choices.clone();
        copy.switches = self.switches.clone();
        copy
    }

    /// What switching this node off does. A node can only be bypassed if its kind has a bypass.
    pub fn bypass(&self) -> Bypass {
        self.kind.behavior().bypass()
    }

    /// The word to show on the node while it is switched off, or None while it is on.
    pub fn bypass_label(&self) -> Option<&'static str> {
        if self.bypassed {
            self.bypass().label()
        } else {
            None
        }
    }

    /// Switches the node off or on. Returns false, and changes nothing, for a kind that can't be
    /// switched off.
    pub fn toggle_bypass(&mut self) -> bool {
        if self.bypass() == Bypass::None {
            return false;
        }
        self.bypassed = !self.bypassed;
        true
    }

    pub fn title(&self) -> &'static str {
        self.kind.title()
    }

    /// How wide the node is.
    pub fn width(&self) -> f32 {
        self.kind.behavior().width()
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

    /// Whether section `n` is folded.
    pub fn section_collapsed(&self, n: usize) -> bool {
        self.sections_collapsed.get(n).copied().unwrap_or(false)
    }

    /// Folds section `n` if it is open, and opens it if it is folded.
    pub fn toggle_section(&mut self, n: usize) {
        let count = self.kind.behavior().sections().len();
        if n >= count {
            return;
        }
        if self.sections_collapsed.len() < count {
            self.sections_collapsed.resize(count, false);
        }
        self.sections_collapsed[n] = !self.sections_collapsed[n];
    }

    /// Where each row sits, top to bottom, in the same order as `rows`.
    ///
    /// An open node lays its rows at a steady pitch, with a section header between groups. A row in
    /// a folded section keeps its pins and drops everything else: the pins pack close together
    /// under the section's header. A node folded as a whole does the same for every row, with no
    /// section headers, so a collapsed node is its header over a short stack of pins.
    pub fn row_places(&self) -> Vec<RowPlace> {
        self.places_of(&self.rows())
    }

    /// `row_places` for a given list of rows, in place of the kind's own.
    fn places_of(&self, rows: &[Row]) -> Vec<RowPlace> {
        let mut top = if self.collapsed {
            HEADER_HEIGHT + COLLAPSED_PADDING
        } else {
            HEADER_HEIGHT
        };
        let mut section = None;
        rows.iter()
            .map(|row| {
                if let Some(Control::Section(n)) = row.control {
                    section = Some(n);
                    if self.collapsed {
                        return RowPlace {
                            y: top,
                            shape: RowShape::Hidden,
                        };
                    }
                    let y = top + SECTION_HEIGHT / 2.0;
                    top += SECTION_HEIGHT;
                    return RowPlace {
                        y,
                        shape: RowShape::SectionHeader(n),
                    };
                }
                let folded = self.collapsed || section.is_some_and(|n| self.section_collapsed(n));
                if !folded {
                    // A viewer or a graph is taller than a row, and the rows below it start under it.
                    let pitch = match row.control {
                        Some(Control::Viewer) => VIEWER_ROW,
                        Some(Control::Eq) => EQ_ROW,
                        Some(Control::Monitor) => MONITOR_ROW,
                        Some(Control::Curve) => CURVE_HEIGHT,
                        // The EQ's analyzer checkbox sits close under its graph.
                        Some(Control::Switch(_)) if self.kind == NodeKind::Eq => EQ_SWITCH_ROW,
                        _ => ROW_PITCH,
                    };
                    let y = top + pitch / 2.0;
                    top += pitch;
                    RowPlace {
                        y,
                        shape: RowShape::Full,
                    }
                } else if row.has_pins() {
                    let y = top + PACKED_PITCH / 2.0;
                    top += PACKED_PITCH;
                    RowPlace {
                        y,
                        shape: RowShape::Packed,
                    }
                } else {
                    RowPlace {
                        y: top,
                        shape: RowShape::Hidden,
                    }
                }
            })
            .collect()
    }

    /// The section whose header is under `point`, if there is one. A node folded as a whole has no
    /// headers on show.
    pub fn section_header_at(&self, point: Point) -> Option<usize> {
        if self.collapsed {
            return None;
        }
        self.row_places()
            .iter()
            .find_map(|place| match place.shape {
                RowShape::SectionHeader(n) => {
                    let header = Rectangle {
                        x: self.position.x,
                        y: self.position.y + place.y - SECTION_HEIGHT / 2.0,
                        width: self.width(),
                        height: SECTION_HEIGHT,
                    };
                    header.contains(point).then_some(n)
                }
                _ => None,
            })
    }

    /// The number shown in the box for typing over knob `id`: its value in the units the knob
    /// reads in.
    pub fn knob_entry_text(&self, id: KnobId) -> String {
        knobs::entry_text(id.to_entry(self.value(id)))
    }

    /// Sets knob `id` from a number typed in the units `knob_entry_text` uses. The value stops at
    /// the ends of the knob's range.
    pub fn enter_knob_value(&mut self, id: KnobId, number: f32) {
        let normalized = id.from_entry(number);
        self.set_value(id, if normalized.is_nan() { 0.0 } else { normalized });
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
        let places = self.row_places();
        self.rows()
            .iter()
            .enumerate()
            .filter_map(|(index, row)| match row.control {
                // A dropdown in a folded section is not on show.
                Some(Control::Dropdown(which)) if places[index].shape == RowShape::Full => {
                    Some((index, which))
                }
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
                        y: self.position.y + places[index].y - DROPDOWN_HEIGHT / 2.0,
                        width: self.width() - 2.0 * DROPDOWN_INSET,
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

    /// Full height of the node. Each row's control takes the space of its row, so the height runs
    /// to the lowest one. A node folded as a whole is its header and its packed pins.
    pub fn height(&self) -> f32 {
        self.height_of(&self.rows())
    }

    /// `height` for a given list of rows, in place of the kind's own.
    fn height_of(&self, rows: &[Row]) -> f32 {
        let bottom = self
            .places_of(rows)
            .iter()
            .zip(rows)
            .map(|(place, row)| match place.shape {
                RowShape::Hidden => 0.0,
                RowShape::Packed => place.y + PACKED_PITCH / 2.0,
                RowShape::SectionHeader(_) => place.y + SECTION_HEIGHT / 2.0,
                RowShape::Full => match row.control {
                    Some(Control::Curve) => place.y + CURVE_HEIGHT / 2.0,
                    Some(Control::Viewer) => place.y + VIEWER_ROW / 2.0,
                    Some(Control::Eq) => place.y + EQ_ROW / 2.0,
                    Some(Control::Monitor) => place.y + MONITOR_ROW / 2.0,
                    Some(Control::Dropdown(_)) | Some(Control::Switch(_)) => {
                        place.y + SWITCH_ROW_HEIGHT / 2.0
                    }
                    _ => place.y + ROW_PITCH / 2.0,
                },
            })
            .fold(PIN_ROW0_Y, f32::max);
        if self.collapsed {
            // A collapsed node with no pins is only its header.
            let packed = self
                .places_of(rows)
                .iter()
                .any(|place| place.shape == RowShape::Packed);
            return if packed {
                bottom + COLLAPSED_PADDING
            } else {
                HEADER_HEIGHT
            };
        }
        bottom + 8.0
    }

    pub fn bounds(&self) -> Rectangle {
        Rectangle {
            x: self.position.x,
            y: self.position.y,
            width: self.width(),
            height: self.height(),
        }
    }

    pub fn header_bounds(&self) -> Rectangle {
        Rectangle {
            x: self.position.x,
            y: self.position.y,
            width: self.width(),
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
        let places = self.row_places();
        Rectangle {
            x: self.position.x + DROPDOWN_INSET,
            y: self.position.y + places.get(index).map_or(PIN_ROW0_Y, |p| p.y) - CURVE_HEIGHT / 2.0,
            width: self.width() - 2.0 * DROPDOWN_INSET,
            height: CURVE_HEIGHT,
        }
    }

    /// True if this node draws a viewer and it is on show. A folded section or a collapsed node
    /// hides it.
    pub fn has_viewer(&self) -> bool {
        self.viewer_bounds().is_some()
    }

    /// The viewer's box, centered on its row. None if the node has no viewer, or it is folded
    /// away.
    pub fn viewer_bounds(&self) -> Option<Rectangle> {
        self.viewer_bounds_of(&self.rows())
    }

    /// `viewer_bounds` for a given list of rows, in place of the kind's own.
    fn viewer_bounds_of(&self, rows: &[Row]) -> Option<Rectangle> {
        let index = rows
            .iter()
            .position(|row| row.control == Some(Control::Viewer))?;
        let place = self.places_of(rows).get(index).copied()?;
        if place.shape != RowShape::Full {
            return None;
        }
        Some(Rectangle {
            x: self.position.x + DROPDOWN_INSET,
            y: self.position.y + place.y - VIEWER_HEIGHT / 2.0,
            width: self.width() - 2.0 * DROPDOWN_INSET,
            height: VIEWER_HEIGHT,
        })
    }

    /// The EQ graph's box, centered on its row. None if the node has no graph, or it is folded
    /// away.
    pub fn eq_bounds(&self) -> Option<Rectangle> {
        let rows = self.rows();
        let index = rows
            .iter()
            .position(|row| row.control == Some(Control::Eq))?;
        let place = self.places_of(&rows).get(index).copied()?;
        if place.shape != RowShape::Full {
            return None;
        }
        Some(Rectangle {
            x: self.position.x + DROPDOWN_INSET,
            y: self.position.y + place.y - EQ_HEIGHT / 2.0,
            width: self.width() - 2.0 * DROPDOWN_INSET,
            height: EQ_HEIGHT,
        })
    }

    /// The monitor screen's box, centered on its row. None if the node has no screen, or it is
    /// folded away.
    pub fn monitor_bounds(&self) -> Option<Rectangle> {
        let rows = self.rows();
        let index = rows
            .iter()
            .position(|row| row.control == Some(Control::Monitor))?;
        let place = self.places_of(&rows).get(index).copied()?;
        if place.shape != RowShape::Full {
            return None;
        }
        Some(Rectangle {
            x: self.position.x + DROPDOWN_INSET,
            y: self.position.y + place.y - MONITOR_HEIGHT / 2.0,
            width: self.width() - 2.0 * DROPDOWN_INSET,
            height: MONITOR_HEIGHT,
        })
    }

    /// The toggle in each band's section header of an EQ node, as the band it flips and where it
    /// sits. The header stays on show when its section is folded. A node folded as a whole, and
    /// every other kind, has none.
    pub fn eq_toggles(&self) -> Vec<(usize, Rectangle)> {
        if self.kind != NodeKind::Eq || self.collapsed {
            return Vec::new();
        }
        self.row_places()
            .iter()
            .filter_map(|place| match place.shape {
                RowShape::SectionHeader(band) => Some((
                    band,
                    Rectangle {
                        x: self.position.x + self.width() - EQ_TOGGLE_INSET - EQ_TOGGLE_WIDTH,
                        y: self.position.y + place.y - EQ_TOGGLE_HEIGHT / 2.0,
                        width: EQ_TOGGLE_WIDTH,
                        height: EQ_TOGGLE_HEIGHT,
                    },
                )),
                _ => None,
            })
            .collect()
    }

    /// The band whose toggle is under `point`, if any.
    pub fn eq_toggle_at(&self, point: Point) -> Option<usize> {
        self.eq_toggles()
            .into_iter()
            .find(|(_, bounds)| bounds.contains(point))
            .map(|(band, _)| band)
    }

    /// What the node's viewer draws right now, from knob values in `live`.
    pub fn trace(&self, live: &Live) -> Option<Trace> {
        self.kind.behavior().trace(self, live, TRACE_SAMPLES)
    }

    /// The row of switch `index`, on the row that holds it. A switch in a folded section, or in a
    /// folded node, is not on show and has no area.
    pub fn switch_bounds(&self, index: usize) -> Rectangle {
        let row = self
            .rows()
            .iter()
            .position(|row| row.control == Some(Control::Switch(index)))
            .unwrap_or(0);
        let place = self.row_places().get(row).copied();
        let shown = place.is_some_and(|place| place.shape == RowShape::Full);
        Rectangle {
            x: self.position.x + DROPDOWN_INSET,
            y: self.position.y + place.map_or(PIN_ROW0_Y, |p| p.y) - SWITCH_ROW_HEIGHT / 2.0,
            width: self.width() - 2.0 * DROPDOWN_INSET,
            height: if shown { SWITCH_ROW_HEIGHT } else { 0.0 },
        }
    }

    /// True if switch `index` is on show.
    pub fn switch_shown(&self, index: usize) -> bool {
        self.switch_bounds(index).height > 0.0
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

    /// The height of a pin's row, from the top of the node. Pins of a folded section or a folded
    /// node sit close together.
    pub fn pin_offset(&self, row: usize) -> f32 {
        self.row_places()
            .get(row)
            .map_or(HEADER_HEIGHT / 2.0, |place| place.y)
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
            self.position.x + self.width(),
            self.position.y + self.pin_offset(row),
        ))
    }

    /// Every knob on this node with where it is drawn. A collapsed node has no knobs on show.
    pub fn knob_slots(&self) -> Vec<KnobSlot> {
        if self.collapsed {
            return Vec::new();
        }
        let places = self.row_places();
        self.rows()
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                let id = row.knob?;
                // A knob in a folded section is not on show. Its pin still is.
                if places[index].shape != RowShape::Full {
                    return None;
                }
                Some(KnobSlot {
                    id,
                    center: Point::new(
                        self.position.x + ROW_KNOB_CENTER_X,
                        self.position.y + places[index].y,
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
