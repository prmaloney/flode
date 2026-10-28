//! Tests for the editor's graph model.

use crate::dsp::wavetable::Waveform;
use crate::nodes::connection::Connection;
use crate::nodes::knobs::KnobId;
use crate::nodes::oscillator;
use crate::nodes::{Category, FlodeNode, NodeKind, Port};
use crate::patch::control::{ControlLevels, MapStage, Source, MAX_ENVELOPES, MAX_LFOS};
use crate::patch::stage::{LaneGraph, StageParams};
use nih_plug_iced::{Point, Rectangle};

use super::{curve::*, draw::*, graph::*, menus::*, saved::*};

/// The smallest working graph: an oscillator at node 0, wired to the output at node 1, with a MIDI
/// input at node 2 feeding it notes. Most tests want exactly this, so they start from it and add
/// what they need. The default graph a new instance loads has an envelope as well, and its own
/// tests are in `default_graph_tests`.
/// A saw node, for an oscillator's table pin, since an oscillator with no table is silent.
fn saw_frame(at: Point) -> FlodeNode {
    FlodeNode::new(NodeKind::SawWave, at)
}

fn minimal_graph() -> Graph {
    let mut graph = Graph::with(
        vec![
            FlodeNode::new(NodeKind::Oscillator, Point::new(DEFAULT_OSC_X, 80.0)),
            FlodeNode::new(NodeKind::Output, Point::new(DEFAULT_OUTPUT_X, 80.0)),
            FlodeNode::new(NodeKind::Input, Point::new(0.0, 80.0)),
            saw_frame(Point::new(0.0, 400.0)),
        ],
        vec![
            Connection {
                from: 3,
                out: Port::Table,
                to: 0,
                input: Port::TableIn,
            },
            Connection {
                from: 0,
                out: Port::Audio,
                to: 1,
                input: Port::Slot(0),
            },
            Connection {
                from: 2,
                out: Port::Notes,
                to: 0,
                input: Port::Notes,
            },
        ],
    );
    graph.refresh_slots();
    graph
}

/// The MIDI input node of a graph, which the helper below adds if there is none.
fn midi_input(graph: &mut Graph) -> usize {
    match graph.nodes.iter().position(|n| n.kind == NodeKind::Input) {
        Some(index) => index,
        None => graph.add_node(NodeKind::Input, Point::new(0.0, 0.0)),
    }
}

/// Adds an oscillator that plays: its notes pin is wired to the graph's MIDI input.
fn add_oscillator(graph: &mut Graph, position: Point) -> usize {
    let input = midi_input(graph);
    let oscillator = graph.add_node(NodeKind::Oscillator, position);
    graph.connect(input, Port::Notes, oscillator, Port::Notes);
    oscillator
}

/// The oscillator settings of the stage at `index` in a slot.
fn osc_params(lane: &LaneGraph, index: usize) -> crate::patch::stage::OscillatorParams {
    match &lane.stages[index].params {
        StageParams::Oscillator(params) => params.clone(),
        _ => panic!("stage {index} is not an oscillator"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A header moves toward its signal's wire color and stays dark. Audio keeps the neutral.
    #[test]
    fn a_header_leans_toward_its_signal() {
        use crate::editor::style::{CONTROL_COLOR, NODE_HEADER_COLOR, NOTES_COLOR};
        use crate::nodes::Signal;
        let neutral = NODE_HEADER_COLOR;
        assert_eq!(header_tint(neutral, None), neutral);
        assert_eq!(header_tint(neutral, Some(Signal::Audio)), neutral);
        let control = header_tint(neutral, Some(Signal::Control));
        assert!(control.g > neutral.g && control.g < CONTROL_COLOR.g);
        let notes = header_tint(neutral, Some(Signal::Notes));
        assert!(notes.r > neutral.r && notes.r < NOTES_COLOR.r);
        // Dark enough for white titles: no channel past half.
        for color in [control, notes] {
            assert!(color.r < 0.5 && color.g < 0.5 && color.b < 0.6, "{color:?}");
        }
    }

    fn knob_at(id: KnobId, plain: f32) -> f32 {
        id.to_normalized(plain)
    }

    /// Envelope into a map that ranges 0.2 to 0.6, into a second map that ranges 0.5 to 1.0,
    /// into the gain. The shape 0 reaches 0.5 + 0.5 * 0.2 and the shape 1 reaches 0.5 + 0.5 * 0.6.
    #[test]
    fn chained_maps_compose_into_one_range() {
        let mut graph = minimal_graph();
        let env = graph.add_node(NodeKind::Envelope, Point::new(0.0, 0.0));
        let first = graph.add_node(NodeKind::Map, Point::new(0.0, 0.0));
        let second = graph.add_node(NodeKind::Map, Point::new(0.0, 0.0));
        graph.nodes[first].set_value(KnobId::MapMin, knob_at(KnobId::MapMin, 0.2));
        graph.nodes[first].set_value(KnobId::MapMax, knob_at(KnobId::MapMax, 0.6));
        graph.nodes[second].set_value(KnobId::MapMin, knob_at(KnobId::MapMin, 0.5));
        graph.nodes[second].set_value(KnobId::MapMax, knob_at(KnobId::MapMax, 1.0));
        graph.connect(env, Port::Shape, first, Port::Shape);
        graph.connect(first, Port::Shape, second, Port::Shape);
        graph.connect(second, Port::Shape, 1, Port::Knob(KnobId::Gain));

        // The source's level passes through the first map, then the second.
        let route = graph.control_route(1, KnobId::Gain).unwrap();
        let wiring = graph.wiring();
        let at = |envelope: f32| {
            route.apply(&wiring.with_signals(ControlLevels {
                envelope: [envelope; MAX_ENVELOPES],
                ..ControlLevels::default()
            }))
        };
        assert!((at(0.0) - 0.6).abs() < 1e-5);
        assert!((at(1.0) - 0.8).abs() < 1e-5);
        assert!(graph.is_driven(1, KnobId::Gain));
    }

    /// A map with nothing upstream does not drive its knob.
    #[test]
    fn map_without_envelope_does_not_drive() {
        let mut graph = minimal_graph();
        let map = graph.add_node(NodeKind::Map, Point::new(0.0, 0.0));
        graph.connect(map, Port::Shape, 1, Port::Knob(KnobId::Gain));
        assert_eq!(graph.control_route(1, KnobId::Gain), None);
        assert!(!graph.is_driven(1, KnobId::Gain));
    }

    /// Two filters keep separate knobs. A wire into one's cutoff drives only that filter.
    #[test]
    fn a_control_wire_drives_only_its_own_node() {
        let mut graph = minimal_graph();
        let env = graph.add_node(NodeKind::Envelope, Point::new(0.0, 0.0));
        let first = graph.add_node(NodeKind::Filter, Point::new(0.0, 0.0));
        let second = graph.add_node(NodeKind::Filter, Point::new(0.0, 0.0));
        graph.connect(env, Port::Shape, first, Port::Knob(KnobId::Cutoff));
        assert!(graph.is_driven(first, KnobId::Cutoff));
        assert!(!graph.is_driven(second, KnobId::Cutoff));
    }
}

#[cfg(test)]
mod slot_tests {
    use super::*;

    /// The default graph has one slot in use, and one empty pin below it.
    #[test]
    fn default_output_has_one_slot_and_an_empty_pin() {
        let graph = minimal_graph();
        let output = &graph.nodes[1];
        assert_eq!(output.used, 1);
        assert_eq!(output.slots, 2);
        assert!(graph.patch().lanes[0].is_active());
        assert!(!graph.patch().lanes[1].is_active());
    }

    /// Wiring the empty pin adds a slot and a new empty pin below it.
    #[test]
    fn wiring_the_empty_pin_adds_a_slot() {
        let mut graph = minimal_graph();
        let second = add_oscillator(&mut graph, Point::new(0.0, 200.0));
        graph.connect(second, Port::Audio, 1, Port::Slot(1));
        let output = &graph.nodes[1];
        assert_eq!(output.used, 2);
        assert_eq!(output.slots, 3);
        assert!(graph.patch().lanes[1].is_active());
        assert!(graph.nodes[1]
            .rows()
            .iter()
            .any(|row| row.knob == Some(KnobId::Slot2)));
    }

    /// Saving and loading a graph with several output slots keeps every wire. The slot pins show
    /// only for connected slots, so a load has to count the wires before it checks them.
    #[test]
    fn wires_into_later_output_slots_survive_saving() {
        let mut graph = minimal_graph();
        let second = add_oscillator(&mut graph, Point::new(0.0, 200.0));
        let third = add_oscillator(&mut graph, Point::new(0.0, 400.0));
        graph.connect(second, Port::Audio, 1, Port::Slot(1));
        graph.connect(third, Port::Audio, 1, Port::Slot(2));
        // Three audio wires, and a notes wire for each oscillator.
        assert_eq!(graph.connections.len(), 6);

        let loaded = Graph::from_saved(&graph.to_saved());
        assert_eq!(loaded.connections, graph.connections);
        assert_eq!(loaded.nodes[1].used, 3);
        assert!(loaded.patch().lanes[2].is_active());
    }

    /// Two oscillators wired to two slots keep their own knobs. Turning one does not move the other.
    #[test]
    fn two_oscillators_keep_separate_knobs() {
        let mut graph = minimal_graph();
        let second = add_oscillator(&mut graph, Point::new(0.0, 200.0));
        graph.connect(second, Port::Audio, 1, Port::Slot(1));

        graph.nodes[0].set_value(KnobId::Detune, KnobId::Detune.to_normalized(60.0));
        let lanes = graph.patch().lanes;
        let levels = ControlLevels::default();
        let first_detune = osc_params(&lanes[0], 0).detune.plain(&levels);
        let second_detune = osc_params(&lanes[1], 0).detune.plain(&levels);
        assert!((first_detune - 60.0).abs() < 0.01);
        assert!((second_detune - KnobId::Detune.default_plain()).abs() < 0.01);
    }

    /// Saves from before slots wired the output's single audio pin, which loads into slot 0.
    #[test]
    fn old_output_links_load_into_slot_zero() {
        let node = |kind| SavedNode {
            kind,
            x: 0.0,
            y: 0.0,
            collapsed: false,
            bypassed: false,
            sections_collapsed: Vec::new(),
            choices: Vec::new(),
            choice: 0,
            switches: Vec::new(),
            switched: false,
            knobs: Vec::new(),
            waveform: None,
            filter_type: None,
            out_min: None,
            out_max: None,
            points: Vec::new(),
            group: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            names: Vec::new(),
            sample_path: None,
        };
        let saved = SavedGraph {
            version: 0,
            preset_name: None,
            preset_category: String::new(),
            nodes: vec![node(NodeKind::Oscillator), node(NodeKind::Output)],
            connections: vec![(0, 1)],
            links: Vec::new(),
        };
        let graph = Graph::from_saved(&saved);
        // The audio wire goes to the first slot, and the upgrade adds a notes wire.
        assert_eq!(graph.connections.len(), 2);
        assert_eq!(graph.connections[0].input, Port::Slot(0));
        assert!(graph.patch().lanes[0].is_active());
    }

    /// Saved knob values and the dropdown come back, and old fields still load.
    #[test]
    fn saving_and_loading_keeps_node_state() {
        let mut graph = minimal_graph();
        graph.nodes[0].set_value(KnobId::Unison, KnobId::Unison.to_normalized(5.0));
        graph.nodes[0].choose(1, 2);
        graph.nodes[1].switches = vec![true, true];
        let loaded = Graph::from_saved(&graph.to_saved());
        assert_eq!(loaded.nodes[0].plain(KnobId::Unison), 5.0);
        assert_eq!(
            oscillator::warp_mode(&loaded.nodes[0]),
            crate::dsp::warp::WarpMode::Sync
        );
        assert_eq!(loaded.nodes[1].switches, vec![true, true]);

        let legacy = SavedNode {
            kind: NodeKind::Oscillator,
            x: 0.0,
            y: 0.0,
            collapsed: false,
            bypassed: false,
            sections_collapsed: Vec::new(),
            choices: Vec::new(),
            choice: 0,
            switches: Vec::new(),
            switched: false,
            knobs: Vec::new(),
            waveform: Some(Waveform::Triangle),
            filter_type: None,
            out_min: None,
            out_max: None,
            points: Vec::new(),
            group: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            names: Vec::new(),
            sample_path: None,
        };
        // An old save's waveform still loads into the first dropdown, which an oscillator no
        // longer shows. It plays no part in the sound.
        assert_eq!(legacy.into_node().choice(0), Waveform::Triangle.index());
    }

    /// A node dropped on the wire into an output slot splices into that wire.
    #[test]
    fn a_node_dropped_on_a_slot_wire_is_spliced_in() {
        let mut graph = minimal_graph();
        let curve = graph.link_curve(&graph.connections[0]).unwrap();
        let middle = bezier_point(&curve, 0.5);
        let filter = graph.add_node(
            NodeKind::Filter,
            Point::new(middle.x - 80.0, middle.y - 20.0),
        );

        assert_eq!(graph.link_under(filter), Some(0));
        graph.insert_into(filter, 0);
        graph.refresh_slots();

        assert_eq!(graph.connections.len(), 3);
        assert_eq!(graph.connections[0].to, filter);
        assert_eq!(graph.connections[2].from, filter);
        assert_eq!(graph.connections[2].input, Port::Slot(0));
        assert!(graph.patch().lanes[0]
            .stages
            .iter()
            .any(|stage| matches!(stage.params, StageParams::Filter(_))));
    }
}

#[cfg(test)]
mod lfo_and_category_tests {
    use super::*;

    /// An LFO wired into the output's gain drives that knob from LFO 0.
    #[test]
    fn an_lfo_drives_a_knob_as_its_own_source() {
        let mut graph = minimal_graph();
        let lfo = graph.add_node(NodeKind::Lfo, Point::new(0.0, 0.0));
        graph.connect(lfo, Port::Shape, 1, Port::Knob(KnobId::Gain));

        let route = graph.control_route(1, KnobId::Gain).unwrap();
        assert_eq!(route.source, Source::Lfo(0));
        let at = |lfo: f32| {
            route.apply(&ControlLevels {
                lfo: [lfo; MAX_LFOS],
                ..ControlLevels::default()
            })
        };
        assert!((at(0.25) - 0.25).abs() < 1e-6);

        let settings = graph.patch().lfos;
        let rate = settings[0].expect("the first lfo runs").rate;
        assert!(settings[1].is_none());
        let default = KnobId::LfoRate.default_plain();
        assert!((rate.plain(&ControlLevels::default()) - default).abs() < 1e-3);
    }

    /// The first eight LFOs are the ones the audio thread runs. Later LFOs make no signal.
    #[test]
    fn only_the_first_lfos_are_run() {
        let mut graph = minimal_graph();
        let mut lfos = Vec::new();
        for x in 0..9 {
            let lfo = graph.add_node(NodeKind::Lfo, Point::new(x as f32 * 10.0, 200.0));
            graph.connect(lfo, Port::Shape, 1, Port::Knob(KnobId::Gain));
            lfos.push(lfo);
        }
        // The ninth LFO is past the eight the audio thread runs.
        assert_eq!(graph.wiring().lfo_index(lfos[8]), None);
        assert_eq!(graph.wiring().lfo_index(lfos[7]), Some(7));
        assert!(graph.patch().lfos.iter().all(Option::is_some));
    }

    /// An LFO or envelope past the limit is tagged in its header.
    #[test]
    fn a_node_past_the_limit_is_tagged() {
        let mut graph = minimal_graph();
        let lfos: Vec<usize> = (0..9)
            .map(|x| graph.add_node(NodeKind::Lfo, Point::new(x as f32 * 10.0, 200.0)))
            .collect();
        assert_eq!(over_limit_label(&graph, lfos[7]), None);
        assert_eq!(over_limit_label(&graph, lfos[8]), Some("Over limit"));
        assert_eq!(over_limit_label(&graph, 0), None);
    }

    /// An envelope can drive an LFO's rate, and the LFO reports that route.
    #[test]
    fn an_envelope_can_drive_an_lfo_rate() {
        let mut graph = minimal_graph();
        let env = graph.add_node(NodeKind::Envelope, Point::new(0.0, 0.0));
        let lfo = graph.add_node(NodeKind::Lfo, Point::new(0.0, 0.0));
        graph.connect(env, Port::Shape, lfo, Port::Knob(KnobId::LfoRate));
        let route = graph.control_route(lfo, KnobId::LfoRate).unwrap();
        assert_eq!(route.source, Source::Envelope(0));
        assert!(graph.is_driven(lfo, KnobId::LfoRate));
    }
}

#[cfg(test)]
mod input_node_tests {
    use super::*;
    use crate::nodes::input::MidiSource;

    /// The mod wheel pin of an input node drives a knob as its own source.
    #[test]
    fn a_midi_pin_drives_a_knob() {
        let mut graph = minimal_graph();
        let input = graph.add_node(NodeKind::Input, Point::new(0.0, 0.0));
        graph.connect(
            input,
            Port::Midi(MidiSource::ModWheel),
            1,
            Port::Knob(KnobId::Gain),
        );
        let route = graph.control_route(1, KnobId::Gain).unwrap();
        assert_eq!(route.source, Source::Midi(MidiSource::ModWheel));
        assert!(graph.is_driven(1, KnobId::Gain));
    }

    /// A MIDI source goes through a map like any other control signal.
    #[test]
    fn a_midi_pin_can_feed_a_map() {
        let mut graph = minimal_graph();
        let input = graph.add_node(NodeKind::Input, Point::new(0.0, 0.0));
        let map = graph.add_node(NodeKind::Map, Point::new(0.0, 0.0));
        graph.connect(input, Port::Midi(MidiSource::Velocity), map, Port::Shape);
        graph.connect(map, Port::Shape, 1, Port::Knob(KnobId::Gain));
        // The knob follows the map, and the map reads the MIDI source.
        let route = graph.control_route(1, KnobId::Gain).unwrap();
        assert_eq!(route.source, Source::Signal(0));
        let table = graph.patch().signals;
        assert_eq!(
            table.nodes().next().unwrap().input(),
            Some(Source::Midi(MidiSource::Velocity))
        );
    }

    /// Every source survives a trip through the saved byte, so routes keep their meaning.
    /// A map dropped on a wire that drives a knob splices into it. The knob keeps its route, now
    /// through the map.
    #[test]
    fn a_map_dropped_on_a_control_wire_is_spliced_in() {
        let mut graph = minimal_graph();
        let env = graph.add_node(NodeKind::Envelope, Point::new(0.0, 0.0));
        graph.connect(env, Port::Shape, 1, Port::Knob(KnobId::Gain));
        let link = graph.connections.len() - 1;
        let curve = graph.link_curve(&graph.connections[link]).unwrap();
        let middle = bezier_point(&curve, 0.5);
        let map = graph.add_node(NodeKind::Map, Point::new(middle.x - 80.0, middle.y - 20.0));

        assert_eq!(graph.link_under(map), Some(link));
        graph.insert_into(map, link);
        assert_eq!(graph.connections[link].to, map);
        assert_eq!(graph.connections[link].input, Port::Shape);
        let route = graph.control_route(1, KnobId::Gain).unwrap();
        assert_eq!(route.source, Source::Signal(0));
        assert_eq!(
            graph.patch().signals.nodes().next().unwrap().input(),
            Some(Source::Envelope(0))
        );
    }
}

#[cfg(test)]
mod map_curve_tests {
    use super::*;

    /// Inputs outside the input range hold at the ends, so the output is flat there.
    #[test]
    fn a_map_clamps_outside_its_input_range() {
        let stage = MapStage::fixed(0.25, 0.75, 0.0, 1.0);
        assert_eq!(stage.apply(0.1), 0.0);
        assert!((stage.apply(0.5) - 0.5).abs() < 1e-6);
        assert_eq!(stage.apply(0.9), 1.0);
    }

    /// A handle's position on the curve follows its input and output values.
    #[test]
    fn handles_sit_on_the_curve() {
        let mut node = FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0));
        node.set_value(KnobId::MapInMin, 0.0);
        node.set_value(KnobId::MapMin, 0.0);
        node.set_value(KnobId::MapInMax, 1.0);
        node.set_value(KnobId::MapMax, 1.0);
        let bounds = node.curve_bounds();
        let handles = map_handles(&node, &crate::nodes::viewer::Live::of(&node));
        assert!((handles[0].x - bounds.x).abs() < 1e-3);
        assert!((handles[1].y - bounds.y).abs() < 1e-3);
    }

    fn live_of(node: &FlodeNode) -> crate::nodes::viewer::Live {
        crate::nodes::viewer::Live::of(node)
    }

    /// A handle follows a value that a wire drives, and a press finds it where it is drawn.
    #[test]
    fn a_handle_follows_a_driven_value() {
        let node = FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0));
        let own = map_handles(&node, &live_of(&node))[1];
        let live =
            crate::nodes::viewer::Live::new(&node, |id| (id == KnobId::MapInMax).then_some(0.5));
        let driven = map_handles(&node, &live)[1];
        assert!(driven.x < own.x);
        assert_eq!(map_handle_at(&node, &live, driven), Some(1));
        assert_eq!(map_handle_at(&node, &live_of(&node), driven), None);
    }

    /// A drag moves the values a wire does not drive and leaves the others.
    #[test]
    fn a_drag_leaves_driven_map_values_alone() {
        let mut node = FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0));
        let bounds = node.curve_bounds();
        let target = Point::new(
            bounds.x + bounds.width * 0.25,
            bounds.y + bounds.height * 0.25,
        );
        drag_map_handle(
            &mut node,
            1,
            target,
            0.0,
            MapDriven {
                input: true,
                output: false,
            },
        );
        assert_eq!(node.value(KnobId::MapInMax), 1.0);
        assert!((node.value(KnobId::MapMax) - 0.75).abs() < 1e-3);
        drag_map_handle(
            &mut node,
            1,
            target,
            0.0,
            MapDriven {
                input: false,
                output: true,
            },
        );
        assert!((node.value(KnobId::MapInMax) - 0.25).abs() < 1e-3);
        assert!((node.value(KnobId::MapMax) - 0.75).abs() < 1e-3);
    }

    /// An LFO wired to a map's In max moves the cut: the route through the map reads the LFO's
    /// live level for that value.
    #[test]
    fn an_lfo_can_drive_a_maps_range() {
        use crate::nodes::connection::Connection;
        // An LFO drives the map's In max, and an envelope runs through the map into a knob.
        let mut nodes = vec![
            FlodeNode::new(NodeKind::Lfo, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Envelope, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0)),
        ];
        nodes[2].set_value(KnobId::MapInMax, 1.0);
        let links = vec![
            Connection {
                from: 1,
                out: Port::Shape,
                to: 2,
                input: Port::Shape,
            },
            Connection {
                from: 2,
                out: Port::Shape,
                to: 3,
                input: Port::Knob(KnobId::Cutoff),
            },
            Connection {
                from: 0,
                out: Port::Shape,
                to: 2,
                input: Port::Knob(KnobId::MapInMax),
            },
        ];
        let graph = Graph::with(nodes, links);
        let wiring = graph.wiring();
        let route = wiring
            .control_route(3, KnobId::Cutoff)
            .expect("a route to the cutoff");
        assert_eq!(route.source, Source::Signal(0));
        let at = |lfo: f32| {
            route.apply(&wiring.with_signals(ControlLevels {
                envelope: [0.4; MAX_ENVELOPES],
                lfo: [lfo; MAX_LFOS],
                ..ControlLevels::default()
            }))
        };
        // With the LFO at 1.0 the map reads the full range, so 0.4 stays 0.4. With it at 0.5,
        // 0.4 is 80 percent of the way through the range.
        assert!((at(1.0) - 0.4).abs() < 1e-5);
        assert!((at(0.5) - 0.8).abs() < 1e-5);
        // The map's own knob reads as driven, and the others do not.
        assert!(graph.is_driven(2, KnobId::MapInMax));
        assert!(!graph.is_driven(2, KnobId::MapInMin));
    }

    /// A map's value can be driven through other maps, as any knob can. An LFO through a map into
    /// another map's Offset follows the first map's curve.
    #[test]
    fn a_map_value_can_be_driven_through_another_map() {
        use crate::nodes::connection::Connection;
        // LFO, into map B, into the Offset of map A. An envelope runs through map A into a knob.
        let mut nodes = vec![
            FlodeNode::new(NodeKind::Lfo, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Envelope, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0)),
        ];
        // Map B squeezes the LFO's 0 to 1 into 0.5 to 1.0, so the offset runs from 0 to +1.
        nodes[1].set_value(KnobId::MapMin, KnobId::MapMin.to_normalized(0.5));
        let link = |from, out, to, input| Connection {
            from,
            out,
            to,
            input,
        };
        let links = vec![
            link(0, Port::Shape, 1, Port::Shape),
            link(1, Port::Shape, 2, Port::Knob(KnobId::MapOffset)),
            link(3, Port::Shape, 2, Port::Shape),
            link(2, Port::Shape, 4, Port::Knob(KnobId::Cutoff)),
        ];
        let graph = Graph::with(nodes, links);
        // The offset reads as driven, so its knob is greyed and its handle follows.
        assert!(graph.is_driven(2, KnobId::MapOffset));
        let wiring = graph.wiring();
        let route = wiring.control_route(4, KnobId::Cutoff).unwrap();
        let at = |lfo: f32, envelope: f32| {
            route.apply(&wiring.with_signals(ControlLevels {
                lfo: [lfo; MAX_LFOS],
                envelope: [envelope; MAX_ENVELOPES],
                ..ControlLevels::default()
            }))
        };
        // LFO at 0: map B gives 0.5, so the offset is 0.5 * 2 - 1 = 0 and the envelope is as is.
        assert!((at(0.0, 0.3) - 0.3).abs() < 1e-5);
        // LFO at 1: map B gives 1.0, so the offset is +1 and the output stops at the top.
        assert!((at(1.0, 0.3) - 1.0).abs() < 1e-5);
        // LFO at 0.5: map B gives 0.75, so the offset is +0.5.
        assert!((at(0.5, 0.3) - 0.8).abs() < 1e-5);
    }

    /// A map nothing feeds makes no signal, so a knob wired to it is not driven. A loop of maps
    /// makes none either, and neither hangs the compiler.
    #[test]
    fn a_map_with_no_input_or_in_a_loop_drives_nothing() {
        use crate::nodes::connection::Connection;
        let link = |from, to, input| Connection {
            from,
            out: Port::Shape,
            to,
            input,
        };
        // Map 0 has no input. Maps 1 and 2 feed each other.
        let nodes = vec![
            FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0)),
        ];
        let links = vec![
            link(0, 3, Port::Knob(KnobId::Cutoff)),
            link(1, 2, Port::Shape),
            link(2, 1, Port::Shape),
            link(1, 3, Port::Knob(KnobId::Resonance)),
        ];
        let graph = Graph::with(nodes, links);
        assert!(!graph.is_driven(3, KnobId::Cutoff));
        assert!(!graph.is_driven(3, KnobId::Resonance));
        assert_eq!(graph.patch().signals.len(), 0);
    }

    /// A map that two knobs read is in the table once, and the table is in running order: a map
    /// comes after the maps that feed it.
    #[test]
    fn the_table_runs_feeders_first_and_each_map_once() {
        use crate::nodes::connection::Connection;
        let link = |from, to, input| Connection {
            from,
            out: Port::Shape,
            to,
            input,
        };
        // The later node feeds the earlier one, so the table must reorder them.
        let nodes = vec![
            FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Lfo, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0)),
        ];
        let links = vec![
            link(2, 1, Port::Shape),
            link(1, 0, Port::Shape),
            link(0, 3, Port::Knob(KnobId::Cutoff)),
            link(0, 3, Port::Knob(KnobId::Resonance)),
        ];
        let graph = Graph::with(nodes, links);
        let table = graph.patch().signals;
        assert_eq!(table.len(), 2);
        let nodes: Vec<_> = table.nodes().collect();
        // Map 1 reads the LFO and runs first. Map 0 reads map 1.
        assert_eq!(nodes[0].input(), Some(Source::Lfo(0)));
        assert_eq!(nodes[1].input(), Some(Source::Signal(0)));
        let a = graph.control_route(3, KnobId::Cutoff).unwrap();
        let b = graph.control_route(3, KnobId::Resonance).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.source, Source::Signal(1));
    }

    /// Each source kind reaches a map value, whatever its index.
    #[test]
    fn every_source_kind_can_drive_a_map_value() {
        use crate::nodes::input::MidiSource;
        use crate::patch::control::{MapNode, MapValue, SignalTable};
        let mut levels = ControlLevels::default();
        levels.envelope[7] = 0.1;
        levels.lfo[1] = 0.2;
        levels.midi[MidiSource::Aftertouch.index()] = 0.3;
        levels.macros[3] = 0.4;
        let cases = [
            (Source::Envelope(7), 0.1),
            (Source::Lfo(1), 0.2),
            (Source::Midi(MidiSource::Aftertouch), 0.3),
            (Source::Macro(3), 0.4),
        ];
        for (source, level) in cases {
            // The source drives the output's high end, and the input is 1.0, so the output is
            // the source's level.
            let mut table = SignalTable::EMPTY;
            table.push(MapNode {
                input: Source::Macro(0),
                in_lo: MapValue::fixed(0.0),
                in_hi: MapValue::fixed(1.0),
                out_lo: MapValue::fixed(0.0),
                out_hi: MapValue {
                    value: 0.9,
                    source: Some(source),
                },
                offset: MapValue::fixed(0.0),
                center_range: None,
            });
            levels.macros[0] = 1.0;
            table.evaluate(&mut levels);
            assert!((levels.signals[0] - level).abs() < 1e-6, "{source:?}");
        }
    }

    /// A route is the source and nothing else, so it stays small.
    #[test]
    fn a_route_stays_small() {
        use crate::patch::control::Route;
        assert!(std::mem::size_of::<Route>() <= 4);
    }

    /// The offset pushes the whole curve up or down, and the output stays between 0 and 1.
    #[test]
    fn the_offset_pushes_the_curve_and_stays_in_range() {
        let up = MapStage::IDENTITY.with_offset(0.25);
        assert!((up.apply(0.0) - 0.25).abs() < 1e-6);
        assert!((up.apply(0.5) - 0.75).abs() < 1e-6);
        assert_eq!(up.apply(1.0), 1.0);
        let down = MapStage::IDENTITY.with_offset(-0.25);
        assert_eq!(down.apply(0.0), 0.0);
        assert!((down.apply(1.0) - 0.75).abs() < 1e-6);
        // No offset leaves a map as it was.
        assert_eq!(MapStage::IDENTITY.apply(0.3), 0.3);
    }

    /// A source that drives the offset stretches 0 to 1 into -1 to 1, so 0.5 is no offset.
    #[test]
    fn a_driven_offset_is_centered_on_half() {
        use crate::patch::control::{MapNode, MapValue, SignalTable};
        let mut table = SignalTable::EMPTY;
        table.push(MapNode {
            input: Source::Macro(0),
            in_lo: MapValue::fixed(0.0),
            in_hi: MapValue::fixed(1.0),
            out_lo: MapValue::fixed(0.0),
            out_hi: MapValue::fixed(1.0),
            offset: MapValue {
                value: 0.0,
                source: Some(Source::Lfo(0)),
            },
            center_range: None,
        });
        let mut levels = ControlLevels::default();
        levels.macros[0] = 0.4;
        for (lfo, expected) in [(0.5, 0.4), (0.75, 0.9), (0.0, 0.0)] {
            levels.lfo[0] = lfo;
            table.evaluate(&mut levels);
            assert!((levels.signals[0] - expected).abs() < 1e-6, "lfo {lfo}");
        }
    }

    /// The handles ride the curve's ends, so an offset moves them, and a drag puts the end of the
    /// curve under the cursor whatever the offset is.
    #[test]
    fn the_handles_follow_the_offset() {
        let mut node = FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0));
        let bounds = node.curve_bounds();
        let flat = map_handles(&node, &live_of(&node));
        node.set_value(KnobId::MapOffset, KnobId::MapOffset.to_normalized(0.25));
        let pushed = map_handles(&node, &live_of(&node));
        // The low end rose a quarter of the box, and the high end is already at the top.
        assert!((flat[0].y - pushed[0].y - bounds.height * 0.25).abs() < 1e-2);
        assert!((pushed[1].y - bounds.y).abs() < 1e-3);
        // Dragging the low handle to the middle of the box sets an output under the offset.
        let target = Point::new(bounds.x + 10.0, bounds.y + bounds.height / 2.0);
        drag_map_handle(&mut node, 0, target, 0.25, MapDriven::default());
        assert!((node.value(KnobId::MapMin) - 0.25).abs() < 1e-3);
        let landed = map_handles(&node, &live_of(&node))[0];
        assert!((landed.y - target.y).abs() < 1e-2);
    }

    /// The offset is an automatable knob with a pin, shows signed, and is typed as a percentage.
    #[test]
    fn the_offset_is_a_signed_pinned_knob() {
        let mut node = FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0));
        assert!(node.input_ports().contains(&Port::Knob(KnobId::MapOffset)));
        assert_eq!(node.plain(KnobId::MapOffset), 0.0);
        assert_eq!(
            KnobId::MapOffset.format(KnobId::MapOffset.to_normalized(0.25)),
            "+25%"
        );
        assert_eq!(
            KnobId::MapOffset.format(KnobId::MapOffset.to_normalized(-0.5)),
            "-50%"
        );
        node.enter_knob_value(KnobId::MapOffset, -30.0);
        assert!((node.plain(KnobId::MapOffset) + 0.3).abs() < 1e-4);
        assert_eq!(node.knob_entry_text(KnobId::MapOffset), "-30");
        // Bipolar changes the four range values, not the offset.
        node.set_switch(0, true);
        assert_eq!(node.knob_entry_text(KnobId::MapOffset), "-30");
    }

    /// An LFO on the offset moves a route through the map, and the offset moves the knob.
    #[test]
    fn an_lfo_can_drive_a_maps_offset() {
        use crate::nodes::connection::Connection;
        let nodes = vec![
            FlodeNode::new(NodeKind::Lfo, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Envelope, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0)),
        ];
        let links = vec![
            Connection {
                from: 1,
                out: Port::Shape,
                to: 2,
                input: Port::Shape,
            },
            Connection {
                from: 2,
                out: Port::Shape,
                to: 3,
                input: Port::Knob(KnobId::Cutoff),
            },
            Connection {
                from: 0,
                out: Port::Shape,
                to: 2,
                input: Port::Knob(KnobId::MapOffset),
            },
        ];
        let graph = Graph::with(nodes, links);
        let wiring = graph.wiring();
        let route = wiring.control_route(3, KnobId::Cutoff).unwrap();
        let at = |lfo: f32| {
            route.apply(&wiring.with_signals(ControlLevels {
                envelope: [0.4; MAX_ENVELOPES],
                lfo: [lfo; MAX_LFOS],
                ..ControlLevels::default()
            }))
        };
        assert!((at(0.5) - 0.4).abs() < 1e-5);
        assert!((at(0.6) - 0.6).abs() < 1e-5);
        assert!(graph.is_driven(2, KnobId::MapOffset));
    }
}

#[cfg(test)]
mod envelope_migration_tests {
    use super::*;

    /// An old save with an envelope in the sound chain loads with the envelope driving the
    /// oscillator's gain, and the audio running straight to the output.
    #[test]
    fn an_envelope_in_the_chain_becomes_a_gain_driver() {
        let node = |kind, x| SavedNode {
            kind,
            x,
            y: 0.0,
            collapsed: false,
            bypassed: false,
            sections_collapsed: Vec::new(),
            choices: Vec::new(),
            choice: 0,
            switches: Vec::new(),
            switched: false,
            knobs: Vec::new(),
            waveform: None,
            filter_type: None,
            out_min: None,
            out_max: None,
            points: Vec::new(),
            group: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            names: Vec::new(),
            sample_path: None,
        };
        let saved = SavedGraph {
            version: 0,
            preset_name: None,
            preset_category: String::new(),
            nodes: vec![
                node(NodeKind::Oscillator, 0.0),
                node(NodeKind::Envelope, 100.0),
                node(NodeKind::Output, 200.0),
            ],
            connections: Vec::new(),
            links: vec![
                Connection {
                    from: 0,
                    out: Port::Audio,
                    to: 1,
                    input: Port::Audio,
                },
                Connection {
                    from: 1,
                    out: Port::Audio,
                    to: 2,
                    input: Port::Slot(0),
                },
            ],
        };
        let graph = Graph::from_saved(&saved);
        // The oscillator now feeds the output directly.
        assert!(graph
            .connections
            .iter()
            .any(|c| c.from == 0 && c.out == Port::Audio && c.to == 2 && c.input == Port::Slot(0)));
        // The envelope drives the oscillator's amplitude.
        let lane = graph.patch().lanes[0].clone();
        assert!(lane.is_active());
        assert_eq!(osc_params(&lane, 0).envelope, Some(0));
    }
}

#[cfg(test)]
mod mix_tests {
    use super::*;

    /// A mix node between two oscillators and the output.
    fn chain() -> (Graph, usize, usize, usize) {
        let mut graph = minimal_graph();
        graph.connections.clear();
        let a = add_oscillator(&mut graph, Point::new(0.0, 0.0));
        let b = add_oscillator(&mut graph, Point::new(0.0, 0.0));
        let mix = graph.add_node(NodeKind::Mix, Point::new(0.0, 0.0));
        graph.connect(a, Port::Audio, mix, Port::AudioIn(0));
        graph.connect(b, Port::Audio, mix, Port::AudioIn(1));
        graph.connect(mix, Port::Audio, 1, Port::Slot(0));
        graph.refresh_slots();
        (graph, a, b, mix)
    }

    #[test]
    fn a_mix_node_combines_its_two_oscillators() {
        let (graph, _, _, _) = chain();
        let lane = graph.patch().lanes[0].clone();
        assert_eq!(lane.stages.len(), 3);
        assert!(matches!(lane.stages[2].params, StageParams::Mix(_)));
        assert_eq!(lane.stages[2].inputs, [Some(0), Some(1)]);
    }

    /// Each oscillator keeps its own waveform and knobs.
    #[test]
    fn each_oscillator_keeps_its_own_settings() {
        let (mut graph, a, b, _) = chain();
        graph.nodes[a].choices = vec![Waveform::Square.index()];
        graph.nodes[b].choices = vec![Waveform::Triangle.index()];
        graph.nodes[a].set_value(KnobId::Detune, KnobId::Detune.to_normalized(40.0));

        let lane = graph.patch().lanes[0].clone();
        let (first, second) = (osc_params(&lane, 0), osc_params(&lane, 1));
        assert_eq!(first.waveform, Waveform::Square);
        assert_eq!(second.waveform, Waveform::Triangle);
        assert_eq!(first.detune.plain(&ControlLevels::default()), 40.0);
    }

    /// The mix node's own dropdown and knob reach the patch.
    #[test]
    fn the_mix_type_and_amount_reach_the_patch() {
        let (mut graph, _, _, mix_node) = chain();
        graph.nodes[mix_node].choices = vec![2];
        graph.nodes[mix_node].set_value(KnobId::Mix, KnobId::Mix.to_normalized(0.75));

        let lane = graph.patch().lanes[0].clone();
        let StageParams::Mix(mix) = lane.stages[2].params else {
            panic!("not a mix");
        };
        assert_eq!(mix.mode, crate::dsp::mix::MixMode::RingMod);
        assert_eq!(mix.amount.plain(&ControlLevels::default()), 0.75);
    }

    /// A filter wired between an oscillator and the mix is part of that branch.
    #[test]
    fn a_filter_before_the_mix_reaches_the_patch() {
        let (mut graph, a, _, mix) = chain();
        let filter = graph.add_node(NodeKind::Filter, Point::new(0.0, 0.0));
        graph.connect(a, Port::Audio, filter, Port::Audio);
        graph.connect(filter, Port::Audio, mix, Port::AudioIn(0));

        let lane = graph.patch().lanes[0].clone();
        assert_eq!(lane.stages.len(), 4);
        let root = lane.stages.last().unwrap();
        let first_input = &lane.stages[root.inputs[0].unwrap() as usize];
        assert!(matches!(first_input.params, StageParams::Filter(_)));
    }

    /// A mix node dropped on an audio wire splices in with the old source on input A, the top one,
    /// and the mix feeding the old target.
    #[test]
    fn a_mix_dropped_on_an_audio_wire_joins_at_input_a() {
        let mut graph = minimal_graph();
        let curve = graph.link_curve(&graph.connections[0]).unwrap();
        let middle = bezier_point(&curve, 0.5);
        let mix = graph.add_node(NodeKind::Mix, Point::new(middle.x - 80.0, middle.y - 20.0));

        assert_eq!(graph.link_under(mix), Some(0));
        graph.insert_into(mix, 0);
        graph.refresh_slots();

        // The notes wire stays, and the audio wire becomes two.
        assert_eq!(graph.connections.len(), 3);
        assert_eq!(graph.connections[0].from, 0);
        assert_eq!(graph.connections[0].to, mix);
        assert_eq!(graph.connections[0].input, Port::AudioIn(0));
        assert_eq!(graph.connections[2].from, mix);
        assert_eq!(graph.connections[2].out, Port::Audio);
        assert_eq!(graph.connections[2].input, Port::Slot(0));

        // The oscillator feeds signal A, and signal B waits for a wire.
        let lane = graph.patch().lanes[0].clone();
        assert_eq!(lane.stages.len(), 2);
        assert_eq!(lane.stages[1].inputs, [Some(0), None]);
    }

    /// A source with no audio input still does not splice into a wire.
    #[test]
    fn an_oscillator_dropped_on_an_audio_wire_does_not_splice() {
        let mut graph = minimal_graph();
        let curve = graph.link_curve(&graph.connections[0]).unwrap();
        let middle = bezier_point(&curve, 0.5);
        let osc = graph.add_node(
            NodeKind::Oscillator,
            Point::new(middle.x - 80.0, middle.y - 20.0),
        );
        assert_eq!(graph.link_under(osc), None);
    }

    /// A chain without a mix node is a single oscillator.
    #[test]
    fn a_plain_oscillator_chain_is_one_stage() {
        let lane = minimal_graph().patch().lanes[0].clone();
        assert_eq!(lane.stages.len(), 1);
    }
}

#[cfg(test)]
mod lfo_retrigger_tests {
    use super::*;

    /// The Retrigger switch on an LFO node reaches the patch, and a save keeps it.
    #[test]
    fn the_retrigger_switch_reaches_the_patch_and_survives_saving() {
        let mut graph = minimal_graph();
        let lfo = graph.add_node(NodeKind::Lfo, Point::new(0.0, 200.0));
        assert_eq!(graph.nodes[lfo].switch_labels(), &["Retrigger", "One shot"]);
        assert!(!graph.patch().lfos[0].unwrap().retrigger);

        graph.nodes[lfo].set_switch(0, true);
        assert!(graph.patch().lfos[0].unwrap().retrigger);

        let loaded = Graph::from_saved(&graph.to_saved());
        assert!(loaded.patch().lfos[0].unwrap().retrigger);
    }
}

#[cfg(test)]
mod layering_tests {
    use super::*;
    use crate::editor::canvas::{Message, NodeEditor};
    use crate::editor::piano::{self, PianoEvent};
    use nih_plug_iced::mouse;
    use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};

    fn three_nodes() -> Graph {
        let mut graph = minimal_graph();
        graph.add_node(NodeKind::Filter, Point::new(0.0, 300.0));
        graph
    }

    /// Nodes draw in their own order, and the selected node draws last, so it is on top.
    #[test]
    fn the_selected_node_draws_last() {
        let mut graph = three_nodes();
        assert_eq!(graph.draw_order(), vec![0, 1, 2, 3]);
        graph.selected = Some(0);
        assert_eq!(graph.draw_order(), vec![1, 2, 3, 0]);
        graph.selected = Some(1);
        assert_eq!(graph.draw_order(), vec![0, 2, 3, 1]);
        graph.selected = None;
        assert_eq!(graph.draw_order(), vec![0, 1, 2, 3]);
    }

    /// A node being placed is on top too, and a stale selection never indexes past the nodes.
    #[test]
    fn a_node_being_placed_draws_on_top() {
        let mut graph = three_nodes();
        graph.interaction = Interaction::Placing {
            node: 0,
            grab: nih_plug_iced::Vector::new(0.0, 0.0),
        };
        assert_eq!(graph.draw_order().last(), Some(&0));
        graph.interaction = Interaction::Idle;
        graph.selected = Some(99);
        assert_eq!(graph.draw_order(), vec![0, 1, 2, 3]);
    }

    /// Where two nodes overlap, a press goes to the one on top, which is the selected one.
    #[test]
    fn a_press_goes_to_the_node_on_top() {
        let mut graph = minimal_graph();
        let a = graph.add_node(NodeKind::Reverb, Point::new(500.0, 100.0));
        let b = graph.add_node(NodeKind::Reverb, Point::new(520.0, 120.0));
        let overlap = Point::new(560.0, 160.0);
        // By default the later node is on top.
        assert_eq!(graph.node_at(overlap), Some(b));
        // Selecting the earlier one brings it to the front.
        graph.selected = Some(a);
        assert_eq!(graph.node_at(overlap), Some(a));
    }

    /// Text is hidden when any part of it is under a covering rectangle, and shown otherwise.
    #[test]
    fn text_is_hidden_only_where_it_overlaps_a_cover() {
        use nih_plug_iced::widget::canvas::Text;
        let label = Text {
            content: "Threshold".to_string(),
            position: Point::new(100.0, 100.0),
            size: 12.0,
            ..Text::default()
        };
        let area = text_area(&label);
        assert!(area.width > 40.0 && area.width < 90.0, "{}", area.width);
        let cover_at = |x: f32| Rectangle {
            x,
            y: 90.0,
            width: 50.0,
            height: 40.0,
        };
        // The cover starts after the label's first letters, so the start point is clear of it.
        assert!(overlaps(&cover_at(area.x + 20.0), &area));
        assert!(!overlaps(&cover_at(area.x + area.width + 5.0), &area));
        assert!(!overlaps(&cover_at(area.x - 60.0), &area));
    }

    /// With a dropdown list open over the key strip, a press on a row picks that row. It does not
    /// play the key under it.
    #[test]
    fn a_dropdown_row_over_the_keys_is_picked_not_played() {
        let bounds = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        };
        let strip = piano::strip_bounds(bounds);
        let mut graph = minimal_graph();
        // Move the oscillator so the last row of its Wave list lands 30 px into the strip.
        graph.nodes[0].position = Point::new(100.0, 0.0);
        let dropdown = graph.nodes[0].dropdowns().remove(0);
        let rows = dropdown.labels.len();
        let list = dropdown_list_bounds(dropdown.bounds, rows);
        let shift = strip.y + 30.0 - (list.y + list.height - 6.0);
        graph.nodes[0].position = Point::new(100.0, shift);
        let dropdown = graph.nodes[0].dropdowns().remove(0);
        graph.dropdown_menu = Some((0, dropdown.which));

        let list = dropdown_list_bounds(dropdown.bounds, rows);
        let last_row = Point::new(list.x + 10.0, list.y + list.height - 6.0);
        assert!(strip.contains(last_row), "the row is not over the strip");
        assert_eq!(list_row_at(list, last_row, rows), Some(rows - 1));

        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        let (_, message) = editor.update(
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds,
            Cursor::Available(last_row),
        );
        assert!(
            matches!(message, Some(Message::GraphChanged)),
            "{message:?}"
        );
        assert!(graph.mouse_note.is_none());
        assert!(graph.dropdown_menu.is_none());
        assert_eq!(graph.nodes[0].choice(dropdown.which), rows - 1);
    }

    /// With no list open, a press on the strip still plays a key.
    #[test]
    fn a_press_on_the_keys_still_plays_a_note() {
        let bounds = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        };
        let strip = piano::strip_bounds(bounds);
        let mut graph = minimal_graph();
        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        let (_, message) = editor.update(
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds,
            Cursor::Available(Point::new(500.0, strip.y + strip.height - 4.0)),
        );
        assert!(matches!(message, Some(Message::Note(_))), "{message:?}");
    }

    /// Presses the left mouse button at `at` in a 1024 by 712 editor.
    fn press_at(graph: &mut Graph, at: Point) -> Option<Message> {
        let bounds = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        };
        let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        let (_, message) = editor.update(
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds,
            Cursor::Available(at),
        );
        message
    }

    fn button_center(direction: i32) -> Point {
        let strip = piano::strip_bounds(Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        });
        // The buttons sit in the column at the strip's left, in its lower half.
        let x = if direction < 0 { 34.0 } else { 82.0 };
        Point::new(x, strip.y + 44.0)
    }

    /// The octave buttons shift the keys by an octave, and play no note.
    #[test]
    fn the_octave_buttons_shift_the_keys() {
        let mut graph = minimal_graph();
        assert_eq!(graph.piano.octave(), 0);
        assert!(press_at(&mut graph, button_center(1)).is_none());
        assert_eq!(graph.piano.octave(), 1);
        assert!(press_at(&mut graph, button_center(1)).is_none());
        assert_eq!(graph.piano.octave(), 2);
        assert!(press_at(&mut graph, button_center(-1)).is_none());
        assert_eq!(graph.piano.octave(), 1);
        assert!(graph.mouse_note.is_none());
    }

    /// After a shift, a click on the first key plays the note an octave up.
    #[test]
    fn a_click_after_a_shift_plays_the_shifted_note() {
        let mut graph = minimal_graph();
        press_at(&mut graph, button_center(1));
        let strip = piano::strip_bounds(Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        });
        let first_key = Point::new(piano::CONTROLS_WIDTH + 4.0, strip.y + strip.height - 4.0);
        let message = press_at(&mut graph, first_key);
        assert!(matches!(
            message,
            Some(Message::Note(PianoEvent::NoteOn { note: 60, .. }))
        ));
    }

    /// A click on the strip plays a note, and the octave keys shift the strip. This used to also
    /// check that a plugin, as opposed to the standalone app, kept the computer keyboard from
    /// playing. That gate is gone: see `mode_tests` for what replaced it. A new graph starts in
    /// Keyboard mode either way, so the keyboard plays here too.
    #[test]
    fn a_click_on_the_strip_plays_and_the_octave_keys_shift_it() {
        use nih_plug_iced::keyboard::{self, KeyCode, Modifiers};
        let bounds = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        };
        let mut graph = minimal_graph();
        let key = |graph: &mut Graph, key_code| {
            let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
            editor
                .update(
                    Event::Keyboard(keyboard::Event::KeyPressed {
                        key_code,
                        modifiers: Modifiers::default(),
                    }),
                    bounds,
                    Cursor::Unavailable,
                )
                .1
        };
        key(&mut graph, KeyCode::X);
        assert_eq!(graph.piano.octave(), 1);

        let strip = piano::strip_bounds(bounds);
        let message = press_at(
            &mut graph,
            Point::new(piano::CONTROLS_WIDTH + 4.0, strip.y + strip.height - 4.0),
        );
        assert!(matches!(message, Some(Message::Note(_))), "{message:?}");
    }

    /// The octave column has no keys. A press on it, away from the buttons, plays nothing and does
    /// not start dragging the graph.
    #[test]
    fn a_press_in_the_octave_column_plays_nothing() {
        let mut graph = minimal_graph();
        let strip = piano::strip_bounds(Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        });
        assert!(press_at(&mut graph, Point::new(6.0, strip.y + 4.0)).is_none());
        assert!(graph.mouse_note.is_none());
        assert_eq!(graph.piano.octave(), 0);
    }
}

#[cfg(test)]
mod bypass_and_duplicate_tests {
    use super::*;
    use crate::editor::canvas::{Message, NodeEditor};
    use crate::editor::piano::PianoEvent;
    use crate::editor::style::MENU_ITEM_HEIGHT;
    use crate::nodes::{HEADER_HEIGHT, NODE_WIDTH};
    use nih_plug_iced::keyboard::{self, KeyCode, Modifiers};
    use nih_plug_iced::mouse;
    use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};

    fn bounds() -> Rectangle {
        Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        }
    }

    /// An oscillator through a filter into the output: nodes 0, 2, 1 (the default graph has the
    /// oscillator at 0 and the output at 1).
    fn osc_filter_output() -> (Graph, usize) {
        let mut graph = minimal_graph();
        let filter = graph.add_node(NodeKind::Filter, Point::new(300.0, 300.0));
        let wire = graph
            .connections
            .iter()
            .position(|c| c.to == 1)
            .expect("the default wire");
        graph.insert_into(filter, wire);
        (graph, filter)
    }

    fn stage_count(graph: &Graph) -> usize {
        graph.patch().lanes[0].stages.len()
    }

    fn press(editor: &mut NodeEditor, key_code: KeyCode, modifiers: Modifiers) -> Option<Message> {
        editor
            .update(
                Event::Keyboard(keyboard::Event::KeyPressed {
                    key_code,
                    modifiers,
                }),
                bounds(),
                Cursor::Unavailable,
            )
            .1
    }

    fn release(editor: &mut NodeEditor, key_code: KeyCode) {
        editor.update(
            Event::Keyboard(keyboard::Event::KeyReleased {
                key_code,
                modifiers: Modifiers::default(),
            }),
            bounds(),
            Cursor::Unavailable,
        );
    }

    fn editor(graph: &mut Graph) -> NodeEditor<'_> {
        NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2])
    }

    // The node model.

    /// Sources mute and processors bypass. The nodes with nothing to switch off refuse.
    #[test]
    fn only_some_kinds_can_be_switched_off() {
        for kind in NodeKind::ALL {
            let mut node = FlodeNode::new(kind, Point::new(0.0, 0.0));
            let expected = match kind {
                NodeKind::Oscillator | NodeKind::Sampler => crate::nodes::Bypass::Mute,
                NodeKind::Output
                | NodeKind::Envelope
                | NodeKind::Lfo
                | NodeKind::Input
                | NodeKind::Arpeggiator
                | NodeKind::Chord
                | NodeKind::Macro
                | NodeKind::Map
                | NodeKind::Math
                | NodeKind::Monitor
                | NodeKind::Wavetable
                | NodeKind::CurveFrame
                | NodeKind::Harmonics
                | NodeKind::Sweep
                | NodeKind::WaveFile
                | NodeKind::GroupInput
                | NodeKind::GroupOutput => crate::nodes::Bypass::None,
                _ => crate::nodes::Bypass::PassThrough,
            };
            assert_eq!(node.bypass(), expected, "{kind:?}");
            let toggled = node.toggle_bypass();
            assert_eq!(toggled, expected != crate::nodes::Bypass::None, "{kind:?}");
            assert_eq!(node.bypassed, toggled, "{kind:?}");
        }
    }

    /// A switched-off node names how, and a live one names nothing.
    #[test]
    fn a_switched_off_node_shows_its_state() {
        let mut osc = FlodeNode::new(NodeKind::Oscillator, Point::new(0.0, 0.0));
        let mut filter = FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0));
        assert_eq!(osc.bypass_label(), None);
        osc.toggle_bypass();
        filter.toggle_bypass();
        assert_eq!(osc.bypass_label(), Some("mute"));
        assert_eq!(filter.bypass_label(), Some("bypass"));
    }

    // What the audio thread plays.

    /// A muted oscillator is silence, so its slot has no stages.
    #[test]
    fn a_muted_oscillator_makes_no_sound() {
        let mut graph = minimal_graph();
        assert_eq!(stage_count(&graph), 1);
        assert!(graph.toggle_bypass(0));
        assert_eq!(stage_count(&graph), 0);
        assert!(graph.toggle_bypass(0));
        assert_eq!(stage_count(&graph), 1);
    }

    /// A bypassed filter hands the oscillator straight to the output.
    #[test]
    fn a_bypassed_filter_passes_its_input_through() {
        let (mut graph, filter) = osc_filter_output();
        assert_eq!(stage_count(&graph), 2);
        graph.toggle_bypass(filter);
        let lane = graph.patch().lanes[0].clone();
        assert_eq!(lane.stages.len(), 1);
        assert!(matches!(lane.stages[0].params, StageParams::Oscillator(_)));
    }

    /// A bypassed effect at the end of a chain leaves the per-voice path to play, with no tap.
    #[test]
    fn a_bypassed_effect_leaves_the_voices_to_play() {
        let mut graph = minimal_graph();
        let reverb = graph.add_node(NodeKind::Reverb, Point::new(300.0, 300.0));
        let wire = graph.connections.iter().position(|c| c.to == 1).unwrap();
        graph.insert_into(reverb, wire);
        let with_effect = graph.patch().lanes[0].clone();
        assert!(with_effect.stages.last().unwrap().params.is_effect());
        assert!(with_effect.stages[0].tap);

        graph.toggle_bypass(reverb);
        let bypassed = graph.patch().lanes[0].clone();
        assert_eq!(bypassed.stages.len(), 1);
        assert!(!bypassed.stages[0].tap);
    }

    /// A bypassed mix plays signal A, and a muted source on B is silence in the mix.
    #[test]
    fn a_bypassed_mix_plays_a_and_a_muted_source_is_silent() {
        let mut graph = minimal_graph();
        let second = add_oscillator(&mut graph, Point::new(0.0, 300.0));
        let mix = graph.add_node(NodeKind::Mix, Point::new(300.0, 300.0));
        let wire = graph.connections.iter().position(|c| c.to == 1).unwrap();
        graph.insert_into(mix, wire);
        graph.connect(second, Port::Audio, mix, Port::AudioIn(1));
        assert_eq!(stage_count(&graph), 3);

        graph.toggle_bypass(mix);
        assert_eq!(stage_count(&graph), 1);
        graph.toggle_bypass(mix);

        graph.toggle_bypass(second);
        let lane = graph.patch().lanes[0].clone();
        assert_eq!(lane.stages.len(), 2);
        assert_eq!(lane.stages[1].inputs[1], None);
    }

    /// A node that can't be switched off ignores a stray flag, so an old or edited save is safe.
    #[test]
    fn the_flag_on_a_node_that_cannot_be_switched_off_does_nothing() {
        let mut graph = minimal_graph();
        graph.nodes[1].bypassed = true;
        assert_eq!(stage_count(&graph), 1);
        assert!(!graph.toggle_bypass(1));
    }

    // Saving.

    /// The off state survives saving, and a save from before it loads switched on.
    #[test]
    fn the_off_state_survives_saving() {
        let mut graph = minimal_graph();
        graph.toggle_bypass(0);
        let loaded = Graph::from_saved(&graph.to_saved());
        assert!(loaded.nodes[0].bypassed);
        assert_eq!(loaded.patch().lanes[0].stages.len(), 0);

        let mut json = serde_json::to_value(graph.to_saved()).unwrap();
        for node in json["nodes"].as_array_mut().unwrap() {
            node.as_object_mut().unwrap().remove("bypassed");
        }
        let old: SavedGraph = serde_json::from_value(json).unwrap();
        assert!(!Graph::from_saved(&old).nodes[0].bypassed);
    }

    // Duplicate.

    /// A copy has the same settings, sits offset from the original, has no wires, and is selected.
    #[test]
    fn a_duplicate_copies_the_settings_but_not_the_wires() {
        let mut graph = minimal_graph();
        graph.nodes[0].set_value(KnobId::Unison, KnobId::Unison.to_normalized(5.0));
        graph.nodes[0].choose(1, 2);
        graph.nodes[0].toggle_bypass();
        let wires = graph.connections.len();
        let count = graph.nodes.len();

        let copy = graph.duplicate_node(0).expect("an oscillator duplicates");
        assert_eq!(graph.nodes.len(), count + 1);
        assert_eq!(graph.nodes[copy].kind, NodeKind::Oscillator);
        assert_eq!(graph.nodes[copy].plain(KnobId::Unison), 5.0);
        assert_eq!(
            oscillator::warp_mode(&graph.nodes[copy]),
            crate::dsp::warp::WarpMode::Sync
        );
        assert!(graph.nodes[copy].bypassed);
        assert!(graph.nodes[copy].position.x > graph.nodes[0].position.x);
        assert!(graph.nodes[copy].position.y > graph.nodes[0].position.y);
        assert_eq!(graph.connections.len(), wires);
        assert!(!graph
            .connections
            .iter()
            .any(|c| c.from == copy || c.to == copy));
        assert_eq!(graph.selected, Some(copy));
    }

    /// The copy is its own node. Turning a knob on it leaves the original alone.
    #[test]
    fn a_duplicate_does_not_share_knobs_with_the_original() {
        let mut graph = minimal_graph();
        let copy = graph.duplicate_node(0).unwrap();
        graph.nodes[copy].set_value(KnobId::Detune, 1.0);
        assert_eq!(
            graph.nodes[0].plain(KnobId::Detune),
            KnobId::Detune.default_plain()
        );
    }

    /// An output node is one per patch, so it can't be copied.
    #[test]
    fn an_output_node_cannot_be_duplicated() {
        let mut graph = minimal_graph();
        let count = graph.nodes.len();
        assert_eq!(graph.duplicate_node(1), None);
        assert_eq!(graph.nodes.len(), count);
        assert_eq!(graph.duplicate_node(9), None);
    }

    /// The macro node is one per graph too. It can't be copied or added a second time.
    #[test]
    fn a_macro_node_is_one_per_graph() {
        let mut graph = minimal_graph();
        assert!(graph.can_add(NodeKind::Macro));
        let macros = graph.add_node(NodeKind::Macro, Point::new(0.0, 0.0));
        assert!(!graph.can_add(NodeKind::Macro));
        assert!(!graph.can_add(NodeKind::Output));
        assert!(graph.can_add(NodeKind::Oscillator));
        assert!(graph.can_add(NodeKind::Input));
        let count = graph.nodes.len();
        assert_eq!(graph.duplicate_node(macros), None);
        assert_eq!(graph.nodes.len(), count);
        // Deleting it makes room for another.
        graph.delete_node(macros);
        assert!(graph.can_add(NodeKind::Macro));
    }

    // The right-click menu.

    #[test]
    fn each_kind_offers_the_entries_that_apply() {
        use crate::editor::menus::{menu_items, MenuItem};
        let items = |kind| menu_items(&FlodeNode::new(kind, Point::new(0.0, 0.0)));
        assert_eq!(
            items(NodeKind::Oscillator),
            &[
                MenuItem::Duplicate,
                MenuItem::Bypass,
                MenuItem::DeleteInPlace,
                MenuItem::Delete
            ]
        );
        assert_eq!(
            items(NodeKind::Reverb),
            &[
                MenuItem::Duplicate,
                MenuItem::Bypass,
                MenuItem::DeleteInPlace,
                MenuItem::Delete
            ]
        );
        assert_eq!(
            items(NodeKind::Lfo),
            &[
                MenuItem::Duplicate,
                MenuItem::DeleteInPlace,
                MenuItem::Delete
            ]
        );
        assert_eq!(
            items(NodeKind::Output),
            &[MenuItem::DeleteInPlace, MenuItem::Delete]
        );
    }

    /// The bypass entry says what it will do, and flips after it is used.
    #[test]
    fn the_bypass_entry_names_its_action() {
        use crate::editor::menus::MenuItem;
        let mut osc = FlodeNode::new(NodeKind::Oscillator, Point::new(0.0, 0.0));
        let mut filter = FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0));
        assert_eq!(MenuItem::Bypass.label(&osc), "Mute");
        assert_eq!(MenuItem::Bypass.label(&filter), "Bypass");
        osc.toggle_bypass();
        filter.toggle_bypass();
        assert_eq!(MenuItem::Bypass.label(&osc), "Unmute");
        assert_eq!(MenuItem::Bypass.label(&filter), "Enable");
        assert_eq!(MenuItem::Duplicate.label(&osc), "Duplicate");
        assert_eq!(MenuItem::Delete.label(&osc), "Delete");
    }

    /// The menu is as tall as its entries, and a press picks the row it lands on.
    #[test]
    fn a_press_picks_the_entry_under_it() {
        use crate::editor::menus::{menu_bounds, menu_item_at, menu_items, ContextMenu, MenuItem};
        let node = FlodeNode::new(NodeKind::Oscillator, Point::new(0.0, 0.0));
        let menu = ContextMenu {
            node: 0,
            position: Point::new(50.0, 60.0),
            items: menu_items(&node),
            target: crate::editor::menus::MenuTarget::Node,
        };
        let area = menu_bounds(menu);
        assert_eq!(area.height, 4.0 * MENU_ITEM_HEIGHT);
        let at = |row: f32| Point::new(60.0, 60.0 + (row + 0.5) * MENU_ITEM_HEIGHT);
        assert_eq!(menu_item_at(menu, at(0.0)), Some(MenuItem::Duplicate));
        assert_eq!(menu_item_at(menu, at(1.0)), Some(MenuItem::Bypass));
        assert_eq!(menu_item_at(menu, at(2.0)), Some(MenuItem::DeleteInPlace));
        assert_eq!(menu_item_at(menu, at(3.0)), Some(MenuItem::Delete));
        assert_eq!(menu_item_at(menu, at(4.0)), None);
        assert_eq!(menu_item_at(menu, Point::new(10.0, 70.0)), None);
    }

    /// Right-click a node, then click each entry. Each does its job through the editor.
    #[test]
    fn the_menu_entries_work_through_the_editor() {
        use crate::editor::menus::menu_bounds;
        let click_entry = |graph: &mut Graph, row: usize| -> Option<Message> {
            let target = graph.nodes[0].position;
            let inside = Point::new(target.x + 20.0, target.y + 12.0);
            let mut editor = editor(graph);
            editor.update(
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)),
                bounds(),
                Cursor::Available(inside),
            );
            let menu = editor.graph.menu.expect("the menu opens");
            let area = menu_bounds(menu);
            let entry = Point::new(
                area.x + 10.0,
                area.y + (row as f32 + 0.5) * MENU_ITEM_HEIGHT,
            );
            editor
                .update(
                    Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                    bounds(),
                    Cursor::Available(entry),
                )
                .1
        };

        let mut graph = minimal_graph();
        let message = click_entry(&mut graph, 1);
        assert!(matches!(message, Some(Message::GraphChanged)));
        assert!(graph.nodes[0].bypassed);
        assert!(graph.menu.is_none());

        // Duplicate leaves the copy in the placing state, following the cursor.
        let mut graph = minimal_graph();
        click_entry(&mut graph, 0);
        assert_eq!(graph.nodes.len(), 4);
        assert_eq!(graph.selected, Some(3));
        assert!(matches!(
            graph.interaction,
            Interaction::Placing { node: 3, .. }
        ));

        let mut graph = minimal_graph();
        click_entry(&mut graph, 2);
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.nodes[0].kind, NodeKind::Output);
    }

    // Shortcuts.

    /// M mutes the selected node, and pressing it again brings it back.
    #[test]
    fn m_toggles_the_selected_node() {
        let mut graph = minimal_graph();
        graph.selected = Some(0);
        let mut editor = editor(&mut graph);
        let message = press(&mut editor, KeyCode::M, Modifiers::default());
        assert!(matches!(message, Some(Message::GraphChanged)));
        assert!(editor.graph.nodes[0].bypassed);
        release(&mut editor, KeyCode::M);
        press(&mut editor, KeyCode::M, Modifiers::default());
        assert!(!editor.graph.nodes[0].bypassed);
    }

    /// Holding the key repeats its press. The node toggles once, not on every repeat.
    #[test]
    fn holding_m_toggles_once() {
        let mut graph = minimal_graph();
        graph.selected = Some(0);
        let mut editor = editor(&mut graph);
        press(&mut editor, KeyCode::M, Modifiers::default());
        for _ in 0..5 {
            let message = press(&mut editor, KeyCode::M, Modifiers::default());
            assert!(message.is_none());
        }
        assert!(editor.graph.nodes[0].bypassed);
        release(&mut editor, KeyCode::M);
        press(&mut editor, KeyCode::M, Modifiers::default());
        assert!(!editor.graph.nodes[0].bypassed);
    }

    /// With nothing selected, or a node that can't be switched off, M does nothing.
    #[test]
    fn m_needs_a_node_that_can_be_switched_off() {
        let mut graph = minimal_graph();
        let mut editor = editor(&mut graph);
        assert!(press(&mut editor, KeyCode::M, Modifiers::default()).is_none());
        release(&mut editor, KeyCode::M);
        editor.graph.selected = Some(1);
        assert!(press(&mut editor, KeyCode::M, Modifiers::default()).is_none());
        assert!(!editor.graph.nodes[1].bypassed);
    }

    fn move_cursor(editor: &mut NodeEditor, to: Point) {
        editor.update(
            Event::Mouse(mouse::Event::CursorMoved { position: to }),
            bounds(),
            Cursor::Available(to),
        );
    }

    /// Shift+D copies the selected node and selects the copy.
    #[test]
    fn shift_d_duplicates_the_selected_node() {
        let mut graph = minimal_graph();
        graph.selected = Some(0);
        let mut editor = editor(&mut graph);
        let message = press(&mut editor, KeyCode::D, Modifiers::SHIFT);
        assert!(matches!(message, Some(Message::GraphChanged)));
        assert_eq!(editor.graph.nodes.len(), 4);
        assert_eq!(editor.graph.selected, Some(3));
    }

    /// The copy is placing: it sits centred under the cursor, follows it, and the next press sets
    /// it down where it is.
    #[test]
    fn a_duplicate_follows_the_cursor_until_the_next_press() {
        use nih_plug_iced::Vector;
        let mut graph = minimal_graph();
        graph.selected = Some(0);
        let mut editor = editor(&mut graph);
        let grab = Vector::new(NODE_WIDTH / 2.0, HEADER_HEIGHT / 2.0);

        let first = Point::new(400.0, 300.0);
        move_cursor(&mut editor, first);
        press(&mut editor, KeyCode::D, Modifiers::SHIFT);
        assert!(matches!(
            editor.graph.interaction,
            Interaction::Placing { node: 3, .. }
        ));
        let at = editor.graph.nodes[3].position;
        assert_eq!((at.x, at.y), (first.x - grab.x, first.y - grab.y));

        // It follows the cursor.
        let second = Point::new(600.0, 250.0);
        move_cursor(&mut editor, second);
        let at = editor.graph.nodes[3].position;
        assert_eq!((at.x, at.y), (second.x - grab.x, second.y - grab.y));

        // The next press sets it down, and it stays when the cursor moves on.
        editor.update(
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds(),
            Cursor::Available(second),
        );
        assert!(matches!(editor.graph.interaction, Interaction::Idle));
        move_cursor(&mut editor, Point::new(100.0, 100.0));
        let at = editor.graph.nodes[3].position;
        assert_eq!((at.x, at.y), (second.x - grab.x, second.y - grab.y));
    }

    /// While a copy is being placed, Shift+D does not make another.
    #[test]
    fn shift_d_waits_while_a_node_is_being_placed() {
        let mut graph = minimal_graph();
        graph.selected = Some(0);
        let mut editor = editor(&mut graph);
        press(&mut editor, KeyCode::D, Modifiers::SHIFT);
        release(&mut editor, KeyCode::D);
        let message = press(&mut editor, KeyCode::D, Modifiers::SHIFT);
        assert!(message.is_none());
        assert_eq!(editor.graph.nodes.len(), 4);
    }

    /// A plain D still plays a note, and does not copy anything.
    #[test]
    fn a_plain_d_still_plays_the_piano() {
        let mut graph = minimal_graph();
        graph.selected = Some(0);
        let mut editor = editor(&mut graph);
        let message = press(&mut editor, KeyCode::D, Modifiers::default());
        assert!(matches!(
            message,
            Some(Message::Note(PianoEvent::NoteOn { .. }))
        ));
        assert_eq!(editor.graph.nodes.len(), 3);
    }

    /// Shortcuts that include Control or Command are left to the host.
    #[test]
    fn control_and_command_combinations_are_ignored() {
        let mut graph = minimal_graph();
        graph.selected = Some(0);
        let mut editor = editor(&mut graph);
        assert!(press(&mut editor, KeyCode::M, Modifiers::CTRL).is_none());
        assert!(press(&mut editor, KeyCode::M, Modifiers::LOGO).is_none());
        assert!(!editor.graph.nodes[0].bypassed);
    }
}

#[cfg(test)]
mod default_graph_tests {
    use super::*;

    /// A new instance starts with an oscillator and an output, wired together, an envelope wired
    /// to the oscillator's shape pin, and a MIDI input wired to its notes pin.
    #[test]
    fn the_default_graph_has_an_oscillator_an_output_an_envelope_and_a_midi_input() {
        let graph = Graph::default();
        let kinds: Vec<NodeKind> = graph.nodes.iter().map(|n| n.kind).collect();
        assert_eq!(
            kinds,
            [
                NodeKind::Oscillator,
                NodeKind::Output,
                NodeKind::Envelope,
                NodeKind::Input
            ]
        );
        assert!(graph.connections.contains(&Connection {
            from: 3,
            out: Port::Notes,
            to: 0,
            input: Port::Notes,
        }));
        assert!(graph.connections.contains(&Connection {
            from: 0,
            out: Port::Audio,
            to: 1,
            input: Port::Slot(0),
        }));
        assert!(graph.connections.contains(&Connection {
            from: 2,
            out: Port::Shape,
            to: 0,
            input: Port::Shape,
        }));
        assert_eq!(graph.connections.len(), 3);
    }

    /// Every wire of the default graph is valid, so none is dropped on a save and load.
    #[test]
    fn every_default_wire_is_valid_and_survives_saving() {
        let graph = Graph::default();
        assert!(graph.connections.iter().all(|c| c.is_valid(&graph.nodes)));
        let loaded = Graph::from_saved(&graph.to_saved());
        assert_eq!(loaded.connections, graph.connections);
        assert_eq!(loaded.nodes.len(), 4);
    }

    /// The envelope reaches the patch, and the oscillator takes its amplitude from it.
    #[test]
    fn the_envelope_shapes_the_oscillator_in_the_patch() {
        let patch = Graph::default().patch();
        assert!(patch.has_envelopes());
        assert!(patch.envelopes[0].is_some());
        let lane = &patch.lanes[0];
        assert_eq!(lane.stages.len(), 1);
        assert_eq!(osc_params(lane, 0).envelope, Some(0));
    }

    /// The nodes do not overlap, and all sit inside the default window above the key strip. The
    /// MIDI input, envelope, and oscillator run left to right, then the output.
    #[test]
    fn the_default_nodes_do_not_overlap() {
        let graph = Graph::default();
        let bounds: Vec<Rectangle> = graph.nodes.iter().map(|n| n.bounds()).collect();
        let strip_top = 712.0 - crate::editor::piano::STRIP_HEIGHT;
        for (i, a) in bounds.iter().enumerate() {
            assert!(
                a.x >= 0.0 && a.x + a.width <= 1024.0,
                "node {i} is off screen"
            );
            assert!(a.y + a.height <= strip_top, "node {i} reaches the keys");
            for b in &bounds[i + 1..] {
                let apart = a.x + a.width <= b.x
                    || b.x + b.width <= a.x
                    || a.y + a.height <= b.y
                    || b.y + b.height <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
        assert!(graph.nodes[2].position.x < graph.nodes[0].position.x);
        assert!(graph.nodes[3].position.x < graph.nodes[0].position.x);
    }

    /// The default graph plays: a note makes sound, and it ends after release.
    #[test]
    fn the_default_graph_plays_a_note_and_lets_it_go() {
        use crate::dsp::wavetable::WaveTables;
        use crate::engine::{Engine, Transport};
        use std::sync::Arc;
        let patch = Graph::default().patch();
        let mut engine = Engine::new(Arc::new(WaveTables::new()));
        engine.begin_block(&patch, Transport::default());
        engine.note_on(60, 1.0);
        let mut peak = 0.0f32;
        for n in 0..4410 {
            peak = peak.max(engine.process_sample(n).peak());
        }
        assert!(peak > 0.05, "{peak}");

        engine.note_off(60);
        for n in 0..44100 {
            engine.process_sample(n);
        }
        let after = (0..100)
            .map(|n| engine.process_sample(n).peak())
            .fold(0.0f32, f32::max);
        assert_eq!(after, 0.0);
    }
}

#[cfg(test)]
mod section_tests {
    use super::*;
    use crate::editor::canvas::{Message, NodeEditor};
    use crate::nodes::{
        RowShape, COLLAPSED_PADDING, HEADER_HEIGHT, NODE_WIDTH, PACKED_PITCH, PIN_ROW0_Y,
        ROW_PITCH, SECTION_HEIGHT,
    };
    use nih_plug_iced::mouse;
    use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};

    fn node(kind: NodeKind) -> FlodeNode {
        FlodeNode::new(kind, Point::new(100.0, 100.0))
    }

    // Nodes with no sections lay out as before.

    /// A node with no sections puts its rows at the old steady pitch, so the nodes that never
    /// needed sections do not move.
    #[test]
    fn a_node_with_no_sections_keeps_its_old_layout() {
        for kind in [
            NodeKind::Mix,
            NodeKind::Reverb,
            NodeKind::Delay,
            NodeKind::Chorus,
        ] {
            let node = node(kind);
            for (index, place) in node.row_places().iter().enumerate() {
                assert_eq!(place.shape, RowShape::Full, "{kind:?}");
                assert_eq!(place.y, PIN_ROW0_Y + index as f32 * ROW_PITCH, "{kind:?}");
            }
        }
    }

    // Sections.

    /// The oscillator starts with pitch and warp folded and unison open.
    #[test]
    fn the_oscillator_starts_with_pitch_and_warp_folded() {
        let osc = node(NodeKind::Oscillator);
        assert_eq!(osc.sections_collapsed, vec![true, false, true]);
        assert!(osc.section_collapsed(0) && !osc.section_collapsed(1) && osc.section_collapsed(2));
    }

    /// A folded section keeps its pins, packed together, and drops its knobs.
    #[test]
    fn a_folded_section_keeps_its_pins_and_drops_its_knobs() {
        let osc = node(NodeKind::Oscillator);
        let rows = osc.rows();
        let packed: Vec<usize> = (0..rows.len())
            .filter(|&i| osc.row_places()[i].shape == RowShape::Packed)
            .collect();
        // Coarse, fine, and octave are folded and keep their pins, and so do the warp row's audio
        // pin and the warp amount.
        assert_eq!(packed.len(), 5);
        assert!(osc.input_point(Port::AudioIn(0)).is_some());
        assert!(osc.input_point(Port::Knob(KnobId::Coarse)).is_some());
        assert!(osc.input_point(Port::Knob(KnobId::Fine)).is_some());
        assert!(osc.input_point(Port::Knob(KnobId::Octave)).is_some());
        assert!(osc.input_point(Port::Knob(KnobId::WarpAmount)).is_some());
        let shown: Vec<KnobId> = osc.knob_slots().iter().map(|slot| slot.id).collect();
        assert!(!shown.contains(&KnobId::Coarse));
        assert!(!shown.contains(&KnobId::WarpAmount));
        assert!(shown.contains(&KnobId::Unison) && shown.contains(&KnobId::OscGain));
    }

    /// Packed pins sit one packed pitch apart, and a pin of an open section sits a full row down.
    #[test]
    fn packed_pins_sit_close_together() {
        let mut osc = node(NodeKind::Oscillator);
        let at = |osc: &FlodeNode, id| osc.input_point(Port::Knob(id)).unwrap();
        let (coarse, fine, octave) = (
            at(&osc, KnobId::Coarse),
            at(&osc, KnobId::Fine),
            at(&osc, KnobId::Octave),
        );
        assert_eq!(fine.y - coarse.y, PACKED_PITCH);
        assert_eq!(octave.y - fine.y, PACKED_PITCH);
        assert_eq!(coarse.x, osc.position.x);

        osc.toggle_section(0);
        let (coarse, fine) = (at(&osc, KnobId::Coarse), at(&osc, KnobId::Fine));
        assert_eq!(fine.y - coarse.y, ROW_PITCH);
    }

    /// Folding a section moves what is under it up, and opening it moves it back.
    #[test]
    fn folding_a_section_moves_the_rows_under_it() {
        let mut osc = node(NodeKind::Oscillator);
        let unison = |osc: &FlodeNode| osc.input_point(Port::Knob(KnobId::Unison)).unwrap().y;
        let height = |osc: &FlodeNode| osc.height();
        let (closed_unison, closed_height) = (unison(&osc), height(&osc));
        osc.toggle_section(0);
        assert!(unison(&osc) > closed_unison);
        assert!(height(&osc) > closed_height);
        osc.toggle_section(0);
        assert_eq!(unison(&osc), closed_unison);
        assert_eq!(height(&osc), closed_height);
    }

    /// The oscillator's envelope input and audio output share a row above every section, so
    /// neither moves when a section folds.
    #[test]
    fn the_envelope_pin_and_audio_pin_do_not_move_when_sections_fold() {
        let mut osc = node(NodeKind::Oscillator);
        let before = (
            osc.input_point(Port::Shape).unwrap(),
            osc.output_point(Port::Audio).unwrap(),
        );
        assert_eq!(before.0.y, before.1.y);
        for section in 0..3 {
            osc.toggle_section(section);
        }
        assert_eq!(osc.input_point(Port::Shape).unwrap(), before.0);
        assert_eq!(osc.output_point(Port::Audio).unwrap(), before.1);
    }

    /// The default oscillator is short enough to sit above the key strip in the default window.
    #[test]
    fn the_default_oscillator_clears_the_keyboard() {
        let graph = Graph::default();
        let bottom = graph.nodes[0].position.y + graph.nodes[0].height();
        let strip_top = 712.0 - crate::editor::piano::STRIP_HEIGHT;
        assert!(bottom <= strip_top, "bottom {bottom}, strip {strip_top}");
    }

    /// A dropdown in a folded section is not on show, and one in an open section is.
    #[test]
    fn a_dropdown_in_a_folded_section_is_hidden() {
        let mut osc = node(NodeKind::Oscillator);
        let warp = |osc: &FlodeNode| osc.dropdowns().iter().any(|d| d.which == 1);
        assert!(!warp(&osc));
        let at = Point::new(osc.position.x + 40.0, osc.position.y + 200.0);
        assert!(osc.dropdown_at(at).is_none());
        osc.toggle_section(2);
        assert!(warp(&osc));
        // The wave dropdown above the sections never hides.
        assert!(osc.dropdowns().iter().any(|d| d.which == 0));
    }

    /// A switch above a folded section stays on show, and folding does not touch other nodes.
    #[test]
    fn a_switch_above_a_folded_section_stays_on_show() {
        let compressor = node(NodeKind::Compressor);
        assert!(compressor.section_collapsed(0));
        assert!(compressor.switch_shown(0));
        let delay = node(NodeKind::Delay);
        assert!(delay.switch_shown(0));
    }

    /// Toggling a section that does not exist does nothing.
    #[test]
    fn toggling_a_missing_section_does_nothing() {
        let mut filter = node(NodeKind::Filter);
        filter.toggle_section(0);
        assert!(filter.sections_collapsed.is_empty());
        let mut osc = node(NodeKind::Oscillator);
        osc.toggle_section(9);
        assert_eq!(osc.sections_collapsed, vec![true, false, true]);
    }

    // Clicks.

    fn click(graph: &mut Graph, at: Point) -> Option<Message> {
        let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        editor
            .update(
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                Rectangle {
                    x: 0.0,
                    y: 0.0,
                    width: 1024.0,
                    height: 712.0,
                },
                Cursor::Available(at),
            )
            .1
    }

    /// A press on a section's header folds or opens it, and selects the node.
    #[test]
    fn pressing_a_section_header_folds_it() {
        let mut graph = minimal_graph();
        let header = graph.nodes[0]
            .row_places()
            .iter()
            .position(|p| p.shape == RowShape::SectionHeader(0))
            .unwrap();
        let y = graph.nodes[0].position.y + graph.nodes[0].row_places()[header].y;
        let at = Point::new(graph.nodes[0].position.x + 80.0, y);
        assert!(graph.nodes[0].section_collapsed(0));
        graph.selected = None;
        assert!(matches!(click(&mut graph, at), Some(Message::GraphChanged)));
        assert!(!graph.nodes[0].section_collapsed(0));
        assert_eq!(graph.selected, Some(0));
        click(&mut graph, at);
        assert!(graph.nodes[0].section_collapsed(0));
    }

    /// The header is the whole band across the node, and a click below it is not a header press.
    #[test]
    fn only_the_header_band_folds_a_section() {
        let osc = node(NodeKind::Oscillator);
        let places = osc.row_places();
        let header = places
            .iter()
            .find(|p| p.shape == RowShape::SectionHeader(1))
            .unwrap();
        let x = osc.position.x + 5.0;
        let top = osc.position.y + header.y - SECTION_HEIGHT / 2.0;
        assert_eq!(osc.section_header_at(Point::new(x, top + 2.0)), Some(1));
        assert_eq!(
            osc.section_header_at(Point::new(x, top + SECTION_HEIGHT + 2.0)),
            None
        );
    }

    // Collapsed nodes.

    /// A collapsed node is its header over its pins, packed. Every pin is still there.
    #[test]
    fn a_collapsed_node_keeps_every_pin_packed_under_the_header() {
        let mut osc = node(NodeKind::Oscillator);
        let open_inputs = osc.input_ports();
        let open_outputs = osc.output_ports();
        osc.collapsed = true;
        assert_eq!(osc.input_ports(), open_inputs);
        assert_eq!(osc.output_ports(), open_outputs);

        let mut ys = Vec::new();
        for port in &open_inputs {
            let at = osc.input_point(*port).expect("the pin is still there");
            assert_eq!(at.x, osc.position.x);
            assert!(
                osc.bounds().contains(Point::new(at.x + 1.0, at.y)),
                "{port:?}"
            );
            assert!(at.y > osc.position.y + HEADER_HEIGHT, "{port:?}");
            ys.push(at.y);
        }
        // The pins are one packed pitch apart, top to bottom.
        for pair in ys.windows(2) {
            assert_eq!(pair[1] - pair[0], PACKED_PITCH);
        }
        let out = osc.output_point(Port::Audio).unwrap();
        assert_eq!(out.x, osc.position.x + NODE_WIDTH);
    }

    /// A collapsed node is only as tall as its pins need, much shorter than the open node.
    #[test]
    fn a_collapsed_node_is_short() {
        let mut osc = node(NodeKind::Oscillator);
        let open = osc.height();
        osc.collapsed = true;
        let pins = osc.input_ports().len() as f32;
        let expected = HEADER_HEIGHT + 2.0 * COLLAPSED_PADDING + pins * PACKED_PITCH;
        assert!((osc.height() - expected).abs() < 1.0, "{}", osc.height());
        assert!(osc.height() < open / 2.0);
    }

    /// A collapsed node shows no knobs, dropdowns, switches, or section headers.
    #[test]
    fn a_collapsed_node_shows_only_pins() {
        let mut compressor = node(NodeKind::Compressor);
        compressor.collapsed = true;
        assert!(compressor.knob_slots().is_empty());
        assert!(compressor.dropdowns().is_empty());
        assert!(!compressor.switch_shown(0));
        assert!(compressor
            .row_places()
            .iter()
            .all(|p| !matches!(p.shape, RowShape::SectionHeader(_) | RowShape::Full)));
        assert_eq!(compressor.section_header_at(Point::new(110.0, 200.0)), None);
    }

    /// A collapsed node's pins pack from the top, so a wire into one lands inside the node.
    #[test]
    fn expanding_a_collapsed_node_restores_its_layout() {
        let mut osc = node(NodeKind::Oscillator);
        let open = osc.height();
        let pin = osc.input_point(Port::Knob(KnobId::Unison)).unwrap();
        osc.collapsed = true;
        assert_ne!(osc.input_point(Port::Knob(KnobId::Unison)).unwrap(), pin);
        osc.collapsed = false;
        assert_eq!(osc.input_point(Port::Knob(KnobId::Unison)).unwrap(), pin);
        assert_eq!(osc.height(), open);
    }

    // Wires and pins.

    /// A control wire to a knob in a folded section connects, and its end follows the pin when the
    /// section opens.
    #[test]
    fn a_wire_to_a_folded_knob_connects_and_follows_the_pin() {
        let mut graph = minimal_graph();
        let lfo = graph.add_node(NodeKind::Lfo, Point::new(0.0, 400.0));
        graph.connect(lfo, Port::Shape, 0, Port::Knob(KnobId::Fine));
        let wire = *graph
            .connections
            .iter()
            .find(|c| c.input == Port::Knob(KnobId::Fine))
            .expect("the wire is made");
        let folded = graph.nodes[0]
            .input_point(Port::Knob(KnobId::Fine))
            .unwrap();
        assert!(graph.link_curve(&wire).is_some());

        graph.nodes[0].toggle_section(0);
        let open = graph.nodes[0]
            .input_point(Port::Knob(KnobId::Fine))
            .unwrap();
        assert!(open.y > folded.y);
        assert!(graph.link_curve(&wire).is_some());
    }

    /// Where two packed pins are both in reach, the nearer one is picked.
    #[test]
    fn the_nearest_pin_wins_among_packed_pins() {
        let graph = minimal_graph();
        let osc = &graph.nodes[0];
        let coarse = osc.input_point(Port::Knob(KnobId::Coarse)).unwrap();
        let fine = osc.input_point(Port::Knob(KnobId::Fine)).unwrap();
        // Both are inside the hit radius from the point between them, a little nearer to fine.
        let between = Point::new(coarse.x, coarse.y + PACKED_PITCH * 0.6);
        assert_eq!(
            graph.terminal_at(between),
            Some(Terminal::In(0, Port::Knob(KnobId::Fine)))
        );
        let nearer_coarse = Point::new(coarse.x, coarse.y + PACKED_PITCH * 0.3);
        assert_eq!(
            graph.terminal_at(nearer_coarse),
            Some(Terminal::In(0, Port::Knob(KnobId::Coarse)))
        );
        let _ = fine;
    }

    // Saving and copying.

    /// The folds are saved, and a save from before sections loads with each kind's own folds.
    #[test]
    fn the_folds_survive_saving() {
        let mut graph = minimal_graph();
        graph.nodes[0].toggle_section(0);
        graph.nodes[0].toggle_section(1);
        let loaded = Graph::from_saved(&graph.to_saved());
        assert_eq!(loaded.nodes[0].sections_collapsed, vec![false, true, true]);

        let mut json = serde_json::to_value(graph.to_saved()).unwrap();
        for node in json["nodes"].as_array_mut().unwrap() {
            node.as_object_mut().unwrap().remove("sections_collapsed");
        }
        let old: SavedGraph = serde_json::from_value(json).unwrap();
        let loaded = Graph::from_saved(&old);
        assert_eq!(loaded.nodes[0].sections_collapsed, vec![true, false, true]);
    }

    /// Every wire of a saved graph survives whatever is folded, since the pins never go away.
    #[test]
    fn folding_never_drops_a_wire_on_load() {
        let mut graph = minimal_graph();
        let lfo = graph.add_node(NodeKind::Lfo, Point::new(0.0, 400.0));
        graph.connect(lfo, Port::Shape, 0, Port::Knob(KnobId::Coarse));
        graph.connect(lfo, Port::Shape, 0, Port::Knob(KnobId::WarpAmount));
        let wires = graph.connections.len();
        for collapsed in [true, false] {
            graph.nodes[0].collapsed = collapsed;
            let loaded = Graph::from_saved(&graph.to_saved());
            assert_eq!(loaded.connections.len(), wires, "collapsed {collapsed}");
        }
    }

    /// A copy keeps its folds.
    #[test]
    fn a_duplicate_keeps_the_folds() {
        let mut graph = minimal_graph();
        graph.nodes[0].toggle_section(2);
        let copy = graph.duplicate_node(0).unwrap();
        assert_eq!(
            graph.nodes[copy].sections_collapsed,
            graph.nodes[0].sections_collapsed
        );
    }
}

#[cfg(test)]
mod preset_ui_tests {
    use super::*;

    fn drag(index: usize, value: f32) -> Vec<MacroEdit> {
        vec![
            MacroEdit::Begin(index),
            MacroEdit::Set(index, value),
            MacroEdit::End(index),
        ]
    }

    /// A load that leaves every macro where the host has it writes nothing to the host.
    #[test]
    fn unchanged_macros_are_not_written_to_the_host() {
        let edits: Vec<MacroEdit> = (0..4).flat_map(|i| drag(i, 0.0)).collect();
        assert!(without_unchanged_macros(&edits, |_| 0.0).is_empty());
    }

    /// A macro that moves keeps its whole drag, and the others drop out.
    #[test]
    fn a_macro_that_moves_is_still_written() {
        let mut edits = drag(0, 0.0);
        edits.extend(drag(1, 0.45));
        let kept = without_unchanged_macros(&edits, |_| 0.0);
        assert_eq!(kept, drag(1, 0.45));
    }

    /// A drag that has only started has no Set, so it is passed on as it is.
    #[test]
    fn a_drag_with_no_value_yet_is_kept() {
        let edits = vec![MacroEdit::Begin(2)];
        assert_eq!(without_unchanged_macros(&edits, |_| 0.7), edits);
        let edits = vec![MacroEdit::End(2)];
        assert_eq!(without_unchanged_macros(&edits, |_| 0.7), edits);
    }

    /// Each row of the preset list lights on its own line, and a submenu row is not on the list.
    #[test]
    fn a_preset_row_knows_its_line_in_the_list() {
        assert_eq!(PresetRow::SaveAs.list_line(3), Some(0));
        assert_eq!(PresetRow::Init.list_line(3), Some(1));
        assert_eq!(
            PresetRow::Category(2).list_line(3),
            Some(PRESET_ACTION_ROWS + 2)
        );
        assert_eq!(
            PresetRow::Loose(1).list_line(3),
            Some(PRESET_ACTION_ROWS + 3 + 1)
        );
        assert_eq!(PresetRow::Sub(0, 1).list_line(3), None);
    }

    use crate::editor::canvas::{Message, NodeEditor};
    use crate::editor::style::MENU_ITEM_HEIGHT;
    use crate::presets::Library;
    use nih_plug_iced::mouse;
    use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};
    use nih_plug_iced::Vector;
    use std::path::PathBuf;

    fn bounds() -> Rectangle {
        Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        }
    }

    /// A folder for one test's presets, removed when the test ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("flode-ui-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn graph_in(dir: &TempDir) -> Graph {
        let mut graph = minimal_graph();
        graph.preset_dir = Some(dir.0.clone());
        graph
    }

    fn click(editor: &mut NodeEditor, at: Point) -> Option<Message> {
        let press = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let message = editor.update(press, bounds(), Cursor::Available(at)).1;
        let release = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        editor.update(release, bounds(), Cursor::Available(at));
        message
    }

    fn burger_button() -> Point {
        let burger = burger_bounds();
        Point::new(
            burger.x + burger.width / 2.0,
            burger.y + burger.height / 2.0,
        )
    }

    fn presets_row() -> Point {
        let row = burger_row_bounds(BURGER_PRESETS_ROW);
        Point::new(row.x + row.width / 2.0, row.y + row.height / 2.0)
    }

    /// Opens the Presets list the way a person does now: press the burger button, then its
    /// Presets row.
    fn open_presets(editor: &mut NodeEditor) {
        click(editor, burger_button());
        click(editor, presets_row());
    }

    /// What the list shows for the presets in a test's folder.
    fn library_of(dir: &TempDir) -> Library {
        crate::presets::library_in(&dir.0)
    }

    /// The middle of row `row` in the main preset list.
    fn list_row(library: &Library, row: usize) -> Point {
        let list = preset_list_bounds(bounds(), library);
        Point::new(
            list.x + 20.0,
            list.y + (row as f32 + 0.5) * MENU_ITEM_HEIGHT,
        )
    }

    /// The middle of row `row` in the submenu of category `category`.
    fn sub_row(library: &Library, category: usize, row: usize) -> Point {
        let sub = submenu_bounds(bounds(), library, category);
        Point::new(sub.x + 20.0, sub.y + (row as f32 + 0.5) * MENU_ITEM_HEIGHT)
    }

    /// Sends the open dialog's answer, as the worker thread would, and polls for it. Starts a
    /// dialog first if none is open.
    fn answer_dialog(graph: &mut Graph, path: Option<PathBuf>) {
        if graph.save_dialog.is_none() {
            graph.start_save_dialog();
        }
        let sender = graph.save_dialog_sender.clone().expect("a dialog is open");
        sender.send(path).unwrap();
        graph.poll_save_dialog();
    }

    /// Save a preset through the UI: open the list, pick Save as, and answer the dialog with a
    /// file in the category's folder. The editor then saves it, which the test does by hand with
    /// the macro values it is given.
    fn save_through_the_ui(graph: &mut Graph, name: &str, category: &str, macros: [f32; 4]) {
        let dir = graph.preset_dir.clone().unwrap();
        let library = crate::presets::library_in(&dir);
        let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        open_presets(&mut editor);
        click(&mut editor, list_row(&library, 0));
        assert!(editor.graph.save_dialog.is_some());
        answer_dialog(
            graph,
            Some(dir.join(category).join(format!("{name}.flode"))),
        );
        graph.save_pending_preset(macros);
    }

    /// Saving again to the same file replaces that preset, and the dialog starts with the
    /// current preset's name and folder.
    #[test]
    fn saving_again_replaces_the_file() {
        let dir = TempDir::new("again");
        let mut graph = graph_in(&dir);
        save_through_the_ui(&mut graph, "Pluck", "Lead", [0.0; 4]);
        assert_eq!(
            save_dialog_start(&dir.0, &graph.preset_category, graph.preset_name.as_deref()),
            (dir.0.join("Lead"), "Pluck.flode".to_string())
        );
        save_through_the_ui(&mut graph, "Pluck", "Lead", [0.9; 4]);
        let library = library_of(&dir);
        assert_eq!(library.categories.len(), 1);
        assert_eq!(library.categories[0].presets, ["Pluck"]);
        assert_eq!(
            crate::presets::load_in(&dir.0, "Lead", "Pluck")
                .unwrap()
                .macros,
            [0.9; 4]
        );
    }

    /// Loading a preset swaps the graph and sends its macro values to the host.
    #[test]
    fn loading_a_preset_swaps_the_graph_and_sets_the_macros() {
        let dir = TempDir::new("load");
        let mut graph = graph_in(&dir);
        graph.add_node(NodeKind::Reverb, Point::new(400.0, 300.0));
        save_through_the_ui(&mut graph, "Wet", "Pad", [0.5, 0.6, 0.7, 0.8]);
        let library = library_of(&dir);

        let mut other = graph_in(&dir);
        other.delete_node(0);
        assert_eq!(other.nodes.len(), 2);
        let mut editor = NodeEditor::new(&mut other, ControlLevels::default(), [0; 2], [0.0; 2]);
        // The list shows the category. Clicking it opens its submenu, and the preset is in that.
        open_presets(&mut editor);
        assert_eq!(
            editor.graph.preset_list.as_ref().map(|m| m.library.clone()),
            Some(library.clone())
        );
        click(&mut editor, list_row(&library, 2));
        assert_eq!(editor.graph.preset_list.as_ref().unwrap().open, Some(0));
        let message = click(&mut editor, sub_row(&library, 0, 0));
        assert!(matches!(message, Some(Message::GraphChanged)));

        assert!(editor.graph.preset_list.is_none());
        assert_eq!(editor.graph.preset_category, "Pad");
        assert_eq!(editor.graph.nodes.len(), 4);
        assert!(editor
            .graph
            .nodes
            .iter()
            .any(|n| n.kind == NodeKind::Reverb));
        assert_eq!(editor.graph.preset_name.as_deref(), Some("Wet"));
        assert!(editor.graph.patch().lanes[0].is_active());
        // The host hears each macro's gesture and value.
        let sets: Vec<_> = editor
            .graph
            .macro_edits
            .iter()
            .filter_map(|edit| match edit {
                MacroEdit::Set(i, v) => Some((*i, *v)),
                _ => None,
            })
            .collect();
        assert_eq!(sets, [(0, 0.5), (1, 0.6), (2, 0.7), (3, 0.8)]);
    }

    /// An unreadable preset leaves the graph alone and says so.
    #[test]
    fn a_missing_preset_changes_nothing() {
        let dir = TempDir::new("missing");
        let mut graph = graph_in(&dir);
        let count = graph.nodes.len();
        assert!(!graph.load_preset("Pad", "ghost"));
        assert_eq!(graph.nodes.len(), count);
        assert!(graph
            .status
            .as_ref()
            .is_some_and(|(text, _)| text.contains("ghost")));
    }

    /// Save as... asks for a dialog, and a second click while it is open asks for nothing more.
    #[test]
    fn save_as_asks_for_one_dialog_at_a_time() {
        let dir = TempDir::new("once");
        let mut graph = graph_in(&dir);
        let library = Library::default();
        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        open_presets(&mut editor);
        click(&mut editor, list_row(&library, 0));
        assert_eq!(editor.graph.save_dialog_requests, 1);
        assert!(editor.graph.save_dialog.is_some() && editor.graph.preset_list.is_none());
        open_presets(&mut editor);
        click(&mut editor, list_row(&library, 0));
        assert_eq!(editor.graph.save_dialog_requests, 1);
        // Nothing is saved until the dialog answers.
        assert!(editor.graph.pending_save.is_none());
        // The folder the dialog starts in exists.
        assert!(dir.0.is_dir());
    }

    /// The dialog starts in the preset folder, or the current category's folder in it, and
    /// suggests the current name.
    #[test]
    fn the_dialog_starts_where_the_preset_lives() {
        let dir = std::path::Path::new("/presets");
        assert_eq!(
            save_dialog_start(dir, "Bass", Some("Reese")),
            (PathBuf::from("/presets/Bass"), "Reese.flode".to_string())
        );
        assert_eq!(
            save_dialog_start(dir, "", Some("Plain")),
            (PathBuf::from("/presets"), "Plain.flode".to_string())
        );
        assert_eq!(
            save_dialog_start(dir, "", None),
            (PathBuf::from("/presets"), "Untitled.flode".to_string())
        );
    }

    /// A category folder is made before the dialog opens, so the panel can start in it.
    #[test]
    fn the_current_category_folder_is_made_for_the_dialog() {
        let dir = TempDir::new("start-folder");
        let mut graph = graph_in(&dir);
        graph.preset_category = "Lead".to_string();
        graph.start_save_dialog();
        assert!(dir.0.join("Lead").is_dir());
    }

    #[test]
    fn save_as_without_a_preset_folder_says_so() {
        let mut graph = minimal_graph();
        graph.preset_dir = None;
        graph.start_save_dialog();
        assert_eq!(graph.save_dialog_requests, 0);
        assert!(graph
            .status
            .as_ref()
            .is_some_and(|(text, _)| text == "Could not save: no preset folder"));
    }

    /// A path the dialog returns is saved with the macro values, and names the preset.
    #[test]
    fn a_chosen_path_is_saved_with_the_macros() {
        let dir = TempDir::new("chosen");
        let mut graph = graph_in(&dir);
        answer_dialog(&mut graph, Some(dir.0.join("Pad").join("Warm pad.flode")));
        assert!(graph.pending_save.is_some());
        graph.save_pending_preset([0.1, 0.2, 0.3, 0.4]);

        let saved = crate::presets::load_in(&dir.0, "Pad", "Warm pad").unwrap();
        assert_eq!(saved.macros, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(graph.preset_name.as_deref(), Some("Warm pad"));
        assert_eq!(graph.preset_category, "Pad");
        assert!(graph.pending_save.is_none() && graph.save_dialog.is_none());
        assert!(graph
            .status
            .as_ref()
            .is_some_and(|(text, _)| text == "Saved Warm pad"));
        // The name and category stay with the project's saved graph.
        let reloaded = Graph::from_saved(&graph.to_saved());
        assert_eq!(reloaded.preset_name.as_deref(), Some("Warm pad"));
        assert_eq!(reloaded.preset_category, "Pad");
    }

    /// A file outside the preset folder is written where the user chose, with no category.
    #[test]
    fn a_path_outside_the_preset_folder_has_no_category() {
        let dir = TempDir::new("inside");
        let elsewhere = TempDir::new("outside");
        let mut graph = graph_in(&dir);
        graph.preset_category = "Bass".to_string();
        answer_dialog(&mut graph, Some(elsewhere.0.join("Far.flode")));
        graph.save_pending_preset([0.0; 4]);
        assert!(elsewhere.0.join("Far.flode").is_file());
        assert_eq!(graph.preset_name.as_deref(), Some("Far"));
        assert_eq!(graph.preset_category, "");
        assert!(library_of(&dir).is_empty());
    }

    /// A deeper folder inside the preset folder is written there and has no category either.
    #[test]
    fn a_path_two_folders_down_has_no_category() {
        let dir = TempDir::new("deep");
        let mut graph = graph_in(&dir);
        answer_dialog(
            &mut graph,
            Some(dir.0.join("A").join("B").join("Deep.flode")),
        );
        graph.save_pending_preset([0.0; 4]);
        assert!(dir.0.join("A").join("B").join("Deep.flode").is_file());
        assert_eq!(graph.preset_category, "");
    }

    /// A name with no extension, or another one, gets `.flode` on the end of the whole name.
    #[test]
    fn a_chosen_name_always_ends_in_flode() {
        let dir = TempDir::new("extension");
        let mut graph = graph_in(&dir);
        for (typed, file, name) in [
            ("Bare", "Bare.flode", "Bare"),
            ("Text.txt", "Text.txt.flode", "Text.txt"),
            ("My.Pad", "My.Pad.flode", "My.Pad"),
            ("Right.flode", "Right.flode", "Right"),
        ] {
            answer_dialog(&mut graph, Some(dir.0.join(typed)));
            graph.save_pending_preset([0.0; 4]);
            assert!(dir.0.join(file).is_file(), "{typed}");
            assert_eq!(graph.preset_name.as_deref(), Some(name));
        }
        assert!(!dir.0.join("Bare").exists());
    }

    /// A file that landed in the preset folder shows in the list and loads back through the
    /// normal flow.
    #[test]
    fn a_saved_file_in_the_preset_folder_loads_back() {
        let dir = TempDir::new("loads-back");
        let mut graph = graph_in(&dir);
        graph.add_node(NodeKind::Reverb, Point::new(400.0, 300.0));
        answer_dialog(&mut graph, Some(dir.0.join("Pad").join("Wet")));
        graph.save_pending_preset([0.5, 0.6, 0.7, 0.8]);
        answer_dialog(&mut graph, Some(dir.0.join("Loose")));
        graph.save_pending_preset([0.0; 4]);
        let library = library_of(&dir);
        assert_eq!(library.categories[0].name, "Pad");
        assert_eq!(library.categories[0].presets, ["Wet"]);
        assert_eq!(library.loose, ["Loose"]);

        let mut other = graph_in(&dir);
        assert!(other.load_preset("Pad", "Wet"));
        assert!(other.nodes.iter().any(|n| n.kind == NodeKind::Reverb));
        assert_eq!(other.preset_category, "Pad");
        let sets: Vec<f32> = other
            .macro_edits
            .iter()
            .filter_map(|edit| match edit {
                MacroEdit::Set(_, v) => Some(*v),
                _ => None,
            })
            .collect();
        assert_eq!(sets, [0.5, 0.6, 0.7, 0.8]);
    }

    /// Canceling the dialog saves nothing and says nothing, and a new dialog can open.
    #[test]
    fn a_canceled_dialog_does_nothing() {
        let dir = TempDir::new("cancel");
        let mut graph = graph_in(&dir);
        answer_dialog(&mut graph, None);
        assert!(graph.pending_save.is_none() && graph.save_dialog.is_none());
        assert!(graph.status.is_none());
        graph.start_save_dialog();
        assert_eq!(graph.save_dialog_requests, 2);
    }

    /// While the dialog has not answered, polling changes nothing.
    #[test]
    fn polling_an_open_dialog_waits() {
        let dir = TempDir::new("waiting");
        let mut graph = graph_in(&dir);
        graph.start_save_dialog();
        graph.poll_save_dialog();
        assert!(graph.save_dialog.is_some() && graph.pending_save.is_none());
        assert!(graph.status.is_none());
    }

    /// A dialog that dies without an answer is reported, and the editor carries on.
    #[test]
    fn a_dialog_that_died_shows_the_status_line() {
        let dir = TempDir::new("died");
        let mut graph = graph_in(&dir);
        graph.start_save_dialog();
        drop(graph.save_dialog_sender.take());
        graph.poll_save_dialog();
        assert!(graph.save_dialog.is_none() && graph.pending_save.is_none());
        assert!(graph
            .status
            .as_ref()
            .is_some_and(|(text, _)| text == "Could not open the save dialog"));
        graph.start_save_dialog();
        assert!(graph.save_dialog.is_some());
    }

    /// A load while the dialog is open keeps waiting for its answer.
    #[test]
    fn a_load_keeps_the_open_dialog() {
        let dir = TempDir::new("keep-dialog");
        let mut graph = graph_in(&dir);
        crate::presets::save_in(
            &dir.0,
            &crate::presets::Preset {
                name: "Wet".to_string(),
                category: String::new(),
                graph: graph.to_saved(),
                macros: [0.0; 4],
            },
        )
        .unwrap();
        graph.start_save_dialog();
        assert!(graph.load_preset("", "Wet"));
        assert!(graph.save_dialog.is_some());
        graph.start_save_dialog();
        assert_eq!(graph.save_dialog_requests, 1);
    }

    /// The Presets row opens the list, and reopening the burger menu and pressing it again, or
    /// pressing elsewhere, closes it.
    #[test]
    fn the_presets_row_toggles_the_list() {
        let dir = TempDir::new("toggle");
        let mut graph = graph_in(&dir);
        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        open_presets(&mut editor);
        assert!(editor.graph.preset_list.is_some());
        open_presets(&mut editor);
        assert!(editor.graph.preset_list.is_none());
        open_presets(&mut editor);
        click(&mut editor, Point::new(400.0, 400.0));
        assert!(editor.graph.preset_list.is_none());
    }

    /// A library of two categories and two loose presets, as the list shows it.
    fn sample_library() -> Library {
        use crate::presets::Category;
        let category = |name: &str, presets: &[&str]| Category {
            name: name.to_string(),
            presets: presets.iter().map(|p| p.to_string()).collect(),
        };
        Library {
            categories: vec![
                category("Bass", &["Acid", "Sub", "Wob"]),
                category("Pad", &["Warm"]),
            ],
            loose: vec!["One".to_string(), "Two".to_string()],
        }
    }

    /// Each row of the list is an action: save, a category, or a loose preset. The row under an
    /// empty list's save entry is a note, not an action.
    #[test]
    fn rows_map_to_actions() {
        let library = sample_library();
        let menu = PresetMenu {
            library: library.clone(),
            open: None,
        };
        let at = |row: usize| preset_row_at(bounds(), &menu, list_row(&library, row));
        assert_eq!(at(0), Some(PresetRow::SaveAs));
        assert_eq!(at(1), Some(PresetRow::Init));
        assert_eq!(at(2), Some(PresetRow::Category(0)));
        assert_eq!(at(3), Some(PresetRow::Category(1)));
        assert_eq!(at(4), Some(PresetRow::Loose(0)));
        assert_eq!(at(5), Some(PresetRow::Loose(1)));
        assert_eq!(at(6), None);

        // With none saved, the two actions are still there, and the row under them is a note.
        let empty = PresetMenu {
            library: Library::default(),
            open: None,
        };
        let at_empty = |row: usize| preset_row_at(bounds(), &empty, list_row(&empty.library, row));
        assert_eq!(at_empty(0), Some(PresetRow::SaveAs));
        assert_eq!(at_empty(1), Some(PresetRow::Init));
        assert_eq!(at_empty(2), None);
        assert_eq!(shorten("Short"), "Short");
        assert_eq!(shorten("A very long preset name").chars().count(), 10);
    }

    /// An open submenu has a row for each preset in its category. It sits beside the list, level
    /// with its category, and does not overlap the list or run into the keys.
    #[test]
    fn the_submenu_holds_its_categorys_presets() {
        let library = sample_library();
        let menu = PresetMenu {
            library: library.clone(),
            open: Some(0),
        };
        let at = |row: usize| preset_row_at(bounds(), &menu, sub_row(&library, 0, row));
        assert_eq!(at(0), Some(PresetRow::Sub(0, 0)));
        assert_eq!(at(2), Some(PresetRow::Sub(0, 2)));
        assert_eq!(at(3), None);

        let list = preset_list_bounds(bounds(), &library);
        for category in 0..2 {
            let sub = submenu_bounds(bounds(), &library, category);
            let apart = sub.x >= list.x + list.width || sub.x + sub.width <= list.x;
            assert!(apart, "{category}");
            assert!(sub.y >= list.y);
            assert!(sub.y + sub.height <= bounds().height - crate::editor::piano::STRIP_HEIGHT);
            assert!(sub.x >= 0.0 && sub.x + sub.width <= bounds().width);
        }
        // The Bass submenu starts level with the Bass row, under the two action rows.
        let sub = submenu_bounds(bounds(), &library, 0);
        assert_eq!(sub.y, list.y + 2.0 * MENU_ITEM_HEIGHT);
        // With no submenu open, a point where it would be is not a row.
        let closed = PresetMenu {
            library: library.clone(),
            open: None,
        };
        assert_eq!(
            preset_row_at(bounds(), &closed, sub_row(&library, 0, 0)),
            None
        );
    }

    /// A very long list stops above the keys, in the list and in a submenu.
    #[test]
    fn long_lists_stop_above_the_keys() {
        use crate::presets::Category;
        let many = |prefix: &str| (0..100).map(|n| format!("{prefix}{n}")).collect::<Vec<_>>();
        let library = Library {
            categories: vec![Category {
                name: "All".to_string(),
                presets: many("p"),
            }],
            loose: many("l"),
        };
        let keys = bounds().height - crate::editor::piano::STRIP_HEIGHT;
        let list = preset_list_bounds(bounds(), &library);
        assert!(list.y + list.height <= keys);
        let sub = submenu_bounds(bounds(), &library, 0);
        assert!(sub.y + sub.height <= keys);
        let (categories, loose) = preset_rows_shown(bounds(), &library);
        assert_eq!(categories, 1);
        assert!(loose > 0 && loose < 100);
    }

    /// Moving the cursor over a category opens its submenu, onto another row closes it, and
    /// crossing the gap to the submenu leaves it open.
    #[test]
    fn hovering_a_category_opens_its_submenu() {
        let dir = TempDir::new("hover");
        let mut graph = graph_in(&dir);
        let mut seed = graph_in(&dir);
        save_through_the_ui(&mut seed, "Sub", "Bass", [0.0; 4]);
        save_through_the_ui(&mut seed, "Plain", "", [0.0; 4]);
        let library = library_of(&dir);
        assert_eq!(library.categories.len(), 1);

        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        open_presets(&mut editor);
        let move_to = |editor: &mut NodeEditor, at: Point| {
            editor.update(
                Event::Mouse(mouse::Event::CursorMoved { position: at }),
                bounds(),
                Cursor::Available(at),
            )
        };
        let open = |editor: &NodeEditor| editor.graph.preset_list.as_ref().unwrap().open;

        move_to(&mut editor, list_row(&library, 2));
        assert_eq!(open(&editor), Some(0));
        // Off the list and onto the submenu, it stays.
        move_to(&mut editor, sub_row(&library, 0, 0));
        assert_eq!(open(&editor), Some(0));
        move_to(&mut editor, Point::new(5.0, 400.0));
        assert_eq!(open(&editor), Some(0));
        // Onto the loose preset's row, it closes.
        move_to(&mut editor, list_row(&library, 3));
        assert_eq!(open(&editor), None);

        // A click on a category row opens it, and a second click closes it.
        click(&mut editor, list_row(&library, 2));
        assert_eq!(open(&editor), Some(0));
        click(&mut editor, list_row(&library, 2));
        assert_eq!(open(&editor), None);
        assert!(editor.graph.preset_list.is_some());
    }

    /// The Init row puts the starting graph back, drops what was on the canvas, forgets the current
    /// preset, sets the macros back, and keeps the view and the preset folder.
    #[test]
    fn init_goes_back_to_the_default_patch() {
        let dir = TempDir::new("init");
        let mut seed = graph_in(&dir);
        save_through_the_ui(&mut seed, "Keeper", "Bass", [0.2; 4]);

        // A canvas that has wandered off: a preset loaded, a node added, the view moved.
        let mut graph = graph_in(&dir);
        graph.load_preset("Bass", "Keeper");
        graph.add_node(NodeKind::Reverb, Point::new(500.0, 400.0));
        graph.add_node(NodeKind::Delay, Point::new(700.0, 400.0));
        graph.view.offset = Vector::new(40.0, 25.0);
        graph.macro_edits.clear();
        let library = library_of(&dir);

        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        open_presets(&mut editor);
        let message = click(&mut editor, list_row(&library, 1));
        assert!(matches!(message, Some(Message::GraphChanged)));

        let default = Graph::default();
        assert!(editor.graph.preset_list.is_none());
        assert_eq!(editor.graph.nodes.len(), default.nodes.len());
        assert!(editor.graph.patch() == default.patch());
        assert_eq!(editor.graph.preset_name, None);
        assert_eq!(editor.graph.preset_category, "");
        assert_eq!(editor.graph.view.offset, Vector::new(40.0, 25.0));
        assert_eq!(editor.graph.preset_dir.as_deref(), Some(dir.0.as_path()));
        // The host is told to put every macro back to its starting value.
        let sets: Vec<_> = editor
            .graph
            .macro_edits
            .iter()
            .filter_map(|edit| match edit {
                MacroEdit::Set(i, v) => Some((*i, *v)),
                _ => None,
            })
            .collect();
        assert_eq!(sets, [(0, 0.0), (1, 0.0), (2, 0.0), (3, 0.0)]);
        // The next save suggests a blank name.
        assert_eq!(editor.graph.preset_name, None);
        // And the preset on disk is untouched.
        assert!(crate::presets::load_in(&dir.0, "Bass", "Keeper").is_ok());
    }

    /// A loose preset loads from the main list.
    #[test]
    fn a_loose_preset_loads_from_the_main_list() {
        let dir = TempDir::new("loose-load");
        let mut seed = graph_in(&dir);
        seed.add_node(NodeKind::Delay, Point::new(400.0, 300.0));
        save_through_the_ui(&mut seed, "Plain", "", [0.0; 4]);
        let library = library_of(&dir);

        let mut graph = graph_in(&dir);
        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        open_presets(&mut editor);
        click(&mut editor, list_row(&library, 2));
        assert!(editor.graph.nodes.iter().any(|n| n.kind == NodeKind::Delay));
        assert_eq!(editor.graph.preset_name.as_deref(), Some("Plain"));
        assert_eq!(editor.graph.preset_category, "");
    }
}

#[cfg(test)]
mod wire_curve_tests {
    use super::*;

    /// The curve's x along a wire, sampled from end to end.
    fn xs(curve: &[Point; 4]) -> Vec<f32> {
        (0..=40)
            .map(|i| bezier_point(curve, i as f32 / 40.0).x)
            .collect()
    }

    /// A short wire to a pin just ahead of it runs forward without doubling back, so it has no
    /// kink.
    #[test]
    fn a_short_wire_does_not_double_back() {
        for (dx, dy) in [(20.0, 10.0), (30.0, -25.0), (12.0, 0.0), (40.0, 40.0)] {
            let from = Point::new(100.0, 100.0);
            let curve = wire_curve(from, Point::new(from.x + dx, from.y + dy));
            let xs = xs(&curve);
            assert!(
                xs.windows(2).all(|pair| pair[1] >= pair[0] - 1e-3),
                "{dx} {dy}"
            );
            assert!(xs
                .iter()
                .all(|&x| x >= from.x - 1e-3 && x <= from.x + dx + 1e-3));
        }
    }

    /// A long wire keeps its full 60 px handles, and a wire straight down still leaves level.
    #[test]
    fn a_long_wire_keeps_its_reach() {
        let curve = wire_curve(Point::new(0.0, 0.0), Point::new(400.0, 100.0));
        assert_eq!(curve[1], Point::new(200.0, 0.0));
        assert_eq!(curve[2], Point::new(200.0, 100.0));
        let curve = wire_curve(Point::new(0.0, 0.0), Point::new(0.0, 300.0));
        assert_eq!(curve[1], Point::new(60.0, 0.0));
        assert_eq!(curve[2], Point::new(-60.0, 300.0));
    }
}

#[cfg(test)]
mod notes_tests {
    use super::*;

    /// A save from before the MIDI input node: the same graph, written without a version.
    fn old_save(graph: &Graph) -> SavedGraph {
        let mut json = serde_json::to_value(graph.to_saved()).unwrap();
        json.as_object_mut().unwrap().remove("version");
        serde_json::from_value(json).unwrap()
    }

    fn has_notes_wire(graph: &Graph, to: usize) -> bool {
        graph
            .connections
            .iter()
            .any(|c| c.out == Port::Notes && c.to == to && c.input == Port::Notes)
    }

    /// An oscillator plays the notes wired into it, so a notes wire only joins a MIDI input to an
    /// oscillator.
    #[test]
    fn a_notes_wire_joins_an_input_to_an_oscillator() {
        let mut graph = minimal_graph();
        let filter = graph.add_node(NodeKind::Filter, Point::new(0.0, 300.0));
        let wires = graph.connections.len();
        graph.connect(2, Port::Notes, filter, Port::Audio);
        graph.connect(2, Port::Notes, filter, Port::Knob(KnobId::Cutoff));
        assert_eq!(graph.connections.len(), wires);
        // Audio does not feed a notes pin either.
        graph.connect(filter, Port::Audio, 0, Port::Notes);
        assert_eq!(graph.connections.len(), wires);
        assert!(has_notes_wire(&graph, 0));
    }

    #[test]
    fn an_oscillator_with_no_notes_wire_is_silent_and_stays_so_after_saving() {
        let mut graph = minimal_graph();
        graph.disconnect_input(0, Port::Notes);
        assert!(!graph.patch().lanes[0].is_active());
        let loaded = Graph::from_saved(&graph.to_saved());
        assert!(!has_notes_wire(&loaded, 0));
        assert!(!loaded.patch().lanes[0].is_active());
    }

    #[test]
    fn the_notes_wire_survives_saving() {
        let graph = minimal_graph();
        let loaded = Graph::from_saved(&graph.to_saved());
        assert_eq!(loaded.connections, graph.connections);
        assert!(loaded.patch().lanes[0].is_active());
    }

    /// A save with no MIDI input gets one, wired to the oscillator, and it takes the glide and
    /// mono settings the output node held, so the patch sounds the same.
    #[test]
    fn an_old_save_gets_a_midi_input_with_its_voice_settings() {
        let mut graph = minimal_graph();
        graph.delete_node(2);
        graph.nodes[1].set_switch(0, true);
        graph.nodes[1].set_switch(1, true);
        graph.nodes[1].set_value(KnobId::Glide, KnobId::Glide.to_normalized(0.4));

        let loaded = Graph::from_saved(&old_save(&graph));
        let input = loaded
            .nodes
            .iter()
            .position(|n| n.kind == NodeKind::Input)
            .expect("an input is added");
        assert!(has_notes_wire(&loaded, 0));
        assert!(loaded.nodes[input].position.x < loaded.nodes[0].position.x);
        let patch = loaded.patch();
        assert!(patch.lanes[0].is_active());
        assert!(patch.globals.mono && patch.globals.glide_always);
        let glide = patch.globals.glide.plain(&ControlLevels::default());
        assert!((glide - 0.4).abs() < 1e-3);
    }

    /// A save that already has an input uses it, and does not add another.
    #[test]
    fn an_old_save_with_an_input_reuses_it() {
        let mut graph = minimal_graph();
        graph.disconnect_input(0, Port::Notes);
        let loaded = Graph::from_saved(&old_save(&graph));
        assert_eq!(loaded.nodes.len(), 3);
        assert!(has_notes_wire(&loaded, 0));
    }

    /// The upgrade runs once. A saved version 1 graph with an unwired oscillator stays unwired.
    #[test]
    fn the_upgrade_does_not_run_on_a_current_save() {
        let mut graph = minimal_graph();
        graph.disconnect_input(0, Port::Notes);
        let loaded = Graph::from_saved(&graph.to_saved());
        assert_eq!(loaded.nodes.len(), 3);
        assert!(!has_notes_wire(&loaded, 0));
    }

    /// The Octave knob reaches four octaves each way.
    #[test]
    fn the_octave_knob_spans_four_octaves() {
        assert_eq!(KnobId::Octave.to_plain(0.0), -4.0);
        assert_eq!(KnobId::Octave.to_plain(1.0), 4.0);
        assert_eq!(KnobId::Octave.to_plain(0.5), 0.0);
    }

    /// A version 1 save wrote the Octave knob over -2 to 2. Loading it keeps each oscillator's
    /// pitch.
    #[test]
    fn an_older_save_keeps_its_octave() {
        let mut graph = minimal_graph();
        let mut saved = graph.to_saved();
        saved.version = 1;
        for node in &mut saved.nodes {
            for knob in &mut node.knobs {
                if knob.0 == KnobId::Octave {
                    // Old +1 octave, which sat at 0.75 on the old -2 to 2 span.
                    knob.1 = 0.75;
                }
            }
        }
        graph = Graph::from_saved(&saved);
        assert_eq!(graph.nodes[0].plain(KnobId::Octave), 1.0);
    }

    /// A current save already has the new span, so loading it changes nothing.
    #[test]
    fn a_current_save_keeps_its_octave() {
        let mut graph = minimal_graph();
        graph.nodes[0].set_value(KnobId::Octave, KnobId::Octave.to_normalized(3.0));
        let loaded = Graph::from_saved(&graph.to_saved());
        assert_eq!(loaded.nodes[0].plain(KnobId::Octave), 3.0);
    }

    /// A pre-upgrade save with an oscillator wired to a mix, two oscillators in all, wires both.
    #[test]
    fn the_upgrade_wires_every_oscillator() {
        let mut graph = minimal_graph();
        let second = add_oscillator(&mut graph, Point::new(0.0, 300.0));
        graph.connect(second, Port::Audio, 1, Port::Slot(1));
        graph.connections.retain(|c| c.out != Port::Notes);
        let loaded = Graph::from_saved(&old_save(&graph));
        assert!(has_notes_wire(&loaded, 0));
        assert!(has_notes_wire(&loaded, second));
        assert!(loaded.patch().lanes[1].is_active());
    }
}

#[cfg(test)]
mod macro_tests {
    use super::*;
    use crate::editor::canvas::NodeEditor;
    use nih_plug_iced::mouse;
    use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};

    fn bounds() -> Rectangle {
        Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        }
    }

    /// The minimal graph with a macro node at the far right, and the node's index.
    fn graph_with_macros() -> (Graph, usize) {
        let mut graph = minimal_graph();
        let node = graph.add_node(NodeKind::Macro, Point::new(600.0, 100.0));
        (graph, node)
    }

    fn mouse_event(editor: &mut NodeEditor, event: mouse::Event, at: Point) {
        editor.update(Event::Mouse(event), bounds(), Cursor::Available(at));
    }

    /// Pressing, dragging, and releasing a macro knob tells the host: the gesture starts, the
    /// value follows the mouse, and the gesture ends.
    #[test]
    fn dragging_a_macro_knob_tells_the_host() {
        let (mut graph, node) = graph_with_macros();
        let center = graph.nodes[node].knob_slots()[1].center;
        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);

        mouse_event(
            &mut editor,
            mouse::Event::ButtonPressed(mouse::Button::Left),
            center,
        );
        // Dragging up 50 px raises the knob a quarter of its range.
        let up = Point::new(center.x, center.y - 50.0);
        mouse_event(&mut editor, mouse::Event::CursorMoved { position: up }, up);
        mouse_event(
            &mut editor,
            mouse::Event::ButtonReleased(mouse::Button::Left),
            up,
        );

        let edits = &editor.graph.macro_edits;
        assert_eq!(edits.first(), Some(&MacroEdit::Begin(1)));
        assert_eq!(edits.last(), Some(&MacroEdit::End(1)));
        let value = edits
            .iter()
            .rev()
            .find_map(|edit| match edit {
                MacroEdit::Set(1, value) => Some(*value),
                _ => None,
            })
            .expect("the drag sets the value");
        assert!((value - 0.25).abs() < 1e-3, "{value}");
        assert!(edits
            .iter()
            .all(|edit| !matches!(edit, MacroEdit::Begin(i) | MacroEdit::Set(i, _) | MacroEdit::End(i) if *i != 1)));
    }

    /// A knob on any other node never tells the host anything.
    #[test]
    fn other_knobs_are_not_host_parameters() {
        let (mut graph, _) = graph_with_macros();
        let center = graph.nodes[0].knob_slots()[0].center;
        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        mouse_event(
            &mut editor,
            mouse::Event::ButtonPressed(mouse::Button::Left),
            center,
        );
        let up = Point::new(center.x, center.y - 30.0);
        mouse_event(&mut editor, mouse::Event::CursorMoved { position: up }, up);
        mouse_event(
            &mut editor,
            mouse::Event::ButtonReleased(mouse::Button::Left),
            up,
        );
        assert!(editor.graph.macro_edits.is_empty());
    }

    /// The host's values show on the macro node's knobs, and a knob under the mouse is left alone.
    #[test]
    fn the_hosts_values_show_on_the_knobs() {
        let (mut graph, node) = graph_with_macros();
        graph.show_macros([0.1, 0.2, 0.3, 0.4]);
        for (index, id) in KnobId::MACROS.iter().enumerate() {
            let expected = 0.1 * (index + 1) as f32;
            assert!((graph.nodes[node].value(*id) - expected).abs() < 1e-6);
        }

        graph.interaction = Interaction::Knob {
            node,
            id: KnobId::Macro2,
        };
        graph.show_macros([0.9; 4]);
        assert!((graph.nodes[node].value(KnobId::Macro1) - 0.9).abs() < 1e-6);
        assert!((graph.nodes[node].value(KnobId::Macro2) - 0.2).abs() < 1e-6);
    }

    /// A macro wired to a knob drives it from the live level, and the patch carries the route.
    #[test]
    fn a_macro_drives_a_knob_in_the_patch() {
        let (mut graph, node) = graph_with_macros();
        let filter = graph.add_node(NodeKind::Filter, Point::new(300.0, 300.0));
        graph.connect(node, Port::Macro(3), filter, Port::Knob(KnobId::Cutoff));
        let route = graph.control_route(filter, KnobId::Cutoff).unwrap();
        assert_eq!(route.source, Source::Macro(3));
        assert!(graph.is_driven(filter, KnobId::Cutoff));

        // The wire and the node survive a save.
        let loaded = Graph::from_saved(&graph.to_saved());
        assert_eq!(loaded.connections, graph.connections);
    }
}

#[cfg(test)]
mod parallel_effects_tests {
    use super::*;
    use crate::editor::canvas::{Message, NodeEditor};
    use crate::editor::style::WAVEFORM_ITEM_HEIGHT;
    use crate::nodes::mix::FM_ROW;
    use nih_plug_iced::mouse;
    use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};

    /// The default window, which the canvas events are given.
    const BOUNDS: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 1024.0,
        height: 712.0,
    };

    /// An oscillator, the output, and a MIDI input, with nothing wired to the output yet. The
    /// oscillator is node 0 and the output is node 1.
    fn bare() -> Graph {
        let mut graph = minimal_graph();
        graph.connections.retain(|c| c.out != Port::Audio);
        graph
    }

    /// Oscillator 0 into two effect chains, a reverb and a delay, that a mix adds, into the
    /// output. Returns the graph and the mix's index.
    fn parallel() -> (Graph, usize) {
        let mut graph = bare();
        let reverb = graph.add_node(NodeKind::Reverb, Point::new(300.0, 100.0));
        let delay = graph.add_node(NodeKind::Delay, Point::new(300.0, 300.0));
        let mix = graph.add_node(NodeKind::Mix, Point::new(600.0, 200.0));
        graph.connect(0, Port::Audio, reverb, Port::Audio);
        graph.connect(0, Port::Audio, delay, Port::Audio);
        graph.connect(reverb, Port::Audio, mix, Port::AudioIn(0));
        graph.connect(delay, Port::Audio, mix, Port::AudioIn(1));
        graph.connect(mix, Port::Audio, 1, Port::Slot(0));
        graph.refresh_slots();
        (graph, mix)
    }

    /// An effect's output can go to a filter or a mix, which the old rule did not allow.
    #[test]
    fn an_effect_can_feed_a_filter_and_a_mix() {
        let mut graph = bare();
        let reverb = graph.add_node(NodeKind::Reverb, Point::new(300.0, 100.0));
        let filter = graph.add_node(NodeKind::Filter, Point::new(500.0, 100.0));
        let mix = graph.add_node(NodeKind::Mix, Point::new(500.0, 300.0));
        graph.connect(reverb, Port::Audio, filter, Port::Audio);
        graph.connect(reverb, Port::Audio, mix, Port::AudioIn(0));
        let wires = |to| graph.connections.iter().filter(|c| c.to == to).count();
        assert_eq!(wires(filter), 1);
        assert_eq!(wires(mix), 1);
    }

    /// Two effect chains from one oscillator meet in a mix, and the slot plays that mix. The
    /// oscillator is placed once, and the other three stages are on the bus.
    #[test]
    fn two_effect_chains_play_through_a_mix() {
        let (graph, mix) = parallel();
        assert!(graph
            .connections
            .iter()
            .any(|c| c.to == mix && c.input == Port::AudioIn(1)));
        let lane = graph.patch().lanes[0].clone();
        let bus: Vec<bool> = lane.stages.iter().map(|s| s.bus).collect();
        assert_eq!(bus, [false, true, true, true]);
        assert!(lane.stages[0].tap);
        assert!(matches!(lane.stages[3].params, StageParams::Mix(_)));
    }

    /// Dropping a filter onto a wire that leaves an effect now splices it in.
    #[test]
    fn a_filter_dropped_on_an_effect_wire_splices_in() {
        let mut graph = bare();
        let reverb = graph.add_node(NodeKind::Reverb, Point::new(300.0, 300.0));
        graph.connect(0, Port::Audio, reverb, Port::Audio);
        graph.connect(reverb, Port::Audio, 1, Port::Slot(0));
        let filter = graph.add_node(NodeKind::Filter, Point::new(0.0, 0.0));
        let wire = graph
            .connections
            .iter()
            .position(|c| c.from == reverb && c.to == 1)
            .unwrap();
        graph.insert_into(filter, wire);
        let from_reverb = graph
            .connections
            .iter()
            .any(|c| c.from == reverb && c.to == filter);
        let to_output = graph
            .connections
            .iter()
            .any(|c| c.from == filter && c.to == 1);
        assert!(from_reverb && to_output);
    }

    /// FM is dimmed on a mix that an effect feeds, and only there.
    #[test]
    fn fm_is_dimmed_on_a_mix_after_an_effect() {
        let (graph, mix) = parallel();
        let rows = crate::dsp::mix::MixMode::ALL.len();
        for row in 0..rows {
            assert_eq!(
                graph.dropdown_row_disabled(mix, 0, row),
                row == FM_ROW,
                "row {row}"
            );
        }
        // Another node's dropdown, and the mix's own later dropdowns, are never dimmed.
        assert!(!graph.dropdown_row_disabled(0, 0, FM_ROW));
        assert!(!graph.dropdown_row_disabled(mix, 1, FM_ROW));
    }

    /// A mix of two oscillators runs per voice, so FM is open to it.
    #[test]
    fn fm_is_open_on_a_mix_of_voices() {
        let mut graph = bare();
        let second = add_oscillator(&mut graph, Point::new(0.0, 300.0));
        let mix = graph.add_node(NodeKind::Mix, Point::new(300.0, 200.0));
        graph.connect(0, Port::Audio, mix, Port::AudioIn(0));
        graph.connect(second, Port::Audio, mix, Port::AudioIn(1));
        graph.connect(mix, Port::Audio, 1, Port::Slot(0));
        assert!(!graph.dropdown_row_disabled(mix, 0, FM_ROW));
    }

    /// A bypassed effect makes no stage, so a mix after it is still per voice and FM is open.
    #[test]
    fn a_bypassed_effect_does_not_dim_fm() {
        let (mut graph, mix) = parallel();
        let find = |graph: &Graph, kind| graph.nodes.iter().position(|n| n.kind == kind).unwrap();
        let reverb = find(&graph, NodeKind::Reverb);
        let delay = find(&graph, NodeKind::Delay);
        assert!(graph.dropdown_row_disabled(mix, 0, FM_ROW));
        graph.toggle_bypass(reverb);
        graph.toggle_bypass(delay);
        assert!(!graph.dropdown_row_disabled(mix, 0, FM_ROW));
    }

    /// The point on the open list's `row`, for a mix whose Type dropdown is open.
    fn row_point(graph: &Graph, mix: usize, row: usize) -> Point {
        let dropdown = graph.nodes[mix]
            .dropdowns()
            .into_iter()
            .find(|d| d.which == 0)
            .unwrap();
        let list = dropdown_list_bounds(dropdown.bounds, dropdown.labels.len());
        // The view starts unscaled and unmoved, so editor and screen coordinates agree.
        Point::new(
            list.x + 20.0,
            list.y + (row as f32 + 0.5) * WAVEFORM_ITEM_HEIGHT,
        )
    }

    /// A press on the dimmed FM row closes the list and leaves the mode as it was. A press on
    /// another row picks it.
    #[test]
    fn a_press_on_the_dimmed_row_changes_nothing() {
        let (mut graph, mix) = parallel();
        graph.nodes[mix].choose(0, 0);
        graph.dropdown_menu = Some((mix, 0));
        let at = row_point(&graph, mix, FM_ROW);
        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        let (_, message) = editor.update(
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            BOUNDS,
            Cursor::Available(at),
        );
        assert!(message.is_none(), "{message:?}");
        assert_eq!(graph.nodes[mix].choice(0), 0);
        assert!(graph.dropdown_menu.is_none());

        graph.dropdown_menu = Some((mix, 0));
        let at = row_point(&graph, mix, crate::dsp::mix::MixMode::Add as usize);
        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        let (_, message) = editor.update(
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            BOUNDS,
            Cursor::Available(at),
        );
        assert!(matches!(message, Some(Message::GraphChanged)));
        assert_eq!(
            graph.nodes[mix].choice(0),
            crate::dsp::mix::MixMode::Add as usize
        );
    }

    /// The dimmed row does not light under the cursor.
    #[test]
    fn the_dimmed_row_does_not_hover() {
        let (mut graph, mix) = parallel();
        graph.dropdown_menu = Some((mix, 0));
        let fm = row_point(&graph, mix, FM_ROW);
        assert_eq!(graph.hover_at(fm), None);
        let add = row_point(&graph, mix, crate::dsp::mix::MixMode::Add as usize);
        assert!(matches!(graph.hover_at(add), Some(Hover::Row { row, .. }) if row == 4));
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use crate::editor::canvas::NodeEditor;
    use nih_plug_iced::keyboard::{self, Modifiers};
    use nih_plug_iced::mouse;
    use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};

    const BOUNDS: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 1024.0,
        height: 712.0,
    };

    /// Three effects in a row, with nothing wired. They are nodes 0, 1 and 2.
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

    fn send(graph: &mut Graph, event: Event, at: Point) {
        let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        editor.update(event, BOUNDS, Cursor::Available(at));
    }

    fn mouse_event(graph: &mut Graph, event: mouse::Event, at: Point) {
        send(graph, Event::Mouse(event), at);
    }

    fn press(graph: &mut Graph, at: Point) {
        mouse_event(graph, mouse::Event::ButtonPressed(mouse::Button::Left), at);
    }

    fn move_to(graph: &mut Graph, at: Point) {
        mouse_event(graph, mouse::Event::CursorMoved { position: at }, at);
    }

    fn release(graph: &mut Graph, at: Point) {
        mouse_event(graph, mouse::Event::ButtonReleased(mouse::Button::Left), at);
    }

    fn hold_shift(graph: &mut Graph, held: bool) {
        let modifiers = if held {
            Modifiers::SHIFT
        } else {
            Modifiers::default()
        };
        send(
            graph,
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)),
            Point::new(0.0, 0.0),
        );
    }

    fn header(graph: &Graph, node: usize) -> Point {
        let bounds = graph.nodes[node].header_bounds();
        Point::new(
            bounds.x + bounds.width * 0.6,
            bounds.y + bounds.height / 2.0,
        )
    }

    /// Presses and releases the left button on a node's header.
    fn click(graph: &mut Graph, node: usize) {
        let at = header(graph, node);
        press(graph, at);
        release(graph, at);
    }

    fn sorted(mut nodes: Vec<usize>) -> Vec<usize> {
        nodes.sort_unstable();
        nodes
    }

    #[test]
    fn shift_click_adds_and_drops_nodes() {
        let mut graph = row();
        click(&mut graph, 0);
        hold_shift(&mut graph, true);
        click(&mut graph, 2);
        assert_eq!(sorted(graph.selection()), [0, 2]);
        assert_eq!(graph.selected, Some(2));
        // Shift+click on a selected node drops it.
        click(&mut graph, 0);
        assert_eq!(graph.selection(), vec![2]);
    }

    #[test]
    fn shift_click_does_not_start_a_drag() {
        let mut graph = row();
        hold_shift(&mut graph, true);
        let at = header(&graph, 1);
        press(&mut graph, at);
        assert!(matches!(graph.interaction, Interaction::Idle));
    }

    #[test]
    fn a_plain_click_selects_one_node() {
        let mut graph = row();
        graph.set_selection(vec![0, 1]);
        click(&mut graph, 2);
        assert_eq!(graph.selection(), vec![2]);
    }

    #[test]
    fn dragging_a_selected_node_moves_the_group() {
        let mut graph = row();
        graph.set_selection(vec![0, 1]);
        let before: Vec<Point> = graph.nodes.iter().map(|n| n.position).collect();
        let start = header(&graph, 1);
        press(&mut graph, start);
        let to = Point::new(start.x + 40.0, start.y + 30.0);
        move_to(&mut graph, to);
        release(&mut graph, to);
        for node in [0, 1] {
            assert_eq!(graph.nodes[node].position.x, before[node].x + 40.0);
            assert_eq!(graph.nodes[node].position.y, before[node].y + 30.0);
        }
        assert_eq!(graph.nodes[2].position, before[2]);
        assert_eq!(sorted(graph.selection()), [0, 1]);
    }

    #[test]
    fn dragging_an_unselected_node_moves_only_it() {
        let mut graph = row();
        graph.set_selection(vec![0, 1]);
        let before: Vec<Point> = graph.nodes.iter().map(|n| n.position).collect();
        let start = header(&graph, 2);
        press(&mut graph, start);
        let to = Point::new(start.x, start.y + 50.0);
        move_to(&mut graph, to);
        release(&mut graph, to);
        assert_eq!(graph.nodes[2].position.y, before[2].y + 50.0);
        assert_eq!(graph.nodes[0].position, before[0]);
        assert_eq!(graph.selection(), vec![2]);
    }

    #[test]
    fn dragging_a_box_selects_the_nodes_it_touches() {
        let mut graph = row();
        let from = Point::new(20.0, 100.0);
        press(&mut graph, from);
        assert!(matches!(graph.interaction, Interaction::Boxing { .. }));
        // The box reaches into the second node and stops short of the third.
        let to = Point::new(450.0, 190.0);
        move_to(&mut graph, to);
        assert_eq!(sorted(graph.selection()), [0, 1]);
        release(&mut graph, to);
        assert!(matches!(graph.interaction, Interaction::Idle));
        assert_eq!(sorted(graph.selection()), [0, 1]);
    }

    #[test]
    fn a_box_drawn_back_up_and_left_selects_too() {
        let mut graph = row();
        press(&mut graph, Point::new(1000.0, 120.0));
        move_to(&mut graph, Point::new(740.0, 190.0));
        release(&mut graph, Point::new(740.0, 190.0));
        assert_eq!(graph.selection(), vec![2]);
    }

    #[test]
    fn a_box_shrinks_as_the_mouse_moves_back() {
        let mut graph = row();
        press(&mut graph, Point::new(20.0, 100.0));
        move_to(&mut graph, Point::new(450.0, 190.0));
        move_to(&mut graph, Point::new(150.0, 190.0));
        assert_eq!(graph.selection(), vec![0]);
    }

    #[test]
    fn a_click_on_empty_canvas_clears_the_selection() {
        let mut graph = row();
        graph.set_selection(vec![0, 1]);
        press(&mut graph, Point::new(20.0, 100.0));
        release(&mut graph, Point::new(20.0, 100.0));
        assert!(graph.selection().is_empty());
    }

    #[test]
    fn a_shift_box_adds_to_the_selection() {
        let mut graph = row();
        graph.select(2);
        hold_shift(&mut graph, true);
        press(&mut graph, Point::new(20.0, 100.0));
        move_to(&mut graph, Point::new(150.0, 190.0));
        release(&mut graph, Point::new(150.0, 190.0));
        assert_eq!(sorted(graph.selection()), [0, 2]);
    }

    #[test]
    fn the_menu_deletes_the_whole_selection() {
        let mut graph = row();
        graph.connect(0, Port::Audio, 1, Port::Audio);
        graph.set_selection(vec![0, 1]);
        let at = header(&graph, 0);
        mouse_event(
            &mut graph,
            mouse::Event::ButtonPressed(mouse::Button::Right),
            at,
        );
        // A right press on a node of the group keeps the group.
        assert_eq!(sorted(graph.selection()), [0, 1]);
        let menu = graph.menu.expect("menu open");
        // Delete is the last entry.
        let area = menu_bounds(menu);
        let delete = Point::new(area.x + 10.0, area.y + area.height - 4.0);
        press(&mut graph, delete);
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].kind, NodeKind::Chorus);
        assert!(graph.connections.is_empty());
        assert!(graph.selection().is_empty());
    }

    #[test]
    fn deleting_a_node_shifts_the_rest_of_the_selection_down() {
        let mut graph = row();
        graph.set_selection(vec![1, 2]);
        graph.delete_node(0);
        assert_eq!(sorted(graph.selection()), [0, 1]);
    }

    fn open_menu(graph: &mut Graph, node: usize) -> crate::editor::menus::ContextMenu {
        let at = header(graph, node);
        mouse_event(graph, mouse::Event::ButtonPressed(mouse::Button::Right), at);
        graph.menu.expect("menu open")
    }

    #[test]
    fn auto_pack_is_offered_only_for_a_group() {
        use crate::editor::menus::MenuItem;
        let mut graph = row();
        graph.select(0);
        assert!(!open_menu(&mut graph, 0).items.contains(&MenuItem::AutoPack));
        graph.set_selection(vec![0, 1]);
        let menu = open_menu(&mut graph, 0);
        assert!(menu.items.contains(&MenuItem::AutoPack));
        // Delete stays last.
        assert_eq!(menu.items.last(), Some(&MenuItem::Delete));
    }

    #[test]
    fn auto_pack_puts_a_chain_in_signal_order() {
        use crate::editor::menus::MenuItem;
        let mut graph = row();
        // Chorus feeds Delay, and Delay feeds Reverb, which is the reverse of how they sit.
        graph.connect(2, Port::Audio, 1, Port::Audio);
        graph.connect(1, Port::Audio, 0, Port::Audio);
        graph.set_selection(vec![0, 1, 2]);
        let menu = open_menu(&mut graph, 0);
        let row_of = menu.items.iter().position(|i| *i == MenuItem::AutoPack);
        let area = menu_bounds(menu);
        let at = Point::new(
            area.x + 10.0,
            area.y + (row_of.unwrap() as f32 + 0.5) * area.height / menu.items.len() as f32,
        );
        press(&mut graph, at);
        let x = |node: usize| graph.nodes[node].position.x;
        assert!(x(2) < x(1) && x(1) < x(0));
        assert_eq!(sorted(graph.selection()), [0, 1, 2]);
    }

    #[test]
    fn a_right_press_on_an_unselected_node_selects_only_it() {
        let mut graph = row();
        graph.set_selection(vec![0, 1]);
        let at = header(&graph, 2);
        mouse_event(
            &mut graph,
            mouse::Event::ButtonPressed(mouse::Button::Right),
            at,
        );
        assert_eq!(graph.selection(), vec![2]);
    }
}

#[cfg(test)]
mod splice_and_link_tests {
    use super::*;
    use crate::editor::canvas::NodeEditor;
    use crate::nodes::NODE_WIDTH;
    use nih_plug_iced::keyboard::{self, Modifiers};
    use nih_plug_iced::mouse;
    use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};
    use std::time::{Duration, Instant};

    const BOUNDS: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 1024.0,
        height: 712.0,
    };

    /// An oscillator feeding a delay that feeds the output, with the oscillator and delay `gap`
    /// apart and the output well to the right. Returns the graph and the oscillator-to-delay wire.
    fn chain(gap: f32) -> (Graph, usize) {
        let mut graph = Graph::with(
            vec![
                FlodeNode::new(NodeKind::Oscillator, Point::new(0.0, 80.0)),
                FlodeNode::new(NodeKind::Delay, Point::new(NODE_WIDTH + gap, 80.0)),
                FlodeNode::new(NodeKind::Output, Point::new(900.0, 80.0)),
            ],
            Vec::new(),
        );
        graph.connect(0, Port::Audio, 1, Port::Audio);
        graph.connect(1, Port::Audio, 2, Port::Slot(0));
        (graph, 0)
    }

    #[test]
    fn a_splice_between_close_nodes_spaces_them_out() {
        let (mut graph, link) = chain(10.0);
        let filter = graph.add_node(NodeKind::Filter, Point::new(NODE_WIDTH + 2.0, 300.0));
        let output_before = graph.nodes[2].position.x;
        graph.insert_into(filter, link);
        graph.finish_slides();
        let left = graph.nodes[0].bounds();
        let new = graph.nodes[filter].bounds();
        let right = graph.nodes[1].bounds();
        assert!(new.x - (left.x + left.width) >= 40.0, "left gap");
        assert!(right.x - (new.x + new.width) >= 40.0, "right gap");
        // The output is downstream of the delay, so it moved by the same amount.
        let shift = right.x - (NODE_WIDTH + 10.0);
        assert!(shift > 0.0);
        assert_eq!(graph.nodes[2].position.x, output_before + shift);
        // Heights stay where they were.
        assert_eq!(graph.nodes[filter].position.y, 300.0);
        assert_eq!(graph.nodes[1].position.y, 80.0);
    }

    #[test]
    fn a_splice_with_room_to_spare_moves_nothing_downstream() {
        let (mut graph, link) = chain(400.0);
        let filter = graph.add_node(NodeKind::Filter, Point::new(NODE_WIDTH + 100.0, 300.0));
        graph.insert_into(filter, link);
        graph.finish_slides();
        assert_eq!(graph.nodes[1].position.x, NODE_WIDTH + 400.0);
        assert_eq!(graph.nodes[2].position.x, 900.0);
        assert_eq!(graph.nodes[filter].position.x, NODE_WIDTH + 100.0);
    }

    #[test]
    fn a_node_dropped_outside_a_wide_enough_gap_slides_into_it() {
        let (mut graph, link) = chain(400.0);
        // Dropped over the oscillator's own space, left of where the margin allows.
        let filter = graph.add_node(NodeKind::Filter, Point::new(20.0, 300.0));
        graph.insert_into(filter, link);
        graph.finish_slides();
        assert_eq!(graph.nodes[filter].position.x, NODE_WIDTH + 40.0);
    }

    #[test]
    fn nodes_ease_toward_their_place() {
        let (mut graph, link) = chain(10.0);
        let filter = graph.add_node(NodeKind::Filter, Point::new(NODE_WIDTH + 2.0, 300.0));
        graph.insert_into(filter, link);
        let start = graph.nodes[1].position.x;
        let end = graph.slides.iter().find(|s| s.node == 1).unwrap().to.x;
        assert_eq!(
            graph.nodes[1].position.x, start,
            "nothing moves until a frame"
        );
        let now = Instant::now();
        assert!(!graph.advance_slides(now + Duration::from_millis(60)));
        let mid = graph.nodes[1].position.x;
        assert!(mid > start && mid < end, "{start} < {mid} < {end}");
        // Easing out covers more than half the way in a third of the time.
        assert!(mid - start > (end - start) / 3.0);
        assert!(graph.advance_slides(now + Duration::from_secs(1)));
        assert_eq!(graph.nodes[1].position.x, end);
        assert!(graph.slides.is_empty());
    }

    #[test]
    fn a_wire_that_loops_back_does_not_push_nodes() {
        // A filter, a delay and a second filter in a ring, so the delay's downstream reaches back
        // to the first filter.
        let mut graph = Graph::with(
            vec![
                FlodeNode::new(NodeKind::Filter, Point::new(0.0, 80.0)),
                FlodeNode::new(NodeKind::Delay, Point::new(NODE_WIDTH + 10.0, 80.0)),
                FlodeNode::new(NodeKind::Filter, Point::new(900.0, 80.0)),
            ],
            Vec::new(),
        );
        graph.connect(0, Port::Audio, 1, Port::Audio);
        graph.connect(1, Port::Audio, 2, Port::Audio);
        graph.connect(2, Port::Audio, 0, Port::Audio);
        let link = graph
            .connections
            .iter()
            .position(|c| c.from == 0 && c.to == 1)
            .unwrap();
        let filter = graph.add_node(NodeKind::Filter, Point::new(NODE_WIDTH + 2.0, 300.0));
        graph.insert_into(filter, link);
        graph.finish_slides();
        assert_eq!(graph.nodes[2].position.x, 900.0);
        assert_eq!(graph.nodes[1].position.x, NODE_WIDTH + 10.0);
    }

    fn node_graph(kinds: &[NodeKind]) -> Graph {
        let nodes = kinds
            .iter()
            .enumerate()
            .map(|(i, &kind)| FlodeNode::new(kind, Point::new(i as f32 * 300.0, 80.0)))
            .collect();
        Graph::with(nodes, Vec::new())
    }

    fn link(graph: &Graph, from: usize, to: usize) -> Option<(Port, Port)> {
        graph.best_link(from, to).map(|c| (c.out, c.input))
    }

    #[test]
    fn matching_names_win() {
        let graph = node_graph(&[NodeKind::Envelope, NodeKind::Oscillator]);
        assert_eq!(link(&graph, 0, 1), Some((Port::Shape, Port::Shape)));
    }

    #[test]
    fn audio_goes_to_the_first_free_audio_input() {
        let mut graph = node_graph(&[NodeKind::Oscillator, NodeKind::Oscillator, NodeKind::Mix]);
        assert_eq!(link(&graph, 0, 2), Some((Port::Audio, Port::AudioIn(0))));
        graph.connect(0, Port::Audio, 2, Port::AudioIn(0));
        assert_eq!(link(&graph, 1, 2), Some((Port::Audio, Port::AudioIn(1))));
    }

    #[test]
    fn audio_goes_to_the_next_empty_output_slot() {
        let mut graph = node_graph(&[NodeKind::Oscillator, NodeKind::Oscillator, NodeKind::Output]);
        graph.auto_connect(0, 2);
        assert!(graph
            .connections
            .iter()
            .any(|c| c.from == 0 && c.input == Port::Slot(0)));
        graph.auto_connect(1, 2);
        assert!(graph
            .connections
            .iter()
            .any(|c| c.from == 1 && c.input == Port::Slot(1)));
    }

    #[test]
    fn notes_go_to_the_notes_pin() {
        let graph = node_graph(&[NodeKind::Input, NodeKind::Oscillator]);
        assert_eq!(link(&graph, 0, 1), Some((Port::Notes, Port::Notes)));
    }

    #[test]
    fn audio_beats_control_when_both_could_join() {
        // A filter takes audio on one pin and control on its knobs, so an LFO's control goes to a
        // knob and an oscillator's audio goes to the audio pin.
        let graph = node_graph(&[NodeKind::Oscillator, NodeKind::Lfo, NodeKind::Filter]);
        assert_eq!(link(&graph, 0, 2), Some((Port::Audio, Port::Audio)));
        let (out, input) = link(&graph, 1, 2).expect("lfo to knob");
        assert_eq!(out, Port::Shape);
        assert!(matches!(input, Port::Knob(_)));
    }

    #[test]
    fn a_wire_already_there_is_not_made_twice() {
        let mut graph = node_graph(&[NodeKind::Oscillator, NodeKind::Filter]);
        assert!(graph.auto_connect(0, 1));
        let count = graph.connections.len();
        // Nothing else of the filter takes audio, so a second try finds no wire.
        assert!(!graph.auto_connect(0, 1) || graph.connections.len() == count);
    }

    #[test]
    fn dragging_from_the_later_node_still_connects() {
        let mut graph = node_graph(&[NodeKind::Oscillator, NodeKind::Filter]);
        assert!(graph.auto_connect(1, 0));
        assert!(graph
            .connections
            .iter()
            .any(|c| c.from == 0 && c.to == 1 && c.out == Port::Audio));
    }

    #[test]
    fn nodes_with_nothing_to_join_stay_apart() {
        let mut graph = node_graph(&[NodeKind::Output, NodeKind::Output]);
        assert!(!graph.auto_connect(0, 1));
        assert!(graph.connections.is_empty());
    }

    fn send(graph: &mut Graph, event: Event, at: Point) {
        let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        editor.update(event, BOUNDS, Cursor::Available(at));
    }

    fn mouse_at(graph: &mut Graph, event: mouse::Event, at: Point) {
        send(graph, Event::Mouse(event), at);
    }

    fn middle(graph: &Graph, node: usize) -> Point {
        let bounds = graph.nodes[node].header_bounds();
        Point::new(
            bounds.x + bounds.width * 0.6,
            bounds.y + bounds.height / 2.0,
        )
    }

    fn shift(graph: &mut Graph, held: bool) {
        let modifiers = if held {
            Modifiers::SHIFT
        } else {
            Modifiers::default()
        };
        send(
            graph,
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)),
            Point::new(0.0, 0.0),
        );
    }

    #[test]
    fn shift_right_drag_from_one_node_to_another_connects_them() {
        let mut graph = node_graph(&[NodeKind::Oscillator, NodeKind::Filter]);
        shift(&mut graph, true);
        let (a, b) = (middle(&graph, 0), middle(&graph, 1));
        mouse_at(
            &mut graph,
            mouse::Event::ButtonPressed(mouse::Button::Right),
            a,
        );
        assert!(matches!(graph.interaction, Interaction::AutoLinking { .. }));
        assert!(graph.menu.is_none());
        mouse_at(&mut graph, mouse::Event::CursorMoved { position: b }, b);
        mouse_at(
            &mut graph,
            mouse::Event::ButtonReleased(mouse::Button::Right),
            b,
        );
        assert!(matches!(graph.interaction, Interaction::Idle));
        assert_eq!(graph.connections.len(), 1);
        assert_eq!(graph.connections[0].to, 1);
    }

    #[test]
    fn shift_right_drag_that_ends_on_empty_canvas_does_nothing() {
        let mut graph = node_graph(&[NodeKind::Oscillator, NodeKind::Filter]);
        shift(&mut graph, true);
        let a = middle(&graph, 0);
        mouse_at(
            &mut graph,
            mouse::Event::ButtonPressed(mouse::Button::Right),
            a,
        );
        let empty = Point::new(900.0, 600.0);
        mouse_at(
            &mut graph,
            mouse::Event::ButtonReleased(mouse::Button::Right),
            empty,
        );
        assert!(graph.connections.is_empty());
        assert!(matches!(graph.interaction, Interaction::Idle));
    }

    /// Shift+right drag between two nodes, through the editor's real mouse events.
    fn drag_link(graph: &mut Graph, from: usize, to: usize) {
        shift(graph, true);
        let (a, b) = (middle(graph, from), middle(graph, to));
        mouse_at(graph, mouse::Event::ButtonPressed(mouse::Button::Right), a);
        mouse_at(graph, mouse::Event::CursorMoved { position: b }, b);
        mouse_at(graph, mouse::Event::ButtonReleased(mouse::Button::Right), b);
        shift(graph, false);
    }

    #[test]
    fn shift_right_drag_connects_a_math_node() {
        // An LFO into the math node, then the math node onward into a filter.
        let mut graph = node_graph(&[NodeKind::Lfo, NodeKind::Math, NodeKind::Filter]);
        drag_link(&mut graph, 0, 1);
        assert_eq!(graph.connections.len(), 1, "{:?}", graph.connections);
        assert_eq!(graph.connections[0].to, 1);
        assert_eq!(graph.connections[0].input, Port::Knob(KnobId::MathA));
        drag_link(&mut graph, 1, 2);
        assert_eq!(graph.connections.len(), 2, "{:?}", graph.connections);
        assert!(graph.connections.iter().any(|c| c.from == 1 && c.to == 2));
        assert!(graph.is_driven(2, KnobId::Cutoff));
    }

    #[test]
    fn a_plain_right_click_still_opens_the_menu() {
        let mut graph = node_graph(&[NodeKind::Oscillator, NodeKind::Filter]);
        let a = middle(&graph, 0);
        mouse_at(
            &mut graph,
            mouse::Event::ButtonPressed(mouse::Button::Right),
            a,
        );
        assert!(graph.menu.is_some());
    }
}

#[cfg(test)]
mod eq_tests {
    use super::*;
    use crate::nodes::eq;
    use crate::nodes::viewer::Live;

    fn eq_node() -> FlodeNode {
        FlodeNode::new(NodeKind::Eq, Point::new(40.0, 60.0))
    }

    /// The node is wider than the others, and everything it draws stays inside it.
    #[test]
    fn the_eq_node_is_wide_and_its_parts_fit() {
        let node = eq_node();
        assert!(node.width() > crate::nodes::NODE_WIDTH);
        assert_eq!(node.bounds().width, node.width());
        let graph = node.eq_bounds().expect("the graph is on show");
        let inside = |r: Rectangle| {
            r.x >= node.position.x
                && r.x + r.width <= node.position.x + node.width()
                && r.y >= node.position.y
                && r.y + r.height <= node.position.y + node.height()
        };
        assert!(inside(graph));
        let toggles = node.eq_toggles();
        assert_eq!(toggles.len(), 4);
        for (_, toggle) in toggles {
            assert!(inside(toggle) && toggle.width > 30.0);
            assert!(!super::overlaps(&toggle, &graph));
        }
        // The bands start folded, so the node is short, and a folded band's pins are still there.
        assert!(node.height() < 440.0, "{}", node.height());
        assert_eq!(node.input_ports().len(), 1 + 12);
        // Another kind keeps the usual width.
        let other = FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0));
        assert_eq!(other.width(), crate::nodes::NODE_WIDTH);
    }

    #[test]
    fn a_collapsed_eq_has_no_graph_to_grab() {
        let mut node = eq_node();
        let handle = eq_handles(&node, &Live::of(&node)).unwrap()[1];
        assert_eq!(eq_handle_at(&node, &Live::of(&node), handle), Some(1));
        node.collapsed = true;
        assert!(eq_handles(&node, &Live::of(&node)).is_none());
        assert_eq!(eq_handle_at(&node, &Live::of(&node), handle), None);
        assert!(node.eq_toggles().is_empty());
    }

    #[test]
    fn a_press_grabs_the_point_it_lands_on() {
        let node = eq_node();
        let handles = eq_handles(&node, &Live::of(&node)).unwrap();
        for (band, handle) in handles.iter().enumerate() {
            assert_eq!(eq_handle_at(&node, &Live::of(&node), *handle), Some(band));
            assert_eq!(
                eq_handle_at(
                    &node,
                    &Live::of(&node),
                    Point::new(handle.x + 5.0, handle.y - 5.0)
                ),
                Some(band)
            );
        }
        let far = Point::new(handles[1].x, handles[1].y - 40.0);
        assert_eq!(eq_handle_at(&node, &Live::of(&node), far), None);
    }

    /// Dragging right raises the frequency and dragging up raises the gain, and the point lands
    /// under the cursor.
    #[test]
    fn dragging_a_point_sets_frequency_and_gain() {
        let mut node = eq_node();
        let bounds = node.eq_bounds().unwrap();
        let target = Point::new(
            bounds.x + bounds.width * 0.75,
            bounds.y + bounds.height * 0.25,
        );
        drag_eq_point(&mut node, 1, target, EqDriven::default());
        let handle = eq_handles(&node, &Live::of(&node)).unwrap()[1];
        assert!((handle.x - target.x).abs() < 0.5 && (handle.y - target.y).abs() < 0.5);
        assert!(node.plain(eq::FREQ[1]) > 1000.0);
        assert!((node.plain(eq::GAIN[1]) - 9.0).abs() < 0.1);
        // The other bands did not move.
        assert!((node.plain(eq::FREQ[2]) - eq::FREQ[2].default_plain()).abs() < 0.1);
        assert_eq!(node.plain(eq::GAIN[2]), 0.0);
    }

    #[test]
    fn a_drag_past_the_box_stops_at_its_edges() {
        let mut node = eq_node();
        drag_eq_point(
            &mut node,
            2,
            Point::new(-500.0, -500.0),
            EqDriven::default(),
        );
        assert!((node.plain(eq::FREQ[2]) - 20.0).abs() < 0.01);
        assert!((node.plain(eq::GAIN[2]) - 18.0).abs() < 0.01);
        drag_eq_point(
            &mut node,
            2,
            Point::new(5000.0, 5000.0),
            EqDriven::default(),
        );
        assert!((node.plain(eq::FREQ[2]) - 20000.0).abs() < 1.0);
        assert!((node.plain(eq::GAIN[2]) + 18.0).abs() < 0.01);
    }

    /// A cut has no gain, so a vertical drag moves only its frequency.
    #[test]
    fn a_cut_point_moves_only_sideways() {
        let mut node = eq_node();
        node.set_switch(eq::band_switch(0), true);
        let bounds = node.eq_bounds().unwrap();
        drag_eq_point(
            &mut node,
            0,
            Point::new(bounds.x + bounds.width * 0.3, bounds.y + 2.0),
            EqDriven::default(),
        );
        assert_eq!(node.plain(eq::GAIN[0]), 0.0);
        assert!(node.plain(eq::FREQ[0]) > 20.0);
    }

    #[test]
    fn a_toggle_is_found_by_its_position() {
        let node = eq_node();
        for (band, toggle) in node.eq_toggles() {
            let middle = Point::new(
                toggle.x + toggle.width / 2.0,
                toggle.y + toggle.height / 2.0,
            );
            assert_eq!(node.eq_toggle_at(middle), Some(band));
        }
        assert_eq!(node.eq_toggle_at(Point::new(0.0, 0.0)), None);
    }

    /// The graph, the toggles, and the points survive saving and loading.
    #[test]
    fn an_eq_survives_a_save() {
        let mut graph = minimal_graph();
        let index = graph.add_node(NodeKind::Eq, Point::new(300.0, 200.0));
        {
            let node = &mut graph.nodes[index];
            let bounds = node.eq_bounds().unwrap();
            drag_eq_point(
                node,
                2,
                Point::new(bounds.x + 100.0, bounds.y + 20.0),
                EqDriven::default(),
            );
            node.set_switch(eq::band_switch(3), true);
        }
        let freq = graph.nodes[index].plain(eq::FREQ[2]);
        let saved = graph.to_saved();
        let loaded = Graph::from_saved(&saved);
        let node = &loaded.nodes[index];
        assert_eq!(node.kind, NodeKind::Eq);
        assert!((node.plain(eq::FREQ[2]) - freq).abs() < 0.01);
        assert!(node.switch(eq::band_switch(3)));
        assert!(!node.switch(eq::band_switch(0)));
    }

    /// The toggles sit in the section headers, so they stay on show while the sections are
    /// folded, and open sections move them down with their rows.
    #[test]
    fn the_toggles_ride_the_section_headers() {
        let mut node = eq_node();
        let folded = node.eq_toggles();
        assert!(node.section_collapsed(0));
        node.toggle_section(0);
        let open = node.eq_toggles();
        assert_eq!(open.len(), 4);
        assert_eq!(open[0].1.y, folded[0].1.y);
        assert!(open[1].1.y > folded[1].1.y);
    }

    /// A band's point follows a wire that drives its frequency or gain, and the knob row shows
    /// the same values the point is drawn from.
    #[test]
    fn a_driven_point_follows_its_wire() {
        let node = eq_node();
        let own = eq_handles(&node, &Live::of(&node)).unwrap()[1];
        let live = Live::new(&node, |id| match id {
            id if id == eq::FREQ[1] => Some(0.9),
            id if id == eq::GAIN[1] => Some(1.0),
            _ => None,
        });
        let driven = eq_handles(&node, &live).unwrap()[1];
        assert!(driven.x > own.x && driven.y < own.y);
        // Grabbing it by where it is drawn finds it.
        assert_eq!(eq_handle_at(&node, &live, driven), Some(1));
        assert_eq!(eq_handle_at(&node, &live, own), None);
    }

    /// Dragging a point leaves a driven value to its wire and moves the other.
    #[test]
    fn a_drag_leaves_driven_values_alone() {
        let mut node = eq_node();
        let bounds = node.eq_bounds().unwrap();
        let target = Point::new(bounds.x + bounds.width * 0.8, bounds.y + 5.0);
        let (freq, gain) = (node.value(eq::FREQ[1]), node.value(eq::GAIN[1]));
        drag_eq_point(
            &mut node,
            1,
            target,
            EqDriven {
                freq: true,
                gain: false,
            },
        );
        assert_eq!(node.value(eq::FREQ[1]), freq);
        assert!(node.value(eq::GAIN[1]) > gain);
        let freq_after = node.value(eq::FREQ[1]);
        drag_eq_point(
            &mut node,
            1,
            target,
            EqDriven {
                freq: false,
                gain: true,
            },
        );
        assert!(node.value(eq::FREQ[1]) > freq_after);
    }

    /// A knob moves the point, and a drag moves the knob, since both are one stored value.
    #[test]
    fn the_knobs_and_the_points_share_values() {
        let mut node = eq_node();
        node.set_value(eq::GAIN[2], eq::GAIN[2].to_normalized(12.0));
        let handle = eq_handles(&node, &Live::of(&node)).unwrap()[2];
        let bounds = node.eq_bounds().unwrap();
        assert!(handle.y < bounds.y + bounds.height / 2.0);
        drag_eq_point(
            &mut node,
            2,
            Point::new(handle.x, bounds.y + bounds.height),
            EqDriven::default(),
        );
        assert!((node.plain(eq::GAIN[2]) + 18.0).abs() < 0.01);
    }

    /// A graph with an oscillator through an EQ into the output.
    fn eq_chain() -> (Graph, usize) {
        let mut graph = minimal_graph();
        let eq_node = graph.add_node(NodeKind::Eq, Point::new(300.0, 200.0));
        graph.connect(0, Port::Audio, eq_node, Port::Audio);
        graph.connect(eq_node, Port::Audio, 1, Port::Slot(0));
        graph.refresh_slots();
        (graph, eq_node)
    }

    /// The analyzer shows the spectrum of the instance the compiler gave the node, and only while
    /// the checkbox is on.
    #[test]
    fn the_analyzer_reads_its_nodes_instance() {
        use crate::dsp::analyzer::{BINS, FLOOR_DB};
        let (mut graph, node) = eq_chain();
        graph.spectra[0] = [-20.0; BINS];
        graph.spectra[1] = [-50.0; BINS];
        assert!(graph.eq_spectrum(node).is_none());
        graph.nodes[node].set_switch(0, true);
        assert_eq!(graph.eq_spectrum(node), Some(&[-20.0; BINS]));
        assert_ne!(graph.eq_spectrum(node), Some(&[FLOOR_DB; BINS]));
        // The checkbox reaches the stage, so the engine runs the analyzer.
        let stage = graph.patch().lanes[0]
            .stages
            .iter()
            .find_map(|s| match s.params {
                StageParams::Eq(p) => Some(p),
                _ => None,
            });
        assert!(stage.expect("an EQ stage").analyzer);
    }

    /// The checkbox sits just under the graph.
    #[test]
    fn the_analyzer_checkbox_is_under_the_graph() {
        let node = eq_node();
        let graph = node.eq_bounds().unwrap();
        let checkbox = node.switch_bounds(0);
        assert!(node.switch_shown(0));
        assert!(checkbox.y >= graph.y + graph.height);
        assert!(checkbox.y - (graph.y + graph.height) < 12.0);
    }

    /// The smoothed spectrum passes through every point and has one segment between each pair.
    #[test]
    fn the_smooth_curve_passes_through_its_points() {
        let points: Vec<Point> = [10.0, 40.0, 5.0, 90.0, 60.0]
            .iter()
            .enumerate()
            .map(|(i, y)| Point::new(i as f32 * 10.0, *y))
            .collect();
        let segments = smooth_segments(&points, 0.0, 100.0);
        assert_eq!(segments.len(), points.len() - 1);
        for (segment, point) in segments.iter().zip(&points[1..]) {
            assert_eq!(segment.2, *point);
        }
        assert!(smooth_segments(&points[..1], 0.0, 100.0).is_empty());
    }

    /// A steep peak next to a floor stays inside the box, and a flat run stays flat.
    #[test]
    fn the_smooth_curve_stays_in_the_box() {
        let points = [
            Point::new(0.0, 100.0),
            Point::new(10.0, 100.0),
            Point::new(20.0, 0.0),
            Point::new(30.0, 100.0),
            Point::new(40.0, 100.0),
        ];
        for (c1, c2, _) in smooth_segments(&points, 0.0, 100.0) {
            assert!((0.0..=100.0).contains(&c1.y) && (0.0..=100.0).contains(&c2.y));
        }
        let flat = [
            Point::new(0.0, 50.0),
            Point::new(10.0, 50.0),
            Point::new(20.0, 50.0),
        ];
        assert!(smooth_segments(&flat, 0.0, 100.0)
            .iter()
            .all(|(c1, c2, end)| c1.y == 50.0 && c2.y == 50.0 && end.y == 50.0));
    }
}

#[cfg(test)]
mod envelope_handle_tests {
    use super::*;
    use crate::dsp::envelope::EnvPhase;
    use crate::editor::canvas::NodeEditor;
    use crate::nodes::viewer::Live;
    use nih_plug_iced::mouse;
    use nih_plug_iced::widget::canvas::{event::Event, Cursor, Frame, Program};

    const BOUNDS: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 1024.0,
        height: 712.0,
    };

    const KNOBS: [KnobId; 4] = [
        KnobId::Attack,
        KnobId::Decay,
        KnobId::Sustain,
        KnobId::Release,
    ];

    fn envelope_node() -> FlodeNode {
        let mut node = FlodeNode::new(NodeKind::Envelope, Point::new(300.0, 200.0));
        for (id, n) in [
            (KnobId::Attack, 0.3),
            (KnobId::Decay, 0.5),
            (KnobId::Sustain, 0.6),
            (KnobId::Release, 0.4),
        ] {
            node.set_value(id, n);
        }
        node
    }

    fn values(node: &FlodeNode) -> [f32; 4] {
        KNOBS.map(|id| node.value(id))
    }

    fn handles_of(node: &FlodeNode) -> [Point; 3] {
        envelope_handles(node, &Live::of(node)).expect("the viewer is on show")
    }

    fn drag(node: &mut FlodeNode, handle: usize, to: Point, driven: EnvelopeDriven) {
        let live = Live::of(node);
        drag_envelope_handle(node, handle, to, &live, driven);
    }

    /// A graph with the envelope as its only node.
    fn graph_of(node: FlodeNode) -> Graph {
        Graph::with(vec![node], Vec::new())
    }

    fn send(graph: &mut Graph, event: mouse::Event, at: Point) {
        let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        editor.update(Event::Mouse(event), BOUNDS, Cursor::Available(at));
    }

    fn press(graph: &mut Graph, at: Point) {
        send(graph, mouse::Event::ButtonPressed(mouse::Button::Left), at);
    }

    fn release(graph: &mut Graph, at: Point) {
        send(graph, mouse::Event::ButtonReleased(mouse::Button::Left), at);
    }

    #[test]
    fn handle_positions_follow_the_knob_values() {
        let mut node = envelope_node();
        let bounds = node.viewer_bounds().unwrap();
        let before = handles_of(&node);
        // The attack and the release sit on the top and bottom edges, and the decay at the sustain.
        assert!((before[0].y - bounds.y).abs() < 1e-3);
        assert!((before[2].y - (bounds.y + bounds.height)).abs() < 1e-3);
        assert!((before[1].y - (bounds.y + 0.4 * bounds.height)).abs() < 0.5);
        assert!(before[0].x < before[1].x && before[1].x < before[2].x);
        node.set_value(KnobId::Sustain, 0.2);
        node.set_value(KnobId::Release, 1.0);
        let after = handles_of(&node);
        assert_eq!(after[0], before[0]);
        assert_eq!(after[1].x, before[1].x);
        assert!(after[1].y > before[1].y);
        assert!(after[2].x > before[2].x);
        assert!(after[2].x <= bounds.x + bounds.width + 1e-3);
    }

    #[test]
    fn dragging_the_attack_handle_sets_only_attack() {
        let mut node = envelope_node();
        let before = values(&node);
        let bounds = node.viewer_bounds().unwrap();
        drag(
            &mut node,
            0,
            Point::new(
                bounds.x + 0.2 * bounds.width,
                bounds.y + bounds.height / 2.0,
            ),
            EnvelopeDriven::default(),
        );
        let after = values(&node);
        assert_ne!(after[0], before[0]);
        assert_eq!(after[1..], before[1..]);
        // The handle lands under the cursor's x, and its y stays on the top edge.
        let handle = handles_of(&node)[0];
        assert!((handle.x - (bounds.x + 0.2 * bounds.width)).abs() < 0.5);
        assert!((handle.y - bounds.y).abs() < 1e-3);
    }

    #[test]
    fn dragging_the_decay_handle_sets_decay_and_sustain() {
        let mut node = envelope_node();
        let before = values(&node);
        let bounds = node.viewer_bounds().unwrap();
        let target = Point::new(
            bounds.x + 0.35 * bounds.width,
            bounds.y + 0.25 * bounds.height,
        );
        drag(&mut node, 1, target, EnvelopeDriven::default());
        let after = values(&node);
        assert_ne!(after[1], before[1]);
        assert!((after[2] - 0.75).abs() < 0.01, "{}", after[2]);
        assert_eq!((after[0], after[3]), (before[0], before[3]));
        let handle = handles_of(&node)[1];
        assert!((handle.x - target.x).abs() < 0.5 && (handle.y - target.y).abs() < 0.5);
    }

    #[test]
    fn dragging_the_release_handle_sets_only_release() {
        let mut node = envelope_node();
        let before = values(&node);
        let bounds = node.viewer_bounds().unwrap();
        drag(
            &mut node,
            2,
            Point::new(bounds.x + 0.9 * bounds.width, bounds.y + 3.0),
            EnvelopeDriven::default(),
        );
        let after = values(&node);
        assert!(after[3] > before[3]);
        assert_eq!(after[..3], before[..3]);
    }

    #[test]
    fn a_drag_past_the_box_stops_at_its_edges() {
        let mut node = envelope_node();
        drag(
            &mut node,
            1,
            Point::new(-500.0, -500.0),
            EnvelopeDriven::default(),
        );
        assert_eq!(node.value(KnobId::Decay), 0.0);
        assert_eq!(node.value(KnobId::Sustain), 1.0);
        drag(
            &mut node,
            1,
            Point::new(5000.0, 5000.0),
            EnvelopeDriven::default(),
        );
        assert_eq!(node.value(KnobId::Decay), 1.0);
        assert_eq!(node.value(KnobId::Sustain), 0.0);
        for handle in [0, 2] {
            drag(
                &mut node,
                handle,
                Point::new(5000.0, 5000.0),
                EnvelopeDriven::default(),
            );
        }
        assert!((node.value(KnobId::Attack) - 1.0).abs() < 1e-4);
        assert!((node.value(KnobId::Release) - 1.0).abs() < 1e-4);
        drag(
            &mut node,
            0,
            Point::new(-500.0, 0.0),
            EnvelopeDriven::default(),
        );
        assert_eq!(node.value(KnobId::Attack), 0.0);
    }

    /// A driven knob stays where its wire puts it, and the other knob of the handle still moves.
    #[test]
    fn a_drag_leaves_driven_knobs_alone() {
        let mut node = envelope_node();
        let bounds = node.viewer_bounds().unwrap();
        let target = Point::new(bounds.x + 0.7 * bounds.width, bounds.y + 2.0);
        let before = values(&node);
        drag(
            &mut node,
            1,
            target,
            EnvelopeDriven {
                time: true,
                level: false,
            },
        );
        assert_eq!(node.value(KnobId::Decay), before[1]);
        assert!(node.value(KnobId::Sustain) > before[2]);
        let sustain = node.value(KnobId::Sustain);
        drag(
            &mut node,
            1,
            Point::new(bounds.x + 0.7 * bounds.width, bounds.y + bounds.height),
            EnvelopeDriven {
                time: false,
                level: true,
            },
        );
        assert_eq!(node.value(KnobId::Sustain), sustain);
        assert_ne!(node.value(KnobId::Decay), before[1]);
        drag(
            &mut node,
            2,
            target,
            EnvelopeDriven {
                time: true,
                level: false,
            },
        );
        assert_eq!(node.value(KnobId::Release), before[3]);
    }

    /// A stage after a driven one is placed by where the wire puts it, so the drag lands under
    /// the cursor on the drawn curve.
    #[test]
    fn a_drag_reads_the_stages_before_it_from_the_live_values() {
        let mut node = envelope_node();
        let live = Live::new(&node, |id| (id == KnobId::Attack).then_some(1.0));
        let bounds = node.viewer_bounds().unwrap();
        let target = Point::new(bounds.x + 0.45 * bounds.width, bounds.y + 5.0);
        drag_envelope_handle(&mut node, 1, target, &live, EnvelopeDriven::default());
        let live = Live::new(&node, |id| (id == KnobId::Attack).then_some(1.0));
        let handle = envelope_handles(&node, &live).unwrap()[1];
        assert!((handle.x - target.x).abs() < 0.5);
    }

    #[test]
    fn a_driven_knob_moves_its_handle() {
        let node = envelope_node();
        let own = handles_of(&node);
        let live = Live::new(&node, |id| match id {
            KnobId::Decay => Some(1.0),
            KnobId::Sustain => Some(0.0),
            _ => None,
        });
        let driven = envelope_handles(&node, &live).unwrap();
        assert_eq!(driven[0], own[0]);
        assert!(driven[1].x > own[1].x && driven[1].y > own[1].y);
        assert!(driven[2].x > own[2].x);
        assert_eq!(envelope_handle_at(&node, &live, driven[1]), Some(1));
        assert_eq!(envelope_handle_at(&node, &live, own[1]), None);
    }

    #[test]
    fn a_press_grabs_the_handle_it_lands_on() {
        let node = envelope_node();
        let live = Live::of(&node);
        for (handle, at) in handles_of(&node).iter().enumerate() {
            assert_eq!(envelope_handle_at(&node, &live, *at), Some(handle));
            let near = Point::new(at.x + 4.0, at.y + 4.0);
            assert_eq!(envelope_handle_at(&node, &live, near), Some(handle));
        }
        let at = handles_of(&node)[1];
        let far = Point::new(at.x, at.y + 30.0);
        assert_eq!(envelope_handle_at(&node, &live, far), None);
    }

    #[test]
    fn a_collapsed_envelope_has_no_handles() {
        let mut node = envelope_node();
        let at = handles_of(&node)[1];
        assert_eq!(envelope_handle_at(&node, &Live::of(&node), at), Some(1));
        node.collapsed = true;
        assert!(envelope_handles(&node, &Live::of(&node)).is_none());
        assert_eq!(envelope_handle_at(&node, &Live::of(&node), at), None);
    }

    #[test]
    fn other_nodes_have_no_envelope_handles() {
        let node = FlodeNode::new(NodeKind::Filter, Point::new(300.0, 200.0));
        assert!(envelope_handles(&node, &Live::of(&node)).is_none());
    }

    /// A press on a handle selects the node and starts a drag. The cursor then moves the knobs,
    /// and the release ends the drag.
    #[test]
    fn pressing_and_dragging_a_handle_turns_the_knobs() {
        let mut graph = graph_of(envelope_node());
        let bounds = graph.nodes[0].viewer_bounds().unwrap();
        let start = handles_of(&graph.nodes[0])[1];
        press(&mut graph, start);
        assert_eq!(graph.selected, Some(0));
        assert_eq!(
            graph.interaction,
            Interaction::EnvelopeHandle { node: 0, handle: 1 }
        );
        let to = Point::new(
            bounds.x + 0.35 * bounds.width,
            bounds.y + 0.1 * bounds.height,
        );
        send(&mut graph, mouse::Event::CursorMoved { position: to }, to);
        let handle = handles_of(&graph.nodes[0])[1];
        assert!((handle.x - to.x).abs() < 0.5 && (handle.y - to.y).abs() < 0.5);
        assert!((graph.nodes[0].value(KnobId::Sustain) - 0.9).abs() < 0.01);
        release(&mut graph, to);
        assert_eq!(graph.interaction, Interaction::Idle);
    }

    #[test]
    fn a_double_click_resets_the_handles_knobs() {
        for (handle, reset, kept) in [
            (
                0,
                vec![KnobId::Attack],
                vec![KnobId::Decay, KnobId::Sustain],
            ),
            (
                1,
                vec![KnobId::Decay, KnobId::Sustain],
                vec![KnobId::Attack, KnobId::Release],
            ),
            (
                2,
                vec![KnobId::Release],
                vec![KnobId::Attack, KnobId::Sustain],
            ),
        ] {
            let mut graph = graph_of(envelope_node());
            let at = handles_of(&graph.nodes[0])[handle];
            let before = values(&graph.nodes[0]);
            press(&mut graph, at);
            release(&mut graph, at);
            press(&mut graph, at);
            for id in reset {
                assert_eq!(graph.nodes[0].value(id), id.default_normalized(), "{id:?}");
            }
            for id in kept {
                let place = KNOBS.iter().position(|k| *k == id).unwrap();
                assert_eq!(graph.nodes[0].value(id), before[place], "{id:?}");
            }
            // The second press ended the gesture, so a move does not drag.
            assert_eq!(graph.interaction, Interaction::Idle);
        }
    }

    #[test]
    fn a_press_on_the_box_away_from_a_handle_starts_no_drag() {
        let mut graph = graph_of(envelope_node());
        let bounds = graph.nodes[0].viewer_bounds().unwrap();
        let before = values(&graph.nodes[0]);
        let at = Point::new(
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        );
        assert_eq!(
            envelope_handle_at(&graph.nodes[0], &Live::of(&graph.nodes[0]), at),
            None
        );
        press(&mut graph, at);
        assert_eq!(graph.interaction, Interaction::Idle);
        assert_eq!(graph.selected, Some(0));
        send(
            &mut graph,
            mouse::Event::CursorMoved { position: at },
            Point::new(at.x + 20.0, at.y),
        );
        assert_eq!(values(&graph.nodes[0]), before);
    }

    /// The viewer draws with the handles, held and not, and with a live voice's dot on the curve.
    #[test]
    fn the_viewer_draws_with_a_live_voice() {
        let mut graph = graph_of(envelope_node());
        let mut levels = ControlLevels::default();
        for phase in [
            EnvPhase::Attack,
            EnvPhase::Decay,
            EnvPhase::Sustain,
            EnvPhase::Release,
        ] {
            levels.envelope_phase[0] = Some(phase);
            levels.envelope[0] = 0.4;
            for held in [false, true] {
                graph.interaction = if held {
                    Interaction::EnvelopeHandle { node: 0, handle: 1 }
                } else {
                    Interaction::Idle
                };
                let mut frame = Frame::new(BOUNDS.size());
                draw_viewer(&mut frame, 0, &graph, levels);
            }
        }
        draw_editor(&graph, levels, [0; 2], [0.0; 2], BOUNDS);
    }
}

#[cfg(test)]
mod knob_entry_tests {
    use super::*;
    use crate::editor::canvas::{Message, NodeEditor};
    use nih_plug_iced::keyboard::{self, KeyCode, Modifiers};
    use nih_plug_iced::mouse;
    use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};

    fn bounds() -> Rectangle {
        Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        }
    }

    /// A graph with one node of `kind` set well clear of the other nodes, and where its knob `id`
    /// is drawn.
    fn graph_with(kind: NodeKind, id: KnobId) -> (Graph, usize, Point) {
        let mut graph = minimal_graph();
        let node = graph.add_node(kind, Point::new(300.0, 120.0));
        let center = graph.nodes[node]
            .knob_slots()
            .into_iter()
            .find(|slot| slot.id == id)
            .expect("the knob is on show")
            .center;
        (graph, node, center)
    }

    fn editor(graph: &mut Graph) -> NodeEditor<'_> {
        NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2])
    }

    fn press(editor: &mut NodeEditor, at: Point) -> Option<Message> {
        let press = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        editor.update(press, bounds(), Cursor::Available(at)).1
    }

    fn release(editor: &mut NodeEditor, at: Point) {
        let release = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        editor.update(release, bounds(), Cursor::Available(at));
    }

    fn click(editor: &mut NodeEditor, at: Point) -> Option<Message> {
        let message = press(editor, at);
        release(editor, at);
        message
    }

    fn double_click(editor: &mut NodeEditor, at: Point) {
        click(editor, at);
        click(editor, at);
    }

    fn type_text(editor: &mut NodeEditor, text: &str) {
        for c in text.chars() {
            let event = Event::Keyboard(keyboard::Event::CharacterReceived(c));
            editor.update(event, bounds(), Cursor::Unavailable);
        }
    }

    fn key(editor: &mut NodeEditor, key_code: KeyCode) -> Option<Message> {
        let event = Event::Keyboard(keyboard::Event::KeyPressed {
            key_code,
            modifiers: Modifiers::default(),
        });
        editor.update(event, bounds(), Cursor::Unavailable).1
    }

    #[test]
    fn a_double_click_opens_the_box_with_the_value_selected() {
        let (mut graph, node, at) = graph_with(NodeKind::Filter, KnobId::Cutoff);
        let mut editor = editor(&mut graph);
        click(&mut editor, at);
        assert!(editor.graph.knob_entry.is_none(), "one click only drags");
        click(&mut editor, at);
        let entry = editor.graph.knob_entry.as_ref().expect("the box is open");
        assert_eq!((entry.node, entry.id), (node, KnobId::Cutoff));
        assert_eq!(entry.text, "8000");
        assert!(entry.fresh);
        // The double click did not reset the value, and the box sits over the knob.
        assert_eq!(
            editor.graph.nodes[node].plain(KnobId::Cutoff).round(),
            8000.0
        );
        let rect = editor.graph.knob_entry_bounds().unwrap();
        assert!(rect.contains(at));
    }

    #[test]
    fn typing_a_number_and_pressing_enter_sets_the_value_exactly() {
        let (mut graph, node, at) = graph_with(NodeKind::Filter, KnobId::Cutoff);
        let mut editor = editor(&mut graph);
        double_click(&mut editor, at);
        type_text(&mut editor, "440");
        assert_eq!(editor.graph.knob_entry.as_ref().unwrap().text, "440");
        let message = key(&mut editor, KeyCode::Enter);
        assert!(matches!(message, Some(Message::GraphChanged)));
        assert!(editor.graph.knob_entry.is_none());
        let plain = editor.graph.nodes[node].plain(KnobId::Cutoff);
        assert!((plain - 440.0).abs() < 0.1, "{plain}");
    }

    /// A number past the end of the range stops at the end.
    #[test]
    fn a_number_past_the_range_stops_at_the_end() {
        let (mut graph, node, at) = graph_with(NodeKind::Filter, KnobId::Cutoff);
        let mut editor = editor(&mut graph);
        double_click(&mut editor, at);
        type_text(&mut editor, "99999");
        key(&mut editor, KeyCode::Enter);
        assert_eq!(
            editor.graph.nodes[node].plain(KnobId::Cutoff).round(),
            20000.0
        );
    }

    /// A knob the node shows as a percentage is typed as one.
    #[test]
    fn a_percent_knob_takes_a_percentage() {
        let (mut graph, node, at) = graph_with(NodeKind::Reverb, KnobId::Mix);
        let mut editor = editor(&mut graph);
        double_click(&mut editor, at);
        // A new reverb is 30 percent wet, and the box shows it as typed.
        assert_eq!(editor.graph.knob_entry.as_ref().unwrap().text, "30");
        type_text(&mut editor, "75");
        key(&mut editor, KeyCode::Enter);
        assert!((editor.graph.nodes[node].plain(KnobId::Mix) - 0.75).abs() < 1e-4);
    }

    #[test]
    fn escape_closes_the_box_and_changes_nothing() {
        let (mut graph, node, at) = graph_with(NodeKind::Filter, KnobId::Cutoff);
        let before = graph.nodes[node].value(KnobId::Cutoff);
        let mut editor = editor(&mut graph);
        double_click(&mut editor, at);
        type_text(&mut editor, "440");
        key(&mut editor, KeyCode::Escape);
        assert!(editor.graph.knob_entry.is_none());
        assert_eq!(editor.graph.nodes[node].value(KnobId::Cutoff), before);
    }

    /// Backspace edits, and on a selected value it clears the whole thing.
    #[test]
    fn backspace_edits_the_text() {
        let (mut graph, _, at) = graph_with(NodeKind::Filter, KnobId::Cutoff);
        let mut editor = editor(&mut graph);
        double_click(&mut editor, at);
        key(&mut editor, KeyCode::Backspace);
        assert_eq!(editor.graph.knob_entry.as_ref().unwrap().text, "");
        type_text(&mut editor, "123");
        key(&mut editor, KeyCode::Backspace);
        assert_eq!(editor.graph.knob_entry.as_ref().unwrap().text, "12");
    }

    /// Letters that are not part of a number are ignored, and the text has a length limit.
    #[test]
    fn only_number_keys_are_taken() {
        let (mut graph, _, at) = graph_with(NodeKind::Filter, KnobId::Cutoff);
        let mut editor = editor(&mut graph);
        double_click(&mut editor, at);
        type_text(&mut editor, "a1b2.5z");
        assert_eq!(editor.graph.knob_entry.as_ref().unwrap().text, "12.5");
        type_text(&mut editor, "99999999999999999999");
        assert!(editor.graph.knob_entry.as_ref().unwrap().text.len() <= KNOB_ENTRY_MAX_LEN);
    }

    /// Text that is not a number closes the box and leaves the value.
    #[test]
    fn text_that_is_not_a_number_changes_nothing() {
        let (mut graph, node, at) = graph_with(NodeKind::Filter, KnobId::Cutoff);
        let before = graph.nodes[node].value(KnobId::Cutoff);
        let mut editor = editor(&mut graph);
        double_click(&mut editor, at);
        type_text(&mut editor, "-");
        let message = key(&mut editor, KeyCode::Enter);
        assert!(matches!(message, Some(Message::Hovered)));
        assert!(editor.graph.knob_entry.is_none());
        assert_eq!(editor.graph.nodes[node].value(KnobId::Cutoff), before);
    }

    /// A press elsewhere keeps what was typed. A box nobody typed in changes nothing.
    #[test]
    fn clicking_away_keeps_what_was_typed() {
        let (mut graph, node, at) = graph_with(NodeKind::Filter, KnobId::Cutoff);
        let before = graph.nodes[node].value(KnobId::Cutoff);
        let mut editor = editor(&mut graph);
        let away = Point::new(700.0, 500.0);

        double_click(&mut editor, at);
        click(&mut editor, away);
        assert!(editor.graph.knob_entry.is_none());
        assert_eq!(editor.graph.nodes[node].value(KnobId::Cutoff), before);

        double_click(&mut editor, at);
        type_text(&mut editor, "1000");
        click(&mut editor, away);
        assert!(editor.graph.knob_entry.is_none());
        assert!((editor.graph.nodes[node].plain(KnobId::Cutoff) - 1000.0).abs() < 0.1);
    }

    /// The box takes the keys, so a digit is not a piano note.
    #[test]
    fn the_box_keeps_the_keys_from_the_piano() {
        let (mut graph, _, at) = graph_with(NodeKind::Filter, KnobId::Cutoff);
        let mut editor = editor(&mut graph);
        double_click(&mut editor, at);
        let event = Event::Keyboard(keyboard::Event::KeyPressed {
            key_code: KeyCode::A,
            modifiers: Modifiers::default(),
        });
        let (status, message) = editor.update(event, bounds(), Cursor::Unavailable);
        assert_eq!(status, nih_plug_iced::event::Status::Captured);
        assert!(!matches!(message, Some(Message::Note(_))));
    }

    /// A knob a control wire drives is greyed out, and a double click does nothing to it.
    #[test]
    fn a_driven_knob_has_no_box() {
        let (mut graph, node, at) = graph_with(NodeKind::Filter, KnobId::Cutoff);
        let lfo = graph.add_node(NodeKind::Lfo, Point::new(40.0, 400.0));
        graph.connect(lfo, Port::Shape, node, Port::Knob(KnobId::Cutoff));
        assert!(graph.is_driven(node, KnobId::Cutoff));
        let mut editor = editor(&mut graph);
        double_click(&mut editor, at);
        assert!(editor.graph.knob_entry.is_none());
    }

    /// A macro knob is a host parameter, so setting it records one gesture for the host.
    #[test]
    fn a_macro_knob_reports_the_typed_value_to_the_host() {
        let (mut graph, _, at) = graph_with(NodeKind::Macro, KnobId::Macro1);
        let mut editor = editor(&mut graph);
        double_click(&mut editor, at);
        editor.graph.macro_edits.clear();
        type_text(&mut editor, "75");
        key(&mut editor, KeyCode::Enter);
        let edits = &editor.graph.macro_edits;
        assert_eq!(edits.first(), Some(&MacroEdit::Begin(0)));
        assert!(edits.contains(&MacroEdit::Set(0, 0.75)));
        assert_eq!(edits.last(), Some(&MacroEdit::End(0)));
    }

    /// Alt+click, which a double click used to do, puts a knob back to its default.
    #[test]
    fn alt_click_resets_a_knob() {
        let (mut graph, node, at) = graph_with(NodeKind::Filter, KnobId::Cutoff);
        graph.nodes[node].set_value(KnobId::Cutoff, 0.2);
        let mut editor = editor(&mut graph);
        let alt = Event::Keyboard(keyboard::Event::ModifiersChanged(Modifiers::ALT));
        editor.update(alt, bounds(), Cursor::Unavailable);
        click(&mut editor, at);
        assert_eq!(
            editor.graph.nodes[node].value(KnobId::Cutoff),
            KnobId::Cutoff.default_normalized()
        );
        assert!(editor.graph.knob_entry.is_none());
        // With Alt up, a click leaves the value alone.
        let up = Event::Keyboard(keyboard::Event::ModifiersChanged(Modifiers::default()));
        editor.update(up, bounds(), Cursor::Unavailable);
        editor.graph.nodes[node].set_value(KnobId::Cutoff, 0.2);
        click(&mut editor, at);
        assert_eq!(editor.graph.nodes[node].value(KnobId::Cutoff), 0.2);
    }

    /// A bipolar map's Center is typed as a percentage, and its Range is signed, so a negative one
    /// runs the swing backward.
    #[test]
    fn a_bipolar_maps_center_and_range_are_typed_as_percentages() {
        let mut graph = minimal_graph();
        let node = graph.add_node(NodeKind::Map, Point::new(300.0, 120.0));
        graph.toggle_map_polarity(node);
        graph.nodes[node].toggle_section(0);
        let at = |graph: &Graph, id: KnobId| {
            graph.nodes[node]
                .knob_slots()
                .into_iter()
                .find(|slot| slot.id == id)
                .expect("the section is open")
                .center
        };
        let (center_at, range_at) = (at(&graph, KnobId::MapCenter), at(&graph, KnobId::MapRange));
        let mut editor = editor(&mut graph);
        // A new map is the full swing, centered at 50 and 100 wide.
        double_click(&mut editor, center_at);
        assert_eq!(editor.graph.knob_entry.as_ref().unwrap().text, "50");
        type_text(&mut editor, "25");
        key(&mut editor, KeyCode::Enter);
        double_click(&mut editor, range_at);
        assert_eq!(editor.graph.knob_entry.as_ref().unwrap().text, "100");
        type_text(&mut editor, "-40");
        key(&mut editor, KeyCode::Enter);
        let node = &editor.graph.nodes[node];
        assert!((node.plain(KnobId::MapCenter) - 0.25).abs() < 1e-4);
        assert!((node.plain(KnobId::MapRange) + 0.4).abs() < 1e-4);
    }
}

#[cfg(test)]
mod menu_zoom_tests {
    use super::*;
    use crate::editor::canvas::NodeEditor;
    use crate::editor::menus::menu_bounds;
    use crate::editor::style::{MENU_ITEM_HEIGHT, MENU_WIDTH};
    use nih_plug_iced::mouse;
    use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};
    use nih_plug_iced::Vector;

    fn bounds() -> Rectangle {
        Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        }
    }

    fn mouse_at(graph: &mut Graph, button: mouse::Button, at: Point) {
        let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        editor.update(
            Event::Mouse(mouse::Event::ButtonPressed(button)),
            bounds(),
            Cursor::Available(at),
        );
    }

    /// Where a node's header sits on screen at the graph's current view.
    fn header_on_screen(graph: &Graph, node: usize) -> Point {
        let n = &graph.nodes[node];
        let world = Point::new(n.position.x + 80.0, n.position.y + 10.0);
        Point::new(
            world.x * graph.view.zoom + graph.view.offset.x,
            world.y * graph.view.zoom + graph.view.offset.y,
        )
    }

    /// The menu is as big, and where the click was, at every zoom and pan.
    #[test]
    fn the_menu_keeps_its_size_and_place_at_any_zoom() {
        let mut sizes = Vec::new();
        for (zoom, offset) in [
            (1.0, Vector::new(0.0, 0.0)),
            (0.4, Vector::new(30.0, 20.0)),
            (2.5, Vector::new(-500.0, -50.0)),
        ] {
            let mut graph = minimal_graph();
            graph.view.zoom = zoom;
            graph.view.offset = offset;
            let at = header_on_screen(&graph, 0);
            mouse_at(&mut graph, mouse::Button::Right, at);
            let menu = graph
                .menu
                .unwrap_or_else(|| panic!("no menu at zoom {zoom}, click {at:?}"));
            // The menu's corner is the click, in screen coordinates.
            assert_eq!(menu.position, at, "zoom {zoom}");
            let area = menu_bounds(menu);
            assert_eq!(area.width, MENU_WIDTH);
            sizes.push(area.height);
            assert_eq!(area.height, menu.items.len() as f32 * MENU_ITEM_HEIGHT);
        }
        assert!(sizes.windows(2).all(|pair| pair[0] == pair[1]));
    }

    /// Clicking an entry by where it is on screen works when zoomed and panned.
    #[test]
    fn an_entry_is_picked_by_its_screen_position() {
        let mut graph = minimal_graph();
        graph.view.zoom = 2.0;
        graph.view.offset = Vector::new(-100.0, -40.0);
        let nodes_before = graph.nodes.len();
        let at = header_on_screen(&graph, 0);
        mouse_at(&mut graph, mouse::Button::Right, at);
        let menu = graph.menu.expect("the menu opens");
        // Delete is the last entry. Its row on screen is the same height as at zoom 1.
        let area = menu_bounds(menu);
        let delete = Point::new(
            area.x + 10.0,
            area.y + (menu.items.len() as f32 - 0.5) * MENU_ITEM_HEIGHT,
        );
        mouse_at(&mut graph, mouse::Button::Left, delete);
        assert_eq!(graph.nodes.len(), nodes_before - 1);
        assert!(graph.menu.is_none());
    }

    /// A press on the menu is the menu's, wherever the world under it is.
    #[test]
    fn a_press_on_the_menu_does_not_reach_the_nodes_under_it() {
        let mut graph = minimal_graph();
        graph.view.zoom = 0.5;
        let at = header_on_screen(&graph, 0);
        mouse_at(&mut graph, mouse::Button::Right, at);
        let menu = graph.menu.expect("the menu opens");
        let area = menu_bounds(menu);
        let editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        let inside = Point::new(area.x + 5.0, area.y + 5.0);
        let world = editor.graph.view.to_world(inside);
        assert!(editor.overlay_at(world, inside, bounds()));
        let outside = Point::new(area.x + area.width + 40.0, area.y + 5.0);
        let world = editor.graph.view.to_world(outside);
        assert!(!editor.overlay_at(world, outside, bounds()));
    }
}

#[cfg(test)]
mod math_node_tests {
    use super::*;
    use crate::nodes::connection::Connection;
    use crate::nodes::math::MathOp;

    fn link(from: usize, out: Port, to: usize, input: Port) -> Connection {
        Connection {
            from,
            out,
            to,
            input,
        }
    }

    fn math(op: MathOp) -> FlodeNode {
        let mut node = FlodeNode::new(NodeKind::Math, Point::new(0.0, 0.0));
        node.choose(0, MathOp::ALL.iter().position(|&o| o == op).unwrap());
        node
    }

    /// The route to a knob through a math node, worked out at the given source levels.
    fn level(graph: &Graph, node: usize, id: KnobId, levels: ControlLevels) -> f32 {
        let wiring = graph.wiring();
        let route = wiring.control_route(node, id).expect("a route");
        route.apply(&wiring.with_signals(levels))
    }

    /// A math node with nothing wired in is a constant: its two knobs, combined. It is a source,
    /// so a knob can follow it.
    #[test]
    fn a_math_node_with_nothing_wired_is_a_constant() {
        let mut add = math(MathOp::Add);
        add.set_value(KnobId::MathA, 0.25);
        add.set_value(KnobId::MathB, 0.5);
        let nodes = vec![add, FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0))];
        let links = vec![link(0, Port::Shape, 1, Port::Knob(KnobId::Cutoff))];
        let graph = Graph::with(nodes, links);
        assert!(graph.is_driven(1, KnobId::Cutoff));
        let at = level(&graph, 1, KnobId::Cutoff, ControlLevels::default());
        assert!((at - 0.75).abs() < 1e-6, "{at}");
    }

    /// An LFO into A and a constant in B: the node combines the live signal with the knob.
    #[test]
    fn a_driver_in_a_and_a_constant_in_b() {
        let mut multiply = math(MathOp::Multiply);
        multiply.set_value(KnobId::MathB, 0.5);
        let nodes = vec![
            FlodeNode::new(NodeKind::Lfo, Point::new(0.0, 0.0)),
            multiply,
            FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0)),
        ];
        let links = vec![
            link(0, Port::Shape, 1, Port::Knob(KnobId::MathA)),
            link(1, Port::Shape, 2, Port::Knob(KnobId::Cutoff)),
        ];
        let graph = Graph::with(nodes, links);
        // Only A is driven.
        assert!(graph.is_driven(1, KnobId::MathA));
        assert!(!graph.is_driven(1, KnobId::MathB));
        for lfo in [0.0, 0.4, 1.0] {
            let levels = ControlLevels {
                lfo: [lfo; MAX_LFOS],
                ..ControlLevels::default()
            };
            let at = level(&graph, 2, KnobId::Cutoff, levels);
            assert!((at - lfo * 0.5).abs() < 1e-6, "lfo {lfo}: {at}");
        }
    }

    /// Two different drivers into the two inputs.
    #[test]
    fn two_drivers_combine() {
        let nodes = vec![
            FlodeNode::new(NodeKind::Lfo, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Envelope, Point::new(0.0, 0.0)),
            math(MathOp::Maximum),
            FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0)),
        ];
        let links = vec![
            link(0, Port::Shape, 2, Port::Knob(KnobId::MathA)),
            link(1, Port::Shape, 2, Port::Knob(KnobId::MathB)),
            link(2, Port::Shape, 3, Port::Knob(KnobId::Cutoff)),
        ];
        let graph = Graph::with(nodes, links);
        let levels = |lfo: f32, envelope: f32| ControlLevels {
            lfo: [lfo; MAX_LFOS],
            envelope: [envelope; MAX_ENVELOPES],
            ..ControlLevels::default()
        };
        assert!((level(&graph, 3, KnobId::Cutoff, levels(0.3, 0.7)) - 0.7).abs() < 1e-6);
        assert!((level(&graph, 3, KnobId::Cutoff, levels(0.9, 0.2)) - 0.9).abs() < 1e-6);
    }

    /// The chosen operation reaches the compiled node, and a change to the dropdown changes the
    /// signal.
    #[test]
    fn the_dropdown_picks_the_operation_that_runs() {
        for op in MathOp::ALL {
            let mut node = math(op);
            node.set_value(KnobId::MathA, 0.75);
            node.set_value(KnobId::MathB, 0.25);
            let nodes = vec![node, FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0))];
            let links = vec![link(0, Port::Shape, 1, Port::Knob(KnobId::Cutoff))];
            let graph = Graph::with(nodes, links);
            let at = level(&graph, 1, KnobId::Cutoff, ControlLevels::default());
            assert!((at - op.apply(0.75, 0.25)).abs() < 1e-6, "{op:?}: {at}");
        }
    }

    /// Math nodes chain with each other and with maps, in any order in the graph. The nodes are
    /// listed with the last link in the chain first, so the table must put them in running order.
    #[test]
    fn math_and_map_nodes_chain_in_any_order() {
        // Filter <- map (inverts) <- math (A + B) <- lfo and a constant.
        let mut invert = FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0));
        invert.set_value(KnobId::MapMin, 1.0);
        invert.set_value(KnobId::MapMax, 0.0);
        let mut add = math(MathOp::Add);
        add.set_value(KnobId::MathB, 0.25);
        let nodes = vec![
            invert,
            add,
            FlodeNode::new(NodeKind::Lfo, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0)),
        ];
        let links = vec![
            link(1, Port::Shape, 0, Port::Shape),
            link(2, Port::Shape, 1, Port::Knob(KnobId::MathA)),
            link(0, Port::Shape, 3, Port::Knob(KnobId::Cutoff)),
        ];
        let graph = Graph::with(nodes, links);
        let table = graph.patch().signals;
        assert_eq!(table.len(), 2);
        for lfo in [0.0, 0.25, 0.5] {
            let levels = ControlLevels {
                lfo: [lfo; MAX_LFOS],
                ..ControlLevels::default()
            };
            let at = level(&graph, 3, KnobId::Cutoff, levels);
            // The math node gives lfo + 0.25, and the map inverts it.
            assert!((at - (1.0 - (lfo + 0.25))).abs() < 1e-6, "lfo {lfo}: {at}");
        }
    }

    /// A math node can drive a map's own values, such as its Offset, and a map can drive a math
    /// node's input.
    #[test]
    fn math_drives_a_map_value_and_a_map_drives_math() {
        // Lfo -> math (Power) A. The math output drives a map's Offset. An envelope runs through
        // the map into a knob.
        let nodes = vec![
            FlodeNode::new(NodeKind::Lfo, Point::new(0.0, 0.0)),
            math(MathOp::Minimum),
            FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Envelope, Point::new(0.0, 0.0)),
            FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0)),
        ];
        let links = vec![
            link(0, Port::Shape, 1, Port::Knob(KnobId::MathA)),
            link(1, Port::Shape, 2, Port::Knob(KnobId::MapOffset)),
            link(3, Port::Shape, 2, Port::Shape),
            link(2, Port::Shape, 4, Port::Knob(KnobId::Cutoff)),
        ];
        let graph = Graph::with(nodes, links);
        assert!(graph.is_driven(2, KnobId::MapOffset));
        let at = |lfo: f32| {
            let levels = ControlLevels {
                lfo: [lfo; MAX_LFOS],
                envelope: [0.2; MAX_ENVELOPES],
                ..ControlLevels::default()
            };
            level(&graph, 4, KnobId::Cutoff, levels)
        };
        // Min(lfo, 0.5) is the offset's source. At 0.5 the offset is 0, and above it stays 0.
        assert!((at(0.5) - 0.2).abs() < 1e-6);
        assert!((at(1.0) - 0.2).abs() < 1e-6);
        // At lfo 0.25 the source is 0.25, so the offset is 0.25 * 2 - 1 = -0.5, which floors at 0.
        assert_eq!(at(0.25), 0.0);
    }

    /// Two math nodes feeding each other do not hang the compiler. The loop is cut where it closes:
    /// the node that would read itself back reads its own knob in place of that wire, so both
    /// nodes still make a signal.
    #[test]
    fn math_nodes_in_a_loop_are_cut_where_it_closes() {
        let nodes = vec![
            math(MathOp::Add),
            math(MathOp::Add),
            FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0)),
        ];
        let links = vec![
            link(0, Port::Shape, 1, Port::Knob(KnobId::MathA)),
            link(1, Port::Shape, 0, Port::Knob(KnobId::MathA)),
            link(0, Port::Shape, 2, Port::Knob(KnobId::Cutoff)),
        ];
        let graph = Graph::with(nodes, links);
        assert_eq!(graph.patch().signals.len(), 2);
        // Node 1 compiles first, with its A held at its knob, since node 0 is mid-compile when
        // node 1 asks for it. Node 0 then reads node 1.
        let nodes: Vec<_> = graph.patch().signals.nodes().copied().collect();
        let driven = |node: &crate::patch::control::SignalNode| match node {
            crate::patch::control::SignalNode::Math(math) => math.a.source,
            _ => panic!("not a math node"),
        };
        assert_eq!(driven(&nodes[0]), None);
        assert_eq!(driven(&nodes[1]), Some(Source::Signal(0)));
        // The cutoff follows node 0, which is the second in the table.
        let route = graph.control_route(2, KnobId::Cutoff).unwrap();
        assert_eq!(route.source, Source::Signal(1));
    }

    /// The node saves and loads, with its operation and constants.
    #[test]
    fn a_math_node_survives_saving() {
        let mut graph = minimal_graph();
        let node = graph.add_node(NodeKind::Math, Point::new(300.0, 200.0));
        graph.nodes[node].choose(0, 4);
        graph.nodes[node].set_value(KnobId::MathB, 0.8);
        let loaded = Graph::from_saved(&graph.to_saved());
        let loaded = &loaded.nodes[node];
        assert_eq!(loaded.kind, NodeKind::Math);
        assert_eq!(crate::nodes::math::operation(loaded), MathOp::Power);
        assert!((loaded.value(KnobId::MathB) - 0.8).abs() < 1e-6);
    }

    /// A graph of the kinds in `kinds`, none of them wired.
    fn graph_of(kinds: &[NodeKind]) -> Graph {
        let nodes = kinds
            .iter()
            .map(|&kind| FlodeNode::new(kind, Point::new(0.0, 0.0)))
            .collect();
        Graph::with(nodes, Vec::new())
    }

    /// Auto connect wires a driver into the math node's first free input, and the second drag
    /// takes the other one.
    #[test]
    fn auto_connect_wires_a_driver_into_a_math_input() {
        let mut graph = graph_of(&[NodeKind::Lfo, NodeKind::Envelope, NodeKind::Math]);
        assert!(graph.auto_connect(0, 2));
        assert!(graph.is_driven(2, KnobId::MathA));
        assert!(!graph.is_driven(2, KnobId::MathB));
        assert!(graph.auto_connect(1, 2));
        assert!(graph.is_driven(2, KnobId::MathB));
    }

    /// Dragging from the math node to a source that takes no control wire connects the other way
    /// round, as it does for any pair. The macro node has outputs and no knob pins.
    #[test]
    fn auto_connect_works_from_the_math_node_too() {
        let mut graph = graph_of(&[NodeKind::Macro, NodeKind::Math]);
        assert!(graph.auto_connect(1, 0));
        assert!(graph.is_driven(1, KnobId::MathA));
    }

    /// Dragging from the math node to a node that has knob pins wires the math output forward into
    /// the first of them, since that way is valid.
    #[test]
    fn auto_connect_from_the_math_node_prefers_its_own_output() {
        let mut graph = graph_of(&[NodeKind::Lfo, NodeKind::Math]);
        assert!(graph.auto_connect(1, 0));
        assert!(graph
            .connections
            .iter()
            .any(|c| c.from == 1 && c.to == 0 && c.out == Port::Shape));
    }

    /// The math node's output goes to a knob of another node, and to a map's input.
    #[test]
    fn auto_connect_wires_the_math_output_onward() {
        let mut graph = graph_of(&[NodeKind::Math, NodeKind::Filter]);
        assert!(graph.auto_connect(0, 1));
        // The first free knob of the filter takes it.
        assert!(graph.is_driven(1, KnobId::Cutoff));

        let mut graph = graph_of(&[NodeKind::Math, NodeKind::Map]);
        assert!(graph.auto_connect(0, 1));
        assert!(graph
            .connections
            .iter()
            .any(|c| c.from == 0 && c.to == 1 && c.input == Port::Shape));
    }

    /// A map into a math node, and a math node into another.
    #[test]
    fn auto_connect_joins_math_with_maps_and_other_math() {
        // A map has nothing feeding it here, so it makes no signal. The wire is still made.
        let mut graph = graph_of(&[NodeKind::Map, NodeKind::Math]);
        assert!(graph.auto_connect(0, 1));
        assert!(graph
            .connections
            .iter()
            .any(|c| { c.from == 0 && c.to == 1 && c.input == Port::Knob(KnobId::MathA) }));

        let mut graph = graph_of(&[NodeKind::Math, NodeKind::Math]);
        assert!(graph.auto_connect(0, 1));
        assert!(graph.is_driven(1, KnobId::MathA));
    }

    /// Audio does not go to a math node. An oscillator's audio never ends on a math input. The
    /// only wire the pair can make is the math output into one of the oscillator's knobs.
    #[test]
    fn auto_connect_never_sends_audio_into_a_math_node() {
        let mut graph = graph_of(&[NodeKind::Oscillator, NodeKind::Math]);
        graph.auto_connect(0, 1);
        assert!(graph
            .connections
            .iter()
            .all(|c| c.from == 1 && c.to == 0 && c.out == Port::Shape));
    }

    /// Dropping a math node on a wire that drives a knob splices it in: the old source feeds A, and
    /// the node's output drives the knob. The signal then passes through the node.
    #[test]
    fn a_math_node_dropped_on_a_control_wire_is_spliced_in() {
        let mut graph = minimal_graph();
        let env = graph.add_node(NodeKind::Envelope, Point::new(0.0, 0.0));
        graph.connect(env, Port::Shape, 1, Port::Knob(KnobId::Gain));
        let link = graph.connections.len() - 1;
        let curve = graph.link_curve(&graph.connections[link]).unwrap();
        let middle = bezier_point(&curve, 0.5);
        let node = graph.add_node(NodeKind::Math, Point::new(middle.x - 80.0, middle.y - 20.0));
        graph.nodes[node].choose(
            0,
            MathOp::ALL
                .iter()
                .position(|&o| o == MathOp::Multiply)
                .unwrap(),
        );
        graph.nodes[node].set_value(KnobId::MathB, 0.5);

        assert_eq!(graph.link_under(node), Some(link));
        graph.insert_into(node, link);
        // The envelope feeds A, and the node feeds the gain.
        assert_eq!(graph.connections[link].to, node);
        assert_eq!(graph.connections[link].input, Port::Knob(KnobId::MathA));
        let onward = graph.connections.last().unwrap();
        assert_eq!((onward.from, onward.to), (node, 1));
        assert_eq!(onward.input, Port::Knob(KnobId::Gain));
        assert!(graph.is_driven(node, KnobId::MathA));
        assert!(!graph.is_driven(node, KnobId::MathB));
        // The envelope times one half.
        let levels = ControlLevels {
            envelope: [0.6; MAX_ENVELOPES],
            ..ControlLevels::default()
        };
        let at = level(&graph, 1, KnobId::Gain, levels);
        assert!((at - 0.3).abs() < 1e-6, "{at}");
    }

    /// A math node has no audio, so it does not splice into an audio wire. A node that already has
    /// a wire does not splice either.
    #[test]
    fn a_math_node_does_not_splice_into_audio_or_when_wired() {
        let mut graph = minimal_graph();
        let curve = graph.link_curve(&graph.connections[0]).unwrap();
        let middle = bezier_point(&curve, 0.5);
        let node = graph.add_node(NodeKind::Math, Point::new(middle.x - 80.0, middle.y - 20.0));
        assert_eq!(graph.link_under(node), None);

        let env = graph.add_node(NodeKind::Envelope, Point::new(0.0, 0.0));
        graph.connect(env, Port::Shape, 1, Port::Knob(KnobId::Gain));
        let link = graph.connections.len() - 1;
        let curve = graph.link_curve(&graph.connections[link]).unwrap();
        let middle = bezier_point(&curve, 0.5);
        graph.nodes[node].position = Point::new(middle.x - 80.0, middle.y - 20.0);
        assert_eq!(graph.link_under(node), Some(link));
        graph.connect(env, Port::Shape, node, Port::Knob(KnobId::MathB));
        assert_eq!(graph.link_under(node), None);
    }

    /// The node is in the add menu with Modulation, and takes no audio.
    #[test]
    fn the_math_node_is_in_the_modulation_menu() {
        assert!(Category::Modulation.kinds().contains(&NodeKind::Math));
        let node = FlodeNode::new(NodeKind::Math, Point::new(0.0, 0.0));
        assert!(!node.kind.behavior().audio_in() && !node.kind.behavior().audio_out());
        // The knobs take control wires and the output makes one.
        assert_eq!(node.output_ports(), vec![Port::Shape]);
        assert!(node.output_ports()[0].is_control_out());
    }
}

#[cfg(test)]
mod map_polarity_tests {
    use super::*;
    use crate::editor::canvas::NodeEditor;
    use crate::nodes::connection::Connection;
    use crate::nodes::map;
    use nih_plug_iced::mouse;
    use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};

    fn link(from: usize, out: Port, to: usize, input: Port) -> Connection {
        Connection {
            from,
            out,
            to,
            input,
        }
    }

    fn level(graph: &Graph, node: usize, id: KnobId, levels: ControlLevels) -> f32 {
        let wiring = graph.wiring();
        let route = wiring.control_route(node, id).expect("a route");
        route.apply(&wiring.with_signals(levels))
    }

    /// An envelope through a map into the filter's cutoff.
    fn envelope_map_filter() -> Graph {
        // The nodes sit apart, so a click on one cannot land on another.
        let nodes = vec![
            FlodeNode::new(NodeKind::Envelope, Point::new(0.0, 100.0)),
            FlodeNode::new(NodeKind::Map, Point::new(300.0, 100.0)),
            FlodeNode::new(NodeKind::Filter, Point::new(600.0, 100.0)),
        ];
        let links = vec![
            link(0, Port::Shape, 1, Port::Shape),
            link(1, Port::Shape, 2, Port::Knob(KnobId::Cutoff)),
        ];
        Graph::with(nodes, links)
    }

    fn at_envelope(graph: &Graph, envelope: f32) -> f32 {
        let levels = ControlLevels {
            envelope: [envelope; MAX_ENVELOPES],
            ..ControlLevels::default()
        };
        level(graph, 2, KnobId::Cutoff, levels)
    }

    /// A 30 percent range centered at 50 gives 35 to 65 percent, as the five-node chain did.
    #[test]
    fn a_bipolar_map_is_a_range_around_a_center() {
        let mut graph = envelope_map_filter();
        graph.toggle_map_polarity(1);
        graph.nodes[1].set_value(KnobId::MapCenter, KnobId::MapCenter.to_normalized(0.5));
        graph.nodes[1].set_value(KnobId::MapRange, KnobId::MapRange.to_normalized(0.3));
        assert!((at_envelope(&graph, 0.0) - 0.35).abs() < 1e-5);
        assert!((at_envelope(&graph, 1.0) - 0.65).abs() < 1e-5);
        assert!((at_envelope(&graph, 0.5) - 0.5).abs() < 1e-5);
        // Moving the center slides the whole swing, and the range stays the same width.
        graph.nodes[1].set_value(KnobId::MapCenter, KnobId::MapCenter.to_normalized(0.7));
        assert!((at_envelope(&graph, 0.0) - 0.55).abs() < 1e-5);
        assert!((at_envelope(&graph, 1.0) - 0.85).abs() < 1e-5);
    }

    /// A negative range runs the swing backward.
    #[test]
    fn a_negative_range_inverts() {
        let mut graph = envelope_map_filter();
        graph.toggle_map_polarity(1);
        graph.nodes[1].set_value(KnobId::MapRange, KnobId::MapRange.to_normalized(-1.0));
        assert!((at_envelope(&graph, 0.0) - 1.0).abs() < 1e-5);
        assert!(at_envelope(&graph, 1.0).abs() < 1e-5);
    }

    /// Toggling leaves the signal as it was, in both directions, through the editor.
    #[test]
    fn toggling_keeps_the_signal() {
        let mut graph = envelope_map_filter();
        graph.nodes[1].set_value(KnobId::MapMin, KnobId::MapMin.to_normalized(0.2));
        graph.nodes[1].set_value(KnobId::MapMax, KnobId::MapMax.to_normalized(0.6));
        let samples = [0.0, 0.3, 0.7, 1.0];
        let before: Vec<f32> = samples.iter().map(|&e| at_envelope(&graph, e)).collect();
        graph.toggle_map_polarity(1);
        assert!(map::bipolar(&graph.nodes[1]));
        let bipolar: Vec<f32> = samples.iter().map(|&e| at_envelope(&graph, e)).collect();
        graph.toggle_map_polarity(1);
        assert!(!map::bipolar(&graph.nodes[1]));
        let after: Vec<f32> = samples.iter().map(|&e| at_envelope(&graph, e)).collect();
        for i in 0..samples.len() {
            assert!((before[i] - bipolar[i]).abs() < 1e-5, "{i}");
            assert!((before[i] - after[i]).abs() < 1e-5, "{i}");
        }
    }

    /// The wires of the pair that leaves go with it, and the other pair's wires stay.
    #[test]
    fn toggling_drops_the_wires_of_the_pair_that_leaves() {
        let mut graph = envelope_map_filter();
        let lfo = graph.add_node(NodeKind::Lfo, Point::new(0.0, 0.0));
        graph.connect(lfo, Port::Shape, 1, Port::Knob(KnobId::MapMin));
        graph.connect(lfo, Port::Shape, 1, Port::Knob(KnobId::MapInMin));
        assert!(graph.is_driven(1, KnobId::MapMin));
        graph.toggle_map_polarity(1);
        // The Out min wire is gone. In min's stays.
        assert!(!graph
            .connections
            .iter()
            .any(|c| c.to == 1 && c.input == Port::Knob(KnobId::MapMin)));
        assert!(graph.is_driven(1, KnobId::MapInMin));
        // Every wire that is left still ends on a pin that exists.
        assert!(graph.connections.iter().all(|c| c.is_valid(&graph.nodes)));
        graph.connect(lfo, Port::Shape, 1, Port::Knob(KnobId::MapCenter));
        graph.toggle_map_polarity(1);
        assert!(!graph
            .connections
            .iter()
            .any(|c| c.to == 1 && c.input == Port::Knob(KnobId::MapCenter)));
        assert!(graph.connections.iter().all(|c| c.is_valid(&graph.nodes)));
    }

    /// A source on Range moves the swing's width live, and a source on Center moves its middle.
    #[test]
    fn center_and_range_can_be_driven() {
        let mut graph = envelope_map_filter();
        let lfo = graph.add_node(NodeKind::Lfo, Point::new(0.0, 0.0));
        graph.toggle_map_polarity(1);
        graph.connect(lfo, Port::Shape, 1, Port::Knob(KnobId::MapRange));
        assert!(graph.is_driven(1, KnobId::MapRange));
        let levels = |lfo_level: f32| ControlLevels {
            envelope: [1.0; MAX_ENVELOPES],
            lfo: [lfo_level; MAX_LFOS],
            ..ControlLevels::default()
        };
        // The range follows the source from -1 to +1, so the top of the swing is the center plus
        // half of it. At 0.5 the range is 0, so the output is the center. At 1.0 it is 100 wide,
        // and the top reaches 1. At 0.75 it is 50 wide, and the top is 0.75.
        assert!((level(&graph, 2, KnobId::Cutoff, levels(0.5)) - 0.5).abs() < 1e-5);
        assert!((level(&graph, 2, KnobId::Cutoff, levels(1.0)) - 1.0).abs() < 1e-5);
        assert!((level(&graph, 2, KnobId::Cutoff, levels(0.75)) - 0.75).abs() < 1e-5);
        // At 0.0 the range is -100, so a full input gives the bottom.
        assert!(level(&graph, 2, KnobId::Cutoff, levels(0.0)).abs() < 1e-5);

        graph.connect(lfo, Port::Shape, 1, Port::Knob(KnobId::MapCenter));
        assert!(graph.is_driven(1, KnobId::MapCenter));
    }

    /// The hidden Out min and Out max are not read while bipolar, even if they hold other values.
    #[test]
    fn the_hidden_ends_are_ignored_while_bipolar() {
        let mut graph = envelope_map_filter();
        graph.toggle_map_polarity(1);
        graph.nodes[1].set_value(KnobId::MapMin, 0.9);
        graph.nodes[1].set_value(KnobId::MapMax, 0.9);
        assert!((at_envelope(&graph, 0.0) - 0.0).abs() < 1e-5);
        assert!((at_envelope(&graph, 1.0) - 1.0).abs() < 1e-5);
    }

    /// A bipolar map has no Offset: its pin goes with the wire on it, and a value the knob still
    /// holds does not reach the signal.
    #[test]
    fn a_bipolar_map_has_no_offset() {
        let mut graph = envelope_map_filter();
        let lfo = graph.add_node(NodeKind::Lfo, Point::new(0.0, 0.0));
        graph.connect(lfo, Port::Shape, 1, Port::Knob(KnobId::MapOffset));
        assert!(graph.is_driven(1, KnobId::MapOffset));
        graph.toggle_map_polarity(1);
        assert!(!graph
            .connections
            .iter()
            .any(|c| c.to == 1 && c.input == Port::Knob(KnobId::MapOffset)));
        assert!(!graph.nodes[1]
            .input_ports()
            .contains(&Port::Knob(KnobId::MapOffset)));
        // A value left in the knob is not applied.
        graph.nodes[1].set_value(KnobId::MapOffset, KnobId::MapOffset.to_normalized(0.4));
        assert!(at_envelope(&graph, 0.0).abs() < 1e-5);
        assert!((at_envelope(&graph, 1.0) - 1.0).abs() < 1e-5);
    }

    /// Going bipolar folds the offset into Center, so the signal stays what it was.
    #[test]
    fn going_bipolar_folds_the_offset_into_center() {
        let mut graph = envelope_map_filter();
        graph.nodes[1].set_value(KnobId::MapMax, KnobId::MapMax.to_normalized(0.5));
        graph.nodes[1].set_value(KnobId::MapOffset, KnobId::MapOffset.to_normalized(0.25));
        let samples = [0.0, 0.5, 1.0];
        let before: Vec<f32> = samples.iter().map(|&e| at_envelope(&graph, e)).collect();
        assert!((before[0] - 0.25).abs() < 1e-5 && (before[2] - 0.75).abs() < 1e-5);
        graph.toggle_map_polarity(1);
        // The swing from 25 to 75 is centered at 50 and 50 wide.
        assert!((graph.nodes[1].plain(KnobId::MapCenter) - 0.5).abs() < 1e-5);
        assert!((graph.nodes[1].plain(KnobId::MapRange) - 0.5).abs() < 1e-5);
        for (i, &e) in samples.iter().enumerate() {
            assert!((at_envelope(&graph, e) - before[i]).abs() < 1e-5, "{e}");
        }
    }

    /// Dragging a handle on a bipolar map changes Center and Range, and the end under the cursor
    /// follows it while the other end stays.
    #[test]
    fn dragging_a_handle_sets_center_and_range() {
        use crate::editor::curve::{drag_map_handle, map_handles, MapDriven};
        use crate::nodes::viewer::Live;
        let mut node = FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0));
        map::set_bipolar(&mut node, true);
        let bounds = node.curve_bounds();
        // Drag the low end up to 20 percent, with the high end at 100.
        let target = Point::new(bounds.x + 10.0, bounds.y + bounds.height * 0.8);
        drag_map_handle(&mut node, 0, target, 0.0, MapDriven::default());
        let (low, high) = map::ends_of(node.plain(KnobId::MapCenter), node.plain(KnobId::MapRange));
        assert!(
            (low - 0.2).abs() < 1e-4 && (high - 1.0).abs() < 1e-4,
            "{low} {high}"
        );
        assert!((node.plain(KnobId::MapCenter) - 0.6).abs() < 1e-4);
        assert!((node.plain(KnobId::MapRange) - 0.8).abs() < 1e-4);
        // The handle is where it was dragged to.
        let handle = map_handles(&node, &Live::of(&node))[0];
        assert!((handle.y - target.y).abs() < 1e-2);
        // The hidden ends were not touched.
        assert_eq!(node.plain(KnobId::MapMin), 0.0);
        assert_eq!(node.plain(KnobId::MapMax), 1.0);
        // A wire on Center or Range holds the output still.
        let before = (node.value(KnobId::MapCenter), node.value(KnobId::MapRange));
        let up = Point::new(bounds.x + 10.0, bounds.y + 2.0);
        drag_map_handle(
            &mut node,
            1,
            up,
            0.0,
            MapDriven {
                input: false,
                output: true,
            },
        );
        assert_eq!(
            before,
            (node.value(KnobId::MapCenter), node.value(KnobId::MapRange))
        );
    }

    /// Clicking the Bipolar checkbox through the editor converts the values and swaps the knobs.
    #[test]
    fn clicking_the_checkbox_toggles_the_polarity() {
        let mut graph = envelope_map_filter();
        graph.nodes[1].set_value(KnobId::MapMin, KnobId::MapMin.to_normalized(0.2));
        graph.nodes[1].set_value(KnobId::MapMax, KnobId::MapMax.to_normalized(0.6));
        let checkbox = graph.nodes[1].switch_bounds(0);
        let at = Point::new(checkbox.x + 6.0, checkbox.y + checkbox.height / 2.0);
        let click = |graph: &mut Graph| {
            let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
            let bounds = Rectangle {
                x: 0.0,
                y: 0.0,
                width: 1024.0,
                height: 712.0,
            };
            editor.update(
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                bounds,
                Cursor::Available(at),
            );
            editor.update(
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                bounds,
                Cursor::Available(at),
            );
        };
        click(&mut graph);
        assert!(map::bipolar(&graph.nodes[1]));
        assert!((graph.nodes[1].plain(KnobId::MapCenter) - 0.4).abs() < 1e-5);
        assert!((graph.nodes[1].plain(KnobId::MapRange) - 0.4).abs() < 1e-5);
        click(&mut graph);
        assert!(!map::bipolar(&graph.nodes[1]));
        assert!((graph.nodes[1].plain(KnobId::MapMin) - 0.2).abs() < 1e-5);
        assert!((graph.nodes[1].plain(KnobId::MapMax) - 0.6).abs() < 1e-5);
    }

    /// A bipolar map saves and loads, with its wires on Center and Range intact.
    #[test]
    fn a_bipolar_map_survives_saving() {
        let mut graph = envelope_map_filter();
        let lfo = graph.add_node(NodeKind::Lfo, Point::new(0.0, 0.0));
        graph.toggle_map_polarity(1);
        graph.nodes[1].set_value(KnobId::MapCenter, KnobId::MapCenter.to_normalized(0.3));
        graph.connect(lfo, Port::Shape, 1, Port::Knob(KnobId::MapRange));
        let loaded = Graph::from_saved(&graph.to_saved());
        assert!(map::bipolar(&loaded.nodes[1]));
        assert!((loaded.nodes[1].plain(KnobId::MapCenter) - 0.3).abs() < 1e-5);
        assert!(loaded.is_driven(1, KnobId::MapRange));
        assert_eq!(loaded.connections.len(), graph.connections.len());
    }
}

mod quick_add_flow_tests {
    use super::*;
    use crate::editor::canvas::NodeEditor;
    use nih_plug_iced::keyboard::{self, KeyCode, Modifiers};
    use nih_plug_iced::mouse;
    use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};

    fn bounds() -> Rectangle {
        Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        }
    }

    fn key(graph: &mut Graph, key_code: KeyCode, modifiers: Modifiers) {
        let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        editor.update(
            Event::Keyboard(keyboard::Event::KeyPressed {
                key_code,
                modifiers,
            }),
            bounds(),
            Cursor::Unavailable,
        );
        editor.update(
            Event::Keyboard(keyboard::Event::KeyReleased {
                key_code,
                modifiers,
            }),
            bounds(),
            Cursor::Unavailable,
        );
    }

    fn typed(graph: &mut Graph, text: &str) {
        let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        for c in text.chars() {
            editor.update(
                Event::Keyboard(keyboard::Event::CharacterReceived(c)),
                bounds(),
                Cursor::Unavailable,
            );
        }
    }

    fn open(graph: &mut Graph) {
        graph.last_screen = Point::new(300.0, 300.0);
        key(graph, KeyCode::A, Modifiers::SHIFT);
        // The A that opened the box also arrives as a typed character.
        typed(graph, "A");
    }

    /// Shift+A opens the box with nothing selected, and the opening A is not typed into it.
    #[test]
    fn shift_a_opens_an_empty_box() {
        let mut graph = Graph::default();
        open(&mut graph);
        let box_state = graph.quick_add.as_ref().expect("box is open");
        assert!(box_state.query.is_empty());
    }

    /// Typing a name and pressing Enter adds that node in placement mode.
    #[test]
    fn enter_adds_the_top_result_in_placement_mode() {
        let mut graph = Graph::default();
        let before = graph.nodes.len();
        open(&mut graph);
        typed(&mut graph, "reverb");
        key(&mut graph, KeyCode::Enter, Modifiers::default());
        assert!(graph.quick_add.is_none());
        assert_eq!(graph.nodes.len(), before + 1);
        assert_eq!(graph.nodes[before].kind, NodeKind::Reverb);
        assert!(matches!(
            graph.interaction,
            Interaction::Placing { node, .. } if node == before
        ));
    }

    /// The arrow keys move the lit result, so Enter adds a different one.
    #[test]
    fn arrows_pick_another_result() {
        let mut graph = Graph::default();
        open(&mut graph);
        typed(&mut graph, "fl");
        let results = graph.quick_add_results();
        assert!(results.len() >= 2);
        key(&mut graph, KeyCode::Down, Modifiers::default());
        key(&mut graph, KeyCode::Enter, Modifiers::default());
        assert_eq!(graph.nodes.last().map(|n| n.kind), Some(results[1]));
    }

    /// Escape closes the box and adds nothing.
    #[test]
    fn escape_closes_without_adding() {
        let mut graph = Graph::default();
        let before = graph.nodes.len();
        open(&mut graph);
        typed(&mut graph, "lfo");
        key(&mut graph, KeyCode::Escape, Modifiers::default());
        assert!(graph.quick_add.is_none());
        assert_eq!(graph.nodes.len(), before);
    }

    /// Clicking a result adds its node in placement mode.
    #[test]
    fn clicking_a_result_adds_it() {
        let mut graph = Graph::default();
        let before = graph.nodes.len();
        open(&mut graph);
        typed(&mut graph, "mix");
        let open_box = graph.quick_add.clone().expect("open");
        let rows = graph.quick_add_results().len();
        let area = quick_add_bounds(bounds(), open_box.screen, rows);
        let at = Point::new(area.x + 20.0, area.y + QUICK_ADD_FIELD_HEIGHT + 10.0);
        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        editor.update(
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds(),
            Cursor::Available(at),
        );
        assert_eq!(graph.nodes.len(), before + 1);
        assert!(matches!(graph.interaction, Interaction::Placing { .. }));
    }

    /// A click outside the box closes it and adds nothing.
    #[test]
    fn clicking_outside_closes_it() {
        let mut graph = Graph::default();
        let before = graph.nodes.len();
        open(&mut graph);
        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        editor.update(
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds(),
            Cursor::Available(Point::new(900.0, 150.0)),
        );
        assert!(graph.quick_add.is_none());
        assert_eq!(graph.nodes.len(), before);
    }

    /// A kind the graph holds one of is not offered again.
    #[test]
    fn a_second_output_is_not_offered() {
        let mut graph = Graph::default();
        graph.add_node(NodeKind::Output, Point::new(0.0, 0.0));
        open(&mut graph);
        typed(&mut graph, "output");
        assert!(!graph.quick_add_results().contains(&NodeKind::Output));
    }
}

#[cfg(test)]
mod monitor_tests {
    use super::*;
    use crate::dsp::monitor::{MonitorData, MonitorMode};
    use crate::editor::canvas::NodeEditor;
    use crate::editor::monitor::{draw_screen, MonitorHistory};
    use crate::nodes::monitor;
    use crate::nodes::{Control, MONITOR_HEIGHT, MONITOR_ROW};
    use nih_plug_iced::mouse;
    use nih_plug_iced::widget::canvas::{event::Event, Cursor, Frame, Program};
    use std::time::Instant;

    const BOUNDS: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 1024.0,
        height: 712.0,
    };

    fn monitor_in(mode: MonitorMode) -> FlodeNode {
        let mut node = FlodeNode::new(NodeKind::Monitor, Point::new(300.0, 200.0));
        node.choose(0, mode as usize);
        node
    }

    #[test]
    fn the_screen_sits_inside_the_node_below_the_dropdown() {
        let node = monitor_in(MonitorMode::Scope);
        let screen = node.monitor_bounds().expect("a monitor has a screen");
        let bounds = node.bounds();
        assert_eq!(screen.height, MONITOR_HEIGHT);
        assert!(screen.x >= bounds.x && screen.x + screen.width <= bounds.x + bounds.width);
        assert!(screen.y + screen.height <= bounds.y + bounds.height);
        let dropdown = node.dropdowns().remove(0).bounds;
        assert!(dropdown.y + dropdown.height <= screen.y);
        // The screen adds its row to the node's height.
        let rows = node.rows();
        assert!(rows.iter().any(|row| row.control == Some(Control::Monitor)));
        assert!(bounds.height > MONITOR_ROW);
    }

    #[test]
    fn a_collapsed_monitor_hides_its_screen() {
        let mut node = monitor_in(MonitorMode::Scope);
        node.collapsed = true;
        assert!(node.monitor_bounds().is_none());
    }

    #[test]
    fn a_monitor_has_no_output_and_one_audio_input() {
        let node = monitor_in(MonitorMode::Scope);
        assert_eq!(node.input_ports(), vec![Port::Audio]);
        assert!(node.output_ports().is_empty());
    }

    #[test]
    fn saving_and_loading_keeps_every_mode() {
        for mode in MonitorMode::ALL {
            let mut graph = Graph::with(vec![monitor_in(mode)], Vec::new());
            graph.refresh_slots();
            let loaded = Graph::from_saved(&graph.to_saved());
            assert_eq!(loaded.nodes.len(), 1);
            assert_eq!(loaded.nodes[0].kind, NodeKind::Monitor);
            assert_eq!(monitor::mode(&loaded.nodes[0]), mode, "{mode:?}");
        }
    }

    #[test]
    fn the_quick_add_box_and_the_utility_list_offer_the_monitor() {
        assert!(NodeKind::ALL.contains(&NodeKind::Monitor));
        assert!(Category::Utility.kinds().contains(&NodeKind::Monitor));
        let found = quick_add_results("monit", |_| true);
        assert_eq!(found, vec![NodeKind::Monitor]);
    }

    /// An oscillator at node 0 and a monitor at node 1, next to it.
    fn oscillator_and_monitor() -> Graph {
        Graph::with(
            vec![
                FlodeNode::new(NodeKind::Oscillator, Point::new(40.0, 80.0)),
                FlodeNode::new(NodeKind::Monitor, Point::new(500.0, 80.0)),
            ],
            Vec::new(),
        )
    }

    #[test]
    fn an_audio_wire_ends_on_the_monitor_pin() {
        let mut graph = oscillator_and_monitor();
        graph.connect(0, Port::Audio, 1, Port::Audio);
        assert_eq!(graph.connections.len(), 1);
        assert_eq!(graph.connections[0].to, 1);
        let mut other = oscillator_and_monitor();
        assert!(other.auto_connect(0, 1));
        assert_eq!(other.connections[0].input, Port::Audio);
        // Dragging from the monitor still finds the oscillator.
        let mut backward = oscillator_and_monitor();
        assert!(backward.auto_connect(1, 0));
        assert_eq!(
            (backward.connections[0].from, backward.connections[0].to),
            (0, 1)
        );
    }

    #[test]
    fn a_monitor_dropped_on_a_wire_is_not_spliced() {
        let mut graph = minimal_graph();
        let curve = graph.link_curve(&graph.connections[0]).unwrap();
        let middle = bezier_point(&curve, 0.5);
        let node = graph.add_node(
            NodeKind::Monitor,
            Point::new(middle.x - 100.0, middle.y - 40.0),
        );
        assert_eq!(graph.link_under(node), None);
    }

    #[test]
    fn pressing_the_screen_selects_the_node_like_its_body() {
        let mut graph = oscillator_and_monitor();
        let screen = graph.nodes[1].monitor_bounds().unwrap();
        let start = Point::new(
            screen.x + screen.width / 2.0,
            screen.y + screen.height / 2.0,
        );
        let before = graph.nodes[1].position;
        let send = |graph: &mut Graph, event: mouse::Event, at: Point| {
            let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
            editor.update(Event::Mouse(event), BOUNDS, Cursor::Available(at));
        };
        send(
            &mut graph,
            mouse::Event::ButtonPressed(mouse::Button::Left),
            start,
        );
        let to = Point::new(start.x - 30.0, start.y + 40.0);
        send(&mut graph, mouse::Event::CursorMoved { position: to }, to);
        send(
            &mut graph,
            mouse::Event::ButtonReleased(mouse::Button::Left),
            to,
        );
        // A press on a node's body selects it and does nothing else, as on any other node. Only
        // the header drags.
        assert_eq!(graph.nodes[1].position, before);
        assert_eq!(graph.selected, Some(1));
        assert!(matches!(graph.interaction, Interaction::Idle));
        assert!(graph.dropdown_menu.is_none());
    }

    #[test]
    fn a_monitor_with_no_wiring_has_no_data_and_draws_the_hint() {
        let graph = oscillator_and_monitor();
        assert!(graph.monitor_data(1).is_none());
        // The whole editor draws without a panic.
        draw_editor(&graph, ControlLevels::default(), [0; 2], [0.0; 2], BOUNDS);
    }

    /// A measurement with something in every field.
    fn busy_data() -> MonitorData {
        let mut data = MonitorData::default();
        for (i, sample) in data.trace.iter_mut().enumerate() {
            let phase = i as f32 / 20.0;
            *sample = [phase.sin(), (phase * 1.5).cos() * 0.6];
        }
        for (i, db) in data.spectrum.iter_mut().enumerate() {
            *db = -90.0 + (i as f32 * 0.7).sin().abs() * 80.0;
        }
        data.levels = [-0.05, -6.0, -14.0, -20.0];
        data.correlation = -0.4;
        data
    }

    #[test]
    fn every_mode_draws_with_and_without_data() {
        let screen = monitor_in(MonitorMode::Scope).monitor_bounds().unwrap();
        let data = busy_data();
        let now = Instant::now();
        for mode in MonitorMode::ALL {
            let mut history = MonitorHistory::new(0, mode);
            for i in 0..300 {
                history.feed(
                    mode,
                    Some(&data),
                    now + std::time::Duration::from_millis(40 * i),
                );
            }
            for (data, history) in [
                (Some(&data), Some(&history)),
                (Some(&data), None),
                (None, None),
                (Some(&MonitorData::default()), Some(&history)),
            ] {
                let mut frame = Frame::new(BOUNDS.size());
                draw_screen(&mut frame, screen, mode, data, history, &[], 1.0);
                // And under an overlay that hides the text.
                let mut frame = Frame::new(BOUNDS.size());
                draw_screen(&mut frame, screen, mode, data, history, &[BOUNDS], 1.0);
            }
        }
    }

    #[test]
    fn deleting_a_node_renumbers_the_monitor_histories() {
        let mut graph = Graph::with(
            vec![
                FlodeNode::new(NodeKind::Oscillator, Point::new(0.0, 0.0)),
                monitor_in(MonitorMode::Levels),
                monitor_in(MonitorMode::Spectrogram),
            ],
            Vec::new(),
        );
        graph.monitor_history = vec![
            MonitorHistory::new(1, MonitorMode::Levels),
            MonitorHistory::new(2, MonitorMode::Spectrogram),
        ];
        graph.delete_node(1);
        assert_eq!(graph.monitor_history.len(), 1);
        assert_eq!(graph.monitor_history[0].node, 1);
        graph.delete_node(0);
        assert_eq!(graph.monitor_history[0].node, 0);
        graph.delete_node(0);
        assert!(graph.monitor_history.is_empty());
    }

    #[test]
    fn updating_creates_a_history_only_for_the_time_modes() {
        let mut graph = Graph::with(
            vec![
                monitor_in(MonitorMode::Scope),
                monitor_in(MonitorMode::Levels),
            ],
            Vec::new(),
        );
        graph.update_monitors(Instant::now());
        assert_eq!(graph.monitor_history.len(), 1);
        assert_eq!(graph.monitor_history[0].node, 1);
        // The history stays empty while the node has no data to record.
        assert_eq!(graph.monitor_history[0].levels.len(), 0);
    }
}

mod back_wiring_tests {
    use super::*;
    use crate::editor::canvas::NodeEditor;
    use nih_plug_iced::keyboard::{self, KeyCode, Modifiers};
    use nih_plug_iced::mouse;
    use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};

    fn bounds() -> Rectangle {
        Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        }
    }

    fn mouse_at(graph: &mut Graph, event: mouse::Event, at: Point) {
        let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        editor.update(Event::Mouse(event), bounds(), Cursor::Available(at));
    }

    fn press(graph: &mut Graph, at: Point) {
        mouse_at(graph, mouse::Event::ButtonPressed(mouse::Button::Left), at);
    }

    fn release(graph: &mut Graph, at: Point) {
        mouse_at(graph, mouse::Event::ButtonReleased(mouse::Button::Left), at);
    }

    fn key(graph: &mut Graph, key_code: KeyCode) {
        let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        editor.update(
            Event::Keyboard(keyboard::Event::KeyPressed {
                key_code,
                modifiers: Modifiers::default(),
            }),
            bounds(),
            Cursor::Unavailable,
        );
    }

    /// A graph with a filter at the right and an oscillator at the left, and nothing wired.
    fn graph_with_filter() -> (Graph, usize, usize) {
        let mut graph = Graph::default();
        graph.nodes.clear();
        graph.connections.clear();
        let osc = graph.add_node(NodeKind::Oscillator, Point::new(120.0, 300.0));
        let filter = graph.add_node(NodeKind::Filter, Point::new(520.0, 300.0));
        graph.clear_selection();
        (graph, osc, filter)
    }

    /// Dragging from a free input onto an output wires the two.
    #[test]
    fn dragging_back_to_an_output_wires_it() {
        let (mut graph, osc, filter) = graph_with_filter();
        let input = graph.nodes[filter].input_point(Port::Audio).unwrap();
        let output = graph.nodes[osc].output_point(Port::Audio).unwrap();
        press(&mut graph, input);
        assert!(matches!(graph.interaction, Interaction::BackWiring { .. }));
        release(&mut graph, output);
        assert_eq!(graph.connections.len(), 1);
        let link = graph.connections[0];
        assert_eq!((link.from, link.to, link.input), (osc, filter, Port::Audio));
        assert!(matches!(graph.interaction, Interaction::Idle));
    }

    /// Dropping on an output of the wrong kind of signal wires nothing.
    #[test]
    fn a_mismatched_output_does_not_wire() {
        let (mut graph, _, filter) = graph_with_filter();
        let lfo = graph.add_node(NodeKind::Lfo, Point::new(120.0, 500.0));
        let input = graph.nodes[filter].input_point(Port::Audio).unwrap();
        let output = graph.nodes[lfo].output_point(Port::Shape).unwrap();
        press(&mut graph, input);
        release(&mut graph, output);
        assert!(graph.connections.is_empty());
        assert!(graph.quick_add.is_none());
    }

    /// Dropping on empty canvas opens the quick add box, which lists only what can feed the input.
    #[test]
    fn dropping_on_empty_canvas_offers_the_nodes_that_can_feed() {
        let (mut graph, _, filter) = graph_with_filter();
        let input = graph.nodes[filter].input_point(Port::Audio).unwrap();
        press(&mut graph, input);
        release(&mut graph, Point::new(300.0, 520.0));
        let open = graph.quick_add.clone().expect("the box opens");
        assert_eq!(open.feed, Some((filter, Port::Audio)));
        let kinds = graph.quick_add_results();
        assert!(kinds.contains(&NodeKind::Reverb));
        assert!(!kinds.contains(&NodeKind::Lfo));
        assert!(!kinds.contains(&NodeKind::Output));
        assert!(!kinds.contains(&NodeKind::Monitor));
    }

    /// A control input lists the control sources, not the audio nodes.
    #[test]
    fn a_knob_input_lists_control_sources() {
        let (mut graph, _, filter) = graph_with_filter();
        let pin = Port::Knob(KnobId::Cutoff);
        let input = graph.nodes[filter].input_point(pin).unwrap();
        press(&mut graph, input);
        release(&mut graph, Point::new(300.0, 520.0));
        let kinds = graph.quick_add_results();
        assert!(kinds.contains(&NodeKind::Lfo));
        assert!(kinds.contains(&NodeKind::Math));
        assert!(!kinds.contains(&NodeKind::Oscillator));
    }

    /// Picking a node adds it, wires it to the input, and leaves it following the cursor.
    #[test]
    fn picking_a_node_adds_and_wires_it() {
        let (mut graph, _, filter) = graph_with_filter();
        let before = graph.nodes.len();
        let input = graph.nodes[filter].input_point(Port::Audio).unwrap();
        press(&mut graph, input);
        release(&mut graph, Point::new(300.0, 520.0));
        for c in "reverb".chars() {
            let mut editor =
                NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
            editor.update(
                Event::Keyboard(keyboard::Event::CharacterReceived(c)),
                bounds(),
                Cursor::Unavailable,
            );
        }
        key(&mut graph, KeyCode::Enter);
        assert_eq!(graph.nodes.len(), before + 1);
        let added = graph.nodes.len() - 1;
        assert_eq!(graph.nodes[added].kind, NodeKind::Reverb);
        assert!(matches!(
            graph.interaction,
            Interaction::Placing { node, .. } if node == added
        ));
        let link = graph.connections.last().copied().expect("a wire");
        assert_eq!(
            (link.from, link.to, link.input),
            (added, filter, Port::Audio)
        );
    }

    /// Escape closes the box and leaves the graph as it was.
    #[test]
    fn escape_leaves_the_graph_alone() {
        let (mut graph, _, filter) = graph_with_filter();
        let before = graph.nodes.len();
        let input = graph.nodes[filter].input_point(Port::Audio).unwrap();
        press(&mut graph, input);
        release(&mut graph, Point::new(300.0, 520.0));
        key(&mut graph, KeyCode::Escape);
        assert!(graph.quick_add.is_none());
        assert_eq!(graph.nodes.len(), before);
        assert!(graph.connections.is_empty());
    }

    /// An input that has a wire still detaches it, as before.
    #[test]
    fn a_wired_input_still_detaches() {
        let (mut graph, osc, filter) = graph_with_filter();
        graph.connect(osc, Port::Audio, filter, Port::Audio);
        let input = graph.nodes[filter].input_point(Port::Audio).unwrap();
        press(&mut graph, input);
        assert!(matches!(graph.interaction, Interaction::Wiring { .. }));
        assert!(graph.connections.is_empty());
    }

    /// The quick add box a plain Shift+A opens has no input to feed, so it lists everything that
    /// can be added.
    #[test]
    fn a_plain_quick_add_is_not_filtered() {
        let (mut graph, _, _) = graph_with_filter();
        graph.open_quick_add();
        assert_eq!(graph.quick_add.as_ref().and_then(|open| open.feed), None);
        assert!(!graph.quick_add_results().is_empty());
    }
}

mod monitor_wiring_tests {
    use super::*;

    /// The starting graph, plus a monitor listening to its oscillator.
    fn graph_with_monitor() -> (Graph, usize) {
        let mut graph = Graph::default();
        let osc = graph
            .nodes
            .iter()
            .position(|node| node.kind == NodeKind::Oscillator)
            .expect("the starting graph has an oscillator");
        let monitor = graph.add_node(NodeKind::Monitor, Point::new(600.0, 400.0));
        graph.connect(osc, Port::Audio, monitor, Port::Audio);
        (graph, monitor)
    }

    /// A monitor wired to something that plays gets an instance, and the editor finds its data.
    #[test]
    fn a_wired_monitor_finds_its_instance() {
        let (graph, monitor) = graph_with_monitor();
        assert!(graph.monitor_data(monitor).is_some());
    }

    /// A monitor with nothing wired to it has no instance.
    #[test]
    fn an_unwired_monitor_has_none() {
        let mut graph = Graph::default();
        let monitor = graph.add_node(NodeKind::Monitor, Point::new(600.0, 400.0));
        assert!(graph.monitor_data(monitor).is_none());
    }

    /// The monitor does not change the sound path. The output still has its one chain.
    #[test]
    fn the_monitor_leaves_the_output_chain_alone() {
        let (graph, _) = graph_with_monitor();
        let patch = graph.wiring().patch();
        assert!(patch.lanes[0].is_active());
    }
}

#[cfg(test)]
mod lfo_curve_tests {
    use super::*;
    use crate::dsp::curve::{Curve, MAX_CURVE_POINTS, MIN_GAP};
    use crate::editor::canvas::{Message, NodeEditor};
    use crate::nodes::lfo::{self, LfoShape};
    use crate::presets::{self, Preset};
    use nih_plug_iced::mouse;
    use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};

    const BOUNDS: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 1024.0,
        height: 712.0,
    };

    /// Where the Wave dropdown's Custom row is.
    const CUSTOM: usize = 5;

    fn lfo_node() -> FlodeNode {
        let mut node = FlodeNode::new(NodeKind::Lfo, Point::new(300.0, 200.0));
        node.choose(0, CUSTOM);
        node
    }

    fn graph_of(node: FlodeNode) -> Graph {
        Graph::with(vec![node], Vec::new())
    }

    /// Where curve position `(x, y)` is on screen for the node.
    fn spot(node: &FlodeNode, x: f32, y: f32) -> Point {
        let bounds = node.viewer_bounds().expect("the viewer is on show");
        Point::new(
            bounds.x + x * bounds.width,
            bounds.y + (1.0 - y) * bounds.height,
        )
    }

    fn send(graph: &mut Graph, event: mouse::Event, at: Point) -> Option<Message> {
        let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        editor
            .update(Event::Mouse(event), BOUNDS, Cursor::Available(at))
            .1
    }

    fn press(graph: &mut Graph, at: Point) -> Option<Message> {
        send(graph, mouse::Event::ButtonPressed(mouse::Button::Left), at)
    }

    fn release(graph: &mut Graph, at: Point) -> Option<Message> {
        send(graph, mouse::Event::ButtonReleased(mouse::Button::Left), at)
    }

    fn click(graph: &mut Graph, at: Point) {
        press(graph, at);
        release(graph, at);
    }

    fn double_click(graph: &mut Graph, at: Point) {
        click(graph, at);
        click(graph, at);
    }

    fn move_to(graph: &mut Graph, to: Point) {
        send(graph, mouse::Event::CursorMoved { position: to }, to);
    }

    fn points(graph: &Graph) -> Vec<[f32; 2]> {
        lfo::curve(&graph.nodes[0]).points().to_vec()
    }

    #[test]
    fn the_wave_dropdown_ends_with_custom_and_keeps_the_old_rows() {
        let node = FlodeNode::new(NodeKind::Lfo, Point::new(0.0, 0.0));
        let labels = node.dropdowns()[0].labels;
        assert_eq!(
            labels,
            ["Sine", "Saw", "Square", "Triangle", "Noise", "Custom"]
        );
        for (row, wave) in Waveform::ALL.iter().enumerate() {
            let mut node = FlodeNode::new(NodeKind::Lfo, Point::new(0.0, 0.0));
            node.choose(0, row);
            assert_eq!(lfo::shape(&node), LfoShape::Wave(*wave));
            assert_eq!(lfo::waveform(&node), *wave);
        }
        // The oscillator's list does not change.
        let osc = FlodeNode::new(NodeKind::Oscillator, Point::new(0.0, 0.0));
        assert_eq!(osc.dropdowns()[0].labels.len(), 5);
        assert_eq!(lfo::shape(&lfo_node()), LfoShape::Custom);
    }

    #[test]
    fn the_node_has_a_one_shot_switch_after_retrigger() {
        let node = FlodeNode::new(NodeKind::Lfo, Point::new(0.0, 0.0));
        assert_eq!(node.switch_labels(), &["Retrigger", "One shot"]);
        assert!(node.switch_shown(1));
    }

    #[test]
    fn a_new_lfo_has_the_default_curve() {
        let graph = graph_of(lfo_node());
        assert_eq!(points(&graph), Curve::default().points());
    }

    #[test]
    fn handles_sit_on_the_curve_points() {
        let node = lfo_node();
        let handles = lfo_handles(&node).expect("a custom LFO has handles");
        assert_eq!(handles.len(), 3);
        assert_eq!(handles[0], spot(&node, 0.0, 0.0));
        assert_eq!(handles[1], spot(&node, 0.5, 1.0));
        assert_eq!(handles[2], spot(&node, 1.0, 0.0));
    }

    #[test]
    fn a_shape_that_is_not_custom_has_no_handles() {
        let mut node = lfo_node();
        let at = spot(&node, 0.5, 1.0);
        assert_eq!(lfo_handle_at(&node, at), Some(1));
        node.choose(0, 1);
        assert!(lfo_handles(&node).is_none());
        assert_eq!(lfo_handle_at(&node, at), None);
    }

    #[test]
    fn a_collapsed_lfo_has_no_handles() {
        let mut node = lfo_node();
        node.collapsed = true;
        assert!(lfo_handles(&node).is_none());
    }

    #[test]
    fn other_nodes_have_no_lfo_handles() {
        let mut node = FlodeNode::new(NodeKind::Envelope, Point::new(300.0, 200.0));
        node.choices = vec![CUSTOM];
        assert!(lfo_handles(&node).is_none());
    }

    #[test]
    fn dragging_a_point_moves_it() {
        let mut graph = graph_of(lfo_node());
        let start = spot(&graph.nodes[0], 0.5, 1.0);
        press(&mut graph, start);
        assert_eq!(graph.selected, Some(0));
        assert_eq!(
            graph.interaction,
            Interaction::LfoPoint { node: 0, point: 1 }
        );
        let to = spot(&graph.nodes[0], 0.3, 0.4);
        move_to(&mut graph, to);
        let moved = points(&graph)[1];
        assert!((moved[0] - 0.3).abs() < 1e-4 && (moved[1] - 0.4).abs() < 1e-4);
        assert!(matches!(
            release(&mut graph, to),
            Some(Message::GraphChanged)
        ));
        assert_eq!(graph.interaction, Interaction::Idle);
        assert_eq!(points(&graph).len(), 3);
    }

    #[test]
    fn a_drag_stops_at_the_neighbours_and_the_box() {
        let mut node = lfo_node();
        let mut curve = lfo::curve(&node);
        curve.insert(0.25, 0.5);
        lfo::set_curve(&mut node, &curve);
        let mut graph = graph_of(node);
        // The middle point is index 2 now. Dragging it far left stops short of its neighbour.
        let start = spot(&graph.nodes[0], 0.5, 1.0);
        press(&mut graph, start);
        let far_left = spot(&graph.nodes[0], -3.0, 5.0);
        move_to(&mut graph, far_left);
        let point = points(&graph)[2];
        assert!((point[0] - (0.25 + MIN_GAP)).abs() < 1e-4, "{point:?}");
        assert_eq!(point[1], 1.0);
        let far_right = spot(&graph.nodes[0], 4.0, -5.0);
        move_to(&mut graph, far_right);
        let point = points(&graph)[2];
        assert!((point[0] - (1.0 - MIN_GAP)).abs() < 1e-4, "{point:?}");
        assert_eq!(point[1], 0.0);
        release(&mut graph, far_right);
        assert_eq!(points(&graph).len(), 4);
    }

    #[test]
    fn an_end_point_keeps_its_x() {
        let mut graph = graph_of(lfo_node());
        for (x, index) in [(0.0, 0), (1.0, 2)] {
            let start = spot(&graph.nodes[0], x, 0.0);
            press(&mut graph, start);
            assert_eq!(
                graph.interaction,
                Interaction::LfoPoint {
                    node: 0,
                    point: index
                }
            );
            let to = spot(&graph.nodes[0], 0.4, 0.6);
            move_to(&mut graph, to);
            release(&mut graph, to);
            let end = points(&graph)[index];
            assert!(end[0] == x && (end[1] - 0.6).abs() < 1e-4, "{end:?}");
        }
    }

    #[test]
    fn a_double_click_on_empty_space_adds_a_point_there() {
        let mut graph = graph_of(lfo_node());
        let at = spot(&graph.nodes[0], 0.25, 0.2);
        click(&mut graph, at);
        assert_eq!(points(&graph).len(), 3);
        press(&mut graph, at);
        assert_eq!(points(&graph).len(), 4);
        let added = points(&graph)[1];
        assert!((added[0] - 0.25).abs() < 1e-4 && (added[1] - 0.2).abs() < 1e-4);
        // No drag is left running, so the release and a move change nothing.
        assert_eq!(graph.interaction, Interaction::Idle);
        release(&mut graph, at);
        let away = spot(&graph.nodes[0], 0.9, 0.9);
        move_to(&mut graph, away);
        assert_eq!(points(&graph)[1], added);
    }

    #[test]
    fn a_single_press_on_empty_space_adds_nothing() {
        let mut graph = graph_of(lfo_node());
        let at = spot(&graph.nodes[0], 0.25, 0.2);
        press(&mut graph, at);
        release(&mut graph, at);
        assert_eq!(points(&graph).len(), 3);
        // The press still selects the node, as a press on its body does.
        assert_eq!(graph.selected, Some(0));
        assert_eq!(graph.interaction, Interaction::Idle);
        // Two slow presses are two single presses. Another spot far away is not a double click.
        let elsewhere = spot(&graph.nodes[0], 0.8, 0.8);
        click(&mut graph, elsewhere);
        assert_eq!(points(&graph).len(), 3);
    }

    #[test]
    fn a_double_click_on_an_inner_point_removes_it() {
        let mut graph = graph_of(lfo_node());
        let at = spot(&graph.nodes[0], 0.25, 0.2);
        double_click(&mut graph, at);
        assert_eq!(points(&graph).len(), 4);
        let inner = spot(&graph.nodes[0], 0.25, 0.2);
        press(&mut graph, inner);
        assert_eq!(
            graph.interaction,
            Interaction::LfoPoint { node: 0, point: 1 }
        );
        release(&mut graph, inner);
        click(&mut graph, inner);
        assert_eq!(points(&graph).len(), 3);
        assert_eq!(points(&graph), Curve::default().points());
        assert_eq!(graph.interaction, Interaction::Idle);
    }

    #[test]
    fn a_double_click_on_an_end_point_does_nothing() {
        let mut graph = graph_of(lfo_node());
        for x in [0.0, 1.0] {
            let at = spot(&graph.nodes[0], x, 0.0);
            click(&mut graph, at);
            press(&mut graph, at);
            assert_eq!(graph.interaction, Interaction::Idle);
            release(&mut graph, at);
            assert_eq!(points(&graph), Curve::default().points());
        }
    }

    #[test]
    fn there_is_no_adding_past_sixteen_points() {
        let mut node = lfo_node();
        node.points = (0..MAX_CURVE_POINTS)
            .map(|n| [n as f32 / (MAX_CURVE_POINTS - 1) as f32, 0.9])
            .collect();
        let mut graph = graph_of(node);
        assert_eq!(points(&graph).len(), MAX_CURVE_POINTS);
        assert!(graph.status.is_none());
        let before = points(&graph);
        let at = spot(&graph.nodes[0], 0.52, 0.1);
        double_click(&mut graph, at);
        assert_eq!(points(&graph), before);
        let (text, _) = graph.status.clone().expect("the status line says so");
        assert!(text.contains("16"), "{text}");
    }

    #[test]
    fn a_double_click_does_nothing_when_the_wave_is_not_custom() {
        let mut node = lfo_node();
        node.choose(0, 0);
        let mut graph = graph_of(node);
        let at = spot(&graph.nodes[0], 0.25, 0.2);
        double_click(&mut graph, at);
        let on_point = spot(&graph.nodes[0], 0.5, 1.0);
        double_click(&mut graph, on_point);
        assert!(graph.nodes[0].points.is_empty());
        assert_eq!(graph.interaction, Interaction::Idle);
        assert!(graph.status.is_none());
    }

    #[test]
    fn an_edit_asks_for_the_patch_to_be_published() {
        let mut graph = graph_of(lfo_node());
        let at = spot(&graph.nodes[0], 0.25, 0.2);
        click(&mut graph, at);
        assert!(matches!(press(&mut graph, at), Some(Message::GraphChanged)));
        release(&mut graph, at);
        let handle = spot(&graph.nodes[0], 0.5, 1.0);
        assert!(matches!(
            press(&mut graph, handle),
            Some(Message::GraphChanged)
        ));
        let to = spot(&graph.nodes[0], 0.5, 0.5);
        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        let moved = editor.update(
            Event::Mouse(mouse::Event::CursorMoved { position: to }),
            BOUNDS,
            Cursor::Available(to),
        );
        assert!(matches!(moved.1, Some(Message::GraphChanged)));
    }

    #[test]
    fn choosing_custom_keeps_the_points() {
        let mut node = FlodeNode::new(NodeKind::Lfo, Point::new(300.0, 200.0));
        node.points = vec![[0.0, 0.1], [0.4, 0.9], [1.0, 0.5]];
        node.choose(0, 2);
        node.choose(0, CUSTOM);
        assert_eq!(node.points, vec![[0.0, 0.1], [0.4, 0.9], [1.0, 0.5]]);
        node.choose(0, 1);
        assert_eq!(node.points.len(), 3);
    }

    #[test]
    fn the_curve_and_one_shot_reach_the_patch() {
        let mut graph = minimal_graph();
        let index = graph.add_node(NodeKind::Lfo, Point::new(0.0, 200.0));
        let settings = graph.patch().lfos[0].unwrap();
        assert!(!settings.one_shot);
        assert_eq!(settings.curve, None);

        graph.nodes[index].points = vec![[0.0, 0.2], [0.5, 0.8], [1.0, 0.4]];
        // Still a built-in wave, so the points are not sent.
        assert_eq!(graph.patch().lfos[0].unwrap().curve, None);

        graph.nodes[index].choose(0, CUSTOM);
        graph.nodes[index].set_switch(1, true);
        let settings = graph.patch().lfos[0].unwrap();
        assert!(settings.one_shot);
        assert!(!settings.retrigger);
        assert_eq!(
            settings.curve,
            Curve::from_points(&[[0.0, 0.2], [0.5, 0.8], [1.0, 0.4]])
        );

        // Custom with no points of its own sends the default.
        graph.nodes[index].points.clear();
        assert_eq!(graph.patch().lfos[0].unwrap().curve, Some(Curve::default()));
    }

    #[test]
    fn a_duplicate_keeps_the_points() {
        let mut graph = graph_of(lfo_node());
        let at = spot(&graph.nodes[0], 0.25, 0.2);
        double_click(&mut graph, at);
        let copy = graph.duplicate_node(0).expect("an LFO can be copied");
        assert_eq!(graph.nodes[copy].points, graph.nodes[0].points);
        assert_eq!(graph.nodes[copy].points.len(), 4);
        // Editing the copy leaves the original alone.
        graph.nodes[copy].points.clear();
        assert_eq!(graph.nodes[0].points.len(), 4);
    }

    #[test]
    fn saving_and_loading_keeps_the_points() {
        let mut graph = graph_of(lfo_node());
        let at = spot(&graph.nodes[0], 0.25, 0.2);
        double_click(&mut graph, at);
        let loaded = Graph::from_saved(&graph.to_saved());
        assert_eq!(loaded.nodes[0].points, graph.nodes[0].points);
        assert_eq!(lfo::shape(&loaded.nodes[0]), LfoShape::Custom);
        // Through the JSON a project or preset holds.
        let json = serde_json::to_string(&graph.to_saved()).unwrap();
        let saved: SavedGraph = serde_json::from_str(&json).unwrap();
        assert_eq!(Graph::from_saved(&saved).nodes[0].points.len(), 4);
    }

    #[test]
    fn a_preset_keeps_the_points() {
        let dir = std::env::temp_dir().join(format!("flode-lfo-curve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut graph = graph_of(lfo_node());
        graph.nodes[0].points = vec![[0.0, 0.3], [0.2, 1.0], [0.7, 0.0], [1.0, 0.6]];
        let preset = Preset {
            name: "Curve".to_string(),
            category: String::new(),
            graph: graph.to_saved(),
            macros: [0.0; 4],
        };
        presets::save_in(&dir, &preset).unwrap();
        let mut fresh = graph_of(FlodeNode::new(NodeKind::Lfo, Point::new(0.0, 0.0)));
        fresh.preset_dir = Some(dir.clone());
        assert!(fresh.load_preset("", "Curve"));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            fresh.nodes[0].points,
            vec![[0.0, 0.3], [0.2, 1.0], [0.7, 0.0], [1.0, 0.6]]
        );
        assert_eq!(lfo::shape(&fresh.nodes[0]), LfoShape::Custom);
    }

    /// The JSON of a one node graph, with `edit` applied to its first node.
    fn load_with(edit: impl Fn(&mut serde_json::Map<String, serde_json::Value>)) -> Graph {
        let mut value = serde_json::to_value(graph_of(lfo_node()).to_saved()).unwrap();
        let node = value["nodes"][0].as_object_mut().unwrap();
        edit(node);
        let saved: SavedGraph = serde_json::from_value(value).unwrap();
        Graph::from_saved(&saved)
    }

    #[test]
    fn a_save_without_points_loads_the_default_curve() {
        let graph = load_with(|node| {
            node.remove("points");
        });
        assert!(graph.nodes[0].points.is_empty());
        assert_eq!(lfo::curve(&graph.nodes[0]), Curve::default());
        assert_eq!(graph.patch().lfos[0].unwrap().curve, Some(Curve::default()));
    }

    #[test]
    fn points_that_make_no_curve_load_as_the_default() {
        for bad in [
            serde_json::json!([[0.0, 0.0]]),
            serde_json::json!([[0.0, 0.0], [0.6, 1.0], [0.4, 0.0], [1.0, 0.0]]),
            serde_json::json!([[0.0, 0.0], [0.5, 1.5], [1.0, 0.0]]),
            serde_json::json!([[0.1, 0.0], [1.0, 0.0]]),
            serde_json::json!((0..17).map(|n| [n as f32 / 16.0, 0.5]).collect::<Vec<_>>()),
        ] {
            let graph = load_with(|node| {
                node.insert("points".to_string(), bad.clone());
            });
            assert!(graph.nodes[0].points.is_empty(), "{bad}");
            assert_eq!(lfo::curve(&graph.nodes[0]), Curve::default());
        }
    }
}

#[cfg(test)]
mod gain_node_tests {
    use super::*;
    use crate::nodes::{gain, Bypass, Control};

    /// An oscillator, a gain node, and the output, wired in that order.
    fn chain() -> (Graph, usize) {
        let mut graph = minimal_graph();
        graph.connections.clear();
        let osc = add_oscillator(&mut graph, Point::new(0.0, 0.0));
        let node = graph.add_node(NodeKind::Gain, Point::new(0.0, 0.0));
        graph.connect(osc, Port::Audio, node, Port::Audio);
        graph.connect(node, Port::Audio, 1, Port::Slot(0));
        graph.refresh_slots();
        (graph, node)
    }

    #[test]
    fn a_new_gain_node_has_the_expected_rows_and_defaults() {
        let node = FlodeNode::new(NodeKind::Gain, Point::new(0.0, 0.0));
        let rows = node.rows();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1].knob, Some(KnobId::UtilGain));
        assert_eq!(rows[1].input, Some(Port::Knob(KnobId::UtilGain)));
        assert_eq!(rows[2].knob, Some(KnobId::Pan));
        assert_eq!(rows[2].input, Some(Port::Knob(KnobId::Pan)));
        assert_eq!(rows[3].control, Some(Control::Switch(0)));
        assert_eq!(node.switch_labels(), &["Invert"]);
        assert_eq!(node.plain(KnobId::UtilGain), 0.0);
        assert_eq!(node.plain(KnobId::Pan), 0.0);
        assert!(!node.switch(0));
        assert_eq!(
            node.input_ports(),
            vec![
                Port::Audio,
                Port::Knob(KnobId::UtilGain),
                Port::Knob(KnobId::Pan)
            ]
        );
        assert_eq!(node.output_ports(), vec![Port::Audio]);
        assert_eq!(node.bypass(), Bypass::PassThrough);
    }

    #[test]
    fn the_settings_reach_the_patch() {
        let (mut graph, node) = chain();
        graph.nodes[node].set_value(KnobId::UtilGain, KnobId::UtilGain.to_normalized(-6.0));
        graph.nodes[node].set_value(KnobId::Pan, KnobId::Pan.to_normalized(0.5));
        graph.nodes[node].set_switch(0, true);
        let lane = graph.patch().lanes[0].clone();
        let StageParams::Gain(params) = lane.stages[1].params else {
            panic!("not a gain stage");
        };
        let levels = ControlLevels::default();
        assert!((params.gain.plain(&levels) + 6.0).abs() < 1e-3);
        assert!((params.pan.plain(&levels) - 0.5).abs() < 1e-3);
        assert!(params.invert);
        assert!(!lane.stages[1].bus);
    }

    #[test]
    fn saving_and_loading_keeps_the_settings() {
        let (mut graph, node) = chain();
        graph.nodes[node].set_value(KnobId::UtilGain, KnobId::UtilGain.to_normalized(12.0));
        graph.nodes[node].set_value(KnobId::Pan, KnobId::Pan.to_normalized(-0.25));
        graph.nodes[node].set_switch(0, true);
        let json = serde_json::to_string(&graph.to_saved()).unwrap();
        let loaded = Graph::from_saved(&serde_json::from_str::<SavedGraph>(&json).unwrap());
        let node = loaded
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Gain)
            .expect("the gain node loads");
        assert!((node.plain(KnobId::UtilGain) - 12.0).abs() < 1e-3);
        assert!((node.plain(KnobId::Pan) + 0.25).abs() < 1e-3);
        assert!(gain::invert(node));
    }

    #[test]
    fn the_utility_menu_and_the_quick_add_box_offer_the_gain_node() {
        assert!(NodeKind::ALL.contains(&NodeKind::Gain));
        assert!(Category::Utility.kinds().contains(&NodeKind::Gain));
        assert_eq!(quick_add_results("gain", |_| true), vec![NodeKind::Gain]);
    }
}

/// The Sampler's file loading: the dialog, the background decode, save and reload by path, and a
/// missing or broken file's graceful failure.
mod sampler_ui_tests {
    use super::*;
    use crate::dsp::sample::SampleData;
    use crate::nodes::sampler;
    use std::sync::Arc;

    /// A folder for one test's sample files, removed when the test ends.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("flode-sampler-ui-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Writes a short, valid mono WAV file at `path`.
    fn write_wav(path: &std::path::Path) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..100 {
            writer.write_sample((i * 100) as i16).unwrap();
        }
        writer.finalize().unwrap();
    }

    fn sampler_graph() -> (Graph, usize) {
        let mut graph = minimal_graph();
        let node = graph.add_node(NodeKind::Sampler, Point::new(0.0, 0.0));
        (graph, node)
    }

    /// Spins on `poll_sampler_loads` until the background decode thread has answered, or panics
    /// if it takes too long. A tiny file decodes almost instantly, so this never waits long in
    /// practice.
    fn wait_for_sample_load(graph: &mut Graph) {
        for _ in 0..500 {
            graph.poll_sampler_loads();
            if graph.sampler_loads.is_empty() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!("the sample load did not finish in time");
    }

    /// A dropped audio file makes a Sampler centered on the drop point, with the file loading.
    #[test]
    fn a_dropped_audio_file_makes_a_loaded_sampler_at_the_drop() {
        let dir = TempDir::new("drop");
        let path = dir.0.join("clip.wav");
        write_wav(&path);
        let mut graph = minimal_graph();
        let before = graph.nodes.len();
        assert!(graph.drop_sample(path.clone(), Point::new(400.0, 300.0)));
        assert_eq!(graph.nodes.len(), before + 1);
        let node = graph.nodes.len() - 1;
        assert_eq!(graph.nodes[node].kind, NodeKind::Sampler);
        assert_eq!(graph.nodes[node].sample_path, Some(path));
        let at = graph.nodes[node].position;
        assert_eq!(
            (at.x, at.y),
            (400.0 - crate::nodes::NODE_WIDTH / 2.0, 300.0)
        );
        assert_eq!(graph.selected, Some(node));
        wait_for_sample_load(&mut graph);
        assert!(graph.nodes[node].sample.is_some());
    }

    /// The drop point follows the view, so a panned or zoomed editor still lands under the cursor.
    #[test]
    fn a_drop_lands_under_the_cursor_in_a_zoomed_view() {
        let dir = TempDir::new("drop-zoom");
        let path = dir.0.join("clip.wav");
        write_wav(&path);
        let mut graph = minimal_graph();
        graph.view.zoom = 2.0;
        graph.view.offset = nih_plug_iced::Vector::new(100.0, 50.0);
        let screen = Point::new(500.0, 250.0);
        assert!(graph.drop_sample(path, screen));
        let node = &graph.nodes[graph.nodes.len() - 1];
        let world = graph.view.to_world(screen);
        assert_eq!(
            (node.position.x, node.position.y),
            (world.x - crate::nodes::NODE_WIDTH / 2.0, world.y)
        );
    }

    /// A file that is not audio adds nothing and says so.
    #[test]
    fn a_dropped_non_audio_file_adds_nothing() {
        let mut graph = minimal_graph();
        let before = graph.nodes.len();
        assert!(!graph.drop_sample(
            std::path::PathBuf::from("/x/notes.txt"),
            Point::new(1.0, 1.0)
        ));
        assert_eq!(graph.nodes.len(), before);
        assert!(graph.status.as_ref().unwrap().0.contains("notes.txt"));
    }

    /// Several files dropped on one spot don't stack on each other.
    #[test]
    fn files_dropped_together_are_staggered() {
        let dir = TempDir::new("drop-many");
        let mut graph = minimal_graph();
        for name in ["a.wav", "b.wav", "c.wav"] {
            let path = dir.0.join(name);
            write_wav(&path);
            assert!(graph.drop_sample(path, Point::new(400.0, 300.0)));
        }
        let n = graph.nodes.len();
        let spots: Vec<(f32, f32)> = graph.nodes[n - 3..]
            .iter()
            .map(|node| (node.position.x, node.position.y))
            .collect();
        assert_ne!(spots[0], spots[1]);
        assert_ne!(spots[1], spots[2]);
        assert_ne!(spots[0], spots[2]);
        while !graph.sampler_loads.is_empty() {
            wait_for_sample_load(&mut graph);
        }
    }

    /// Starting a load decodes a valid file on a background thread and stores the result as an
    /// `Arc`, with the status line reporting success.
    #[test]
    fn a_valid_file_loads_in_the_background_and_is_kept_as_an_arc() {
        let dir = TempDir::new("valid");
        let path = dir.0.join("hit.wav");
        write_wav(&path);
        let (mut graph, node) = sampler_graph();
        graph.start_sample_load(node, path.clone());
        assert_eq!(graph.nodes[node].sample_path, Some(path));
        wait_for_sample_load(&mut graph);
        let sample = graph.nodes[node]
            .sample
            .as_ref()
            .expect("the sample loaded");
        assert_eq!(sample.sample_rate, 44100.0);
        assert_eq!(sample.channels, 1);
        assert!(graph.status.as_ref().unwrap().0.contains("Loaded"));
    }

    /// A missing file fails cleanly: the node stays sample-less and the status line says why,
    /// without panicking.
    #[test]
    fn a_missing_file_fails_cleanly() {
        let (mut graph, node) = sampler_graph();
        let missing = std::path::PathBuf::from("/no/such/flode/sample.wav");
        graph.start_sample_load(node, missing);
        wait_for_sample_load(&mut graph);
        assert!(graph.nodes[node].sample.is_none());
        assert!(graph.status.as_ref().unwrap().0.contains("Could not load"));
    }

    /// A file that exists but is not a WAV also fails cleanly.
    #[test]
    fn a_corrupt_file_fails_cleanly() {
        let dir = TempDir::new("corrupt");
        let path = dir.0.join("not-a-wav.wav");
        std::fs::write(&path, b"not actually a wav file").unwrap();
        let (mut graph, node) = sampler_graph();
        graph.start_sample_load(node, path);
        wait_for_sample_load(&mut graph);
        assert!(graph.nodes[node].sample.is_none());
        assert!(graph.status.as_ref().unwrap().0.contains("Could not load"));
    }

    /// The full dialog sequence: the file picker answers with a path, which starts the decode, and
    /// the decode's own answer lands on the node.
    #[test]
    fn the_dialog_answer_starts_a_load() {
        let dir = TempDir::new("dialog");
        let path = dir.0.join("picked.wav");
        write_wav(&path);
        let (mut graph, node) = sampler_graph();
        graph.start_sampler_dialog(node);
        let sender = graph
            .sampler_dialog_sender
            .clone()
            .expect("the dialog is open");
        sender.send(Some(path)).unwrap();
        graph.poll_sampler_dialog();
        assert!(graph.sampler_dialog.is_none());
        wait_for_sample_load(&mut graph);
        assert!(graph.nodes[node].sample.is_some());
    }

    /// Canceling the dialog starts no load and leaves the node as it was.
    #[test]
    fn canceling_the_dialog_starts_no_load() {
        let (mut graph, node) = sampler_graph();
        graph.start_sampler_dialog(node);
        let sender = graph.sampler_dialog_sender.clone().unwrap();
        sender.send(None).unwrap();
        graph.poll_sampler_dialog();
        assert!(graph.sampler_loads.is_empty());
        assert!(graph.nodes[node].sample_path.is_none());
    }

    /// Saving keeps the path, the root note, the knobs, the dropdown, and the switches, and
    /// loading the save starts the decode again, so a fresh load with the file present reloads the
    /// audio without the user re-browsing for it.
    #[test]
    fn save_and_load_round_trip_reloads_a_present_file() {
        let dir = TempDir::new("roundtrip");
        let path = dir.0.join("snare.wav");
        write_wav(&path);
        let (mut graph, node) = sampler_graph();
        graph.start_sample_load(node, path.clone());
        wait_for_sample_load(&mut graph);
        assert!(graph.nodes[node].sample.is_some());

        let n = &mut graph.nodes[node];
        n.set_value(KnobId::SamplerRoot, KnobId::SamplerRoot.to_normalized(72.0));
        n.set_value(
            KnobId::SamplerStart,
            KnobId::SamplerStart.to_normalized(10.0),
        );
        n.set_value(
            KnobId::SamplerLoopStart,
            KnobId::SamplerLoopStart.to_normalized(20.0),
        );
        n.set_value(
            KnobId::SamplerLoopEnd,
            KnobId::SamplerLoopEnd.to_normalized(80.0),
        );
        n.choose(0, 1); // Loop: Forward
        n.set_switch(0, false); // pitch tracking off
        n.set_switch(1, true); // one shot
        n.set_switch(2, true); // reverse

        let json = serde_json::to_string(&graph.to_saved()).unwrap();
        let mut loaded = Graph::from_saved(&serde_json::from_str(&json).unwrap());
        let loaded_index = loaded
            .nodes
            .iter()
            .position(|n| n.kind == NodeKind::Sampler)
            .expect("the sampler loads");

        assert_eq!(loaded.nodes[loaded_index].sample_path, Some(path));
        assert!(
            loaded.nodes[loaded_index].sample.is_none(),
            "not yet decoded"
        );
        assert!((loaded.nodes[loaded_index].plain(KnobId::SamplerRoot) - 72.0).abs() < 1e-3);
        assert!((loaded.nodes[loaded_index].plain(KnobId::SamplerStart) - 10.0).abs() < 1e-3);
        assert!((loaded.nodes[loaded_index].plain(KnobId::SamplerLoopStart) - 20.0).abs() < 1e-3);
        assert!((loaded.nodes[loaded_index].plain(KnobId::SamplerLoopEnd) - 80.0).abs() < 1e-3);
        assert_eq!(
            sampler::loop_mode(&loaded.nodes[loaded_index]),
            sampler::LoopMode::Forward
        );
        assert!(!sampler::pitch_tracking(&loaded.nodes[loaded_index]));
        assert!(sampler::one_shot(&loaded.nodes[loaded_index]));
        assert!(sampler::reverse(&loaded.nodes[loaded_index]));

        // The decode `from_saved` started finishes in the background, exactly as a fresh "Load
        // sample..." click would.
        wait_for_sample_load(&mut loaded);
        assert!(loaded.nodes[loaded_index].sample.is_some());
    }

    /// Loading a save whose file no longer exists leaves the node sample-less and reports the
    /// failure, naming the node and the path, without panicking or blocking the rest of the patch.
    #[test]
    fn from_saved_reports_a_missing_file_without_panicking() {
        let (mut graph, node) = sampler_graph();
        let missing = std::path::PathBuf::from("/no/such/flode/sample.wav");
        graph.nodes[node].sample_path = Some(missing.clone());

        let json = serde_json::to_string(&graph.to_saved()).unwrap();
        let mut loaded = Graph::from_saved(&serde_json::from_str(&json).unwrap());
        wait_for_sample_load(&mut loaded);

        let loaded_index = loaded
            .nodes
            .iter()
            .position(|n| n.kind == NodeKind::Sampler)
            .unwrap();
        assert!(loaded.nodes[loaded_index].sample.is_none());
        assert!(loaded.status.as_ref().unwrap().0.contains("Could not load"));
        // The rest of the patch still compiles and plays; a sample-less Sampler is simply silent.
        let _ = loaded.patch();
    }

    /// Duplicating a Sampler keeps the same loaded sample, an `Arc` clone, and the same path.
    #[test]
    fn duplicate_keeps_the_sample_and_path() {
        let (mut graph, node) = sampler_graph();
        let data = Arc::new(SampleData {
            sample_rate: 44100.0,
            channels: 1,
            frames: vec![0.0; 10],
        });
        graph.nodes[node].sample = Some(data.clone());
        graph.nodes[node].sample_path = Some(std::path::PathBuf::from("/tmp/x.wav"));
        let copy = graph.nodes[node].duplicate(Point::new(50.0, 50.0));
        assert!(Arc::ptr_eq(&copy.sample.unwrap(), &data));
        assert_eq!(copy.sample_path, graph.nodes[node].sample_path);
    }

    /// Ping-pong looping reverses direction at each loop boundary and never leaves the loop
    /// region.
    #[test]
    fn ping_pong_bounces_within_the_loop() {
        use crate::dsp::frame::Frame;
        use crate::engine::stage::testing::Rig;
        use crate::engine::stage::{AudioStage, Sampler};
        use crate::nodes::sampler::LoopMode;
        use crate::patch::control::Param;
        use crate::patch::stage::{SamplerParams, MAX_INPUTS};

        let data = Arc::new(SampleData {
            sample_rate: 44100.0,
            channels: 1,
            frames: (0..1000).map(|i| (i + 1) as f32 / 1000.0).collect(),
        });
        let p = SamplerParams {
            data: Some(data),
            root: Param::fixed(KnobId::SamplerRoot, 69.0),
            loop_mode: LoopMode::PingPong,
            loop_start: Param::fixed(KnobId::SamplerLoopStart, 10.0),
            loop_end: Param::fixed(KnobId::SamplerLoopEnd, 20.0),
            ..SamplerParams::default()
        };
        let mut rig = Rig::new();
        let mut state = Sampler::start(&p, &mut rig.start_context());
        let mut saw_backward = false;
        let mut last = None;
        for _ in 0..5000 {
            Sampler::process(
                &p,
                &mut state,
                &[Frame::SILENT; MAX_INPUTS],
                &mut rig.tick(),
            );
            let position = state.position();
            assert!((95.0..=205.0).contains(&position), "{position}");
            if let Some(prev) = last {
                if position < prev {
                    saw_backward = true;
                }
            }
            last = Some(position);
        }
        assert!(saw_backward, "ping-pong never reversed direction");
    }
}
