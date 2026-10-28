//! Tests for the frame nodes in the editor: editing a curve frame's points, loading a wave file,
//! and saving and loading the wavetable nodes.

use super::canvas::{Message, NodeEditor};
use super::{curve::*, graph::*, saved::*};
use crate::dsp::curve::{Curve, MAX_CURVE_POINTS};
use crate::dsp::frames::{slice_frames, SliceMode};
use crate::nodes::connection::Connection;
use crate::nodes::knobs::KnobId;
use crate::nodes::{frames, lfo, FlodeNode, NodeKind, Port};
use crate::patch::control::ControlLevels;
use nih_plug_iced::mouse;
use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};
use nih_plug_iced::{Point, Rectangle};

const BOUNDS: Rectangle = Rectangle {
    x: 0.0,
    y: 0.0,
    width: 1024.0,
    height: 712.0,
};

fn node(kind: NodeKind) -> FlodeNode {
    FlodeNode::new(kind, Point::new(300.0, 200.0))
}

/// A folder for one test's files, removed when the test ends.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("flode-frame-ui-{tag}-{}", std::process::id()));
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

/// Writes a mono WAV of a sine with a period of 100 samples, `len` samples long.
fn write_wav(path: &std::path::Path, len: usize) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for n in 0..len {
        let v = (std::f64::consts::TAU * n as f64 / 100.0).sin() * 12000.0;
        writer.write_sample(v as i16).unwrap();
    }
    writer.finalize().unwrap();
}

/// Spins on `poll_sampler_loads` until the decode thread has answered. True if any poll said a
/// sample was set.
fn wait_for_load(graph: &mut Graph) -> bool {
    let mut set = false;
    for _ in 0..500 {
        set |= graph.poll_sampler_loads();
        if graph.sampler_loads.is_empty() {
            return set;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("the load did not finish in time");
}

mod points {
    use super::*;

    fn graph() -> Graph {
        Graph::with(vec![node(NodeKind::CurveFrame)], Vec::new())
    }

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

    /// Double clicks where curve position `(x, y)` is on the first node.
    fn double_click(graph: &mut Graph, x: f32, y: f32) {
        let at = spot(&graph.nodes[0], x, y);
        for _ in 0..2 {
            press(graph, at);
            release(graph, at);
        }
    }

    fn points(graph: &Graph) -> Vec<[f32; 2]> {
        lfo::curve(&graph.nodes[0]).points().to_vec()
    }

    /// A new curve frame shows handles on the default curve, and a drag moves a point and says
    /// the graph changed.
    #[test]
    fn dragging_a_point_moves_it() {
        let mut graph = graph();
        assert_eq!(points(&graph), Curve::default().points());
        let start = spot(&graph.nodes[0], 0.5, 1.0);
        assert_eq!(lfo_handle_at(&graph.nodes[0], start), Some(1));
        press(&mut graph, start);
        let to = spot(&graph.nodes[0], 0.3, 0.4);
        let moved = send(&mut graph, mouse::Event::CursorMoved { position: to }, to);
        assert!(matches!(moved, Some(Message::GraphChanged)));
        let point = points(&graph)[1];
        assert!((point[0] - 0.3).abs() < 1e-4 && (point[1] - 0.4).abs() < 1e-4);
        assert!(matches!(
            release(&mut graph, to),
            Some(Message::GraphChanged)
        ));
        // The points are stored on the node, where the frame is built from.
        assert_eq!(graph.nodes[0].points.len(), 3);
    }

    #[test]
    fn a_double_click_adds_and_removes_a_point() {
        let mut graph = graph();
        double_click(&mut graph, 0.25, 0.8);
        assert_eq!(points(&graph).len(), 4);
        // The handle just added is under the cursor, so a double click on it takes it away.
        double_click(&mut graph, 0.25, 0.8);
        assert_eq!(points(&graph).len(), 3);
        // The end points stay.
        double_click(&mut graph, 0.0, 0.0);
        assert_eq!(points(&graph).len(), 3);
    }

    #[test]
    fn the_curve_stops_at_the_most_points() {
        let mut graph = graph();
        let mut curve = lfo::curve(&graph.nodes[0]);
        for i in 0..40 {
            curve.insert((i as f32 + 0.5) / 40.0, 0.5);
        }
        assert!(curve.is_full());
        lfo::set_curve(&mut graph.nodes[0], &curve);
        // Some empty spot far from every handle.
        double_click(&mut graph, 0.2, 0.05);
        assert_eq!(points(&graph).len(), MAX_CURVE_POINTS);
        assert!(graph.status.is_some());
    }

    /// The other frame nodes have no draggable points.
    #[test]
    fn other_frame_nodes_have_no_handles() {
        for kind in [NodeKind::Harmonics, NodeKind::Sweep, NodeKind::WaveFile] {
            assert!(lfo_handles(&node(kind)).is_none(), "{kind:?}");
        }
        assert!(lfo_handles(&node(NodeKind::CurveFrame)).is_some());
    }
}

mod wave_file {
    use super::*;

    fn graph() -> (Graph, usize) {
        let mut graph = Graph::with(Vec::new(), Vec::new());
        let index = graph.add_node(NodeKind::WaveFile, Point::new(0.0, 0.0));
        (graph, index)
    }

    #[test]
    fn a_load_sets_the_sample_says_the_frame_count_and_asks_for_a_republish() {
        let dir = TempDir::new("load");
        let path = dir.0.join("kick.wav");
        write_wav(&path, 4410);
        let (mut graph, index) = graph();
        graph.start_sample_load(index, path.clone());
        assert_eq!(graph.nodes[index].sample_path, Some(path));
        assert!(wait_for_load(&mut graph), "the poll reports a new sample");
        let data = graph.nodes[index].sample.clone().expect("it loaded");
        let want = slice_frames(&data, SliceMode::Cycles).len();
        assert!(want > 1);
        let text = graph.status.as_ref().unwrap().0.clone();
        assert_eq!(text, format!("Loaded kick.wav: {want} frames"));
        assert_eq!(frames::frame_count(&graph.nodes[index]), Some(want));
    }

    /// The slice dropdown changes the count and the picture.
    #[test]
    fn the_slice_mode_changes_the_count_and_the_trace() {
        let dir = TempDir::new("slice");
        let path = dir.0.join("tone.wav");
        write_wav(&path, 8192);
        let (mut graph, index) = graph();
        graph.start_sample_load(index, path);
        wait_for_load(&mut graph);
        let cycles = frames::frame_count(&graph.nodes[index]).unwrap();
        graph.nodes[index].choose(0, 1);
        assert_eq!(frames::frame_count(&graph.nodes[index]), Some(8));
        assert_ne!(cycles, 8);
    }

    #[test]
    fn a_missing_file_names_the_node() {
        let (mut graph, index) = graph();
        graph.start_sample_load(index, "/no/such/flode/file.wav".into());
        assert!(!wait_for_load(&mut graph));
        let text = graph.status.as_ref().unwrap().0.clone();
        assert!(text.contains("Could not load"), "{text}");
        assert!(text.contains("Wave file"), "{text}");
        assert!(graph.nodes[index].sample.is_none());
    }

    #[test]
    fn the_picker_answer_loads_into_the_node() {
        let dir = TempDir::new("picker");
        let path = dir.0.join("picked.wav");
        write_wav(&path, 2000);
        let (mut graph, index) = graph();
        graph.start_sampler_dialog(index);
        graph
            .sampler_dialog_sender
            .clone()
            .unwrap()
            .send(Some(path))
            .unwrap();
        graph.poll_sampler_dialog();
        assert!(wait_for_load(&mut graph));
        assert!(graph.nodes[index].sample.is_some());
    }
}

mod saving {
    use super::*;

    fn wire(from: usize, to: usize, input: Port) -> Connection {
        Connection {
            from,
            out: Port::Table,
            to,
            input,
        }
    }

    /// Wavetable at 4, curve, harmonics, sweep and file into its frames, then an oscillator.
    fn graph(file: std::path::PathBuf) -> Graph {
        let mut curve = node(NodeKind::CurveFrame);
        curve.points = vec![[0.0, 0.2], [0.4, 0.9], [1.0, 0.3]];
        let mut harmonics = node(NodeKind::Harmonics);
        harmonics.set_value(KnobId::HarmCount, KnobId::HarmCount.to_normalized(7.0));
        harmonics.set_value(KnobId::HarmTilt, KnobId::HarmTilt.to_normalized(0.8));
        let mut sweep = node(NodeKind::Sweep);
        sweep.choose(0, 2);
        sweep.set_value(KnobId::SweepFrames, KnobId::SweepFrames.to_normalized(9.0));
        let mut file_node = node(NodeKind::WaveFile);
        file_node.choose(0, 2);
        file_node.sample_path = Some(file);
        let mut table = node(NodeKind::Wavetable);
        table.set_value(KnobId::WtPosition, 0.6);
        table.choose(0, 1);
        let osc = node(NodeKind::Oscillator);
        let mut graph = Graph::with(
            vec![curve, harmonics, sweep, file_node, table, osc],
            vec![
                wire(0, 4, Port::Frame(0)),
                wire(1, 4, Port::Frame(1)),
                wire(2, 4, Port::Frame(2)),
                wire(3, 4, Port::Frame(3)),
                Connection {
                    from: 4,
                    out: Port::Table,
                    to: 5,
                    input: Port::TableIn,
                },
            ],
        );
        graph.refresh_slots();
        graph
    }

    #[test]
    fn all_the_new_kinds_round_trip_with_their_wires() {
        let dir = TempDir::new("save");
        let file = dir.0.join("w.wav");
        write_wav(&file, 3000);
        let graph = graph(file.clone());
        assert_eq!(graph.nodes[4].slots, 5);
        let json = serde_json::to_string(&graph.to_saved()).unwrap();
        let mut loaded = Graph::from_saved(&serde_json::from_str(&json).unwrap());
        assert_eq!(loaded.nodes.len(), 6);
        assert_eq!(loaded.connections.len(), 5);
        for (a, b) in graph.connections.iter().zip(&loaded.connections) {
            assert_eq!(a, b);
        }
        let kinds: Vec<NodeKind> = loaded.nodes.iter().map(|n| n.kind).collect();
        assert_eq!(
            kinds,
            [
                NodeKind::CurveFrame,
                NodeKind::Harmonics,
                NodeKind::Sweep,
                NodeKind::WaveFile,
                NodeKind::Wavetable,
                NodeKind::Oscillator
            ]
        );
        assert_eq!(loaded.nodes[0].points, graph.nodes[0].points);
        assert!((loaded.nodes[1].plain(KnobId::HarmCount) - 7.0).abs() < 1e-3);
        assert!((loaded.nodes[1].plain(KnobId::HarmTilt) - 0.8).abs() < 1e-3);
        assert_eq!(loaded.nodes[2].choice(0), 2);
        assert!((loaded.nodes[2].plain(KnobId::SweepFrames) - 9.0).abs() < 1e-3);
        assert_eq!(loaded.nodes[3].choice(0), 2);
        assert_eq!(loaded.nodes[3].sample_path, Some(file));
        assert_eq!(loaded.nodes[4].choice(0), 1);
        assert!((loaded.nodes[4].value(KnobId::WtPosition) - 0.6).abs() < 1e-5);
        assert_eq!((loaded.nodes[4].used, loaded.nodes[4].slots), (4, 5));
        // The file loads again in the background.
        assert!(loaded.nodes[3].sample.is_none());
        assert!(wait_for_load(&mut loaded));
        assert!(loaded.nodes[3].sample.is_some());
    }

    #[test]
    fn duplicating_keeps_points_sample_and_slots() {
        let dir = TempDir::new("dup");
        let file = dir.0.join("d.wav");
        write_wav(&file, 3000);
        let mut graph = graph(file);
        graph.start_sample_load(3, graph.nodes[3].sample_path.clone().unwrap());
        wait_for_load(&mut graph);
        let at = Point::new(10.0, 10.0);
        let curve = graph.nodes[0].duplicate(at);
        assert_eq!(curve.points, graph.nodes[0].points);
        let file = graph.nodes[3].duplicate(at);
        assert!(file.sample.is_some());
        assert_eq!(file.sample_path, graph.nodes[3].sample_path);
        assert_eq!(file.choice(0), 2);
        let table = graph.nodes[4].duplicate(at);
        assert_eq!((table.slots, table.used), (5, 4));
        assert_eq!(table.choice(0), 1);
    }

    /// A save written before these kinds existed, as plain JSON, still loads.
    #[test]
    fn an_older_save_still_loads() {
        let json = r#"{
            "version": 3,
            "nodes": [
                {"kind": "Oscillator", "x": 10.0, "y": 20.0},
                {"kind": "Output", "x": 300.0, "y": 20.0},
                {"kind": "Input", "x": 0.0, "y": 20.0}
            ],
            "links": [
                {"from": 0, "out": "Audio", "to": 1, "input": {"Slot": 0}},
                {"from": 2, "out": "Notes", "to": 0, "input": "Notes"}
            ]
        }"#;
        let saved: SavedGraph = serde_json::from_str(json).unwrap();
        let loaded = Graph::from_saved(&saved);
        assert_eq!(loaded.nodes.len(), 3);
        assert_eq!(loaded.connections.len(), 2);
        assert_eq!(loaded.nodes[1].used, 1);
        assert!(loaded.nodes.iter().all(|n| n.sample_path.is_none()));
    }
}

/// Dropping a wavetable node onto a table wire splices it in, the way a filter splices into an
/// audio wire.
mod splice {
    use super::*;

    /// A saw at node 0 wired straight into an oscillator at node 1, far enough apart for a node to
    /// fit between them.
    fn saw_into_oscillator() -> Graph {
        Graph::with(
            vec![
                FlodeNode::new(NodeKind::SawWave, Point::new(0.0, 100.0)),
                FlodeNode::new(NodeKind::Oscillator, Point::new(600.0, 100.0)),
            ],
            vec![Connection {
                from: 0,
                out: Port::Table,
                to: 1,
                input: Port::TableIn,
            }],
        )
    }

    /// Adds a node of `kind` centered on the middle of the first wire.
    fn add_on_the_wire(graph: &mut Graph, kind: NodeKind) -> usize {
        let curve = graph.link_curve(&graph.connections[0]).unwrap();
        let middle = super::super::draw::bezier_point(&curve, 0.5);
        graph.add_node(kind, Point::new(middle.x - 80.0, middle.y - 20.0))
    }

    #[test]
    fn a_wavetable_dropped_on_a_table_wire_takes_it_on_its_first_frame_pin() {
        let mut graph = saw_into_oscillator();
        let table = add_on_the_wire(&mut graph, NodeKind::Wavetable);
        let link = graph
            .link_under(table)
            .expect("the wire from the saw to the oscillator passes under it");
        graph.insert_into(table, link);
        assert!(graph.connections.contains(&Connection {
            from: 0,
            out: Port::Table,
            to: table,
            input: Port::Frame(0),
        }));
        assert!(graph.connections.contains(&Connection {
            from: table,
            out: Port::Table,
            to: 1,
            input: Port::TableIn,
        }));
        assert_eq!(graph.connections.len(), 2);
        // The wired frame pin shows the empty one below it, the same as a wire made by hand.
        assert_eq!(graph.nodes[table].slots, 2);
    }

    #[test]
    fn a_wavetable_feeding_a_wavetable_splices_too() {
        let mut graph = Graph::with(
            vec![
                FlodeNode::new(NodeKind::SawWave, Point::new(0.0, 100.0)),
                FlodeNode::new(NodeKind::Wavetable, Point::new(600.0, 100.0)),
            ],
            vec![Connection {
                from: 0,
                out: Port::Table,
                to: 1,
                input: Port::Frame(0),
            }],
        );
        let table = add_on_the_wire(&mut graph, NodeKind::Wavetable);
        let link = graph.link_under(table).expect("a frame wire splices too");
        graph.insert_into(table, link);
        assert!(graph.connections.contains(&Connection {
            from: table,
            out: Port::Table,
            to: 1,
            input: Port::Frame(0),
        }));
    }

    #[test]
    fn a_frame_node_does_not_splice_since_it_takes_no_table() {
        let mut graph = saw_into_oscillator();
        let square = add_on_the_wire(&mut graph, NodeKind::SquareWave);
        assert_eq!(graph.link_under(square), None);
    }

    #[test]
    fn a_wavetable_does_not_splice_into_an_audio_wire() {
        let mut graph = Graph::with(
            vec![
                FlodeNode::new(NodeKind::Oscillator, Point::new(0.0, 100.0)),
                FlodeNode::new(NodeKind::Output, Point::new(600.0, 100.0)),
            ],
            vec![Connection {
                from: 0,
                out: Port::Audio,
                to: 1,
                input: Port::Slot(0),
            }],
        );
        let table = add_on_the_wire(&mut graph, NodeKind::Wavetable);
        assert_eq!(graph.link_under(table), None);
    }
}
