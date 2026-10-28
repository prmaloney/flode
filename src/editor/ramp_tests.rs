//! Tests for the wavetable node's ramp: its layout, the handles, and linking regions to pins.

use super::canvas::{Message, NodeEditor};
use super::graph::{Graph, Interaction};
use super::ramp::*;
use crate::nodes::connection::Connection;
use crate::nodes::knobs::KnobId;
use crate::nodes::viewer::Live;
use crate::nodes::{FlodeNode, NodeKind, Port, HEADER_HEIGHT, RAMP_ROW};
use crate::patch::control::ControlLevels;
use nih_plug_iced::mouse;
use nih_plug_iced::widget::canvas::{event::Event, Cursor, Frame, Program};
use nih_plug_iced::{Point, Rectangle};

const BOUNDS: Rectangle = Rectangle {
    x: 0.0,
    y: 0.0,
    width: 1024.0,
    height: 712.0,
};

/// A wavetable node at node 0, and a frame node for each of `slots`, wired to those frame inputs.
fn graph_with(slots: &[usize]) -> Graph {
    let mut nodes = vec![FlodeNode::new(
        NodeKind::Wavetable,
        Point::new(300.0, 200.0),
    )];
    for _ in slots {
        nodes.push(FlodeNode::new(NodeKind::CurveFrame, Point::new(0.0, 0.0)));
    }
    // The wires go in directly, since a gap is what unplugging a middle input leaves behind.
    let links = slots
        .iter()
        .enumerate()
        .map(|(i, &slot)| Connection {
            from: i + 1,
            out: Port::Table,
            to: 0,
            input: Port::Frame(slot),
        })
        .collect();
    let mut graph = Graph::with(nodes, links);
    graph.refresh_slots();
    graph
}

fn layout(graph: &Graph) -> RampLayout {
    ramp_layout(graph, 0).expect("the ramp is on show")
}

fn share(graph: &Graph, slot: usize) -> f32 {
    graph.nodes[0].plain(KnobId::share(slot))
}

fn set_share(graph: &mut Graph, slot: usize, plain: f32) {
    let id = KnobId::share(slot);
    graph.nodes[0].set_value(id, id.to_normalized(plain));
}

fn send(graph: &mut Graph, event: mouse::Event, at: Point) -> Option<Message> {
    let mut editor = NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2]);
    editor
        .update(Event::Mouse(event), BOUNDS, Cursor::Available(at))
        .1
}

fn press(graph: &mut Graph, at: Point) {
    send(graph, mouse::Event::ButtonPressed(mouse::Button::Left), at);
}

fn release(graph: &mut Graph, at: Point) {
    send(graph, mouse::Event::ButtonReleased(mouse::Button::Left), at);
}

fn move_to(graph: &mut Graph, at: Point) -> Option<Message> {
    send(graph, mouse::Event::CursorMoved { position: at }, at)
}

fn middle(rect: Rectangle) -> Point {
    Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
}

#[test]
fn no_connected_inputs_gives_an_empty_bar() {
    let graph = graph_with(&[]);
    let layout = layout(&graph);
    assert_eq!(layout.count(), 0);
    assert!(layout.slots().is_empty());
    assert_eq!(layout.handle_at(middle(layout.bar)), None);
    assert_eq!(layout.slot_at(middle(layout.bar)), None);
}

#[test]
fn one_connected_input_fills_the_bar_with_no_handle() {
    let graph = graph_with(&[0]);
    let layout = layout(&graph);
    assert_eq!(layout.count(), 1);
    assert_eq!(layout.slots(), &[0]);
    let rect = layout.region_rect(0);
    assert_eq!((rect.x, rect.width), (layout.bar.x, layout.bar.width));
    assert_eq!(layout.handle_at(middle(layout.bar)), None);
}

#[test]
fn three_equal_inputs_split_the_bar_in_thirds() {
    let graph = graph_with(&[0, 1, 2]);
    let layout = layout(&graph);
    assert_eq!(layout.count(), 3);
    for region in 0..3 {
        let rect = layout.region_rect(region);
        assert!((rect.width - layout.bar.width / 3.0).abs() < 1e-3);
    }
    let first = layout.handle_x(0) - layout.bar.x;
    assert!((first - layout.bar.width / 3.0).abs() < 1e-3);
}

/// Inputs on pins 1 and 3 only give two regions, colored by their pins.
#[test]
fn a_gap_keeps_each_regions_pin_number() {
    let graph = graph_with(&[0, 2]);
    let layout = layout(&graph);
    assert_eq!(layout.count(), 2);
    assert_eq!(layout.slots(), &[0, 2]);
    let left = Point::new(layout.bar.x + 3.0, layout.bar.y + 3.0);
    let right = Point::new(layout.bar.x + layout.bar.width - 3.0, layout.bar.y + 3.0);
    assert_eq!(layout.slot_at(left), Some(0));
    assert_eq!(layout.slot_at(right), Some(2));
}

#[test]
fn shares_set_the_region_widths() {
    let mut graph = graph_with(&[0, 1]);
    set_share(&mut graph, 0, 0.75);
    set_share(&mut graph, 1, 0.25);
    let layout = layout(&graph);
    assert!((layout.region_rect(0).width - 0.75 * layout.bar.width).abs() < 1e-2);
}

#[test]
fn a_share_transfer_keeps_the_total_and_the_other_boundaries() {
    let mut shares = [0.4, 0.3, 0.2, 0.1];
    let total: f32 = shares.iter().sum();
    let edges = |s: &[f32]| -> Vec<f32> {
        let mut run = 0.0;
        s.iter()
            .map(|v| {
                run += v;
                run / s.iter().sum::<f32>()
            })
            .collect()
    };
    let before = edges(&shares);
    transfer_share(&mut shares, 1, 0.55, 0.02, 1.0);
    let after = edges(&shares);
    assert!((shares.iter().sum::<f32>() - total).abs() < 1e-6);
    assert!((after[1] - 0.55).abs() < 1e-5);
    assert_eq!(after[0], before[0]);
    assert!((after[2] - before[2]).abs() < 1e-6);
}

#[test]
fn a_share_transfer_clamps_at_the_knob_range() {
    let mut shares = [0.5, 0.5, 0.5];
    transfer_share(&mut shares, 0, 0.0, 0.02, 1.0);
    assert!((shares[0] - 0.02).abs() < 1e-6);
    assert!((shares[1] - 0.98).abs() < 1e-6 || (shares[1] - 1.0).abs() < 1e-6);
    assert!(shares.iter().all(|&s| (0.02 - 1e-6..=1.0).contains(&s)));
    assert!((shares.iter().sum::<f32>() - 1.5).abs() < 1e-6);
    let mut shares = [0.5, 0.5, 0.5];
    transfer_share(&mut shares, 1, 1.0, 0.02, 1.0);
    assert!(shares.iter().all(|&s| (0.02 - 1e-6..=1.0).contains(&s)));
    assert!((shares.iter().sum::<f32>() - 1.5).abs() < 1e-6);
    assert_eq!(shares[0], 0.5);
}

/// A press on a handle, a move, and a release change the two shares beside it and say so.
#[test]
fn dragging_a_handle_trades_share_between_its_neighbours() {
    let mut graph = graph_with(&[0, 1, 2]);
    let layout = layout(&graph);
    let at = Point::new(layout.handle_x(0), layout.bar.y + 5.0);
    press(&mut graph, at);
    assert_eq!(
        graph.interaction,
        Interaction::RampHandle {
            node: 0,
            boundary: 0
        }
    );
    let to = Point::new(layout.bar.x + 0.2 * layout.bar.width, at.y);
    let message = move_to(&mut graph, to);
    assert!(matches!(message, Some(Message::GraphChanged)));
    assert!(share(&graph, 0) < 0.5 && share(&graph, 1) > 0.5);
    assert!((share(&graph, 0) + share(&graph, 1) - 1.0).abs() < 1e-3);
    assert!((share(&graph, 2) - 0.5).abs() < 1e-3);
    let moved = self::layout(&graph);
    assert!((moved.handle_x(0) - to.x).abs() < 1.0);
    release(&mut graph, to);
    assert_eq!(graph.interaction, Interaction::Idle);
}

#[test]
fn a_double_click_on_the_bar_resets_every_share() {
    let mut graph = graph_with(&[0, 1]);
    set_share(&mut graph, 0, 0.9);
    set_share(&mut graph, 1, 0.1);
    let layout = layout(&graph);
    let at = Point::new(layout.bar.x + 4.0, layout.bar.y + 10.0);
    press(&mut graph, at);
    release(&mut graph, at);
    assert!((share(&graph, 0) - 0.9).abs() < 1e-3);
    press(&mut graph, at);
    assert!((share(&graph, 0) - 0.5).abs() < 1e-3);
    assert!((share(&graph, 1) - 0.5).abs() < 1e-3);
    assert_eq!(graph.interaction, Interaction::Idle);
}

#[test]
fn a_single_press_on_the_bar_changes_nothing() {
    let mut graph = graph_with(&[0, 1]);
    set_share(&mut graph, 0, 0.9);
    let layout = layout(&graph);
    let at = Point::new(layout.bar.x + 4.0, layout.bar.y + 10.0);
    press(&mut graph, at);
    assert_eq!(graph.interaction, Interaction::Idle);
    move_to(&mut graph, Point::new(at.x + 50.0, at.y));
    assert!((share(&graph, 0) - 0.9).abs() < 1e-3);
}

#[test]
fn the_nearest_handle_is_picked() {
    let mut graph = graph_with(&[0, 1, 2]);
    set_share(&mut graph, 1, 0.02);
    let layout = layout(&graph);
    // The two handles are close together now, so each press lands on the nearer one.
    let (a, b) = (layout.handle_x(0), layout.handle_x(1));
    assert!(b - a < HANDLE_HITS);
    let y = layout.bar.y + 5.0;
    assert_eq!(layout.handle_at(Point::new(a - 1.0, y)), Some(0));
    assert_eq!(layout.handle_at(Point::new(b + 1.0, y)), Some(1));
    assert_eq!(layout.handle_at(Point::new(a - 40.0, y)), None);
    assert_eq!(layout.handle_at(Point::new(a, y + 80.0)), None);
}

/// The widest two handles can be apart and still both be in reach of a point between them.
const HANDLE_HITS: f32 = 18.0;

#[test]
fn the_position_dot_follows_a_driven_position() {
    let graph = graph_with(&[0, 1]);
    let layout = layout(&graph);
    let node = &graph.nodes[0];
    let own = live_position(&Live::of(node));
    let driven = Live::new(node, |id| (id == KnobId::WtPosition).then_some(0.75));
    assert_eq!(live_position(&driven), 0.75);
    let x = layout.position_x(live_position(&driven));
    assert!((x - (layout.bar.x + 0.75 * layout.bar.width)).abs() < 1e-3);
    assert!(x > layout.position_x(own) || own >= 0.75);
}

#[test]
fn the_cursor_links_a_region_and_its_pin_both_ways() {
    let mut graph = graph_with(&[0, 2]);
    let layout = layout(&graph);
    // Over a region.
    let over = Point::new(layout.bar.x + layout.bar.width - 3.0, layout.bar.y + 3.0);
    assert!(matches!(move_to(&mut graph, over), Some(Message::Hovered)));
    assert_eq!(graph.ramp_hover, Some((0, 2)));
    // Over a pin.
    let pin = graph.nodes[0].input_point(Port::Frame(0)).unwrap();
    move_to(&mut graph, pin);
    assert_eq!(graph.ramp_hover, Some((0, 0)));
    // Over the empty pin, which has no region, and over nothing.
    let empty = graph.nodes[0].input_point(Port::Frame(3)).unwrap();
    move_to(&mut graph, empty);
    assert_eq!(graph.ramp_hover, None);
    move_to(&mut graph, Point::new(900.0, 600.0));
    assert_eq!(graph.ramp_hover, None);
    // The lit states draw.
    graph.cursor_in = true;
    for hover in [Some((0, 0)), Some((0, 2)), None] {
        graph.ramp_hover = hover;
        let mut frame = Frame::new(BOUNDS.size());
        super::draw::draw_ramp(&mut frame, 0, &graph, ControlLevels::default(), &[], 1.0);
    }
}

#[test]
fn connecting_a_frame_resets_that_slots_share() {
    let mut graph = graph_with(&[0]);
    set_share(&mut graph, 1, 0.03);
    set_share(&mut graph, 0, 0.7);
    let source = graph.add_node(NodeKind::CurveFrame, Point::new(0.0, 0.0));
    graph.connect(source, Port::Table, 0, Port::Frame(1));
    assert!((share(&graph, 1) - 0.5).abs() < 1e-3);
    // Another slot's share is left alone.
    assert!((share(&graph, 0) - 0.7).abs() < 1e-3);
}

#[test]
fn the_ramp_row_is_counted_in_the_node_height() {
    let graph = graph_with(&[0]);
    let node = &graph.nodes[0];
    let bounds = node.ramp_bounds().unwrap();
    let outer = node.bounds();
    assert!(bounds.x >= outer.x && bounds.x + bounds.width <= outer.x + outer.width);
    assert!(bounds.y >= outer.y + HEADER_HEIGHT);
    assert!(bounds.y + bounds.height <= outer.y + outer.height);
    assert!(RAMP_ROW > bounds.height);
    let layout = layout(&graph);
    assert!(layout.bar.y + layout.bar.height + 3.0 <= bounds.y + bounds.height);
    // The Blend dropdown sits under the ramp's row, not over it.
    let dropdown = node.dropdowns()[0].bounds;
    assert!(dropdown.y >= bounds.y + bounds.height);
}

#[test]
fn a_collapsed_node_has_no_ramp_or_handle() {
    let mut graph = graph_with(&[0, 1]);
    let before = layout(&graph);
    let at = Point::new(before.handle_x(0), before.bar.y + 5.0);
    graph.nodes[0].collapsed = true;
    assert!(graph.nodes[0].ramp_bounds().is_none());
    assert!(ramp_layout(&graph, 0).is_none());
    press(&mut graph, at);
    assert!(!matches!(graph.interaction, Interaction::RampHandle { .. }));
    let mut frame = Frame::new(BOUNDS.size());
    super::draw::draw_ramp(&mut frame, 0, &graph, ControlLevels::default(), &[], 1.0);
}

#[test]
fn the_ramp_draws_in_every_state() {
    for slots in [&[][..], &[0], &[0, 1, 2], &[1, 3]] {
        let mut graph = graph_with(slots);
        for boundary in 0..slots.len().saturating_sub(1) {
            graph.interaction = Interaction::RampHandle { node: 0, boundary };
            let mut frame = Frame::new(BOUNDS.size());
            super::draw::draw_ramp(&mut frame, 0, &graph, ControlLevels::default(), &[], 1.0);
        }
        super::draw::draw_editor(&graph, ControlLevels::default(), [0; 2], [0.0; 2], BOUNDS);
    }
}
