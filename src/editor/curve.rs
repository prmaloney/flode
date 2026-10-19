//! The map node's transfer curve: where its handles sit and which one a press grabs.

use crate::dsp::effects::eq::BANDS;
use crate::nodes::eq;
use crate::nodes::viewer::Live;
use crate::nodes::FlodeNode;
use nih_plug_iced::Point;

use crate::nodes::map;
pub(super) use crate::nodes::map::{MAP_CENTER_RANGE, MAP_IN, MAP_OUT};
/// How close a press must be to a curve handle to grab it.
pub(super) const HANDLE_HIT_RADIUS: f32 = 9.0;
/// How many points the curve is drawn with.
pub(super) const CURVE_SAMPLES: usize = 32;

/// Where the two handles of a map's curve sit, in editor coordinates. They read `live`, so a value
/// that a wire drives moves its handle. A handle is at an end of the curve, which the offset
/// pushes up or down. A bipolar map's ends are the center less and plus half the range.
pub(super) fn map_handles(node: &FlodeNode, live: &Live) -> [Point; 2] {
    let bounds = node.curve_bounds();
    let (low, high) = map::output_ends(node, live);
    let ends = [low, high];
    std::array::from_fn(|end| {
        let x = live.value(MAP_IN[end]);
        let y = (ends[end] + map::offset(node, live)).clamp(0.0, 1.0);
        Point::new(
            bounds.x + x * bounds.width,
            bounds.y + (1.0 - y) * bounds.height,
        )
    })
}

/// The handle of a map under `point`, if any.
pub(super) fn map_handle_at(node: &FlodeNode, live: &Live, point: Point) -> Option<usize> {
    if node.collapsed || !node.has_curve() {
        return None;
    }
    map_handles(node, live).iter().position(|handle| {
        let dx = handle.x - point.x;
        let dy = handle.y - point.y;
        dx * dx + dy * dy <= HANDLE_HIT_RADIUS * HANDLE_HIT_RADIUS
    })
}

/// Moves end `end` of a map's curve to where `point` is over the curve box. The x is the input and
/// the y is where the end of the curve shows, so the output value is that less `offset`, which is
/// the offset now. A bipolar map has no value for an end, so the other end stays and Center and
/// Range are worked out from the two. A value that a wire drives stays where the wire puts it, and
/// `driven.output` means a wire drives either of a bipolar map's two output values.
pub(super) fn drag_map_handle(
    node: &mut FlodeNode,
    end: usize,
    point: Point,
    offset: f32,
    driven: MapDriven,
) {
    let bounds = node.curve_bounds();
    let x = ((point.x - bounds.x) / bounds.width).clamp(0.0, 1.0);
    let y = (1.0 - (point.y - bounds.y) / bounds.height).clamp(0.0, 1.0);
    if !driven.input {
        node.set_value(MAP_IN[end], x);
    }
    if driven.output {
        return;
    }
    let value = (y - offset).clamp(0.0, 1.0);
    if map::bipolar(node) {
        let (low, high) = map::ends_of(
            node.plain(MAP_CENTER_RANGE[0]),
            node.plain(MAP_CENTER_RANGE[1]),
        );
        let ends = if end == 0 {
            (value, high)
        } else {
            (low, value)
        };
        let (center, range) = map::center_and_range(ends.0, ends.1);
        node.set_value(
            MAP_CENTER_RANGE[0],
            MAP_CENTER_RANGE[0].to_normalized(center),
        );
        node.set_value(
            MAP_CENTER_RANGE[1],
            MAP_CENTER_RANGE[1].to_normalized(range),
        );
    } else {
        node.set_value(MAP_OUT[end], value);
    }
}

/// Which of a map handle's two values a wire drives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct MapDriven {
    pub(super) input: bool,
    pub(super) output: bool,
}

/// Where the four points of an EQ node's graph sit, in editor coordinates. They read `live`, so a
/// knob that a wire drives moves its point. None if the node has no graph on show.
pub(super) fn eq_handles(node: &FlodeNode, live: &Live) -> Option<[Point; BANDS]> {
    let bounds = node.eq_bounds()?;
    Some(std::array::from_fn(|band| {
        let (x, y) = eq::point_of(node, live, band);
        Point::new(
            bounds.x + x * bounds.width,
            bounds.y + (1.0 - y) * bounds.height,
        )
    }))
}

/// The EQ point under `point`, if any. Where points overlap, the nearest one wins, and the higher
/// band wins a tie, since it is drawn last.
pub(super) fn eq_handle_at(node: &FlodeNode, live: &Live, point: Point) -> Option<usize> {
    let handles = eq_handles(node, live)?;
    let distance = |handle: &Point| (handle.x - point.x).hypot(handle.y - point.y);
    handles
        .iter()
        .enumerate()
        .filter(|(_, handle)| distance(handle) <= HANDLE_HIT_RADIUS)
        .min_by(|(a, ha), (b, hb)| {
            distance(ha)
                .partial_cmp(&distance(hb))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.cmp(a))
        })
        .map(|(band, _)| band)
}

/// Which of a band's two values a wire drives, so the drag leaves them alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct EqDriven {
    pub(super) freq: bool,
    pub(super) gain: bool,
}

/// Moves EQ band `band` to where `point` is over the graph. The x sets the frequency. The y sets
/// the gain, unless the band is a cut or a notch, which has no gain. A value that a wire drives
/// stays where the wire puts it.
pub(super) fn drag_eq_point(node: &mut FlodeNode, band: usize, point: Point, driven: EqDriven) {
    let Some(bounds) = node.eq_bounds() else {
        return;
    };
    let x = ((point.x - bounds.x) / bounds.width).clamp(0.0, 1.0);
    let y = (1.0 - (point.y - bounds.y) / bounds.height).clamp(0.0, 1.0);
    if !driven.freq {
        node.set_value(
            eq::FREQ[band],
            eq::FREQ[band].to_normalized(eq::frequency_at(x)),
        );
    }
    if !driven.gain && eq::kind(node, band).uses_gain() {
        node.set_value(eq::GAIN[band], eq::GAIN[band].to_normalized(eq::db_at(y)));
    }
}
