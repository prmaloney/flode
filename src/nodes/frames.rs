//! The nodes that make frames for a wavetable: a drawn curve, a list of harmonics, a sweep that
//! makes many frames at once, and frames cut from an audio file. Each has one table output, which
//! goes to a frame input of a wavetable node. Their knobs have no pins, since a frame is worked
//! out when the patch is built, not while the sound plays.

use std::cell::RefCell;
use std::sync::Arc;

use super::viewer::{sample, Live, Trace};
use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};
use crate::dsp::curve::Curve;
use crate::dsp::frames::{first_frame, harmonic_frame, sweep_frames, SliceMode, SweepKind};
use crate::dsp::sample::SampleData;

/// The row every frame node starts with: the table output.
fn output_row() -> Row {
    Row::default().with_output(Port::Table)
}

/// A frame's samples, from -1.0 to 1.0, as the box draws them: `samples` points from 0.0 to 1.0.
fn frame_points(frame: &[f32], samples: usize) -> Vec<f32> {
    if frame.is_empty() {
        return vec![0.5; samples];
    }
    sample(samples, |x| {
        let at = (x * frame.len() as f32).round() as usize;
        frame[at.min(frame.len() - 1)] * 0.5 + 0.5
    })
}

/// How many traces the cache below keeps.
const CACHE_SIZE: usize = 16;

thread_local! {
    /// Traces that took real work to make, by the numbers that made them. The viewer asks for a
    /// trace every frame, and a frame from a recipe or a file is too slow to build that often.
    static CACHE: RefCell<Vec<([u64; 6], Vec<f32>)>> = const { RefCell::new(Vec::new()) };
}

/// The trace points for `key`, from the cache, or from `make` if they are not in it.
fn cached(key: [u64; 6], make: impl FnOnce() -> Vec<f32>) -> Vec<f32> {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((_, points)) = cache.iter().find(|(k, _)| *k == key) {
            return points.clone();
        }
        let points = make();
        if cache.len() >= CACHE_SIZE {
            cache.remove(0);
        }
        cache.push((key, points.clone()));
        points
    })
}

pub struct CurveFrame;

impl NodeBehavior for CurveFrame {
    fn title(&self) -> &'static str {
        "Curve frame"
    }

    /// The drawn cycle is a viewer with points to drag, like an LFO's custom wave.
    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![output_row(), Row::control(Control::Viewer)]
    }

    /// The curve as drawn, straight between its points, so the handles sit on the line. The frame
    /// itself also has its average removed, which this leaves out.
    fn trace(&self, node: &FlodeNode, _live: &Live, samples: usize) -> Option<Trace> {
        let curve = Curve::from_points(&node.points).unwrap_or_default();
        Some(Trace {
            points: sample(samples, |x| curve.eval(x)),
            ..Trace::default()
        })
    }
}

pub struct Harmonics;

impl NodeBehavior for Harmonics {
    fn title(&self) -> &'static str {
        "Harmonics"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            output_row(),
            Row::control(Control::Viewer),
            Row::plain_knob(KnobId::HarmCount),
            Row::plain_knob(KnobId::HarmTilt),
            Row::plain_knob(KnobId::HarmOddEven),
        ]
    }

    /// The frame the knobs make.
    fn trace(&self, _node: &FlodeNode, live: &Live, samples: usize) -> Option<Trace> {
        let count = live.plain(KnobId::HarmCount).round().max(1.0) as usize;
        let tilt = live.plain(KnobId::HarmTilt);
        let odd_even = live.plain(KnobId::HarmOddEven);
        let key = [
            1,
            count as u64,
            tilt.to_bits() as u64,
            odd_even.to_bits() as u64,
            0,
            samples as u64,
        ];
        let points = cached(key, || {
            frame_points(&harmonic_frame(count, tilt, odd_even), samples)
        });
        Some(Trace {
            points,
            ..Trace::default()
        })
    }
}

/// The Kind dropdown's rows for a sweep: what changes from the first frame to the last.
pub const SWEEP_LABELS: [&str; 4] = ["Harmonics", "PWM", "Sync", "Fold"];

pub struct Sweep;

impl NodeBehavior for Sweep {
    fn title(&self) -> &'static str {
        "Sweep"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            output_row(),
            Row::control(Control::Viewer),
            Row::control(Control::Dropdown(0)),
            Row::plain_knob(KnobId::SweepFrames),
            Row::plain_knob(KnobId::SweepAmount),
        ]
    }

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&SWEEP_LABELS]
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Kind"]
    }

    /// The last frame of the sweep, where it ends. A trace has no faint second line, so the first
    /// frame is not drawn.
    fn trace(&self, node: &FlodeNode, live: &Live, samples: usize) -> Option<Trace> {
        let kind = SweepKind::from_index(sweep_index(node));
        let amount = live.plain(KnobId::SweepAmount);
        let key = [
            2,
            kind.index() as u64,
            amount.to_bits() as u64,
            0,
            0,
            samples as u64,
        ];
        let points = cached(key, || {
            // With two frames, the second is the last one of any longer sweep.
            let frames = sweep_frames(kind, 2, amount);
            frames
                .last()
                .map_or_else(|| vec![0.5; samples], |frame| frame_points(frame, samples))
        });
        Some(Trace {
            points,
            ..Trace::default()
        })
    }
}

/// The Slice dropdown's rows for a file: cut one pitch period per frame, or a fixed number of
/// samples.
pub const SLICE_LABELS: [&str; 4] = ["Cycles", "1024", "2048", "4096"];

pub struct WaveFile;

impl NodeBehavior for WaveFile {
    fn title(&self) -> &'static str {
        "Wave file"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            output_row(),
            Row::control(Control::LoadSample),
            Row::control(Control::Viewer),
            Row::control(Control::Dropdown(0)),
        ]
    }

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&SLICE_LABELS]
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Slice"]
    }

    /// The first frame the file will make, or a flat line with nothing loaded.
    fn trace(&self, node: &FlodeNode, _live: &Live, samples: usize) -> Option<Trace> {
        let flat = vec![0.5; samples];
        let Some(data) = &node.sample else {
            return Some(Trace {
                points: flat,
                ..Trace::default()
            });
        };
        let mode = slice_mode(node);
        let key = [
            3,
            Arc::as_ptr(data) as usize as u64,
            data.len() as u64,
            data.sample_rate.to_bits() as u64,
            slice_index(node) as u64,
            samples as u64,
        ];
        let points = cached(key, || {
            first_frame(data, mode).map_or(flat, |frame| frame_points(&frame, samples))
        });
        Some(Trace {
            points,
            ..Trace::default()
        })
    }
}

/// The sweep a node has chosen, as an index into `SWEEP_LABELS`.
pub fn sweep_index(node: &FlodeNode) -> usize {
    node.choice(0).min(SWEEP_LABELS.len() - 1)
}

/// The slice mode a file node has chosen, as an index into `SLICE_LABELS`.
pub fn slice_index(node: &FlodeNode) -> usize {
    node.choice(0).min(SLICE_LABELS.len() - 1)
}

/// How a file node cuts its file, from its Slice dropdown.
pub fn slice_mode(node: &FlodeNode) -> SliceMode {
    match slice_index(node) {
        0 => SliceMode::Cycles,
        1 => SliceMode::Fixed(1024),
        2 => SliceMode::Fixed(2048),
        _ => SliceMode::Fixed(4096),
    }
}

/// How many frames a file node's loaded file will make, or None with nothing loaded.
pub fn frame_count(node: &FlodeNode) -> Option<usize> {
    let data: &Arc<SampleData> = node.sample.as_ref()?;
    Some(crate::dsp::frames::frame_count(data, slice_mode(node)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    fn node(kind: NodeKind) -> FlodeNode {
        FlodeNode::new(kind, Point::new(0.0, 0.0))
    }

    /// Every frame node makes a table and takes no signal of its own.
    #[test]
    fn frame_nodes_make_a_table_and_take_no_wires() {
        for kind in [
            NodeKind::CurveFrame,
            NodeKind::Harmonics,
            NodeKind::Sweep,
            NodeKind::WaveFile,
        ] {
            let node = node(kind);
            assert_eq!(node.output_ports(), vec![Port::Table], "{kind:?}");
            assert!(node.input_ports().is_empty(), "{kind:?}");
        }
    }

    fn points_of(node: &FlodeNode) -> Vec<f32> {
        node.trace(&Live::of(node)).expect("a viewer").points
    }

    fn sine_file() -> Arc<SampleData> {
        Arc::new(SampleData {
            sample_rate: 44100.0,
            channels: 1,
            frames: (0..8192)
                .map(|n| (std::f32::consts::TAU * n as f32 / 128.0).sin() * 0.5)
                .collect(),
        })
    }

    /// Every frame node draws a full box of points in range.
    #[test]
    fn every_trace_has_samples_in_range() {
        let mut file = node(NodeKind::WaveFile);
        file.sample = Some(sine_file());
        for node in [
            node(NodeKind::CurveFrame),
            node(NodeKind::Harmonics),
            node(NodeKind::Sweep),
            file,
        ] {
            let points = points_of(&node);
            assert_eq!(points.len(), crate::nodes::viewer::TRACE_SAMPLES);
            assert!(points.iter().all(|p| (0.0..=1.0).contains(p)));
            assert!(
                points.iter().any(|&p| (p - 0.5).abs() > 0.1),
                "{:?}",
                node.kind
            );
        }
    }

    #[test]
    fn the_traces_follow_their_knobs() {
        let mut harmonics = node(NodeKind::Harmonics);
        let before = points_of(&harmonics);
        harmonics.set_value(KnobId::HarmCount, KnobId::HarmCount.to_normalized(3.0));
        assert_ne!(points_of(&harmonics), before);
        let before = points_of(&harmonics);
        harmonics.set_value(KnobId::HarmTilt, KnobId::HarmTilt.to_normalized(0.1));
        assert_ne!(points_of(&harmonics), before);

        let mut sweep = node(NodeKind::Sweep);
        let mut seen = Vec::new();
        for kind in 0..4 {
            sweep.choose(0, kind);
            let points = points_of(&sweep);
            assert!(!seen.contains(&points), "kind {kind} repeats");
            seen.push(points);
        }
        sweep.choose(0, 0);
        let end = points_of(&sweep);
        sweep.set_value(KnobId::SweepAmount, KnobId::SweepAmount.to_normalized(0.0));
        assert_ne!(points_of(&sweep), end);
    }

    #[test]
    fn a_curve_trace_is_the_drawn_curve() {
        let mut node = node(NodeKind::CurveFrame);
        node.points = vec![[0.0, 0.0], [0.5, 1.0], [1.0, 0.25]];
        let points = points_of(&node);
        assert_eq!(points[0], 0.0);
        assert_eq!(points[points.len() - 1], 0.25);
        let middle = points[(points.len() - 1) / 2];
        assert!(middle > 0.95, "{middle}");
    }

    #[test]
    fn a_wave_file_is_flat_until_it_has_a_file() {
        let mut node = node(NodeKind::WaveFile);
        assert!(points_of(&node).iter().all(|&p| p == 0.5));
        assert_eq!(frame_count(&node), None);
        node.sample = Some(sine_file());
        let cycles = points_of(&node);
        assert!(cycles.iter().any(|&p| p != 0.5));
        // About one frame for each of the 64 periods in the file.
        assert!(frame_count(&node).is_some_and(|n| (60..=64).contains(&n)));
        node.choose(0, 3);
        assert_eq!(slice_mode(&node), SliceMode::Fixed(4096));
        assert_eq!(frame_count(&node), Some(2));
    }

    #[test]
    fn the_dropdowns_have_a_row_for_each_choice() {
        assert_eq!(SWEEP_LABELS.len(), 4);
        assert_eq!(SLICE_LABELS.len(), 4);
        assert_eq!(sweep_index(&node(NodeKind::Sweep)), 0);
        assert_eq!(slice_index(&node(NodeKind::WaveFile)), 0);
    }
}
