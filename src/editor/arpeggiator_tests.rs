//! Tests for the arpeggiator wired up through the editor's graph model: that it compiles on its
//! own, alongside an oscillator still wired straight to a MIDI input in the same patch, and that it
//! still works once it sits inside a group.

use super::graph::{Graph, DEFAULT_OUTPUT_X};
use super::saved::SavedNode;
use crate::nodes::connection::Connection;
use crate::nodes::knobs::KnobId;
use crate::nodes::{arpeggiator, FlodeNode, NodeKind, Port};
use crate::patch::stage::StageParams;
use crate::patch::NoteSourceSpec;
use nih_plug_iced::Point;

const INPUT: usize = 0;
const ARP: usize = 1;
const ARPED_OSC: usize = 2;
const RAW_OSC: usize = 3;
const OUTPUT: usize = 4;

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

/// One MIDI input feeding two oscillators in different output slots: one through an arpeggiator,
/// and one wired straight to the input, bypassing it.
fn graph_with_both_paths() -> Graph {
    let mut graph = Graph::with(
        vec![
            node(NodeKind::Input, 0.0),
            node(NodeKind::Arpeggiator, 200.0),
            node(NodeKind::Oscillator, 400.0),
            node(NodeKind::Oscillator, 400.0),
            node(NodeKind::Output, DEFAULT_OUTPUT_X),
        ],
        vec![
            wire(INPUT, Port::Notes, ARP, Port::Notes),
            wire(ARP, Port::Notes, ARPED_OSC, Port::Notes),
            wire(INPUT, Port::Notes, RAW_OSC, Port::Notes),
            wire(ARPED_OSC, Port::Audio, OUTPUT, Port::Slot(0)),
            wire(RAW_OSC, Port::Audio, OUTPUT, Port::Slot(1)),
        ],
    );
    graph.refresh_slots();
    graph
}

/// Both oscillators compile: one reads the arpeggiator's slot, and the other reads the MIDI input's
/// slot directly, so the same host notes reach both at once, one of them reshaped.
#[test]
fn an_arpeggiated_slot_and_a_raw_slot_both_compile_from_one_input() {
    let graph = graph_with_both_paths();
    let patch = graph.patch();
    assert!(patch.lanes[0].is_active());
    assert!(patch.lanes[1].is_active());

    assert_eq!(patch.note_sources[0], Some(NoteSourceSpec::Input));
    let Some(NoteSourceSpec::Arpeggiator(settings)) = patch.note_sources[1] else {
        panic!("expected an arpeggiator at slot 1");
    };
    assert_eq!(settings.input, Some(0));

    let StageParams::Oscillator(ref arped) = patch.lanes[0].stages[0].params else {
        panic!("expected an oscillator");
    };
    assert_eq!(arped.notes, Some(1));
    assert_eq!(patch.lanes[0].notes_slot, Some(1));

    let StageParams::Oscillator(ref raw) = patch.lanes[1].stages[0].params else {
        panic!("expected an oscillator");
    };
    assert_eq!(raw.notes, Some(0));
    assert_eq!(patch.lanes[1].notes_slot, Some(0));
}

/// An arpeggiator still works once it is pulled into a group: the group's pins carry its Notes
/// wires the same way they already carry Audio or Control ones.
#[test]
fn an_arpeggiator_inside_a_group_still_feeds_its_oscillator() {
    let mut graph = graph_with_both_paths();
    assert_eq!(graph.create_group(&[ARP]), Some(ARP));
    assert_eq!(graph.nodes[ARP].kind, NodeKind::Group);

    let patch = graph.patch();
    assert!(patch.lanes[0].is_active(), "the arpeggiated slot plays");
    assert!(patch.lanes[1].is_active(), "the raw slot still plays");
    let has_arpeggiator = patch
        .note_sources
        .iter()
        .any(|slot| matches!(slot, Some(NoteSourceSpec::Arpeggiator(_))));
    assert!(has_arpeggiator, "the grouped arpeggiator still registers");
}

/// An arpeggiator's own panel state, its pattern dropdown and its knobs, needs no save-format
/// field of its own: it round-trips through the same `choices` and `knobs` every node already
/// saves with, the way `env_configs` and `Lfos` are compiler and engine state rather than saved
/// state, and never need a save field either.
#[test]
fn an_arpeggiators_panel_round_trips_through_the_generic_save_fields() {
    let mut node = FlodeNode::new(NodeKind::Arpeggiator, Point::new(0.0, 0.0));
    node.choose(0, 2); // Up-down
    node.set_value(KnobId::LfoRate, KnobId::LfoRate.to_normalized(3.0));
    node.set_value(KnobId::ArpOctaves, KnobId::ArpOctaves.to_normalized(3.0));
    node.set_value(KnobId::ArpGate, KnobId::ArpGate.to_normalized(0.4));

    let loaded = SavedNode::new(&node).into_node();
    assert_eq!(arpeggiator::pattern(&loaded), arpeggiator::pattern(&node));
    assert_eq!(loaded.value(KnobId::LfoRate), node.value(KnobId::LfoRate));
    assert_eq!(
        loaded.value(KnobId::ArpOctaves),
        node.value(KnobId::ArpOctaves)
    );
    assert_eq!(loaded.value(KnobId::ArpGate), node.value(KnobId::ArpGate));
}
