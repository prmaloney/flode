//! The map node's transfer curve: where its handles sit and which one a press grabs.

use crate::nodes::knobs::KnobId;
use crate::nodes::FlodeNode;
use nih_plug_iced::Point;

/// The map knobs for the low and high ends of the curve: the input and output of each.
pub(super) const MAP_IN: [KnobId; 2] = [KnobId::MapInMin, KnobId::MapInMax];
pub(super) const MAP_OUT: [KnobId; 2] = [KnobId::MapMin, KnobId::MapMax];
/// How close a press must be to a curve handle to grab it.
pub(super) const HANDLE_HIT_RADIUS: f32 = 9.0;
/// How many points the curve is drawn with.
pub(super) const CURVE_SAMPLES: usize = 32;

/// Where the two handles of a map's curve sit, in editor coordinates.
pub(super) fn map_handles(node: &FlodeNode) -> [Point; 2] {
    let bounds = node.curve_bounds();
    std::array::from_fn(|end| {
        let x = node.value(MAP_IN[end]);
        let y = node.value(MAP_OUT[end]);
        Point::new(
            bounds.x + x * bounds.width,
            bounds.y + (1.0 - y) * bounds.height,
        )
    })
}

/// The handle of a map under `point`, if any.
pub(super) fn map_handle_at(node: &FlodeNode, point: Point) -> Option<usize> {
    if node.collapsed || !node.has_curve() {
        return None;
    }
    map_handles(node).iter().position(|handle| {
        let dx = handle.x - point.x;
        let dy = handle.y - point.y;
        dx * dx + dy * dy <= HANDLE_HIT_RADIUS * HANDLE_HIT_RADIUS
    })
}
