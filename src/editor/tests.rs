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
        assert!(graph.any_driven());
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
