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
        let mut graph = Graph::default();
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
        let mut graph = Graph::default();
        let map = graph.add_node(NodeKind::Map, Point::new(0.0, 0.0));
        graph.connect(map, Port::Shape, 1, Port::Knob(KnobId::Gain));
        assert_eq!(graph.control_route(1, KnobId::Gain), None);
        assert!(!graph.is_driven(1, KnobId::Gain));
    }

    /// Two filters keep separate knobs. A wire into one's cutoff drives only that filter.
    #[test]
    fn a_control_wire_drives_only_its_own_node() {
        let mut graph = Graph::default();
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
        let graph = Graph::default();
        let output = &graph.nodes[1];
        assert_eq!(output.used, 1);
        assert_eq!(output.slots, 2);
        assert!(graph.patch().lanes[0].is_active());
        assert!(!graph.patch().lanes[1].is_active());
    }

    /// Wiring the empty pin adds a slot and a new empty pin below it.
    #[test]
    fn wiring_the_empty_pin_adds_a_slot() {
        let mut graph = Graph::default();
        let second = graph.add_node(NodeKind::Oscillator, Point::new(0.0, 200.0));
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
        let mut graph = Graph::default();
        let second = graph.add_node(NodeKind::Oscillator, Point::new(0.0, 200.0));
        let third = graph.add_node(NodeKind::Oscillator, Point::new(0.0, 400.0));
        graph.connect(second, Port::Audio, 1, Port::Slot(1));
        graph.connect(third, Port::Audio, 1, Port::Slot(2));
        assert_eq!(graph.connections.len(), 3);

        let loaded = Graph::from_saved(&graph.to_saved());
        assert_eq!(loaded.connections, graph.connections);
        assert_eq!(loaded.nodes[1].used, 3);
        assert!(loaded.patch().lanes[2].is_active());
    }

    /// Two oscillators wired to two slots keep their own knobs. Turning one does not move the other.
    #[test]
    fn two_oscillators_keep_separate_knobs() {
        let mut graph = Graph::default();
        let second = graph.add_node(NodeKind::Oscillator, Point::new(0.0, 200.0));
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
            nodes: vec![node(NodeKind::Oscillator), node(NodeKind::Output)],
            connections: vec![(0, 1)],
            links: Vec::new(),
        };
        let graph = Graph::from_saved(&saved);
        assert_eq!(graph.connections.len(), 1);
        assert_eq!(graph.connections[0].input, Port::Slot(0));
        assert!(graph.patch().lanes[0].is_active());
    }

    /// Saved knob values and the dropdown come back, and old fields still load.
    #[test]
    fn saving_and_loading_keeps_node_state() {
        let mut graph = Graph::default();
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
        let mut graph = Graph::default();
        let curve = graph.link_curve(&graph.connections[0]).unwrap();
        let middle = bezier_point(&curve, 0.5);
        let filter = graph.add_node(
            NodeKind::Filter,
            Point::new(middle.x - 80.0, middle.y - 20.0),
        );

        assert_eq!(graph.link_under(filter), Some(0));
        graph.insert_into(filter, 0);
        graph.refresh_slots();

        assert_eq!(graph.connections.len(), 2);
        assert_eq!(graph.connections[0].to, filter);
        assert_eq!(graph.connections[1].from, filter);
        assert_eq!(graph.connections[1].input, Port::Slot(0));
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
        let mut graph = Graph::default();
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
        let mut graph = Graph::default();
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
        let mut graph = Graph::default();
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
        let mut graph = Graph::default();
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
        let mut graph = Graph::default();
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
        let mut graph = Graph::default();
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
        let mut graph = Graph::default();
        graph.connections.clear();
        let a = graph.add_node(NodeKind::Oscillator, Point::new(0.0, 0.0));
        let b = graph.add_node(NodeKind::Oscillator, Point::new(0.0, 0.0));
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
        let mut graph = Graph::default();
        let curve = graph.link_curve(&graph.connections[0]).unwrap();
        let middle = bezier_point(&curve, 0.5);
        let mix = graph.add_node(NodeKind::Mix, Point::new(middle.x - 80.0, middle.y - 20.0));

        assert_eq!(graph.link_under(mix), Some(0));
        graph.insert_into(mix, 0);
        graph.refresh_slots();

        assert_eq!(graph.connections.len(), 2);
        assert_eq!(graph.connections[0].from, 0);
        assert_eq!(graph.connections[0].to, mix);
        assert_eq!(graph.connections[0].input, Port::AudioIn(0));
        assert_eq!(graph.connections[1].from, mix);
        assert_eq!(graph.connections[1].out, Port::Audio);
        assert_eq!(graph.connections[1].input, Port::Slot(0));

        // The oscillator feeds signal A, and signal B waits for a wire.
        let lane = graph.patch().lanes[0].clone();
        assert_eq!(lane.stages.len(), 2);
        assert_eq!(lane.stages[1].inputs, [Some(0), None]);
    }

    /// A source with no audio input still does not splice into a wire.
    #[test]
    fn an_oscillator_dropped_on_an_audio_wire_does_not_splice() {
        let mut graph = Graph::default();
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
        let lane = Graph::default().patch().lanes[0].clone();
        assert_eq!(lane.stages.len(), 1);
    }
}

#[cfg(test)]
mod lfo_retrigger_tests {
    use super::*;

    /// The Retrigger switch on an LFO node reaches the patch, and a save keeps it.
    #[test]
    fn the_retrigger_switch_reaches_the_patch_and_survives_saving() {
        let mut graph = Graph::default();
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
    use crate::editor::piano;
    use nih_plug_iced::mouse;
    use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};

    fn three_nodes() -> Graph {
        let mut graph = Graph::default();
        graph.add_node(NodeKind::Filter, Point::new(0.0, 300.0));
        graph
    }

    /// Nodes draw in their own order, and the selected node draws last, so it is on top.
    #[test]
    fn the_selected_node_draws_last() {
        let mut graph = three_nodes();
        assert_eq!(graph.draw_order(), vec![0, 1, 2]);
        graph.selected = Some(0);
        assert_eq!(graph.draw_order(), vec![1, 2, 0]);
        graph.selected = Some(1);
        assert_eq!(graph.draw_order(), vec![0, 2, 1]);
        graph.selected = None;
        assert_eq!(graph.draw_order(), vec![0, 1, 2]);
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
        assert_eq!(graph.draw_order(), vec![0, 1, 2]);
    }

    /// Where two nodes overlap, a press goes to the one on top, which is the selected one.
    #[test]
    fn a_press_goes_to_the_node_on_top() {
        let mut graph = Graph::default();
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
        let mut graph = Graph::default();
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
        let mut graph = Graph::default();
        let mut editor = NodeEditor::new(&mut graph, ControlLevels::default(), [0; 2], [0.0; 2]);
        let (_, message) = editor.update(
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds,
            Cursor::Available(Point::new(500.0, strip.y + strip.height - 4.0)),
        );
        assert!(matches!(message, Some(Message::Note(_))), "{message:?}");
    }
}

#[cfg(test)]
mod bypass_and_duplicate_tests {
    use super::*;
    use crate::editor::canvas::{Message, NodeEditor};
    use crate::editor::piano::PianoEvent;
    use crate::editor::style::MENU_ITEM_HEIGHT;
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
        let mut graph = Graph::default();
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
        let mut graph = Graph::default();
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
        let mut graph = Graph::default();
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
        let mut graph = Graph::default();
        let second = graph.add_node(NodeKind::Oscillator, Point::new(0.0, 300.0));
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
        let mut graph = Graph::default();
        graph.nodes[1].bypassed = true;
        assert_eq!(stage_count(&graph), 1);
        assert!(!graph.toggle_bypass(1));
    }

    // Saving.

    /// The off state survives saving, and a save from before it loads switched on.
    #[test]
    fn the_off_state_survives_saving() {
        let mut graph = Graph::default();
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
        let mut graph = Graph::default();
        graph.nodes[0].set_value(KnobId::Unison, KnobId::Unison.to_normalized(5.0));
        graph.nodes[0].choose(0, 2);
        graph.nodes[0].toggle_bypass();
        let wires = graph.connections.len();

        let copy = graph.duplicate_node(0).expect("an oscillator duplicates");
        assert_eq!(graph.nodes.len(), 3);
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
        let mut graph = Graph::default();
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
        let mut graph = Graph::default();
        assert_eq!(graph.duplicate_node(1), None);
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.duplicate_node(9), None);
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

        let mut graph = Graph::default();
        let message = click_entry(&mut graph, 1);
        assert!(matches!(message, Some(Message::GraphChanged)));
        assert!(graph.nodes[0].bypassed);
        assert!(graph.menu.is_none());

        let mut graph = Graph::default();
        click_entry(&mut graph, 0);
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.selected, Some(2));

        let mut graph = Graph::default();
        click_entry(&mut graph, 2);
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].kind, NodeKind::Output);
    }

    // Shortcuts.

    /// M mutes the selected node, and pressing it again brings it back.
    #[test]
    fn m_toggles_the_selected_node() {
        let mut graph = Graph::default();
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
        let mut graph = Graph::default();
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
        let mut graph = Graph::default();
        let mut editor = editor(&mut graph);
        assert!(press(&mut editor, KeyCode::M, Modifiers::default()).is_none());
        release(&mut editor, KeyCode::M);
        editor.graph.selected = Some(1);
        assert!(press(&mut editor, KeyCode::M, Modifiers::default()).is_none());
        assert!(!editor.graph.nodes[1].bypassed);
    }

    /// Shift+D copies the selected node and selects the copy.
    #[test]
    fn shift_d_duplicates_the_selected_node() {
        let mut graph = Graph::default();
        graph.selected = Some(0);
        let mut editor = editor(&mut graph);
        let message = press(&mut editor, KeyCode::D, Modifiers::SHIFT);
        assert!(matches!(message, Some(Message::GraphChanged)));
        assert_eq!(editor.graph.nodes.len(), 3);
        assert_eq!(editor.graph.selected, Some(2));
    }

    /// A plain D still plays a note, and does not copy anything.
    #[test]
    fn a_plain_d_still_plays_the_piano() {
        let mut graph = Graph::default();
        graph.selected = Some(0);
        let mut editor = editor(&mut graph);
        let message = press(&mut editor, KeyCode::D, Modifiers::default());
        assert!(matches!(
            message,
            Some(Message::Note(PianoEvent::NoteOn { .. }))
        ));
        assert_eq!(editor.graph.nodes.len(), 2);
    }

    /// Shortcuts that include Control or Command are left to the host.
    #[test]
    fn control_and_command_combinations_are_ignored() {
        let mut graph = Graph::default();
        graph.selected = Some(0);
        let mut editor = editor(&mut graph);
        assert!(press(&mut editor, KeyCode::M, Modifiers::CTRL).is_none());
        assert!(press(&mut editor, KeyCode::M, Modifiers::LOGO).is_none());
        assert!(!editor.graph.nodes[0].bypassed);
    }
}
