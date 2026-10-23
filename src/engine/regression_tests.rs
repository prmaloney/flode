//! The most important test in this change: every patch that has one MIDI Input node, which is
//! every patch that exists today, has to compile and sound exactly as it did before note routing
//! became per-wire instead of a single global broadcast. This compiles a real Input-Oscillator-
//! Output graph through `Wiring`, the same way the editor does, and checks its rendered audio
//! against a patch built the old way: a bare oscillator stage with no note-routing slot of its own,
//! which is what every hand-built `Patch` in this codebase still looks like.

use super::tests::{engine_on, peak, render};
use super::*;
use crate::nodes::connection::Connection;
use crate::nodes::{FlodeNode, NodeKind, Port};
use crate::patch::stage::{OscillatorParams, StageParams, StageSpec};
use crate::patch::wiring::Wiring;
use nih_plug_iced::Point;

fn node(kind: NodeKind) -> FlodeNode {
    FlodeNode::new(kind, Point::new(0.0, 0.0))
}

/// A real Input into Oscillator into Output graph, compiled the way the editor compiles one.
fn compiled_patch() -> Patch {
    let mut nodes = vec![
        node(NodeKind::Input),
        node(NodeKind::Oscillator),
        node(NodeKind::Output),
    ];
    nodes[2].used = 1;
    let links = vec![
        Connection {
            from: 0,
            out: Port::Notes,
            to: 1,
            input: Port::Notes,
        },
        Connection {
            from: 1,
            out: Port::Audio,
            to: 2,
            input: Port::Slot(0),
        },
    ];
    Wiring::new(&nodes, &links).patch()
}

/// The same sound, built the way every other test in this codebase still builds a `Patch` by hand:
/// a bare oscillator stage with no note-routing slot of its own. This is the shape a `Patch` had
/// before `notes_slot` and `note_sources` existed, and it is what `LaneGraph::silent` still
/// defaults to, so this is the behavior the slot-based engine has to keep matching exactly.
fn hand_built_patch() -> Patch {
    let mut patch = Patch::silent();
    patch.lanes[0]
        .stages
        .push(StageSpec::new(StageParams::Oscillator(
            OscillatorParams::default(),
        )));
    patch
}

/// Plays a chord through `patch` and returns what came out.
fn played(patch: &Patch) -> Vec<f32> {
    let mut engine = engine_on(patch);
    for note in [60u8, 64, 67] {
        engine.note_on(note, 0.8);
    }
    let held = render(&mut engine, 4410);
    engine.note_off(64);
    let releasing = render(&mut engine, 4410);
    [held, releasing].concat()
}

#[test]
fn a_compiled_graph_matches_the_old_hand_built_shape_bit_for_bit() {
    let compiled = played(&compiled_patch());
    let hand_built = played(&hand_built_patch());
    assert_eq!(compiled, hand_built);
    assert!(peak(&compiled) > 0.05);
}

/// The compiler sets the oscillator's `notes` field and the lane's `notes_slot`, which is the part
/// of this change that is new; everything else about the compiled stage is the same oscillator a
/// patch has always compiled to.
#[test]
fn the_compiler_fills_in_the_new_slot_fields_and_nothing_else() {
    let compiled = compiled_patch();
    let StageParams::Oscillator(osc) = compiled.lanes[0].stages[0].params else {
        panic!("expected an oscillator stage");
    };
    assert_eq!(osc.notes, Some(0));
    assert_eq!(compiled.lanes[0].notes_slot, Some(0));
    assert_eq!(
        osc,
        OscillatorParams {
            notes: Some(0),
            ..OscillatorParams::default()
        }
    );
}
