//! Tests for the Keyboard/Edit mode toggle: Tab, the Edit-mode letter shortcuts, and Grab. The
//! regression that Keyboard mode behaves exactly as before, byte for byte, lives in `tests.rs`
//! and `shortcuts.rs`, unmodified and starting from the default mode. These tests cover what is
//! new.

use super::canvas::{Message, NodeEditor};
use super::graph::{EditorMode, Graph, Interaction};
use super::piano::PianoEvent;
use crate::nodes::{FlodeNode, NodeKind, Port, NODE_WIDTH};
use crate::patch::control::ControlLevels;
use nih_plug_iced::keyboard::{self, KeyCode, Modifiers};
use nih_plug_iced::mouse;
use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};
use nih_plug_iced::{Point, Rectangle};

fn bounds() -> Rectangle {
    Rectangle {
        x: 0.0,
        y: 0.0,
        width: 1024.0,
        height: 712.0,
    }
}

/// Three effects in a row, nothing wired, so there is always something to select, grab, or
/// delete.
fn row() -> Graph {
    Graph::with(
        vec![
            FlodeNode::new(NodeKind::Reverb, Point::new(100.0, 150.0)),
            FlodeNode::new(NodeKind::Delay, Point::new(400.0, 150.0)),
            FlodeNode::new(NodeKind::Chorus, Point::new(700.0, 150.0)),
        ],
        Vec::new(),
    )
}

/// An oscillator feeding a delay that feeds the output, with a wide gap between the oscillator
/// and the delay, so a third node has room to land on the wire between them.
fn wired_chain_with_gap() -> Graph {
    let mut graph = Graph::with(
        vec![
            FlodeNode::new(NodeKind::Oscillator, Point::new(0.0, 80.0)),
            FlodeNode::new(NodeKind::Delay, Point::new(NODE_WIDTH + 400.0, 80.0)),
            FlodeNode::new(NodeKind::Output, Point::new(900.0, 80.0)),
        ],
        Vec::new(),
    );
    graph.connect(0, Port::Audio, 1, Port::Audio);
    graph.connect(1, Port::Audio, 2, Port::Slot(0));
    graph
}

/// A filter and a reverb grouped and opened, so the group's pane shows the edge nodes at 0 and 1,
/// the filter at 2, and the reverb at 3.
fn grouped_and_opened() -> Graph {
    let mut graph = Graph::with(
        vec![
            FlodeNode::new(NodeKind::Input, Point::new(0.0, 80.0)),
            FlodeNode::new(NodeKind::Oscillator, Point::new(200.0, 80.0)),
            FlodeNode::new(NodeKind::Filter, Point::new(400.0, 80.0)),
            FlodeNode::new(NodeKind::Reverb, Point::new(600.0, 80.0)),
            FlodeNode::new(NodeKind::Output, Point::new(900.0, 80.0)),
        ],
        Vec::new(),
    );
    graph.connect(0, Port::Notes, 1, Port::Notes);
    graph.connect(1, Port::Audio, 2, Port::Audio);
    graph.connect(2, Port::Audio, 3, Port::Audio);
    graph.connect(3, Port::Audio, 4, Port::Slot(0));
    let group = graph
        .create_group(&[2, 3])
        .expect("the filter and reverb group");
    assert!(graph.open_group(group));
    graph
}

fn editor(graph: &mut Graph) -> NodeEditor<'_> {
    NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2])
}

/// Presses a key and lets it go, and returns what the press sent.
fn key(editor: &mut NodeEditor, key_code: KeyCode, modifiers: Modifiers) -> Option<Message> {
    let message = editor
        .update(
            Event::Keyboard(keyboard::Event::KeyPressed {
                key_code,
                modifiers,
            }),
            bounds(),
            Cursor::Unavailable,
        )
        .1;
    editor.update(
        Event::Keyboard(keyboard::Event::KeyReleased {
            key_code,
            modifiers: Modifiers::default(),
        }),
        bounds(),
        Cursor::Unavailable,
    );
    message
}

fn plain(editor: &mut NodeEditor, key_code: KeyCode) -> Option<Message> {
    key(editor, key_code, Modifiers::default())
}

fn mouse_at(editor: &mut NodeEditor, event: mouse::Event, at: Point) -> Option<Message> {
    editor
        .update(Event::Mouse(event), bounds(), Cursor::Available(at))
        .1
}

fn press(editor: &mut NodeEditor, at: Point) -> Option<Message> {
    mouse_at(editor, mouse::Event::ButtonPressed(mouse::Button::Left), at)
}

fn release(editor: &mut NodeEditor, at: Point) -> Option<Message> {
    mouse_at(
        editor,
        mouse::Event::ButtonReleased(mouse::Button::Left),
        at,
    )
}

fn right_press(editor: &mut NodeEditor, at: Point) -> Option<Message> {
    mouse_at(
        editor,
        mouse::Event::ButtonPressed(mouse::Button::Right),
        at,
    )
}

fn move_to(editor: &mut NodeEditor, at: Point) -> Option<Message> {
    mouse_at(editor, mouse::Event::CursorMoved { position: at }, at)
}

/// Every key the piano maps to a note or an octave shift in Keyboard mode.
const PIANO_KEYS: [KeyCode; 19] = [
    KeyCode::A,
    KeyCode::W,
    KeyCode::S,
    KeyCode::E,
    KeyCode::D,
    KeyCode::F,
    KeyCode::T,
    KeyCode::G,
    KeyCode::Y,
    KeyCode::H,
    KeyCode::U,
    KeyCode::J,
    KeyCode::K,
    KeyCode::O,
    KeyCode::L,
    KeyCode::P,
    KeyCode::Semicolon,
    KeyCode::Z,
    KeyCode::X,
];

// Tab.

#[test]
fn keyboard_mode_is_the_default() {
    assert_eq!(row().mode, EditorMode::Keyboard);
}

#[test]
fn tab_toggles_the_mode_and_changes_nothing_else() {
    let mut graph = row();
    graph.select(0);
    let before: Vec<Point> = graph.nodes.iter().map(|n| n.position).collect();
    let mut ed = editor(&mut graph);
    let message = plain(&mut ed, KeyCode::Tab);
    assert_eq!(ed.graph.mode, EditorMode::Edit);
    // A pure mode change is not a graph edit, the same way flipping `shift` or `alt` is not.
    assert!(!matches!(message, Some(Message::GraphChanged)));
    assert_eq!(ed.graph.selected, Some(0));
    let after: Vec<Point> = ed.graph.nodes.iter().map(|n| n.position).collect();
    assert_eq!(before, after);
    assert!(matches!(ed.graph.interaction, Interaction::Idle));
    plain(&mut ed, KeyCode::Tab);
    assert_eq!(ed.graph.mode, EditorMode::Keyboard);
}

// Edit mode turns the piano off.

#[test]
fn edit_mode_sends_no_note_for_any_piano_key() {
    for key_code in PIANO_KEYS {
        let mut graph = row();
        graph.select(0);
        let mut ed = editor(&mut graph);
        plain(&mut ed, KeyCode::Tab);
        let message = plain(&mut ed, key_code);
        assert!(
            !matches!(message, Some(Message::Note(_))),
            "{key_code:?} sent a note in Edit mode"
        );
    }
}

#[test]
fn edit_mode_x_deletes_instead_of_shifting_the_octave() {
    let mut graph = row();
    graph.select(0);
    let mut ed = editor(&mut graph);
    plain(&mut ed, KeyCode::Tab);
    assert_eq!(ed.graph.piano.octave(), 0);
    plain(&mut ed, KeyCode::X);
    assert_eq!(ed.graph.piano.octave(), 0, "X no longer shifts the octave");
    assert_eq!(ed.graph.nodes.len(), 2, "X deleted the selected node");
}

#[test]
fn delete_key_also_removes_the_selection_in_edit_mode() {
    let mut graph = row();
    graph.select(0);
    let mut ed = editor(&mut graph);
    plain(&mut ed, KeyCode::Tab);
    plain(&mut ed, KeyCode::Delete);
    assert_eq!(ed.graph.nodes.len(), 2);
}

#[test]
fn keyboard_mode_x_still_shifts_the_octave_as_before() {
    let mut graph = row();
    let mut ed = editor(&mut graph);
    plain(&mut ed, KeyCode::X);
    assert_eq!(ed.graph.piano.octave(), 1);
    assert_eq!(ed.graph.nodes.len(), 3, "nothing was deleted");
}

// A and Alt+A.

#[test]
fn a_selects_all_and_alt_a_deselects_in_edit_mode() {
    let mut graph = row();
    let mut ed = editor(&mut graph);
    plain(&mut ed, KeyCode::Tab);
    key(&mut ed, KeyCode::A, Modifiers::default());
    assert_eq!(ed.graph.selection().len(), 3);
    key(&mut ed, KeyCode::A, Modifiers::ALT);
    assert!(ed.graph.selection().is_empty());
}

#[test]
fn a_plain_a_plays_a_note_in_keyboard_mode_not_select_all() {
    let mut graph = row();
    let mut ed = editor(&mut graph);
    let message = plain(&mut ed, KeyCode::A);
    assert!(matches!(
        message,
        Some(Message::Note(PianoEvent::NoteOn { .. }))
    ));
    assert!(ed.graph.selection().is_empty(), "A did not select anything");
}

// Grab.

#[test]
fn grab_moves_the_whole_selection_and_a_click_commits_it() {
    let mut graph = row();
    graph.set_selection(vec![0, 1]);
    let before: Vec<Point> = graph.nodes.iter().map(|n| n.position).collect();
    let mut ed = editor(&mut graph);
    plain(&mut ed, KeyCode::Tab);
    let start = Point::new(500.0, 500.0);
    move_to(&mut ed, start);
    plain(&mut ed, KeyCode::G);
    assert!(matches!(ed.graph.interaction, Interaction::Grabbing { .. }));
    let to = Point::new(start.x + 40.0, start.y + 30.0);
    move_to(&mut ed, to);
    for node in [0, 1] {
        assert_eq!(ed.graph.nodes[node].position.x, before[node].x + 40.0);
        assert_eq!(ed.graph.nodes[node].position.y, before[node].y + 30.0);
    }
    assert_eq!(
        ed.graph.nodes[2].position, before[2],
        "not in the selection"
    );
    let message = press(&mut ed, to);
    assert!(matches!(message, Some(Message::GraphChanged)));
    assert!(matches!(ed.graph.interaction, Interaction::Idle));
    assert_eq!(ed.graph.nodes[0].position.x, before[0].x + 40.0);
}

#[test]
fn enter_also_commits_a_grab_in_place() {
    let mut graph = row();
    graph.select(0);
    let before = graph.nodes[0].position;
    let mut ed = editor(&mut graph);
    plain(&mut ed, KeyCode::Tab);
    move_to(&mut ed, Point::new(300.0, 300.0));
    plain(&mut ed, KeyCode::G);
    let to = Point::new(340.0, 330.0);
    move_to(&mut ed, to);
    plain(&mut ed, KeyCode::Enter);
    assert!(matches!(ed.graph.interaction, Interaction::Idle));
    assert_eq!(ed.graph.nodes[0].position.x, before.x + 40.0);
}

#[test]
fn escape_cancels_a_grab_and_restores_every_nodes_position() {
    let mut graph = row();
    graph.set_selection(vec![0, 1, 2]);
    let before: Vec<Point> = graph.nodes.iter().map(|n| n.position).collect();
    let mut ed = editor(&mut graph);
    plain(&mut ed, KeyCode::Tab);
    move_to(&mut ed, Point::new(200.0, 200.0));
    plain(&mut ed, KeyCode::G);
    move_to(&mut ed, Point::new(260.0, 240.0));
    assert_ne!(ed.graph.nodes[0].position, before[0], "it did move first");
    assert!(matches!(
        plain(&mut ed, KeyCode::Escape),
        Some(Message::Hovered)
    ));
    assert!(matches!(ed.graph.interaction, Interaction::Idle));
    for node in 0..3 {
        assert_eq!(ed.graph.nodes[node].position, before[node], "node {node}");
    }
    // Esc restores the positions with no other side effects: the selection is unchanged.
    assert_eq!(ed.graph.selection().len(), 3);
}

#[test]
fn a_right_click_also_cancels_a_grab() {
    let mut graph = row();
    graph.select(0);
    let before = graph.nodes[0].position;
    let mut ed = editor(&mut graph);
    plain(&mut ed, KeyCode::Tab);
    move_to(&mut ed, Point::new(200.0, 200.0));
    plain(&mut ed, KeyCode::G);
    move_to(&mut ed, Point::new(260.0, 260.0));
    right_press(&mut ed, Point::new(260.0, 260.0));
    assert!(matches!(ed.graph.interaction, Interaction::Idle));
    assert_eq!(ed.graph.nodes[0].position, before);
    // No menu opened either: the right click only canceled the grab.
    assert!(ed.graph.menu.is_none());
}

#[test]
fn g_with_nothing_selected_does_nothing() {
    let mut graph = row();
    graph.clear_selection();
    let mut ed = editor(&mut graph);
    plain(&mut ed, KeyCode::Tab);
    let message = plain(&mut ed, KeyCode::G);
    assert!(matches!(ed.graph.interaction, Interaction::Idle));
    assert!(message.is_none());
}

#[test]
fn grab_of_a_lone_node_splices_it_into_the_wire_it_lands_on() {
    let mut graph = wired_chain_with_gap();
    let filter = graph.add_node(NodeKind::Filter, Point::new(300.0, 300.0));
    graph.select(filter);
    let mut ed = editor(&mut graph);
    plain(&mut ed, KeyCode::Tab);
    move_to(&mut ed, Point::new(310.0, 310.0));
    plain(&mut ed, KeyCode::G);
    // Onto the wire between the oscillator and the delay, which runs along y = 80.
    let onto_wire = Point::new(NODE_WIDTH + 150.0, 80.0);
    move_to(&mut ed, onto_wire);
    assert!(
        ed.graph.link_target.is_some(),
        "a lone grabbed node still finds the wire"
    );
    press(&mut ed, onto_wire);
    assert!(ed
        .graph
        .connections
        .iter()
        .any(|c| c.from == filter || c.to == filter));
}

#[test]
fn grab_and_delete_work_inside_an_open_groups_pane() {
    let mut graph = grouped_and_opened();
    graph.select(2); // the filter, inside the open group's pane
    let before = graph.nodes[2].position;
    let mut ed = editor(&mut graph);
    plain(&mut ed, KeyCode::Tab);
    move_to(&mut ed, Point::new(50.0, 50.0));
    plain(&mut ed, KeyCode::G);
    assert!(matches!(ed.graph.interaction, Interaction::Grabbing { .. }));
    let to = Point::new(90.0, 80.0);
    move_to(&mut ed, to);
    assert_ne!(ed.graph.nodes[2].position, before);
    press(&mut ed, to);
    assert!(matches!(ed.graph.interaction, Interaction::Idle));
    assert!(ed.graph.is_selected(2));
    plain(&mut ed, KeyCode::X);
    assert_eq!(
        ed.graph.nodes.len(),
        3,
        "the edge nodes and the reverb remain"
    );
    assert_eq!(ed.graph.group_depth(), 1, "still inside the open group");
}

// The removed `computer_keys` gate.

#[test]
fn keyboard_mode_sends_notes_from_a_fresh_graph() {
    // `computer_keys` used to gate this on standalone versus plugin. A new `Graph` no longer
    // tracks that distinction at all: it only tracks the mode, which starts at Keyboard either
    // way, so the keyboard plays here where it used to depend on how the editor was built.
    let mut graph = row();
    let mut ed = editor(&mut graph);
    let message = plain(&mut ed, KeyCode::D);
    assert!(matches!(
        message,
        Some(Message::Note(PianoEvent::NoteOn { .. }))
    ));
}

#[test]
fn edit_mode_sends_no_notes_either() {
    let mut graph = row();
    let mut ed = editor(&mut graph);
    plain(&mut ed, KeyCode::Tab);
    for key_code in PIANO_KEYS {
        let message = plain(&mut ed, key_code);
        assert!(!matches!(message, Some(Message::Note(_))), "{key_code:?}");
    }
}

// The mode badge.

#[test]
fn the_mode_badge_sits_above_the_strip_and_does_not_toggle_on_a_click() {
    let bounds = bounds();
    let badge = super::draw::mode_badge_bounds(bounds);
    let strip = super::piano::strip_bounds(bounds);
    assert!(
        badge.y + badge.height <= strip.y,
        "the badge sits above the strip"
    );
    assert!(
        badge.x + badge.width <= bounds.width,
        "the badge fits in the editor"
    );

    // It is a legend, not a button: only Tab switches modes.
    let mut graph = row();
    let mut ed = editor(&mut graph);
    let at = Point::new(badge.x + badge.width / 2.0, badge.y + badge.height / 2.0);
    press(&mut ed, at);
    release(&mut ed, at);
    assert_eq!(ed.graph.mode, EditorMode::Keyboard);
}

// Notes wires auto-splice too, now that an arpeggiator has a notes pin on each side.

/// An Input feeding an Oscillator, with a wide gap for a third node to land on the wire between
/// them.
fn notes_chain_with_gap() -> Graph {
    let mut graph = Graph::with(
        vec![
            FlodeNode::new(NodeKind::Input, Point::new(0.0, 80.0)),
            FlodeNode::new(NodeKind::Oscillator, Point::new(NODE_WIDTH + 400.0, 80.0)),
        ],
        Vec::new(),
    );
    graph.connect(0, Port::Notes, 1, Port::Notes);
    graph
}

#[test]
fn dropping_an_arpeggiator_on_a_notes_wire_splices_it_in() {
    let mut graph = notes_chain_with_gap();
    let curve = graph.link_curve(&graph.connections[0]).unwrap();
    let middle = super::draw::bezier_point(&curve, 0.5);
    let arp = graph.add_node(
        NodeKind::Arpeggiator,
        Point::new(middle.x - 80.0, middle.y - 20.0),
    );
    let link = graph
        .link_under(arp)
        .expect("the wire from the input to the oscillator passes under it");
    graph.insert_into(arp, link);
    assert!(graph
        .connections
        .iter()
        .any(|c| c.from == 0 && c.out == Port::Notes && c.to == arp && c.input == Port::Notes));
    assert!(graph
        .connections
        .iter()
        .any(|c| c.from == arp && c.out == Port::Notes && c.to == 1 && c.input == Port::Notes));
    assert_eq!(graph.connections.len(), 2);
}

/// An audio-only node, such as a reverb, does not splice into a notes wire, and an arpeggiator
/// does not splice into an audio wire: `link_under` only matches a node's own kind of path.
#[test]
fn a_node_only_splices_into_its_own_kind_of_wire() {
    let mut notes = notes_chain_with_gap();
    let reverb = notes.add_node(NodeKind::Reverb, Point::new(200.0, 300.0));
    assert_eq!(notes.link_under(reverb), None);

    let mut audio = wired_chain_with_gap();
    let arp = audio.add_node(NodeKind::Arpeggiator, Point::new(200.0, 300.0));
    assert_eq!(audio.link_under(arp), None);
}

// Delete and reconnect: Blender's node editor trick, on Command+X in Edit mode and in the
// right-click menu.

#[test]
fn command_x_deletes_the_selected_node_and_reconnects_around_it() {
    let mut graph = wired_chain_with_gap();
    graph.select(1);
    let mut ed = editor(&mut graph);
    ed.graph.mode = EditorMode::Edit;
    key(&mut ed, KeyCode::X, super::shortcuts::COMMAND);
    assert_eq!(graph.nodes.len(), 2);
    assert!(graph
        .connections
        .iter()
        .any(|c| c.from == 0 && c.out == Port::Audio && c.to == 1 && c.input == Port::Slot(0)));
}

/// A chain of three nodes deleted and reconnected in one go bypasses the whole run, not just the
/// node next to each neighbour.
#[test]
fn deleting_a_run_of_nodes_in_place_bypasses_all_of_them() {
    let mut graph = Graph::with(
        vec![
            FlodeNode::new(NodeKind::Oscillator, Point::new(0.0, 80.0)),
            FlodeNode::new(NodeKind::Filter, Point::new(200.0, 80.0)),
            FlodeNode::new(NodeKind::Distortion, Point::new(400.0, 80.0)),
            FlodeNode::new(NodeKind::Chorus, Point::new(600.0, 80.0)),
            FlodeNode::new(NodeKind::Output, Point::new(900.0, 80.0)),
        ],
        Vec::new(),
    );
    graph.connect(0, Port::Audio, 1, Port::Audio);
    graph.connect(1, Port::Audio, 2, Port::Audio);
    graph.connect(2, Port::Audio, 3, Port::Audio);
    graph.connect(3, Port::Audio, 4, Port::Slot(0));
    graph.select(1);
    graph.also_selected = vec![2, 3];
    graph.delete_selected_in_place();
    assert_eq!(graph.nodes.len(), 2);
    assert!(graph
        .connections
        .iter()
        .any(|c| c.from == 0 && c.out == Port::Audio && c.to == 1 && c.input == Port::Slot(0)));
}

/// A node with no single clear through-path, such as the Output node or an unwired source, is
/// just removed: there is nothing to reconnect.
#[test]
fn a_node_with_no_through_path_is_simply_removed() {
    let mut graph = wired_chain_with_gap();
    let before = graph.connections.len();
    graph.delete_in_place(2); // the Output node: audio in, nothing out
    assert_eq!(graph.nodes.len(), 2);
    // The wire that fed the Output is gone with it, same as a plain delete.
    assert!(graph.connections.len() < before);
}

/// Deleting and reconnecting a node whose input has more than one wire, such as a mix node's A
/// and B both wired, only carries the one on its through-path (the first audio input) forward.
#[test]
fn only_the_through_path_wire_carries_over() {
    let mut graph = Graph::with(
        vec![
            FlodeNode::new(NodeKind::Oscillator, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Oscillator, Point::new(0.0, 200.0)),
            FlodeNode::new(NodeKind::Mix, Point::new(300.0, 100.0)),
            FlodeNode::new(NodeKind::Output, Point::new(600.0, 100.0)),
        ],
        Vec::new(),
    );
    graph.connect(0, Port::Audio, 2, Port::AudioIn(0));
    graph.connect(1, Port::Audio, 2, Port::AudioIn(1));
    graph.connect(2, Port::Audio, 3, Port::Slot(0));
    graph.delete_in_place(2);
    assert_eq!(graph.nodes.len(), 3);
    assert!(graph
        .connections
        .iter()
        .any(|c| c.from == 0 && c.to == 2 && c.input == Port::Slot(0)));
}

/// The right-click menu offers Delete and reconnect next to Delete, and it does the same thing
/// the shortcut does.
#[test]
fn the_menu_entry_deletes_and_reconnects() {
    use crate::editor::menus::{menu_bounds, MenuItem};
    let mut graph = wired_chain_with_gap();
    let target = graph.nodes[1].position;
    let inside = Point::new(target.x + 20.0, target.y + 12.0);
    let mut ed = editor(&mut graph);
    ed.update(
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)),
        bounds(),
        Cursor::Available(inside),
    );
    let menu = ed.graph.menu.expect("the menu opens");
    let items = menu.items;
    let row = items
        .iter()
        .position(|&item| item == MenuItem::DeleteInPlace)
        .expect("Delete and reconnect is offered");
    let area = menu_bounds(menu);
    let entry = Point::new(
        area.x + 10.0,
        area.y + (row as f32 + 0.5) * super::style::MENU_ITEM_HEIGHT,
    );
    ed.update(
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        bounds(),
        Cursor::Available(entry),
    );
    assert_eq!(graph.nodes.len(), 2);
    assert!(graph
        .connections
        .iter()
        .any(|c| c.from == 0 && c.to == 1 && c.input == Port::Slot(0)));
}
