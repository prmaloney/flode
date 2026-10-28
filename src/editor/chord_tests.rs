//! Tests for the chord node wired up through the editor's graph model: that it compiles on its own,
//! that it chains with an arpeggiator in both directions, that it splices into a notes wire the same
//! way an arpeggiator does, and that its panel round trips through a save.

use super::graph::{Graph, DEFAULT_OUTPUT_X};
use super::saved::SavedNode;
use crate::nodes::chord::ChordType;
use crate::nodes::connection::Connection;
use crate::nodes::knobs::KnobId;
use crate::nodes::{chord, FlodeNode, NodeKind, Port};
use crate::patch::stage::StageParams;
use crate::patch::NoteSourceSpec;
use nih_plug_iced::Point;

const INPUT: usize = 0;
const CHORD: usize = 1;
const OSC: usize = 2;
const OUTPUT: usize = 3;

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

/// A MIDI input into a chord into an oscillator.
fn graph_with_chord() -> Graph {
    let mut graph = Graph::with(
        vec![
            node(NodeKind::Input, 0.0),
            node(NodeKind::Chord, 200.0),
            node(NodeKind::Oscillator, 400.0),
            node(NodeKind::Output, DEFAULT_OUTPUT_X),
        ],
        vec![
            wire(INPUT, Port::Notes, CHORD, Port::Notes),
            wire(CHORD, Port::Notes, OSC, Port::Notes),
            wire(OSC, Port::Audio, OUTPUT, Port::Slot(0)),
        ],
    );
    graph.refresh_slots();
    graph
}

/// A chord node compiles to a `NoteSourceSpec::Chord` reading its MIDI input's slot, and the
/// oscillator it feeds reads the chord's own slot.
#[test]
fn a_chord_compiles_and_feeds_its_oscillator() {
    let graph = graph_with_chord();
    let patch = graph.patch();
    assert!(patch.lanes[0].is_active());
    assert_eq!(patch.note_sources[0], Some(NoteSourceSpec::Input));
    let Some(NoteSourceSpec::Chord(settings)) = patch.note_sources[1] else {
        panic!("expected a chord at slot 1");
    };
    assert_eq!(settings.input, Some(0));
    assert_eq!(settings.chord, ChordType::Major);

    let StageParams::Oscillator(ref osc) = patch.lanes[0].stages[0].params else {
        panic!("expected an oscillator");
    };
    assert_eq!(osc.notes, Some(1));
    assert_eq!(patch.lanes[0].notes_slot, Some(1));
}

/// Three kinds of note source share one counter: a MIDI input, an arpeggiator reading it, and a
/// chord reading the arpeggiator, each land in the next free slot and each names the slot that
/// actually feeds it.
#[test]
fn a_chord_fed_by_an_arpeggiator_reads_the_right_upstream_slot() {
    let mut graph = Graph::with(
        vec![
            node(NodeKind::Input, 0.0),       // 0
            node(NodeKind::Arpeggiator, 1.0), // 1
            node(NodeKind::Chord, 2.0),       // 2
            node(NodeKind::Oscillator, 3.0),  // 3
            node(NodeKind::Output, DEFAULT_OUTPUT_X),
        ],
        vec![
            wire(0, Port::Notes, 1, Port::Notes),
            wire(1, Port::Notes, 2, Port::Notes),
            wire(2, Port::Notes, 3, Port::Notes),
            wire(3, Port::Audio, 4, Port::Slot(0)),
        ],
    );
    graph.refresh_slots();
    let patch = graph.patch();

    assert_eq!(patch.note_sources[0], Some(NoteSourceSpec::Input));
    let Some(NoteSourceSpec::Arpeggiator(arp)) = patch.note_sources[1] else {
        panic!("expected an arpeggiator at slot 1");
    };
    assert_eq!(arp.input, Some(0));
    let Some(NoteSourceSpec::Chord(chord)) = patch.note_sources[2] else {
        panic!("expected a chord at slot 2");
    };
    assert_eq!(chord.input, Some(1));
    assert_eq!(patch.lanes[0].notes_slot, Some(2));
}

/// The same chain the other way: an arpeggiator reading a chord's output still finds the chord's
/// own slot.
#[test]
fn an_arpeggiator_fed_by_a_chord_reads_the_right_upstream_slot() {
    let mut graph = Graph::with(
        vec![
            node(NodeKind::Input, 0.0),       // 0
            node(NodeKind::Chord, 1.0),       // 1
            node(NodeKind::Arpeggiator, 2.0), // 2
            node(NodeKind::Oscillator, 3.0),  // 3
            node(NodeKind::Output, DEFAULT_OUTPUT_X),
        ],
        vec![
            wire(0, Port::Notes, 1, Port::Notes),
            wire(1, Port::Notes, 2, Port::Notes),
            wire(2, Port::Notes, 3, Port::Notes),
            wire(3, Port::Audio, 4, Port::Slot(0)),
        ],
    );
    graph.refresh_slots();
    let patch = graph.patch();

    let Some(NoteSourceSpec::Chord(chord)) = patch.note_sources[1] else {
        panic!("expected a chord at slot 1");
    };
    assert_eq!(chord.input, Some(0));
    let Some(NoteSourceSpec::Arpeggiator(arp)) = patch.note_sources[2] else {
        panic!("expected an arpeggiator at slot 2");
    };
    assert_eq!(arp.input, Some(1));
}

/// Dropping a chord onto an existing notes wire splices it in, the same as an arpeggiator does.
#[test]
fn dropping_a_chord_on_a_notes_wire_splices_it_in() {
    let mut graph = Graph::with(
        vec![
            FlodeNode::new(NodeKind::Input, Point::new(0.0, 80.0)),
            FlodeNode::new(
                NodeKind::Oscillator,
                Point::new(crate::nodes::NODE_WIDTH + 400.0, 80.0),
            ),
        ],
        Vec::new(),
    );
    graph.connect(0, Port::Notes, 1, Port::Notes);
    let curve = graph.link_curve(&graph.connections[0]).unwrap();
    let middle = super::draw::bezier_point(&curve, 0.5);
    let chord = graph.add_node(
        NodeKind::Chord,
        Point::new(middle.x - 80.0, middle.y - 20.0),
    );
    let link = graph
        .link_under(chord)
        .expect("the wire from the input to the oscillator passes under it");
    graph.insert_into(chord, link);
    assert!(graph
        .connections
        .iter()
        .any(|c| c.from == 0 && c.out == Port::Notes && c.to == chord && c.input == Port::Notes));
    assert!(graph
        .connections
        .iter()
        .any(|c| c.from == chord && c.out == Port::Notes && c.to == 1 && c.input == Port::Notes));
    assert_eq!(graph.connections.len(), 2);
}

/// A chord's panel, its chord-type dropdown and its spread knob, needs no save-format field of its
/// own: it round-trips through the same `choices` and `knobs` every node already saves with.
#[test]
fn a_chords_panel_round_trips_through_the_generic_save_fields() {
    let mut node = FlodeNode::new(NodeKind::Chord, Point::new(0.0, 0.0));
    node.choose(0, 3); // Minor 7
    node.set_value(KnobId::ChordSpread, KnobId::ChordSpread.to_normalized(0.6));

    let loaded = SavedNode::new(&node).into_node();
    assert_eq!(chord::chord_type(&loaded), chord::chord_type(&node));
    assert_eq!(
        loaded.value(KnobId::ChordSpread),
        node.value(KnobId::ChordSpread)
    );
}
