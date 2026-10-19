//! The map node. It rescales a control signal. The input range says which part of the incoming
//! signal to read, and the output range says where that part lands. Inputs outside the input range
//! hold at the nearest end. The curve on the node shows the mapping and where the live signal is.
//!
//! The ends of the ranges, and an offset that pushes the whole curve up or down, are knobs in a
//! folded section, each with a pin, so a control source can drive them. A driven end moves its
//! handle on the curve. The handles on the curve set the same values by dragging.
//!
//! The Bipolar switch gives the output range as a center and a range. With it on, the Out min and
//! Out max knobs give way to Center, the middle of the output swing, and Range, how wide it is.
//! The swing then runs from half the range below the center to half the range above it, so
//! Center sets where the swing sits and Range sets how far it goes. A negative range runs it
//! backward, so the map inverts. Switching converts the stored values, so the signal does not
//! change when the switch does.

use super::viewer::Live;
use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row, Section};
use crate::patch::control::MapStage;

pub struct Map;

/// The map knobs for the low and high ends of the curve: the input and output of each.
pub const MAP_IN: [KnobId; 2] = [KnobId::MapInMin, KnobId::MapInMax];
pub const MAP_OUT: [KnobId; 2] = [KnobId::MapMin, KnobId::MapMax];
/// The output swing as a middle and a width, which a bipolar map shows in place of `MAP_OUT`.
pub const MAP_CENTER_RANGE: [KnobId; 2] = [KnobId::MapCenter, KnobId::MapRange];
/// The knob that pushes the whole curve up or down.
pub const MAP_OFFSET: KnobId = KnobId::MapOffset;

impl NodeBehavior for Map {
    fn title(&self) -> &'static str {
        "Map"
    }

    fn rows(&self, node: &FlodeNode) -> Vec<Row> {
        // The output swing is two knobs, as its ends or as a center and a range. A knob that is
        // not on show has no pin, so a wire never points at one.
        let mut rows = vec![
            Row::input(Port::Shape).with_output(Port::Shape),
            Row::control(Control::Curve),
            Row::control(Control::Switch(0)),
            // The values fold away, since the handles on the curve set them. A folded section
            // keeps its pins, so a wire can still drive them.
            Row::section(0),
            Row::driven_knob(KnobId::MapInMin),
            Row::driven_knob(KnobId::MapInMax),
        ];
        if bipolar(node) {
            // Center moves the swing as Offset does, so a bipolar map has no Offset.
            rows.push(Row::driven_knob(MAP_CENTER_RANGE[0]));
            rows.push(Row::driven_knob(MAP_CENTER_RANGE[1]));
        } else {
            rows.push(Row::driven_knob(MAP_OUT[0]));
            rows.push(Row::driven_knob(MAP_OUT[1]));
            rows.push(Row::driven_knob(MAP_OFFSET));
        }
        rows
    }

    fn sections(&self) -> &'static [Section] {
        &[Section {
            title: "Range",
            collapsed: true,
        }]
    }

    fn switch_labels(&self) -> &'static [&'static str] {
        &["Bipolar"]
    }
}

/// True if the map gives its output swing as a center and a range.
pub fn bipolar(node: &FlodeNode) -> bool {
    node.switch(0)
}

/// The middle and the width of a swing from `low` to `high`. The width is negative when the swing
/// runs backward.
pub fn center_and_range(low: f32, high: f32) -> (f32, f32) {
    ((low + high) / 2.0, high - low)
}

/// The two ends of a swing with this middle and width. An end can land outside 0 to 1, where the
/// map's output stops.
pub fn ends_of(center: f32, range: f32) -> (f32, f32) {
    (center - range / 2.0, center + range / 2.0)
}

/// The two ends of the output swing as `live` has them, from whichever pair of knobs the map is
/// showing. A knob a wire drives reads where its source puts it.
pub fn output_ends(node: &FlodeNode, live: &Live) -> (f32, f32) {
    if bipolar(node) {
        ends_of(live.plain(KnobId::MapCenter), live.plain(KnobId::MapRange))
    } else {
        (live.plain(KnobId::MapMin), live.plain(KnobId::MapMax))
    }
}

/// The offset as `live` has it. A bipolar map shows no Offset, and its Center does the same job,
/// so whatever the knob holds is not applied.
pub fn offset(node: &FlodeNode, live: &Live) -> f32 {
    if bipolar(node) {
        0.0
    } else {
        live.plain(MAP_OFFSET)
    }
}

/// The map's stage with every value held where `live` has it. The editor draws this, so a driven
/// value shows where its source has put it.
pub fn live_stage(node: &FlodeNode, live: &Live) -> MapStage {
    let (out_lo, out_hi) = output_ends(node, live);
    MapStage::fixed(
        live.plain(KnobId::MapInMin),
        live.plain(KnobId::MapInMax),
        out_lo,
        out_hi,
    )
    .with_offset(offset(node, live))
}

/// The knobs that only a unipolar map shows, and the ones that only a bipolar map shows, so a
/// switch can say which knobs left.
const UNIPOLAR_ONLY: [KnobId; 3] = [KnobId::MapMin, KnobId::MapMax, KnobId::MapOffset];
const BIPOLAR_ONLY: [KnobId; 2] = MAP_CENTER_RANGE;

/// Switches the map's polarity. The stored values that the new form shows are worked out from the
/// ones the old form had, so the signal stays what it was. Going bipolar folds the offset into the
/// center, since Center moves the swing the same way, and clears the offset. Returns the knobs that
/// went off show, whose wires no longer have a pin to end on, or None if the map was already that
/// way.
///
/// The center stops at the ends of its knob, so an offset that would take it past one loses the
/// part past the end.
pub fn set_bipolar(node: &mut FlodeNode, on: bool) -> Option<&'static [KnobId]> {
    if on == bipolar(node) {
        return None;
    }
    if on {
        let (center, range) =
            center_and_range(node.plain(KnobId::MapMin), node.plain(KnobId::MapMax));
        let center = (center + node.plain(MAP_OFFSET)).clamp(0.0, 1.0);
        node.set_value(KnobId::MapCenter, KnobId::MapCenter.to_normalized(center));
        node.set_value(KnobId::MapRange, KnobId::MapRange.to_normalized(range));
        node.set_value(MAP_OFFSET, MAP_OFFSET.to_normalized(0.0));
    } else {
        let (low, high) = ends_of(node.plain(KnobId::MapCenter), node.plain(KnobId::MapRange));
        node.set_value(
            KnobId::MapMin,
            KnobId::MapMin.to_normalized(low.clamp(0.0, 1.0)),
        );
        node.set_value(
            KnobId::MapMax,
            KnobId::MapMax.to_normalized(high.clamp(0.0, 1.0)),
        );
    }
    node.set_switch(0, on);
    Some(if on { &UNIPOLAR_ONLY } else { &BIPOLAR_ONLY })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    fn node() -> FlodeNode {
        FlodeNode::new(NodeKind::Map, Point::new(0.0, 0.0))
    }

    fn knobs(node: &FlodeNode) -> Vec<KnobId> {
        node.rows().iter().filter_map(|row| row.knob).collect()
    }

    /// The ranges and the offset are pinned knobs in a section that starts folded, under the curve
    /// and the polarity checkbox.
    #[test]
    fn the_ranges_are_pinned_knobs_in_a_folded_section() {
        let node = node();
        let rows = node.rows();
        assert_eq!(rows[1].control, Some(Control::Curve));
        assert_eq!(rows[2].control, Some(Control::Switch(0)));
        assert_eq!(
            knobs(&node),
            [MAP_IN[0], MAP_IN[1], MAP_OUT[0], MAP_OUT[1], MAP_OFFSET]
        );
        assert!(rows
            .iter()
            .filter(|row| row.knob.is_some())
            .all(|row| row.input == Some(Port::Knob(row.knob.unwrap()))));
        assert!(node.section_collapsed(0));
        assert_eq!(node.switch_labels(), &["Bipolar"]);
        // The folded section keeps its pins.
        assert_eq!(node.input_ports().len(), 1 + 5);
    }

    /// Bipolar swaps Out min and Out max for Center and Range, with a pin on each, and drops the
    /// Offset, which Center makes redundant.
    #[test]
    fn bipolar_shows_center_and_range_in_place_of_the_ends_and_the_offset() {
        let mut node = node();
        node.set_switch(0, true);
        assert_eq!(
            knobs(&node),
            [
                MAP_IN[0],
                MAP_IN[1],
                MAP_CENTER_RANGE[0],
                MAP_CENTER_RANGE[1]
            ]
        );
        let ports = node.input_ports();
        assert_eq!(ports.len(), 1 + 4);
        assert!(ports.contains(&Port::Knob(KnobId::MapCenter)));
        assert!(!ports.contains(&Port::Knob(KnobId::MapMin)));
        assert!(!ports.contains(&Port::Knob(KnobId::MapOffset)));
        assert_eq!(KnobId::MapCenter.name(), "Center");
        assert_eq!(KnobId::MapRange.name(), "Range");
    }

    /// Nothing sits on top of anything else: the curve, the checkbox, and the section's header
    /// each have their own space, in order.
    #[test]
    fn the_curve_checkbox_and_header_do_not_overlap() {
        let node = node();
        let curve = node.curve_bounds();
        let checkbox = node.switch_bounds(0);
        assert!(checkbox.y >= curve.y + curve.height);
        let header = node
            .row_places()
            .iter()
            .find_map(|place| match place.shape {
                crate::nodes::RowShape::SectionHeader(_) => Some(place.y),
                _ => None,
            })
            .unwrap();
        assert!(
            node.position.y + header - crate::nodes::SECTION_HEIGHT / 2.0
                >= checkbox.y + checkbox.height
        );
        assert!(node.height() > curve.height);
    }

    #[test]
    fn a_new_map_is_the_identity_and_unipolar() {
        let node = node();
        assert!(!bipolar(&node));
        let stage = live_stage(&node, &Live::of(&node));
        for x in [0.0, 0.3, 1.0] {
            assert!((stage.apply(x) - x).abs() < 1e-6);
        }
    }

    #[test]
    fn center_and_range_are_the_middle_and_width_of_the_swing() {
        assert_eq!(center_and_range(0.0, 1.0), (0.5, 1.0));
        assert_eq!(center_and_range(0.35, 0.65).0, 0.5);
        assert!((center_and_range(0.35, 0.65).1 - 0.3).abs() < 1e-6);
        // A backward swing has a negative width.
        assert_eq!(center_and_range(1.0, 0.0), (0.5, -1.0));
        for (low, high) in [(0.0, 1.0), (0.2, 0.6), (1.0, 0.0), (0.4, 0.4)] {
            let (center, range) = center_and_range(low, high);
            let (l, h) = ends_of(center, range);
            assert!((l - low).abs() < 1e-6 && (h - high).abs() < 1e-6);
        }
    }

    /// A new bipolar map is the full swing: centered at 50 percent, and 100 percent wide.
    #[test]
    fn the_new_center_and_range_are_the_full_swing() {
        let node = node();
        assert_eq!(node.plain(KnobId::MapCenter), 0.5);
        assert_eq!(node.plain(KnobId::MapRange), 1.0);
    }

    /// Switching to bipolar works out Center and Range from the ends and the offset, and back
    /// again, so the signal is the same before and after, both ways. The offset goes into Center,
    /// so it is zero once the map is back, and the ends carry it instead.
    #[test]
    fn switching_polarity_keeps_the_signal() {
        let signal = |node: &FlodeNode| {
            let stage = live_stage(node, &Live::of(node));
            [0.0, 0.25, 0.5, 0.75, 1.0].map(|x| stage.apply(x))
        };
        let mut node = node();
        node.set_value(KnobId::MapMin, KnobId::MapMin.to_normalized(0.35));
        node.set_value(KnobId::MapMax, KnobId::MapMax.to_normalized(0.65));
        node.set_value(KnobId::MapOffset, KnobId::MapOffset.to_normalized(0.1));
        let before = signal(&node);
        set_bipolar(&mut node, true);
        assert!(bipolar(&node));
        // The swing is 30 percent wide, and the 10 percent offset moved its center to 60. The
        // offset is cleared.
        assert!((node.plain(KnobId::MapCenter) - 0.6).abs() < 1e-5);
        assert!((node.plain(KnobId::MapRange) - 0.3).abs() < 1e-5);
        assert_eq!(node.plain(KnobId::MapOffset), 0.0);
        for (a, b) in before.iter().zip(signal(&node)) {
            assert!((a - b).abs() < 1e-5);
        }
        set_bipolar(&mut node, false);
        assert!(!bipolar(&node));
        // The ends are where the offset put them, and the offset stays at zero.
        assert!((node.plain(KnobId::MapMin) - 0.45).abs() < 1e-5);
        assert!((node.plain(KnobId::MapMax) - 0.75).abs() < 1e-5);
        assert_eq!(node.plain(KnobId::MapOffset), 0.0);
        for (a, b) in before.iter().zip(signal(&node)) {
            assert!((a - b).abs() < 1e-5);
        }
    }

    /// An offset the bipolar map still holds is not applied, since Offset is not on show.
    #[test]
    fn a_hidden_offset_is_not_applied() {
        let mut node = node();
        node.set_switch(0, true);
        node.set_value(KnobId::MapOffset, KnobId::MapOffset.to_normalized(0.4));
        let live = Live::of(&node);
        assert_eq!(offset(&node, &live), 0.0);
        let stage = live_stage(&node, &live);
        assert!((stage.apply(0.0)).abs() < 1e-6);
        assert!((stage.apply(1.0) - 1.0).abs() < 1e-6);
        // A unipolar map applies it.
        node.set_switch(0, false);
        assert!((offset(&node, &Live::of(&node)) - 0.4).abs() < 1e-5);
    }

    /// An offset that would take the center past an end of its knob loses what is past it.
    #[test]
    fn an_offset_past_the_edge_stops_at_the_centers_end() {
        let mut node = node();
        node.set_value(KnobId::MapMin, KnobId::MapMin.to_normalized(0.6));
        node.set_value(KnobId::MapMax, KnobId::MapMax.to_normalized(1.0));
        node.set_value(KnobId::MapOffset, KnobId::MapOffset.to_normalized(0.5));
        set_bipolar(&mut node, true);
        // The center was 80 and the offset adds 50, but the center stops at 100.
        assert!((node.plain(KnobId::MapCenter) - 1.0).abs() < 1e-5);
    }

    /// An inverting map keeps its direction through the switch, because the range is signed.
    #[test]
    fn an_inverting_map_survives_the_switch() {
        let mut node = node();
        node.set_value(KnobId::MapMin, 1.0);
        node.set_value(KnobId::MapMax, 0.0);
        set_bipolar(&mut node, true);
        assert!((node.plain(KnobId::MapRange) + 1.0).abs() < 1e-5);
        let stage = live_stage(&node, &Live::of(&node));
        assert!((stage.apply(0.0) - 1.0).abs() < 1e-5);
        assert!(stage.apply(1.0).abs() < 1e-5);
        set_bipolar(&mut node, false);
        assert!((node.plain(KnobId::MapMin) - 1.0).abs() < 1e-5);
        assert!(node.plain(KnobId::MapMax).abs() < 1e-5);
    }

    /// The knobs that went off show are reported, so their wires can go.
    #[test]
    fn switching_reports_the_knobs_that_left() {
        let mut node = node();
        // Out min, Out max and Offset leave when it goes bipolar, and Center and Range leave when
        // it comes back.
        assert_eq!(
            set_bipolar(&mut node, true),
            Some(&[MAP_OUT[0], MAP_OUT[1], MAP_OFFSET][..])
        );
        assert_eq!(set_bipolar(&mut node, false), Some(&MAP_CENTER_RANGE[..]));
        // Setting it the way it already is changes nothing and reports nothing.
        assert_eq!(set_bipolar(&mut node, false), None);
    }

    /// A center near an edge puts an end outside 0 to 1, and the output stops there.
    #[test]
    fn a_swing_past_the_edge_stops_at_the_edge() {
        let mut node = node();
        node.set_switch(0, true);
        node.set_value(KnobId::MapCenter, KnobId::MapCenter.to_normalized(0.9));
        node.set_value(KnobId::MapRange, KnobId::MapRange.to_normalized(0.3));
        let stage = live_stage(&node, &Live::of(&node));
        assert!((stage.apply(0.0) - 0.75).abs() < 1e-5);
        assert_eq!(stage.apply(1.0), 1.0);
    }
}
