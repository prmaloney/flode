use nih_plug::prelude::{Editor, GuiContext};
use nih_plug_iced::*;
use rtrb::Producer;
use std::sync::{Arc, Mutex};

use crate::node_editor::{self, NodeEditor};
use crate::piano::PianoEvent;
use crate::routing::AudioRouting;
use crate::FlodeParams;

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

    graph: node_editor::Graph,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    /// A message from the node graph.
    Graph(node_editor::Message),
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
            Ok(saved) => node_editor::Graph::from_saved(&saved),
            Err(_) => node_editor::Graph::default(),
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
            Message::Graph(node_editor::Message::Note(note)) => {
                if let Some(tx) = &self.note_tx {
                    // A full queue drops the note. The lock is only ever taken on the GUI thread.
                    if let Ok(mut tx) = tx.lock() {
                        let _ = tx.push(note);
                    }
                }
            }
            // The cursor moved over or off a dropdown. The next frame draws the change.
            Message::Graph(node_editor::Message::Hovered) => {}
            Message::Tick => {}
            // A change to the graph, a node, or a knob. The audio thread gets the new state, and
            // the project saves it.
            Message::Graph(node_editor::Message::GraphChanged) => {
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
        let editor: Canvas<node_editor::Message, NodeEditor<'_>> = Canvas::new(NodeEditor::new(
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
