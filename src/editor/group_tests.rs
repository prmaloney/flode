//! Tests for node groups: making them, opening them, editing their pins, compiling them, and
//! saving them.

use super::graph::{Graph, DEFAULT_OUTPUT_X};
use super::group::PinSide;
use super::menus::quick_add_results;
use crate::nodes::connection::Connection;
use crate::nodes::knobs::KnobId;
use crate::nodes::{Category, FlodeNode, NodeKind, Port, Signal};
use nih_plug_iced::Point;

const INPUT: usize = 0;
const OSC: usize = 1;
const FILTER: usize = 2;
const REVERB: usize = 3;
const OUTPUT: usize = 4;
const LFO: usize = 5;

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

/// MIDI input into an oscillator, through a filter and a reverb to the output, with an LFO
/// driving the filter's cutoff. The filter and the reverb are the nodes the tests group.
fn chain() -> Graph {
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
    graph.nodes[REVERB].set_value(KnobId::Mix, 0.6);
    graph.refresh_slots();
    graph
}

fn audio_pin(n: usize) -> Port {
    Port::Pin(Signal::Audio, n)
}

fn control_pin(n: usize) -> Port {
    Port::Pin(Signal::Control, n)
}

/// What the audio thread would play, as text, so two patches compare and show their difference.
fn patch_text(graph: &Graph) -> String {
    format!("{:?}", graph.patch())
}

/// The wires of a graph in a fixed order, for comparing two graphs that list them differently.
fn sorted_links(graph: &Graph) -> Vec<String> {
    let mut links: Vec<String> = graph.connections.iter().map(|c| format!("{c:?}")).collect();
    links.sort();
    links
}

fn all_valid(graph: &Graph) -> bool {
    graph.connections.iter().all(|c| c.is_valid(&graph.nodes))
}

fn grouped() -> Graph {
    let mut graph = chain();
    assert_eq!(graph.create_group(&[FILTER, REVERB]), Some(FILTER));
    graph
}

#[test]
fn a_group_is_made_of_the_selected_nodes_and_pins_for_the_wires_that_crossed() {
    let graph = grouped();
    // The group takes the first member's place, so the output and LFO move down by one.
    let kinds: Vec<NodeKind> = graph.nodes.iter().map(|n| n.kind).collect();
    assert_eq!(
        kinds,
        vec![
            NodeKind::Input,
            NodeKind::Oscillator,
            NodeKind::Group,
            NodeKind::Output,
            NodeKind::Lfo,
        ]
    );
    let group = &graph.nodes[2];
    let names = |pins: &[crate::nodes::group::GroupPin]| -> Vec<String> {
        pins.iter().map(|p| p.name.clone()).collect()
    };
    assert_eq!(names(&group.inputs), vec!["Audio", "Cutoff"]);
    assert_eq!(group.inputs[0].signal, Signal::Audio);
    assert_eq!(group.inputs[1].signal, Signal::Control);
    assert_eq!(names(&group.outputs), vec!["Audio"]);
    assert_eq!(group.label(), "Group");
    assert!(all_valid(&graph));
    assert_eq!(graph.connections.len(), 4);
    assert!(graph
        .connections
        .contains(&wire(1, Port::Audio, 2, audio_pin(0))));
    assert!(graph
        .connections
        .contains(&wire(4, Port::Shape, 2, control_pin(1))));
    assert!(graph
        .connections
        .contains(&wire(2, audio_pin(0), 3, Port::Slot(0))));
    assert_eq!(graph.nodes[3].used, 1);
    assert_eq!(graph.selection(), vec![2]);

    // Inside: the two edge nodes, then the members, joined by the wire between them.
    let data = group.group.as_ref().unwrap();
    let inside: Vec<NodeKind> = data.nodes.iter().map(|n| n.kind).collect();
    assert_eq!(
        inside,
        vec![
            NodeKind::GroupInput,
            NodeKind::GroupOutput,
            NodeKind::Filter,
            NodeKind::Reverb
        ]
    );
    assert_eq!(data.nodes[0].outputs, group.inputs);
    assert_eq!(data.nodes[1].inputs, group.outputs);
    assert!(data.links.contains(&wire(2, Port::Audio, 3, Port::Audio)));
    assert!(data.links.contains(&wire(0, audio_pin(0), 2, Port::Audio)));
    assert!(data
        .links
        .contains(&wire(0, control_pin(1), 2, Port::Knob(KnobId::Cutoff))));
    assert!(data.links.contains(&wire(3, Port::Audio, 1, audio_pin(0))));
    assert!(data.links.iter().all(|c| c.is_valid(&data.nodes)));
    // The edge nodes sit left and right of the members.
    assert!(data.nodes[0].position.x < data.nodes[2].position.x);
    assert!(data.nodes[1].position.x > data.nodes[3].position.x);
    assert_eq!(group.position, data.nodes[2].position);
}

#[test]
fn a_group_compiles_to_the_same_patch_as_the_nodes_by_hand() {
    let by_hand = chain();
    let mut graph = chain();
    let before = patch_text(&graph);
    assert!(before.contains("Filter"));
    assert!(graph.create_group(&[FILTER, REVERB]).is_some());
    assert_eq!(patch_text(&graph), before);
    assert_eq!(graph.patch(), by_hand.patch());
}

#[test]
fn the_sound_keeps_playing_while_a_group_is_open() {
    let mut graph = grouped();
    let closed = graph.patch();
    assert!(graph.open_group(2));
    assert_eq!(graph.patch(), closed);
    graph.nodes[2].set_value(KnobId::Cutoff, 0.9);
    let edited = graph.patch();
    assert_ne!(edited, closed);
    assert!(graph.close_group());
    assert_eq!(graph.patch(), edited);
}

#[test]
fn a_source_that_feeds_several_inside_inputs_uses_one_pin() {
    let mut graph = Graph::with(
        vec![
            node(NodeKind::Oscillator, 0.0),
            node(NodeKind::Filter, 200.0),
            node(NodeKind::Filter, 400.0),
            node(NodeKind::Mix, 600.0),
            node(NodeKind::Reverb, 800.0),
            node(NodeKind::Delay, 800.0),
        ],
        vec![
            wire(0, Port::Audio, 1, Port::Audio),
            wire(0, Port::Audio, 2, Port::Audio),
            wire(1, Port::Audio, 3, Port::AudioIn(0)),
            wire(2, Port::Audio, 3, Port::AudioIn(1)),
            wire(3, Port::Audio, 4, Port::Audio),
            wire(3, Port::Audio, 5, Port::Audio),
        ],
    );
    assert_eq!(graph.create_group(&[1, 2, 3]), Some(1));
    let group = &graph.nodes[1];
    assert_eq!(group.inputs.len(), 1);
    assert_eq!(group.outputs.len(), 1);
    assert_eq!(graph.connections.len(), 3);
    let data = group.group.as_ref().unwrap();
    let from_pin = data
        .links
        .iter()
        .filter(|c| c.from == 0 && c.out == audio_pin(0))
        .count();
    assert_eq!(from_pin, 2);
    assert!(all_valid(&graph));
    // Both outside inputs hang off the one output pin.
    let out_pin = graph
        .connections
        .iter()
        .filter(|c| c.from == 1 && c.out == audio_pin(0))
        .count();
    assert_eq!(out_pin, 2);
}

#[test]
fn pins_with_the_same_natural_name_get_numbers() {
    let mut graph = Graph::with(
        vec![
            node(NodeKind::Oscillator, 0.0),
            node(NodeKind::Oscillator, 0.0),
            node(NodeKind::Mix, 200.0),
        ],
        vec![
            wire(0, Port::Audio, 2, Port::Audio),
            wire(1, Port::Audio, 2, Port::AudioIn(1)),
        ],
    );
    assert_eq!(graph.create_group(&[2]), Some(2));
    let names: Vec<&str> = graph.nodes[2]
        .inputs
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(names.len(), 2);
    assert_ne!(names[0], names[1]);
}

#[test]
fn members_that_cannot_be_grouped_are_dropped() {
    let mut graph = chain();
    assert_eq!(graph.create_group(&[OUTPUT]), None);
    assert_eq!(graph.nodes.len(), 6);
    assert_eq!(graph.create_group(&[]), None);
    assert_eq!(graph.create_group(&[99]), None);
    // The output is dropped and the filter is grouped alone.
    assert_eq!(graph.create_group(&[OUTPUT, FILTER, FILTER]), Some(FILTER));
    assert_eq!(graph.nodes.len(), 6);
    assert_eq!(graph.nodes[FILTER].kind, NodeKind::Group);
    assert_eq!(graph.nodes[OUTPUT].kind, NodeKind::Output);
    // The edge nodes of an open group can't be members.
    assert!(graph.open_group(FILTER));
    assert_eq!(graph.create_group(&[0, 1]), None);
}

#[test]
fn a_group_can_hold_an_unwired_node() {
    let mut graph = chain();
    graph.nodes.push(node(NodeKind::Delay, 0.0));
    assert_eq!(graph.create_group(&[6]), Some(6));
    assert!(graph.nodes[6].inputs.is_empty());
    assert!(graph.nodes[6].outputs.is_empty());
    assert_eq!(graph.nodes[6].group.as_ref().unwrap().nodes.len(), 3);
}

#[test]
fn ungroup_puts_the_nodes_and_wires_back() {
    let original = chain();
    let mut graph = chain();
    graph.create_group(&[FILTER, REVERB]);
    assert!(graph.ungroup(FILTER));
    let kinds = |g: &Graph| g.nodes.iter().map(|n| n.kind).collect::<Vec<_>>();
    assert_eq!(kinds(&graph), kinds(&original));
    assert_eq!(sorted_links(&graph), sorted_links(&original));
    for (a, b) in graph.nodes.iter().zip(&original.nodes) {
        assert_eq!(a.position, b.position, "{:?}", a.kind);
    }
    assert_eq!(
        graph.nodes[FILTER].plain(KnobId::Cutoff),
        original.nodes[FILTER].plain(KnobId::Cutoff)
    );
    assert_eq!(graph.selection(), vec![FILTER, REVERB]);
    assert_eq!(graph.nodes[OUTPUT].used, 1);
    assert_eq!(graph.patch(), original.patch());
}

#[test]
fn ungroup_keeps_the_layout_relative_to_the_group_node() {
    let mut graph = grouped();
    graph.nodes[FILTER].position = Point::new(1000.0, 500.0);
    assert!(graph.ungroup(FILTER));
    assert_eq!(graph.nodes[FILTER].position, Point::new(1000.0, 500.0));
    assert_eq!(graph.nodes[REVERB].position, Point::new(1200.0, 500.0));
}

#[test]
fn ungroup_connects_a_pin_wired_straight_across() {
    let mut graph = Graph::with(
        vec![
            node(NodeKind::Oscillator, 0.0),
            node(NodeKind::Filter, 200.0),
            node(NodeKind::Reverb, 400.0),
        ],
        vec![
            wire(0, Port::Audio, 1, Port::Audio),
            wire(1, Port::Audio, 2, Port::Audio),
        ],
    );
    graph.create_group(&[1]);
    assert!(graph.open_group(1));
    // Wire the input pin straight to the output pin, and drop the filter from the path.
    graph.connections.clear();
    graph
        .connections
        .push(wire(0, audio_pin(0), 1, audio_pin(0)));
    assert!(graph.close_group());
    assert!(graph.ungroup(1));
    assert_eq!(
        graph.connections,
        vec![wire(0, Port::Audio, 2, Port::Audio)]
    );
}

#[test]
fn ungroup_refuses_a_node_that_is_not_a_group() {
    let mut graph = chain();
    assert!(!graph.ungroup(FILTER));
    assert!(!graph.ungroup(99));
    assert_eq!(graph.nodes.len(), 6);
}

#[test]
fn opening_a_group_shows_its_graph_and_closing_puts_it_back() {
    let mut graph = grouped();
    graph.set_selection(vec![0, 2]);
    assert_eq!(graph.group_depth(), 0);
    assert!(graph.group_path_names().is_empty());
    assert!(graph.open_group(2));
    assert_eq!(graph.group_depth(), 1);
    assert_eq!(graph.group_path_names(), vec!["Group".to_string()]);
    assert_eq!(graph.nodes.len(), 4);
    assert_eq!(graph.nodes[0].kind, NodeKind::GroupInput);
    assert!(graph.selection().is_empty());
    // The group node has nothing in it while its graph is out on show.
    assert!(graph.open_groups[0].nodes[2]
        .group
        .as_ref()
        .unwrap()
        .nodes
        .is_empty());
    // An edit inside lands in the group.
    graph.add_node(NodeKind::Delay, Point::new(100.0, 300.0));
    assert!(graph.close_group());
    assert_eq!(graph.group_depth(), 0);
    assert_eq!(graph.nodes.len(), 5);
    assert_eq!(graph.nodes[2].group.as_ref().unwrap().nodes.len(), 5);
    assert_eq!(graph.selection(), vec![0, 2]);
    assert!(!graph.close_group());
}

#[test]
fn only_a_group_node_opens() {
    let mut graph = grouped();
    assert!(!graph.open_group(0));
    assert!(!graph.open_group(99));
    assert_eq!(graph.group_depth(), 0);
}

#[test]
fn opening_resets_the_view_and_closing_brings_it_back() {
    let mut graph = grouped();
    graph.view.zoom = 2.0;
    graph.view.offset = nih_plug_iced::Vector::new(40.0, 10.0);
    let before = graph.view;
    assert!(graph.open_group(2));
    assert_ne!(graph.view, before);
    assert!(graph.view.zoom <= 1.0);
    assert!(graph.close_group());
    assert_eq!(graph.view, before);
}

#[test]
fn the_edge_nodes_cannot_be_deleted_or_added() {
    let mut graph = grouped();
    assert!(graph.open_group(2));
    graph.delete_node(0);
    graph.delete_node(1);
    assert_eq!(graph.nodes.len(), 4);
    assert_eq!(graph.duplicate_node(0), None);
    for kind in [NodeKind::Group, NodeKind::GroupInput, NodeKind::GroupOutput] {
        assert!(!graph.can_add(kind));
    }
    // The root already has an output and macros, so a group can't take another.
    assert!(!graph.can_add(NodeKind::Output));
    assert!(!graph.can_add(NodeKind::Macro));
    assert!(graph.can_add(NodeKind::Filter));
}

#[test]
fn the_add_menus_never_offer_group_parts() {
    let graph = chain();
    for kind in [NodeKind::Group, NodeKind::GroupInput, NodeKind::GroupOutput] {
        assert!(!graph.can_add(kind));
        for category in [
            Category::Midi,
            Category::Modulation,
            Category::Effects,
            Category::Utility,
        ] {
            assert!(!category.kinds().contains(&kind));
        }
    }
    let results = quick_add_results("", |kind| graph.can_add(kind));
    assert!(results.iter().all(|kind| !kind.is_group_part()));
    for query in ["group", "in", "out", "g"] {
        let results = quick_add_results(query, |kind| graph.can_add(kind));
        assert!(results.iter().all(|kind| !kind.is_group_part()), "{query}");
    }
}

#[test]
fn deleting_a_group_node_drops_its_wires() {
    let mut graph = grouped();
    graph.delete_node(2);
    assert_eq!(graph.nodes.len(), 4);
    assert_eq!(
        graph.connections,
        vec![wire(0, Port::Notes, 1, Port::Notes)]
    );
    assert_eq!(graph.nodes[2].used, 0);
}

#[test]
fn copying_a_group_copies_its_graph() {
    let mut graph = grouped();
    let copy = graph.duplicate_node(2).unwrap();
    assert_eq!(copy, graph.nodes.len() - 1);
    assert_eq!(graph.nodes[copy].group.as_ref().unwrap().nodes.len(), 4);
    assert_eq!(graph.nodes[copy].inputs, graph.nodes[2].inputs);
    graph.nodes[copy].group.as_mut().unwrap().nodes.pop();
    assert_eq!(graph.nodes[2].group.as_ref().unwrap().nodes.len(), 4);
    // The copy has no wires, and the same graph inside.
    assert!(graph
        .connections
        .iter()
        .all(|c| c.from != copy && c.to != copy));
}

#[test]
fn adding_a_pin_shows_on_both_sides() {
    let mut graph = grouped();
    assert_eq!(
        graph.add_pin(PinSide::Input, "Extra", Signal::Control),
        None
    );
    graph.open_group(2);
    assert_eq!(
        graph.add_pin(PinSide::Input, "Extra", Signal::Control),
        Some(2)
    );
    assert_eq!(graph.add_pin(PinSide::Output, " ", Signal::Audio), Some(1));
    assert_eq!(graph.nodes[0].outputs.len(), 3);
    assert_eq!(graph.nodes[1].inputs[1].name, "Out 2");
    let face = &graph.open_groups[0].nodes[2];
    assert_eq!(face.inputs.len(), 3);
    assert_eq!(face.inputs[2].name, "Extra");
    assert_eq!(face.outputs.len(), 2);
    assert_eq!(graph.nodes[0].output_ports()[2], control_pin(2));
    graph.close_group();
    assert_eq!(graph.nodes[2].input_ports().len(), 3);
    assert_eq!(graph.nodes[2].input_ports()[2], control_pin(2));
}

#[test]
fn removing_a_pin_drops_its_wires_on_both_sides_and_renumbers_the_rest() {
    let mut graph = grouped();
    graph.open_group(2);
    // Drop the audio input pin. The cutoff pin is now pin 0.
    assert!(graph.remove_pin(PinSide::Input, 0));
    assert!(!graph.remove_pin(PinSide::Input, 5));
    assert_eq!(graph.nodes[0].outputs.len(), 1);
    assert_eq!(graph.nodes[0].outputs[0].name, "Cutoff");
    assert_eq!(graph.connections.len(), 3);
    assert!(graph
        .connections
        .contains(&wire(0, control_pin(0), 2, Port::Knob(KnobId::Cutoff))));
    let outside = &graph.open_groups[0];
    assert_eq!(outside.nodes[2].inputs.len(), 1);
    assert_eq!(outside.connections.len(), 3);
    assert!(outside
        .connections
        .contains(&wire(4, Port::Shape, 2, control_pin(0))));
    assert!(!outside.connections.iter().any(|c| c.from == 1));
    graph.close_group();
    assert!(all_valid(&graph));
    // The oscillator no longer reaches the filter, so nothing is wired to the pin.
    assert_eq!(graph.nodes[2].inputs.len(), 1);
}

#[test]
fn removing_an_output_pin_drops_the_wire_to_the_next_node() {
    let mut graph = grouped();
    graph.open_group(2);
    assert!(graph.remove_pin(PinSide::Output, 0));
    assert!(graph.nodes[1].inputs.is_empty());
    assert!(graph.nodes[1..].iter().all(|n| n.kind != NodeKind::Output));
    let outside = &graph.open_groups[0];
    assert!(outside.nodes[2].outputs.is_empty());
    assert!(!outside.connections.iter().any(|c| c.from == 2));
    // The output node's slot count follows the wire that left.
    assert_eq!(outside.nodes[3].used, 0);
    graph.close_group();
    assert!(!graph.patch().is_audible());
}

#[test]
fn renaming_changes_the_pin_and_the_group() {
    let mut graph = grouped();
    assert!(!graph.rename_group("Voice"));
    assert!(graph.rename_group_node(2, "Tone"));
    assert!(!graph.rename_group_node(0, "Nope"));
    assert_eq!(graph.nodes[2].label(), "Tone");
    graph.open_group(2);
    assert!(graph.rename_group("Voice"));
    assert!(!graph.rename_group("  "));
    assert!(graph.rename_pin(PinSide::Input, 1, "Brightness"));
    assert!(!graph.rename_pin(PinSide::Input, 9, "Nope"));
    assert!(!graph.rename_pin(PinSide::Input, 1, ""));
    assert_eq!(graph.nodes[0].outputs[1].name, "Brightness");
    assert_eq!(graph.group_path_names(), vec!["Voice".to_string()]);
    graph.close_group();
    assert_eq!(graph.nodes[2].inputs[1].name, "Brightness");
    assert_eq!(graph.nodes[2].label(), "Voice");
}

#[test]
fn groups_inside_groups_nest_and_compile() {
    let original = chain();
    let mut graph = chain();
    graph.create_group(&[FILTER]);
    graph.create_group(&[FILTER, REVERB]);
    assert_eq!(graph.nodes[FILTER].kind, NodeKind::Group);
    assert_eq!(graph.patch(), original.patch());
    assert!(graph.open_group(FILTER));
    assert_eq!(graph.nodes[2].kind, NodeKind::Group);
    assert!(graph.open_group(2));
    assert_eq!(graph.group_depth(), 2);
    assert_eq!(graph.nodes[2].kind, NodeKind::Filter);
    assert_eq!(graph.patch(), original.patch());
    assert!(graph.close_group());
    assert!(graph.close_group());
    assert_eq!(graph.patch(), original.patch());
}

#[test]
fn an_unwired_pin_leaves_the_node_inside_unwired() {
    let mut graph = grouped();
    graph.open_group(2);
    graph.connections.retain(|c| c.from != 0);
    graph.close_group();
    // The same graph by hand, with the oscillator and the LFO no longer wired to the filter.
    let mut by_hand = chain();
    by_hand.connections.retain(|c| c.to != FILTER);
    assert_eq!(graph.patch(), by_hand.patch());
    assert_ne!(graph.patch(), chain().patch());
}

#[test]
fn queries_about_nodes_in_a_group_see_through_the_pins() {
    let mut graph = grouped();
    // The LFO is outside and drives the filter's cutoff through pin 1.
    assert!(graph.control_route(2, KnobId::Cutoff).is_none());
    assert!(!graph.is_driven(2, KnobId::Cutoff));
    assert_eq!(graph.lfo_index(4), Some(0));
    graph.open_group(2);
    assert!(graph.is_driven(2, KnobId::Cutoff));
    assert!(graph.control_route(2, KnobId::Cutoff).is_some());
    assert!(!graph.is_driven(2, KnobId::Resonance));
    // The edge nodes have no place in the compiled graph.
    assert!(!graph.is_driven(0, KnobId::Cutoff));
    assert_eq!(graph.lfo_index(0), None);
    assert_eq!(graph.envelope_index(1), None);
    graph.close_group();
    // The group node has none either.
    assert!(graph.control_route(2, KnobId::Cutoff).is_none());
}

#[test]
fn flat_indices_follow_the_flat_order() {
    // An LFO inside the group comes before the one outside in the flat order.
    let mut graph = Graph::with(
        vec![
            node(NodeKind::Oscillator, 0.0),
            node(NodeKind::Lfo, 100.0),
            node(NodeKind::Lfo, 200.0),
        ],
        Vec::new(),
    );
    graph.create_group(&[2]);
    assert_eq!(graph.lfo_index(1), Some(0));
    graph.open_group(2);
    assert_eq!(graph.lfo_index(2), Some(1));
}

#[test]
fn a_cached_read_sees_a_knob_edit() {
    let mut graph = grouped();
    graph.open_group(2);
    let cutoff = |graph: &Graph| {
        graph
            .with_flat(2, |wiring, node| {
                wiring.knob_value(node, KnobId::Cutoff).value
            })
            .unwrap()
    };
    let generation = |graph: &Graph| {
        graph
            .flat_cache
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, |cache| cache.generation)
    };
    assert_eq!(cutoff(&graph), 0.3);
    let built = generation(&graph);
    assert!(built > 0);
    // Reads that change nothing reuse the cache.
    for _ in 0..10 {
        assert_eq!(cutoff(&graph), 0.3);
        assert!(graph.is_driven(2, KnobId::Cutoff));
    }
    assert_eq!(generation(&graph), built);
    // An edit made straight on the node, as the canvas does, shows on the next read.
    graph.nodes[2].set_value(KnobId::Cutoff, 0.8);
    assert_eq!(cutoff(&graph), 0.8);
    assert_eq!(generation(&graph), built + 1);
    // So does an edit to a node outside the open group, and a change of wires.
    graph.open_groups[0].nodes[1].bypassed = true;
    assert_eq!(cutoff(&graph), 0.8);
    assert_eq!(generation(&graph), built + 2);
    graph.connections.clear();
    assert!(!graph.is_driven(2, KnobId::Cutoff));
    assert_eq!(generation(&graph), built + 3);
}

#[test]
fn a_graph_with_no_groups_never_builds_the_cache() {
    let graph = chain();
    assert!(graph.is_driven(FILTER, KnobId::Cutoff));
    assert!(graph.control_route(FILTER, KnobId::Cutoff).is_some());
    assert_eq!(graph.lfo_index(LFO), Some(0));
    assert!(graph.flat_cache.lock().unwrap().is_none());
    assert_eq!(graph.patch(), graph.wiring().patch());
}

#[test]
fn a_monitor_inside_a_group_finds_its_instance() {
    let mut graph = Graph::with(
        vec![
            node(NodeKind::Oscillator, 0.0),
            node(NodeKind::Monitor, 200.0),
            node(NodeKind::Output, 400.0),
            node(NodeKind::Input, 0.0),
        ],
        vec![
            wire(0, Port::Audio, 1, Port::Audio),
            wire(1, Port::Audio, 2, Port::Slot(0)),
            wire(3, Port::Notes, 0, Port::Notes),
        ],
    );
    graph.refresh_slots();
    let plain = graph.monitor_pools();
    assert_eq!(plain.len(), 1);
    graph.create_group(&[1]);
    assert!(graph.monitor_pools().is_empty());
    graph.open_group(1);
    assert_eq!(graph.monitor_pools().len(), 1);
    assert_eq!(graph.monitor_pools()[0].0, 2);
}

#[test]
fn a_save_inside_a_group_is_the_root_graph() {
    let mut graph = grouped();
    let closed = serde_json::to_value(graph.to_saved()).unwrap();
    graph.open_group(2);
    graph.nodes[2].set_value(KnobId::Cutoff, 0.7);
    let open = serde_json::to_value(graph.to_saved()).unwrap();
    assert_ne!(open, closed);
    let loaded = Graph::from_saved(&graph.to_saved());
    assert_eq!(loaded.group_depth(), 0);
    assert_eq!(loaded.nodes.len(), 5);
    let inside = &loaded.nodes[2].group.as_ref().unwrap().nodes[2];
    assert_eq!(
        inside.value(KnobId::Cutoff),
        graph.nodes[2].value(KnobId::Cutoff)
    );
    // Closing then saving gives the same thing.
    graph.close_group();
    assert_eq!(serde_json::to_value(graph.to_saved()).unwrap(), open);
}

#[test]
fn a_group_survives_saving_and_loading() {
    let mut graph = grouped();
    graph.rename_group_node(2, "Tone");
    let saved = serde_json::to_string(&graph.to_saved()).unwrap();
    let loaded: super::saved::SavedGraph = serde_json::from_str(&saved).unwrap();
    let loaded = Graph::from_saved(&loaded);
    assert_eq!(loaded.nodes[2].label(), "Tone");
    assert_eq!(loaded.nodes[2].inputs, graph.nodes[2].inputs);
    assert_eq!(loaded.nodes[2].outputs, graph.nodes[2].outputs);
    assert_eq!(sorted_links(&loaded), sorted_links(&graph));
    assert_eq!(loaded.nodes[2].group.as_ref().unwrap().links.len(), 4);
    assert_eq!(loaded.patch(), graph.patch());
    assert_eq!(loaded.patch(), chain().patch());
}

#[test]
fn a_group_in_a_group_survives_saving_and_loading() {
    let mut graph = chain();
    graph.create_group(&[FILTER]);
    graph.create_group(&[FILTER, REVERB]);
    graph.rename_group_node(FILTER, "Outer");
    graph.open_group(FILTER);
    graph.rename_group_node(2, "Inner");
    graph.nodes[3].set_value(KnobId::Mix, 0.25);
    graph.close_group();
    let json = serde_json::to_string(&graph.to_saved()).unwrap();
    let loaded = Graph::from_saved(&serde_json::from_str(&json).unwrap());
    assert_eq!(loaded.nodes[FILTER].label(), "Outer");
    let outer = loaded.nodes[FILTER].group.as_ref().unwrap();
    assert_eq!(outer.nodes[2].label(), "Inner");
    let inner = outer.nodes[2].group.as_ref().unwrap();
    assert_eq!(inner.nodes[2].kind, NodeKind::Filter);
    assert_eq!(
        outer.nodes[3].value(KnobId::Mix),
        graph.nodes[FILTER].group.as_ref().unwrap().nodes[3].value(KnobId::Mix)
    );
    assert_eq!(loaded.patch(), graph.patch());
    // A load and a save change nothing more, once the dropdown choices a load fills in are there.
    let again = Graph::from_saved(&loaded.to_saved());
    assert_eq!(
        serde_json::to_string(&again.to_saved()).unwrap(),
        serde_json::to_string(&loaded.to_saved()).unwrap()
    );
}

#[test]
fn a_save_without_groups_writes_no_group_fields_and_loads_as_before() {
    let graph = chain();
    let saved = serde_json::to_value(graph.to_saved()).unwrap();
    assert_eq!(saved["version"], 3);
    for node in saved["nodes"].as_array().unwrap() {
        let node = node.as_object().unwrap();
        assert!(!node.contains_key("group"));
        assert!(!node.contains_key("inputs"));
        assert!(!node.contains_key("outputs"));
    }
    let loaded = Graph::from_saved(&serde_json::from_value(saved).unwrap());
    assert_eq!(loaded.patch(), graph.patch());
    assert_eq!(sorted_links(&loaded), sorted_links(&graph));
}

#[test]
fn the_old_octave_migration_still_runs_on_old_saves() {
    let mut graph = chain();
    graph.nodes[OSC].set_value(KnobId::Octave, 0.75);
    let mut saved = graph.to_saved();
    saved.version = 1;
    let loaded = Graph::from_saved(&saved);
    assert_eq!(loaded.nodes[OSC].value(KnobId::Octave), 0.625);
    saved.version = 3;
    let loaded = Graph::from_saved(&saved);
    assert_eq!(loaded.nodes[OSC].value(KnobId::Octave), 0.75);
}

#[test]
fn a_pin_that_no_longer_exists_loses_its_wire_on_load() {
    let mut graph = grouped();
    let mut saved = graph.to_saved();
    // Cut the group node's pins from the save, and leave its wires.
    saved.nodes[2].inputs.clear();
    let data = saved.nodes[2].group.as_mut().unwrap();
    data.nodes[0].outputs.clear();
    graph.close_group();
    let loaded = Graph::from_saved(&saved);
    assert!(loaded.nodes[2].inputs.is_empty());
    assert!(all_valid(&loaded));
    assert!(loaded.connections.iter().all(|c| c.to != 2));
}

#[test]
fn making_a_group_selects_it_alone() {
    let mut graph = chain();
    graph.set_selection(vec![INPUT, OUTPUT]);
    graph.create_group(&[FILTER]);
    // One member left and one group node came in, so the output keeps its number.
    assert_eq!(graph.selection(), vec![FILTER]);
    assert_eq!(graph.nodes[OUTPUT].kind, NodeKind::Output);
}

/// The graph of the chain with the filter and reverb gone, and the oscillator wired to the
/// output.
fn direct() -> Graph {
    let mut graph = Graph::with(
        vec![
            node(NodeKind::Input, 0.0),
            node(NodeKind::Oscillator, 200.0),
            node(NodeKind::Output, DEFAULT_OUTPUT_X),
            node(NodeKind::Lfo, 300.0),
        ],
        vec![
            wire(0, Port::Notes, 1, Port::Notes),
            wire(1, Port::Audio, 2, Port::Slot(0)),
        ],
    );
    graph.refresh_slots();
    graph
}

#[test]
fn a_switched_off_group_plays_as_if_it_were_a_wire() {
    let mut graph = grouped();
    let on = graph.patch();
    assert!(graph.toggle_bypass(2));
    let off = graph.patch();
    assert_ne!(off, on);
    assert_eq!(off, direct().patch());
    // Switching it back on restores the sound.
    assert!(graph.toggle_bypass(2));
    assert_eq!(graph.patch(), on);
}

#[test]
fn the_nodes_of_a_switched_off_group_take_no_lfo_or_effect() {
    let mut graph = chain();
    assert!(graph.create_group(&[FILTER, REVERB, LFO]).is_some());
    assert!(graph.patch().lfos.iter().any(Option::is_some));
    let group = graph
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::Group)
        .unwrap();
    assert!(graph.toggle_bypass(group));
    let patch = graph.patch();
    assert!(patch.lfos.iter().all(Option::is_none));
    assert!(!format!("{patch:?}").contains("Reverb"));
}

#[test]
fn a_switched_off_group_works_while_it_is_open() {
    let mut graph = grouped();
    assert!(graph.toggle_bypass(2));
    let off = graph.patch();
    assert!(graph.open_group(2));
    assert_eq!(graph.patch(), off);
    assert!(graph.close_group());
    assert_eq!(graph.patch(), off);
    assert!(graph.nodes[2].bypassed);
}

#[test]
fn a_switched_off_group_survives_saving_and_copying() {
    let mut graph = grouped();
    graph.toggle_bypass(2);
    let loaded = Graph::from_saved(&graph.to_saved());
    assert!(loaded.nodes[2].bypassed);
    assert_eq!(loaded.patch(), graph.patch());
    let copy = graph.nodes[2].duplicate(Point::new(0.0, 0.0));
    assert!(copy.bypassed);
}

#[test]
fn a_new_input_pin_is_named_after_the_first_knob_it_feeds() {
    let mut graph = grouped();
    assert!(graph.open_group(2));
    let pin = graph.add_pin(PinSide::Input, "", Signal::Control).unwrap();
    assert!(graph.nodes[0].outputs[pin].name.starts_with("In "));
    graph.connect(0, control_pin(pin), 2, Port::Knob(KnobId::Resonance));
    assert_eq!(graph.nodes[0].outputs[pin].name, "Resonance");
    // The group's face carries the name too.
    assert!(graph.close_group());
    assert_eq!(graph.nodes[2].inputs[pin].name, "Resonance");
}

#[test]
fn a_second_wire_does_not_rename_a_pin() {
    let mut graph = grouped();
    assert!(graph.open_group(2));
    let pin = graph.add_pin(PinSide::Input, "", Signal::Control).unwrap();
    graph.connect(0, control_pin(pin), 2, Port::Knob(KnobId::Resonance));
    graph.connect(0, control_pin(pin), 3, Port::Knob(KnobId::Mix));
    assert_eq!(graph.nodes[0].outputs[pin].name, "Resonance");
}

#[test]
fn a_pin_named_by_hand_keeps_its_name() {
    let mut graph = grouped();
    assert!(graph.open_group(2));
    let pin = graph
        .add_pin(PinSide::Input, "Tone", Signal::Control)
        .unwrap();
    graph.connect(0, control_pin(pin), 2, Port::Knob(KnobId::Resonance));
    assert_eq!(graph.nodes[0].outputs[pin].name, "Tone");
    // A default name that is renamed to another default looking name is still a default.
    let other = graph.add_pin(PinSide::Input, "", Signal::Control).unwrap();
    assert!(graph.rename_pin(PinSide::Input, other, "Mine"));
    graph.connect(0, control_pin(other), 3, Port::Knob(KnobId::Mix));
    assert_eq!(graph.nodes[0].outputs[other].name, "Mine");
}

#[test]
fn an_output_pin_is_named_after_the_first_output_that_feeds_it() {
    let mut graph = grouped();
    assert!(graph.open_group(2));
    let pin = graph.add_pin(PinSide::Output, "", Signal::Control).unwrap();
    // A shape pin is the control output of an LFO.
    let lfo = graph.add_node(NodeKind::Lfo, Point::new(300.0, 300.0));
    graph.connect(lfo, Port::Shape, 1, control_pin(pin));
    assert_eq!(graph.nodes[1].inputs[pin].name, "Shape");
}

#[test]
fn a_name_that_is_taken_gets_a_number() {
    let mut graph = grouped();
    assert!(graph.open_group(2));
    let first = graph.add_pin(PinSide::Input, "", Signal::Control).unwrap();
    graph.connect(0, control_pin(first), 2, Port::Knob(KnobId::Resonance));
    let second = graph.add_pin(PinSide::Input, "", Signal::Control).unwrap();
    graph.connect(0, control_pin(second), 2, Port::Knob(KnobId::Resonance));
    assert_eq!(graph.nodes[0].outputs[first].name, "Resonance");
    assert_eq!(graph.nodes[0].outputs[second].name, "Resonance 2");
}
