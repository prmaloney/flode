//! A freeform node graph for the plugin UI. Click a pill button at the top to add a node. It
//! follows the mouse until the next click places it. Drag a node by its header, and click one to
//! select it. Right-click a node to delete it. The arrow in a header collapses the node. Pins sit
//! on the node edges, with labels. Inputs are on the left and outputs are on the right. Audio
//! wires are grey. Control wires are green. A knob that a control wire drives is greyed out and
//! shows the live signal. Scrolling pans the canvas, and a pinch zooms it.
//!
//! The nodes themselves are in `nodes`. This module holds the graph, the wiring between nodes,
//! and the editor widget that draws the graph and handles the mouse.

pub mod canvas;
mod curve;
mod demo;
mod depth;
mod draw;
#[cfg(test)]
mod factory;
pub mod graph;
pub mod group;
#[cfg(test)]
mod group_file_tests;
#[cfg(test)]
mod group_knob_tests;
#[cfg(test)]
mod group_tests;
mod group_ui;
#[cfg(test)]
mod group_ui_tests;
pub mod knob;
mod logo;
mod menus;
mod meter;
mod monitor;
mod pack;
mod pane;
pub mod piano;
pub mod saved;
mod shortcuts;
mod style;
#[cfg(test)]
mod tests;
mod view;

use nih_plug::prelude::{Editor, GuiContext, Param, ParamSetter};
use nih_plug_iced::*;
use rtrb::Producer;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::editor::piano::PianoEvent;
use crate::patch::link::PatchLink;
use crate::FlodeParams;
use canvas::NodeEditor;

pub(crate) fn default_state() -> Arc<IcedState> {
    IcedState::from_size(1024, 712)
}

/// `note_tx` carries the notes a click on the key strip plays. It is `None` in a test with no audio
/// side. `computer_keys` is true in the standalone app, where the computer keyboard plays notes
/// too. In a plugin the host owns the keyboard, so the strip is the only way to play from the
/// editor. `link` hands the audio thread the patch and shows the editor what it reports.
pub(crate) fn create(
    params: Arc<FlodeParams>,
    editor_state: Arc<IcedState>,
    note_tx: Option<Arc<Mutex<Producer<PianoEvent>>>>,
    computer_keys: bool,
    link: Arc<PatchLink>,
) -> Option<Box<dyn Editor>> {
    create_iced_editor::<FlodeEditor>(editor_state, (params, note_tx, computer_keys, link))
}

struct FlodeEditor {
    params: Arc<FlodeParams>,
    context: Arc<dyn GuiContext>,
    note_tx: Option<Arc<Mutex<Producer<PianoEvent>>>>,
    link: Arc<PatchLink>,

    graph: graph::Graph,
    /// The held output levels, which fall back between frames.
    meter: meter::Ballistics,
    /// When the meter last updated, to time its fall.
    meter_updated: Option<Instant>,
    /// The note-on count the editor last saw from the audio thread, and when it last changed.
    notes_seen: u32,
    midi_flash: Option<Instant>,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    /// A message from the node graph.
    Graph(canvas::Message),
    /// A timer tick, sent while a control drives a knob so the live indicator redraws.
    Tick,
}

impl FlodeEditor {
    /// Passes the macro knob changes the graph recorded to the host, so it can record automation.
    fn send_macro_edits(&mut self) {
        if self.graph.macro_edits.is_empty() {
            return;
        }
        let setter = ParamSetter::new(self.context.as_ref());
        // A preset that leaves a macro where the host has it needs no write to that parameter.
        let edits = std::mem::take(&mut self.graph.macro_edits);
        let edits = graph::without_unchanged_macros(&edits, |index| {
            self.params
                .macro_param(index)
                .unmodulated_normalized_value()
        });
        for edit in edits {
            match edit {
                graph::MacroEdit::Begin(index) => {
                    setter.begin_set_parameter(self.params.macro_param(index))
                }
                graph::MacroEdit::Set(index, value) => {
                    setter.set_parameter_normalized(self.params.macro_param(index), value)
                }
                graph::MacroEdit::End(index) => {
                    setter.end_set_parameter(self.params.macro_param(index))
                }
            }
        }
    }

    /// Saves the preset the user just named. It is saved here because the macro values are the
    /// host's, and the graph does not hold them. The graph is also saved with the project, so
    /// the preset's name stays on the pill.
    fn save_pending_preset(&mut self) {
        if self.graph.pending_save.is_none() {
            return;
        }
        let macros = std::array::from_fn(|index| {
            self.params
                .macro_param(index)
                .unmodulated_normalized_value()
        });
        self.graph.save_pending_preset(macros);
        if let Ok(mut saved) = self.params.graph.lock() {
            *saved = self.graph.to_saved();
        }
    }

    /// Shows the host's macro values on the macro node, which they may have moved by automation.
    fn show_macros(&mut self) {
        let values = std::array::from_fn(|index| {
            self.params
                .macro_param(index)
                .unmodulated_normalized_value()
        });
        self.graph.show_macros(values);
    }
}

impl IcedEditor for FlodeEditor {
    type Executor = executor::Default;
    type Message = Message;
    type InitializationFlags = (
        Arc<FlodeParams>,
        Option<Arc<Mutex<Producer<PianoEvent>>>>,
        bool,
        Arc<PatchLink>,
    );

    fn new(
        (params, note_tx, computer_keys, link): Self::InitializationFlags,
        context: Arc<dyn GuiContext>,
    ) -> (Self, Command<Self::Message>) {
        let mut graph = match params.graph.lock() {
            Ok(saved) => graph::Graph::from_saved(&saved),
            Err(_) => graph::Graph::default(),
        };
        demo::apply(&mut graph);
        graph.computer_keys = computer_keys;
        link.publish(graph.patch());
        // The demo hook can hold a note down from the start, so a picture shows live signals.
        if let (Some(tx), Some(note)) = (&note_tx, demo::note()) {
            if let Ok(mut tx) = tx.lock() {
                let _ = tx.push(PianoEvent::NoteOn {
                    note,
                    velocity: 0.8,
                });
            }
        }
        let editor = FlodeEditor {
            params,
            context,
            note_tx,
            link,
            graph,
            meter: meter::Ballistics::default(),
            meter_updated: None,
            notes_seen: 0,
            midi_flash: None,
        };
        (editor, Command::none())
    }

    fn context(&self) -> &dyn GuiContext {
        self.context.as_ref()
    }

    fn update(
        &mut self,
        _window: &mut WindowQueue,
        message: Self::Message,
    ) -> Command<Self::Message> {
        match message {
            Message::Graph(canvas::Message::Note(note)) => {
                if let Some(tx) = &self.note_tx {
                    // A full queue drops the note. The lock is only ever taken on the GUI thread.
                    if let Ok(mut tx) = tx.lock() {
                        let _ = tx.push(note);
                    }
                }
            }
            // The cursor moved over or off a dropdown. The next frame draws the change.
            Message::Graph(canvas::Message::Hovered) => {}
            Message::Tick => {}
            // A change to the graph, a node, or a knob. The audio thread gets the new state, and
            // the project saves it.
            Message::Graph(canvas::Message::GraphChanged) => {
                self.link.publish(self.graph.patch());
                if let Ok(mut saved) = self.params.graph.lock() {
                    *saved = self.graph.to_saved();
                }
            }
        }
        self.graph.poll_save_dialog();
        self.save_pending_preset();
        self.send_macro_edits();
        Command::none()
    }

    fn subscription(
        &self,
        window_subs: &mut WindowSubs<Self::Message>,
    ) -> Subscription<Self::Message> {
        // Redraw every frame, so the output meter moves and a driven knob's indicator follows its
        // control.
        window_subs.on_frame = Some(Message::Tick);
        Subscription::none()
    }

    fn view(&mut self) -> Element<'_, Self::Message> {
        self.show_macros();
        let meters = self.link.meters();
        let now = Instant::now();
        let seconds = self
            .meter_updated
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
        self.meter_updated = Some(now);
        // Nodes spaced out by a splice ease into place. The saved graph follows once they land.
        if self.graph.advance_slides(now) {
            if let Ok(mut saved) = self.params.graph.lock() {
                *saved = self.graph.to_saved();
            }
        }
        self.graph.spectra = meters.spectra;
        self.graph.monitors = meters.monitors;
        self.graph.update_monitors(now);
        let output = self.meter.update(meters.output, seconds);

        // The MIDI light flashes when the audio thread has taken in a note-on since the last frame,
        // and fades after.
        if meters.notes_in != self.notes_seen {
            self.notes_seen = meters.notes_in;
            self.midi_flash = Some(now);
        }
        self.graph.midi_light = self.midi_flash.map_or(0.0, |flash| {
            meter::midi_light(now.duration_since(flash).as_secs_f32())
        });
        let editor: Canvas<canvas::Message, NodeEditor<'_>> = Canvas::new(NodeEditor::new(
            &mut self.graph,
            meters.levels,
            meters.active_notes,
            output,
        ))
        .width(Length::Fill)
        .height(Length::Fill);

        Element::from(editor).map(Message::Graph)
    }

    fn background_color(&self) -> nih_plug_iced::Color {
        style::BACKGROUND_COLOR
    }
}
