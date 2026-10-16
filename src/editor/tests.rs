//! Tests for the editor's graph model.

use crate::dsp::wavetable::Waveform;
use crate::nodes::connection::Connection;
use crate::nodes::knobs::KnobId;
use crate::nodes::oscillator;
use crate::nodes::{Category, FlodeNode, NodeKind, Port};
use crate::patch::control::{ControlLevels, MapStage, Source, MAX_ENVELOPES};
use crate::patch::stage::{LaneGraph, StageParams};
use nih_plug_iced::{Point, Rectangle};

use super::{curve::*, draw::*, graph::*, menus::*, saved::*};

/// The smallest working graph: an oscillator at node 0, wired to the output at node 1, with a MIDI
/// input at node 2 feeding it notes. Most tests want exactly this, so they start from it and add
/// what they need. The default graph a new instance loads has an envelope as well, and its own
/// tests are in `default_graph_tests`.
fn minimal_graph() -> Graph {
    let mut graph = Graph::with(
        vec![
            FlodeNode::new(NodeKind::Oscillator, Point::new(DEFAULT_OSC_X, 80.0)),
            FlodeNode::new(NodeKind::Output, Point::new(DEFAULT_OUTPUT_X, 80.0)),
            FlodeNode::new(NodeKind::Input, Point::new(0.0, 80.0)),
        ],
        vec![
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
    match lane.stages[index].params {
        StageParams::Oscillator(params) => params,
        _ => panic!("stage {index} is not an oscillator"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let at = |envelope: f32| {
            route.apply(&ControlLevels {
                envelope: [envelope; MAX_ENVELOPES],
                ..ControlLevels::default()
            })
        };
        assert!((at(0.0) - 0.6).abs() < 1e-6);
        assert!((at(1.0) - 0.8).abs() < 1e-6);
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
        graph.nodes[0].choose(0, 2);
        graph.nodes[1].switches = vec![true, true];
        let loaded = Graph::from_saved(&graph.to_saved());
        assert_eq!(loaded.nodes[0].plain(KnobId::Unison), 5.0);
        assert_eq!(oscillator::waveform(&loaded.nodes[0]), Waveform::Square);
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
        };
        assert_eq!(
            oscillator::waveform(&legacy.into_node()),
            Waveform::Triangle
        );
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
                lfo: [lfo, 0.0],
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

    /// The first LFO is the one the audio thread runs. Later LFOs make no signal.
    #[test]
    fn only_the_first_lfos_are_run() {
        let mut graph = minimal_graph();
        let mut lfos = Vec::new();
        for x in 0..3 {
            let lfo = graph.add_node(NodeKind::Lfo, Point::new(x as f32 * 10.0, 200.0));
            graph.connect(lfo, Port::Shape, 1, Port::Knob(KnobId::Gain));
            lfos.push(lfo);
        }
        // The third LFO is past the two the audio thread runs.
        assert_eq!(graph.wiring().lfo_index(lfos[2]), None);
        assert_eq!(graph.wiring().lfo_index(lfos[1]), Some(1));
        assert!(graph.patch().lfos.iter().all(Option::is_some));
    }

    /// The pill's buttons open the Driver, Util, and Effects lists, and each list holds its kinds.
    #[test]
    fn categories_list_their_kinds() {
        let bounds = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        };
        assert_eq!(
            pill_button_at(bounds, Point::new(450.0, 30.0)),
            Some(PillItem::Category(Category::Driver))
        );
        assert_eq!(
            category_row_at(bounds, Category::Driver, Point::new(400.0, 98.0)),
            Some(NodeKind::Lfo)
        );
        assert!(Category::Effects.kinds().contains(&NodeKind::Filter));
        assert!(!Category::Driver.kinds().contains(&NodeKind::Filter));
        assert!(Category::Util.kinds().contains(&NodeKind::Output));
        assert!(Category::Util.kinds().contains(&NodeKind::Map));
    }

    /// Every kind is in exactly one category, except the oscillator, which has its own button.
    #[test]
    fn every_kind_is_categorised_or_on_the_pill() {
        for kind in NodeKind::ALL {
            let in_category = [Category::Driver, Category::Util, Category::Effects]
                .iter()
                .any(|c| c.kinds().contains(&kind));
            let on_pill = PILL_ITEMS.contains(&PillItem::Kind(kind));
            assert!(in_category ^ on_pill, "{kind:?}");
        }
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
        let route = graph.control_route(1, KnobId::Gain).unwrap();
        assert_eq!(route.source, Source::Midi(MidiSource::Velocity));
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
        assert_eq!(route.source, Source::Envelope(0));
    }
}

#[cfg(test)]
mod map_curve_tests {
    use super::*;

    /// Inputs outside the input range hold at the ends, so the output is flat there.
    #[test]
    fn a_map_clamps_outside_its_input_range() {
        let stage = MapStage {
            in_lo: 0.25,
            in_hi: 0.75,
            out_lo: 0.0,
            out_hi: 1.0,
        };
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
        let handles = map_handles(&node);
        assert!((handles[0].x - bounds.x).abs() < 1e-3);
        assert!((handles[1].y - bounds.y).abs() < 1e-3);
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
        assert_eq!(graph.nodes[lfo].switch_labels(), &["Retrigger"]);
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

    /// In a plugin the computer keyboard plays nothing, but a click on the strip still does, and
    /// the octave keys still shift the strip.
    #[test]
    fn in_a_plugin_only_clicks_play() {
        use nih_plug_iced::keyboard::{self, KeyCode, Modifiers};
        let bounds = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        };
        let mut graph = minimal_graph();
        graph.computer_keys = false;
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
        assert!(key(&mut graph, KeyCode::A).is_none());
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
                NodeKind::Oscillator => crate::nodes::Bypass::Mute,
                NodeKind::Output
                | NodeKind::Envelope
                | NodeKind::Lfo
                | NodeKind::Input
                | NodeKind::Macro
                | NodeKind::Map => crate::nodes::Bypass::None,
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
        graph.nodes[0].choose(0, 2);
        graph.nodes[0].toggle_bypass();
        let wires = graph.connections.len();
        let count = graph.nodes.len();

        let copy = graph.duplicate_node(0).expect("an oscillator duplicates");
        assert_eq!(graph.nodes.len(), count + 1);
        assert_eq!(graph.nodes[copy].kind, NodeKind::Oscillator);
        assert_eq!(graph.nodes[copy].plain(KnobId::Unison), 5.0);
        assert_eq!(oscillator::waveform(&graph.nodes[copy]), Waveform::Square);
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
            &[MenuItem::Duplicate, MenuItem::Bypass, MenuItem::Delete]
        );
        assert_eq!(
            items(NodeKind::Reverb),
            &[MenuItem::Duplicate, MenuItem::Bypass, MenuItem::Delete]
        );
        assert_eq!(
            items(NodeKind::Lfo),
            &[MenuItem::Duplicate, MenuItem::Delete]
        );
        assert_eq!(items(NodeKind::Output), &[MenuItem::Delete]);
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
        };
        let area = menu_bounds(menu);
        assert_eq!(area.height, 3.0 * MENU_ITEM_HEIGHT);
        let at = |row: f32| Point::new(60.0, 60.0 + (row + 0.5) * MENU_ITEM_HEIGHT);
        assert_eq!(menu_item_at(menu, at(0.0)), Some(MenuItem::Duplicate));
        assert_eq!(menu_item_at(menu, at(1.0)), Some(MenuItem::Bypass));
        assert_eq!(menu_item_at(menu, at(2.0)), Some(MenuItem::Delete));
        assert_eq!(menu_item_at(menu, at(3.0)), None);
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
        // Coarse, fine, and octave are folded and keep their pins.
        assert_eq!(packed.len(), 4);
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
    use crate::editor::piano::PianoEvent;
    use crate::editor::style::MENU_ITEM_HEIGHT;
    use crate::presets::Library;
    use nih_plug_iced::keyboard::{self, KeyCode, Modifiers};
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

    fn key(editor: &mut NodeEditor, event: keyboard::Event) -> Option<Message> {
        editor
            .update(Event::Keyboard(event), bounds(), Cursor::Unavailable)
            .1
    }

    fn type_text(editor: &mut NodeEditor, text: &str) {
        for c in text.chars() {
            key(editor, keyboard::Event::CharacterReceived(c));
        }
    }

    fn press_key(editor: &mut NodeEditor, key_code: KeyCode) -> Option<Message> {
        let event = keyboard::Event::KeyPressed {
            key_code,
            modifiers: Modifiers::default(),
        };
        key(editor, event)
    }

    fn presets_button() -> Point {
        let segment = pill_segment(bounds(), 4);
        Point::new(
            segment.x + segment.width / 2.0,
            segment.y + segment.height / 2.0,
        )
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

    /// Save a preset through the UI: open the list, pick Save as, type a name, press Tab, type a
    /// category, press Enter. The editor then saves it, which the test does by hand with the
    /// macro values it is given.
    fn save_through_the_ui(graph: &mut Graph, name: &str, category: &str, macros: [f32; 4]) {
        let library = graph
            .preset_dir
            .as_deref()
            .map(crate::presets::library_in)
            .unwrap_or_default();
        let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        click(&mut editor, presets_button());
        click(&mut editor, list_row(&library, 0));
        assert!(editor.graph.name_entry.is_some());
        // Clear the name and category the box started with, then type.
        for _ in 0..60 {
            press_key(&mut editor, KeyCode::Backspace);
        }
        type_text(&mut editor, name);
        press_key(&mut editor, KeyCode::Tab);
        for _ in 0..60 {
            press_key(&mut editor, KeyCode::Backspace);
        }
        type_text(&mut editor, category);
        let message = press_key(&mut editor, KeyCode::Enter);
        assert!(matches!(message, Some(Message::GraphChanged)));
        graph.save_pending_preset(macros);
    }

    #[test]
    fn saving_through_the_ui_writes_a_named_preset_in_its_category() {
        let dir = TempDir::new("save");
        let mut graph = graph_in(&dir);
        save_through_the_ui(&mut graph, "  Warm pad ", " Pad ", [0.1, 0.2, 0.3, 0.4]);

        let library = library_of(&dir);
        assert_eq!(library.categories.len(), 1);
        assert_eq!(library.categories[0].name, "Pad");
        assert_eq!(library.categories[0].presets, ["Warm pad"]);
        assert!(library.loose.is_empty());
        let saved = crate::presets::load_in(&dir.0, "Pad", "Warm pad").unwrap();
        assert_eq!(saved.macros, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(graph.preset_name.as_deref(), Some("Warm pad"));
        assert_eq!(graph.preset_category, "Pad");
        assert!(graph.name_entry.is_none() && graph.pending_save.is_none());
        // The name and category stay with the project's saved graph.
        let reloaded = Graph::from_saved(&graph.to_saved());
        assert_eq!(reloaded.preset_name.as_deref(), Some("Warm pad"));
        assert_eq!(reloaded.preset_category, "Pad");
    }

    /// A preset saved with no category sits loose in the folder, and it is listed after the
    /// categories.
    #[test]
    fn a_preset_with_no_category_is_listed_loose() {
        let dir = TempDir::new("loose");
        let mut graph = graph_in(&dir);
        save_through_the_ui(&mut graph, "Plain", "", [0.0; 4]);
        let library = library_of(&dir);
        assert!(library.categories.is_empty());
        assert_eq!(library.loose, ["Plain"]);
        assert!(dir.0.join("Plain.flode").is_file());
    }

    /// Saving again under a name that is already there replaces that preset, and the box starts
    /// with the current preset's name and category.
    #[test]
    fn saving_again_keeps_the_name_and_category_and_replaces_the_file() {
        let dir = TempDir::new("again");
        let mut graph = graph_in(&dir);
        save_through_the_ui(&mut graph, "Pluck", "Lead", [0.0; 4]);
        graph.start_name_entry();
        let entry = graph.name_entry.clone().unwrap();
        assert_eq!(
            (entry.name.as_str(), entry.category.as_str()),
            ("Pluck", "Lead")
        );
        graph.confirm_name_entry();
        graph.save_pending_preset([0.9; 4]);
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
        click(&mut editor, presets_button());
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

    /// While the box is open, keys are text. They do not play the piano or run a shortcut.
    #[test]
    fn the_name_box_takes_the_keys() {
        let dir = TempDir::new("keys");
        let mut graph = graph_in(&dir);
        graph.selected = Some(0);
        graph.start_name_entry();
        graph.name_entry.as_mut().unwrap().name.clear();
        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        let name = |editor: &NodeEditor| editor.graph.name_entry.clone().unwrap();

        // 'a' is a piano key and 'm' is the bypass shortcut. Both are just letters here.
        let message = press_key(&mut editor, KeyCode::A);
        assert!(!matches!(message, Some(Message::Note(_))));
        type_text(&mut editor, "am");
        press_key(&mut editor, KeyCode::M);
        assert!(!editor.graph.nodes[0].bypassed);
        assert_eq!(name(&editor).name, "am");

        // Characters a file name can't hold are dropped, and Backspace removes one.
        type_text(&mut editor, "a/b:c");
        assert_eq!(name(&editor).name, "amabc");
        press_key(&mut editor, KeyCode::Backspace);
        assert_eq!(name(&editor).name, "amab");

        // Tab moves the typing to the category, and back.
        press_key(&mut editor, KeyCode::Tab);
        assert_eq!(name(&editor).field, EntryField::Category);
        type_text(&mut editor, "Bass");
        assert_eq!(
            (name(&editor).name.as_str(), name(&editor).category.as_str()),
            ("amab", "Bass")
        );
        press_key(&mut editor, KeyCode::Tab);
        assert_eq!(name(&editor).field, EntryField::Name);

        // Enter with a blank name keeps the box open.
        editor.graph.name_entry.as_mut().unwrap().name = "   ".to_string();
        press_key(&mut editor, KeyCode::Enter);
        assert!(editor.graph.name_entry.is_some());
        assert!(editor.graph.pending_save.is_none());

        // Enter with a name takes the name and the category.
        editor.graph.name_entry.as_mut().unwrap().name = " Dark ".to_string();
        press_key(&mut editor, KeyCode::Enter);
        assert!(editor.graph.name_entry.is_none());
        assert_eq!(
            editor.graph.pending_save,
            Some(("Dark".to_string(), "Bass".to_string()))
        );
        editor.graph.pending_save = None;
        editor.graph.start_name_entry();

        press_key(&mut editor, KeyCode::Escape);
        assert!(editor.graph.name_entry.is_none());
        assert!(editor.graph.pending_save.is_none());
    }

    /// The name stops growing at the longest allowed name.
    #[test]
    fn the_name_has_a_length_limit() {
        let dir = TempDir::new("limit");
        let mut graph = graph_in(&dir);
        graph.start_name_entry();
        graph.name_entry.as_mut().unwrap().name.clear();
        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        type_text(&mut editor, &"x".repeat(100));
        press_key(&mut editor, KeyCode::Tab);
        type_text(&mut editor, &"y".repeat(100));
        let entry = editor.graph.name_entry.clone().unwrap();
        assert_eq!(entry.name.chars().count(), crate::presets::MAX_NAME_LEN);
        assert_eq!(entry.category.chars().count(), crate::presets::MAX_NAME_LEN);
    }

    /// A key held when the box opens still ends its note when it comes up.
    #[test]
    fn a_held_piano_key_still_releases_while_the_box_is_open() {
        let dir = TempDir::new("held");
        let mut graph = graph_in(&dir);
        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        let on = press_key(&mut editor, KeyCode::A);
        assert!(matches!(on, Some(Message::Note(PianoEvent::NoteOn { .. }))));
        editor.graph.start_name_entry();
        let up = keyboard::Event::KeyReleased {
            key_code: KeyCode::A,
            modifiers: Modifiers::default(),
        };
        let off = key(&mut editor, up);
        assert!(matches!(
            off,
            Some(Message::Note(PianoEvent::NoteOff { .. }))
        ));
    }

    /// A press outside the box cancels it, and one on the keys does not play a note.
    #[test]
    fn a_press_outside_the_box_cancels_it() {
        let dir = TempDir::new("outside");
        let mut graph = graph_in(&dir);
        graph.start_name_entry();
        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        let keys = Point::new(300.0, 700.0);
        let message = click(&mut editor, keys);
        assert!(!matches!(message, Some(Message::Note(_))));
        assert!(editor.graph.name_entry.is_none());

        // A press inside the box leaves it open.
        editor.graph.start_name_entry();
        let inside = name_box_bounds(bounds(), &[]);
        click(&mut editor, Point::new(inside.x + 10.0, inside.y + 10.0));
        assert!(editor.graph.name_entry.is_some());
    }

    /// The button opens the list, and pressing it again or pressing elsewhere closes it.
    #[test]
    fn the_presets_button_toggles_the_list() {
        let dir = TempDir::new("toggle");
        let mut graph = graph_in(&dir);
        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        click(&mut editor, presets_button());
        assert!(editor.graph.preset_list.is_some());
        click(&mut editor, presets_button());
        assert!(editor.graph.preset_list.is_none());
        click(&mut editor, presets_button());
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
        click(&mut editor, presets_button());
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
        click(&mut editor, presets_button());
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
        // The next save starts from a blank name.
        editor.graph.start_name_entry();
        assert_eq!(editor.graph.name_entry.as_ref().unwrap().name, "");
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
        click(&mut editor, presets_button());
        click(&mut editor, list_row(&library, 2));
        assert!(editor.graph.nodes.iter().any(|n| n.kind == NodeKind::Delay));
        assert_eq!(editor.graph.preset_name.as_deref(), Some("Plain"));
        assert_eq!(editor.graph.preset_category, "");
    }

    /// The save box with these categories to pick from.
    fn entry_with(categories: &[&str]) -> NameEntry {
        NameEntry {
            name: "Sound".to_string(),
            category: String::new(),
            field: EntryField::Name,
            categories: categories.iter().map(|c| c.to_string()).collect(),
        }
    }

    /// A typed category is matched to an existing one without regard to case, and one that
    /// matches nothing is tagged new, so a typo like "baas" shows before it makes a folder.
    #[test]
    fn a_typed_category_is_matched_or_marked_new() {
        let mut entry = entry_with(&["Bass", "Lead", "Pad"]);
        assert!(!entry.is_new_category());
        for (typed, new) in [
            ("Bass", false),
            (" bass ", false),
            ("LEAD", false),
            ("baas", true),
        ] {
            entry.category = typed.to_string();
            assert_eq!(entry.is_new_category(), new, "{typed}");
        }
        entry.category = "pad".to_string();
        assert_eq!(entry.existing_category().map(String::as_str), Some("Pad"));
    }

    /// Up and down step through the existing categories and wrap, and move the typing to the
    /// category field.
    #[test]
    fn the_arrow_keys_step_through_the_categories() {
        let mut entry = entry_with(&["Bass", "Lead", "Pad"]);
        entry.cycle_category(1);
        assert_eq!(entry.category, "Bass");
        assert_eq!(entry.field, EntryField::Category);
        entry.cycle_category(1);
        entry.cycle_category(1);
        assert_eq!(entry.category, "Pad");
        entry.cycle_category(1);
        assert_eq!(entry.category, "Bass");
        entry.cycle_category(-1);
        assert_eq!(entry.category, "Pad");

        // From nothing, up goes to the last. A typed category that matches one starts from it.
        let mut entry = entry_with(&["Bass", "Lead", "Pad"]);
        entry.cycle_category(-1);
        assert_eq!(entry.category, "Pad");
        entry.category = "lead".to_string();
        entry.cycle_category(1);
        assert_eq!(entry.category, "Pad");
        // With no categories there is nothing to pick, and the typed text stays.
        let mut empty = entry_with(&[]);
        empty.category = "Mine".to_string();
        empty.cycle_category(1);
        assert_eq!(empty.category, "Mine");
    }

    /// Saving with a category that differs from an existing one only in case files the preset
    /// under the existing one.
    #[test]
    fn a_case_slip_files_under_the_existing_category() {
        let dir = TempDir::new("case");
        let mut seed = graph_in(&dir);
        save_through_the_ui(&mut seed, "First", "Bass", [0.0; 4]);

        let mut graph = graph_in(&dir);
        save_through_the_ui(&mut graph, "Second", "bass", [0.0; 4]);
        let library = library_of(&dir);
        assert_eq!(library.categories.len(), 1);
        assert_eq!(library.categories[0].name, "Bass");
        assert_eq!(library.categories[0].presets, ["First", "Second"]);
    }

    /// The save box starts with the categories that exist, and a click on a chip picks one.
    #[test]
    fn a_chip_picks_an_existing_category() {
        let dir = TempDir::new("chips");
        let mut seed = graph_in(&dir);
        save_through_the_ui(&mut seed, "A", "Bass", [0.0; 4]);
        save_through_the_ui(&mut seed, "B", "Pad", [0.0; 4]);

        let mut graph = graph_in(&dir);
        graph.start_name_entry();
        graph.name_entry.as_mut().unwrap().name = "Sound".to_string();
        assert_eq!(
            graph.name_entry.as_ref().unwrap().categories,
            ["Bass", "Pad"]
        );
        let categories = graph.name_entry.as_ref().unwrap().categories.clone();
        let area = name_box_bounds(bounds(), &categories);
        let layout = name_box_layout(&categories);
        assert_eq!(layout.chips.len(), 2);
        let (_, pad) = layout.chips[1];
        let middle = Point::new(
            area.x + pad.x + pad.width / 2.0,
            area.y + pad.y + pad.height / 2.0,
        );

        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        click(&mut editor, middle);
        let entry = editor.graph.name_entry.clone().expect("the box stays open");
        assert_eq!(entry.category, "Pad");
        assert_eq!(entry.field, EntryField::Category);
        assert!(!entry.is_new_category());
        // Enter files the preset under it.
        press_key(&mut editor, KeyCode::Enter);
        assert_eq!(
            editor.graph.pending_save,
            Some(("Sound".to_string(), "Pad".to_string()))
        );
    }

    /// The chips wrap inside the box, there are at most three rows, and every chip is clickable
    /// inside the box's bounds.
    #[test]
    fn the_chips_wrap_inside_the_box() {
        let names: Vec<String> = (0..40).map(|n| format!("Category {n}")).collect();
        let layout = name_box_layout(&names);
        assert!(!layout.chips.is_empty() && layout.chips.len() < names.len());
        let rows: std::collections::BTreeSet<i32> =
            layout.chips.iter().map(|(_, chip)| chip.y as i32).collect();
        assert_eq!(rows.len(), 3);
        for (_, chip) in &layout.chips {
            assert!(chip.x >= 0.0 && chip.x + chip.width <= 360.0);
            assert!(chip.y + chip.height <= layout.hint_y);
        }
        assert!(layout.height > layout.hint_y);
        // The box is the same size as before when there is nothing to pick from.
        assert_eq!(name_box_layout(&[]).height, 136.0);
        // The box still sits above the keys.
        let area = name_box_bounds(bounds(), &names);
        assert!(area.y + area.height <= bounds().height - crate::editor::piano::STRIP_HEIGHT);
    }

    /// A category that is not usable keeps the box open and says so.
    #[test]
    fn a_bad_category_keeps_the_box_open() {
        let dir = TempDir::new("bad-category");
        let mut graph = graph_in(&dir);
        graph.start_name_entry();
        let entry = graph.name_entry.as_mut().unwrap();
        entry.name = "Fine".to_string();
        entry.category = ".hidden".to_string();
        graph.confirm_name_entry();
        assert!(graph.name_entry.is_some() && graph.pending_save.is_none());
        assert!(graph.status.is_some());
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
