//! The graph model and its editing state: nodes, wires, and what the mouse is doing to them.

use crate::dsp::analyzer::{BINS, FLOOR_DB};
use crate::dsp::monitor::MonitorData;
use crate::dsp::sample::{is_audio_file, SampleData};
use crate::editor::monitor::MonitorHistory;
use crate::editor::piano::Piano;
use crate::engine::POOL;
use crate::groups::{self, GroupEntry};
use crate::nodes::connection::Connection;
use crate::nodes::eq;
use crate::nodes::frames;
use crate::nodes::knobs::{self, KnobId};
use crate::nodes::macros::MACRO_COUNT;
use crate::nodes::mix;
use crate::nodes::output::MAX_SLOTS;
use crate::nodes::wavetable::MAX_FRAMES;
use crate::nodes::{Category, FlodeNode, NodeKind, Port, Signal, HEADER_HEIGHT, NODE_WIDTH};
use crate::patch::control::{ControlLevels, Route};
use crate::patch::wiring::Wiring;
use crate::patch::Patch;
use crate::presets::{self, Library, Preset};
use nih_plug_iced::keyboard::KeyCode;
use nih_plug_iced::{Point, Rectangle, Size, Vector};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

use super::group::{FlatCache, OpenGroup};
use super::group_ui::{Rename, DEFAULT_CANVAS};
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
    /// Dragging a selection box over empty canvas, in world coordinates.
    Boxing {
        start: Point,
        cursor: Point,
    },
    /// Shift and right drag from a node. Releasing over another node wires the two together the
    /// way `Graph::auto_connect` picks. `start` and `cursor` are in world coordinates.
    AutoLinking {
        from: usize,
        start: Point,
        cursor: Point,
    },
    /// Dragging a wire from an output port. `cursor` is the current end of the wire.
    Wiring {
        from: usize,
        out: Port,
        cursor: Point,
    },
    /// Dragging a wire back from an input port that has no wire. Dropping it on an output port
    /// wires the two. Dropping it on empty canvas opens the quick add box for the input.
    BackWiring {
        to: usize,
        input: Port,
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
    /// Dragging one point of an EQ node's graph. `band` is 0 for the lowest.
    EqPoint {
        node: usize,
        band: usize,
    },
    /// Dragging one handle of an envelope's curve. `handle` is 0 for the attack, 1 for the decay
    /// and sustain, and 2 for the release.
    EnvelopeHandle {
        node: usize,
        handle: usize,
    },
    /// Dragging a handle on a wavetable's ramp. `boundary` is 0 for the handle between the first
    /// and second regions.
    RampHandle {
        node: usize,
        boundary: usize,
    },
    /// Dragging one point of an LFO's custom curve. `point` is 0 for the first.
    LfoPoint {
        node: usize,
        point: usize,
    },
    /// A modal grab, started by G in Edit mode: the selection follows the cursor with no button
    /// held. `node` and `grab` track the driving node the same way `MovingNode` does. `Graph`'s
    /// `grab_positions` holds where every selected node started, the same way `box_base` holds a
    /// box drag's starting selection, so Esc or a right click can put them back exactly.
    Grabbing {
        node: usize,
        grab: Vector,
    },
}

/// Which keys the computer keyboard sends to. Tab switches between them. Runtime-only, like
/// `shift` and `alt`: a preset does not remember which mode the editor was in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum EditorMode {
    /// The keyboard plays notes through the piano, on A to P, semicolon, Z, and X, as it always
    /// has.
    #[default]
    Keyboard,
    /// The keyboard is handed to the editor instead: G grabs the selection, X and Delete remove
    /// it, A selects everything, and Alt+A clears the selection. The piano does not run at all.
    Edit,
}

impl EditorMode {
    /// What Tab switches to.
    pub(super) fn toggled(self) -> Self {
        match self {
            EditorMode::Keyboard => EditorMode::Edit,
            EditorMode::Edit => EditorMode::Keyboard,
        }
    }

    /// The word the badge over the strip shows.
    pub(super) fn label(self) -> &'static str {
        match self {
            EditorMode::Keyboard => "Keyboard",
            EditorMode::Edit => "Edit",
        }
    }
}

/// A node easing from where it was to where it now belongs.
#[derive(Debug, Clone, Copy)]
pub(super) struct Slide {
    pub(super) node: usize,
    from: Point,
    pub(super) to: Point,
    start: Instant,
}

/// How long a node takes to slide into place.
const SLIDE_SECONDS: f32 = 0.18;
/// How far each file of a multi file drop sits from the one before, in editor units.
const DROP_STAGGER: f32 = 28.0;

/// Fast at first, then settling: 0.0 to 1.0 in, 0.0 to 1.0 out.
fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
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

/// `edits` without the ones for a macro whose every `Set` leaves it where the host already has it.
/// A preset load records a full drag for each of the four macros, and most presets leave them at
/// zero, so this keeps a load from writing host parameters it does not change. A macro with no
/// `Set` in `edits`, such as a drag that has only started, keeps its edits. `current` is the
/// host's value of a macro.
pub(super) fn without_unchanged_macros(
    edits: &[MacroEdit],
    current: impl Fn(usize) -> f32,
) -> Vec<MacroEdit> {
    let moves = |index: usize| {
        edits.iter().any(|edit| match edit {
            MacroEdit::Set(i, value) => *i == index && (value - current(index)).abs() > 1e-4,
            _ => false,
        })
    };
    let sets = |index: usize| {
        edits
            .iter()
            .any(|edit| matches!(edit, MacroEdit::Set(i, _) if *i == index))
    };
    edits
        .iter()
        .copied()
        .filter(|edit| {
            let index = match edit {
                MacroEdit::Begin(i) | MacroEdit::Set(i, _) | MacroEdit::End(i) => *i,
            };
            !sets(index) || moves(index)
        })
        .collect()
}

/// The box for typing a knob's value, opened by a double click on the knob. The text starts as the
/// knob's current value, selected, so the first key typed replaces it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct KnobEntry {
    pub(super) node: usize,
    pub(super) id: KnobId,
    pub(super) text: String,
    /// True until the text is edited. A box left alone changes nothing when it closes.
    pub(super) fresh: bool,
}

/// The longest number the box takes.
pub(super) const KNOB_ENTRY_MAX_LEN: usize = 12;
/// The size of the box, and so how much it covers around the knob.
pub(super) const KNOB_ENTRY_SIZE: (f32, f32) = (76.0, 22.0);

/// The open preset list: what is saved, and the category whose submenu is open, if one is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PresetMenu {
    pub(super) library: Library,
    /// The position in `library.categories` of the open submenu.
    pub(super) open: Option<usize>,
}

/// Where the save dialog starts and what file name it suggests. It starts in the preset folder
/// `dir`, or in the current preset's category folder inside it, and suggests the preset's name,
/// or "Untitled", with the preset extension.
pub(super) fn save_dialog_start(
    dir: &std::path::Path,
    category: &str,
    name: Option<&str>,
) -> (PathBuf, String) {
    let folder = if category.is_empty() {
        dir.to_path_buf()
    } else {
        dir.join(category)
    };
    let file = format!("{}.{}", name.unwrap_or("Untitled"), presets::EXTENSION);
    (folder, file)
}

/// What a save dialog is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SaveKind {
    Preset,
    Group,
}

impl SaveKind {
    /// The title, file filter name, and file extension the dialog shows.
    #[cfg(not(test))]
    fn dialog_text(self) -> (&'static str, &'static str, &'static str) {
        match self {
            SaveKind::Preset => ("Save preset", "Flode preset", presets::EXTENSION),
            SaveKind::Group => ("Save group", "Flode group", groups::EXTENSION),
        }
    }
}

/// A sample file decoding in the background: which node asked for it, and the receiving end of
/// the channel its decode thread sends the result through. The mutex only makes the editor
/// `Sync`, which the receiver is not.
pub(super) struct SamplerLoad {
    node: usize,
    receiver: Mutex<mpsc::Receiver<Result<Arc<SampleData>, String>>>,
}

/// Graph state that persists between frames.
pub struct Graph {
    pub(super) nodes: Vec<FlodeNode>,
    pub(super) connections: Vec<Connection>,
    pub(super) interaction: Interaction,
    pub(super) piano: Piano,
    /// The primary selected node: the last one picked. It is drawn on top and the shortcuts act
    /// on it.
    pub(super) selected: Option<usize>,
    /// The other nodes of a multiple selection. They move and delete with the primary one.
    pub(super) also_selected: Vec<usize>,
    /// Nodes easing into new places. Their positions move a little each frame.
    pub(super) slides: Vec<Slide>,
    /// The selection a box drag started from. A box with Shift held adds to it.
    pub(super) box_base: Vec<usize>,
    /// Where every selected node sat when a grab (G, in Edit mode) started, so Esc or a right
    /// click can put them back exactly. Empty outside a grab.
    pub(super) grab_positions: Vec<(usize, Point)>,
    /// True while Shift is held, as the last keyboard event reported it.
    pub(super) shift: bool,
    /// True while Alt (Option) is held, as the last keyboard event reported it. A click on a knob
    /// with it held puts the knob back to its default.
    pub(super) alt: bool,
    /// The box for typing a knob's value, while one is open.
    pub(super) knob_entry: Option<KnobEntry>,
    pub(super) menu: Option<ContextMenu>,
    /// The quick add box, while it is open.
    pub(super) quick_add: Option<QuickAdd>,
    /// The node whose dropdown list is open.
    pub(super) dropdown_menu: Option<(usize, usize)>,
    /// The audio connection a dragged node would be inserted into, if dropped now.
    pub(super) link_target: Option<usize>,
    /// The wavetable node and frame slot the cursor is linking: over a pin or over its region.
    pub(super) ramp_hover: Option<(usize, usize)>,
    /// The key on the strip the mouse holds down, and so the note it is playing.
    pub(super) mouse_note: Option<u8>,
    /// Whether the keyboard plays notes or sends editor shortcuts, toggled by Tab. The strip
    /// marks the keys it plays while this is Keyboard, in a plugin as well as standalone.
    pub(super) mode: EditorMode,
    /// How lit the MIDI indicator is, from 0.0 to 1.0. It jumps up when a note arrives and fades.
    pub(super) midi_light: f32,
    /// What each EQ instance of the audio thread last measured, by pool number. The view copies
    /// it in each frame.
    pub(super) spectra: [[f32; BINS]; POOL],
    /// What each monitor instance last measured, by pool number. Copied in each frame.
    pub(super) monitors: [MonitorData; POOL],
    /// The history of each monitor node that shows a spectrogram or level strip. Entries name
    /// nodes by index, so a delete renumbers them.
    pub(super) monitor_history: Vec<MonitorHistory>,
    /// True while the cursor is inside the editor. Hover highlights use it, so one does not stay
    /// lit after the cursor leaves.
    pub(super) cursor_in: bool,
    /// What the cursor is over, if it is over a dropdown or one of its open rows.
    pub(super) hover: Option<Hover>,
    /// True while the burger menu's top-level list is open.
    pub(super) burger_menu: bool,
    /// The category whose list is open, nested under the burger menu.
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
    /// The folder saved groups live in. None if the home folder is unknown. Tests start with none,
    /// so no test reads or writes the real folder.
    pub(super) group_dir: Option<PathBuf>,
    /// The open list of saved groups, read when it opened.
    pub(super) group_list: Option<Vec<GroupEntry>>,
    /// The group the open save dialog will write, as it was when "Save group" was chosen. It is
    /// None while a preset dialog is open.
    pub(super) group_save: Option<SavedGroupFile>,
    /// The answer of the save dialog, while one is open. It holds the chosen path, or None if the
    /// user canceled.
    /// The mutex only makes the editor `Sync`, which the receiver is not.
    pub(super) save_dialog: Option<Mutex<mpsc::Receiver<Option<PathBuf>>>>,
    /// The file the save dialog chose. The editor saves the preset to it, since the macro values
    /// are the host's.
    pub(super) pending_save: Option<PathBuf>,
    /// How many times a dialog was asked for. Tests read it, since they open no real dialog.
    #[cfg(test)]
    pub(super) save_dialog_requests: usize,
    /// The other end of `save_dialog` in tests, which send the answer through it.
    #[cfg(test)]
    pub(super) save_dialog_sender: Option<mpsc::Sender<Option<PathBuf>>>,
    /// A line shown under the burger menu for a few seconds, such as "Saved Lead".
    pub(super) status: Option<(String, Instant)>,
    /// The open file dialog for a Sampler's "Load sample..." button, and which node asked for it.
    /// The mutex only makes the editor `Sync`, which the receiver is not.
    pub(super) sampler_dialog: Option<Mutex<mpsc::Receiver<Option<PathBuf>>>>,
    pub(super) sampler_dialog_node: Option<usize>,
    /// The other end of `sampler_dialog` in tests, which send the answer through it.
    #[cfg(test)]
    pub(super) sampler_dialog_sender: Option<mpsc::Sender<Option<PathBuf>>>,
    /// Sample files decoding in the background, one per node still waiting on its load.
    pub(super) sampler_loads: Vec<SamplerLoad>,
    /// The last press on an LFO's viewer, to tell a double click that adds or removes a point.
    pub(super) viewer_click: Option<nih_plug_iced::mouse::Click>,
    /// The groups that are open, outermost first. While one is open, `nodes` and `connections`
    /// hold the graph inside the innermost one, and each entry holds the state of the graph
    /// outside it. Empty when the root graph is shown.
    pub(super) open_groups: Vec<OpenGroup>,
    /// The box that renames a group, a pin, or a macro knob, while one is open.
    pub(super) rename: Option<Rename>,
    /// Where a press landed on the dimmed margin around an open group's pane, until its release.
    /// A release that has not moved far closes the group. A drag does not.
    pub(super) margin_press: Option<Point>,
    /// The last press on a group node's header, to tell a double click that opens the group.
    pub(super) header_click: Option<nih_plug_iced::mouse::Click>,
    /// The size of the editor as the last event saw it. The pane of an open group, and the view
    /// that fits its graph, follow it.
    pub(super) canvas_size: Size,
    /// The graph compiled with its groups expanded, for queries about the shown graph. It is
    /// checked against a fingerprint of the graph on every read, so it needs no clearing. See
    /// `Graph::with_flat`. A mutex, and not a `RefCell`, so the editor stays `Sync`.
    pub(super) flat_cache: Mutex<Option<FlatCache>>,
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
/// shaping its amplitude and a MIDI input feeding it notes. The oscillator plays a saw wave, made
/// by a saw node wired to its table pin. The oscillator is node 0 and the output is node 1, the
/// envelope is node 2, and the MIDI input is node 3, with the saw node 4. The MIDI input is at
/// the left edge, with the envelope to its right and the saw node under the envelope.
impl Default for Graph {
    fn default() -> Self {
        let mut nodes = vec![
            FlodeNode::new(NodeKind::Oscillator, Point::new(DEFAULT_OSC_X, 80.0)),
            FlodeNode::new(NodeKind::Output, Point::new(DEFAULT_OUTPUT_X, 80.0)),
            FlodeNode::new(NodeKind::Envelope, Point::new(DEFAULT_ENVELOPE_X, 80.0)),
            FlodeNode::new(NodeKind::Input, Point::new(DEFAULT_INPUT_X, 80.0)),
            FlodeNode::new(NodeKind::SawWave, Point::new(0.0, 0.0)),
        ];
        // The oscillator's sound comes from a saw node. It sits under the envelope, next to the
        // oscillator it feeds.
        nodes[4].position = Point::new(DEFAULT_ENVELOPE_X, 80.0 + nodes[2].height() + 24.0);
        let mut graph = Self::with(
            nodes,
            vec![
                Connection {
                    from: 4,
                    out: Port::Table,
                    to: 0,
                    input: Port::TableIn,
                },
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
pub(super) fn show_slots(nodes: &mut [FlodeNode], connections: &[Connection]) {
    for (index, node) in nodes.iter_mut().enumerate() {
        // An output node's slots, or a wavetable node's frame inputs. Either way a pin shows for
        // each connected one and one empty pin below the last.
        let limit = match node.kind {
            NodeKind::Output => MAX_SLOTS,
            NodeKind::Wavetable => MAX_FRAMES,
            _ => continue,
        };
        let used = connections
            .iter()
            .filter(|c| c.to == index)
            .filter_map(|c| match c.input {
                Port::Slot(slot) | Port::Frame(slot) => Some(slot + 1),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        node.used = used.min(limit);
        node.slots = (node.used + 1).min(limit);
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
            also_selected: Vec::new(),
            box_base: Vec::new(),
            grab_positions: Vec::new(),
            slides: Vec::new(),
            shift: false,
            menu: None,
            dropdown_menu: None,
            link_target: None,
            ramp_hover: None,
            hover: None,
            mouse_note: None,
            mode: EditorMode::Keyboard,
            cursor_in: false,
            midi_light: 0.0,
            spectra: [[FLOOR_DB; BINS]; POOL],
            monitors: [MonitorData::default(); POOL],
            monitor_history: Vec::new(),
            burger_menu: false,
            category_menu: None,
            view: View::default(),
            last_screen: Point::new(0.0, 0.0),
            shortcut_held: None,
            macro_edits: Vec::new(),
            preset_dir: presets::directory(),
            preset_name: None,
            preset_category: String::new(),
            preset_list: None,
            group_dir: if cfg!(test) {
                None
            } else {
                groups::directory()
            },
            group_list: None,
            group_save: None,
            save_dialog: None,
            knob_entry: None,
            quick_add: None,
            alt: false,
            pending_save: None,
            #[cfg(test)]
            save_dialog_requests: 0,
            #[cfg(test)]
            save_dialog_sender: None,
            status: None,
            sampler_dialog: None,
            sampler_dialog_node: None,
            #[cfg(test)]
            sampler_dialog_sender: None,
            sampler_loads: Vec::new(),
            viewer_click: None,
            open_groups: Vec::new(),
            rename: None,
            margin_press: None,
            header_click: None,
            canvas_size: Size::new(DEFAULT_CANVAS.0, DEFAULT_CANVAS.1),
            flat_cache: Mutex::new(None),
        }
    }

    /// Shows a line under the burger menu for a few seconds.
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

    /// Flips a map's Bipolar switch. The values the new form shows are worked out from the old
    /// ones, so the signal does not change. The knobs that go off show take their wires with them,
    /// since a pin that is not there has nothing to end on.
    pub(super) fn toggle_map_polarity(&mut self, node: usize) {
        let on = !crate::nodes::map::bipolar(&self.nodes[node]);
        let Some(left) = crate::nodes::map::set_bipolar(&mut self.nodes[node], on) else {
            return;
        };
        self.connections
            .retain(|c| !(c.to == node && left.iter().any(|&id| c.input == Port::Knob(id))));
        self.refresh_slots();
    }

    /// Opens the box for typing knob `id` of `node`, with its value in the box and selected.
    pub(super) fn open_knob_entry(&mut self, node: usize, id: KnobId) {
        self.knob_entry = Some(KnobEntry {
            node,
            id,
            text: self.nodes[node].knob_entry_text(id),
            fresh: true,
        });
    }

    /// Where the box for typing a knob's value sits, in editor coordinates: centered on the knob.
    pub(super) fn knob_entry_bounds(&self) -> Option<Rectangle> {
        let entry = self.knob_entry.as_ref()?;
        let slot = self
            .nodes
            .get(entry.node)?
            .knob_slots()
            .into_iter()
            .find(|slot| slot.id == entry.id)?;
        let (width, height) = KNOB_ENTRY_SIZE;
        Some(Rectangle {
            x: slot.center.x - width / 2.0,
            y: slot.center.y - height / 2.0,
            width,
            height,
        })
    }

    /// Closes the box and sets the knob to the number typed in it. A number that does not read
    /// leaves the knob as it was. Returns true if the knob changed.
    pub(super) fn confirm_knob_entry(&mut self) -> bool {
        let Some(entry) = self.knob_entry.take() else {
            return false;
        };
        let Some(number) = knobs::parse_entry(&entry.text) else {
            self.say("Type a number");
            return false;
        };
        let (node, id) = (entry.node, entry.id);
        // A macro knob is a host parameter, so the host hears this as one gesture.
        self.macro_begin(node, id);
        self.nodes[node].enter_knob_value(id, number);
        self.macro_set(node, id);
        self.macro_end(node, id);
        true
    }

    /// Opens the quick add box with its corner at the cursor, and closes the menus it would
    /// overlap.
    pub(super) fn open_quick_add(&mut self) {
        self.open_quick_add_with(None, self.view.to_world(self.last_screen));
    }

    /// Opens the quick add box where a wire dragged back from input `input` of node `to` was
    /// dropped on empty canvas. The box lists the kinds that can feed that input, and the one
    /// picked is wired to it.
    pub(super) fn open_quick_add_feeding(&mut self, to: usize, input: Port, world: Point) {
        self.open_quick_add_with(Some((to, input)), world);
    }

    fn open_quick_add_with(&mut self, feed: Option<(usize, Port)>, world: Point) {
        self.menu = None;
        self.burger_menu = false;
        self.category_menu = None;
        self.preset_list = None;
        self.group_list = None;
        self.dropdown_menu = None;
        // The saved groups are read now, so one saved a moment ago is in the list. A drag back from
        // an input lists only the groups with an output pin that can feed it.
        let groups = self
            .read_groups()
            .into_iter()
            .filter_map(|entry| match feed {
                None => Some(QuickGroup { entry, out: None }),
                Some((to, input)) => {
                    let node = self.load_group(&entry).ok()?;
                    let out = self.feeding_port_of(&node, to, input)?;
                    Some(QuickGroup {
                        entry,
                        out: Some(out),
                    })
                }
            })
            .collect();
        self.quick_add = Some(QuickAdd {
            query: String::new(),
            selected: 0,
            screen: self.last_screen,
            world,
            feed,
            groups,
            skip_char: feed.is_none(),
        });
    }

    /// Opens the list of saved groups, reading the folder now.
    pub(super) fn open_group_list(&mut self) {
        self.group_list = Some(self.read_groups());
    }

    /// The saved groups in the group folder. None if the folder is missing or unknown.
    fn read_groups(&self) -> Vec<GroupEntry> {
        self.group_dir
            .as_deref()
            .map(groups::list_in)
            .unwrap_or_default()
    }

    /// Reads a saved group and builds its node. The error is a line for the status line.
    pub(super) fn load_group(&self, entry: &GroupEntry) -> Result<FlodeNode, String> {
        let dir = self.group_dir.as_deref().ok_or("no group folder")?;
        groups::load_in(dir, entry)
    }

    /// Adds a copy of a saved group to the graph on show, which is the open group if one is open,
    /// and returns its index. A file that can't be loaded adds nothing and says why.
    pub(super) fn add_saved_group(&mut self, entry: &GroupEntry, position: Point) -> Option<usize> {
        let mut node = match self.load_group(entry) {
            Ok(node) => node,
            Err(error) => {
                self.say(format!("Could not load {}: {error}", entry.name));
                return None;
            }
        };
        node.position = position;
        self.nodes.push(node);
        let index = self.nodes.len() - 1;
        self.select(index);
        Some(index)
    }

    /// The output pin of a new node of `kind` that can feed input `input` of node `to`, if it has
    /// one. Only pins that carry the same kind of signal can join.
    pub(super) fn feeding_port(&self, kind: NodeKind, to: usize, input: Port) -> Option<Port> {
        self.feeding_port_of(&FlodeNode::new(kind, Point::new(0.0, 0.0)), to, input)
    }

    /// `feeding_port` for a node that is already built, such as a saved group. It takes the first
    /// output pin that carries the input's signal.
    pub(super) fn feeding_port_of(
        &self,
        probe: &FlodeNode,
        to: usize,
        input: Port,
    ) -> Option<Port> {
        self.nodes.get(to)?.input_point(input)?;
        probe
            .output_ports()
            .into_iter()
            .find(|&out| out.signal() == input.signal() && probe.output_point(out).is_some())
    }

    /// What the quick add box lists for what is typed in it: node kinds, and saved groups. Kinds
    /// the graph can't take another of are left out, and so are the ones that can't feed the input
    /// the box was opened from.
    pub(super) fn quick_add_picks(&self) -> Vec<QuickPick> {
        match &self.quick_add {
            Some(open) => quick_add_picks(
                &open.query,
                |kind| {
                    self.can_add(kind)
                        && open
                            .feed
                            .is_none_or(|(to, input)| self.feeding_port(kind, to, input).is_some())
                },
                &open.groups,
            ),
            None => Vec::new(),
        }
    }

    /// The node kinds among `quick_add_picks`, for the tests that look at what the box offers.
    #[cfg(test)]
    pub(super) fn quick_add_results(&self) -> Vec<NodeKind> {
        self.quick_add_picks()
            .into_iter()
            .filter_map(|pick| match pick {
                QuickPick::Kind(kind) => Some(kind),
                QuickPick::Group(_) => None,
            })
            .collect()
    }

    /// Opens the system save dialog on a worker thread, so the editor keeps running while it is
    /// up. The answer arrives on `save_dialog` and `poll_save_dialog` picks it up. Does nothing
    /// while a dialog is already open.
    pub(super) fn start_save_dialog(&mut self) {
        self.preset_list = None;
        if self.save_dialog.is_some() {
            return;
        }
        let Some(dir) = self.preset_dir.clone() else {
            self.say("Could not save: no preset folder");
            return;
        };
        let (folder, file) =
            save_dialog_start(&dir, &self.preset_category, self.preset_name.as_deref());
        self.open_save_dialog(folder, file, SaveKind::Preset);
    }

    /// Opens the save dialog for group node `node` of the graph on show, starting in the group
    /// folder with the group's name. The group is copied now, so it is saved as it stands at this
    /// click, whatever is edited while the dialog is up. Does nothing while a dialog is open.
    pub(super) fn start_group_save_dialog(&mut self, node: usize) {
        if self.save_dialog.is_some() {
            return;
        }
        let Some(node) = self.nodes.get(node).filter(|n| n.kind == NodeKind::Group) else {
            return;
        };
        let name = node.label().into_owned();
        let copy = node.duplicate(node.position);
        self.start_group_save_of(&copy, &name);
    }

    /// `start_group_save_dialog` for a group node that is already built.
    pub(super) fn start_group_save_of(&mut self, node: &FlodeNode, name: &str) {
        if self.save_dialog.is_some() {
            return;
        }
        let Some(dir) = self.group_dir.clone() else {
            self.say("Could not save: no group folder");
            return;
        };
        self.group_save = Some(SavedGroupFile::new(node, name));
        // A slash in the name would read as a folder in the dialog.
        let file = format!("{}.{}", name.replace(['/', '\\'], "-"), groups::EXTENSION);
        self.open_save_dialog(dir, file, SaveKind::Group);
    }

    /// Opens the dialog in `folder` with `file` as the suggested name, for a save of `kind`.
    fn open_save_dialog(&mut self, folder: PathBuf, file: String, kind: SaveKind) {
        // The panel can only start in a folder that exists.
        let _ = std::fs::create_dir_all(&folder);
        let (sender, receiver) = mpsc::channel();
        // Tests never open a real dialog. They read the request and send the answer themselves.
        #[cfg(test)]
        {
            let _ = (&folder, &file, kind);
            self.save_dialog_requests += 1;
            self.save_dialog_sender = Some(sender);
        }
        #[cfg(not(test))]
        {
            let spawned = std::thread::Builder::new()
                .name("flode-save-dialog".into())
                .spawn(move || {
                    // rfd panics if the platform has no running application. The sender then
                    // drops without an answer, which the poll reports.
                    let picked = std::panic::catch_unwind(|| {
                        let (title, filter, extension) = kind.dialog_text();
                        rfd::FileDialog::new()
                            .set_title(title)
                            .set_directory(folder)
                            .set_file_name(file)
                            .add_filter(filter, &[extension])
                            .save_file()
                    });
                    if let Ok(picked) = picked {
                        let _ = sender.send(picked);
                    }
                });
            if spawned.is_err() {
                self.group_save = None;
                self.say("Could not open the save dialog");
                return;
            }
        }
        self.save_dialog = Some(Mutex::new(receiver));
    }

    /// Collects the save dialog's answer, if it has one. A chosen path for a preset waits in
    /// `pending_save` for the editor to save, since the macro values are the host's. A chosen path
    /// for a group is written here. A cancel does nothing, and a dialog that died is reported.
    pub(super) fn poll_save_dialog(&mut self) {
        let Some(receiver) = &self.save_dialog else {
            return;
        };
        let answer = match receiver.lock() {
            Ok(receiver) => receiver.try_recv(),
            Err(_) => Err(mpsc::TryRecvError::Disconnected),
        };
        match answer {
            Ok(Some(path)) => {
                self.save_dialog = None;
                match self.group_save.take() {
                    Some(file) => self.write_group(&path, file),
                    None => self.pending_save = Some(path),
                }
            }
            Ok(None) => {
                self.save_dialog = None;
                self.group_save = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.save_dialog = None;
                self.group_save = None;
                self.say("Could not open the save dialog");
            }
        }
    }

    /// Opens the native file picker for a Sampler's "Load sample..." button, on a worker thread,
    /// filtered to audio files. The answer arrives on `sampler_dialog` and `poll_sampler_dialog`
    /// picks it up. Does nothing while a dialog is already open.
    pub(super) fn start_sampler_dialog(&mut self, node: usize) {
        if self.sampler_dialog.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        #[cfg(test)]
        {
            self.sampler_dialog_sender = Some(sender);
        }
        #[cfg(not(test))]
        {
            let spawned = std::thread::Builder::new()
                .name("flode-sampler-dialog".into())
                .spawn(move || {
                    let picked = std::panic::catch_unwind(|| {
                        rfd::FileDialog::new()
                            .set_title("Load sample")
                            .add_filter("Audio", &crate::dsp::sample::EXTENSIONS)
                            .pick_file()
                    });
                    if let Ok(picked) = picked {
                        let _ = sender.send(picked);
                    }
                });
            if spawned.is_err() {
                self.say("Could not open the file dialog");
                return;
            }
        }
        self.sampler_dialog = Some(Mutex::new(receiver));
        self.sampler_dialog_node = Some(node);
    }

    /// Collects the file picker's answer, if it has one, and starts decoding the chosen file. A
    /// cancel does nothing, and a dialog that died is reported.
    pub(super) fn poll_sampler_dialog(&mut self) {
        let Some(receiver) = &self.sampler_dialog else {
            return;
        };
        let answer = match receiver.lock() {
            Ok(receiver) => receiver.try_recv(),
            Err(_) => Err(mpsc::TryRecvError::Disconnected),
        };
        match answer {
            Ok(Some(path)) => {
                self.sampler_dialog = None;
                if let Some(node) = self.sampler_dialog_node.take() {
                    self.start_sample_load(node, path);
                }
            }
            Ok(None) => {
                self.sampler_dialog = None;
                self.sampler_dialog_node = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.sampler_dialog = None;
                self.sampler_dialog_node = None;
                self.say("Could not open the file dialog");
            }
        }
    }

    /// Adds a Sampler for an audio file dropped on the editor at `screen`, centered there, and
    /// starts loading the file into it. That makes a drop a one step way to a playable Sampler.
    /// Several files dropped together land one after the other, each offset from the last so none
    /// hides another. A file that isn't audio is told so on the status line and adds nothing.
    /// Returns whether a node was added.
    pub(super) fn drop_sample(&mut self, path: PathBuf, screen: Point) -> bool {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        if !is_audio_file(&path) {
            self.say(format!("{name} is not an audio file flode can load"));
            return false;
        }
        if !self.can_add(NodeKind::Sampler) {
            return false;
        }
        let mut point = self.view.to_world(screen);
        point.x -= NODE_WIDTH / 2.0;
        // A node already at this spot is from the same drop. Step down and right off it.
        while self.nodes.iter().any(|node| {
            (node.position.x - point.x).abs() < 1.0 && (node.position.y - point.y).abs() < 1.0
        }) {
            point.x += DROP_STAGGER;
            point.y += DROP_STAGGER;
        }
        let node = self.add_node(NodeKind::Sampler, point);
        self.start_sample_load(node, path);
        true
    }

    /// Starts decoding `path` for Sampler node `node` on a background thread, so a large file's
    /// decode never freezes the editor. Sets the node's path right away, so a save made while the
    /// decode is still running remembers it, and clears any sample the node had, so the viewer
    /// shows nothing loaded until the decode finishes.
    pub(super) fn start_sample_load(&mut self, node: usize, path: PathBuf) {
        let Some(target) = self.nodes.get_mut(node) else {
            return;
        };
        target.sample_path = Some(path.clone());
        target.sample = None;
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("flode-sample-load".into())
            .spawn(move || {
                let result = SampleData::load(&path).map(Arc::new);
                let _ = sender.send(result);
            });
        if spawned.is_err() {
            self.say("Could not load the sample: no thread available");
            return;
        }
        self.sampler_loads.push(SamplerLoad {
            node,
            receiver: Mutex::new(receiver),
        });
    }

    /// Collects the answers of any sample decodes that have finished, the same way
    /// `poll_save_dialog` collects a save dialog's answer. A decode that succeeds sets the node's
    /// `sample`. One that fails, including a missing file, leaves the node sample-less and reports
    /// the failure on the status line, naming the file. Either way the rest of the patch keeps
    /// playing. Returns true if a decode finished and set a sample, so the caller can send the
    /// patch to the audio thread again.
    pub(super) fn poll_sampler_loads(&mut self) -> bool {
        let mut loaded = false;
        let mut done = Vec::new();
        for (slot, load) in self.sampler_loads.iter().enumerate() {
            let answer = match load.receiver.lock() {
                Ok(receiver) => receiver.try_recv(),
                Err(_) => Err(mpsc::TryRecvError::Disconnected),
            };
            match answer {
                Ok(result) => done.push((slot, load.node, Some(result))),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => done.push((slot, load.node, None)),
            }
        }
        for &(_, node, ref result) in &done {
            let path = self
                .nodes
                .get(node)
                .and_then(|n| n.sample_path.clone())
                .unwrap_or_default();
            match result {
                Some(Ok(data)) => {
                    let mut frames = None;
                    if let Some(n) = self.nodes.get_mut(node) {
                        n.sample = Some(data.clone());
                        loaded = true;
                        // A file node says how many frames its file will make.
                        if n.kind == NodeKind::WaveFile {
                            frames = frames::frame_count(n);
                        }
                    }
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    match frames {
                        Some(1) => self.say(format!("Loaded {name}: 1 frame")),
                        Some(count) => self.say(format!("Loaded {name}: {count} frames")),
                        None => self.say(format!("Loaded {name}")),
                    }
                }
                Some(Err(error)) => {
                    let title = self.nodes.get(node).map(|n| n.title()).unwrap_or("Sampler");
                    self.say(format!(
                        "Could not load sample for {title} ({}): {error}",
                        path.display()
                    ));
                }
                None => {
                    self.say("Could not load the sample: the loader thread stopped");
                }
            }
        }
        for &(slot, ..) in done.iter().rev() {
            self.sampler_loads.remove(slot);
        }
        loaded
    }

    /// Writes a saved group to the file the dialog chose. The file's name is the group's name in
    /// the file. The group in the patch keeps the name it has.
    fn write_group(&mut self, chosen: &std::path::Path, mut file: SavedGroupFile) {
        let path = groups::with_extension(chosen);
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("Group")
            .to_string();
        file.rename(&name);
        match groups::save_to(&path, &file) {
            Ok(()) => self.say(format!("Saved group {name}")),
            Err(error) => self.say(format!("Could not save: {error}")),
        }
    }

    /// Saves the preset to the file the dialog chose, with the macro knobs' current values. The
    /// name is the file's name and the category is the folder it is in, if that is one level
    /// inside the preset folder. Does nothing if no file is waiting.
    pub(super) fn save_pending_preset(&mut self, macros: [f32; MACRO_COUNT]) {
        let Some(chosen) = self.pending_save.take() else {
            return;
        };
        let Some(dir) = self.preset_dir.clone() else {
            self.say("Could not save: no preset folder");
            return;
        };
        let path = presets::with_extension(&chosen);
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("Untitled")
            .to_string();
        let preset = Preset {
            name: name.clone(),
            category: String::new(),
            graph: self.to_saved(),
            macros,
        };
        if let Err(error) = presets::save_to(&path, &preset) {
            self.say(format!("Could not save: {error}"));
            return;
        }
        // The panel may hand back a path through a link, such as /var for /private/var, so both
        // are compared as the file system resolves them.
        let resolved = |path: &std::path::Path| path.canonicalize().unwrap_or(path.to_path_buf());
        self.preset_category = presets::category_of(&resolved(&dir), &resolved(&path));
        self.preset_name = Some(name.clone());
        self.say(format!("Saved {name}"));
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
        // A load while a group is open leaves the group, so the root graph's view is the one to
        // keep.
        let view = self.open_groups.first().map_or(self.view, |open| open.view);
        let piano = std::mem::take(&mut self.piano);
        let mode = self.mode;
        let shift = self.shift;
        let dir = self.preset_dir.clone();
        let group_dir = self.group_dir.clone();
        let canvas_size = self.canvas_size;
        // A dialog that is open still answers after a load, and a second one must not open. A
        // group dialog still saves the group it was opened for.
        let dialog = self.save_dialog.take();
        let group_save = self.group_save.take();
        #[cfg(test)]
        let (requests, sender) = (self.save_dialog_requests, self.save_dialog_sender.take());
        *self = fresh;
        self.save_dialog = dialog;
        self.group_save = group_save;
        self.group_dir = group_dir;
        #[cfg(test)]
        {
            self.save_dialog_requests = requests;
            self.save_dialog_sender = sender;
        }
        self.view = view;
        self.piano = piano;
        self.mode = mode;
        self.shift = shift;
        self.preset_dir = dir;
        self.canvas_size = canvas_size;
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
        // Every edit of a knob comes through here, so it is also where a knob on the input edge
        // node of a group carries its new value to the knobs it feeds.
        self.push_edge_values(node);
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
        // The macro node can be in the graph outside an open group, which still shows its knobs
        // when the group closes.
        let outside = self
            .open_groups
            .iter_mut()
            .flat_map(|open| open.nodes.iter_mut());
        for node in self
            .nodes
            .iter_mut()
            .chain(outside)
            .filter(|n| n.kind == NodeKind::Macro)
        {
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
                // A row that can't be used does not light, but it still covers what is under it.
                if let Some(row) = list_row_at(list, point, rows) {
                    if self.dropdown_row_disabled(index, which, row) {
                        return None;
                    }
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

    /// The wires of the graph on show, as the compiler reads them. It does not look through
    /// groups. The queries below do, and so does `patch`.
    pub fn wiring(&self) -> Wiring<'_> {
        Wiring::new(&self.nodes, &self.connections)
    }

    /// True if row `row` of dropdown `which` on `node` can't be used right now, so the list shows
    /// it dimmed and a press on it does nothing. FM mix needs each voice's pitch, so a mix that
    /// runs on the bus, with an effect upstream, can't use it.
    pub(super) fn dropdown_row_disabled(&self, node: usize, which: usize, row: usize) -> bool {
        self.nodes[node].kind == NodeKind::Mix
            && which == 0
            && row == mix::FM_ROW
            && self
                .with_flat(node, |wiring, node| wiring.runs_on_bus(node))
                .unwrap_or(false)
    }

    /// True if a control pin of the open group's input edge feeds knob `id` of `node`.
    fn fed_by_pin(&self, node: usize, id: KnobId) -> bool {
        !self.open_groups.is_empty()
            && self.connections.iter().any(|c| {
                c.to == node
                    && c.input == Port::Knob(id)
                    && matches!(c.out, Port::Pin(Signal::Control, _))
                    && self
                        .nodes
                        .get(c.from)
                        .is_some_and(|n| n.kind == NodeKind::GroupInput)
            })
    }

    /// True if a control wire drives this knob. A driven knob can't be dragged.
    pub(super) fn is_driven(&self, node: usize, id: KnobId) -> bool {
        // A knob on a group's face is driven by a wire into its pin.
        if let Some(pin) = self.nodes[node].face_pin(id) {
            let wired = |links: &[Connection], group: usize| {
                links
                    .iter()
                    .any(|c| c.to == group && matches!(c.input, Port::Pin(_, n) if n == pin))
            };
            // The same knob on the input edge node inside is locked by a wire into the group
            // node in the graph outside.
            if self.nodes[node].kind == NodeKind::GroupInput {
                return self
                    .open_groups
                    .last()
                    .is_some_and(|open| wired(&open.connections, open.group));
            }
            return wired(&self.connections, node);
        }
        // Inside an open group, a pin of the group's input edge rules the knobs it feeds, so
        // they are locked to the value on the group's face.
        if self.fed_by_pin(node, id) {
            return true;
        }
        self.with_flat(node, |wiring, node| wiring.is_driven(node, id))
            .unwrap_or(false)
    }

    /// The spectrum an EQ node's analyzer shows, or None while the analyzer is off or the node
    /// plays in no slot. It reads the instance the compiler gave the node.
    pub(super) fn eq_spectrum(&self, node: usize) -> Option<&[f32; BINS]> {
        if !eq::analyzer(&self.nodes[node]) {
            return None;
        }
        let pool = self.with_flat(node, |wiring, node| {
            wiring
                .eq_pools()
                .into_iter()
                .find(|(found, _)| *found == node)
        })??;
        self.spectra.get(usize::from(pool.1))
    }

    /// The control route that drives a knob, if one does.
    pub fn control_route(&self, node: usize, id: KnobId) -> Option<Route> {
        self.with_flat(node, |wiring, node| wiring.control_route(node, id))?
    }

    /// The level arriving at a map's input, if a control source feeds it.
    pub(super) fn map_input_level(&self, node: usize, levels: &ControlLevels) -> Option<f32> {
        self.with_flat(node, |wiring, node| wiring.map_input_level(node, levels))?
    }

    /// The place of an LFO node among the LFOs the audio thread runs, or None if it is past them.
    pub(super) fn lfo_index(&self, node: usize) -> Option<usize> {
        self.with_flat(node, |wiring, node| wiring.lfo_index(node))?
    }

    /// The place of an envelope node among the envelopes the audio thread runs, or None if it is
    /// past them.
    pub(super) fn envelope_index(&self, node: usize) -> Option<usize> {
        self.with_flat(node, |wiring, node| wiring.envelope_index(node))?
    }

    /// The place of a Sampler node among the Samplers the engine tracks a playhead for, or None
    /// if it is past them.
    pub(super) fn sampler_index(&self, node: usize) -> Option<usize> {
        self.with_flat(node, |wiring, node| wiring.sampler_index(node))?
    }

    /// The place of a MIDI input or arpeggiator node among the note sources the audio thread runs,
    /// or None if it is past them.
    pub(super) fn note_source_index(&self, node: usize) -> Option<usize> {
        self.with_flat(node, |wiring, node| wiring.note_source_index(node))?
    }

    /// Which engine instance each monitor node on show runs on, as (node, pool number) pairs.
    pub(super) fn monitor_pools(&self) -> Vec<(usize, u8)> {
        if !self.has_groups() {
            return self.wiring().monitor_pools();
        }
        self.with_flat_cache(|cache| {
            let wiring = Wiring::new(&cache.flat.nodes, &cache.flat.links);
            wiring
                .monitor_pools()
                .into_iter()
                .filter_map(|(flat, pool)| {
                    let shown = cache.shown.iter().position(|&found| found == Some(flat))?;
                    Some((shown, pool))
                })
                .collect()
        })
    }

    /// Compiles the whole graph, with every group expanded in place, into the patch the audio
    /// thread plays. While a group is open this still compiles the graph from its root, with the
    /// open group's edits in it.
    pub fn patch(&self) -> Patch {
        if !self.has_groups() {
            return self.wiring().patch();
        }
        let (nodes, links) = self.root_snapshot();
        let flat = crate::patch::flatten::flatten(&nodes, &links);
        Wiring::new(&flat.nodes, &flat.links).patch()
    }

    /// Rebuilds the graph from saved state. Empty state gives the default graph, and connections
    /// that no longer fit the node kinds are dropped.
    pub fn from_saved(saved: &SavedGraph) -> Self {
        if saved.nodes.is_empty() {
            return Self::default();
        }
        let mut nodes: Vec<FlodeNode> = saved.nodes.iter().map(SavedNode::into_node).collect();
        if saved.version < OCTAVE_SPAN_VERSION {
            migrate_octave_span(&mut nodes);
        }
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
        if saved.version < NOTES_VERSION {
            migrate_notes(&mut nodes, &mut connections);
        }
        let mut graph = Self::with(nodes, connections);
        graph.preset_name = saved.preset_name.clone();
        graph.preset_category = saved.preset_category.clone();
        graph.refresh_slots();
        // Every Sampler that remembers a file starts loading it again right away, so the sound is
        // ready without the user re-browsing for it. A missing or broken file leaves the node
        // sample-less and reports why, without blocking anything else in the patch.
        for index in 0..graph.nodes.len() {
            if matches!(
                graph.nodes[index].kind,
                NodeKind::Sampler | NodeKind::WaveFile
            ) {
                if let Some(path) = graph.nodes[index].sample_path.clone() {
                    graph.start_sample_load(index, path);
                }
            }
        }
        graph
    }

    /// The graph as it saves. This is always the root graph, with the edits of any open group in
    /// it, so a save made inside a group loses nothing and reopens at the root.
    pub fn to_saved(&self) -> SavedGraph {
        let save = |nodes: &[FlodeNode], links: &[Connection]| SavedGraph {
            version: SAVE_VERSION,
            preset_name: self.preset_name.clone(),
            preset_category: self.preset_category.clone(),
            nodes: nodes.iter().map(SavedNode::new).collect(),
            connections: Vec::new(),
            links: links.to_vec(),
        };
        if self.open_groups.is_empty() {
            return save(&self.nodes, &self.connections);
        }
        let (nodes, links) = self.root_snapshot();
        save(&nodes, &links)
    }

    /// Removes the connection on an input port and returns it.
    pub(super) fn disconnect_input(&mut self, to: usize, input: Port) -> Option<Connection> {
        let index = self
            .connections
            .iter()
            .position(|c| c.to == to && c.input == input)?;
        let link = self.connections.remove(index);
        self.refresh_slots();
        self.refresh_edge_knobs();
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
        // map or a math node, splices into a wire that drives a knob. A node with a notes input
        // and output, such as an arpeggiator, splices into a notes wire the same way.
        let audio_node = behavior.audio_in() && behavior.audio_out();
        let control_node = Self::control_splice_input(dragged).is_some()
            && dragged.output_ports().contains(&Port::Shape);
        let notes_node = dragged.input_ports().contains(&Port::Notes)
            && dragged.output_ports().contains(&Port::Notes);
        if !(audio_node || control_node || notes_node) {
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
                c.out.is_audio_out() && c.input.is_audio()
            } else if control_node {
                c.out.is_control_out() && matches!(c.input, Port::Knob(_))
            } else {
                c.out.signal() == Signal::Notes && c.input.signal() == Signal::Notes
            };
            fits && self.link_curve(c).is_some_and(|curve| {
                (0..=WIRE_SAMPLES)
                    .any(|i| bounds.contains(bezier_point(&curve, i as f32 / WIRE_SAMPLES as f32)))
            })
        })
    }

    /// The input pin that a control wire spliced into `node` ends on: a map's shape pin, or a math
    /// node's A. None for a node that takes no control signal this way.
    fn control_splice_input(node: &FlodeNode) -> Option<Port> {
        if node.input_ports().contains(&Port::Shape) {
            Some(Port::Shape)
        } else if node.kind == NodeKind::Math {
            Some(Port::Knob(KnobId::MathA))
        } else {
            None
        }
    }

    /// Splices `node` into the connection `link`: the old source feeds the node, and the node
    /// feeds the old target. Control wires are spliced through the node's shape pins, or into A on
    /// a math node. An audio wire goes into the node's first audio input, which is the top one on a
    /// node with several, such as signal A on a mix. A notes wire goes into the node's notes pin.
    pub(super) fn insert_into(&mut self, node: usize, link: usize) {
        let old = self.connections[link];
        let (input, out) = if old.out.signal() == Signal::Notes {
            (Port::Notes, Port::Notes)
        } else if old.out.is_control_out() {
            (
                Self::control_splice_input(&self.nodes[node]).unwrap_or(Port::Shape),
                Port::Shape,
            )
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
        self.make_room(node, old.from, old.to);
    }

    /// The space left on each side of a node spliced between two others.
    const SPLICE_MARGIN: f32 = 40.0;

    /// Spaces out `from` and `to` so `node`, just spliced between them, sits in the gap with a
    /// margin on each side. If the gap is too small, `to` and every node it feeds slide right by
    /// the shortfall, and `node` slides into the gap if it is outside it. A wire that loops back
    /// to `from` has no right side to push, so nothing moves.
    fn make_room(&mut self, node: usize, from: usize, to: usize) {
        let margin = Self::SPLICE_MARGIN;
        let downstream = self.downstream(to);
        if from == to || downstream.contains(&from) || downstream.contains(&node) {
            return;
        }
        let left_edge = self.nodes[from].bounds();
        let right_edge = self.nodes[to].bounds();
        let width = self.nodes[node].bounds().width;
        let gap = right_edge.x - (left_edge.x + left_edge.width);
        let shortfall = (width + 2.0 * margin - gap).max(0.0);
        if shortfall > 0.0 {
            for &moved in &downstream {
                let at = self.target_of(moved);
                self.slide_to(moved, Point::new(at.x + shortfall, at.y));
            }
        }
        let low = left_edge.x + left_edge.width + margin;
        let high = right_edge.x + shortfall - margin - width;
        let at = self.nodes[node].position;
        let x = at.x.clamp(low, high.max(low));
        self.slide_to(node, Point::new(x, at.y));
    }

    /// `start` and every node its wires lead on to, following them from output to input.
    fn downstream(&self, start: usize) -> Vec<usize> {
        let mut reached = vec![start];
        let mut next = 0;
        while next < reached.len() {
            let at = reached[next];
            next += 1;
            for c in self.connections.iter().filter(|c| c.from == at) {
                if !reached.contains(&c.to) {
                    reached.push(c.to);
                }
            }
        }
        reached
    }

    /// Where `node` is, or where it is sliding to.
    fn target_of(&self, node: usize) -> Point {
        self.slides
            .iter()
            .find(|slide| slide.node == node)
            .map_or(self.nodes[node].position, |slide| slide.to)
    }

    /// Starts `node` easing toward `to`, from where it is now.
    pub(super) fn slide_to(&mut self, node: usize, to: Point) {
        self.slides.retain(|slide| slide.node != node);
        let from = self.nodes[node].position;
        if from != to {
            self.slides.push(Slide {
                node,
                from,
                to,
                start: Instant::now(),
            });
        }
    }

    /// Moves every sliding node along its path to the time `now`. Returns true if one reached its
    /// place, which is a change the editor has to save.
    pub(super) fn advance_slides(&mut self, now: Instant) -> bool {
        let mut arrived = false;
        let nodes = &mut self.nodes;
        self.slides.retain(|slide| {
            let t = now.saturating_duration_since(slide.start).as_secs_f32() / SLIDE_SECONDS;
            let done = t >= 1.0;
            let k = if done { 1.0 } else { ease_out(t) };
            nodes[slide.node].position = Point::new(
                slide.from.x + (slide.to.x - slide.from.x) * k,
                slide.from.y + (slide.to.y - slide.from.y) * k,
            );
            arrived |= done;
            !done
        });
        arrived
    }

    /// Puts every sliding node at its place at once.
    pub(super) fn finish_slides(&mut self) {
        for slide in std::mem::take(&mut self.slides) {
            self.nodes[slide.node].position = slide.to;
        }
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
        // A wavetable's frame input starts with an equal share, so a slot used before never comes
        // back tiny.
        if let (Port::Frame(slot), NodeKind::Wavetable) = (input, self.nodes[to].kind) {
            self.nodes[to].reset_value(KnobId::share(slot));
        }
        self.refresh_slots();
        self.refresh_edge_knobs();
        self.take_face_value(link);
        self.name_pin_after_wire(link);
    }

    /// Carries the value of every knob on the input edge node `node` of the open group to the
    /// knobs inside that its pins feed, and to the group node's face in the graph outside. Run
    /// after a knob on the edge node changes.
    pub(super) fn push_edge_values(&mut self, node: usize) {
        if self.nodes.get(node).map(|n| n.kind) != Some(NodeKind::GroupInput) {
            return;
        }
        let knobs = self.nodes[node].face_knobs();
        for (pin, _) in knobs {
            let value = self.nodes[node].outputs[pin].value;
            let targets: Vec<(usize, KnobId)> = self
                .connections
                .iter()
                .filter(|c| c.from == node && matches!(c.out, Port::Pin(_, n) if n == pin))
                .filter_map(|c| match c.input {
                    Port::Knob(id) => Some((c.to, id)),
                    _ => None,
                })
                .collect();
            for (to, id) in targets {
                self.nodes[to].set_value(id, value);
            }
            if let Some(open) = self.open_groups.last_mut() {
                if let Some(face) = open.nodes[open.group].inputs.get_mut(pin) {
                    face.value = value;
                }
            }
        }
    }

    /// Makes the knobs on the input edge node follow the wires inside the open group. A pin shows
    /// a knob while it feeds one, and a pin that starts to feed one takes that knob's value. The
    /// group node's face gets the same, so the two agree.
    pub(super) fn refresh_edge_knobs(&mut self) {
        let Some(edge) = self
            .nodes
            .iter()
            .position(|n| n.kind == NodeKind::GroupInput)
        else {
            return;
        };
        if self.open_groups.is_empty() {
            return;
        }
        for pin in 0..self.nodes[edge].outputs.len() {
            let first = self
                .connections
                .iter()
                .filter(|c| c.from == edge && matches!(c.out, Port::Pin(_, n) if n == pin))
                .find_map(|c| match c.input {
                    Port::Knob(id) => Some((id, self.nodes.get(c.to)?.value(id))),
                    _ => None,
                });
            let signal = self.nodes[edge].outputs[pin].signal;
            let target = first.filter(|_| signal == Signal::Control);
            let current = &mut self.nodes[edge].outputs[pin];
            if current.knob.is_none() {
                if let Some((_, inside)) = target {
                    current.value = inside;
                }
            }
            current.knob = target.map(|(id, _)| id);
            let copy = current.clone();
            if let Some(open) = self.open_groups.last_mut() {
                if let Some(face) = open.nodes[open.group].inputs.get_mut(pin) {
                    *face = copy;
                }
            }
        }
    }

    /// A knob that a wire from an input pin of the open group now feeds takes the value on the
    /// group's face, if that pin already stands for a knob, since the face rules it from now on.
    fn take_face_value(&mut self, link: Connection) {
        let (Port::Pin(Signal::Control, pin), Port::Knob(id)) = (link.out, link.input) else {
            return;
        };
        if self.nodes[link.from].kind != NodeKind::GroupInput {
            return;
        }
        let Some(open) = self.open_groups.last() else {
            return;
        };
        let face = open.nodes[open.group].inputs.get(pin);
        if let Some(value) = face.filter(|p| p.knob.is_some()).map(|p| p.value) {
            self.nodes[link.to].set_value(id, value);
        }
    }

    /// The name an output pin goes by, if it has one.
    fn out_name(port: Port) -> Option<&'static str> {
        port.output_label()
    }

    /// The name an input pin goes by, if it has one. A knob's pin is named for the knob.
    fn in_name(port: Port) -> Option<&'static str> {
        match port {
            Port::Knob(id) => Some(id.name()),
            other => other.input_label(),
        }
    }

    /// The wire from `from` to `to` that makes the most sense, or None if no pins of the two can
    /// join. Only pins that carry the same kind of signal can join. Among those, a wire between
    /// pins with the same name wins, such as a Shape output and a Shape input. Otherwise the
    /// first pair, top to bottom, whose input has no wire yet wins, so audio goes to the first
    /// free audio input. Otherwise the first pair. A wire that is already there is not offered
    /// again.
    pub(super) fn best_link(&self, from: usize, to: usize) -> Option<Connection> {
        let source = self.nodes.get(from)?;
        let target = self.nodes.get(to)?;
        // An oscillator's audio pin is for phase modulation, which has to be wired on purpose. An
        // automatic wire never picks it, or joining a chain from its far end would feed the
        // chain back into its own source.
        let inputs: Vec<Port> = target
            .input_ports()
            .into_iter()
            .filter(|port| !(target.kind == NodeKind::Oscillator && port.is_audio()))
            .collect();
        let links: Vec<Connection> = source
            .output_ports()
            .into_iter()
            .flat_map(|out| {
                inputs.iter().map(move |&input| Connection {
                    from,
                    out,
                    to,
                    input,
                })
            })
            .filter(|link| link.is_valid(&self.nodes) && !self.connections.contains(link))
            .collect();
        let named = |link: &&Connection| {
            matches!(
                (Self::out_name(link.out), Self::in_name(link.input)),
                (Some(a), Some(b)) if a.eq_ignore_ascii_case(b)
            )
        };
        let free = |link: &&Connection| {
            !self
                .connections
                .iter()
                .any(|c| c.to == to && c.input == link.input)
        };
        links
            .iter()
            .find(named)
            .or_else(|| links.iter().find(free))
            .or_else(|| links.first())
            .copied()
    }

    /// Wires `from` to `to` with `best_link`. If nothing joins that way, tries the wire the other
    /// way round, so dragging from the later node of a chain still connects. Returns whether a
    /// wire was made.
    pub(super) fn auto_connect(&mut self, from: usize, to: usize) -> bool {
        let Some(link) = self
            .best_link(from, to)
            .or_else(|| self.best_link(to, from))
        else {
            return false;
        };
        self.connect(link.from, link.out, link.to, link.input);
        true
    }

    /// True if a node of `kind` may be added. A kind that is one per graph, such as the output and
    /// the macro node, can't be added while the graph has one, and can't be added inside a group,
    /// since the root graph has it. The group kinds are made by the group commands only.
    pub(super) fn can_add(&self, kind: NodeKind) -> bool {
        if kind.is_group_part() {
            return false;
        }
        !kind.is_unique()
            || (self.open_groups.is_empty() && !self.nodes.iter().any(|node| node.kind == kind))
    }

    /// Adds a node and selects it. Returns its index.
    pub(super) fn add_node(&mut self, kind: NodeKind, position: Point) -> usize {
        self.nodes.push(FlodeNode::new(kind, position));
        let index = self.nodes.len() - 1;
        self.select(index);
        index
    }

    /// Selects `index` alone.
    pub(super) fn select(&mut self, index: usize) {
        self.selected = Some(index);
        self.also_selected.clear();
    }

    /// Every selected node, the primary one last.
    pub(super) fn selection(&self) -> Vec<usize> {
        let mut nodes = self.also_selected.clone();
        nodes.extend(self.selected);
        nodes
    }

    pub(super) fn is_selected(&self, index: usize) -> bool {
        self.selected == Some(index) || self.also_selected.contains(&index)
    }

    pub(super) fn clear_selection(&mut self) {
        self.selected = None;
        self.also_selected.clear();
    }

    /// Selects every node on show. The last one is primary, same as a box drawn over all of them.
    pub(super) fn select_all(&mut self) {
        self.set_selection((0..self.nodes.len()).collect());
    }

    /// Clears the selection. A plain name for `clear_selection`, for the Edit-mode shortcut.
    pub(super) fn deselect_all(&mut self) {
        self.clear_selection();
    }

    /// Selects exactly `nodes`. The last one becomes the primary node.
    pub(super) fn set_selection(&mut self, mut nodes: Vec<usize>) {
        self.selected = nodes.pop();
        self.also_selected = nodes;
    }

    /// Adds an unselected node to the selection, or drops a selected one from it.
    pub(super) fn toggle_selected(&mut self, index: usize) {
        if !self.is_selected(index) {
            self.also_selected.extend(self.selected);
            self.selected = Some(index);
            return;
        }
        let mut nodes = self.selection();
        nodes.retain(|&node| node != index);
        self.set_selection(nodes);
    }

    /// The nodes a menu entry on `node` acts on: the whole selection if `node` is in it, else
    /// just `node`.
    pub(super) fn menu_targets(&self, node: usize) -> Vec<usize> {
        if self.is_selected(node) {
            self.selection()
        } else {
            vec![node]
        }
    }

    /// Moves every selected node by `delta`.
    pub(super) fn move_selection(&mut self, delta: Vector) {
        for index in self.selection() {
            let position = &mut self.nodes[index].position;
            *position = Point::new(position.x + delta.x, position.y + delta.y);
        }
    }

    /// Selects the nodes the box between `a` and `b` touches, on top of the selection the box
    /// started from.
    pub(super) fn select_in_box(&mut self, a: Point, b: Point) {
        let (left, right) = (a.x.min(b.x), a.x.max(b.x));
        let (top, bottom) = (a.y.min(b.y), a.y.max(b.y));
        let mut nodes = self.box_base.clone();
        for (index, node) in self.nodes.iter().enumerate() {
            let rect = node.bounds();
            let touches = rect.x <= right
                && rect.x + rect.width >= left
                && rect.y <= bottom
                && rect.y + rect.height >= top;
            if !nodes.contains(&index) && touches {
                nodes.push(index);
            }
        }
        self.set_selection(nodes);
    }

    /// Arranges the selected nodes by their wires, with `pack`. Wires to nodes outside the
    /// selection do not count.
    pub(super) fn auto_pack(&mut self) {
        let group = self.selection();
        let at: Vec<Point> = group.iter().map(|&n| self.nodes[n].position).collect();
        let sizes: Vec<nih_plug_iced::Size> = group
            .iter()
            .map(|&n| self.nodes[n].bounds().size())
            .collect();
        let local = |node: usize| group.iter().position(|&g| g == node);
        let wires: Vec<(usize, usize)> = self
            .connections
            .iter()
            .filter_map(|c| Some((local(c.from)?, local(c.to)?)))
            .collect();
        let placed = super::pack::pack(&at, &sizes, &wires);
        for (&node, position) in group.iter().zip(placed) {
            self.nodes[node].position = position;
        }
    }

    /// Removes every selected node.
    pub(super) fn delete_selected(&mut self) {
        let mut nodes = self.selection();
        nodes.sort_unstable();
        // From the highest index down, so the lower ones keep their place.
        for index in nodes.into_iter().rev() {
            self.delete_node(index);
        }
    }

    /// Removes every selected node, reconnecting around each one the way `delete_in_place` does.
    /// Deleting a run of several nodes in a row bypasses the whole run in one go, since each node
    /// is removed from the highest index down, onto wires the step before it already relinked.
    pub(super) fn delete_selected_in_place(&mut self) {
        let mut nodes = self.selection();
        nodes.sort_unstable();
        for index in nodes.into_iter().rev() {
            self.delete_in_place(index);
        }
    }

    /// Removes `node`, first reconnecting whatever fed it straight to whatever it fed, the
    /// Blender node editor trick of Delete and Reconnect. Only a node shaped like one
    /// `link_under` could splice into, a single clear audio, control, or notes path through it,
    /// has anything to reconnect; any other node is simply removed, same as `delete_node`.
    pub(super) fn delete_in_place(&mut self, index: usize) {
        self.bypass_wiring(index);
        self.delete_node(index);
    }

    /// Wires whatever feeds `node`'s one through-path straight to whatever `node` feeds on it,
    /// before `node` is removed. Does nothing for a node with no such path, or with nothing wired
    /// into it, the same as deleting the node outright would then leave.
    fn bypass_wiring(&mut self, node: usize) {
        let Some((in_port, out_port)) = self.through_ports(node) else {
            return;
        };
        let Some((from, out)) = self
            .connections
            .iter()
            .find(|c| c.to == node && c.input == in_port)
            .map(|c| (c.from, c.out))
        else {
            return;
        };
        let targets: Vec<(usize, Port)> = self
            .connections
            .iter()
            .filter(|c| c.from == node && c.out == out_port)
            .map(|c| (c.to, c.input))
            .collect();
        for (to, input) in targets {
            self.connect(from, out, to, input);
        }
    }

    /// The input and output pin of `node`'s one through-path, if it has one: an audio in and out,
    /// a control in and out (a map's shape pin, or a math node's A), or a notes in and out. This
    /// is the same shape `link_under` recognises as splice-able, since reconnecting around a node
    /// is splicing in reverse.
    fn through_ports(&self, node: usize) -> Option<(Port, Port)> {
        let dragged = &self.nodes[node];
        let behavior = dragged.kind.behavior();
        if behavior.audio_in() && behavior.audio_out() {
            let first_audio = dragged
                .input_ports()
                .into_iter()
                .find(|port| port.is_audio())?;
            return Some((first_audio, Port::Audio));
        }
        if let Some(input) = Self::control_splice_input(dragged) {
            if dragged.output_ports().contains(&Port::Shape) {
                return Some((input, Port::Shape));
            }
        }
        if dragged.input_ports().contains(&Port::Notes)
            && dragged.output_ports().contains(&Port::Notes)
        {
            return Some((Port::Notes, Port::Notes));
        }
        None
    }

    /// Makes `node` follow the cursor, centred on `point`, until the next press sets it down. A
    /// press that lands while a wire is under the node splices the node into that wire.
    pub(super) fn start_placing(&mut self, node: usize, point: Point) {
        let grab = Vector::new(NODE_WIDTH / 2.0, HEADER_HEIGHT / 2.0);
        self.nodes[node].position = Point::new(point.x - grab.x, point.y - grab.y);
        self.select(node);
        self.interaction = Interaction::Placing { node, grab };
    }

    /// Starts a modal grab of the selection: G in Edit mode. The primary selected node drives it,
    /// tracked the same way a drag tracks the node under the cursor, and the rest of the
    /// selection follows through `move_selection`. Does nothing with an empty selection.
    pub(super) fn start_grab(&mut self) {
        let Some(node) = self.selected else { return };
        let cursor = self.view.to_world(self.last_screen);
        let at = self.nodes[node].position;
        self.grab_positions = self
            .selection()
            .into_iter()
            .map(|n| (n, self.nodes[n].position))
            .collect();
        self.interaction = Interaction::Grabbing {
            node,
            grab: Vector::new(cursor.x - at.x, cursor.y - at.y),
        };
    }

    /// Ends a grab at its current position, the same as releasing an ordinary drag: a lone moved
    /// node splices into the wire under it. Returns false, doing nothing, outside a grab.
    pub(super) fn commit_grab(&mut self) -> bool {
        let Interaction::Grabbing { node, .. } = self.interaction else {
            return false;
        };
        self.interaction = Interaction::Idle;
        self.grab_positions.clear();
        if let Some(link) = self.link_target.take() {
            self.insert_into(node, link);
        }
        true
    }

    /// Cancels a grab in progress, putting every moved node back exactly where it started, with
    /// no splice and no selection change. Returns false, doing nothing, outside a grab.
    pub(super) fn cancel_grab(&mut self) -> bool {
        if !matches!(self.interaction, Interaction::Grabbing { .. }) {
            return false;
        }
        self.interaction = Interaction::Idle;
        self.link_target = None;
        for (node, position) in self.grab_positions.drain(..) {
            if let Some(found) = self.nodes.get_mut(node) {
                found.position = position;
            }
        }
        true
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
        self.select(copy);
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
        // The edge nodes of a group go only with the group.
        if self
            .nodes
            .get(index)
            .is_some_and(|node| matches!(node.kind, NodeKind::GroupInput | NodeKind::GroupOutput))
        {
            return;
        }
        // Slides name nodes by index, so they land before the indices shift.
        self.finish_slides();
        self.nodes.remove(index);
        self.drop_monitor_history(index);
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
        self.refresh_edge_knobs();
        self.selected = match self.selected {
            Some(selected) if selected == index => None,
            Some(selected) if selected > index => Some(selected - 1),
            other => other,
        };
        self.also_selected.retain(|&node| node != index);
        for node in &mut self.also_selected {
            if *node > index {
                *node -= 1;
            }
        }
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
