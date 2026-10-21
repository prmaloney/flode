//! Tests for the knobs a group node shows on its face, one for each control pin that feeds a knob
//! inside the group, and for the same knobs on the group's input edge node.

use super::canvas::NodeEditor;
use super::graph::{Graph, DEFAULT_OUTPUT_X};
use super::group::PinSide;
use crate::nodes::connection::Connection;
use crate::nodes::group::{GroupPin, GROUP_KNOBS, GROUP_KNOB_COUNT};
use crate::nodes::knobs::KnobId;
use crate::nodes::{FlodeNode, NodeKind, Port, Signal};
use crate::patch::control::ControlLevels;
use nih_plug_iced::mouse;
use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};
use nih_plug_iced::{Point, Rectangle};

const INPUT: usize = 0;
const OSC: usize = 1;
const FILTER: usize = 2;
const REVERB: usize = 3;
const OUTPUT: usize = 4;
const LFO: usize = 5;
const GROUP: usize = 2;

fn node(kind: NodeKind, x: f32) -> FlodeNode {
    FlodeNode::new(kind, Point::new(x, 80.0))
}

fn wire(from: usize, out: Port, to: usize, input: Port) -> Connection {
    Connection {
        from,
        out,
        to,
        input,
    }
}

fn control_pin(n: usize) -> Port {
    Port::Pin(Signal::Control, n)
}

fn bounds() -> Rectangle {
    Rectangle {
        x: 0.0,
        y: 0.0,
        width: 1024.0,
        height: 712.0,
    }
}

/// The usual chain with the filter and the reverb in a group. The filter's cutoff starts at 0.3
/// and an LFO drives it through pin 1 of the group.
fn grouped() -> Graph {
    let mut graph = Graph::with(
        vec![
            node(NodeKind::Input, 0.0),
            node(NodeKind::Oscillator, 200.0),
            node(NodeKind::Filter, 400.0),
            node(NodeKind::Reverb, 600.0),
            node(NodeKind::Output, DEFAULT_OUTPUT_X),
            node(NodeKind::Lfo, 300.0),
        ],
        vec![
            wire(INPUT, Port::Notes, OSC, Port::Notes),
            wire(OSC, Port::Audio, FILTER, Port::Audio),
            wire(FILTER, Port::Audio, REVERB, Port::Audio),
            wire(REVERB, Port::Audio, OUTPUT, Port::Slot(0)),
            wire(LFO, Port::Shape, FILTER, Port::Knob(KnobId::Cutoff)),
        ],
    );
    graph.nodes[FILTER].set_value(KnobId::Cutoff, 0.3);
    graph.refresh_slots();
    assert_eq!(graph.create_group(&[FILTER, REVERB]), Some(GROUP));
    graph
}

/// `grouped`, without the LFO's wire into the group, so the face knob is free.
fn free() -> Graph {
    let mut graph = grouped();
    graph
        .connections
        .retain(|c| c.to != GROUP || c.input != control_pin(1));
    graph
}

fn inner_cutoff(graph: &Graph) -> f32 {
    let data = graph.nodes[GROUP].group.as_ref().expect("a group");
    let filter = data
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Filter)
        .expect("the filter");
    filter.value(KnobId::Cutoff)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

/// Presses on a point, drags it up 60 pixels, and lets go.
fn drag_up(graph: &mut Graph, start: Point) {
    let end = Point::new(start.x, start.y - 60.0);
    let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
    for (event, at) in [
        (mouse::Event::ButtonPressed(mouse::Button::Left), start),
        (mouse::Event::CursorMoved { position: end }, end),
        (mouse::Event::ButtonReleased(mouse::Button::Left), end),
    ] {
        editor.update(Event::Mouse(event), bounds(), Cursor::Available(at));
    }
}

fn face_value(graph: &Graph) -> f32 {
    let open = graph.open_groups.last().expect("a group is open");
    open.nodes[open.group].inputs[1].value
}

#[test]
fn a_pin_that_feeds_a_knob_starts_at_that_knobs_value() {
    let graph = grouped();
    let group = &graph.nodes[GROUP];
    assert_eq!(group.face_knobs(), vec![(1, KnobId::Cutoff)]);
    assert!(close(group.inputs[1].value, 0.3));
    assert_eq!(group.inputs[1].knob, Some(KnobId::Cutoff));
    // The audio pin feeds no knob.
    assert_eq!(group.inputs[0].knob, None);
    // The face knob is on the row of its pin.
    let rows = group.rows();
    assert_eq!(rows[1].knob, Some(GROUP_KNOBS[0]));
    assert_eq!(rows[1].input, Some(control_pin(1)));
    assert_eq!(rows[0].knob, None);
    assert_eq!(group.knob_slots().len(), 1);
    assert!(close(group.value(GROUP_KNOBS[0]), 0.3));
    assert_eq!(group.knob_name(GROUP_KNOBS[0]), "Cutoff");
}

#[test]
fn creating_a_group_changes_no_sound() {
    let mut graph = Graph::with(
        vec![
            node(NodeKind::Oscillator, 0.0),
            node(NodeKind::Filter, 200.0),
        ],
        Vec::new(),
    );
    graph.nodes[1].set_value(KnobId::Cutoff, 0.7);
    let before = format!("{:?}", graph.patch());
    assert_eq!(graph.create_group(&[1]), Some(1));
    assert_eq!(format!("{:?}", graph.patch()), before);
}

#[test]
fn the_knob_follows_the_range_of_the_knob_it_feeds() {
    let mut graph = free();
    let id = GROUP_KNOBS[0];
    let group = &mut graph.nodes[GROUP];
    // A cutoff reads in hertz over its skewed range.
    assert!(group.knob_format(id, group.value(id)).contains("Hz"));
    assert!(close(
        group.plain(id),
        KnobId::Cutoff.to_plain(group.value(id))
    ));
    group.enter_knob_value(id, 1000.0);
    assert!((group.plain(id) - 1000.0).abs() < 1.0);
    // The entry box shows the same number the knob reads.
    let text = group.knob_entry_text(id);
    assert!(
        text.parse::<f32>().is_ok_and(|n| (n - 1000.0).abs() < 2.0),
        "{text}"
    );
    // A reset goes to the cutoff's default.
    group.reset_value(id);
    assert!(close(group.value(id), KnobId::Cutoff.default_normalized()));
}

#[test]
fn setting_the_face_value_sets_the_knob_inside_and_the_patch() {
    let mut graph = free();
    let before = format!("{:?}", graph.patch());
    graph.nodes[GROUP].set_value(GROUP_KNOBS[0], 0.9);
    assert!(close(graph.nodes[GROUP].inputs[1].value, 0.9));
    assert!(close(inner_cutoff(&graph), 0.9));
    assert_ne!(format!("{:?}", graph.patch()), before);
    // A typed number goes through as well.
    graph.nodes[GROUP].enter_knob_value(GROUP_KNOBS[0], 500.0);
    let plain = KnobId::Cutoff.to_plain(inner_cutoff(&graph));
    assert!((plain - 500.0).abs() < 1.0);
}

#[test]
fn dragging_the_face_knob_changes_the_value() {
    let mut graph = free();
    let start = graph.nodes[GROUP].knob_slots()[0].center;
    drag_up(&mut graph, start);
    assert!(graph.nodes[GROUP].inputs[1].value > 0.3 + 0.05);
    assert!(close(
        inner_cutoff(&graph),
        graph.nodes[GROUP].inputs[1].value
    ));
}

#[test]
fn a_wired_pin_locks_its_knob() {
    let mut graph = grouped();
    let id = GROUP_KNOBS[0];
    assert!(graph.is_driven(GROUP, id));
    let slot = graph.nodes[GROUP].knob_slots()[0];
    assert!(slot.pinned);
    drag_up(&mut graph, slot.center);
    assert!(close(graph.nodes[GROUP].inputs[1].value, 0.3));
    // With the wire gone it is free again.
    graph.connections.retain(|c| c.input != control_pin(1));
    assert!(!graph.is_driven(GROUP, id));
}

#[test]
fn a_pin_that_feeds_no_knob_shows_none() {
    let mut graph = free();
    assert!(graph.open_group(GROUP));
    // A control pin with nothing inside, and an audio pin.
    graph.add_pin(PinSide::Input, "Spare", Signal::Control);
    graph.add_pin(PinSide::Input, "Sound", Signal::Audio);
    // The input edge node shows no knob for them either.
    assert_eq!(graph.nodes[0].face_knobs(), vec![(1, KnobId::Cutoff)]);
    assert!(graph.close_group());
    let group = &graph.nodes[GROUP];
    assert_eq!(group.face_knobs(), vec![(1, KnobId::Cutoff)]);
    assert_eq!(group.rows().iter().filter(|r| r.knob.is_some()).count(), 1);
}

#[test]
fn a_pin_dropped_on_a_knob_takes_that_knobs_value() {
    let mut graph = free();
    assert!(graph.open_group(GROUP));
    let reverb = graph
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::Reverb)
        .unwrap();
    graph.nodes[reverb].set_value(KnobId::Mix, 0.8);
    let edge = graph
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::GroupInput)
        .unwrap();
    assert!(graph.drop_wire(edge, Port::NewPin, reverb, Port::Knob(KnobId::Mix)));
    // The edge node shows the knob at once.
    assert_eq!(graph.nodes[edge].face_knobs().len(), 2);
    assert!(graph.close_group());
    let group = &graph.nodes[GROUP];
    assert_eq!(group.face_knobs().len(), 2);
    let (pin, target) = group.face_knobs()[1];
    assert_eq!(target, KnobId::Mix);
    assert!(close(group.inputs[pin].value, 0.8));
}

#[test]
fn removing_a_pin_keeps_the_values_of_the_others() {
    let mut graph = free();
    assert!(graph.open_group(GROUP));
    let reverb = graph
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::Reverb)
        .unwrap();
    graph.nodes[reverb].set_value(KnobId::Mix, 0.8);
    let edge = graph
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::GroupInput)
        .unwrap();
    assert!(graph.drop_wire(edge, Port::NewPin, reverb, Port::Knob(KnobId::Mix)));
    assert!(graph.close_group());
    graph.nodes[GROUP].set_value(GROUP_KNOBS[1], 0.65);
    assert!(graph.open_group(GROUP));
    // Pin 0 is audio, pin 1 the cutoff, and pin 2 the mix. Taking out the cutoff pin renumbers
    // the mix pin.
    assert!(graph.remove_pin(PinSide::Input, 1));
    assert!(graph.rename_pin(PinSide::Input, 1, "Wet"));
    assert!(graph.close_group());
    let group = &graph.nodes[GROUP];
    assert_eq!(group.face_knobs(), vec![(1, KnobId::Mix)]);
    assert!(close(group.inputs[1].value, 0.65));
    assert_eq!(group.inputs[1].name, "Wet");
    assert!(close(group.value(GROUP_KNOBS[0]), 0.65));
}

#[test]
fn face_values_survive_saving_loading_and_copying() {
    let mut graph = free();
    graph.nodes[GROUP].set_value(GROUP_KNOBS[0], 0.8);
    let json = serde_json::to_string(&graph.to_saved()).expect("writes");
    let loaded = Graph::from_saved(&serde_json::from_str(&json).expect("reads"));
    let group = &loaded.nodes[GROUP];
    assert_eq!(group.face_knobs(), vec![(1, KnobId::Cutoff)]);
    assert!(close(group.inputs[1].value, 0.8));
    assert_eq!(
        format!("{:?}", loaded.patch()),
        format!("{:?}", graph.patch())
    );

    // A copy has its own value, and its own knob inside.
    let mut copy = graph.nodes[GROUP].duplicate(Point::new(0.0, 0.0));
    assert!(close(copy.inputs[1].value, 0.8));
    copy.set_value(GROUP_KNOBS[0], 0.2);
    assert!(close(graph.nodes[GROUP].inputs[1].value, 0.8));
    assert!(close(inner_cutoff(&graph), 0.8));
}

#[test]
fn a_save_made_inside_the_open_group_keeps_the_face_value() {
    let mut graph = free();
    graph.nodes[GROUP].set_value(GROUP_KNOBS[0], 0.8);
    assert!(graph.open_group(GROUP));
    let loaded = Graph::from_saved(&graph.to_saved());
    assert!(close(loaded.nodes[GROUP].inputs[1].value, 0.8));
    // Coming back out does not overwrite it.
    assert!(graph.close_group());
    assert!(close(graph.nodes[GROUP].inputs[1].value, 0.8));
}

#[test]
fn inside_the_open_group_a_fed_knob_is_locked() {
    let mut graph = free();
    graph.nodes[GROUP].set_value(GROUP_KNOBS[0], 0.8);
    assert!(graph.open_group(GROUP));
    let filter = graph
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::Filter)
        .unwrap();
    let reverb = graph
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::Reverb)
        .unwrap();
    // The cutoff is fed by the pin, even though nothing outside is wired to it.
    assert!(graph.is_driven(filter, KnobId::Cutoff));
    assert!(close(graph.nodes[filter].value(KnobId::Cutoff), 0.8));
    assert!(!graph.is_driven(reverb, KnobId::Mix));
    // Wiring the pin to a second knob locks it and gives it the face value.
    let edge = graph
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::GroupInput)
        .unwrap();
    graph.connect(edge, control_pin(1), reverb, Port::Knob(KnobId::Mix));
    assert!(graph.is_driven(reverb, KnobId::Mix));
    assert!(close(graph.nodes[reverb].value(KnobId::Mix), 0.8));
}

#[test]
fn a_group_shows_at_most_eight_face_knobs() {
    let mut group = FlodeNode::new(NodeKind::Group, Point::new(0.0, 0.0));
    for n in 0..GROUP_KNOB_COUNT + 3 {
        let mut pin = GroupPin::new(format!("P{n}"), Signal::Control);
        pin.knob = Some(KnobId::Mix);
        group.inputs.push(pin);
    }
    assert_eq!(group.face_knobs().len(), GROUP_KNOB_COUNT);
    assert_eq!(
        group.rows().iter().filter(|row| row.knob.is_some()).count(),
        GROUP_KNOB_COUNT
    );
}

#[test]
fn a_pin_that_reaches_a_knob_only_through_a_nested_group_has_no_face_knob() {
    let mut graph = free();
    assert!(graph.open_group(GROUP));
    let filter = graph
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::Filter)
        .unwrap();
    assert!(graph.create_group(&[filter]).is_some());
    assert!(graph.close_group());
    // The outer pin now feeds the inner group's pin, which is not a knob.
    assert!(graph.nodes[GROUP].face_knobs().is_empty());
}

// The same knob on the group's input edge node.

#[test]
fn the_input_edge_node_shows_the_same_knob() {
    let mut graph = free();
    graph.nodes[GROUP].set_value(GROUP_KNOBS[0], 0.6);
    assert!(graph.open_group(GROUP));
    let edge = &graph.nodes[0];
    assert_eq!(edge.kind, NodeKind::GroupInput);
    assert_eq!(edge.face_knobs(), vec![(1, KnobId::Cutoff)]);
    let id = GROUP_KNOBS[0];
    assert_eq!(edge.knob_slots().len(), 1);
    assert_eq!(edge.rows()[1].knob, Some(id));
    assert!(close(edge.value(id), 0.6));
    assert_eq!(edge.knob_name(id), "Cutoff");
    assert!(edge.knob_format(id, edge.value(id)).contains("Hz"));
    // The audio pin has none, and neither does the empty pin.
    assert_eq!(edge.rows()[0].knob, None);
    assert_eq!(edge.rows().last().and_then(|r| r.knob), None);
}

#[test]
fn dragging_the_edge_knob_updates_the_face_and_the_knobs_inside() {
    let mut graph = free();
    assert!(graph.open_group(GROUP));
    let centre = graph.nodes[0].knob_slots()[0].center;
    let start = graph.view.to_screen(centre);
    drag_up(&mut graph, start);
    let value = graph.nodes[0].outputs[1].value;
    assert!(value > 0.3 + 0.05, "{value}");
    assert!(close(face_value(&graph), value));
    let filter = graph
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::Filter)
        .unwrap();
    assert!(close(graph.nodes[filter].value(KnobId::Cutoff), value));
    // The sound follows, and the value is on the face after closing.
    let patch_open = format!("{:?}", graph.patch());
    assert!(graph.close_group());
    assert!(close(graph.nodes[GROUP].inputs[1].value, value));
    assert!(close(inner_cutoff(&graph), value));
    assert_eq!(format!("{:?}", graph.patch()), patch_open);
}

#[test]
fn the_edge_knob_resets_and_takes_typed_values() {
    let mut graph = free();
    assert!(graph.open_group(GROUP));
    let id = GROUP_KNOBS[0];
    graph.open_knob_entry(0, id);
    let entry = graph.knob_entry.as_mut().expect("the box opens");
    entry.text = "2000".to_string();
    entry.fresh = false;
    assert!(graph.confirm_knob_entry());
    let plain = graph.nodes[0].plain(id);
    assert!((plain - 2000.0).abs() < 2.0, "{plain}");
    assert!(close(face_value(&graph), graph.nodes[0].value(id)));
    assert!((KnobId::Cutoff.to_plain(inner_cutoff_open(&graph)) - 2000.0).abs() < 2.0);
    graph.nodes[0].reset_value(id);
    graph.push_edge_values(0);
    assert!(close(
        face_value(&graph),
        KnobId::Cutoff.default_normalized()
    ));
}

fn inner_cutoff_open(graph: &Graph) -> f32 {
    graph
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Filter)
        .map(|n| n.value(KnobId::Cutoff))
        .expect("the filter")
}

#[test]
fn a_pin_wired_in_the_parent_locks_the_edge_knob() {
    let mut graph = grouped();
    assert!(graph.open_group(GROUP));
    let id = GROUP_KNOBS[0];
    assert!(graph.is_driven(0, id));
    let centre = graph.nodes[0].knob_slots()[0].center;
    let start = graph.view.to_screen(centre);
    drag_up(&mut graph, start);
    assert!(close(graph.nodes[0].outputs[1].value, 0.3));
    assert!(close(face_value(&graph), 0.3));
}

#[test]
fn the_edge_knob_appears_when_a_wire_inside_feeds_a_knob() {
    let mut graph = free();
    assert!(graph.open_group(GROUP));
    let reverb = graph
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::Reverb)
        .unwrap();
    graph.nodes[reverb].set_value(KnobId::Mix, 0.7);
    let edge = 0;
    graph.add_pin(PinSide::Input, "Wet", Signal::Control);
    // No knob while the pin feeds nothing.
    assert_eq!(graph.nodes[edge].face_knobs().len(), 1);
    graph.connect(edge, control_pin(2), reverb, Port::Knob(KnobId::Mix));
    assert_eq!(graph.nodes[edge].face_knobs().len(), 2);
    assert!(close(graph.nodes[edge].outputs[2].value, 0.7));
    // Taking the wire away takes the knob away.
    graph.disconnect_input(reverb, Port::Knob(KnobId::Mix));
    assert_eq!(graph.nodes[edge].face_knobs().len(), 1);
}
