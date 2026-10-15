//! The viewer control. It is a box on a node that draws a shape, such as a waveform, an envelope,
//! or a filter response. It takes no input. Each frame the editor asks the node for a `Trace`,
//! built from the knob values as they are right now, including knobs a control wire drives, so
//! the shape moves when a knob or its wire does.

use super::knobs::KnobId;
use super::FlodeNode;

/// How many points a trace is asked for. The box is about 160 px wide.
pub const TRACE_SAMPLES: usize = 96;

/// What a viewer draws.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Trace {
    /// The shape, left to right at even spacing. Each point is from 0.0 at the bottom of the box
    /// to 1.0 at the top.
    pub points: Vec<f32>,
    /// Vertical ticks, as positions across the box from 0.0 to 1.0. An envelope marks where its
    /// stages end, and a filter marks its cutoff.
    pub markers: Vec<f32>,
    /// True to shade the area under the shape.
    pub fill: bool,
}

/// A node's knob values as they are now. A knob a control wire drives reads the live value of
/// that wire. Every other knob reads the node's own value.
#[derive(Debug, Clone, Copy)]
pub struct Live {
    values: [f32; KnobId::COUNT],
}

impl Live {
    /// The node's own values, with `driven` overriding any knob it returns a value for.
    pub fn new(node: &FlodeNode, driven: impl Fn(KnobId) -> Option<f32>) -> Self {
        let mut values = [0.0; KnobId::COUNT];
        for id in KnobId::ALL {
            values[id as usize] = driven(id).unwrap_or_else(|| node.value(id));
        }
        Self { values }
    }

    /// The node's own values, as when nothing drives any knob.
    #[cfg(test)]
    pub fn of(node: &FlodeNode) -> Self {
        Self::new(node, |_| None)
    }

    /// A knob's value, normalized from 0.0 to 1.0.
    pub fn value(&self, id: KnobId) -> f32 {
        self.values[id as usize]
    }

    /// A knob's value in its own units.
    pub fn plain(&self, id: KnobId) -> f32 {
        id.to_plain(self.value(id))
    }
}

/// Fills `samples` points by calling `f` at positions from 0.0 to 1.0, ends included. A helper for
/// kinds whose shape is a function of one variable.
pub fn sample(samples: usize, f: impl Fn(f32) -> f32) -> Vec<f32> {
    let last = samples.saturating_sub(1).max(1) as f32;
    (0..samples)
        .map(|i| f(i as f32 / last).clamp(0.0, 1.0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::{
        Control, NodeKind, Row, HEADER_HEIGHT, NODE_WIDTH, ROW_PITCH, VIEWER_HEIGHT, VIEWER_ROW,
    };
    use nih_plug_iced::Point;

    /// A viewer row then a knob row, the way a kind would list them.
    fn viewer_then_knob() -> Vec<Row> {
        vec![
            Row::control(Control::Viewer),
            Row::driven_knob(KnobId::Attack),
        ]
    }

    /// The rows under a viewer start below its box, so the box and the knob do not overlap.
    #[test]
    fn rows_after_a_viewer_start_below_it() {
        let node = FlodeNode::new(NodeKind::Envelope, Point::new(10.0, 20.0));
        let rows = viewer_then_knob();
        let places = node.places_of(&rows);
        let bounds = node.viewer_bounds_of(&rows).expect("the viewer is shown");
        assert!(places[1].y - ROW_PITCH / 2.0 >= bounds.y + bounds.height - node.position.y);
    }

    /// The box sits inside the node, clear of its header and its edges.
    #[test]
    fn the_viewer_is_inside_the_node() {
        let node = FlodeNode::new(NodeKind::Envelope, Point::new(10.0, 20.0));
        let rows = viewer_then_knob();
        let bounds = node.viewer_bounds_of(&rows).expect("the viewer is shown");
        assert_eq!(bounds.height, VIEWER_HEIGHT);
        assert!(bounds.x >= node.position.x);
        assert!(bounds.x + bounds.width <= node.position.x + NODE_WIDTH);
        assert!(bounds.y >= node.position.y + HEADER_HEIGHT);
        assert!(bounds.y + bounds.height <= node.position.y + node.height_of(&rows));
    }

    /// The viewer adds its row's height to the node, and nothing else.
    #[test]
    fn the_viewer_makes_the_node_taller_by_its_row() {
        let node = FlodeNode::new(NodeKind::Envelope, Point::new(0.0, 0.0));
        let knob_only = vec![Row::driven_knob(KnobId::Attack)];
        let with_viewer = viewer_then_knob();
        let extra = node.height_of(&with_viewer) - node.height_of(&knob_only);
        assert!((extra - VIEWER_ROW).abs() < 1e-3, "{extra}");
    }

    /// A node folded down to its pins shows no viewer.
    #[test]
    fn a_collapsed_node_hides_the_viewer() {
        let mut node = FlodeNode::new(NodeKind::Envelope, Point::new(0.0, 0.0));
        node.collapsed = true;
        assert!(node.viewer_bounds_of(&viewer_then_knob()).is_none());
    }

    /// A kind with no viewer row has no viewer, and no trace.
    #[test]
    fn a_node_without_a_viewer_has_none() {
        let node = FlodeNode::new(NodeKind::Mix, Point::new(0.0, 0.0));
        assert!(!node.has_viewer());
        assert!(node.trace(&Live::of(&node)).is_none());
    }

    #[test]
    fn sample_includes_both_ends() {
        let points = sample(5, |x| x);
        assert_eq!(points, vec![0.0, 0.25, 0.5, 0.75, 1.0]);
    }

    #[test]
    fn sample_clamps_to_the_box() {
        let points = sample(3, |x| x * 4.0 - 1.0);
        assert_eq!(points, vec![0.0, 1.0, 1.0]);
    }

    /// A driven knob reads the wire's value, and the others read the node's own.
    #[test]
    fn live_prefers_the_driven_value() {
        let mut node = FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0));
        node.set_value(KnobId::Cutoff, 0.25);
        node.set_value(KnobId::Resonance, 0.5);
        let live = Live::new(&node, |id| (id == KnobId::Cutoff).then_some(0.75));
        assert_eq!(live.value(KnobId::Cutoff), 0.75);
        assert_eq!(live.value(KnobId::Resonance), node.value(KnobId::Resonance));
        assert_eq!(
            Live::of(&node).value(KnobId::Cutoff),
            node.value(KnobId::Cutoff)
        );
    }
}
