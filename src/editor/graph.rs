//! The graph model and its editing state: nodes, wires, and what the mouse is doing to them.

use crate::editor::piano::Piano;
use crate::nodes::connection::Connection;
use crate::nodes::knobs::KnobId;
use crate::nodes::macros::MACRO_COUNT;
use crate::nodes::output::MAX_SLOTS;
use crate::nodes::{Category, FlodeNode, NodeKind, Port, HEADER_HEIGHT, NODE_WIDTH};
use crate::patch::control::{ControlLevels, Route};
use crate::patch::wiring::Wiring;
use crate::patch::Patch;
use crate::presets::{self, Library, Preset};
use nih_plug_iced::keyboard::KeyCode;
use nih_plug_iced::{Point, Vector};
use std::path::PathBuf;
use std::time::Instant;

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

/// A change to a macro knob that the editor has to pass to the host, since the macro knobs are host
/// parameters. The index is the macro's, from 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum MacroEdit {
    /// A drag starts. The host groups the changes until the drag ends.
    Begin(usize),
    /// The knob is at this normalized value.
    Set(usize, f32),
    End(usize),
}

/// The open preset list: what is saved, and the category whose submenu is open, if one is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PresetMenu {
    pub(super) library: Library,
    /// The position in `library.categories` of the open submenu.
    pub(super) open: Option<usize>,
}

/// Which field of the save box takes the typing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EntryField {
    Name,
    Category,
}

/// The save box: a name, a category, and the field being typed in. It also holds the categories
/// that already exist, so one can be picked instead of typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NameEntry {
    pub(super) name: String,
    pub(super) category: String,
    pub(super) field: EntryField,
    /// The existing categories, in the order the list shows them.
    pub(super) categories: Vec<String>,
}

impl NameEntry {
    /// The existing category that the typed one names, ignoring case and spaces at the ends.
    pub(super) fn existing_category(&self) -> Option<&String> {
        let typed = self.category.trim().to_lowercase();
        self.categories
            .iter()
            .find(|name| name.to_lowercase() == typed)
    }

    /// True if the typed category is not one that exists yet, so saving would make a folder.
    pub(super) fn is_new_category(&self) -> bool {
        !self.category.trim().is_empty() && self.existing_category().is_none()
    }

    /// Picks the existing category `step` places along from the typed one, wrapping around. From
    /// no category, stepping on goes to the first and stepping back goes to the last. The typing
    /// moves to the category field.
    pub(super) fn cycle_category(&mut self, step: i32) {
        self.field = EntryField::Category;
        let count = self.categories.len() as i32;
        if count == 0 {
            return;
        }
        let at = self
            .existing_category()
            .and_then(|found| self.categories.iter().position(|name| name == found));
        let next = match at {
            Some(at) => (at as i32 + step).rem_euclid(count),
            None if step >= 0 => 0,
            None => count - 1,
        };
        self.category = self.categories[next as usize].clone();
    }

    /// The text of the field being typed in.
    pub(super) fn active_mut(&mut self) -> &mut String {
        match self.field {
            EntryField::Name => &mut self.name,
            EntryField::Category => &mut self.category,
        }
    }

    /// Moves the typing to the other field.
    pub(super) fn switch_field(&mut self) {
        self.field = match self.field {
            EntryField::Name => EntryField::Category,
            EntryField::Category => EntryField::Name,
        };
    }
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
    /// True when the computer keyboard plays notes, which is in the standalone app. The strip
    /// marks the keys it plays.
    pub(super) computer_keys: bool,
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
    /// Changes to macro knobs the editor has not yet passed to the host, oldest first.
    pub(super) macro_edits: Vec<MacroEdit>,
    /// The folder presets are saved in and loaded from. None if the home folder is unknown.
    pub(super) preset_dir: Option<PathBuf>,
    /// The preset this graph was last saved as or loaded from.
    pub(super) preset_name: Option<String>,
    /// The category of that preset. Empty if it has none.
    pub(super) preset_category: String,
    /// The open preset list, read when it opened.
    pub(super) preset_list: Option<PresetMenu>,
    /// The name and category being typed, while the save box is open.
    pub(super) name_entry: Option<NameEntry>,
    /// A name and category the user confirmed. The editor saves the preset under them, since the
    /// macro values are the host's.
    pub(super) pending_save: Option<(String, String)>,
    /// A line shown under the pill for a few seconds, such as "Saved Lead".
    pub(super) status: Option<(String, Instant)>,
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
/// The default graph runs left to right: the MIDI input at the left edge, then the envelope, then
/// the oscillator the envelope shapes. Nothing is stacked, since the envelope and oscillator are
/// tall once they show a viewer.
pub(super) const DEFAULT_INPUT_X: f32 = 24.0;
pub(super) const DEFAULT_ENVELOPE_X: f32 = DEFAULT_INPUT_X + NODE_WIDTH + 48.0;
pub(super) const DEFAULT_OSC_X: f32 = DEFAULT_ENVELOPE_X + NODE_WIDTH + 48.0;
pub(super) const DEFAULT_OUTPUT_X: f32 = 1024.0 - 24.0 - NODE_WIDTH;

/// The graph a new instance starts with: an oscillator, wired to the output, with an envelope
/// shaping its amplitude and a MIDI input feeding it notes. The oscillator is node 0 and the
/// output is node 1, the envelope is node 2, and the MIDI input is node 3. The MIDI input is at
/// the left edge, with the envelope to its right.
impl Default for Graph {
    fn default() -> Self {
        let mut graph = Self::with(
            vec![
                FlodeNode::new(NodeKind::Oscillator, Point::new(DEFAULT_OSC_X, 80.0)),
                FlodeNode::new(NodeKind::Output, Point::new(DEFAULT_OUTPUT_X, 80.0)),
                FlodeNode::new(NodeKind::Envelope, Point::new(DEFAULT_ENVELOPE_X, 80.0)),
                FlodeNode::new(NodeKind::Input, Point::new(DEFAULT_INPUT_X, 80.0)),
            ],
            vec![
                Connection {
                    from: 3,
                    out: Port::Notes,
                    to: 0,
                    input: Port::Notes,
                },
                Connection {
                    from: 0,
                    out: Port::Audio,
                    to: 1,
                    input: Port::Slot(0),
                },
                Connection {
                    from: 2,
                    out: Port::Shape,
                    to: 0,
                    input: Port::Shape,
                },
            ],
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
            computer_keys: false,
            category_menu: None,
            view: View::default(),
            last_screen: Point::new(0.0, 0.0),
            shortcut_held: None,
            macro_edits: Vec::new(),
            preset_dir: presets::directory(),
            preset_name: None,
            preset_category: String::new(),
            preset_list: None,
            name_entry: None,
            pending_save: None,
            status: None,
        }
    }

    /// Shows a line under the pill for a few seconds.
    pub(super) fn say(&mut self, text: impl Into<String>) {
        self.status = Some((text.into(), Instant::now()));
    }

    /// Opens the preset list, reading what is saved now.
    pub(super) fn open_preset_list(&mut self) {
        let library = self
            .preset_dir
            .as_deref()
            .map(presets::library_in)
            .unwrap_or_default();
        self.preset_list = Some(PresetMenu {
            library,
            open: None,
        });
    }

    /// Opens the save box. It starts with the current preset's name and category, so saving again
    /// replaces it.
    pub(super) fn start_name_entry(&mut self) {
        self.preset_list = None;
        let categories = self
            .preset_dir
            .as_deref()
            .map(presets::library_in)
            .unwrap_or_default()
            .categories
            .into_iter()
            .map(|category| category.name)
            .collect();
        self.name_entry = Some(NameEntry {
            name: self.preset_name.clone().unwrap_or_default(),
            category: self.preset_category.clone(),
            field: EntryField::Name,
            categories,
        });
    }

    /// Takes the typed name and category. One that is not usable keeps the box open and says why.
    pub(super) fn confirm_name_entry(&mut self) {
        let Some(entry) = &self.name_entry else {
            return;
        };
        let Some(name) = presets::clean_name(&entry.name) else {
            self.say("Type a name first");
            return;
        };
        // A category that matches an existing one apart from its case is that one, so a slip of
        // the shift key does not make a second folder.
        let typed = entry.existing_category().unwrap_or(&entry.category);
        let Some(category) = presets::clean_category(typed) else {
            self.say("That category is not usable");
            return;
        };
        self.name_entry = None;
        self.pending_save = Some((name, category));
    }

    /// Saves the preset a confirmed name asked for, with the macro knobs' current values. Does
    /// nothing if no name is waiting.
    pub(super) fn save_pending_preset(&mut self, macros: [f32; MACRO_COUNT]) {
        let Some((name, category)) = self.pending_save.take() else {
            return;
        };
        let Some(dir) = self.preset_dir.clone() else {
            self.say("Could not save: no preset folder");
            return;
        };
        let preset = Preset {
            name: name.clone(),
            category: category.clone(),
            graph: self.to_saved(),
            macros,
        };
        match presets::save_in(&dir, &preset) {
            Ok(()) => {
                self.preset_name = Some(name.clone());
                self.preset_category = category;
                self.say(format!("Saved {name}"));
            }
            Err(error) => self.say(format!("Could not save: {error}")),
        }
    }

    /// Replaces the graph with the saved preset `name` in `category` (empty for none), and sends
    /// its macro values to the host. Returns true if the graph changed. A preset that can't be
    /// read leaves the graph alone and says so.
    pub(super) fn load_preset(&mut self, category: &str, name: &str) -> bool {
        let loaded = match self.preset_dir.as_deref() {
            Some(dir) => presets::load_in(dir, category, name),
            None => Err(std::io::Error::other("no preset folder")),
        };
        let preset = match loaded {
            Ok(preset) => preset,
            Err(error) => {
                self.say(format!("Could not load {name}: {error}"));
                return false;
            }
        };
        self.replace_with(Graph::from_saved(&preset.graph), preset.macros);
        self.preset_name = Some(preset.name.clone());
        self.preset_category = preset.category.clone();
        self.say(format!("Loaded {}", preset.name));
        true
    }

    /// Goes back to the graph a new instance starts with, and sets the macro knobs back to their
    /// starting values. Whatever was on the canvas is dropped, saved or not, and no preset is
    /// current afterward.
    pub(super) fn init_default(&mut self) {
        let macros = KnobId::MACROS.map(KnobId::default_normalized);
        self.replace_with(Graph::default(), macros);
        self.say("Back to the default patch");
    }

    /// Swaps in `fresh` for the nodes and wires, and sends `macros` to the host. The view, the
    /// piano's held keys, and where presets live stay as they were, and so do the menus' state,
    /// which is closed.
    fn replace_with(&mut self, fresh: Graph, macros: [f32; MACRO_COUNT]) {
        let view = self.view;
        let piano = std::mem::take(&mut self.piano);
        let computer_keys = self.computer_keys;
        let dir = self.preset_dir.clone();
        *self = fresh;
        self.view = view;
        self.piano = piano;
        self.computer_keys = computer_keys;
        self.preset_dir = dir;
        for (index, &value) in macros.iter().enumerate() {
            self.macro_edits.push(MacroEdit::Begin(index));
            self.macro_edits.push(MacroEdit::Set(index, value));
            self.macro_edits.push(MacroEdit::End(index));
        }
    }

    /// Which macro a knob on a node is, or None for any other knob.
    fn macro_of(&self, node: usize, id: KnobId) -> Option<usize> {
        let is_macro_node = self.nodes.get(node)?.kind == NodeKind::Macro;
        id.macro_index().filter(|_| is_macro_node)
    }

    /// Notes that a drag of a macro knob started. Any other knob is ignored.
    pub(super) fn macro_begin(&mut self, node: usize, id: KnobId) {
        if let Some(index) = self.macro_of(node, id) {
            self.macro_edits.push(MacroEdit::Begin(index));
        }
    }

    /// Notes the macro knob's current value, for the host. Any other knob is ignored.
    pub(super) fn macro_set(&mut self, node: usize, id: KnobId) {
        if let Some(index) = self.macro_of(node, id) {
            let value = self.nodes[node].value(id);
            self.macro_edits.push(MacroEdit::Set(index, value));
        }
    }

    /// Notes that a drag of a macro knob ended. Any other knob is ignored.
    pub(super) fn macro_end(&mut self, node: usize, id: KnobId) {
        if let Some(index) = self.macro_of(node, id) {
            self.macro_edits.push(MacroEdit::End(index));
        }
    }

    /// Shows the host's macro values on the macro node's knobs. The knob being dragged keeps the
    /// value under the mouse.
    pub(super) fn show_macros(&mut self, values: [f32; MACRO_COUNT]) {
        let dragged = match self.interaction {
            Interaction::Knob { id, .. } => Some(id),
            _ => None,
        };
        for node in self.nodes.iter_mut().filter(|n| n.kind == NodeKind::Macro) {
            for (&id, &value) in KnobId::MACROS.iter().zip(&values) {
                if dragged != Some(id) {
                    node.set_value(id, value);
                }
            }
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
        // Pins of a folded section or node sit closer together than the hit radius, so more than
        // one can be in reach. The nearest one wins.
        let mut best: Option<(f32, Terminal)> = None;
        let mut consider = |at: Point, terminal: Terminal| {
            if near(at, point) {
                let (dx, dy) = (at.x - point.x, at.y - point.y);
                let distance = dx * dx + dy * dy;
                if best.is_none_or(|(closest, _)| distance < closest) {
                    best = Some((distance, terminal));
                }
            }
        };
        for (index, node) in self.nodes.iter().enumerate() {
            for port in node.output_ports() {
                if let Some(at) = node.output_point(port) {
                    consider(at, Terminal::Out(index, port));
                }
            }
            for port in node.input_ports() {
                if let Some(at) = node.input_point(port) {
                    consider(at, Terminal::In(index, port));
                }
            }
        }
        best.map(|(_, terminal)| terminal)
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
        let mut connections: Vec<Connection> = links
            .into_iter()
            .filter(|link| link.is_valid(&nodes))
            .collect();
        // Oscillators used to play every note. They now play the notes wired into them, so an
        // older save gets a MIDI input wired to each one, and keeps its glide and mono settings.
        if saved.version < SAVE_VERSION {
            migrate_notes(&mut nodes, &mut connections);
        }
        let mut graph = Self::with(nodes, connections);
        graph.preset_name = saved.preset_name.clone();
        graph.preset_category = saved.preset_category.clone();
        graph.refresh_slots();
        graph
    }

    pub fn to_saved(&self) -> SavedGraph {
        SavedGraph {
            version: SAVE_VERSION,
            preset_name: self.preset_name.clone(),
            preset_category: self.preset_category.clone(),
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

    /// True if a node of `kind` may be added. A kind that is one per graph, such as the output and
    /// the macro node, can't be added while the graph has one.
    pub(super) fn can_add(&self, kind: NodeKind) -> bool {
        !kind.is_unique() || !self.nodes.iter().any(|node| node.kind == kind)
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
