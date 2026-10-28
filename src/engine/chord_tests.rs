//! Engine-level tests for a chord compiled into a real graph: a MIDI input into a chord into an
//! oscillator, the way `Wiring` compiles one. Mirrors the pattern in `regression_tests` and
//! `arpeggiator`'s own engine tests, but through a compiled patch rather than a hand-built one, so
//! the slot wiring between the input, the chord, and the oscillator is exercised end to end.

use super::tests::{engine_on, peak, render};
use super::*;
use crate::nodes::connection::Connection;
use crate::nodes::{FlodeNode, NodeKind, Port};
use crate::patch::wiring::Wiring;
use nih_plug_iced::Point;

fn node(kind: NodeKind) -> FlodeNode {
    FlodeNode::new(kind, Point::new(0.0, 0.0))
}

/// A real Input into Chord into Oscillator into Output graph, compiled the way the editor compiles
/// one. The chord starts at its default: Major, with no spread.
fn chord_patch() -> Patch {
    let mut nodes = vec![
        node(NodeKind::Input),
        node(NodeKind::Chord),
        node(NodeKind::Oscillator),
        node(NodeKind::Output),
        node(NodeKind::SawWave),
    ];
    nodes[3].used = 1;
    let links = vec![
        Connection {
            from: 0,
            out: Port::Notes,
            to: 1,
            input: Port::Notes,
        },
        Connection {
            from: 1,
            out: Port::Notes,
            to: 2,
            input: Port::Notes,
        },
        Connection {
            from: 2,
            out: Port::Audio,
            to: 3,
            input: Port::Slot(0),
        },
        Connection {
            from: 4,
            out: Port::Table,
            to: 2,
            input: Port::TableIn,
        },
    ];
    Wiring::new(&nodes, &links).patch()
}

fn holds(notes: [u64; 2], note: u8) -> bool {
    notes[(note / 64) as usize] & (1u64 << (note % 64)) != 0
}

/// A chord fed into an oscillator sounds the whole chord from one held note, and the whole chord
/// goes quiet together when that note lets go.
#[test]
fn a_chord_fed_into_an_oscillator_plays_the_full_chord() {
    let patch = chord_patch();
    let mut engine = engine_on(&patch);
    engine.note_on(60, 0.8);
    // The chord's own note-on events dispatch inside `process_sample`, the same as an
    // arpeggiator's steps, so the first sample needs rendering before the voices show up.
    let out = render(&mut engine, 10);
    assert!(peak(&out) > 0.0);
    let notes = engine.active_notes();
    assert!(holds(notes, 60), "root");
    assert!(holds(notes, 64), "major third");
    assert!(holds(notes, 67), "fifth");

    engine.note_off(60);
    render(&mut engine, 4410);
    assert_eq!(engine.active_notes(), [0, 0]);
}

/// A chord fed from an arpeggiator, rather than straight from the MIDI input, still sounds. This
/// is the one case `note_held` does not fill from the host's own note on and off: the arpeggiator
/// has to carry its generated notes into `note_held` itself, or the chord downstream of it would
/// see nothing held and stay silent.
#[test]
fn a_chord_fed_by_an_arpeggiator_sounds() {
    let mut nodes = vec![
        node(NodeKind::Input),
        node(NodeKind::Arpeggiator),
        node(NodeKind::Chord),
        node(NodeKind::Oscillator),
        node(NodeKind::Output),
        node(NodeKind::SawWave),
    ];
    nodes[4].used = 1;
    let links = vec![
        Connection {
            from: 0,
            out: Port::Notes,
            to: 1,
            input: Port::Notes,
        },
        Connection {
            from: 1,
            out: Port::Notes,
            to: 2,
            input: Port::Notes,
        },
        Connection {
            from: 2,
            out: Port::Notes,
            to: 3,
            input: Port::Notes,
        },
        Connection {
            from: 3,
            out: Port::Audio,
            to: 4,
            input: Port::Slot(0),
        },
        Connection {
            from: 5,
            out: Port::Table,
            to: 3,
            input: Port::TableIn,
        },
    ];
    let patch = Wiring::new(&nodes, &links).patch();
    let mut engine = engine_on(&patch);
    engine.note_on(60, 0.8);
    engine.note_on(64, 0.8);
    let out = render(&mut engine, 200);
    assert!(peak(&out) > 0.0, "the chain should make sound");
}
