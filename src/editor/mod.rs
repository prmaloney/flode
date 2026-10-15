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
mod draw;
pub mod graph;
pub mod knob;
mod menus;
pub mod piano;
pub mod saved;
mod style;
#[cfg(test)]
mod tests;
mod view;

use nih_plug::prelude::{Editor, GuiContext};
use nih_plug_iced::*;
use rtrb::Producer;
use std::sync::{Arc, Mutex};

use crate::editor::piano::PianoEvent;
use crate::routing::AudioRouting;
use crate::FlodeParams;
use canvas::NodeEditor;

pub(crate) fn default_state() -> Arc<IcedState> {
    IcedState::from_size(1024, 712)
}

/// `note_tx` is `Some` only in standalone mode. Without it, computer keyboard notes are dropped.
/// `routing` tells the audio thread the audio chains and every node's knob values.
pub(crate) fn create(
    params: Arc<FlodeParams>,
    editor_state: Arc<IcedState>,
    note_tx: Option<Arc<Mutex<Producer<PianoEvent>>>>,
    routing: Arc<AudioRouting>,
) -> Option<Box<dyn Editor>> {
    create_iced_editor::<FlodeEditor>(editor_state, (params, note_tx, routing))
}

struct FlodeEditor {
    params: Arc<FlodeParams>,
    context: Arc<dyn GuiContext>,
    note_tx: Option<Arc<Mutex<Producer<PianoEvent>>>>,
    routing: Arc<AudioRouting>,

    graph: graph::Graph,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    /// A message from the node graph.
    Graph(canvas::Message),
    /// A timer tick, sent while a control drives a knob so the live indicator redraws.
    Tick,
}

impl IcedEditor for FlodeEditor {
    type Executor = executor::Default;
    type Message = Message;
    type InitializationFlags = (
        Arc<FlodeParams>,
        Option<Arc<Mutex<Producer<PianoEvent>>>>,
        Arc<AudioRouting>,
    );

    fn new(
        (params, note_tx, routing): Self::InitializationFlags,
        context: Arc<dyn GuiContext>,
    ) -> (Self, Command<Self::Message>) {
        let graph = match params.graph.lock() {
            Ok(saved) => graph::Graph::from_saved(&saved),
            Err(_) => graph::Graph::default(),
        };
        routing.publish(&graph);
        let editor = FlodeEditor {
            params,
            context,
            note_tx,
            routing,
            graph,
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
                self.routing.publish(&self.graph);
                if let Ok(mut saved) = self.params.graph.lock() {
                    *saved = self.graph.to_saved();
                }
            }
        }
        Command::none()
    }

    fn subscription(
        &self,
        window_subs: &mut WindowSubs<Self::Message>,
    ) -> Subscription<Self::Message> {
        // Redraw every frame while a control drives a knob, so its indicator moves.
        if self.routing.any_driven() {
            window_subs.on_frame = Some(Message::Tick);
        }
        Subscription::none()
    }

    fn view(&mut self) -> Element<'_, Self::Message> {
        let editor: Canvas<canvas::Message, NodeEditor<'_>> = Canvas::new(NodeEditor::new(
            &mut self.graph,
            self.routing.levels(),
            self.routing.active_notes(),
        ))
        .width(Length::Fill)
        .height(Length::Fill);

        Element::from(editor).map(Message::Graph)
    }

    fn background_color(&self) -> nih_plug_iced::Color {
        nih_plug_iced::Color {
            r: 0.15,
            g: 0.15,
            b: 0.15,
            a: 1.0,
        }
    }
}
